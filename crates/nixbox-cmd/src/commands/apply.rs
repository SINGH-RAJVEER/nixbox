//! `nixbox apply` — rewrite the managed file and rebuild, changing nothing.
//!
//! This is what you run after `--no-rebuild`, after a rebuild that failed, or
//! when something else edited the generated file.

use std::process::ExitCode;

use anyhow::Result;
use clap::Args;

use crate::apply::{ApplyOpts, Plan, execute};
use crate::cli::GlobalArgs;
use crate::commands::target_name;

#[derive(Args, Debug)]
pub struct ApplyArgs {
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
            "rewrite {} and rebuild {}",
            engine.config.managed_file_for(scope).display(),
            target_name(scope),
        )],
    };
    execute(&mut engine, plan, &args.apply).await
}
