//! Picking the rebuild command for a target, and running it.
//!
//! `resolve` evaluates the user's flake, which can take a while, so [`run_activation`]
//! races it against the caller's cancel signal as well as the rebuild itself.

use std::path::{Path, PathBuf};

use nixbox_config::{Config, Target};
use nixbox_nix::build::{
	BuildEvent, NIXOS_HOST, flake_has_home_configuration, home_manager_switch_cmd,
	nixos_rebuild_switch_cmd, rebuild,
};
use nixbox_nix::nix_log::STRUCTURED_ARGS;
use tokio::sync::{mpsc, oneshot};

use crate::profile::{self, Activation};

/// Emitted when a home-manager change has to be applied through
/// `nixos-rebuild` because there is no standalone home configuration.
pub const HOME_FALLBACK_NOTE: &str =
	"No standalone homeConfigurations found; applying via nixos-rebuild.";

/// Set to `1` in the environment of a rebuild that uses
/// [`Escalation::Askpass`].
pub const ASKPASS_ENV: &str = "NIXBOX_ASKPASS";

/// Carries `LD_LIBRARY_PATH` to the askpass helper. sudo is setuid, so the
/// loader strips every `LD_` variable from its environment before the helper
/// inherits it, and a helper that loads its graphics libraries at runtime
/// needs the path back.
pub const ASKPASS_LIBRARY_PATH_ENV: &str = "NIXBOX_LD_LIBRARY_PATH";

/// When set, build a VM runner at this output link instead of switching NixOS.
pub const VM_BUILD_ENV: &str = "NIXBOX_VM_BUILD";

/// How a rebuild reports its output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LogFormat {
	/// Nix's own plain output, for a terminal that shows it as it comes.
	#[default]
	Plain,
	/// Nix's structured log, which [`crate::progress::BuildProgress`] reads.
	Structured,
}

/// True when [`VM_BUILD_ENV`] sends rebuilds to a VM. Package profiles live on
/// the host, so nothing may build or roll them back while it is set.
#[must_use]
pub fn builds_vm() -> bool {
	std::env::var_os(VM_BUILD_ENV).is_some()
}

/// How a rebuild that needs root gets it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Escalation {
	/// Plain `sudo`, which prompts on the controlling terminal. Right for
	/// the TUI and the CLI, which always have one.
	#[default]
	Terminal,
	/// `sudo -A` with `SUDO_ASKPASS` pointing at this program, for a
	/// front-end started without a terminal. sudo stays the parent of the
	/// rebuild, so cancelling can still signal the whole process group.
	///
	/// sudo runs the helper with the prompt as its only argument and the
	/// rebuild's environment, so [`ASKPASS_ENV`] is set there too: it is how
	/// the helper knows to ask for a password instead of starting normally.
	Askpass(PathBuf),
}

/// A resolved rebuild invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildCommand {
	pub program: String,
	pub args: Vec<String>,
	/// Added to the inherited environment when the command is spawned.
	pub envs: Vec<(String, String)>,
	/// True when a home-manager rebuild fell back to `nixos-rebuild` because
	/// the flake exposes no `homeConfigurations.<user>` output — i.e. the user
	/// wires home-manager in as a NixOS module.
	pub via_nixos_fallback: bool,
}

impl RebuildCommand {
	/// Borrowed args, in the shape [`nixbox_nix::build::rebuild`] wants.
	#[must_use]
	pub fn arg_refs(&self) -> Vec<&str> {
		self.args.iter().map(String::as_str).collect()
	}

	/// Switches a `sudo` invocation over to `escalation`. Anything that does
	/// not go through sudo is left alone.
	#[must_use]
	pub fn escalated(mut self, escalation: &Escalation) -> Self {
		if let Escalation::Askpass(helper) = escalation
			&& self.program == "sudo"
		{
			self.args.insert(0, "-A".into());
			self.envs
				.push(("SUDO_ASKPASS".into(), helper.display().to_string()));
			self.envs.push((ASKPASS_ENV.into(), "1".into()));
			if let Ok(path) = std::env::var("LD_LIBRARY_PATH") {
				self.envs.push((ASKPASS_LIBRARY_PATH_ENV.into(), path));
			}
		}
		self
	}

