//! Independent package profiles.
//!
//! A target in [`PackageMode::Profile`] keeps its packages in a dedicated Nix
//! profile. Package changes build that profile from the configuration's
//! `nixbox.profile` option without switching anything, and every switch of
//! the configuration sets the profile to the same environment, so a fresh
//! machine, a rollback or a nixpkgs update all bring the profile along.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::BaseDirs;
use nixbox_config::Target;
use nixbox_nix::manifest::{FlakeOutput, ManagedFlakeFile};
use nixbox_nix::wiring::attr_name;
use nixbox_nix::{NIXOS_HOST, PackageMode, flake_has_home_configuration, home_configuration_name};
use serde::{Deserialize, Serialize};

use crate::{Engine, Op, RebuildCommand};

/// What has to run after files are written. Saved builds without a mode
/// predate profiles and keep their configuration switch.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Activation {
	/// Build the target's profile alone.
	Packages,
	/// Switch the configuration. A profile-mode target's switch also sets its
	/// profile, so nothing has to be built separately first.
	#[default]
	#[serde(alias = "packages-and-configuration")]
	Configuration,
}

impl Engine {
	/// Decide before mutating files, since removing a module or an external
	/// declaration changes which activation is needed.
	#[must_use]
	pub fn activation_for(&self, ops: &[Op]) -> Activation {
		let configuration = crate::rebuild::builds_vm()
			|| (self
				.config
				.home_user
				.as_deref()
				.is_some_and(|user| user != home_configuration_name())
				&& ops.iter().any(|op| op.scope() == Target::HomeManager))
			|| ops.iter().any(|op| {
				self.manifest_for(op.scope()).mode == PackageMode::Configuration
					|| match op {
						Op::Install { .. }
						| Op::Uninstall { .. }
						| Op::InstallFlakePackage { .. } => false,
						Op::UninstallFlakeOutput {
							input,
							output: FlakeOutput::Package(package),
							scope,
						} => nixbox_nix::scan::scan_flake_packages(
							&self.config.main_file_for(*scope),
							crate::scan_target(*scope),
						)
						.map_or(true, |entries| {
							entries
								.iter()
								.any(|entry| &entry.input == input && &entry.package == package)
						}),
						Op::UninstallFlake { repo, scope } => {
							ManagedFlakeFile::new(self.config.flake_manifest_for(*scope))
								.load()
								.map_or(true, |manifest| manifest.modules.contains_key(repo))
						}
						_ => true,
					}
			});
		if configuration {
			Activation::Configuration
		} else {
			Activation::Packages
		}
	}
}

/// Where `scope`'s profile lives. Its generations sit beside it as
/// `nixbox-<n>-link`.
pub fn profile_path(scope: Target) -> Result<PathBuf> {
	match scope {
		Target::HomeManager => Ok(BaseDirs::new()
			.context("locating home directory")?
			.home_dir()
			.join(".local/state/nix/profiles/nixbox")),
		Target::NixosSystem => Ok(PathBuf::from("/nix/var/nix/profiles/nixbox")),
	}
}

/// How far a target's profile setup has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupStatus {
	/// Packages install through the configuration; there is no profile.
	Configuration,
	/// The files are in profile mode, but no configuration containing them
	/// has been activated on this machine.
	NotActivated,
	/// Activated, but the profile is not on this process's PATH. A new login
	/// session picks it up; so would a shell that reads the new environment.
	NotOnPath,
	/// Activated and on this process's PATH.
	Active,
}

impl SetupStatus {
	/// A stable name for machine-readable output.
	#[must_use]
	pub fn name(self) -> &'static str {
		match self {
			Self::Configuration => "not-set-up",
			Self::NotActivated => "not-activated",
			Self::NotOnPath => "not-on-path",
			Self::Active => "active",
		}
	}

	/// One line for `status` and `doctor`.
	#[must_use]
	pub fn describe(self, scope: Target) -> String {
		let target = scope.label();
		match self {
			Self::Configuration => {
				format!("not set up; run `nixbox --target {target} profile setup` to use one")
			}
			Self::NotActivated => format!(
				"set up but not activated here; run `nixbox --target {target} apply --configuration`"
			),
			Self::NotOnPath => {
				"activated, but not on this process's PATH; start a new login session".into()
			}
			Self::Active => "active".into(),
		}
	}
}

