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
pub mod scan;
pub mod search;
pub mod status;

use nixbox_config::Target;
use nixbox_core::{Engine, scope_matches};

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
