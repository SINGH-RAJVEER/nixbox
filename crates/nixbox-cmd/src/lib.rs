//! The nixbox command tree, shared by every binary.
//!
//! `nixbox`, `nixbox-tui`, and `nixbox-cli` are the same commands with a
//! different interface behind a bare invocation: the desktop GUI, the
//! terminal UI, or none. Every subcommand lives here so the three can never
//! drift apart, and each binary is only a `main` that calls [`run`] with its
//! [`Frontend`].

mod apply;
mod cli;
mod commands;
mod render;

use std::process::ExitCode;
use std::sync::OnceLock;

use clap::{CommandFactory, FromArgMatches};
use tracing_subscriber::EnvFilter;

pub use crate::cli::{Cli, EXIT_FAILURE};

/// The interface a binary opens when it is run without a subcommand.
pub struct Frontend {
	/// The one-line `--help` description of the binary.
	pub about: &'static str,
	/// The longer description `--help` shows.
	pub long_about: &'static str,
	/// The tracing filter while the interface is open, unless `RUST_LOG`
	/// says otherwise. Commands always default to `warn`.
	pub log_filter: &'static str,
	/// Opens the interface and returns once it closes. It runs outside any
	/// async runtime, because the GUI drives its own event loop and the TUI
	/// starts the runtime it needs.
	pub launch: fn() -> anyhow::Result<()>,
}

/// Set once by [`run`], because the binaries this crate backs are invoked
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
	// SAFETY: the only signal handler this program installs, set once before
	// the command has had a chance to spawn work.
	unsafe {
		libc::signal(libc::SIGPIPE, libc::SIG_DFL);
	}
}

/// Parses the command line, starts tracing, and runs the requested command,
/// or opens `frontend` when there is none.
///
/// `program` is the installed name of the binary, which each `main` passes as
/// `env!("CARGO_BIN_NAME")`. It drives the usage line, the completion script,
/// and every "run `X apply`" hint.
///
/// Returns the process exit code rather than exiting, so a caller keeps the
/// choice; an error is printed here because the message is the same for every
/// binary.
pub fn run(program: &'static str, frontend: Option<&Frontend>) -> ExitCode {
	let _ = PROGRAM.set(program);

	let mut command = Cli::command().name(program).bin_name(program);
	if let Some(frontend) = frontend {
		command = command
			.about(frontend.about)
			.long_about(frontend.long_about);
	}
	let cli = match Cli::from_arg_matches(&command.get_matches()) {
		Ok(cli) => cli,
		Err(error) => error.exit(),
	};

	let interface = frontend.filter(|_| cli.command.is_none());
	let default_filter = interface.map_or("warn", |frontend| frontend.log_filter);
	tracing_subscriber::fmt()
		.with_env_filter(
			EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter)),
		)
		.with_writer(std::io::stderr)
		.init();

	let result = match interface {
		Some(frontend) => (frontend.launch)().map(|()| ExitCode::SUCCESS),
		None => {
			// Only a command writes to a pipe; an interface keeps Rust's
			// default so a closed socket cannot kill a window.
			restore_sigpipe();
			tokio::runtime::Builder::new_multi_thread()
				.enable_all()
				.build()
				.map_err(anyhow::Error::from)
				.and_then(|runtime| runtime.block_on(cli.run()))
		}
	};
	match result {
		Ok(code) => code,
		Err(error) => {
			eprintln!("{program}: {error:#}");
			ExitCode::from(EXIT_FAILURE)
		}
	}
}