/// Where `scope`'s profile setup stands on this machine.
#[must_use]
pub fn setup_status(engine: &Engine, scope: Target) -> SetupStatus {
	if engine.manifest_for(scope).mode == PackageMode::Configuration {
		return SetupStatus::Configuration;
	}
	if scope == Target::HomeManager
		&& engine.config.native_modules_dir.is_some()
		&& engine.config.home_user.as_deref() != Some(home_configuration_name().as_str())
	{
		return SetupStatus::NotActivated;
	}
	activation_status(scope)
}

/// [`setup_status`] for a target already known to be in profile mode.
#[must_use]
pub fn activation_status(scope: Target) -> SetupStatus {
	let Ok(profile) = profile_path(scope) else {
		return SetupStatus::NotActivated;
	};
	let marker = match scope {
		Target::HomeManager => {
			BaseDirs::new().map(|base| base.home_dir().join(".local/state/nixbox/profile-ready"))
		}
		Target::NixosSystem => Some(PathBuf::from("/etc/nixbox/profile-ready")),
	};
	let activated = marker
		.and_then(|path| std::fs::read_to_string(path).ok())
		.is_some_and(|value| value.trim() == profile.to_string_lossy());
	if !activated {
		SetupStatus::NotActivated
	} else if on_path(&profile, std::env::var_os("PATH").as_deref()) {
		SetupStatus::Active
	} else {
		SetupStatus::NotOnPath
	}
}

fn on_path(profile: &Path, path: Option<&OsStr>) -> bool {
	let bin = profile.join("bin");
	path.is_some_and(|path| std::env::split_paths(path).any(|dir| dir == bin))
}

/// The flake attribute holding `scope`'s profile environment: the same
/// `nixbox.profile` value the activation script sets the profile to.
#[must_use]
pub fn installable(config_dir: &Path, scope: Target, standalone_home: bool) -> String {
	let nixos = attr_name(NIXOS_HOST);
	let user = nixbox_nix::dendritic::Layout::discover(config_dir)
		.ok()
		.flatten()
		.and_then(|layout| layout.home_user)
		.unwrap_or_else(home_configuration_name);
	let user = attr_name(&user);
	let attr = match scope {
		Target::NixosSystem => format!("nixosConfigurations.{nixos}.config.nixbox.profile"),
		Target::HomeManager if standalone_home => {
			format!("homeConfigurations.{user}.config.nixbox.profile")
		}
		Target::HomeManager => {
			format!("nixosConfigurations.{nixos}.config.home-manager.users.{user}.nixbox.profile")
		}
	};
	format!("{}#{attr}", config_dir.display())
}

/// Records new or changed inputs in `flake.lock` as the invoking user, so a
/// profile build under `sudo` never rewrites the lock file as root.
#[must_use]
pub fn lock_command(config_dir: &Path) -> RebuildCommand {
	plain(
		"nix",
		vec![
			"flake".into(),
			"lock".into(),
			config_dir.to_string_lossy().into_owned(),
		],
	)
}

/// Builds `scope`'s profile environment into a new profile generation.
/// Evaluating the flake decides where a Home Manager configuration lives.
pub async fn command(config_dir: &Path, scope: Target) -> Result<RebuildCommand> {
	let standalone = scope == Target::HomeManager && flake_has_home_configuration(config_dir).await;
	build_command(config_dir, scope, standalone)
}

/// Build the profile from the same selected host/user as mutation and recovery.
pub async fn command_for(config: &nixbox_config::Config, scope: Target) -> Result<RebuildCommand> {
	let Some(layout) = nixbox_nix::dendritic::Layout::select_output(
		&config.config_root(),
		config.nixos_host.as_deref(),
		config.home_user.as_deref(),
		config.home_configuration.as_deref(),
	)?
	else {
		return command(&config.config_root(), scope).await;
	};
	// Applying a different user's profile locally would modify the invoking
	// user's profile. Such changes must activate through the owning host.
	if scope == Target::HomeManager {
		anyhow::ensure!(
			layout.home_user.as_deref() == Some(home_configuration_name().as_str()),
			"the selected Home Manager user differs from the invoking user; use configuration activation"
		);
	}
	let host = attr_name(&layout.host);
	let attribute = if layout.standalone {
		anyhow::ensure!(
			scope == Target::HomeManager,
			"standalone Home Manager has no NixOS scope"
		);
		format!("homeConfigurations.{host}.config.nixbox.profile")
	} else {
		match scope {
			Target::NixosSystem => format!("nixosConfigurations.{host}.config.nixbox.profile"),
			Target::HomeManager => format!(
				"nixosConfigurations.{host}.config.home-manager.users.{}.nixbox.profile",
				attr_name(layout.require_home()?)
			),
		}
	};
	Ok(as_owner(
		scope,
		"nix",
		vec![
			"build".into(),
			"--no-link".into(),
			"--profile".into(),
			profile_path(scope)?.display().to_string(),
			format!("{}#{attribute}", config.config_root().display()),
		],
	))
}

