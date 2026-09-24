//! `nixbox flake` — browse and manage flake modules.
//!
//! Searching and inspecting go through the GitHub API via `gh`, so they need
//! an authenticated `gh auth login`. Adding and removing only touch local
//! files.

use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Subcommand;
use nixbox_config::Target;
use nixbox_core::Op;
use nixbox_nix::flakes::{FlakeDetails, FlakeHit, fetch_flake_details, search_flakes};
use nixbox_nix::manifest::FlakeOutput;
use serde::Serialize;
use serde_json::json;

use crate::apply::{ApplyOpts, Plan, execute};
use crate::cli::GlobalArgs;
use crate::commands::target_name;
use crate::render;

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Search GitHub for flakes.
    Search {
        /// Words to match.
        #[arg(required = true, num_args = 1.., value_name = "QUERY")]
        query: Vec<String>,

        /// Show at most this many results.
        #[arg(long, short = 'n', default_value_t = 20, value_name = "COUNT")]
        limit: usize,
    },

    /// Show what a flake publishes.
    Info {
        /// Repository, as `owner/repo`.
        repo: String,
    },

    /// List the flake modules nixbox manages.
    List,

    /// Import a flake's module into your configuration and rebuild.
    Add {
        /// Repository, as `owner/repo`.
        repo: String,

        #[command(flatten)]
        apply: ApplyOpts,
    },

    /// Drop a flake module from your configuration and rebuild.
    #[command(visible_alias = "rm")]
    Remove {
        /// Repository, as `owner/repo`.
        repo: String,

        #[command(flatten)]
        apply: ApplyOpts,
    },
}

pub async fn run(action: &Action, global: &GlobalArgs) -> Result<ExitCode> {
    match action {
        Action::Search { query, limit } => search(&query.join(" "), *limit, global).await,
        Action::Info { repo } => info(repo, global).await,
        Action::List => list(global),
        Action::Add { repo, apply } => add(repo, apply, global).await,
        Action::Remove { repo, apply } => remove(repo, apply, global).await,
    }
}

#[derive(Serialize)]
struct HitRow {
    repo: String,
    path: String,
    url: String,
}

async fn search(query: &str, limit: usize, global: &GlobalArgs) -> Result<ExitCode> {
    let rows: Vec<HitRow> = search_flakes(query)
        .await?
        .into_iter()
        .take(limit)
        .map(|hit| HitRow {
            repo: hit.repo,
            path: hit.path,
            url: hit.repo_url,
        })
        .collect();

    if global.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(ExitCode::SUCCESS);
    }
    if rows.is_empty() {
        eprintln!("No flakes matched {query}.");
        return Ok(ExitCode::SUCCESS);
    }

    let table: Vec<Vec<String>> = rows
        .iter()
        .map(|row| vec![row.repo.clone(), row.path.clone(), row.url.clone()])
        .collect();
    print!("{}", render::table(&["REPO", "PATH", "URL"], &table));
    Ok(ExitCode::SUCCESS)
}

async fn info(repo: &str, global: &GlobalArgs) -> Result<ExitCode> {
    let details = details_for(repo).await?;

    if global.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "repo": details.repo,
                "url": details.repo_url,
                "description": details.description,
                "stars": details.stars,
                "topics": details.topics,
                "homepage": details.homepage,
                "default_branch": details.default_branch,
                "pushed_at": details.pushed_at,
                "archived": details.archived,
                "inputs": details.inputs,
                "outputs": details.outputs,
                "nixos_module": details.nixos_module,
                "home_manager_module": details.home_manager_module,
                "packages": details.packages.iter().map(|p| &p.attr).collect::<Vec<_>>(),
            }))?
        );
        return Ok(ExitCode::SUCCESS);
    }

    let mut rows = vec![
        ("repo", details.repo.clone()),
        ("url", details.repo_url.clone()),
        ("stars", details.stars.to_string()),
        ("branch", details.default_branch.clone()),
    ];
    if let Some(description) = &details.description {
        rows.push(("about", description.clone()));
    }
    if let Some(homepage) = &details.homepage {
        rows.push(("homepage", homepage.clone()));
    }
    if let Some(pushed) = &details.pushed_at {
        rows.push(("pushed", pushed.clone()));
    }
    if details.archived {
        rows.push(("archived", "yes".to_string()));
    }
    if !details.topics.is_empty() {
        rows.push(("topics", details.topics.join(", ")));
    }
    rows.push(("inputs", join_or_dash(&details.inputs)));
    rows.push(("outputs", join_or_dash(&details.outputs)));
    if let Some(module) = &details.nixos_module {
        rows.push(("nixos module", module.clone()));
    }
    if let Some(module) = &details.home_manager_module {
        rows.push(("hm module", module.clone()));
    }
    if !details.packages.is_empty() {
        let attrs: Vec<String> = details
            .packages
            .iter()
            .map(|package| package.attr.clone())
            .collect();
        rows.push(("packages", attrs.join(", ")));
    }
    print!("{}", render::fields(&rows));
    Ok(ExitCode::SUCCESS)
}

