//! One module per subcommand.

pub mod apply;
pub mod completions;
pub mod config;
pub mod doctor;
pub mod flake;
pub mod install;
pub mod list;
pub mod migrate;
pub mod remove;
pub mod resume;
pub mod scan;
pub mod search;
pub mod status;

use std::pin::pin;
use std::time::Duration;

use anyhow::Result;
use directories::BaseDirs;
use nixbox_config::Target;
use nixbox_core::{Engine, scope_matches};
use nixbox_nix::search::{PackageCatalog, SearchHit, search};
use tokio::time::timeout;

use crate::cli::GlobalArgs;

/// How a package relates to the active target: tracked by nixbox, declared by
/// hand in the user's own config, or neither.
#[must_use]
pub fn package_status(engine: &Engine, name: &str) -> &'static str {
    let target = engine.config.target;
    if engine.is_tracked(name, target) {
        "managed"
    } else if engine
        .external_packages
        .iter()
        .any(|ep| ep.name == name && scope_matches(ep.scope, target))
    {
        "external"
    } else {
        "-"
    }
}

/// The settings-file spelling of a target.
#[must_use]
pub fn target_name(target: Target) -> &'static str {
    match target {
        Target::HomeManager => "home-manager",
        Target::NixosSystem => "nixos",
    }
}

/// Searches the way the TUI does: against a catalog built from the exact
/// nixpkgs revision the target configuration locks, which is the revision an
/// install would actually resolve against.
///
/// An explicit `--channel` means the user asked for a particular channel, so
/// that goes to a live `nix search` instead. So does anything the catalog
/// cannot handle, rather than failing the command outright.
pub async fn search_packages(
    engine: &Engine,
    global: &GlobalArgs,
    query: &str,
) -> Result<Vec<SearchHit>> {
    if global.channel.is_some() {
        return search(&engine.config.channel, query).await;
    }

    match catalog(engine).await {
        Ok(catalog) => Ok(catalog.search(query)),
        Err(error) => {
            eprintln!("Note: {error}; searching {} live.", engine.config.channel);
            search(&engine.config.channel, query).await
        }
    }
}

/// The same catalog the TUI prepares, from the same cache file, so the two
/// front-ends never disagree about what is installable.
///
/// Building one evaluates the whole package set and takes minutes, which a
/// silent command would look hung for. Rather than guess from the cache file
/// whether that is about to happen, this says so only once the work has
/// visibly not finished: a cached load and a lock file the catalog cannot use
/// both return well inside the grace period and print nothing.
async fn catalog(engine: &Engine) -> Result<PackageCatalog> {
    const GRACE: Duration = Duration::from_secs(2);

    let base =
        BaseDirs::new().ok_or_else(|| anyhow::anyhow!("cannot locate the cache directory"))?;
    let cache_path = base.cache_dir().join("nixbox").join("package-catalog.json");

    let config_dir = engine.config.home_manager_dir();
    let mut work = pin!(PackageCatalog::load_or_build(&config_dir, &cache_path));
    match timeout(GRACE, &mut work).await {
        Ok(result) => result,
        Err(_) => {
            eprintln!(
                "Preparing the package catalog for your locked nixpkgs revision. This runs once \
                 per revision."
            );
            work.await
        }
    }
}
