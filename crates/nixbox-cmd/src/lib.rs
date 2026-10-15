//! The command tree for the independently installed NixBox CLI.

mod apply;
mod cli;
mod commands;
mod render;

use std::process::ExitCode;
use std::sync::OnceLock;

use clap::{CommandFactory, FromArgMatches};
use tracing_subscriber::EnvFilter;

pub use crate::cli::{Cli, EXIT_FAILURE};

static PROGRAM: OnceLock<&'static str> = OnceLock::new();

pub(crate) fn program() -> &'static str {
	PROGRAM.get().copied().unwrap_or("nixbox")
}

/// Run CLI commands with explicit arguments, including the executable name.
/// This crate has no interface launch options; GUI and TUI use the engine directly.
pub fn run_from(
	program: &'static str,
	args: impl IntoIterator<Item = std::ffi::OsString>,
) -> ExitCode {
	let _ = PROGRAM.set(program);
	let command = Cli::command().name(program).bin_name(program);
	let cli = match Cli::from_arg_matches(&command.get_matches_from(args)) {
		Ok(cli) => cli,
		Err(error) => error.exit(),
	};
	tracing_subscriber::fmt()
		.with_env_filter(
			EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
		)
		.with_writer(std::io::stderr)
		.init();
	// SAFETY: reset once, before starting command work, so a closed output pipe
	// exits quietly instead of panicking on a broken pipe.
	unsafe {
		libc::signal(libc::SIGPIPE, libc::SIG_DFL);
	}
	let result = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()
		.map_err(anyhow::Error::from)
		.and_then(|runtime| runtime.block_on(cli.run()));
	match result {
		Ok(code) => code,
		Err(error) => {
			eprintln!("{program}: {error:#}");
			ExitCode::from(EXIT_FAILURE)
		}
	}
}