	/// Asks for `format`. Every command a rebuild runs is nix or a wrapper
	/// that forwards trailing nix flags, so the flags go at the end.
	#[must_use]
	pub fn with_log(mut self, format: LogFormat) -> Self {
		if format == LogFormat::Structured {
			self.args
				.extend(STRUCTURED_ARGS.iter().map(|arg| (*arg).to_string()));
		}
		self
	}

	/// The command as a user could retype it.
	#[must_use]
	pub fn display(&self) -> String {
		if self.args.is_empty() {
			self.program.clone()
		} else {
			format!("{} {}", self.program, self.args.join(" "))
		}
	}
}

/// Returns the command that applies pending changes for `scope`.
///
/// For [`Target::HomeManager`] this evaluates the flake to decide between
/// `home-manager switch` and `nixos-rebuild switch`.
/// [`VM_BUILD_ENV`] routes both targets to a VM build without activation.
pub async fn resolve(config_dir: &Path, scope: Target) -> RebuildCommand {
	if builds_vm() {
		return nixos(config_dir);
	}
	match scope {
		Target::NixosSystem => nixos(config_dir),
		Target::HomeManager => {
			if flake_has_home_configuration(config_dir).await {
				let (program, args) = home_manager_switch_cmd(config_dir);
				RebuildCommand {
					program,
					args,
					envs: Vec::new(),
					via_nixos_fallback: false,
				}
			} else {
				RebuildCommand {
					via_nixos_fallback: true,
					..nixos(config_dir)
				}
			}
		}
	}
}

/// Resolve a command using the selection frozen into the engine/session.
pub async fn resolve_for(config: &Config, scope: Target) -> anyhow::Result<RebuildCommand> {
	if let Some(layout) = nixbox_nix::dendritic::Layout::select_output(
		&config.config_root(),
		config.nixos_host.as_deref(),
		config.home_user.as_deref(),
		config.home_configuration.as_deref(),
	)? {
		if scope == Target::HomeManager {
			layout.require_home()?;
		}
		let root = config.config_root();
		let host = nixbox_nix::wiring::attr_name(&layout.host);
		if layout.standalone {
			anyhow::ensure!(
				scope == Target::HomeManager,
				"standalone Home Manager has no NixOS scope"
			);
			if let Some(output) = std::env::var_os(VM_BUILD_ENV) {
				return Ok(RebuildCommand {
					program: "nix".into(),
					args: vec![
						"build".into(),
						"--out-link".into(),
						output.to_string_lossy().into_owned(),
						format!(
							"{}#homeConfigurations.{host}.activationPackage",
							root.display()
						),
					],
					envs: Vec::new(),
					via_nixos_fallback: false,
				});
			}
			return Ok(RebuildCommand {
				program: "home-manager".into(),
				args: vec![
					"switch".into(),
					"--flake".into(),
					format!("{}#{host}", root.display()),
				],
				envs: Vec::new(),
				via_nixos_fallback: false,
			});
		}
		if let Some(output) = std::env::var_os(VM_BUILD_ENV) {
			return Ok(RebuildCommand {
				program: "nix".into(),
				args: vec![
					"build".into(),
					"--print-build-logs".into(),
					"--out-link".into(),
					output.to_string_lossy().into_owned(),
					format!(
						"{}#nixosConfigurations.{host}.config.system.build.vm",
						root.display()
					),
				],
				envs: Vec::new(),
				via_nixos_fallback: false,
			});
		}
		return Ok(RebuildCommand {
			program: "sudo".into(),
			args: vec![
				"nixos-rebuild".into(),
				"switch".into(),
				"--flake".into(),
				format!("{}#{host}", root.display()),
			],
			envs: Vec::new(),
			via_nixos_fallback: scope == Target::HomeManager,
		});
	}
	Ok(resolve(&config.config_root(), scope).await)
}

