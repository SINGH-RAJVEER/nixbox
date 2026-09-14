//! `nixbox scan` — packages declared by hand in the user's own config.

use std::process::ExitCode;

use anyhow::Result;
use nixbox_nix::scan::ScanTarget;
use serde::Serialize;

use crate::cli::GlobalArgs;
use crate::render;

#[derive(Serialize)]
struct Row {
    name: String,
    target: &'static str,
    /// The attribute the package was found in, e.g. `home.packages`.
    attribute: String,
    /// 1-indexed line in the main config file.
    line: usize,
    /// False for same-line list entries, which nixbox will not rewrite.
    migratable: bool,
}

pub fn run(global: &GlobalArgs) -> Result<ExitCode> {
    let engine = global.engine()?;

    let rows: Vec<Row> = engine
        .external_packages
        .iter()
        .map(|ep| Row {
            name: ep.name.clone(),
            target: match ep.scope {
                ScanTarget::HomeManager => "home-manager",
                ScanTarget::Nixos => "nixos",
            },
            attribute: ep.source_attr.clone(),
            line: ep.line.saturating_add(1),
            migratable: ep.migratable,
        })
        .collect();

    if global.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(ExitCode::SUCCESS);
    }

    if rows.is_empty() {
        eprintln!("No externally-declared packages found in your config.");
        return Ok(ExitCode::SUCCESS);
    }

    let table: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            vec![
                row.name.clone(),
                row.target.to_string(),
                row.attribute.clone(),
                row.line.to_string(),
                if row.migratable { "yes" } else { "no" }.to_string(),
            ]
        })
        .collect();
    print!(
        "{}",
        render::table(
            &["NAME", "TARGET", "ATTRIBUTE", "LINE", "MIGRATABLE"],
            &table
        )
    );
    Ok(ExitCode::SUCCESS)
}
