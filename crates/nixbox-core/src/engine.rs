//! Applies [`Op`]s to the user's configuration.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Result;
use nixbox_config::{Config, Target};
use nixbox_nix::{
	Manifest, PackageMode, SettingsFile, SettingsManifest,
	manifest::{
		FlakeOutput, ImportStatus, ManagedFile, ensure_imported, migrate_flake_file,
		write_home_integration,
	},
	scan::{ExternalPackage, ScanTarget, remove_from_source, scan},
};

use crate::flakes::{InstalledFlake, scan_flakes};
use crate::op::Op;
use crate::report::Reporter;

/// A package tracked by nixbox's manifest, tagged with the scope it lives in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedPackage {
	pub name: String,
	pub scope: Target,
}

/// The scan scope matching a rebuild target.
#[must_use]
pub fn scan_target(scope: Target) -> ScanTarget {
	match scope {
		Target::HomeManager => ScanTarget::HomeManager,
		Target::NixosSystem => ScanTarget::Nixos,
	}
}

/// Whether a scanned package belongs to `target`'s scope.
#[must_use]
pub fn scope_matches(scan_scope: ScanTarget, target: Target) -> bool {
	matches!(
		(scan_scope, target),
		(ScanTarget::HomeManager, Target::HomeManager) | (ScanTarget::Nixos, Target::NixosSystem)
	)
}

/// Holds the user's settings plus both manifests, and owns every mutation of
/// the files nixbox generates.
///
/// `external_packages` is maintained as "declared in a main config file and
/// *not* already tracked in the matching manifest" — [`Engine::apply`] keeps
/// that invariant as packages migrate in.
pub struct Engine {
	pub config: Config,
	identity: nixbox_config::ConfigurationIdentity,
	/// Packages tracked in `nixbox-home-packages.nix`.
	pub home_manifest: Manifest,
	/// Packages tracked in `nixbox-system-packages.nix`.
	pub nixos_manifest: Manifest,
	/// Packages declared directly in the user's own config files.
	pub external_packages: Vec<ExternalPackage>,
	/// Flake outputs in the configuration, whether nixbox wired them in or
	/// the user did.
	pub flakes: Vec<InstalledFlake>,
	/// Options set in `nixbox-home-settings.nix`.
	pub home_settings: SettingsManifest,
	/// Options set in `nixbox-system-settings.nix`.
	pub nixos_settings: SettingsManifest,
}

impl Engine {
	/// Resolve placement and target identity without writing or loading manifests.
	pub fn resolve_config(mut config: Config) -> Result<Config> {
		config.native_modules_dir = None;
		config.legacy_system_module = false;
		config.legacy_home_module = false;
		if let Some(layout) = nixbox_nix::dendritic::Layout::select(
			&config.config_root(),
			config.nixos_host.as_deref(),
			config.home_user.as_deref(),
		)? {
			config.nixos_host = Some(layout.host.clone());
			config.home_user.clone_from(&layout.home_user);
			config.native_modules_dir = Some(layout.tree.clone());
			config.legacy_system_module =
				layout.owns_root_file(&config.config_root().join("nixbox-system-packages.nix"))?;
			config.legacy_home_module =
				layout.owns_root_file(&config.config_root().join("nixbox-home-packages.nix"))?;
		}
		Ok(config)
	}

	pub fn ensure_identity(&self) -> Result<()> {
		anyhow::ensure!(
			self.identity == self.config.identity(),
			"configuration selection changed; reload the engine before applying work"
		);
		Ok(())
	}

	/// The file where target integration is attached. Dendritic package
	/// modules enter through flake-parts rather than a conventional entry file.
	#[must_use]
	pub fn integration_file_for(&self, scope: Target) -> std::path::PathBuf {
		if matches!(
			nixbox_nix::dendritic::Layout::select(
				&self.config.config_root(),
				self.config.nixos_host.as_deref(),
				self.config.home_user.as_deref()
			),
			Ok(Some(_))
		) {
			self.config.flake_file()
		} else {
			self.config.main_file_for(scope)
		}
	}

	/// Loads settings from disk, then both manifests, then scans for
	/// externally-declared packages.
	pub fn load() -> Result<Self> {
		Self::with_config(Config::load_or_default()?)
	}

