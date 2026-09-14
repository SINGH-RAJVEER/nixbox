//! `nixbox migrate` — take over a package the user declared by hand.

use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Args;
use nixbox_core::{Op, scope_matches};

use crate::apply::{ApplyOpts, Plan, execute};
use crate::cli::GlobalArgs;
use crate::commands::target_name;

#[derive(Args, Debug)]
pub struct MigrateArgs {
    /// Packages to move into the managed file.
    #[arg(
        required_unless_present = "all",
        conflicts_with = "all",
        num_args = 1..,
        value_name = "PACKAGE"
    )]
    pub packages: Vec<String>,

    /// Migrate every package that can be moved cleanly.
    #[arg(long, short = 'a')]
    pub all: bool,

    #[command(flatten)]
    pub apply: ApplyOpts,
}

pub async fn run(args: &MigrateArgs, global: &GlobalArgs) -> Result<ExitCode> {
    let mut engine = global.engine()?;
    let scope = engine.config.target;

    let in_scope: Vec<_> = engine
        .external_packages
        .iter()
        .filter(|ep| scope_matches(ep.scope, scope))
        .cloned()
        .collect();

    let names: Vec<String> = if args.all {
        let (migratable, stuck): (Vec<_>, Vec<_>) = in_scope.iter().partition(|ep| ep.migratable);
        for ep in stuck {
            eprintln!(
                "Skipping {}: it shares a line in {}, so nixbox will not rewrite it.",
                ep.name, ep.source_attr
            );
        }
        migratable.iter().map(|ep| ep.name.clone()).collect()
    } else {
        let mut chosen = Vec::new();
        let mut problems = Vec::new();
        for name in &args.packages {
            match in_scope.iter().find(|ep| &ep.name == name) {
                Some(ep) if ep.migratable => chosen.push(name.clone()),
                Some(ep) => problems.push(format!(
                    "{name} shares a line in {}; move it by hand",
                    ep.source_attr
                )),
                None if engine.is_tracked(name, scope) => {
                    problems.push(format!("{name} is already managed by nixbox"));
                }
                None => problems.push(format!(
                    "{name} is not declared in your {} config",
                    target_name(scope)
                )),
            }
        }
        if !problems.is_empty() {
            bail!("{}", problems.join("\n       "));
        }
        chosen
    };

    if names.is_empty() {
        eprintln!("Nothing to migrate for {}.", target_name(scope));
        return Ok(ExitCode::SUCCESS);
    }

    let summary = names
        .iter()
        .map(|name| format!("migrate {name} [{}]", scope.tag()))
        .collect();
    let ops = vec![Op::Migrate { names, scope }];
    execute(
        &mut engine,
        Plan {
            scope,
            ops,
            summary,
        },
        &args.apply,
    )
    .await
}
