//! Argument parsing and dispatch.

use std::process::ExitCode;

use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};
use nixbox_config::{Config, Target};
use nixbox_core::Engine;

use crate::commands;

/// Exit status for a command that ran but reported a problem, as opposed to
/// one that failed outright.
pub const EXIT_FAILURE: u8 = 1;

#[derive(Parser, Debug)]
#[command(
    name = "nixbox",
    version,
    about,
    long_about = "Search nixpkgs, and let nixbox write the result into your home-manager or NixOS \
                  configuration and rebuild. Run without a subcommand for the terminal UI."
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
    /// Launch the terminal UI (what you get with no subcommand).
    Tui,

    /// Search a nixpkgs channel.
    Search(commands::search::SearchArgs),

    /// List the packages nixbox manages.
    List(commands::list::ListArgs),

    /// List packages declared in your own config that nixbox does not manage.
    Scan,

    /// Show the active target, the files nixbox owns, and how they are wired.
    Status,

    /// Read or change saved settings.
    Config {
        #[command(subcommand)]
        action: commands::config::Action,
    },

    /// Print a shell completion script.
    Completions {
        /// Shell to generate for.
        shell: clap_complete::Shell,
    },
}

impl Cli {
    pub async fn run(self) -> Result<ExitCode> {
        match self.command {
            None | Some(Command::Tui) => {
                nixbox_tui::run().await?;
                Ok(ExitCode::SUCCESS)
            }
            Some(Command::Search(args)) => commands::search::run(&args, &self.global).await,
            Some(Command::List(args)) => commands::list::run(&args, &self.global),
            Some(Command::Scan) => commands::scan::run(&self.global),
            Some(Command::Status) => commands::status::run(&self.global),
            Some(Command::Config { action }) => commands::config::run(&action, &self.global),
            Some(Command::Completions { shell }) => {
                commands::completions::run(shell);
                Ok(ExitCode::SUCCESS)
            }
        }
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
    fn no_subcommand_means_the_tui() {
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