	/// Same as [`Engine::load`] but with settings supplied by the caller —
	/// used when a front-end overrides the target or channel per invocation.
	pub fn with_config(config: Config) -> Result<Self> {
		let config = Self::resolve_config(config)?;
		if nixbox_nix::dendritic::Layout::select(
			&config.config_root(),
			config.nixos_host.as_deref(),
			config.home_user.as_deref(),
		)?
		.is_some()
		{
			anyhow::ensure!(
				[Target::HomeManager, Target::NixosSystem]
					.into_iter()
					.all(|scope| !config.legacy_flake_manifest_for(scope).exists()),
				"legacy flake-file migration must be completed before dendritic integration"
			);
		}
		if [Target::HomeManager, Target::NixosSystem]
			.into_iter()
			.any(|scope| config.legacy_flake_manifest_for(scope).exists())
		{
			let _journal = crate::vcs::AppliedJournal::acquire(&config)?;
			let mut sources = vec![config.config_root()];
			sources.extend(
				[Target::HomeManager, Target::NixosSystem]
					.into_iter()
					.map(|scope| config.main_file_for(scope)),
			);
			for scope in [Target::HomeManager, Target::NixosSystem] {
				let package_file = config.managed_file_for(scope);
				if migrate_flake_file(
					&package_file,
					&config.legacy_flake_manifest_for(scope),
					&sources,
					scope == Target::HomeManager,
				)? {
					git_track(&package_file, &mut crate::SilentReporter);
				}
			}
		}
		let home_manifest =
			ManagedFile::new(config.managed_file_for(Target::HomeManager)).load()?;
		let nixos_manifest =
			ManagedFile::new(config.managed_file_for(Target::NixosSystem)).load()?;
		let external_packages = scan_externals(&config, &home_manifest, &nixos_manifest);
		let flakes = scan_flakes(&config);
		let home_settings =
			SettingsFile::new(config.settings_file_for(Target::HomeManager)).load()?;
		let nixos_settings =
			SettingsFile::new(config.settings_file_for(Target::NixosSystem)).load()?;
		Ok(Self {
			identity: config.identity(),
			config,
			home_manifest,
			nixos_manifest,
			external_packages,
			flakes,
			home_settings,
			nixos_settings,
		})
	}

	/// Builds an engine from parts, without touching the filesystem.
	#[must_use]
	pub fn from_parts(
		config: Config,
		home_manifest: Manifest,
		nixos_manifest: Manifest,
		external_packages: Vec<ExternalPackage>,
	) -> Self {
		Self {
			identity: config.identity(),
			config,
			home_manifest,
			nixos_manifest,
			external_packages,
			flakes: Vec::new(),
			home_settings: SettingsManifest::default(),
			nixos_settings: SettingsManifest::default(),
		}
	}

	#[must_use]
	pub fn manifest_for(&self, scope: Target) -> &Manifest {
		match scope {
			Target::HomeManager => &self.home_manifest,
			Target::NixosSystem => &self.nixos_manifest,
		}
	}

	pub fn manifest_for_mut(&mut self, scope: Target) -> &mut Manifest {
		match scope {
			Target::HomeManager => &mut self.home_manifest,
			Target::NixosSystem => &mut self.nixos_manifest,
		}
	}

	/// Every managed package from both scopes, home-manager entries first.
	#[must_use]
	pub fn managed_packages(&self) -> Vec<ManagedPackage> {
		let mut out: Vec<ManagedPackage> = Vec::new();
		for name in &self.home_manifest.packages {
			out.push(ManagedPackage {
				name: name.clone(),
				scope: Target::HomeManager,
			});
		}
		for name in &self.nixos_manifest.packages {
			out.push(ManagedPackage {
				name: name.clone(),
				scope: Target::NixosSystem,
			});
		}
		out
	}

	/// True if `name` is already tracked in `scope`'s manifest.
	#[must_use]
	pub fn is_tracked(&self, name: &str, scope: Target) -> bool {
		self.manifest_for(scope).packages.contains(name)
	}

