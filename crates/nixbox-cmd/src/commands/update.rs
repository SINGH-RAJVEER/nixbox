//! `nixbox update` — update every flake input and rebuild the whole system.
//!
//! Runs `nix flake update` on the configuration, switches NixOS, and then
//! switches a standalone Home Manager configuration if the flake has one.
//! Nothing nixbox manages is rewritten.

use std::process::ExitCode;

use anyhow::Result;
use clap::Args;
use nixbox_config::Target;
use nixbox_core::Activation;
use nixbox_core::session::UPDATE_LABEL;

use crate::apply::{confirm, run_activation};
use crate::cli::GlobalArgs;

#[derive(Args, Debug)]
pub struct UpdateArgs {
	/// Print what would run and exit without touching anything.
	#[arg(long)]
	pub dry_run: bool,

	/// Do not ask before updating.
	#[arg(long, short = 'y')]
	pub yes: bool,
}

pub async fn run(args: &UpdateArgs, global: &GlobalArgs) -> Result<ExitCode> {
	let engine = global.engine()?;
	let config_dir = engine.config.home_manager_dir();
	eprintln!(
		"  update every input in {}",
		config_dir.join("flake.lock").display()
	);
	let scope = if engine.config.home_configuration.is_some() {
		Target::HomeManager
	} else {
		Target::NixosSystem
	};
	eprintln!("  rebuild the selected configuration; VM mode builds without activation");

	if args.dry_run {
		eprintln!("Dry run: nothing changed. Drop --dry-run to update the system.");
		return Ok(ExitCode::SUCCESS);
	}
	if !confirm(args.yes, scope, "Update the selected configuration?")? {
		eprintln!("Cancelled.");
		return Ok(ExitCode::SUCCESS);
	}
	run_activation(
		&engine,
		scope,
		UPDATE_LABEL,
		Activation::Update,
		&global.telemetry,
	)
	.await
}
