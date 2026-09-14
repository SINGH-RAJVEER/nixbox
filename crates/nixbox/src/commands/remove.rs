//! `nixbox remove` — stop tracking a package and rebuild.

use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Args;
use nixbox_core::{Op, scope_matches};

use crate::apply::{ApplyOpts, Plan, execute};
use crate::cli::GlobalArgs;
use crate::commands::target_name;

#[derive(Args, Debug)]
pub struct RemoveArgs {
    /// Packages to stop managing.
    #[arg(required = true, num_args = 1.., value_name = "PACKAGE")]
    pub packages: Vec<String>,

    #[command(flatten)]
    pub apply: ApplyOpts,
}

pub async fn run(args: &RemoveArgs, global: &GlobalArgs) -> Result<ExitCode> {
    let mut engine = global.engine()?;
    let scope = engine.config.target;

    let mut ops = Vec::new();
    let mut summary = Vec::new();
    let mut problems = Vec::new();
    for name in &args.packages {
        if engine.is_tracked(name, scope) {
            summary.push(format!("remove {name} [{}]", scope.tag()));
            ops.push(Op::Uninstall {
                name: name.clone(),
                scope,
            });
            continue;
        }
        // Not ours to remove: say which, so the message points somewhere.
        if let Some(external) = engine
            .external_packages
            .iter()
            .find(|ep| &ep.name == name && scope_matches(ep.scope, scope))
        {
            problems.push(format!(
                "{name} is declared in {} at {}:{}, not by nixbox — remove it there, or run \
                 `nixbox migrate {name}` first",
                external.source_attr,
                engine.config.main_file_for(scope).display(),
                external.line.saturating_add(1),
            ));
        } else {
            problems.push(format!("{name} is not managed for {}", target_name(scope)));
        }
    }

    if !problems.is_empty() {
        bail!("{}", problems.join("\n       "));
    }
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
