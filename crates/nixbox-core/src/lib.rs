//! The headless engine behind nixbox.
//!
//! Everything that mutates the user's configuration lives here: manifest
//! bookkeeping, writing the managed `.nix` files, inserting the `imports`
//! entry, keeping git aware of generated files, and working out which rebuild
//! command applies to a given target. Front-ends (the TUI and the CLI) own
//! presentation, scheduling, and cancellation — not policy.

pub mod engine;
pub mod op;
pub mod rebuild;
pub mod report;
pub mod state;

pub use engine::{Engine, ImportState, ManagedPackage, scan_externals, scan_target, scope_matches};
pub use op::Op;
pub use rebuild::{HOME_FALLBACK_NOTE, RebuildCommand};
pub use report::{LogReporter, Reporter, SilentReporter};
pub use state::{InProgress, PersistedState, state_path};

#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Mutex, MutexGuard, PoisonError};

    use nixbox_config::Config;
    use nixbox_nix::search::SearchHit;

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Points `NIXBOX_CONFIG_DIR` at a fresh empty directory for as long as
    /// the guard lives.
    ///
    /// The config crate reads that variable on every path lookup, so it is
    /// process-global state: the guard holds a lock to keep tests that touch
    /// it from overlapping.
    pub(crate) struct TempConfigDir {
        path: PathBuf,
        _guard: MutexGuard<'static, ()>,
    }

    impl TempConfigDir {
        /// A config pinned to this directory, so nothing in a test can reach
        /// the host's real `/etc/nixos`.
        pub(crate) fn config(&self) -> Config {
            Config {
                home_manager_main_file: Some(self.path.join("home.nix")),
                nixos_main_file: Some(self.path.join("configuration.nix")),
                ..Config::default()
            }
        }
    }

    impl Drop for TempConfigDir {
        fn drop(&mut self) {
            // SAFETY: the lock this guard holds is still held here, so no
            // other test is reading the environment concurrently.
            unsafe { std::env::remove_var("NIXBOX_CONFIG_DIR") };
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    pub(crate) fn temp_dir(label: &str) -> TempConfigDir {
        let guard = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("nixbox-{label}-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp config dir");
        // SAFETY: guarded by ENV_LOCK, held for the lifetime of the guard.
        unsafe { std::env::set_var("NIXBOX_CONFIG_DIR", &path) };
        TempConfigDir {
            path,
            _guard: guard,
        }
    }

    pub(crate) fn hit(attr: &str) -> SearchHit {
        SearchHit {
            attr: attr.into(),
            pname: attr.into(),
            version: "1.0".into(),
            description: String::new(),
        }
    }
}
