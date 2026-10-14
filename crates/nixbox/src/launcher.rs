use std::os::unix::process::CommandExt as _;
use std::process::{Command, ExitCode};

/// The flake app this launcher stands in for. The GUI is Linux only, so
/// elsewhere it runs the terminal UI, which takes the same subcommands.
#[cfg(target_os = "linux")]
const PACKAGE: &str = env!("CARGO_BIN_NAME");
#[cfg(not(target_os = "linux"))]
const PACKAGE: &str = "nixbox-tui";

#[rustfmt::skip]
fn flake_ref() -> String {
	format!(
		"github:SINGH-RAJVEER/nixbox/v{}#{PACKAGE}",
		env!("CARGO_PKG_VERSION")
	)
}

#[rustfmt::skip]
pub(crate) fn main() -> ExitCode {
	let error = Command::new("nix")
		.arg("run")
		.arg(flake_ref())
		.arg("--")
		.args(std::env::args_os().skip(1))
		.exec();
	eprintln!("{}: could not run Nix package: {error}", env!("CARGO_BIN_NAME"));
	ExitCode::FAILURE
}
