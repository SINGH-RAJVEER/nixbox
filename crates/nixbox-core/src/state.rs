//! The work that must survive a crash or a kill: operations that were queued
//! but never reached a manifest, the operation that was mid-rebuild, and the
//! last error.
//!
//! The manifest is written *before* the rebuild starts, so an interrupted
//! rebuild has already changed the files on disk. Recovering from one means
//! re-running the rebuild for that scope, not re-applying the operation.
//!
//! This lives in the engine rather than in a front-end because the file is
//! shared: work queued in the TUI is resumable from the CLI and the other way
//! round. The field names are an on-disk format.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::BaseDirs;
use nixbox_config::Target;
use serde::{Deserialize, Serialize};

use crate::op::Op;

/// The rebuild that was running when the process went away.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InProgress {
    pub scope: Target,
    pub label: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistedState {
    #[serde(default)]
    pub pending_queue: Vec<Op>,
    #[serde(default)]
    pub in_progress: Option<InProgress>,
    #[serde(default)]
    pub last_error: Option<String>,
}

impl PersistedState {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending_queue.is_empty() && self.in_progress.is_none() && self.last_error.is_none()
    }

    /// Reads the saved state, or `None` when there is nothing to resume.
    ///
    /// A malformed file is treated as nothing rather than as an error: it is a
    /// cache of interrupted work, and refusing to start because of it would be
    /// worse than dropping it.
    #[must_use]
    pub fn load() -> Option<Self> {
        let path = state_path().ok()?;
        if !path.exists() {
            return None;
        }
        let raw = fs::read_to_string(&path).ok()?;
        serde_json::from_str(&raw).ok()
    }

    /// Writes the state, removing the file entirely once nothing is left.
    pub fn save(&self) -> Result<()> {
        let path = state_path()?;
        if self.is_empty() {
            if path.exists() {
                let _ = fs::remove_file(&path);
            }
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let raw = serde_json::to_string_pretty(self)?;
        fs::write(&path, raw).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    /// Forgets everything, removing the file.
    pub fn clear() -> Result<()> {
        Self::default().save()
    }
}

pub fn state_path() -> Result<PathBuf> {
    let base = BaseDirs::new().context("locating user directories")?;
    Ok(base.config_dir().join("nixbox").join("state.json"))
}
