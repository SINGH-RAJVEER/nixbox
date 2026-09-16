//! `nixbox config` — read and change saved settings.
//!
//! This is the only command that writes `settings.json`; `--target` and
//! `--channel` stay scoped to a single invocation.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Subcommand;
use nixbox_config::{Config, InputMode, THEMES, Target, settings_path};
use serde_json::json;

use crate::cli::GlobalArgs;
use crate::commands::target_name;
use crate::render;

/// Every key `get`/`set` understands, in the order `show` prints them.
const KEYS: [&str; 6] = [
    "channel",
    "target",
    "theme",
    "input-mode",
    "home-manager-main-file",
    "nixos-main-file",
];

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Print every setting.
    Show,
    /// Print the path of the settings file.
    Path,
    /// Print one setting.
    Get {
        /// Setting to read.
        key: String,
    },
    /// Change one setting. An empty value clears an optional path.
    Set {
        /// Setting to change.
        key: String,
        /// New value.
        value: String,
    },
}

pub fn run(action: &Action, global: &GlobalArgs) -> Result<ExitCode> {
    match action {
        Action::Path => {
            println!("{}", settings_path()?.display());
            Ok(ExitCode::SUCCESS)
        }
        Action::Show => show(global),
        Action::Get { key } => get(key, global),
        Action::Set { key, value } => set(key, value),
    }
}

fn show(global: &GlobalArgs) -> Result<ExitCode> {
    // Deliberately the saved settings, not the overridden ones: this is what
    // is on disk.
    let config = Config::load_or_default()?;

    if global.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "channel": config.channel,
                "target": target_name(config.target),
                "theme": config.theme,
                "input-mode": input_mode_name(config.input_mode),
                "home-manager-main-file": config.home_manager_main_file,
                "nixos-main-file": config.nixos_main_file,
            }))?
        );
        return Ok(ExitCode::SUCCESS);
    }

    let rows: Vec<(&str, String)> = KEYS
        .iter()
        .map(|key| (*key, read(&config, key).unwrap_or_default()))
        .collect();
    print!("{}", render::fields(&rows));
    Ok(ExitCode::SUCCESS)
}

fn get(key: &str, global: &GlobalArgs) -> Result<ExitCode> {
    let mut config = Config::load_or_default()?;
    // `get` answers "what would this invocation use", so overrides apply.
    global.apply(&mut config);
    let Some(value) = read(&config, key) else {
        bail!(
            "unknown setting `{key}`. Known settings: {}",
            KEYS.join(", ")
        );
    };
    println!("{value}");
    Ok(ExitCode::SUCCESS)
}

fn set(key: &str, value: &str) -> Result<ExitCode> {
    let mut config = Config::load_or_default()?;
    match key {
        "channel" => {
            if value.trim().is_empty() {
                bail!("channel cannot be empty");
            }
            config.channel = value.to_string();
        }
        "target" => config.target = parse_target(value)?,
        "theme" => {
            if !THEMES.contains(&value) {
                bail!("unknown theme `{value}`. Available: {}", THEMES.join(", "));
            }
            config.theme = value.to_string();
        }
        "input-mode" => config.input_mode = parse_input_mode(value)?,
        "home-manager-main-file" => config.home_manager_main_file = parse_path(value),
        "nixos-main-file" => config.nixos_main_file = parse_path(value),
        other => bail!(
            "unknown setting `{other}`. Known settings: {}",
            KEYS.join(", ")
        ),
    }
    config.save()?;
    println!("{key} = {}", read(&config, key).unwrap_or_default());
    Ok(ExitCode::SUCCESS)
}

fn read(config: &Config, key: &str) -> Option<String> {
    let value = match key {
        "channel" => config.channel.clone(),
        "target" => target_name(config.target).to_string(),
        "theme" => config.theme.clone(),
        "input-mode" => input_mode_name(config.input_mode).to_string(),
        "home-manager-main-file" => path_value(config.home_manager_main_file.as_ref()),
        "nixos-main-file" => path_value(config.nixos_main_file.as_ref()),
        _ => return None,
    };
    Some(value)
}

fn path_value(path: Option<&PathBuf>) -> String {
    path.map_or_else(|| "(default)".to_string(), |p| p.display().to_string())
}

fn parse_path(value: &str) -> Option<PathBuf> {
    if value.trim().is_empty() {
        None
    } else {
        Some(PathBuf::from(value))
    }
}

fn parse_target(value: &str) -> Result<Target> {
    match value {
        "home-manager" | "hm" | "home" => Ok(Target::HomeManager),
        "nixos" | "system" | "nixos-system" => Ok(Target::NixosSystem),
        other => bail!("unknown target `{other}`. Use `home-manager` or `nixos`"),
    }
}

fn parse_input_mode(value: &str) -> Result<InputMode> {
    match value {
        "vim" => Ok(InputMode::Vim),
        "normal" => Ok(InputMode::Normal),
        other => bail!("unknown input mode `{other}`. Use `vim` or `normal`"),
    }
}

fn input_mode_name(mode: InputMode) -> &'static str {
    match mode {
        InputMode::Vim => "vim",
        InputMode::Normal => "normal",
    }
}

#[cfg(test)]
mod tests {
    use super::{KEYS, parse_input_mode, parse_path, parse_target, read};
    use nixbox_config::{Config, InputMode, Target};

    #[test]
    fn every_key_reads_back() {
        let config = Config::default();
        for key in KEYS {
            assert!(read(&config, key).is_some(), "{key} has no reader");
        }
        assert!(read(&config, "nonsense").is_none());
    }

    #[test]
    fn targets_accept_their_aliases_and_reject_junk() {
        assert_eq!(parse_target("hm").expect("hm"), Target::HomeManager);
        assert_eq!(parse_target("system").expect("system"), Target::NixosSystem);
        assert!(parse_target("laptop").is_err());
    }

    #[test]
    fn input_modes_round_trip() {
        assert_eq!(parse_input_mode("vim").expect("vim"), InputMode::Vim);
        assert_eq!(
            parse_input_mode("normal").expect("normal"),
            InputMode::Normal
        );
        assert!(parse_input_mode("emacs").is_err());
    }

    #[test]
    fn an_empty_path_clears_the_override() {
        assert!(parse_path("   ").is_none());
        assert!(parse_path("/tmp/home.nix").is_some());
    }

    #[test]
    fn unset_paths_read_as_default() {
        let config = Config::default();
        assert_eq!(
            read(&config, "nixos-main-file").as_deref(),
            Some("(default)")
        );
    }
}
