//! The `nixbox-cli` binary: every subcommand, no terminal UI.
//!
//! It installs as `nixbox-cli` rather than `nixbox` so it can sit alongside
//! the full build instead of overwriting it. The commands themselves live in
//! `nixbox-cmd`.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    nixbox_cmd::run(env!("CARGO_BIN_NAME")).await
}
