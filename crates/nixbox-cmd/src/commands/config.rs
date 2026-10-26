//! `nixbox config` — read and change saved settings.
//!
//! This is the only command that writes `settings.json`; `--target` and
//! `--channel` stay scoped to a single invocation.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Subcommand;
use nixbox_config::{Config, InputMode, THEMES, TabLabels, Target, settings_path};
use serde_json::json;

use crate::cli::GlobalArgs;
use crate::commands::target_name;
use crate::render;

/// Every key `get`/`set` understands, in the order `show` prints them.
const KEYS: [&str; 14] = [
	"channel",
	"target",
	"theme",
	"input-mode",
	"tab-labels",
	"close-button",
	"dendritic-detection",
	"telemetry-enabled",
	"config-dir",
	"home-manager-main-file",
	"nixos-main-file",
	"nixos-host",
	"home-user",
	"home-configuration",
];

#[derive(Subcommand, Debug)]
pub enum Action {
	/// Preview or apply conversion of owned modules into native flake-parts files.
	MigrateDendritic {
		#[arg(long)]
		dry_run: bool,
		#[arg(long)]
		yes: bool,
		/// Also move existing root-level flake-parts files into the tree.
		#[arg(long)]
		relocate: bool,
	},
	/// Inspect detected targets, diagnostics, and managed-file placement without writing.
	Inspect,
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
		Action::MigrateDendritic {
			dry_run,
			yes,
			relocate,
		} => {
			let mut config = Config::load_or_default()?;
			global.apply(&mut config);
			anyhow::ensure!(
				config.dendritic_detection,
				"enable dendritic detection before migrating dendritic modules"
			);
			let layout = nixbox_nix::dendritic::Layout::select_output(
				&config.config_root(),
				config.nixos_host.as_deref(),
				config.home_user.as_deref(),
				config.home_configuration.as_deref(),
			)?
			.ok_or_else(|| anyhow::anyhow!("no dendritic configuration was detected"))?;
			let changes = layout.plan_managed_migration(*relocate)?;
			if global.json {
				println!("{}", serde_json::to_string_pretty(&changes)?);
			} else {
				for change in &changes {
					println!(
						"{} {}",
						if change.after.is_none() {
							"Remove"
						} else if change.before.is_none() {
							"Create"
						} else {
							"Update"
						},
						change.path.display()
					);
				}
			}
			if !dry_run && !changes.is_empty() {
				anyhow::ensure!(
					*yes,
					"review with --dry-run --json, then pass --yes to apply this migration"
				);
				if let Some(saved) = nixbox_core::PersistedState::load() {
					anyhow::ensure!(
						saved.pending_queue.is_empty() && saved.in_progress.is_none(),
						"finish or discard saved work before migrating managed modules"
					);
				}
				nixbox_core::apply_managed_migration(&config, &changes)?;
			}
			Ok(ExitCode::SUCCESS)
		}
		Action::Inspect => inspect(global),
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
				"tab-labels": config.tab_labels.name(),
				"close-button": config.close_button,
				"dendritic-detection": config.dendritic_detection,
				"telemetry-enabled": config.telemetry.enabled,
				"config-dir": config.config_dir,
				"home-manager-main-file": config.home_manager_main_file,
				"nixos-main-file": config.nixos_main_file,
				"nixos-host": config.nixos_host,
				"home-user": config.home_user,
				"home-configuration": config.home_configuration,
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
	if key == "telemetry-enabled" {
		let enabled = parse_telemetry(value)?;
		nixbox_core::telemetry::set_enabled(&mut config, enabled)?;
		println!("telemetry-enabled = {}", config.telemetry.enabled);
		return Ok(ExitCode::SUCCESS);
	}
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
		"tab-labels" => config.tab_labels = parse_tab_labels(value)?,
		"close-button" => config.close_button = parse_close_button(value)?,
		"dendritic-detection" => {
			if let Some(saved) = nixbox_core::PersistedState::load() {
				anyhow::ensure!(
					saved.pending_queue.is_empty() && saved.in_progress.is_none(),
					"finish or discard saved work before changing detection mode"
				);
			}
			config.dendritic_detection = match value {
				"true" => true,
				"false" => false,
				_ => bail!("dendritic-detection must be true or false"),
			};
		}
		"config-dir" => {
			config.config_dir = parse_path(value);
			config.nixos_host = None;
			config.home_user = None;
			config.home_configuration = None;
		}
		"home-manager-main-file" => config.home_manager_main_file = parse_path(value),
		"nixos-main-file" => config.nixos_main_file = parse_path(value),
		"nixos-host" => {
			config.home_configuration = None;
			config.nixos_host = optional_name(value);
			config.home_user = None;
		}
		"home-user" => config.home_user = optional_name(value),
		"home-configuration" => {
			config.home_configuration = optional_name(value);
			config.nixos_host = None;
			config.home_user = None;
			if config.home_configuration.is_some() {
				config.target = Target::HomeManager;
			}
		}
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
		"tab-labels" => config.tab_labels.name().to_string(),
		"close-button" => config.close_button.to_string(),
		"dendritic-detection" => config.dendritic_detection.to_string(),
		"telemetry-enabled" => config.telemetry.enabled.to_string(),
		"config-dir" => path_value(config.config_dir.as_ref()),
		"home-manager-main-file" => path_value(config.home_manager_main_file.as_ref()),
		"nixos-main-file" => path_value(config.nixos_main_file.as_ref()),
		"nixos-host" => config.nixos_host.clone().unwrap_or_else(|| "(auto)".into()),
		"home-user" => config.home_user.clone().unwrap_or_else(|| "(auto)".into()),
		"home-configuration" => config
			.home_configuration
			.clone()
			.unwrap_or_else(|| "(auto)".into()),
		_ => return None,
	};
	Some(value)
}

