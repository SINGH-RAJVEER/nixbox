//! Restoring the work that a previous run left behind.
//!
//! The file format lives in `nixbox-core`, because the CLI resumes the same
//! state; this module is only the part that puts it back into a running TUI.

use nixbox_core::Restored;

use crate::app::{App, Tab};

/// Pulls any saved state from disk into `app`'s session, which re-runs an
/// interrupted rebuild or drains the queued ops, and says so in the status.
pub(crate) fn restore(app: &mut App) {
	let restored = app.session.restore();

	if let Some(err) = &app.session.last_error {
		app.status = format!("Previous run failed: {err}");
	}
	match restored {
		Restored::Nothing => {}
		Restored::Rebuild(label) => {
			app.tab = Tab::Building;
			app.status = format!(
				"Resuming interrupted build: {}.",
				label.trim_start_matches("resume ")
			);
		}
		Restored::Queue(count) => {
			app.tab = Tab::Queue;
			app.status = format!("Resuming {count} queued op(s).");
		}
	}
}

#[cfg(test)]
mod tests {
	use crate::app::QueuedOp;
	use nixbox_config::Target;
	use nixbox_core::{InProgress, PersistedState};
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
				QueuedOp::SetOptions {
					changes: vec![nixbox_core::OptionChange {
						path: vec!["programs".into(), "git".into(), "enable".into()],
						value: Some(nixbox_nix::SettingValue::Bool(true)),
					}],
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

		assert_eq!(restored.pending_queue.len(), 5);
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
		assert!(matches!(
			&restored.pending_queue[4],
			QueuedOp::SetOptions { changes, scope: Target::HomeManager }
				if changes[0].value == Some(nixbox_nix::SettingValue::Bool(true))
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
