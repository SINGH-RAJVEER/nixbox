//! `nixbox install` — track a package and rebuild.

use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Args;
use nixbox_core::{Engine, Op, scope_matches};
use nixbox_nix::search::SearchHit;

use crate::apply::{ApplyOpts, Plan, execute};
use crate::cli::GlobalArgs;
use crate::commands::{search_packages, target_name};

#[derive(Args, Debug)]
pub struct InstallArgs {
    /// Packages to install, by attribute (`ripgrep`, `python3Packages.rich`)
    /// or by name.
    #[arg(required = true, num_args = 1.., value_name = "PACKAGE")]
    pub packages: Vec<String>,

    #[command(flatten)]
    pub apply: ApplyOpts,
}

pub async fn run(args: &InstallArgs, global: &GlobalArgs) -> Result<ExitCode> {
    let mut engine = global.engine()?;
    let scope = engine.config.target;

    let mut ops = Vec::new();
    let mut summary = Vec::new();
    for name in &args.packages {
        let hit = resolve(&engine, global, name).await?;
        if engine.is_tracked(&hit.attr, scope) {
            eprintln!(
                "{} is already managed for {}.",
                hit.attr,
                target_name(scope)
            );
            continue;
        }
        if let Some(external) = engine
            .external_packages
            .iter()
            .find(|ep| ep.name == hit.attr && scope_matches(ep.scope, scope))
        {
            eprintln!(
                "Note: {} is already declared in {}; `nixbox migrate {}` moves it instead of \
                 declaring it twice.",
                hit.attr, external.source_attr, hit.attr
            );
        }
        summary.push(format!(
            "install {} {} [{}]",
            hit.attr,
            hit.version,
            scope.tag()
        ));
        ops.push(Op::Install { hit, scope });
    }

    if ops.is_empty() {
        eprintln!("Nothing to install.");
        return Ok(ExitCode::SUCCESS);
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

/// Turns what the user typed into a concrete package.
///
/// An exact attribute wins outright; otherwise a single package whose name
/// matches is accepted, and anything ambiguous is handed back to the user
/// rather than guessed at.
async fn resolve(engine: &Engine, global: &GlobalArgs, name: &str) -> Result<SearchHit> {
    let channel = &engine.config.channel;
    let hits = search_packages(engine, global, name).await?;

    if let Some(exact) = hits.iter().find(|hit| hit.attr == name) {
        return Ok(exact.clone());
    }
    let by_name: Vec<&SearchHit> = hits.iter().filter(|hit| hit.pname == name).collect();
    match by_name.as_slice() {
        [only] => Ok((*only).clone()),
        [] => bail!("no package in {channel} matches `{name}`. Try `nixbox search {name}`."),
        many => {
            let attrs: Vec<&str> = many.iter().take(5).map(|hit| hit.attr.as_str()).collect();
            bail!(
                "`{name}` matches several packages ({}). Install one by its attribute.",
                attrs.join(", ")
            )
        }
    }
}