fn list(global: &GlobalArgs) -> Result<ExitCode> {
    let engine = global.engine()?;
    let scope = engine.config.target;
    let outputs = engine.managed_flakes(scope)?;
    let rows: Vec<(String, &str, String)> = outputs
        .into_iter()
        .map(|(repo, output)| match output {
            FlakeOutput::Module(module) => (repo, "module", module),
            FlakeOutput::Package(package) => (repo, "package", package),
        })
        .collect();

    if global.json {
        let rows: Vec<_> = rows
            .iter()
            .map(|(repo, kind, output)| json!({ "repo": repo, "kind": kind, "output": output }))
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(ExitCode::SUCCESS);
    }
    if rows.is_empty() {
        eprintln!(
            "nixbox is not managing any {} flake outputs yet.",
            target_name(scope)
        );
        return Ok(ExitCode::SUCCESS);
    }

    let table: Vec<Vec<String>> = rows
        .into_iter()
        .map(|(repo, kind, output)| vec![repo, kind.to_string(), output])
        .collect();
    print!("{}", render::table(&["REPO", "KIND", "OUTPUT"], &table));
    Ok(ExitCode::SUCCESS)
}

async fn add(repo: &str, opts: &ApplyOpts, global: &GlobalArgs) -> Result<ExitCode> {
    let mut engine = global.engine()?;
    let scope = engine.config.target;

    if engine
        .managed_flakes(scope)?
        .iter()
        .any(|(managed, _)| managed == repo)
    {
        eprintln!("{repo} is already imported for {}.", target_name(scope));
        return Ok(ExitCode::SUCCESS);
    }

    let details = details_for(repo).await?;
    let (summary, ops) = match installable_for(&details, scope)? {
        Installable::Module(module) => (
            vec![format!("import {}#{} [{}]", repo, module, scope.tag())],
            vec![Op::InstallFlake {
                repo: details.repo,
                module,
                scope,
            }],
        ),
        Installable::Package(package) => (
            vec![format!("install {}#{} [{}]", repo, package, scope.tag())],
            vec![Op::InstallFlakePackage {
                repo: details.repo,
                package,
                scope,
            }],
        ),
    };
    execute(
        &mut engine,
        Plan {
            scope,
            ops,
            summary,
        },
        opts,
    )
    .await
}

async fn remove(repo: &str, opts: &ApplyOpts, global: &GlobalArgs) -> Result<ExitCode> {
    let mut engine = global.engine()?;
    let scope = engine.config.target;

    if !engine
        .managed_flakes(scope)?
        .iter()
        .any(|(managed, _)| managed == repo)
    {
        bail!(
            "{repo} is not one of the {} flakes nixbox manages. `nixbox flake list` shows them.",
            target_name(scope)
        );
    }

    let summary = vec![format!("remove flake {} [{}]", repo, scope.tag())];
    let ops = vec![Op::UninstallFlake {
        repo: repo.to_string(),
        scope,
    }];
    execute(
        &mut engine,
        Plan {
            scope,
            ops,
            summary,
        },
        opts,
    )
    .await
}

/// What a flake offers this target.
#[derive(Debug)]
enum Installable {
    Module(String),
    Package(String),
}

