//! The `nixbox-cli` binary: every subcommand, no user interface.
//!
//! The CLI package owns nixbox, nb, and nixbox-cli. Interfaces are separate.

mod entry;

use std::process::ExitCode;

fn main() -> ExitCode {
	entry::main()
}
