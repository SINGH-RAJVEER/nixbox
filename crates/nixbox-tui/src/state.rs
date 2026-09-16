//! Restoring the work that a previous run left behind.
//!
//! The file format lives in `nixbox-core`, because the CLI resumes the same
//! state; this module is only the part that puts it back into a running TUI.

pub(crate) use nixbox_core::{InProgress, PersistedState};

use tokio::sync::mpsc;

use crate::app::{App, AppEvent, Tab};
use crate::ops::{drain_queue, spawn_rebuild};

/// Pulls any saved state from disk into `app` and kicks off whatever work
/// remains: an interrupted rebuild is re-launched first, otherwise pending
/// queued operations for each rebuild scope are batched and drained.
pub(crate) fn restore(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    let Some(saved) = PersistedState::load() else {
        return;
    };

    if let Some(err) = saved.last_error.clone() {
        app.last_error = Some(err.clone());
        app.status = format!("Previous run failed: {}", err);
    }

    for op in saved.pending_queue {
        app.queue.push_back(op);
    }

    if let Some(ip) = saved.in_progress {
        // The manifest was already written for this op before it was killed —
        // re-run the rebuild to actually apply it.
        let label = if ip.label.starts_with("resume ") {
            ip.label.clone()
        } else {
            format!("resume {}", ip.label)
        };
        app.tab = Tab::Building;
        app.status = format!("Resuming interrupted build: {}.", ip.label);
        spawn_rebuild(app, tx, ip.scope, label);
    } else if !app.queue.is_empty() {
        app.tab = Tab::Queue;
        app.status = format!("Resuming {} queued op(s).", app.queue.len());
        drain_queue(app, tx);
    }
}

#[cfg(test)]
mod tests {
    use super::{InProgress, PersistedState};
    use crate::app::QueuedOp;
    use nixbox_config::Target;
    use nixbox_nix::search::SearchHit;

    fn hit(attr: &str) -> SearchHit {
        SearchHit {
            attr: attr.into(),
            pname: attr.into(),
            version: "0".into(),
            description: String::new(),
        }
    }

    #[test]
    fn queue_with_mixed_op_kinds_round_trips_through_json() {
        let state = PersistedState {
            pending_queue: vec![
                QueuedOp::Install {
                    hit: hit("ripgrep"),
                    scope: Target::HomeManager,
                },
                QueuedOp::InstallFlakePackage {
                    repo: "alleneubank/bun-overlay".into(),
                    package: "default".into(),
                    scope: Target::HomeManager,
                },
                QueuedOp::Uninstall {
                    name: "fd".into(),
                    scope: Target::NixosSystem,
                },
                QueuedOp::Migrate {
                    names: vec!["git".into(), "neovim".into()],
                    scope: Target::HomeManager,
                },
            ],
            in_progress: Some(InProgress {
                scope: Target::HomeManager,
                label: "install ripgrep [hm]".into(),
            }),
            last_error: Some("install foo [hm]: nonzero exit".into()),
        };

        let json = serde_json::to_string(&state).expect("serialize");
        let restored: PersistedState = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.pending_queue.len(), 4);
        assert!(matches!(
            &restored.pending_queue[0],
            QueuedOp::Install { hit, scope: Target::HomeManager } if hit.attr == "ripgrep"
        ));
        assert!(matches!(
            &restored.pending_queue[1],
            QueuedOp::InstallFlakePackage { repo, package, scope: Target::HomeManager }
                if repo == "alleneubank/bun-overlay" && package == "default"
        ));
        assert!(matches!(
            &restored.pending_queue[2],
            QueuedOp::Uninstall { name, scope: Target::NixosSystem } if name == "fd"
        ));
        assert!(matches!(
            &restored.pending_queue[3],
            QueuedOp::Migrate { names, scope: Target::HomeManager } if names == &vec!["git".to_string(), "neovim".to_string()]
        ));
        let ip = restored.in_progress.expect("in_progress preserved");
        assert_eq!(ip.scope, Target::HomeManager);
        assert_eq!(ip.label, "install ripgrep [hm]");
        assert_eq!(
            restored.last_error.as_deref(),
            Some("install foo [hm]: nonzero exit"),
        );
    }

    #[test]
    fn default_state_is_empty() {
        let state = PersistedState::default();
        assert!(state.is_empty());
    }
}
