//! `nixbox profile` — move a target's packages into an independent profile
//! and back, and inspect or roll back that profile's generations.

use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Subcommand;
use nixbox_config::Target;
use nixbox_core::profile::{self, SetupStatus};
use nixbox_core::rebuild;
use nixbox_core::{Op, RebuildCommand};
use nixbox_nix::{PackageMode, flake_has_home_configuration};

use crate::apply::{ApplyOpts, Plan, execute};
use crate::cli::{EXIT_FAILURE, GlobalArgs};
use crate::commands::target_name;

#[derive(Subcommand, Debug)]
pub enum Action {
	/// Move the target's packages into an independent profile, then rebuild
	/// once. Later package changes only update the profile.
	Setup {
		#[command(flatten)]
		apply: ApplyOpts,
	},
	/// Move the target's packages back into the configuration, then rebuild.
	Teardown {
		#[command(flatten)]
		apply: ApplyOpts,
	},
	/// List the profile's generations.
	Generations,
	/// Point the profile at its previous generation, or at `--to`.
	Rollback {
		/// The generation to switch to.
		#[arg(long, value_name = "GENERATION")]
		to: Option<u32>,
	},
}

pub async fn run(action: &Action, global: &GlobalArgs) -> Result<ExitCode> {
	match action {
		Action::Setup { apply } => setup(apply, global).await,
		Action::Teardown { apply } => teardown(apply, global).await,
		Action::Generations => {
			let scope = existing_profile(global)?;
			spawn(profile::generations_command(scope)?).await
		}
		Action::Rollback { to } => {
			if rebuild::builds_vm() {
				bail!(
					"{} is set, so rebuilds go to a VM; rolling back would change this machine's profile",
					rebuild::VM_BUILD_ENV
				);
			}
			let scope = existing_profile(global)?;
			let code = spawn(profile::rollback_command(scope, *to)?).await?;
			if code == ExitCode::SUCCESS {
				eprintln!(
					"Rolled back the {} package profile. Its packages are still listed in {}, so the next install, `apply`, rebuild or reboot restores them; remove them there to make this permanent.",
					scope.label(),
					global.engine()?.config.managed_file_for(scope).display()
				);
			}
			Ok(code)
		}
	}
}

async fn setup(apply: &ApplyOpts, global: &GlobalArgs) -> Result<ExitCode> {
	let mut engine = global.engine()?;
	let scope = engine.config.target;
	let path = profile::profile_path(scope)?;
	let home_inside_nixos = scope == Target::HomeManager
		&& !flake_has_home_configuration(&engine.config.home_manager_dir()).await;
	let mut summary = vec![format!(
		"move the {} packages into the profile at {}",
		scope.label(),
		path.display()
	)];
	if home_inside_nixos {
		summary.push(format!(
			"put the profile on PATH for every shell through {}",
			engine.config.home_integration_file().display()
		));
	}
	summary.push(format!(
		"rebuild {} once; later package changes only update the profile",
		target_name(scope)
	));
	let plan = Plan {
		scope,
		ops: vec![Op::SetupProfile {
			scope,
			home_inside_nixos,
		}],
		summary,
	};
	let code = execute(&mut engine, plan, apply, &global.telemetry).await?;
	if code != ExitCode::SUCCESS || apply.dry_run || apply.no_rebuild || rebuild::builds_vm() {
		return Ok(code);
	}
	let status = profile::setup_status(&engine, scope);
	eprintln!("Profile setup: {}.", status.describe(scope));
	if status == SetupStatus::NotActivated {
		eprintln!(
			"The rebuild finished without activating the profile. Check that {} is imported by the configuration that was rebuilt.",
			engine.config.managed_file_for(scope).display()
		);
		return Ok(ExitCode::from(EXIT_FAILURE));
	}
	Ok(code)
}

async fn teardown(apply: &ApplyOpts, global: &GlobalArgs) -> Result<ExitCode> {
	let mut engine = global.engine()?;
	let scope = engine.config.target;
	if engine.manifest_for(scope).mode == PackageMode::Configuration {
		bail!(
			"{} packages already install through the configuration",
			scope.label()
		);
	}
	let path = profile::profile_path(scope)?;
	let plan = Plan {
		scope,
		ops: vec![Op::TeardownProfile { scope }],
		summary: vec![
			format!(
				"move the {} packages back into the configuration",
				scope.label()
			),
			format!(
				"rebuild {}; the profile at {} stays on disk until you delete it",
				target_name(scope),
				path.display()
			),
		],
	};
	execute(&mut engine, plan, apply, &global.telemetry).await
}

/// The selected target, provided its profile has been built at least once.
fn existing_profile(global: &GlobalArgs) -> Result<Target> {
	let scope = global.engine()?.config.target;
	let path = profile::profile_path(scope)?;
	if std::fs::symlink_metadata(&path).is_err() {
		bail!(
			"there is no {} package profile at {}; run `{} --target {} profile setup` first",
			scope.label(),
			path.display(),
			crate::program(),
			scope.label()
		);
	}
	Ok(scope)
}

/// Runs `command` in the foreground, so sudo can prompt on the terminal.
async fn spawn(command: RebuildCommand) -> Result<ExitCode> {
	let status = tokio::process::Command::new(&command.program)
		.args(&command.args)
		.status()
		.await?;
	Ok(if status.success() {
		ExitCode::SUCCESS
	} else {
		ExitCode::from(EXIT_FAILURE)
	})
}
