//! `nixbox status` — what nixbox is pointed at and what it owns.

use std::path::Path;
use std::process::ExitCode;

use anyhow::Result;
use nixbox_config::{Target, settings_path};
use nixbox_core::{Engine, ImportState};
use serde_json::json;

use crate::cli::GlobalArgs;
use crate::commands::target_name;
use crate::render;

const TARGETS: [Target; 2] = [Target::HomeManager, Target::NixosSystem];

fn import_label(state: ImportState) -> &'static str {
    match state {
        ImportState::Imported => "yes",
        ImportState::NotImported => "no",
        ImportState::MainFileMissing => "no main config file",
    }
}

fn exists_suffix(path: &Path) -> &'static str {
    if path.exists() { "" } else { " (missing)" }
}

pub fn run(global: &GlobalArgs) -> Result<ExitCode> {
    let engine = global.engine()?;
    let settings = settings_path()?;

    if global.json {
        let targets: Vec<_> = TARGETS
            .iter()
            .map(|&target| {
                let managed = engine.config.managed_file_for(target);
                let main = engine.config.main_file_for(target);
                json!({
                    "target": target_name(target),
                    "active": target == engine.config.target,
                    "packages": engine.manifest_for(target).packages.len(),
                    "managed_file": managed,
                    "managed_file_exists": managed.exists(),
                    "main_file": main,
                    "main_file_exists": main.exists(),
                    "imported": import_label(engine.import_state(target)),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "target": target_name(engine.config.target),
                "channel": engine.config.channel,
                "config_dir": engine.config.home_manager_dir(),
                "settings_file": settings,
                "external_packages": engine.external_packages.len(),
                "targets": targets,
            }))?
        );
        return Ok(ExitCode::SUCCESS);
    }

    print!(
        "{}",
        render::fields(&[
            ("target", target_name(engine.config.target).to_string()),
            ("channel", engine.config.channel.clone()),
            (
                "config dir",
                engine.config.home_manager_dir().display().to_string()
            ),
            ("settings", settings.display().to_string()),
            ("external", engine.external_packages.len().to_string()),
        ])
    );

    for target in TARGETS {
        print!("{}", target_block(&engine, target));
    }
    Ok(ExitCode::SUCCESS)
}

fn target_block(engine: &Engine, target: Target) -> String {
    let managed = engine.config.managed_file_for(target);
    let main = engine.config.main_file_for(target);
    let active = if target == engine.config.target {
        " (active)"
    } else {
        ""
    };
    let body = render::fields(&[
        (
            "  packages",
            engine.manifest_for(target).packages.len().to_string(),
        ),
        (
            "  managed file",
            format!("{}{}", managed.display(), exists_suffix(&managed)),
        ),
        (
            "  main file",
            format!("{}{}", main.display(), exists_suffix(&main)),
        ),
        (
            "  imported",
            import_label(engine.import_state(target)).to_string(),
        ),
    ]);
    format!("\n{}{}\n{}", target_name(target), active, body)
}
