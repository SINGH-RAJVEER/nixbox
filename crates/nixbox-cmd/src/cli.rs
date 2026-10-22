//! Argument parsing and dispatch.

use std::process::ExitCode;

use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};
use nixbox_config::{Config, Target};
use nixbox_core::Engine;
use nixbox_core::telemetry::{Interface, Telemetry};

use crate::commands;

/// Exit status for a command that ran but reported a problem, as opposed to
/// one that failed outright.
pub const EXIT_FAILURE: u8 = 1;

/// Help shared by the CLI package's three executable names.
const ABOUT: &str = "Command-line package manager for NixOS and Home Manager";

const LONG_ABOUT: &str = "Search nixpkgs, and let nixbox write the result into your home-manager \
                          or NixOS configuration and rebuild. This build has no user interface; \
                          Install the desktop GUI or nixbox-tui separately for an interactive interface.";

/// Printed after the help when `nixbox-cli` runs without a subcommand.
const NO_INTERFACE: &str = "This is nixbox-cli. Install the separate GUI package or nixbox-tui for an interactive interface.";

// `name` is overridden at runtime with the installed binary name, because one
// crate backs `nixbox`, `nixbox-tui`, and `nixbox-cli`. The literal here is
// only the default for tests and for `Cli::command()` called outside `run`.
#[derive(Parser, Debug)]
#[command(
    name = "nixbox",
    version,
    about = ABOUT,
    long_about = LONG_ABOUT
)]
pub struct Cli {
	#[command(subcommand)]
	pub command: Option<Command>,

	#[command(flatten)]
	pub global: GlobalArgs,
}

#[derive(Args, Debug, Clone, Default)]
pub struct GlobalArgs {
	/// Act on this target instead of the one saved in settings.
	#[arg(long, short = 't', global = true, value_name = "TARGET")]
	pub target: Option<TargetArg>,

	/// Use this nixpkgs channel instead of the saved one.
	#[arg(long, short = 'c', global = true, value_name = "CHANNEL")]
	pub channel: Option<String>,

	/// Print JSON instead of a table.
	#[arg(long, global = true)]
	pub json: bool,

	#[arg(skip)]
	pub(crate) telemetry: Telemetry,
}

impl GlobalArgs {
	/// Applies per-invocation overrides. Nothing here is written back to
	/// `settings.json` — `nixbox config set` is the only thing that persists.
	pub fn apply(&self, config: &mut Config) {
		if let Some(target) = self.target {
			config.target = target.into();
		}
		if let Some(channel) = &self.channel {
			config.channel.clone_from(channel);
		}
	}

	/// Loads settings, applies the overrides, and reads both manifests.
	pub fn engine(&self) -> Result<Engine> {
		let mut config = Config::load_or_default()?;
		self.apply(&mut config);
		Engine::with_config(config)
	}
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum TargetArg {
	/// Your home-manager configuration.
	#[value(name = "home-manager", alias = "hm", alias = "home")]
	HomeManager,
	/// Your NixOS system configuration.
	#[value(name = "nixos", alias = "system", alias = "nixos-system")]
	Nixos,
}

impl From<TargetArg> for Target {
	fn from(value: TargetArg) -> Self {
		match value {
			TargetArg::HomeManager => Target::HomeManager,
			TargetArg::Nixos => Target::NixosSystem,
		}
	}
}

#[derive(Subcommand, Debug)]
pub enum Command {
	/// Search a nixpkgs channel.
	Search(commands::search::SearchArgs),

	/// Add packages and rebuild, or update the package profile once set up.
	Install(commands::install::InstallArgs),

	/// Drop packages and rebuild, or update the package profile once set up.
	#[command(visible_alias = "rm", alias = "uninstall")]
	Remove(commands::remove::RemoveArgs),

	/// Move hand-declared packages into the file nixbox manages.
	Migrate(commands::migrate::MigrateArgs),

	/// Rewrite the managed file and activate it without changing the package set.
	Apply(commands::apply::ApplyArgs),

	/// Set up, tear down, list or roll back the independent package profile.
	Profile {
		#[command(subcommand)]
		action: commands::profile::Action,
	},

	/// Finish work a previous run left behind.
	Resume(commands::resume::ResumeArgs),

	/// List the packages nixbox manages.
	List(commands::list::ListArgs),

	/// List packages declared in your own config that nixbox does not manage.
	Scan,

	/// Show the active target, the files nixbox owns, and how they are wired.
	Status,

	/// Check that everything nixbox depends on is in place.
	Doctor,

	/// Browse and manage flake modules.
	Flake {
		#[command(subcommand)]
		action: commands::flake::Action,
	},

	/// Review configuration changes and commit with a journal-generated message.
	Commit(commands::commit::CommitArgs),

	/// Read or change saved settings.
	Config {
		#[command(subcommand)]
		action: commands::config::Action,
	},

	/// Inspect optional telemetry without collecting or sending data.
	Telemetry {
		#[command(subcommand)]
		action: commands::telemetry::Action,
	},

