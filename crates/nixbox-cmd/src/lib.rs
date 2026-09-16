//! The nixbox command tree, shared by both binaries.
//!
//! `nixbox` and `nixbox-cli` are the same program built two ways: the former
//! with the terminal UI behind the `tui` feature, the latter without it. Every
//! subcommand lives here so the two can never drift apart, and each binary is
//! only a `main` that calls [`run`].

mod apply;
mod cli;
mod commands;
mod render;

use std::process::ExitCode;
use std::sync::OnceLock;

use clap::{CommandFactory, FromArgMatches};
use tracing_subscriber::EnvFilter;

pub use crate::cli::{Cli, EXIT_FAILURE};

/// Set once by [`run`], because the two binaries this crate backs are invoked
/// under different names and every message that tells the user what to type
/// next has to name the one they actually ran.
static PROGRAM: OnceLock<&'static str> = OnceLock::new();

/// The name the running binary was installed as, defaulting to `nixbox` for
/// tests and any caller that does not go through [`run`].
pub(crate) fn program() -> &'static str {
    PROGRAM.get().copied().unwrap_or("nixbox")
}

/// Restores the default `SIGPIPE` disposition, which Rust otherwise ignores.
/// Without this, `nixbox list | head` panics on a broken pipe instead of
/// exiting quietly the way every other command-line tool does.
fn restore_sigpipe() {
    // SAFETY: the only signal handler this program installs, set once at the
    // top of `run` before any command has had a chance to spawn work.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

/// Parses the command line, starts tracing, and runs the requested command.
///
/// `program` is the installed name of the binary, which each `main` passes as
/// `env!("CARGO_BIN_NAME")`. It drives the usage line, the completion script,
/// and every "run `X apply`" hint.
///
/// Returns the process exit code rather than exiting, so a caller keeps the
/// choice; an error is printed here because the message is the same for both
/// binaries.
pub async fn run(program: &'static str) -> ExitCode {
    restore_sigpipe();
    let _ = PROGRAM.set(program);

    let matches = Cli::command().name(program).bin_name(program).get_matches();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    match cli.run().await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{program}: {error:#}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}