fn optional_name(value: &str) -> Option<String> {
	(!value.trim().is_empty()).then(|| value.to_string())
}

fn inspect(global: &GlobalArgs) -> Result<ExitCode> {
	let mut config = Config::load_or_default()?;
	global.apply(&mut config);
	let root = config.config_root();
	let discovery = if config.dendritic_detection {
		nixbox_nix::dendritic::Layout::inspect(&root)
	} else {
		Ok(None)
	};
	let mut diagnostics = Vec::new();
	let (layout, discovered) = match discovery {
		Ok(Some(discovery)) => ("dendritic", json!(discovery)),
		Ok(None) => ("conventional", json!(null)),
		Err(error) => {
			diagnostics.push(error.to_string());
			("unsupported", json!(null))
		}
	};
	let selected = if diagnostics.is_empty() {
		match nixbox_core::Engine::resolve_config(config) {
			Ok(config) => Some(config),
			Err(error) => {
				diagnostics.push(error.to_string());
				None
			}
		}
	} else {
		None
	};
	let files: Vec<_> = selected.as_ref().map(|config| [nixbox_config::Target::NixosSystem, nixbox_config::Target::HomeManager].into_iter().map(|target| {
		let path = config.managed_file_for(target);
		json!({ "target": target_name(target), "path": path, "exists": path.exists(), "placement": if path.parent() == Some(root.as_path()) { "root import" } else { "import-tree" }, "available": match target { nixbox_config::Target::NixosSystem => config.home_configuration.is_none(), nixbox_config::Target::HomeManager => config.home_configuration.is_some() || config.native_modules_dir.is_none() || config.home_user.is_some() } })
	}).collect()).unwrap_or_default();
	let report = json!({ "layout": layout, "root": root, "discovery": discovered, "selected": selected.as_ref().map(Config::identity), "managed_files": files, "diagnostics": diagnostics, "writes": false });
	if global.json {
		println!("{}", serde_json::to_string_pretty(&report)?);
	} else {
		println!("Layout: {layout}\nRoot: {}", root.display());
		if let Some(hosts) = report["discovery"]["hosts"].as_array() {
			for host in hosts {
				println!(
					"Discovered host: {} (module: {}, users: {})",
					host["output"].as_str().unwrap_or_default(),
					host["module"].as_str().unwrap_or_default(),
					host["users"]
						.as_array()
						.map(|users| users
							.iter()
							.filter_map(|user| user.as_str())
							.collect::<Vec<_>>()
							.join(", "))
						.unwrap_or_default()
				);
			}
		}
		if let Some(config) = &selected {
			println!(
				"Host: {}\nHome Manager user: {}",
				config.nixos_host.as_deref().unwrap_or("default"),
				config.home_user.as_deref().unwrap_or("unselected")
			);
		}
		for file in &files {
			println!(
				"{}: {} [{}]",
				file["target"].as_str().unwrap_or_default(),
				file["path"].as_str().unwrap_or_default(),
				file["placement"].as_str().unwrap_or_default()
			);
		}
		for diagnostic in &diagnostics {
			println!("Diagnostic: {diagnostic}");
		}
	}
	Ok(if diagnostics.is_empty() {
		ExitCode::SUCCESS
	} else {
		ExitCode::from(crate::cli::EXIT_FAILURE)
	})
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

fn parse_telemetry(value: &str) -> Result<bool> {
	match value {
		"true" => Ok(true),
		"false" => Ok(false),
		_ => bail!(
			"telemetry-enabled accepts only `true` or `false`. Run `nixbox telemetry status` for the collection disclosure and recipient"
		),
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

fn parse_tab_labels(value: &str) -> Result<TabLabels> {
	TabLabels::from_name(value).ok_or_else(|| {
		anyhow::anyhow!("unknown tab labels `{value}`. Use `icons`, `icons-and-names`, or `names`")
	})
}

fn parse_close_button(value: &str) -> Result<bool> {
	match value {
		"true" => Ok(true),
		"false" => Ok(false),
		other => bail!("unknown close button value `{other}`. Use `true` or `false`"),
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
	fn telemetry_accepts_only_explicit_booleans() {
		assert!(super::parse_telemetry("true").expect("true"));
		assert!(!super::parse_telemetry("false").expect("false"));
		for value in ["1", "yes", "TRUE", "", " true "] {
			assert!(super::parse_telemetry(value).is_err());
		}
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
