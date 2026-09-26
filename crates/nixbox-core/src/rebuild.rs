//! Picking the rebuild command for a target, and running it.
//!
//! `resolve` evaluates the user's flake, which can take a while, so [`run`]
//! races it against the caller's cancel signal as well as the rebuild itself.

use std::path::{Path, PathBuf};

use nixbox_config::Target;
use nixbox_nix::build::{
    BuildEvent, flake_has_home_configuration, home_manager_switch_cmd, nixos_rebuild_switch_cmd,
    rebuild,
};
use tokio::sync::{mpsc, oneshot};

/// Emitted when a home-manager change has to be applied through
/// `nixos-rebuild` because there is no standalone home configuration.
pub const HOME_FALLBACK_NOTE: &str =
    "No standalone homeConfigurations found; applying via nixos-rebuild.";

/// How a rebuild that needs root gets it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Escalation {
    /// Plain `sudo`, which prompts on the controlling terminal. Right for
    /// the TUI and the CLI, which always have one.
    #[default]
    Terminal,
    /// `sudo -A` with `SUDO_ASKPASS` pointing at this program, for a
    /// front-end started without a terminal. sudo stays the parent of the
    /// rebuild, so cancelling can still signal the whole process group.
    Askpass(PathBuf),
}

/// A resolved rebuild invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildCommand {
    pub program: String,
    pub args: Vec<String>,
    /// Added to the inherited environment when the command is spawned.
    pub envs: Vec<(String, String)>,
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

    /// Switches a `sudo` invocation over to `escalation`. Anything that does
    /// not go through sudo is left alone.
    #[must_use]
    pub fn escalated(mut self, escalation: &Escalation) -> Self {
        if let Escalation::Askpass(helper) = escalation
            && self.program == "sudo"
        {
            self.args.insert(0, "-A".into());
            self.envs
                .push(("SUDO_ASKPASS".into(), helper.display().to_string()));
        }
        self
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
                    envs: Vec::new(),
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
        envs: Vec::new(),
        via_nixos_fallback: false,
    }
}

/// Resolves and runs the rebuild for `scope`, sending its output and exactly
/// one closing [`BuildEvent::Finished`] or [`BuildEvent::Cancelled`] to `tx`.
///
/// The command line goes out first as a `$ ...` line, followed by
/// [`HOME_FALLBACK_NOTE`] when a home rebuild goes through `nixos-rebuild`.
pub async fn run(
    config_dir: PathBuf,
    scope: Target,
    escalation: Escalation,
    tx: mpsc::Sender<BuildEvent>,
    mut cancel: oneshot::Receiver<()>,
) {
    let command = tokio::select! {
        command = resolve(&config_dir, scope) => command.escalated(&escalation),
        _ = &mut cancel => {
            let _ = tx.send(BuildEvent::Cancelled).await;
            return;
        }
    };
    if command.via_nixos_fallback {
        let _ = tx.send(BuildEvent::Line(HOME_FALLBACK_NOTE.into())).await;
    }
    let _ = tx
        .send(BuildEvent::Line(format!("$ {}", command.display())))
        .await;
    if let Err(e) = rebuild(
        &command.program,
        &command.arg_refs(),
        &command.envs,
        tx.clone(),
        cancel,
    )
    .await
    {
        let _ = tx.send(BuildEvent::Finished(Err(e.to_string()))).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{Escalation, RebuildCommand, nixos};
    use std::path::{Path, PathBuf};

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
            envs: Vec::new(),
            via_nixos_fallback: false,
        };

        assert_eq!(cmd.display(), "true");
    }

    #[test]
    fn askpass_escalation_makes_sudo_ask_through_the_helper() {
        let helper = PathBuf::from("/run/current-system/sw/bin/nixbox-gui");
        let cmd = nixos(Path::new("/tmp/nixos")).escalated(&Escalation::Askpass(helper));

        assert_eq!(cmd.program, "sudo");
        assert_eq!(cmd.args[0], "-A");
        assert_eq!(cmd.args[1], "nixos-rebuild");
        assert_eq!(
            cmd.envs,
            vec![(
                "SUDO_ASKPASS".to_string(),
                "/run/current-system/sw/bin/nixbox-gui".to_string()
            )]
        );
    }

    #[test]
    fn escalation_leaves_commands_without_sudo_alone() {
        let cmd = RebuildCommand {
            program: "home-manager".into(),
            args: vec!["switch".into()],
            envs: Vec::new(),
            via_nixos_fallback: false,
        };

        let escalated = cmd
            .clone()
            .escalated(&Escalation::Askpass(PathBuf::from("/bin/helper")));

        assert_eq!(escalated, cmd);
        assert_eq!(
            nixos(Path::new("/tmp"))
                .escalated(&Escalation::Terminal)
                .args[0],
            "nixos-rebuild"
        );
    }
}
