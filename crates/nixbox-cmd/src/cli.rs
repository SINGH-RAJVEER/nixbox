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

/// `--help` has to describe the binary that was actually built. The crate
/// description cannot do this job, because `nixbox` and `nixbox-cli` are one
/// crate compiled twice.
#[cfg(feature = "tui")]
const ABOUT: &str = "TUI package manager for NixOS and Home Manager";
#[cfg(not(feature = "tui"))]
const ABOUT: &str = "Command-line package manager for NixOS and Home Manager";

#[cfg(feature = "tui")]
const LONG_ABOUT: &str = "Search nixpkgs, and let nixbox write the result into your home-manager \
                          or NixOS configuration and rebuild. Run without a subcommand for the \
                          terminal UI.";
#[cfg(not(feature = "tui"))]
const LONG_ABOUT: &str = "Search nixpkgs, and let nixbox write the result into your home-manager \
                          or NixOS configuration and rebuild. This build has no terminal UI; the \
                          `nixbox` crate is the same commands plus one.";

// `name` is overridden at runtime with the installed binary name, because one
// crate backs both `nixbox` and `nixbox-cli`. The literal here is only the
// default for tests and for `Cli::command()` called outside `run`.
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
    #[cfg(feature = "tui")]
    Tui,

    /// Search a nixpkgs channel.
    Search(commands::search::SearchArgs),

    /// Add packages to your configuration and rebuild.
    Install(commands::install::InstallArgs),

    /// Drop packages from your configuration and rebuild.
    #[command(visible_alias = "rm", alias = "uninstall")]
    Remove(commands::remove::RemoveArgs),

    /// Move hand-declared packages into the file nixbox manages.
    Migrate(commands::migrate::MigrateArgs),

    /// Rewrite the managed file and rebuild without changing the package set.
    Apply(commands::apply::ApplyArgs),

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
            #[cfg(feature = "tui")]
            None | Some(Command::Tui) => {
                nixbox_tui::run().await?;
                Ok(ExitCode::SUCCESS)
            }
            // Built without the TUI: there is nothing to fall back to, so say
            // what this binary can do instead of exiting silently.
            #[cfg(not(feature = "tui"))]
            None => {
                use clap::CommandFactory;
                Cli::command().print_help()?;
                eprintln!("\nThis is nixbox-cli. Install the `nixbox` crate for the terminal UI.");
                Ok(ExitCode::from(EXIT_FAILURE))
            }
            Some(Command::Search(args)) => commands::search::run(&args, &self.global).await,
            Some(Command::Install(args)) => commands::install::run(&args, &self.global).await,
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
    fn a_bare_invocation_carries_no_subcommand() {
        let cli = Cli::parse_from(["nixbox"]);
        assert!(cli.command.is_none());
    }

    /// The feature gate decides whether a bare `nixbox` opens a UI or prints
    /// help, so the subcommand has to track it in both configurations.
    #[test]
    fn the_tui_subcommand_exists_only_in_a_tui_build() {
        let offered = Cli::command()
            .get_subcommands()
            .any(|sub| sub.get_name() == "tui");
        assert_eq!(offered, cfg!(feature = "tui"));
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