	/// Print a shell completion script.
	Completions {
		/// Shell to generate for.
		shell: clap_complete::Shell,
	},
}

impl Cli {
	pub async fn run(mut self) -> Result<ExitCode> {
		let telemetry = if matches!(
			&self.command,
			None | Some(
				Command::Config { .. } | Command::Telemetry { .. } | Command::Completions { .. }
			)
		) {
			Telemetry::default()
		} else {
			let telemetry = Telemetry::new(Interface::Cli, tokio::runtime::Handle::current());
			telemetry.started();
			telemetry
		};
		self.global.telemetry = telemetry.clone();
		let result = match self.command {
			// A binary with an interface opens it before getting here, so
			// this is `nixbox-cli`: say what it can do instead of exiting
			// silently.
			None => {
				use clap::CommandFactory;
				Cli::command()
					.name(crate::program())
					.bin_name(crate::program())
					.print_help()?;
				eprintln!("\n{NO_INTERFACE}");
				Ok(ExitCode::from(EXIT_FAILURE))
			}
			Some(Command::Search(args)) => commands::search::run(&args, &self.global).await,
			Some(Command::Install(args)) => commands::install::run(&args, &self.global).await,
			Some(Command::Profile { action }) => {
				commands::profile::run(&action, &self.global).await
			}
			Some(Command::Remove(args)) => commands::remove::run(&args, &self.global).await,
			Some(Command::Migrate(args)) => commands::migrate::run(&args, &self.global).await,
			Some(Command::Apply(args)) => commands::apply::run(&args, &self.global).await,
			Some(Command::Resume(args)) => commands::resume::run(&args, &self.global).await,
			Some(Command::List(args)) => commands::list::run(&args, &self.global),
			Some(Command::Scan) => commands::scan::run(&self.global),
			Some(Command::Status) => commands::status::run(&self.global),
			Some(Command::Doctor) => commands::doctor::run(&self.global).await,
			Some(Command::Flake { action }) => commands::flake::run(&action, &self.global).await,
			Some(Command::Config { action }) => commands::config::run(&action, &self.global),
			Some(Command::Telemetry { action }) => commands::telemetry::run(&action, &self.global),
			Some(Command::Commit(args)) => commands::commit::run(&args, &self.global),
			Some(Command::Completions { shell }) => {
				commands::completions::run(shell);
				Ok(ExitCode::SUCCESS)
			}
		};
		telemetry.shutdown().await;
		result
	}
}

#[cfg(test)]
mod tests {
	use super::{Cli, GlobalArgs, TargetArg};
	use clap::{CommandFactory, Parser};
	use nixbox_config::{Config, Target};

	#[test]
	fn the_command_tree_is_well_formed() {
		Cli::command().debug_assert();
	}

	#[test]
	fn flake_migrate_accepts_one_output_or_all() {
		use super::Command;
		use crate::commands::flake::Action;

		let one = Cli::try_parse_from([
			"nixbox",
			"flake",
			"migrate",
			"llm-agents#chatgpt",
			"--dry-run",
		])
		.unwrap();
		assert!(matches!(
			one.command,
			Some(Command::Flake {
				action: Action::Migrate {
					output: Some(_),
					all: false,
					..
				}
			})
		));
		let all = Cli::try_parse_from([
			"nixbox",
			"flake",
			"migrate",
			"--all",
			"--target",
			"home-manager",
		])
		.unwrap();
		assert!(matches!(
			all.command,
			Some(Command::Flake {
				action: Action::Migrate {
					output: None,
					all: true,
					..
				}
			})
		));
	}

	#[test]
	fn flake_add_accepts_an_exact_output_path() {
		use super::Command;
		use crate::commands::flake::Action;

		let cli = Cli::try_parse_from([
			"nixbox",
			"flake",
			"add",
			"owner/repo",
			"--output",
			"packages.x86_64-linux.\"my.tool\"",
			"--dry-run",
		])
		.unwrap();
		assert!(
			matches!(cli.command, Some(Command::Flake { action: Action::Add {
            output: Some(path), ..
        } }) if path == "packages.x86_64-linux.\"my.tool\"")
		);
	}

	#[test]
	fn a_bare_invocation_carries_no_subcommand() {
		let cli = Cli::parse_from(["nixbox"]);
		assert!(cli.command.is_none());
	}

	#[test]
	fn target_accepts_its_short_aliases() {
		let cli = Cli::parse_from(["nixbox", "list", "--target", "hm"]);
		assert_eq!(cli.global.target, Some(TargetArg::HomeManager));

		let cli = Cli::parse_from(["nixbox", "list", "-t", "system"]);
		assert_eq!(cli.global.target, Some(TargetArg::Nixos));
	}

	#[test]
	fn overrides_apply_without_touching_anything_else() {
		let mut config = Config::default();
		let theme = config.theme.clone();
		GlobalArgs {
			target: Some(TargetArg::HomeManager),
			channel: Some("nixpkgs-unstable".into()),
			json: false,
			..GlobalArgs::default()
		}
		.apply(&mut config);

		assert_eq!(config.target, Target::HomeManager);
		assert_eq!(config.channel, "nixpkgs-unstable");
		assert_eq!(config.theme, theme);
	}

	#[test]
	fn absent_overrides_leave_the_saved_settings_alone() {
		let mut config = Config::default();
		let before = (config.target, config.channel.clone());
		GlobalArgs::default().apply(&mut config);

		assert_eq!((config.target, config.channel.clone()), before);
	}
}
