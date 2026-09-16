use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Target {
    HomeManager,
    NixosSystem,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum InputMode {
    #[default]
    Vim,
    Normal,
}

impl InputMode {
    pub fn label(self) -> &'static str {
        match self {
            InputMode::Vim => "Vim mode",
            InputMode::Normal => "Normal mode",
        }
    }
}

impl Target {
    pub fn label(self) -> &'static str {
        match self {
            Target::HomeManager => "home-manager",
            Target::NixosSystem => "nixos",
        }
    }

    /// Short scope tag for compact display in the TUI.
    pub fn tag(self) -> &'static str {
        match self {
            Target::HomeManager => "hm",
            Target::NixosSystem => "nixos",
        }
    }
}

const MAX_RECENT_SEARCHES: usize = 20;
pub const DEFAULT_CHANNEL: &str = "nixpkgs-26.05";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub channel: String,
    pub target: Target,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub input_mode: InputMode,
    #[serde(default)]
    pub recent_searches: Vec<String>,
    /// Override for the user's main home-manager config (defaults to
    /// `<nixos_config_dir>/home.nix`). Scanned for externally-declared packages.
    #[serde(default)]
    pub home_manager_main_file: Option<PathBuf>,
    /// Override for the user's main NixOS config (defaults to a local
    /// `configuration.nix` if present, else `/etc/nixos/configuration.nix`).
    #[serde(default)]
    pub nixos_main_file: Option<PathBuf>,
}

/// Every theme name `theme` accepts.
///
/// The palettes themselves belong to the front-end that draws them, but the
/// name is a stored setting, so validating one must not require the TUI to be
/// compiled in. A test in `nixbox-tui` keeps the two lists in step.
pub const THEMES: [&str; 6] = [
    "default",
    "dracula",
    "gruvbox",
    "nord",
    "catppuccin",
    "monokai",
];

fn default_theme() -> String {
    "default".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            channel: DEFAULT_CHANNEL.to_string(),
            target: Target::NixosSystem,
            theme: default_theme(),
            input_mode: InputMode::default(),
            recent_searches: Vec::new(),
            home_manager_main_file: None,
            nixos_main_file: None,
        }
    }
}

impl Config {
    /// Pushes a query to the front of recent_searches, dedups, caps the list.
    pub fn push_recent(&mut self, query: String) {
        let trimmed = query.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        self.recent_searches.retain(|q| q != &trimmed);
        self.recent_searches.insert(0, trimmed);
        if self.recent_searches.len() > MAX_RECENT_SEARCHES {
            self.recent_searches.truncate(MAX_RECENT_SEARCHES);
        }
    }
}

impl Config {
    /// Returns the path where nixbox writes its managed packages file for the
    /// current target.
    pub fn managed_file(&self) -> PathBuf {
        self.managed_file_for(self.target)
    }

    /// Returns the managed packages file for an arbitrary target.
    pub fn managed_file_for(&self, target: Target) -> PathBuf {
        match target {
            Target::HomeManager => nixos_config_dir().join("nixbox-home-packages.nix"),
            Target::NixosSystem => nixos_config_dir().join("nixbox-system-packages.nix"),
        }
    }

    /// Returns the generated module that imports flake modules for `target`.
    pub fn flake_manifest_for(&self, target: Target) -> PathBuf {
        match target {
            Target::HomeManager => nixos_config_dir().join("nixbox-home-flakes.nix"),
            Target::NixosSystem => nixos_config_dir().join("nixbox-system-flakes.nix"),
        }
    }

    /// Returns the root flake that owns the inputs used by nixbox modules.
    pub fn flake_file(&self) -> PathBuf {
        nixos_config_dir().join("flake.nix")
    }

    /// Returns the directory where nixbox keeps its generated nix files.
    pub fn home_manager_dir(&self) -> PathBuf {
        nixos_config_dir()
    }

    /// Returns the user's main config file for the current target.
    pub fn main_file(&self) -> PathBuf {
        self.main_file_for(self.target)
    }