	/// Whether `scope` has a main config file, which a package migrated
	/// across into it needs to be imported from.
	#[must_use]
	pub fn has_main_file(&self, scope: Target) -> bool {
		if let Ok(Some(layout)) = nixbox_nix::dendritic::Layout::select(
			&self.config.config_root(),
			self.config.nixos_host.as_deref(),
			self.config.home_user.as_deref(),
		) {
			return scope == Target::NixosSystem || layout.home_user.is_some();
		}
		self.config.main_file_for(scope).exists()
	}

	/// Reports whether `scope`'s managed file is imported, without writing
	/// anything — [`Engine::apply`] is what repairs a missing import.
	#[must_use]
	pub fn import_state(&self, scope: Target) -> ImportState {
		match nixbox_nix::dendritic::Layout::select(
			&self.config.config_root(),
			self.config.nixos_host.as_deref(),
			self.config.home_user.as_deref(),
		) {
			Ok(Some(layout)) => {
				return if layout
					.is_loaded(&self.config.managed_file_for(scope))
					.unwrap_or(false)
					&& self.config.managed_file_for(scope).exists()
				{
					ImportState::Imported
				} else {
					ImportState::NotImported
				};
			}
			Err(_) => return ImportState::NotImported,
			Ok(None) => {}
		}
		let main_file = self.config.main_file_for(scope);
		let managed_file = self.config.managed_file_for(scope);
		let Some(name) = managed_file.file_name().and_then(|n| n.to_str()) else {
			return ImportState::NotImported;
		};
		let Ok(source) = std::fs::read_to_string(&main_file) else {
			return ImportState::MainFileMissing;
		};
		if source.contains(name) {
			ImportState::Imported
		} else {
			ImportState::NotImported
		}
	}

	/// Re-reads both main config files and refreshes `external_packages`
	/// and `flakes`.
	pub fn refresh_externals(&mut self) {
		self.external_packages =
			scan_externals(&self.config, &self.home_manifest, &self.nixos_manifest);
		self.flakes = scan_flakes(&self.config);
	}

	/// Writes `scope`'s manifest to its managed file in the right format.
	pub fn write_manifest(&self, scope: Target) -> Result<ManagedFile> {
		self.ensure_identity()?;
		let managed = ManagedFile::new(self.config.managed_file_for(scope));
		match scope {
			Target::HomeManager => managed.write_home_manager(self.manifest_for(scope))?,
			Target::NixosSystem => managed.write_nixos(self.manifest_for(scope))?,
		}
		if nixbox_nix::dendritic::Layout::select(
			&self.config.config_root(),
			self.config.nixos_host.as_deref(),
			self.config.home_user.as_deref(),
		)?
		.is_some()
		{
			git_track(managed.path(), &mut crate::SilentReporter);
			git_track(&self.config.flake_file(), &mut crate::SilentReporter);
		}
		Ok(managed)
	}

	/// Rewrites `scope`'s combined package module in `mode`. This is
	/// the only place a target changes mode: ordinary package changes keep
	/// whatever mode the files already have.
	fn set_package_mode(
		&mut self,
		scope: Target,
		mode: PackageMode,
		home_inside_nixos: bool,
		reporter: &mut dyn Reporter,
	) -> Result<()> {
		let main = self.integration_file_for(scope);
		anyhow::ensure!(
			self.has_main_file(scope),
			"{} is missing; create the main {} configuration first",
			main.display(),
			scope.label()
		);
		let integrate =
			scope == Target::HomeManager && mode == PackageMode::Profile && home_inside_nixos;
		let nixos_main = self.integration_file_for(Target::NixosSystem);
		anyhow::ensure!(
			!integrate || self.has_main_file(Target::NixosSystem),
			"Home Manager runs inside NixOS, but {} is missing; set `nixos_main_file` so the profile can join PATH",
			nixos_main.display()
		);

		self.manifest_for_mut(scope).mode = mode;
		let managed = self.write_manifest(scope)?;
		note_import(&main, managed.path(), reporter);
		git_track(managed.path(), reporter);
		git_track(&main, reporter);

		let integration = self.config.home_integration_file();
		if integrate {
			write_home_integration(&integration, true)?;
			note_import(&nixos_main, &integration, reporter);
			git_track(&integration, reporter);
			git_track(&nixos_main, reporter);
		} else if scope == Target::HomeManager && integration.exists() {
			// Keep the import valid, but take the profile off PATH.
			write_home_integration(&integration, false)?;
			git_track(&integration, reporter);
		}

		reporter.info(format!(
			"Wrote {} for {}.",
			match mode {
				PackageMode::Configuration => "configuration-managed packages",
				PackageMode::Profile => "the independent package profile",
			},
			scope.label()
		));
		Ok(())
	}

