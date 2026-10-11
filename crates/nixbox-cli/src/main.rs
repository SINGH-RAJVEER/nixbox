//! The `nixbox-cli` binary: every subcommand, no user interface.
//!
//! It installs as `nixbox-cli` rather than `nixbox` so it can sit alongside
//! the desktop build instead of overwriting it. The commands themselves live
//! in `nixbox-cmd`.

use std::process::ExitCode;

fn main() -> ExitCode {
	nixbox_cmd::run(env!("CARGO_BIN_NAME"), None)
}
