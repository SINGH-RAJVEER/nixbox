use std::os::unix::process::CommandExt as _;
use std::process::{Command, ExitCode};

#[rustfmt::skip]
fn flake_ref() -> String {
	format!(
		"github:SINGH-RAJVEER/nixbox/v{}#nixbox-gui",
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
	eprintln!("nixbox-gui: could not run Nix package: {error}");
	ExitCode::FAILURE
}
