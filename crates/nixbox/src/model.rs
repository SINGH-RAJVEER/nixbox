//! What the views show, worked out from the engine without touching gpui.

use nixbox_config::Target;
use nixbox_core::{Engine, InstalledFlake, ManagedPackage};
use nixbox_nix::scan::{ExternalPackage, ScanTarget};

/// The Installed page's three sections, narrowed by the filter box.
#[derive(Debug, Default)]
pub struct InstalledRows {
	pub managed: Vec<ManagedPackage>,
	pub flakes: Vec<InstalledFlake>,
	pub external: Vec<ExternalPackage>,
}

impl InstalledRows {
	/// Every row whose name, or flake repository, contains `filter`,
	/// ignoring case. An empty filter keeps everything.
	pub fn new(engine: &Engine, filter: &str) -> Self {
		let filter = filter.trim().to_lowercase();
		let matches = |text: &str| filter.is_empty() || text.to_lowercase().contains(&filter);
		Self {
			managed: engine
				.managed_packages()
				.into_iter()
				.filter(|package| matches(&package.name))
				.collect(),
			flakes: engine
				.flakes
				.iter()
				.filter(|flake| {
					matches(&flake.name()) || flake.repo.as_deref().is_some_and(matches)
				})
				.cloned()
				.collect(),
			external: engine
				.external_packages
				.iter()
				.filter(|package| matches(&package.name))
				.cloned()
				.collect(),
		}
	}

	pub fn is_empty(&self) -> bool {
		self.managed.is_empty() && self.flakes.is_empty() && self.external.is_empty()
	}
}

/// Where a package from a search is installed already, as scope tags.
pub fn installed_scopes(engine: &Engine, attr: &str) -> Vec<&'static str> {
	let mut scopes = Vec::new();
	for target in [Target::HomeManager, Target::NixosSystem] {
		let external = engine
			.external_packages
			.iter()
			.any(|package| package.name == attr && package.scope == scan_target(target));
		if engine.manifest_for(target).packages.contains(attr) || external {
			scopes.push(target.tag());
		}
	}
	scopes
}

pub const fn scan_target(target: Target) -> ScanTarget {
	match target {
		Target::HomeManager => ScanTarget::HomeManager,
		Target::NixosSystem => ScanTarget::Nixos,
	}
}

pub const fn target_of(scope: ScanTarget) -> Target {
	match scope {
		ScanTarget::HomeManager => Target::HomeManager,
		ScanTarget::Nixos => Target::NixosSystem,
	}
}

#[cfg(test)]
mod tests {
	use super::{InstalledRows, installed_scopes};
	use nixbox_config::Config;
	use nixbox_core::Engine;
	use nixbox_nix::manifest::Manifest;
	use nixbox_nix::scan::{ExternalPackage, ScanTarget};

	fn engine() -> Engine {
		let mut home = Manifest::default();
		home.packages.insert("ripgrep".into());
		home.packages.insert("fd".into());
		Engine::from_parts(
			Config::default(),
			home,
			Manifest::default(),
			vec![ExternalPackage {
				name: "git".into(),
				source_attr: "environment.systemPackages".into(),
				line: 3,
				migratable: true,
				scope: ScanTarget::Nixos,
			}],
		)
	}

	#[test]
	fn the_filter_matches_names_in_every_section_ignoring_case() {
		let engine = engine();

		assert_eq!(InstalledRows::new(&engine, "").managed.len(), 2);
		let rows = InstalledRows::new(&engine, " RIP ");
		assert_eq!(rows.managed.len(), 1);
		assert!(rows.external.is_empty());
		assert!(InstalledRows::new(&engine, "nothing").is_empty());
		assert_eq!(InstalledRows::new(&engine, "gi").external.len(), 1);
	}

	#[test]
	fn search_hits_show_where_they_are_already_installed() {
		let engine = engine();

		assert_eq!(installed_scopes(&engine, "ripgrep"), vec!["hm"]);
		assert_eq!(installed_scopes(&engine, "git"), vec!["nixos"]);
		assert!(installed_scopes(&engine, "zoxide").is_empty());
	}
}