    /// Returns the user's main config file for an arbitrary target.
    pub fn main_file_for(&self, target: Target) -> PathBuf {
        match target {
            Target::HomeManager => self
                .home_manager_main_file
                .clone()
                .unwrap_or_else(|| nixos_config_dir().join("home.nix")),
            Target::NixosSystem => self.nixos_main_file.clone().unwrap_or_else(|| {
                // Prefer the user-local config dir (common with flake setups),
                // fall back to the traditional system path otherwise.
                let local = nixos_config_dir().join("configuration.nix");
                if local.exists() {
                    local
                } else {
                    PathBuf::from("/etc/nixos/configuration.nix")
                }
            }),
        }
    }

    pub fn load_or_default() -> Result<Self> {
        let path = settings_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let cfg: Config =
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        let path = settings_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let raw = serde_json::to_string_pretty(self)?;
        fs::write(&path, raw).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

pub fn settings_path() -> Result<PathBuf> {
    let base = BaseDirs::new().context("locating user directories")?;
    Ok(base.config_dir().join("nixbox").join("settings.json"))
}

fn nixos_config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("NIXBOX_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(base) = BaseDirs::new() {
        base.config_dir().join("nixos")
    } else {
        PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/nixos")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_nixos_on_nixpkgs_with_default_theme() {
        let cfg = Config::default();
        assert_eq!(cfg.channel, "nixpkgs-26.05");
        assert_eq!(cfg.target, Target::NixosSystem);
        assert_eq!(cfg.theme, "default");
        assert_eq!(cfg.input_mode, InputMode::Vim);
        assert!(cfg.recent_searches.is_empty());
        assert!(cfg.home_manager_main_file.is_none());
        assert!(cfg.nixos_main_file.is_none());
    }

    #[test]
    fn target_labels_and_tags_are_stable() {
        assert_eq!(Target::HomeManager.label(), "home-manager");
        assert_eq!(Target::HomeManager.tag(), "hm");
        assert_eq!(Target::NixosSystem.label(), "nixos");
        assert_eq!(Target::NixosSystem.tag(), "nixos");
    }

    #[test]
    fn push_recent_trims_deduplicates_and_moves_to_front() {
        let mut cfg = Config::default();
        cfg.push_recent(" ripgrep ".into());
        cfg.push_recent("fd".into());
        cfg.push_recent("ripgrep".into());
        cfg.push_recent("   ".into());

        assert_eq!(cfg.recent_searches, vec!["ripgrep", "fd"]);
    }

    #[test]
    fn push_recent_caps_history() {
        let mut cfg = Config::default();
        for i in 0..25 {
            cfg.push_recent(format!("pkg-{i}"));
        }

        assert_eq!(cfg.recent_searches.len(), 20);
        assert_eq!(cfg.recent_searches.first().unwrap(), "pkg-24");
        assert_eq!(cfg.recent_searches.last().unwrap(), "pkg-5");
    }

    #[test]
    fn config_deserialization_applies_defaults_for_new_fields() {
        let cfg: Config = serde_json::from_str(
            r#"{
                "channel": "nixpkgs-unstable",
                "target": "home-manager"
            }"#,
        )
        .expect("deserialize config");

        assert_eq!(cfg.channel, "nixpkgs-unstable");
        assert_eq!(cfg.target, Target::HomeManager);
        assert_eq!(cfg.theme, "default");
        assert_eq!(cfg.input_mode, InputMode::Vim);
        assert!(cfg.recent_searches.is_empty());
        assert!(cfg.home_manager_main_file.is_none());
        assert!(cfg.nixos_main_file.is_none());
    }

    #[test]
    fn input_modes_have_stable_names() {
        assert_eq!(serde_json::to_string(&InputMode::Vim).unwrap(), r#""vim""#);
        assert_eq!(
            serde_json::to_string(&InputMode::Normal).unwrap(),
            r#""normal""#
        );
        assert_eq!(InputMode::Vim.label(), "Vim mode");
        assert_eq!(InputMode::Normal.label(), "Normal mode");
    }

    #[test]
    fn explicit_main_file_overrides_are_respected() {
        let cfg = Config {
            home_manager_main_file: Some(PathBuf::from("/tmp/home.nix")),
            nixos_main_file: Some(PathBuf::from("/tmp/configuration.nix")),
            ..Config::default()
        };

        assert_eq!(
            cfg.main_file_for(Target::HomeManager),
            PathBuf::from("/tmp/home.nix")
        );
        assert_eq!(
            cfg.main_file_for(Target::NixosSystem),
            PathBuf::from("/tmp/configuration.nix")
        );
    }
}