	/// Applies one op: updates the in-memory manifest, writes every file the
	/// op touches, makes sure the managed file is imported, and keeps git
	/// aware of what was generated.
	///
	/// This does *not* rebuild. Writing first is deliberate — if the rebuild
	/// is interrupted the on-disk state already reflects the intent, so
	/// recovery is just re-running the rebuild.
	pub fn apply(&mut self, op: &Op, reporter: &mut dyn Reporter) -> Result<()> {
		self.ensure_identity()?;
		let dendritic = nixbox_nix::dendritic::Layout::select(
			&self.config.config_root(),
			self.config.nixos_host.as_deref(),
			self.config.home_user.as_deref(),
		)?;
		if let Some(layout) = &dendritic {
			if op.scope() == Target::HomeManager {
				layout.require_home()?;
			}
			anyhow::ensure!(
				!matches!(
					op,
					Op::Migrate { .. } | Op::MigrateAcross { .. } | Op::MigrateFlakePackage { .. }
				),
				"migration from dendritic feature modules is read-only until connected-source removal is supported"
			);
			layout.validate_import(&self.config.managed_file_for(op.scope()))?;
			for scope in [Target::HomeManager, Target::NixosSystem] {
				anyhow::ensure!(
					!self.config.legacy_flake_manifest_for(scope).exists(),
					"legacy flake-file migration must be completed before dendritic integration"
				);
			}
		}
		let journal = crate::vcs::AppliedJournal::acquire(&self.config)?;
		let before = crate::vcs::operation_files(&self.config);
		let home = self.home_manifest.clone();
		let system = self.nixos_manifest.clone();
		let home_settings = self.home_settings.clone();
		let system_settings = self.nixos_settings.clone();
		let result = self.apply_inner(op, reporter);
		if let Err(error) = result {
			if dendritic.is_some() {
				self.home_manifest = home;
				self.nixos_manifest = system;
				self.home_settings = home_settings;
				self.nixos_settings = system_settings;
				let writable = [
					self.config.flake_file(),
					self.config.managed_file_for(op.scope()),
					self.config.settings_file_for(op.scope()),
					self.config.home_integration_file(),
				];
				for (path, original) in &before {
					// Conventional entry files and the other target are read-only
					// during this operation, including paths outside the root.
					if !writable.contains(path) {
						continue;
					}
					if std::fs::read(path).ok().as_ref() == original.as_ref() {
						continue;
					}
					match original {
						Some(bytes) => std::fs::write(path, bytes)?,
						None if path.exists() => std::fs::remove_file(path)?,
						None => {}
					}
				}
				self.refresh_externals();
			}
			return Err(error);
		}
		if dendritic.is_some() {
			git_track(&self.config.flake_file(), reporter);
		}
		if let Err(error) = journal.record(&self.config, op, before) {
			reporter.warn(format!(
				"Configuration applied, but VCS journal could not be saved: {error:#}"
			));
		}
		Ok(())
	}