/// The NixOS switch or, with [`VM_BUILD_ENV`], VM build for `config_dir`,
/// without evaluating anything.
#[must_use]
pub fn nixos(config_dir: &Path) -> RebuildCommand {
	if let Some(output) = std::env::var_os(VM_BUILD_ENV) {
		return vm_build(config_dir, Path::new(&output));
	}
	let (program, args) = nixos_rebuild_switch_cmd(config_dir);
	RebuildCommand {
		program,
		args,
		envs: Vec::new(),
		via_nixos_fallback: false,
	}
}

/// Builds the selected configuration's VM without activation or escalation.
#[must_use]
fn vm_build(config_dir: &Path, output: &Path) -> RebuildCommand {
	RebuildCommand {
		program: "nix".into(),
		args: vec![
			"build".into(),
			"--print-build-logs".into(),
			"--out-link".into(),
			output.display().to_string(),
			format!(
				"{}#nixosConfigurations.{NIXOS_HOST}.config.system.build.vm",
				config_dir.display()
			),
		],
		envs: Vec::new(),
		via_nixos_fallback: false,
	}
}

/// Runs `activation` for `scope`: a profile build, or a configuration switch,
/// which also sets a profile-mode target's profile. Sends its output and
/// exactly one closing [`BuildEvent::Finished`] or [`BuildEvent::Cancelled`]
/// to `tx`; cancellation stops whichever step is running.
///
/// Each command line goes out first as a `$ ...` line, preceded by
/// [`HOME_FALLBACK_NOTE`] when a home rebuild goes through `nixos-rebuild`.
pub async fn run_activation(
	config_dir: PathBuf,
	scope: Target,
	activation: Activation,
	escalation: Escalation,
	log: LogFormat,
	tx: mpsc::Sender<BuildEvent>,
	cancel: oneshot::Receiver<()>,
) {
	let config = Config {
		config_dir: Some(config_dir),
		..Config::default()
	};
	run_activation_for(config, scope, activation, escalation, log, tx, cancel).await;
}

pub async fn run_activation_for(
	config: Config,
	scope: Target,
	activation: Activation,
	escalation: Escalation,
	log: LogFormat,
	tx: mpsc::Sender<BuildEvent>,
	mut cancel: oneshot::Receiver<()>,
) {
	let config_dir = config.config_root();
	// The VM build carries the profile inside its own system instead.
	let activation = if builds_vm() {
		Activation::Configuration
	} else {
		activation
	};
	let commands = match activation {
		Activation::Packages => {
			if let Err(error) = profile_parent(scope) {
				let _ = tx
					.send(BuildEvent::Finished(Err(format!("{error:#}"))))
					.await;
				return;
			}
			let build = tokio::select! {
				command = profile::command_for(&config, scope) => command,
				_ = &mut cancel => {
					let _ = tx.send(BuildEvent::Cancelled).await;
					return;
				}
			};
			match build {
				Ok(build) => vec![
					profile::lock_command(&config_dir),
					build.escalated(&escalation).with_log(log),
				],
				Err(error) => {
					let _ = tx
						.send(BuildEvent::Finished(Err(format!("{error:#}"))))
						.await;
					return;
				}
			}
		}
		Activation::Configuration => {
			let command = tokio::select! {
				command = resolve_for(&config, scope) => match command {
					Ok(command) => command.escalated(&escalation).with_log(log),
					Err(error) => { let _ = tx.send(BuildEvent::Finished(Err(error.to_string()))).await; return; }
				},
				_ = &mut cancel => {
					let _ = tx.send(BuildEvent::Cancelled).await;
					return;
				}
			};
			vec![command]
		}
	};
	if !run_commands(commands, &tx, cancel).await {
		return;
	}
	let status = profile::activation_status(scope);
	if activation == Activation::Packages && status != profile::SetupStatus::Active {
		let _ = tx
			.send(BuildEvent::Line(format!(
				"Package profile updated. Profile setup: {}.",
				status.describe(scope)
			)))
			.await;
	}
	let _ = tx.send(BuildEvent::Finished(Ok(()))).await;
}

/// `nix build --profile` does not create the directory a user profile lives
/// in; the system's already exists and belongs to root.
fn profile_parent(scope: Target) -> anyhow::Result<()> {
	if scope == Target::HomeManager {
		let path = profile::profile_path(scope)?;
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)?;
		}
	}
	Ok(())
}

