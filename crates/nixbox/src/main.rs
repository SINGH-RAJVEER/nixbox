mod cli;
mod commands;
mod render;

use std::process::ExitCode;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, EXIT_FAILURE};

/// Restores the default `SIGPIPE` disposition, which Rust otherwise ignores.
/// Without this, `nixbox list | head` panics on a broken pipe instead of
/// exiting quietly the way every other command-line tool does.
fn restore_sigpipe() {
    // SAFETY: called before the runtime starts, so no other thread is
    // installing signal handlers concurrently.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    restore_sigpipe();
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    match cli.run().await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("nixbox: {error:#}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}
