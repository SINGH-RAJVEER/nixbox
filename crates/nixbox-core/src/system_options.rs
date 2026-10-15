//! The NixOS settings browser's policy, shared by every front-end: how the
//! schema is grouped into categories, how search ranks matches, which host
//! opens by default, and how an inspection reads.

use nixbox_nix::system_options::{InspectedValue, SystemOption, format_preview};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Category {
	#[default]
	All,
	General,
	Network,
	Desktop,
	Hardware,
	Users,
	Storage,
	Programs,
	Services,
	Virtualization,
	Nix,
	System,
}

impl Category {
	pub const ALL: [Self; 12] = [
		Self::All,
		Self::General,
		Self::Network,
		Self::Desktop,
		Self::Hardware,
		Self::Users,
		Self::Storage,
		Self::Programs,
		Self::Services,
		Self::Virtualization,
		Self::Nix,
		Self::System,
	];

	#[must_use]
	pub const fn label(self) -> &'static str {
		match self {
			Self::All => "All options",
			Self::General => "General and region",
			Self::Network => "Network",
			Self::Desktop => "Desktop and login",
			Self::Hardware => "Hardware and audio",
			Self::Users => "Users and security",
			Self::Storage => "Boot and storage",
			Self::Programs => "Programs and environment",
			Self::Services => "Services and scheduling",
			Self::Virtualization => "Virtualization",
			Self::Nix => "Nix and maintenance",
			Self::System => "System and custom",
		}
	}

	/// Position in [`Category::ALL`].
	#[must_use]
	pub fn index(self) -> usize {
		Self::ALL
			.iter()
			.position(|category| *category == self)
			.unwrap_or_default()
	}

	/// The category after this one, wrapping, or before it when `delta` is
	/// negative.
	#[must_use]
	pub fn cycle(self, delta: isize) -> Self {
		let len = Self::ALL.len() as isize;
		Self::ALL[(self.index() as isize + delta).rem_euclid(len) as usize]
	}

	/// The canonical category of `option`. Every option has exactly one, and
	/// anything uncurated lands in [`Category::System`].
	#[must_use]
	pub fn for_option(option: &SystemOption) -> Self {
		let first = option.path.first().map(String::as_str);
		let second = option.path.get(1).map(String::as_str);
		match (first, second) {
			(Some("networking"), Some("hostName"))
			| (Some("time" | "i18n" | "console" | "fonts" | "documentation"), _) => Self::General,
			(Some("networking"), _)
			| (Some("systemd"), Some("network"))
			| (Some("services"), Some("resolved" | "openssh")) => Self::Network,
			(Some("xdg"), _)
			| (
				Some("services"),
				Some("xserver" | "desktopManager" | "displayManager" | "printing"),
			) => Self::Desktop,
			(Some("hardware" | "powerManagement"), _)
			| (
				Some("services"),
				Some(
					"pipewire"
					| "pulseaudio"
					| "blueman"
					| "udev"
					| "tlp"
					| "power-profiles-daemon",
				),
			) => Self::Hardware,
			(Some("users" | "security"), _) => Self::Users,
			(Some("boot" | "fileSystems" | "swapDevices"), _)
			| (Some("systemd"), Some("mounts" | "automounts"))
			| (Some("services"), Some("fstrim")) => Self::Storage,
			(Some("programs" | "environment"), _) => Self::Programs,
			(Some("services" | "systemd"), _) => Self::Services,
			(Some("virtualisation" | "containers"), _) => Self::Virtualization,
			(Some("nix" | "nixpkgs"), _) | (Some("system"), Some("autoUpgrade")) => Self::Nix,
			_ => Self::System,
		}
	}
}

/// The options a search shows, and how many each category would show.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filtered {
	/// Indices into the schema, exact path matches first.
	pub visible: Vec<usize>,
	/// Matches per category, in [`Category::ALL`] order, ignoring the
	/// selected category. `All` counts every match.
	pub counts: [usize; Category::ALL.len()],
}

impl Filtered {
	#[must_use]
	pub fn count(&self, category: Category) -> usize {
		self.counts[category.index()]
	}
}

/// Searches option paths and descriptions, case-insensitively. Diagnostic
/// options stay hidden unless `show_diagnostic` is set.
#[must_use]
pub fn filter_options(
	options: &[SystemOption],
	text: &str,
	category: Category,
	show_diagnostic: bool,
) -> Filtered {
	let text = text.trim().to_lowercase();
	let mut counts = [0; Category::ALL.len()];
	let mut matches: Vec<(usize, bool)> = Vec::new();
	for (index, option) in options.iter().enumerate() {
		if option.diagnostic && !show_diagnostic {
			continue;
		}
		let path = option.path.join(".").to_lowercase();
		let found = text.is_empty()
			|| path.contains(&text)
			|| option
				.description
				.as_ref()
				.is_some_and(|description| description.to_lowercase().contains(&text));
		if !found {
			continue;
		}
		let own = Category::for_option(option);
		counts[Category::All.index()] += 1;
		counts[own.index()] += 1;
		if category == Category::All || category == own {
			matches.push((index, path == text));
		}
	}
	matches.sort_by_key(|(_, exact)| !exact);
	Filtered {
		visible: matches.into_iter().map(|(index, _)| index).collect(),
		counts,
	}
}