async fn run_commands(
	commands: Vec<RebuildCommand>,
	tx: &mpsc::Sender<BuildEvent>,
	mut cancel: oneshot::Receiver<()>,
) -> bool {
	for command in commands {
		if cancel.try_recv() != Err(oneshot::error::TryRecvError::Empty) {
			let _ = tx.send(BuildEvent::Cancelled).await;
			return false;
		}
		if command.via_nixos_fallback {
			let _ = tx.send(BuildEvent::Line(HOME_FALLBACK_NOTE.into())).await;
		}
		let _ = tx
			.send(BuildEvent::Line(format!("$ {}", command.display())))
			.await;
		let (step_tx, mut step_rx) = mpsc::channel(64);
		let (stop_tx, stop_rx) = oneshot::channel();
		let mut stop_tx = Some(stop_tx);
		let task = tokio::spawn(async move {
			if let Err(error) = rebuild(
				&command.program,
				&command.arg_refs(),
				&command.envs,
				step_tx.clone(),
				stop_rx,
			)
			.await
			{
				let _ = step_tx
					.send(BuildEvent::Finished(Err(error.to_string())))
					.await;
			}
		});
		let succeeded = loop {
			tokio::select! {
				_ = &mut cancel, if stop_tx.is_some() => {
					if let Some(stop) = stop_tx.take() {
						let _ = stop.send(());
					}
				}
				event = step_rx.recv() => match event {
					Some(event @ (BuildEvent::Line(_) | BuildEvent::Nix(_))) => { let _ = tx.send(event).await; }
					Some(BuildEvent::Finished(Ok(()))) if stop_tx.is_some() => break true,
					Some(BuildEvent::Finished(Ok(())) | BuildEvent::Cancelled) => {
						let _ = tx.send(BuildEvent::Cancelled).await;
						break false;
					}
					Some(BuildEvent::Finished(Err(error))) => {
						let _ = tx.send(BuildEvent::Finished(Err(error))).await;
						break false;
					}
					None => {
						let _ = tx.send(BuildEvent::Finished(Err("activation ended without a verdict".into()))).await;
						break false;
					}
				}
			}
		};
		let _ = task.await;
		if !succeeded {
			return false;
		}
	}
	true
}

#[cfg(test)]
mod tests {
	use super::{Escalation, LogFormat, RebuildCommand, nixos, run_commands, vm_build};
	use nixbox_nix::BuildEvent;
	use std::path::{Path, PathBuf};
	use tokio::sync::{mpsc, oneshot};

	fn shell(script: &str) -> RebuildCommand {
		RebuildCommand {
			program: "sh".into(),
			args: vec!["-c".into(), script.into()],
			envs: vec![],
			via_nixos_fallback: false,
		}
	}

	#[tokio::test]
	async fn steps_do_not_emit_intermediate_success_or_run_after_failure() {
		let (tx, mut rx) = mpsc::channel(64);
		let (_cancel, stop) = oneshot::channel();
		assert!(
			run_commands(
				vec![shell("echo package"), shell("echo configuration")],
				&tx,
				stop
			)
			.await
		);
		let mut lines = Vec::new();
		while let Ok(event) = rx.try_recv() {
			let BuildEvent::Line(line) = event else {
				panic!("an intermediate step emitted a final verdict");
			};
			lines.push(line);
		}
		assert!(lines.iter().any(|line| line == "package"));
		assert!(lines.iter().any(|line| line == "configuration"));

		let (_cancel, stop) = oneshot::channel();
		assert!(!run_commands(vec![shell("exit 1"), shell("echo must-not-run")], &tx, stop).await);
		let mut failed = 0;
		while let Ok(event) = rx.try_recv() {
			match event {
				BuildEvent::Finished(Err(_)) => failed += 1,
				BuildEvent::Line(line) => assert!(!line.contains("must-not-run")),
				_ => panic!("unexpected verdict"),
			}
		}
		assert_eq!(failed, 1);
	}