	fn apply_inner(&mut self, op: &Op, reporter: &mut dyn Reporter) -> Result<()> {
		let scope = op.scope();
		match op {
			Op::SetupProfile {
				home_inside_nixos, ..
			} => {
				return self.set_package_mode(
					scope,
					PackageMode::Profile,
					*home_inside_nixos,
					reporter,
				);
			}
			Op::TeardownProfile { .. } => {
				return self.set_package_mode(scope, PackageMode::Configuration, false, reporter);
			}
			Op::Install { hit, .. } => {
				self.manifest_for_mut(scope).add(&hit.attr);
			}
			Op::InstallFlake { repo, module, .. } => {
				let output = FlakeOutput::Module(module.clone());
				return self.apply_flake_output(repo, &output, scope, reporter);
			}
			Op::InstallFlakePackage { repo, package, .. } => {
				let output = FlakeOutput::Package(package.clone());
				return self.apply_flake_output(repo, &output, scope, reporter);
			}
			Op::UninstallFlake { repo, .. } => {
				return self.remove_flake(repo, scope, reporter);
			}
			Op::UninstallFlakeOutput { input, output, .. } => {
				return self.remove_flake_output(input, output, scope, reporter);
			}
			Op::Uninstall { name, .. } => {
				self.manifest_for_mut(scope).remove(name);
			}
			Op::Migrate { names, .. } => {
				let source = self.config.main_file_for(scope);
				let removed = remove_from_source(&source, scan_target(scope), names)?;
				for name in &removed {
					self.manifest_for_mut(scope).add(name);
				}
				// Drop any externals that just moved into the manifest.
				self.external_packages
					.retain(|ep| !(scope_matches(ep.scope, scope) && removed.contains(&ep.name)));
			}
			Op::MigrateAcross { names, from, .. } => {
				// Wire the destination up before the source loses its lines,
				// so a host without that target keeps its declaration.
				let destination = self.config.main_file_for(scope);
				if ensure_imported(&destination, &self.config.managed_file_for(scope))?
					== ImportStatus::MainFileMissing
				{
					anyhow::bail!(
						"{} not found; {} is not configured on this machine",
						destination.display(),
						scope.title()
					);
				}
				let source = self.config.main_file_for(*from);
				let removed = remove_from_source(&source, scan_target(*from), names)?;
				for name in &removed {
					self.manifest_for_mut(scope).add(name);
				}
				self.external_packages
					.retain(|ep| !(scope_matches(ep.scope, *from) && removed.contains(&ep.name)));
				// The shared tail only tracks the destination's main file.
				if source.exists() {
					git_track(&source, reporter);
				}
			}
			Op::MigrateFlakePackage { input, package, .. } => {
				return self.migrate_flake_package(input, package, scope, reporter);
			}
			Op::SetOptions { changes, .. } => {
				return self.apply_settings(changes, scope, reporter);
			}
		}

		let managed = self.write_manifest(scope)?;
		reporter.info(format!(
			"Wrote {} ({}). {}...",
			managed.path().display(),
			scope.label(),
			op.label(),
		));
		let main_file = self.config.main_file_for(scope);
		note_import(&main_file, managed.path(), reporter);
		// Flakes ignore untracked files — make sure git sees the managed file
		// (and the main config if we just touched it).
		git_track(managed.path(), reporter);
		if main_file.exists() {
			git_track(&main_file, reporter);
		}
		Ok(())
	}
}

/// Whether a target's managed file is wired into the user's own config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportState {
	/// The main config file imports the managed file.
	Imported,
	/// The main config file exists but does not import the managed file.
	NotImported,
	/// There is no main config file to import from.
	MainFileMissing,
}

/// Scans both main config files and returns the packages declared in them,
/// scope-tagged, excluding anything already in the matching manifest.
///
/// The same name can appear twice when it is declared in both scopes — that
/// is intentional, they are two separate declarations.
#[must_use]
pub fn scan_externals(
	config: &Config,
	home_manifest: &Manifest,
	nixos_manifest: &Manifest,
) -> Vec<ExternalPackage> {
	let mut out: Vec<ExternalPackage> = Vec::new();
	match nixbox_nix::dendritic::Layout::select(
		&config.config_root(),
		config.nixos_host.as_deref(),
		config.home_user.as_deref(),
	) {
		Ok(Some(layout)) => {
			for (home, manifest) in [(true, home_manifest), (false, nixos_manifest)] {
				out.extend(
					layout
						.scan_packages(home)
						.unwrap_or_default()
						.into_iter()
						.filter(|entry| !manifest.packages.contains(&entry.name)),
				);
			}
			return out;
		}
		Err(_) => return out,
		Ok(None) => {}
	}

	if let Ok(found) = scan(
		&config.main_file_for(Target::HomeManager),
		ScanTarget::HomeManager,
	) {
		out.extend(
			found
				.into_iter()
				.filter(|ep| !home_manifest.packages.contains(&ep.name)),
		);
	}
	if let Ok(found) = scan(
		&config.main_file_for(Target::NixosSystem),
		ScanTarget::Nixos,
	) {
		out.extend(
			found
				.into_iter()
				.filter(|ep| !nixos_manifest.packages.contains(&ep.name)),
		);
	}

	out
}