fn build_command(
	config_dir: &Path,
	scope: Target,
	standalone_home: bool,
) -> Result<RebuildCommand> {
	let args = vec![
		"build".into(),
		"--no-link".into(),
		"--profile".into(),
		profile_path(scope)?.to_string_lossy().into_owned(),
		installable(config_dir, scope, standalone_home),
	];
	Ok(as_owner(scope, "nix", args))
}

/// Lists `scope`'s profile generations.
pub fn generations_command(scope: Target) -> Result<RebuildCommand> {
	Ok(plain(
		"nix-env",
		vec![
			"--profile".into(),
			profile_path(scope)?.to_string_lossy().into_owned(),
			"--list-generations".into(),
		],
	))
}

/// Points `scope`'s profile at the previous generation, or at `generation`.
pub fn rollback_command(scope: Target, generation: Option<u32>) -> Result<RebuildCommand> {
	let mut args = vec![
		"--profile".into(),
		profile_path(scope)?.to_string_lossy().into_owned(),
	];
	match generation {
		Some(generation) => args.extend(["--switch-generation".into(), generation.to_string()]),
		None => args.push("--rollback".into()),
	}
	Ok(as_owner(scope, "nix-env", args))
}

/// Runs `program` as whoever owns `scope`'s profile: root for the system.
fn as_owner(scope: Target, program: &str, mut args: Vec<String>) -> RebuildCommand {
	match scope {
		Target::HomeManager => plain(program, args),
		Target::NixosSystem => {
			args.insert(0, program.into());
			plain("sudo", args)
		}
	}
}