	#[tokio::test]
	async fn cancellation_stops_the_step_and_prevents_configuration_activation() {
		let (tx, mut rx) = mpsc::channel(64);
		let (cancel, stop) = oneshot::channel();
		let job = tokio::spawn(async move {
			run_commands(
				vec![
					shell("echo ready; exec sleep 30"),
					shell("echo must-not-run"),
				],
				&tx,
				stop,
			)
			.await
		});
		loop {
			if matches!(rx.recv().await, Some(BuildEvent::Line(line)) if line == "ready") {
				break;
			}
		}
		cancel.send(()).unwrap();
		assert!(!job.await.unwrap());
		assert!(matches!(rx.recv().await, Some(BuildEvent::Cancelled)));
		assert!(rx.recv().await.is_none());
	}

	#[test]
	fn nixos_command_targets_the_config_dir() {
		let cmd = nixos(Path::new("/tmp/nixos"));

		assert!(!cmd.via_nixos_fallback);
		assert!(cmd.display().contains("/tmp/nixos"));
		assert_eq!(cmd.arg_refs().len(), cmd.args.len());
	}

	#[test]
	fn vm_build_targets_the_configuration_without_sudo() {
		let cmd = vm_build(Path::new("/tmp/vm config"), Path::new("/tmp/vm result"));
		assert_eq!(cmd.program, "nix");
		assert_eq!(
			cmd.args,
			[
				"build",
				"--print-build-logs",
				"--out-link",
				"/tmp/vm result",
				"/tmp/vm config#nixosConfigurations.nixos.config.system.build.vm",
			]
		);
		assert!(!cmd.via_nixos_fallback);
		assert_eq!(
			cmd.clone()
				.escalated(&Escalation::Askpass(PathBuf::from("/bin/helper"))),
			cmd
		);
	}

	#[tokio::test]
	async fn vm_environment_routes_both_scopes_to_a_build() {
		// Nextest runs each test in its own process.
		unsafe { std::env::set_var(super::VM_BUILD_ENV, "/tmp/vm-result") };
		let expected = vm_build(Path::new("/tmp/nixos"), Path::new("/tmp/vm-result"));
		for scope in [
			nixbox_config::Target::NixosSystem,
			nixbox_config::Target::HomeManager,
		] {
			let cmd = super::resolve(Path::new("/tmp/nixos"), scope).await;
			assert_eq!(cmd, expected);
		}
	}

	#[test]
	fn only_a_structured_log_adds_flags() {
		let cmd = nixos(Path::new("/tmp/nixos"));

		assert_eq!(cmd.clone().with_log(LogFormat::Plain), cmd);
		assert!(
			cmd.with_log(LogFormat::Structured)
				.display()
				.ends_with("#nixos --log-format internal-json -v")
		);
	}

	#[test]
	fn display_handles_an_argless_command() {
		let cmd = RebuildCommand {
			program: "true".into(),
			args: Vec::new(),
			envs: Vec::new(),
			via_nixos_fallback: false,
		};

		assert_eq!(cmd.display(), "true");
	}

	#[test]
	fn askpass_escalation_makes_sudo_ask_through_the_helper() {
		let helper = PathBuf::from("/run/current-system/sw/bin/nixbox");
		let cmd = nixos(Path::new("/tmp/nixos")).escalated(&Escalation::Askpass(helper));

		assert_eq!(cmd.program, "sudo");
		assert_eq!(cmd.args[0], "-A");
		assert_eq!(cmd.args[1], "nixos-rebuild");
		assert_eq!(
			cmd.envs[..2],
			[
				(
					"SUDO_ASKPASS".to_string(),
					"/run/current-system/sw/bin/nixbox".to_string()
				),
				(super::ASKPASS_ENV.to_string(), "1".to_string()),
			]
		);
	}

	#[test]
	fn escalation_leaves_commands_without_sudo_alone() {
		let cmd = RebuildCommand {
			program: "home-manager".into(),
			args: vec!["switch".into()],
			envs: Vec::new(),
			via_nixos_fallback: false,
		};

		let escalated = cmd
			.clone()
			.escalated(&Escalation::Askpass(PathBuf::from("/bin/helper")));

		assert_eq!(escalated, cmd);
		assert_eq!(
			nixos(Path::new("/tmp"))
				.escalated(&Escalation::Terminal)
				.args[0],
			"nixos-rebuild"
		);
	}
}