/// Reports whatever `ensure_imported` had to change, and stays quiet when the
/// managed file was already imported.
pub(crate) fn note_import(main_file: &Path, managed_file: &Path, reporter: &mut dyn Reporter) {
	match ensure_imported(main_file, managed_file) {
		Ok(ImportStatus::AlreadyImported) => {}
		Ok(ImportStatus::InsertedIntoList) => reporter.info(format!(
			"Added import of {} to {}.",
			managed_file.display(),
			main_file.display(),
		)),
		Ok(ImportStatus::CreatedList) => reporter.info(format!(
			"Created imports list in {} and added {}.",
			main_file.display(),
			managed_file.display(),
		)),
		Ok(ImportStatus::MainFileMissing) => reporter.warn(format!(
			"{} not found; you must import {} manually.",
			main_file.display(),
			managed_file.display(),
		)),
		Err(e) => reporter.warn(format!(
			"could not auto-import {} into {}: {}",
			managed_file.display(),
			main_file.display(),
			e,
		)),
	}
}

/// Runs `git add --intent-to-add` if `path` lives inside a git work tree.
/// Silent no-op when git isn't installed or the path isn't inside one. Nix
/// flakes refuse to see files that are present on disk but untracked, so this
/// keeps a freshly written managed file visible to the rebuild.
pub(crate) fn git_track(path: &Path, reporter: &mut dyn Reporter) {
	let Some(parent) = path.parent() else {
		return;
	};
	let inside = Command::new("git")
		.arg("-C")
		.arg(parent)
		.args(["rev-parse", "--is-inside-work-tree"])
		.stdout(Stdio::piped())
		.stderr(Stdio::null())
		.output();
	let Ok(inside) = inside else {
		return;
	};
	if !inside.status.success() || std::str::from_utf8(&inside.stdout).map(str::trim) != Ok("true")
	{
		return;
	}

	let added = Command::new("git")
		.arg("-C")
		.arg(parent)
		.args(["add", "--intent-to-add", "--"])
		.arg(path)
		.stdout(Stdio::null())
		.stderr(Stdio::piped())
		.output();
	let Ok(added) = added else {
		return;
	};
	if added.status.success() {
		reporter.info(format!("git add -N {}", path.display()));
	} else {
		reporter.warn(format!(
			"`git add` failed for {}: {}",
			path.display(),
			String::from_utf8_lossy(&added.stderr).trim()
		));
	}
}

#[cfg(test)]
mod tests {
	use super::{Engine, ManagedPackage, scan_target, scope_matches};
	use crate::op::Op;
	use crate::report::{LogReporter, SilentReporter};
	use nixbox_config::{Config, Target};
	use nixbox_nix::{Manifest, scan::ScanTarget};

	#[test]
	fn startup_upgrades_both_legacy_modules_without_a_package_operation() {
		let dir = crate::tests::temp_dir("startup-merge-flakes");
		let config = dir.config();
		for scope in [Target::HomeManager, Target::NixosSystem] {
			let managed = nixbox_nix::ManagedFile::new(config.managed_file_for(scope));
			let mut manifest = Manifest::default();
			manifest.add("fd");
			match scope {
				Target::HomeManager => managed.write_home_manager(&manifest).unwrap(),
				Target::NixosSystem => managed.write_nixos(&manifest).unwrap(),
			}
			let old = config.legacy_flake_manifest_for(scope);
			let option = match scope {
				Target::HomeManager => "home.packages",
				Target::NixosSystem => "environment.systemPackages",
			};
			std::fs::write(&old, format!(
				"{{ inputs, pkgs, ... }}:\n{{\n\t{option} = [\n\t\t# nixbox:flake-packages:start\n\t\tinputs.tools.packages.${{pkgs.system}}.default # github:owner/tools\n\t\t# nixbox:flake-packages:end\n\t];\n}}\n"
			)).unwrap();
			std::fs::write(
				config.main_file_for(scope),
				format!(
					"{{ ... }}: {{ imports = [ ./{} ./{} ]; }}\n",
					managed.path().file_name().unwrap().to_str().unwrap(),
					old.file_name().unwrap().to_str().unwrap(),
				),
			)
			.unwrap();
		}
		let engine = Engine::with_config(config.clone()).unwrap();
		assert_eq!(engine.managed_packages().len(), 2);
		assert_eq!(engine.flakes.len(), 2);
		for scope in [Target::HomeManager, Target::NixosSystem] {
			assert!(!config.legacy_flake_manifest_for(scope).exists());
			let source = std::fs::read_to_string(config.main_file_for(scope)).unwrap();
			assert!(!source.contains("-flakes.nix"));
			assert_eq!(source.matches("-packages.nix").count(), 1);
		}
		assert_eq!(Engine::with_config(config).unwrap().flakes, engine.flakes);
	}

