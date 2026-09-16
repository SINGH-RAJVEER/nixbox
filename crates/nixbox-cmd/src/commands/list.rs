//! `nixbox list` — the packages nixbox tracks.

use std::process::ExitCode;

use anyhow::Result;
use clap::Args;
use serde::Serialize;

use crate::cli::GlobalArgs;
use crate::commands::target_name;
use crate::render;

#[derive(Args, Debug)]
pub struct ListArgs {
    /// List both targets instead of only the active one.
    #[arg(long, short = 'a')]
    pub all: bool,
}

#[derive(Serialize)]
struct Row {
    name: String,
    target: &'static str,
}

pub fn run(args: &ListArgs, global: &GlobalArgs) -> Result<ExitCode> {
    let engine = global.engine()?;
    let active = engine.config.target;

    let rows: Vec<Row> = engine
        .managed_packages()
        .into_iter()
        .filter(|pkg| args.all || pkg.scope == active)
        .map(|pkg| Row {
            name: pkg.name,
            target: target_name(pkg.scope),
        })
        .collect();

    if global.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(ExitCode::SUCCESS);
    }

    if rows.is_empty() {
        if args.all {
            eprintln!("nixbox is not managing any packages yet.");
        } else {
            eprintln!(
                "nixbox is not managing any {} packages yet. Pass --all to check both targets.",
                target_name(active)
            );
        }
        return Ok(ExitCode::SUCCESS);
    }

    let table: Vec<Vec<String>> = rows
        .iter()
        .map(|row| vec![row.name.clone(), row.target.to_string()])
        .collect();
    print!("{}", render::table(&["NAME", "TARGET"], &table));
    Ok(ExitCode::SUCCESS)
}