/// The host to open: the previous choice while it still exists, then a host
/// named `nixos`, then the only host. Several hosts need an explicit choice.
#[must_use]
pub fn default_host(hosts: &[String], previous: Option<&str>) -> Option<String> {
	previous
		.and_then(|host| hosts.iter().find(|candidate| candidate.as_str() == host))
		.cloned()
		.or_else(|| hosts.iter().find(|host| host.as_str() == "nixos").cloned())
		.or_else(|| match hosts {
			[host] => Some(host.clone()),
			_ => None,
		})
}

/// An inspected value as display text.
#[must_use]
pub fn value_text(value: &InspectedValue) -> String {
	match value {
		InspectedValue::Available(value) => format_preview(value),
		InspectedValue::Unavailable => "Unavailable. This value could not be evaluated.".into(),
		InspectedValue::Sensitive => {
			"Withheld. Credential values and their defaults are not evaluated.".into()
		}
		InspectedValue::Template => {
			"Nested schema. Concrete instance inspection is not implemented yet.".into()
		}
	}
}

/// Why an option cannot be changed from the browser.
#[must_use]
pub fn access_note(option: &SystemOption) -> &'static str {
	if option.read_only {
		"Declared read-only. This setting cannot be changed here."
	} else {
		"Inspection only. Editing system settings is not available in this version."
	}
}

/// Why an option's deeper schema is missing, when it is.
#[must_use]
pub fn partial_note(option: &SystemOption) -> Option<&'static str> {
	if option.children_unavailable {
		Some(
			"This submodule's child schema could not be evaluated. Its declaration files remain available below.",
		)
	} else if option.children_truncated {
		Some(
			"This branch embeds a complete system or exceeds the four-submodule expansion limit. Its deeper fields are not indexed; consult the declaration files below.",
		)
	} else {
		None
	}
}

/// Short labels for an option's schema flags, in display order.
#[must_use]
pub fn option_tags(option: &SystemOption) -> Vec<&'static str> {
	let mut tags = Vec::new();
	if option.nested {
		tags.push("schema");
	}
	if option.read_only {
		tags.push("read-only");
	}
	if option.children_truncated || option.children_unavailable {
		tags.push("partial");
	}
	if option.diagnostic {
		tags.push("diagnostic");
	}
	tags
}

#[cfg(test)]
mod tests {
	use super::*;

	fn option(path: &str, description: &str, diagnostic: bool) -> SystemOption {
		SystemOption {
			path: path.split('.').map(str::to_string).collect(),
			description: Some(description.into()),
			type_description: "boolean".into(),
			declarations: vec!["configuration.nix".into()],
			diagnostic,
			read_only: false,
			nested: false,
			template: false,
			children_unavailable: false,
			children_truncated: false,
		}
	}

	#[test]
	fn host_selection_preserves_a_valid_choice_and_requires_ambiguous_choice() {
		let hosts = vec!["laptop".into(), "workstation".into()];
		assert_eq!(default_host(&hosts, None), None);
		assert_eq!(default_host(&hosts, Some("deleted")), None);
		assert_eq!(
			default_host(&hosts, Some("workstation")),
			Some("workstation".into())
		);
		assert_eq!(
			default_host(&["laptop".into()], None),
			Some("laptop".into())
		);
		assert_eq!(
			default_host(&["laptop".into(), "nixos".into()], None),
			Some("nixos".into())
		);
	}

	#[test]
	fn search_includes_descriptions_custom_namespaces_and_diagnostic_options() {
		let options = vec![
			option(
				"services.pipewire.enable",
				"custom.welcome configures greetings",
				false,
			),
			option("custom.welcome", "Welcome message", false),
			option("custom.debug", "Internal welcome behavior", true),
		];
		let filtered = filter_options(&options, "CUSTOM.WELCOME", Category::All, false);
		assert_eq!(
			filtered.visible,
			[1, 0],
			"Exact paths precede description matches"
		);
		let filtered = filter_options(&options, "welcome", Category::All, false);
		assert_eq!(filtered.visible, [0, 1]);
		let filtered = filter_options(&options, "welcome", Category::All, true);
		assert_eq!(filtered.visible, [0, 1, 2]);
		let filtered = filter_options(&options, "", Category::System, true);
		assert_eq!(
			filtered.visible,
			[1, 2],
			"Custom options remain discoverable"
		);
		assert_eq!(filtered.count(Category::All), 3);
		assert_eq!(filtered.count(Category::Hardware), 1);
		assert_eq!(filtered.count(Category::System), 2);
	}

	#[test]
	fn categories_cycle_in_both_directions() {
		assert_eq!(Category::All.cycle(1), Category::General);
		assert_eq!(Category::All.cycle(-1), Category::System);
		assert_eq!(Category::System.cycle(1), Category::All);
	}
}