	fn engine() -> Engine {
		Engine::from_parts(
			Config::default(),
			Manifest::default(),
			Manifest::default(),
			Vec::new(),
		)
	}

	#[test]
	fn scan_targets_and_scopes_line_up() {
		assert_eq!(scan_target(Target::HomeManager), ScanTarget::HomeManager);
		assert_eq!(scan_target(Target::NixosSystem), ScanTarget::Nixos);
		assert!(scope_matches(ScanTarget::Nixos, Target::NixosSystem));
		assert!(!scope_matches(ScanTarget::Nixos, Target::HomeManager));
	}

	#[test]
	fn managed_packages_lists_home_entries_before_system_entries() {
		let mut engine = engine();
		engine.manifest_for_mut(Target::NixosSystem).add("fd");
		engine.manifest_for_mut(Target::HomeManager).add("ripgrep");

		assert_eq!(
			engine.managed_packages(),
			vec![
				ManagedPackage {
					name: "ripgrep".into(),
					scope: Target::HomeManager,
				},
				ManagedPackage {
					name: "fd".into(),
					scope: Target::NixosSystem,
				},
			]
		);
	}

	#[test]
	fn tracking_is_scoped() {
		let mut engine = engine();
		engine.manifest_for_mut(Target::HomeManager).add("ripgrep");

		assert!(engine.is_tracked("ripgrep", Target::HomeManager));
		assert!(!engine.is_tracked("ripgrep", Target::NixosSystem));
	}

	#[test]
	fn applying_an_install_writes_the_managed_file_and_reports_it() {
		let dir = crate::tests::temp_dir("apply-install");
		let mut engine = Engine::from_parts(
			dir.config(),
			Manifest::default(),
			Manifest::default(),
			Vec::new(),
		);
		let mut reporter = LogReporter::new();

		engine
			.apply(
				&Op::Install {
					hit: crate::tests::hit("ripgrep"),
					scope: Target::NixosSystem,
				},
				&mut reporter,
			)
			.expect("apply install");

		assert!(engine.is_tracked("ripgrep", Target::NixosSystem));
		let written = std::fs::read_to_string(engine.config.managed_file_for(Target::NixosSystem))
			.expect("managed file written");
		assert!(written.contains("ripgrep"));
		assert!(
			reporter
				.lines()
				.iter()
				.any(|line| line.contains("install ripgrep [nixos]"))
		);
		drop(dir);
	}

	#[test]
	fn uninstall_drops_the_package_from_the_managed_file() {
		let dir = crate::tests::temp_dir("apply-uninstall");
		let mut engine = Engine::from_parts(
			dir.config(),
			Manifest::default(),
			Manifest::default(),
			Vec::new(),
		);
		engine.manifest_for_mut(Target::NixosSystem).add("ripgrep");
		engine.manifest_for_mut(Target::NixosSystem).add("fd");

		engine
			.apply(
				&Op::Uninstall {
					name: "fd".into(),
					scope: Target::NixosSystem,
				},
				&mut SilentReporter,
			)
			.expect("apply uninstall");

		let written = std::fs::read_to_string(engine.config.managed_file_for(Target::NixosSystem))
			.expect("managed file written");
		assert!(written.contains("ripgrep"));
		assert!(!written.contains("fd"));
		drop(dir);
	}