/// Picks what to install. Home Manager gets the first package, since a module
/// does nothing until its options are set; NixOS gets the default module.
/// Either falls back to the other kind when the flake lacks the preferred one.
///
/// Both come from evaluating the flake rather than from the names of its
/// outputs, so nothing is offered that the target could not actually import.
fn installable_for(details: &FlakeDetails, scope: Target) -> Result<Installable> {
    let package = details
        .packages
        .first()
        .map(|package| Installable::Package(package.attr.clone()));
    let module = match scope {
        Target::HomeManager => details.home_manager_module.clone(),
        Target::NixosSystem => details.nixos_module.clone(),
    }
    .map(Installable::Module);
    let preferred = match scope {
        Target::HomeManager => package.or(module),
        Target::NixosSystem => module.or(package),
    };
    if let Some(installable) = preferred {
        return Ok(installable);
    }
    bail!(
        "{} has no installable package for this system and no default {} module (it publishes: \
         {}).",
        details.repo,
        target_name(scope),
        join_or_dash(&details.outputs),
    )
}

async fn details_for(repo: &str) -> Result<FlakeDetails> {
    let trimmed = repo.trim_matches('/');
    let parts: Vec<&str> = trimmed.split('/').collect();
    if parts.len() != 2 || parts.iter().any(|part| part.is_empty()) {
        bail!("`{repo}` is not a repository. Use `owner/repo`.");
    }
    fetch_flake_details(&FlakeHit::for_repo(trimmed).await?).await
}

fn join_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::{Installable, installable_for, join_or_dash};
    use nixbox_config::Target;
    use nixbox_nix::flakes::{FlakeDetails, FlakePackage};

    fn details(
        nixos_module: Option<&str>,
        home_manager_module: Option<&str>,
        packages: &[&str],
    ) -> FlakeDetails {
        FlakeDetails {
            repo: "owner/repo".into(),
            repo_url: "https://github.com/owner/repo".into(),
            path: "flake.nix".into(),
            description: None,
            stars: 0,
            topics: Vec::new(),
            homepage: None,
            default_branch: "main".into(),
            pushed_at: None,
            archived: false,
            inputs: Vec::new(),
            outputs: vec!["packages".to_string()],
            packages: packages
                .iter()
                .map(|attr| FlakePackage {
                    attr: (*attr).to_string(),
                    name: (*attr).to_string(),
                    version: "1.0".to_string(),
                })
                .collect(),
            nixos_module: nixos_module.map(str::to_string),
            home_manager_module: home_manager_module.map(str::to_string),
        }
    }

    #[test]
    fn each_target_picks_the_module_it_can_actually_import() {
        let both = details(
            Some("nixosModules.default"),
            Some("homeManagerModules.default"),
            &[],
        );
        assert!(matches!(
            installable_for(&both, Target::NixosSystem).expect("nixos module"),
            Installable::Module(module) if module == "nixosModules.default"
        ));
        assert!(matches!(
            installable_for(&both, Target::HomeManager).expect("hm module"),
            Installable::Module(module) if module == "homeManagerModules.default"
        ));
    }

    #[test]
    fn home_manager_prefers_a_package_over_a_module() {
        let both = details(None, Some("homeModules.default"), &["default"]);
        assert!(matches!(
            installable_for(&both, Target::HomeManager).expect("package"),
            Installable::Package(package) if package == "default"
        ));
    }

    #[test]
    fn a_flake_with_no_module_for_the_target_falls_back_to_its_first_package() {
        let package_only = details(None, None, &["default", "extra"]);
        assert!(matches!(
            installable_for(&package_only, Target::HomeManager).expect("package"),
            Installable::Package(package) if package == "default"
        ));
    }

    #[test]
    fn a_flake_with_nothing_installable_is_refused_with_what_it_does_have() {
        let error = installable_for(&details(None, None, &[]), Target::HomeManager)
            .expect_err("nothing installable");

        assert!(error.to_string().contains("packages"), "{error}");
    }

    #[test]
    fn empty_lists_render_as_a_dash() {
        assert_eq!(join_or_dash(&[]), "-");
        assert_eq!(join_or_dash(&["a".to_string(), "b".to_string()]), "a, b");
    }
}
