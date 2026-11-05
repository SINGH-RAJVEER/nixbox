//! Applicability rules for the settings browser. Hardware facts belong to
//! the selected host only after the user explicitly links this machine.

use std::collections::BTreeSet;

use nixbox_nix::hardware::{HardwareInventory, Presence};
use nixbox_nix::settings::OptionPath;
use nixbox_nix::system_options::SystemOption;

#[derive(Debug, Clone, Default)]
pub struct HardwareFilter {
	/// None means this host has no associated hardware inventory.
	pub inventory: Option<HardwareInventory>,
	pub show_all: bool,
	/// Staged changes must remain reachable even when hardware is absent.
	pub retained: BTreeSet<OptionPath>,
}

impl HardwareFilter {
	#[must_use]
	pub fn includes(&self, option: &SystemOption) -> bool {
		self.show_all
			|| option.configured != Some(false)
			|| self
				.retained
				.iter()
				.any(|path| path.starts_with(&option.path) || option.path.starts_with(path))
			|| self.exclusion_reason(&option.path).is_none()
	}

	/// The hardware mismatch, independently of configured/staged exceptions.
	#[must_use]
	pub fn exclusion_reason(&self, path: &[String]) -> Option<&'static str> {
		let facts = self.inventory.as_ref()?;
		let under = |prefix: &[&str]| {
			path.iter()
				.map(String::as_str)
				.take(prefix.len())
				.eq(prefix.iter().copied())
		};
		let (state, reason) = if under(&["hardware", "cpu", "intel"]) {
			(facts.intel_cpu, "No Intel CPU detected")
		} else if under(&["hardware", "cpu", "amd"]) {
			(facts.amd_cpu, "No AMD CPU detected")
		} else if under(&["hardware", "nvidia", "prime"])
			&& facts.nvidia_gpu == Presence::Present
			&& facts.intel_gpu == Presence::Absent
			&& facts.amd_gpu == Presence::Absent
		{
			(Presence::Absent, "No Intel or AMD GPU detected for PRIME")
		} else if under(&["hardware", "nvidia"])
			|| under(&["hardware", "nvidia-container-toolkit"])
			|| under(&["services", "nvidia-optimus"])
		{
			(facts.nvidia_gpu, "No NVIDIA GPU detected")
		} else if under(&["hardware", "amdgpu"]) {
			(facts.amd_gpu, "No AMD GPU detected")
		} else if under(&["hardware", "intel-gpu"]) {
			(facts.intel_gpu, "No Intel GPU detected")
		} else if under(&["hardware", "bluetooth"]) || under(&["services", "blueman"]) {
			(facts.bluetooth, "No Bluetooth adapter detected")
		} else if under(&["services", "tuned", "ppdSettings", "battery"])
			|| under(&[
				"services",
				"tuned",
				"ppdSettings",
				"main",
				"battery_detection",
			]) || under(&["services", "undervolt", "tempBat"])
		{
			(facts.battery, "No battery detected")
		} else {
			return None;
		};
		(state == Presence::Absent).then_some(reason)
	}
}