	#[test]
	fn migrating_across_moves_a_declaration_into_the_other_target() {
		let dir = crate::tests::temp_dir("apply-migrate-across");
		let config = dir.config();
		let home = config.main_file_for(Target::HomeManager);
		std::fs::write(
			&home,
			"{ pkgs, ... }:\n{\n  home.packages = with pkgs; [\n    ripgrep\n    fira-code\n  ];\n}\n",
		)
		.expect("write home.nix");
		let system = config.main_file_for(Target::NixosSystem);
		std::fs::write(&system, "{ pkgs, ... }:\n{\n}\n").expect("write configuration.nix");
		let externals = super::scan_externals(&config, &Manifest::default(), &Manifest::default());
		let mut engine =
			Engine::from_parts(config, Manifest::default(), Manifest::default(), externals);

		engine
			.apply(
				&Op::MigrateAcross {
					names: vec!["fira-code".into()],
					from: Target::HomeManager,
					scope: Target::NixosSystem,
				},
				&mut SilentReporter,
			)
			.expect("apply migrate across");

		assert_eq!(
			engine.import_state(Target::NixosSystem),
			super::ImportState::Imported
		);
		assert!(engine.is_tracked("fira-code", Target::NixosSystem));
		assert!(!engine.is_tracked("fira-code", Target::HomeManager));
		let source = std::fs::read_to_string(&home).expect("read home.nix");
		assert!(source.contains("ripgrep"));
		assert!(!source.contains("fira-code"));
		let names: Vec<_> = engine
			.external_packages
			.iter()
			.map(|ep| ep.name.as_str())
			.collect();
		assert_eq!(names, ["ripgrep"]);
		drop(dir);
	}

	#[test]
	fn migrating_across_to_a_missing_target_keeps_the_declaration() {
		let dir = crate::tests::temp_dir("apply-migrate-across-missing");
		let config = dir.config();
		let home = config.main_file_for(Target::HomeManager);
		let source = "{ pkgs, ... }:\n{\n  home.packages = with pkgs; [\n    fira-code\n  ];\n}\n";
		std::fs::write(&home, source).expect("write home.nix");
		let mut engine =
			Engine::from_parts(config, Manifest::default(), Manifest::default(), Vec::new());

		let result = engine.apply(
			&Op::MigrateAcross {
				names: vec!["fira-code".into()],
				from: Target::HomeManager,
				scope: Target::NixosSystem,
			},
			&mut SilentReporter,
		);

		assert!(result.is_err());
		assert!(!engine.is_tracked("fira-code", Target::NixosSystem));
		assert_eq!(
			std::fs::read_to_string(&home).expect("read home.nix"),
			source
		);
		drop(dir);
	}
}

#[cfg(test)]
mod import_state_tests {
	use super::{Engine, ImportState};
	use nixbox_config::Target;
	use nixbox_nix::Manifest;

	fn engine(dir: &crate::tests::TempConfigDir) -> Engine {
		Engine::from_parts(
			dir.config(),
			Manifest::default(),
			Manifest::default(),
			Vec::new(),
		)
	}

	#[test]
	fn a_missing_main_file_is_reported_as_such() {
		let dir = crate::tests::temp_dir("import-missing");
		let engine = engine(&dir);

		assert_eq!(
			engine.import_state(Target::NixosSystem),
			ImportState::MainFileMissing
		);
		drop(dir);
	}

	#[test]
	fn an_import_line_is_detected_and_its_absence_is_not() {
		let dir = crate::tests::temp_dir("import-present");
		let engine = engine(&dir);
		let main_file = engine.config.main_file_for(Target::NixosSystem);
		std::fs::write(&main_file, "{ imports = [ ./unrelated.nix ]; }\n")
			.expect("write main file");

		assert_eq!(
			engine.import_state(Target::NixosSystem),
			ImportState::NotImported
		);

		std::fs::write(
			&main_file,
			"{ imports = [ ./nixbox-system-packages.nix ]; }\n",
		)
		.expect("write main file");

		assert_eq!(
			engine.import_state(Target::NixosSystem),
			ImportState::Imported
		);
		drop(dir);
	}
}
