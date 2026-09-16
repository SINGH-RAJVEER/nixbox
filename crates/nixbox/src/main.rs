//! The `nixbox` binary: every subcommand plus the terminal UI.
//!
//! Running it with no subcommand opens the UI. The commands themselves live
//! in `nixbox-cmd`, shared with the UI-less `nixbox-cli` binary.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    nixbox_cmd::run(env!("CARGO_BIN_NAME")).await
}
