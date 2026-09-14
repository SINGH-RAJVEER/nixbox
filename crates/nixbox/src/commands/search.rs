//! `nixbox search` — query a nixpkgs channel.

use std::process::ExitCode;

use anyhow::Result;
use clap::Args;
use nixbox_nix::search::search;
use serde::Serialize;

use crate::cli::GlobalArgs;
use crate::commands::package_status;
use crate::render;

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Words to match. Several words are joined with a space.
    #[arg(required = true, num_args = 1.., value_name = "QUERY")]
    pub query: Vec<String>,

    /// Show at most this many results.
    #[arg(long, short = 'n', default_value_t = 30, value_name = "COUNT")]
    pub limit: usize,
}

#[derive(Serialize)]
struct Row {
    attr: String,
    pname: String,
    version: String,
    description: String,
    /// `managed`, `external`, or `-`, relative to the active target.
    status: &'static str,
}

pub async fn run(args: &SearchArgs, global: &GlobalArgs) -> Result<ExitCode> {
    let engine = global.engine()?;
    let query = args.query.join(" ");
    let hits = search(&engine.config.channel, &query).await?;

    let rows: Vec<Row> = hits
        .into_iter()
        .take(args.limit)
        .map(|hit| Row {
            status: package_status(&engine, &hit.attr),
            attr: hit.attr,
            pname: hit.pname,
            version: hit.version,
            description: hit.description,
        })
        .collect();

    if global.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(ExitCode::SUCCESS);
    }

    if rows.is_empty() {
        eprintln!("No packages in {} matched {query}.", engine.config.channel);
        return Ok(ExitCode::SUCCESS);
    }

    let table: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            vec![
                row.attr.clone(),
                row.version.clone(),
                row.status.to_string(),
                render::truncate(&row.description, 60),
            ]
        })
        .collect();
    print!(
        "{}",
        render::table(&["NAME", "VERSION", "STATUS", "DESCRIPTION"], &table)
    );
    Ok(ExitCode::SUCCESS)
}
