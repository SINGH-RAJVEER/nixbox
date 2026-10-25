use crate::{Engine, PersistedState, Session};
use anyhow::{Result, ensure};
use nixbox_config::{Config, Target};
use nixbox_nix::{Manifest, dendritic::Layout};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationChoice {
	pub label: String,
	pub host: Option<String>,
	pub user: Option<String>,
	pub home: Option<String>,
}

impl ConfigurationChoice {
	pub fn selected(&self, config: &Config) -> bool {
		self.host == config.nixos_host
			&& self.user == config.home_user
			&& self.home == config.home_configuration
	}
	pub fn apply(&self, config: &mut Config) {
		config.nixos_host.clone_from(&self.host);
		config.home_user.clone_from(&self.user);
		config.home_configuration.clone_from(&self.home);
		if self.home.is_some() {
			config.target = Target::HomeManager;
		}
	}
}

pub fn choices(config: &Config) -> Result<Vec<ConfigurationChoice>> {
	let Some(discovery) = Layout::inspect(&config.config_root())? else {
		return Ok(Vec::new());
	};
	let mut choices = Vec::new();
	for host in discovery.hosts {
		if host.users.len() != 1 {
			choices.push(ConfigurationChoice {
				label: format!("NixOS: {}", host.output),
				host: Some(host.output.clone()),
				user: None,
				home: None,
			});
		}
		for user in host.users {
			choices.push(ConfigurationChoice {
				label: format!("NixOS: {} / Home Manager: {user}", host.output),
				host: Some(host.output.clone()),
				user: Some(user),
				home: None,
			});
		}
	}
	for home in discovery.homes {
		choices.push(ConfigurationChoice {
			label: format!("Home Manager: {}", home.output),
			host: None,
			user: home.user,
			home: Some(home.output),
		});
	}
	Ok(choices)
}

impl Engine {
	/// Ambiguous supported layouts can open an interface for target selection.
	/// Operations still go through discovery and refuse an unresolved selection.
	pub fn for_interface(config: Config) -> Result<Self> {
		match Self::with_config(config.clone()) {
			Ok(engine) => Ok(engine),
			Err(error) => {
				if Layout::inspect(&config.config_root())?.is_none()
					|| Layout::select_output(
						&config.config_root(),
						config.nixos_host.as_deref(),
						config.home_user.as_deref(),
						config.home_configuration.as_deref(),
					)
					.is_ok()
				{
					return Err(error);
				}
				Ok(Self::from_parts(
					config,
					Manifest::default(),
					Manifest::default(),
					Vec::new(),
				))
			}
		}
	}
}

impl Session {
	pub fn select_configuration(&mut self, choice: &ConfigurationChoice) -> Result<()> {
		ensure!(
			!self.is_building() && self.queue.is_empty(),
			"finish or drop queued work before changing configuration"
		);
		if let Some(saved) = PersistedState::load() {
			ensure!(
				saved.pending_queue.is_empty() && saved.in_progress.is_none(),
				"finish or discard saved work before changing configuration"
			);
		}
		let mut config = self.engine.config.clone();
		choice.apply(&mut config);
		let engine = Engine::with_config(config)?;
		engine.config.save()?;
		self.engine = engine;
		self.clear_recovery_block();
		self.last_error = None;
		Ok(())
	}
}
