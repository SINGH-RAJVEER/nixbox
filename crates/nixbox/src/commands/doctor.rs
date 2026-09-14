//! `nixbox doctor` — check the things nixbox assumes about your setup.

use std::path::Path;
use std::process::ExitCode;

use anyhow::Result;
use nixbox_config::Target;
use nixbox_core::{Engine, ImportState};
use serde_json::json;
use tokio::process::Command;

use crate::cli::{EXIT_FAILURE, GlobalArgs};
use crate::commands::target_name;
use crate::render;

/// Whether a check passed, passed with a caveat, or means nixbox will not work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Ok,
    Warn,
    Fail,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Warn => "warn",
            Level::Fail => "fail",
        }
    }
}

struct Check {
    name: &'static str,
    level: Level,
    detail: String,
}

pub async fn run(global: &GlobalArgs) -> Result<ExitCode> {
    let engine = global.engine()?;
    let scope = engine.config.target;
    let config_dir = engine.config.home_manager_dir();

    let mut checks = vec![
        nix_available().await,
        config_dir_exists(&config_dir),
        git_work_tree(&config_dir).await,
        flake_file(&engine),
        main_file(&engine, scope),
        imported(&engine, scope),
    ];
    if scope == Target::HomeManager {
        checks.push(home_configuration(&config_dir).await);
    }
    checks.push(gh_available().await);

    let failed = checks.iter().any(|check| check.level == Level::Fail);

    if global.json {
        let rows: Vec<_> = checks
            .iter()
            .map(|check| {
                json!({
                    "check": check.name,
                    "level": check.level.label(),
                    "detail": check.detail,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        let table: Vec<Vec<String>> = checks
            .iter()
            .map(|check| {
                vec![
                    check.name.to_string(),
                    check.level.label().to_string(),
                    check.detail.clone(),
                ]
            })
            .collect();
        print!("{}", render::table(&["CHECK", "STATUS", "DETAIL"], &table));
    }

    if failed {
        Ok(ExitCode::from(EXIT_FAILURE))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

/// Runs a command and returns its trimmed stdout, or None if it could not be
/// run or exited non-zero.
async fn output_of(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().await.ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn nix_available() -> Check {
    match output_of("nix", &["--version"]).await {
        Some(version) => Check {
            name: "nix",
            level: Level::Ok,
            detail: version,
        },
        None => Check {
            name: "nix",
            level: Level::Fail,
            detail: "`nix` is not on PATH; nixbox cannot search or rebuild".into(),
        },
    }
}

fn config_dir_exists(config_dir: &Path) -> Check {
    if config_dir.is_dir() {
        Check {
            name: "config dir",
            level: Level::Ok,
            detail: config_dir.display().to_string(),
        }
    } else {
        Check {
            name: "config dir",
            level: Level::Fail,
            detail: format!(
                "{} does not exist; set NIXBOX_CONFIG_DIR or create it",
                config_dir.display()
            ),
        }
    }
}

async fn git_work_tree(config_dir: &Path) -> Check {
    let inside = output_of(
        "git",
        &[
            "-C",
            &config_dir.display().to_string(),
            "rev-parse",
            "--is-inside-work-tree",
        ],
    )
    .await;
    if inside.as_deref() == Some("true") {
        Check {
            name: "git",
            level: Level::Ok,
            detail: "config dir is a git work tree".into(),
        }
    } else {
        Check {
            name: "git",
            level: Level::Warn,
            detail: "config dir is not a git work tree; flakes ignore untracked files, so \
                     generated files may be invisible to the rebuild"
                .into(),
        }
    }
}

fn flake_file(engine: &Engine) -> Check {
    let path = engine.config.flake_file();
    let Ok(source) = std::fs::read_to_string(&path) else {
        return Check {
            name: "flake.nix",
            level: Level::Warn,
            detail: format!(
                "{} not found; flake modules cannot be added",
                path.display()
            ),
        };
    };
    if source.contains("outputs = inputs@") {
        Check {
            name: "flake.nix",
            level: Level::Ok,
            detail: path.display().to_string(),
        }
    } else {
        Check {
            name: "flake.nix",
            level: Level::Warn,
            detail: format!(
                "{} does not bind `inputs` (outputs = inputs@{{ ... }}); `nixbox flake add` will \
                 refuse until it does",
                path.display()
            ),
        }
    }
}

fn main_file(engine: &Engine, scope: Target) -> Check {
    let path = engine.config.main_file_for(scope);
    if path.exists() {
        Check {
            name: "main config",
            level: Level::Ok,
            detail: path.display().to_string(),
        }
    } else {
        Check {
            name: "main config",
            level: Level::Fail,
            detail: format!(
                "{} not found; set {}-main-file in your settings",
                path.display(),
                target_name(scope)
            ),
        }
    }
}

fn imported(engine: &Engine, scope: Target) -> Check {
    let managed = engine.config.managed_file_for(scope);
    match engine.import_state(scope) {
        ImportState::Imported => Check {
            name: "imports",
            level: Level::Ok,
            detail: format!("{} is imported", managed.display()),
        },
        // Not a failure: the next install inserts the import itself.
        ImportState::NotImported => Check {
            name: "imports",
            level: Level::Warn,
            detail: format!(
                "{} is not imported yet; nixbox adds the import on the next change",
                managed.display()
            ),
        },
        ImportState::MainFileMissing => Check {
            name: "imports",
            level: Level::Warn,
            detail: "no main config file to import from".into(),
        },
    }
}

async fn home_configuration(config_dir: &Path) -> Check {
    if nixbox_nix::build::flake_has_home_configuration(config_dir).await {
        Check {
            name: "home-manager",
            level: Level::Ok,
            detail: "flake exposes a standalone homeConfigurations output".into(),
        }
    } else {
        Check {
            name: "home-manager",
            level: Level::Warn,
            detail: "no standalone homeConfigurations output; changes will be applied with \
                     nixos-rebuild instead"
                .into(),
        }
    }
}

async fn gh_available() -> Check {
    if output_of("gh", &["auth", "status"]).await.is_some() {
        Check {
            name: "gh",
            level: Level::Ok,
            detail: "authenticated; flake search is available".into(),
        }
    } else {
        Check {
            name: "gh",
            level: Level::Warn,
            detail: "`gh` is missing or not logged in; `nixbox flake search` needs `gh auth login`"
                .into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Level, output_of};

    #[test]
    fn levels_have_stable_labels() {
        assert_eq!(Level::Ok.label(), "ok");
        assert_eq!(Level::Warn.label(), "warn");
        assert_eq!(Level::Fail.label(), "fail");
    }

    #[tokio::test]
    async fn a_missing_program_is_reported_as_absent_rather_than_erroring() {
        assert!(output_of("nixbox-no-such-program", &[]).await.is_none());
    }
}
