use std::os::unix::process::CommandExt as _;
use std::process::{Command, ExitCode};

/// The independently installed desktop GUI flake package.
const PACKAGE: &str = if env!("CARGO_PKG_VERSION_PRE").is_empty() {
	"nixbox"
} else {
	"nixbox-nightly"
};

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
