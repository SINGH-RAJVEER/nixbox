//! The NixOS settings browser's policy, shared by every front-end: how the
//! schema is grouped into categories, how search ranks matches, which host
//! opens by default, how an inspection reads, and which options the editor
//! shows together and may change.

use nixbox_nix::settings::OptionPath;
use nixbox_nix::system_options::{EditableOption, InspectedValue, SystemOption, format_preview};

use crate::hardware::HardwareFilter;
use crate::options::{NIXOS_HOST, locked_reason};

/// The most options one editor group evaluates at once.
pub const GROUP_LIMIT: usize = 48;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Category {
	#[default]
	All,
	Configured,
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
	pub const ALL: [Self; 13] = [
		Self::All,
		Self::Configured,
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
			Self::Configured => "Configured options",
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

	/// The canonical topic category of `option`. Every option has exactly one, and
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
	/// selected category. `Configured` includes configured diagnostic options;
	/// the other counts respect `show_diagnostic`.
	pub counts: [usize; Category::ALL.len()],
}

impl Filtered {
	#[must_use]
	pub fn count(&self, category: Category) -> usize {
		self.counts[category.index()]
	}
}

/// Searches option paths and descriptions, case-insensitively. Diagnostic
/// options stay hidden unless `show_diagnostic` is set or the Configured
/// category is selected and the option has a concrete configured definition.
#[must_use]
pub fn filter_options(
	options: &[SystemOption],
	text: &str,
	category: Category,
	show_diagnostic: bool,
	hardware: &HardwareFilter,
) -> Filtered {
	let text = text.trim().to_lowercase();
	let mut counts = [0; Category::ALL.len()];
	let mut matches: Vec<(usize, bool)> = Vec::new();
	for (index, option) in options.iter().enumerate() {
		if !hardware.includes(option) {
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
		let configured = option.is_configured();
		if configured {
			counts[Category::Configured.index()] += 1;
		}
		let ordinary = !option.diagnostic || show_diagnostic;
		if ordinary {
			counts[Category::All.index()] += 1;
			counts[own.index()] += 1;
		}
		if (category == Category::Configured && configured)
			|| (ordinary && (category == Category::All || category == own))
		{
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
		.or_else(|| {
			hosts
				.iter()
				.find(|host| host.as_str() == crate::options::NIXOS_HOST)
				.cloned()
		})
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

/// Why an option cannot be changed from the terminal browser, which only
/// inspects.
#[must_use]
pub fn access_note(option: &SystemOption) -> &'static str {
	if option.read_only {
		"Declared read-only. This setting cannot be changed here."
	} else {
		"Inspection only. Edit system settings in the desktop app."
	}
}

/// The options the editor shows with `selected`: the siblings under its
/// parent that the browser lists, in schema order, keeping the nearest
/// [`GROUP_LIMIT`] when there are more. `selected` is always included.
#[must_use]
pub fn sibling_group(
	options: &[SystemOption],
	selected: &[String],
	show_diagnostic: bool,
	hardware: &HardwareFilter,
) -> Vec<OptionPath> {
	let Some((_, parent)) = selected.split_last() else {
		return Vec::new();
	};
	let siblings: Vec<&OptionPath> = options
		.iter()
		.filter(|option| {
			option.path == selected
				|| (option.path.len() == selected.len()
					&& option.path.starts_with(parent)
					&& !option.template
					&& hardware.includes(option)
					&& (show_diagnostic || !option.diagnostic))
		})
		.map(|option| &option.path)
		.collect();
	let at = siblings
		.iter()
		.position(|path| path.as_slice() == selected)
		.unwrap_or(0);
	let start = at
		.saturating_sub(GROUP_LIMIT / 2)
		.min(siblings.len().saturating_sub(GROUP_LIMIT));
	siblings
		.into_iter()
		.skip(start)
		.take(GROUP_LIMIT)
		.cloned()
		.collect()
}

/// Whether nixbox's NixOS rebuild applies `host`'s configuration, so its
/// options may be edited.
#[must_use]
pub fn host_is_editable(host: &str) -> bool {
	host == NIXOS_HOST
}

/// Why an inspected option cannot be edited, or `None` when it can.
#[must_use]
pub fn edit_lock(option: &EditableOption) -> Option<String> {
	match &option.details.value {
		InspectedValue::Sensitive => Some("credential value".into()),
		InspectedValue::Unavailable => Some("value could not be evaluated".into()),
		InspectedValue::Template => Some("nested schema".into()),
		InspectedValue::Available(_) => locked_reason(&option.entry).or_else(|| {
			option
				.entry
				.value
				.is_none()
				.then(|| "value too large to edit here".into())
		}),
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
	if option.is_configured() {
		tags.push("configured");
	}
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
			configured: Some(false),
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
		let filtered = filter_options(
			&options,
			"CUSTOM.WELCOME",
			Category::All,
			false,
			&HardwareFilter::default(),
		);
		assert_eq!(
			filtered.visible,
			[1, 0],
			"Exact paths precede description matches"
		);
		let filtered = filter_options(
			&options,
			"welcome",
			Category::All,
			false,
			&HardwareFilter::default(),
		);
		assert_eq!(filtered.visible, [0, 1]);
		let filtered = filter_options(
			&options,
			"welcome",
			Category::All,
			true,
			&HardwareFilter::default(),
		);
		assert_eq!(filtered.visible, [0, 1, 2]);
		let filtered = filter_options(
			&options,
			"",
			Category::System,
			true,
			&HardwareFilter::default(),
		);
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
	fn configured_view_uses_provenance_and_excludes_schema_templates() {
		let mut options = vec![
			option("hardware.nvidia.open", "Configured hardware", false),
			option("custom.default", "An option default", false),
			option("custom.unknown", "Unavailable provenance", false),
			option("custom.internal", "Configured diagnostic", true),
			option(
				"users.users.<name>.enable",
				"Inherited parent metadata",
				false,
			),
		];
		options[0].configured = Some(true);
		options[2].configured = None;
		options[3].configured = Some(true);
		options[4].configured = Some(true);
		options[4].nested = true;
		options[4].template = true;
		let hardware = HardwareFilter {
			inventory: Some(nixbox_nix::hardware::HardwareInventory {
				nvidia_gpu: nixbox_nix::hardware::Presence::Absent,
				..Default::default()
			}),
			..Default::default()
		};
		let filtered = filter_options(&options, "", Category::Configured, false, &hardware);
		assert_eq!(filtered.visible, [0, 3]);
		assert_eq!(filtered.count(Category::Configured), 2);
		assert_eq!(filtered.count(Category::All), 4);
		assert_eq!(option_tags(&options[0]), ["configured"]);
		assert!(!option_tags(&options[4]).contains(&"configured"));
		let searched = filter_options(&options, "internal", Category::Configured, false, &hardware);
		assert_eq!(searched.visible, [3]);
		assert_eq!(searched.count(Category::Configured), 1);
		assert_eq!(
			filter_options(&options, "", Category::All, false, &hardware).visible,
			[0, 1, 2, 4]
		);
	}

	#[test]
	fn categories_cycle_in_both_directions() {
		assert_eq!(Category::All.cycle(1), Category::Configured);
		assert_eq!(Category::Configured.cycle(1), Category::General);
		assert_eq!(Category::All.cycle(-1), Category::System);
		assert_eq!(Category::System.cycle(1), Category::All);
	}

	#[test]
	fn hardware_filter_preserves_configured_staged_unknown_and_general_options() {
		use nixbox_nix::hardware::{HardwareInventory, Presence};
		let mut options = vec![
			option("hardware.cpu.intel.updateMicrocode", "Intel", false),
			option("hardware.cpu.amd.updateMicrocode", "AMD", false),
			option("hardware.nvidia.modesetting.enable", "NVIDIA", false),
			option("hardware.bluetooth.enable", "Bluetooth", false),
			option("services.undervolt.tempBat", "Battery", false),
			option("custom.hardwarePolicy", "Custom", false),
			option("services.pipewire.enable", "Audio", false),
		];
		let mut hardware = HardwareFilter {
			inventory: Some(HardwareInventory {
				intel_cpu: Presence::Absent,
				amd_cpu: Presence::Present,
				nvidia_gpu: Presence::Absent,
				bluetooth: Presence::Present,
				battery: Presence::Absent,
				..Default::default()
			}),
			..Default::default()
		};
		let filtered = filter_options(&options, "", Category::All, false, &hardware);
		assert_eq!(filtered.visible, [1, 3, 5, 6]);
		assert_eq!(filtered.count(Category::All), 4);
		assert_eq!(filtered.count(Category::Hardware), 3);
		assert!(
			filter_options(&options, "NVIDIA", Category::All, false, &hardware)
				.visible
				.is_empty()
		);

		options[0].configured = Some(true);
		options[2].configured = None;
		hardware.retained.insert(options[4].path.clone());
		assert_eq!(
			filter_options(&options, "", Category::All, false, &hardware)
				.visible
				.len(),
			options.len()
		);
		options[0].configured = Some(false);
		options[2].configured = Some(false);
		hardware.retained.clear();
		hardware.show_all = true;
		assert_eq!(
			filter_options(&options, "", Category::All, false, &hardware)
				.visible
				.len(),
			options.len()
		);
		hardware.show_all = false;
		hardware.inventory = None;
		assert_eq!(
			filter_options(&options, "", Category::All, false, &hardware)
				.visible
				.len(),
			options.len()
		);
		hardware.inventory = Some(HardwareInventory::default());
		assert_eq!(
			filter_options(&options, "", Category::All, false, &hardware)
				.visible
				.len(),
			options.len()
		);
	}

	#[test]
	fn prime_requires_nvidia_and_another_supported_gpu() {
		use nixbox_nix::hardware::{HardwareInventory, Presence};
		let prime = option("hardware.nvidia.prime.offload.enable", "", false);
		let nvidia = option("hardware.nvidia.modesetting.enable", "", false);
		let mut hardware = HardwareFilter {
			inventory: Some(HardwareInventory {
				nvidia_gpu: Presence::Present,
				intel_gpu: Presence::Absent,
				amd_gpu: Presence::Absent,
				..Default::default()
			}),
			..Default::default()
		};
		assert!(!hardware.includes(&prime));
		assert!(hardware.includes(&nvidia));
		hardware.inventory.as_mut().unwrap().intel_gpu = Presence::Unknown;
		assert!(hardware.includes(&prime));
		hardware.inventory.as_mut().unwrap().intel_gpu = Presence::Present;
		assert!(hardware.includes(&prime));
	}

	#[test]
	fn editor_groups_apply_the_same_hardware_rules() {
		use nixbox_nix::hardware::{HardwareInventory, Presence};
		let options = vec![
			option("services.undervolt.temp", "", false),
			option("services.undervolt.tempBat", "", false),
		];
		let hardware = HardwareFilter {
			inventory: Some(HardwareInventory {
				battery: Presence::Absent,
				..Default::default()
			}),
			..Default::default()
		};
		assert_eq!(
			sibling_group(&options, &options[0].path, false, &hardware),
			[options[0].path.clone()]
		);
	}

	#[test]
	fn groups_are_listed_siblings_around_the_selection() {
		let mut options = vec![
			option("services.foo.enable", "", false),
			option("services.foo.settings.port", "", false),
			option("services.foo.debug", "", true),
			option("services.foo.user", "", false),
			option("services.bar.enable", "", false),
		];
		let selected: OptionPath = options[3].path.clone();
		let names = |group: Vec<OptionPath>| -> Vec<String> {
			group.iter().map(|path| path.join(".")).collect()
		};
		assert_eq!(
			names(sibling_group(
				&options,
				&selected,
				false,
				&HardwareFilter::default()
			)),
			["services.foo.enable", "services.foo.user"]
		);
		assert_eq!(
			sibling_group(&options, &selected, true, &HardwareFilter::default()).len(),
			3
		);

		options = (0..GROUP_LIMIT * 2)
			.map(|index| option(&format!("big.option{index:03}"), "", false))
			.collect();
		let group = sibling_group(
			&options,
			&options[GROUP_LIMIT * 2 - 1].path,
			false,
			&HardwareFilter::default(),
		);
		assert_eq!(group.len(), GROUP_LIMIT);
		assert_eq!(group.last(), Some(&options[GROUP_LIMIT * 2 - 1].path));
	}

	#[test]
	fn edits_are_locked_by_value_and_ownership() {
		use nixbox_nix::options::{OptionEntry, OptionKind};
		use nixbox_nix::system_options::SystemOptionDetails;

		let editable = |value: InspectedValue, plain: Option<serde_json::Value>| EditableOption {
			details: SystemOptionDetails {
				path: vec!["custom".into(), "enable".into()],
				value,
				default: None,
				example: None,
				definitions: Some(Vec::new()),
			},
			entry: OptionEntry {
				path: vec!["custom".into(), "enable".into()],
				kind: OptionKind::Bool,
				type_description: "boolean".into(),
				description: None,
				default: None,
				example: None,
				value: plain,
				defined_in: Vec::new(),
				set_by_nixbox: false,
				set_by_module: false,
				read_only: false,
			},
		};
		let available = InspectedValue::Available(serde_json::Value::Bool(false));
		assert_eq!(
			edit_lock(&editable(
				available.clone(),
				Some(serde_json::Value::Bool(false))
			)),
			None
		);
		assert!(edit_lock(&editable(InspectedValue::Sensitive, None)).is_some());
		assert!(edit_lock(&editable(available.clone(), None)).is_some());
		let mut owned = editable(available, Some(serde_json::Value::Bool(false)));
		owned.entry.defined_in = vec!["configuration.nix".into()];
		assert_eq!(
			edit_lock(&owned).as_deref(),
			Some("set in configuration.nix")
		);
		assert!(host_is_editable(NIXOS_HOST));
		assert!(!host_is_editable("workstation"));
	}
}
