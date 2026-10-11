//! The `nixbox-tui` binary: every subcommand plus the terminal UI.
//!
//! Running it with no subcommand opens the UI. The commands themselves live
//! in `nixbox-cmd`, shared with the desktop `nixbox` and the UI-less
//! `nixbox-cli`.

use std::process::ExitCode;

use nixbox_cmd::Frontend;

const FRONTEND: Frontend = Frontend {
	about: "Terminal UI package manager for NixOS and Home Manager",
	long_about: "Search nixpkgs, and let nixbox write the result into your home-manager or \
	             NixOS configuration and rebuild. Run without a subcommand for the terminal UI.",
	log_filter: "warn",
	launch,
};

fn launch() -> anyhow::Result<()> {
	tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()?
		.block_on(nixbox_tui::run())
}

fn main() -> ExitCode {
	nixbox_cmd::run(env!("CARGO_BIN_NAME"), Some(&FRONTEND))
}
