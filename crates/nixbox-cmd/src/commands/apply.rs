//! `nixbox apply` — rewrite the managed file and activate it, changing nothing.
//!
//! This is what you run after `--no-rebuild`, after an activation that failed,
//! or when something else edited the generated file. A target with a package
//! profile only rebuilds the profile unless `--configuration` asks for a
//! switch, which also sets the profile.

use std::process::ExitCode;

use anyhow::Result;
use clap::Args;
use nixbox_core::Activation;

use crate::apply::{ApplyOpts, Plan, execute_with};
use crate::cli::GlobalArgs;
use crate::commands::target_name;

#[derive(Args, Debug)]
pub struct ApplyArgs {
	/// Switch the configuration, even when only the package profile changed.
	/// Needed after options, migrations or module changes written with
	/// `--no-rebuild`.
	#[arg(long)]
	pub configuration: bool,
	#[command(flatten)]
	pub apply: ApplyOpts,
}

pub async fn run(args: &ApplyArgs, global: &GlobalArgs) -> Result<ExitCode> {
	let mut engine = global.engine()?;
	let scope = engine.config.target;
	let plan = Plan {
		scope,
		ops: Vec::new(),
		summary: vec![format!(
			"rewrite {} and activate {}",
			engine.config.managed_file_for(scope).display(),
			target_name(scope),
		)],
	};
	let activation = if args.configuration
		|| engine.manifest_for(scope).mode == nixbox_nix::PackageMode::Configuration
	{
		Activation::Configuration
	} else {
		Activation::Packages
	};
	execute_with(
		&mut engine,
		plan,
		&args.apply,
		Some(activation),
		&global.telemetry,
	)
	.await
}
