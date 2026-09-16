//! Picking the rebuild command for a target.
//!
//! Cancellation is deliberately not handled here: `resolve` evaluates the
//! user's flake, which can take a while, so each front-end races it against
//! its own cancel signal rather than inheriting one policy.

use std::path::Path;

use nixbox_config::Target;
use nixbox_nix::build::{
    flake_has_home_configuration, home_manager_switch_cmd, nixos_rebuild_switch_cmd,
};

/// Emitted when a home-manager change has to be applied through
/// `nixos-rebuild` because there is no standalone home configuration.
pub const HOME_FALLBACK_NOTE: &str =
    "No standalone homeConfigurations found; applying via nixos-rebuild.";

/// A resolved rebuild invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildCommand {
    pub program: String,
    pub args: Vec<String>,
    /// True when a home-manager rebuild fell back to `nixos-rebuild` because
    /// the flake exposes no `homeConfigurations.<user>` output — i.e. the user
    /// wires home-manager in as a NixOS module.
    pub via_nixos_fallback: bool,
}

impl RebuildCommand {
    /// Borrowed args, in the shape [`nixbox_nix::build::rebuild`] wants.
    #[must_use]
    pub fn arg_refs(&self) -> Vec<&str> {
        self.args.iter().map(String::as_str).collect()
    }

    /// The command as a user could retype it.
    #[must_use]
    pub fn display(&self) -> String {
        if self.args.is_empty() {
            self.program.clone()
        } else {
            format!("{} {}", self.program, self.args.join(" "))
        }
    }
}

/// Returns the command that applies pending changes for `scope`.
///
/// For [`Target::HomeManager`] this evaluates the flake to decide between
/// `home-manager switch` and `nixos-rebuild switch`.
pub async fn resolve(config_dir: &Path, scope: Target) -> RebuildCommand {
    match scope {
        Target::NixosSystem => nixos(config_dir),
        Target::HomeManager => {
            if flake_has_home_configuration(config_dir).await {
                let (program, args) = home_manager_switch_cmd(config_dir);
                RebuildCommand {
                    program,
                    args,
                    via_nixos_fallback: false,
                }
            } else {
                RebuildCommand {
                    via_nixos_fallback: true,
                    ..nixos(config_dir)
                }
            }
        }
    }
}

/// The `nixos-rebuild switch` invocation for `config_dir`, without evaluating
/// anything.
#[must_use]
pub fn nixos(config_dir: &Path) -> RebuildCommand {
    let (program, args) = nixos_rebuild_switch_cmd(config_dir);
    RebuildCommand {
        program,
        args,
        via_nixos_fallback: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{RebuildCommand, nixos};
    use std::path::Path;

    #[test]
    fn nixos_command_targets_the_config_dir() {
        let cmd = nixos(Path::new("/tmp/nixos"));

        assert!(!cmd.via_nixos_fallback);
        assert!(cmd.display().contains("/tmp/nixos"));
        assert_eq!(cmd.arg_refs().len(), cmd.args.len());
    }

    #[test]
    fn display_handles_an_argless_command() {
        let cmd = RebuildCommand {
            program: "true".into(),
            args: Vec::new(),
            via_nixos_fallback: false,
        };

        assert_eq!(cmd.display(), "true");
    }
}
