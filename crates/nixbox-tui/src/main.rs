//! The independently installed terminal interface. CLI commands live elsewhere.

use std::process::ExitCode;

use clap::Parser as _;
use nixbox_config::{Config, Target};
use tracing_subscriber::EnvFilter;

#[derive(clap::Parser)]
#[command(
	version,
	about = "NixBox terminal UI",
	after_help = "For CLI commands, install separately:\n  nix profile add github:SINGH-RAJVEER/nixbox#nixbox-cli\nThen run: nixbox --help or nb --help"
)]
struct Args {
	#[arg(long, short = 't', value_parser = ["home-manager", "hm", "home", "nixos", "system", "nixos-system"])]
	target: Option<String>,
	#[arg(long, short = 'c')]
	channel: Option<String>,
}

fn main() -> ExitCode {
	let args = Args::parse();
	tracing_subscriber::fmt()
		.with_env_filter(
			EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
		)
		.with_writer(std::io::stderr)
		.init();
	let result = (|| -> anyhow::Result<()> {
		let mut config = Config::load_or_default()?;
		if let Some(target) = args.target {
			config.target = if matches!(target.as_str(), "home-manager" | "hm" | "home") {
				Target::HomeManager
			} else {
				Target::NixosSystem
			};
		}
		if let Some(channel) = args.channel {
			config.channel = channel;
		}
		tokio::runtime::Builder::new_multi_thread()
			.enable_all()
			.build()?
			.block_on(nixbox_tui::run_with_config(config))
	})();
	match result {
		Ok(()) => ExitCode::SUCCESS,
		Err(error) => {
			eprintln!("nixbox-tui: {error:#}");
			ExitCode::FAILURE
		}
	}
}