fn plain(program: &str, args: Vec<String>) -> RebuildCommand {
	RebuildCommand {
		program: program.into(),
		args,
		envs: Vec::new(),
		via_nixos_fallback: false,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::SilentReporter;
	use crate::tests::{hit, temp_dir};
	use nixbox_nix::{ManagedFile, Manifest};

	#[test]
	fn only_setup_and_teardown_change_a_targets_mode() {
		let dir = temp_dir("profile-mode");
		let config = dir.config();
		for scope in [Target::HomeManager, Target::NixosSystem] {
			std::fs::write(
				config.main_file_for(scope),
				"{ pkgs, ... }:\n{\n  imports = [ ];\n}\n",
			)
			.unwrap();
		}
		let mut engine = Engine::from_parts(
			config.clone(),
			Manifest::default(),
			Manifest::default(),
			vec![],
		);
		let managed = config.managed_file_for(Target::HomeManager);
		let read = |path: &Path| std::fs::read_to_string(path).unwrap();
		let install = |name: &str| Op::Install {
			hit: hit(name),
			scope: Target::HomeManager,
		};

		engine.apply(&install("fd"), &mut SilentReporter).unwrap();
		assert!(read(&managed).contains("home.packages = ["));

		engine
			.apply(
				&Op::SetupProfile {
					scope: Target::HomeManager,
					home_inside_nixos: true,
				},
				&mut SilentReporter,
			)
			.unwrap();
		engine
			.apply(&install("ripgrep"), &mut SilentReporter)
			.unwrap();
		let loaded = ManagedFile::new(&managed).load().unwrap();
		assert_eq!(loaded.mode, PackageMode::Profile);
		assert_eq!(loaded.packages.len(), 2);
		assert!(read(&managed).contains("nixbox.packages = ["));
		let integration = config.home_integration_file();
		assert!(read(&integration).contains("environment.profiles"));
		assert!(
			read(&config.main_file_for(Target::NixosSystem))
				.contains("./nixbox-home-integration.nix")
		);

		engine
			.apply(
				&Op::TeardownProfile {
					scope: Target::HomeManager,
				},
				&mut SilentReporter,
			)
			.unwrap();
		let loaded = ManagedFile::new(&managed).load().unwrap();
		assert_eq!(loaded.mode, PackageMode::Configuration);
		assert_eq!(loaded.packages.len(), 2);
		assert!(!read(&integration).contains("environment.profiles"));
	}

	#[test]
	fn package_commands_build_the_option_the_switch_activates() {
		let config = Path::new("/tmp/config");
		let user = attr_name(&home_configuration_name());
		for (scope, standalone, attr) in [
			(
				Target::NixosSystem,
				false,
				"nixosConfigurations.nixos.config.nixbox.profile".to_string(),
			),
			(
				Target::HomeManager,
				true,
				format!("homeConfigurations.{user}.config.nixbox.profile"),
			),
			(
				Target::HomeManager,
				false,
				format!(
					"nixosConfigurations.nixos.config.home-manager.users.{user}.nixbox.profile"
				),
			),
		] {
			let cmd = build_command(config, scope, standalone).unwrap();
			assert_eq!(cmd.args.last().unwrap(), &format!("/tmp/config#{attr}"));
			assert!(cmd.args.iter().any(|arg| arg == "--profile"));
			assert!(!cmd.args.iter().any(|arg| arg == "switch"));
			assert_eq!(
				cmd.program,
				if scope == Target::HomeManager {
					"nix"
				} else {
					"sudo"
				}
			);
		}
	}

	#[test]
	fn only_profile_mode_package_changes_skip_the_switch() {
		let dir = temp_dir("profile-activation");
		let mut engine = Engine::from_parts(
			dir.config(),
			Manifest::default(),
			Manifest::default(),
			vec![],
		);
		let install = Op::Install {
			hit: hit("fd"),
			scope: Target::NixosSystem,
		};
		assert_eq!(
			engine.activation_for(std::slice::from_ref(&install)),
			Activation::Configuration
		);

		engine.nixos_manifest.mode = PackageMode::Profile;
		assert_eq!(
			engine.activation_for(std::slice::from_ref(&install)),
			Activation::Packages
		);
		assert_eq!(
			engine.activation_for(&[
				install,
				Op::Migrate {
					names: vec!["htop".into()],
					scope: Target::NixosSystem,
				}
			]),
			Activation::Configuration
		);
	}

	#[test]
	fn vm_builds_never_touch_the_hosts_profile() {
		// Nextest runs each test in its own process.
		unsafe { std::env::set_var(crate::rebuild::VM_BUILD_ENV, "/tmp/vm-result") };
		let dir = temp_dir("profile-vm");
		let mut engine = Engine::from_parts(
			dir.config(),
			Manifest::default(),
			Manifest::default(),
			vec![],
		);
		engine.home_manifest.mode = PackageMode::Profile;
		assert_eq!(
			engine.activation_for(&[Op::Install {
				hit: hit("fd"),
				scope: Target::HomeManager,
			}]),
			Activation::Configuration
		);
	}

	#[test]
	fn saved_activation_modes_load_from_every_release() {
		for (raw, expected) in [
			(
				r#"{"scope":"nixos-system","label":"old rebuild"}"#,
				Activation::Configuration,
			),
			(
				r#"{"scope":"nixos-system","label":"x","activation":"packages-and-configuration"}"#,
				Activation::Configuration,
			),
			(
				r#"{"scope":"nixos-system","label":"x","activation":"packages"}"#,
				Activation::Packages,
			),
		] {
			let saved: crate::InProgress = serde_json::from_str(raw).unwrap();
			assert_eq!(saved.activation, expected);
		}
	}

	#[test]
	fn the_profile_counts_as_on_path_only_through_its_bin_directory() {
		let profile = Path::new("/nix/var/nix/profiles/nixbox");
		let path = std::env::join_paths(["/run/wrappers/bin", "/nix/var/nix/profiles/nixbox/bin"])
			.unwrap();
		assert!(on_path(profile, Some(&path)));
		let path = std::env::join_paths(["/nix/var/nix/profiles/nixbox"]).unwrap();
		assert!(!on_path(profile, Some(&path)));
		assert!(!on_path(profile, None));
	}

	#[test]
	fn rollback_targets_the_profile_as_its_owner() {
		let system = rollback_command(Target::NixosSystem, Some(4)).unwrap();
		assert_eq!(system.program, "sudo");
		assert_eq!(
			system.args,
			[
				"nix-env",
				"--profile",
				"/nix/var/nix/profiles/nixbox",
				"--switch-generation",
				"4"
			]
		);
		let home = rollback_command(Target::HomeManager, None).unwrap();
		assert_eq!(home.program, "nix-env");
		assert_eq!(home.args.last().unwrap(), "--rollback");
	}
}
