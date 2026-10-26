//! Read-only hardware facts. Missing or incomplete probes stay unknown so a
//! settings browser never treats a failed probe as evidence of absence.

use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Presence {
	Present,
	Absent,
	#[default]
	Unknown,
}

impl Presence {
	#[must_use]
	pub const fn label(self) -> &'static str {
		match self {
			Self::Present => "detected",
			Self::Absent => "not detected",
			Self::Unknown => "unknown",
		}
	}
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HardwareInventory {
	pub intel_cpu: Presence,
	pub amd_cpu: Presence,
	pub intel_gpu: Presence,
	pub amd_gpu: Presence,
	pub nvidia_gpu: Presence,
	pub bluetooth: Presence,
	pub battery: Presence,
}

impl HardwareInventory {
	/// Probes this machine only. Callers must bind these facts to a selected
	/// configuration explicitly before using them to filter that host.
	#[must_use]
	pub fn detect() -> Self {
		Self::read(Path::new("/sys"), Path::new("/proc/cpuinfo"))
	}

	fn read(sys: &Path, cpuinfo: &Path) -> Self {
		let mut facts = Self::default();
		if let Ok(cpu) = fs::read_to_string(cpuinfo) {
			let vendors: Vec<_> = cpu
				.lines()
				.filter_map(|line| {
					let (key, value) = line.split_once(':')?;
					(key.trim() == "vendor_id").then_some(value.trim())
				})
				.collect();
			if !vendors.is_empty()
				&& vendors
					.iter()
					.all(|vendor| matches!(*vendor, "GenuineIntel" | "AuthenticAMD"))
			{
				facts.intel_cpu = presence(vendors.contains(&"GenuineIntel"));
				facts.amd_cpu = presence(vendors.contains(&"AuthenticAMD"));
			}
		}

		let pci = pci_devices(&sys.join("bus/pci/devices"));
		let pc_cpu = facts.intel_cpu != Presence::Unknown || facts.amd_cpu != Presence::Unknown;
		if let Some(devices) = &pci {
			// An empty PCI tree does not describe platform GPUs, for example on
			// ARM machines. Do not infer their absence from it.
			if !devices.is_empty() {
				facts.intel_gpu = gpu_presence(
					pc_cpu,
					devices
						.iter()
						.any(|(class, vendor)| class >> 16 == 3 && *vendor == 0x8086),
				);
				facts.amd_gpu = gpu_presence(
					pc_cpu,
					devices
						.iter()
						.any(|(class, vendor)| class >> 16 == 3 && *vendor == 0x1002),
				);
				facts.nvidia_gpu = gpu_presence(
					pc_cpu,
					devices
						.iter()
						.any(|(class, vendor)| class >> 16 == 3 && *vendor == 0x10de),
				);
			}
		}
		facts.bluetooth = bluetooth(sys, pci.as_deref(), pc_cpu);
		facts.battery = battery(sys);
		facts
	}

	#[must_use]
	pub fn summary(&self) -> String {
		let cpu = match (self.intel_cpu, self.amd_cpu) {
			(Presence::Present, Presence::Present) => "Intel + AMD",
			(Presence::Present, _) => "Intel",
			(_, Presence::Present) => "AMD",
			_ => "unknown",
		};
		let mut gpus = Vec::new();
		for (name, state) in [
			("Intel", self.intel_gpu),
			("AMD", self.amd_gpu),
			("NVIDIA", self.nvidia_gpu),
		] {
			if state == Presence::Present {
				gpus.push(name);
			}
		}
		let graphics = if gpus.is_empty() {
			"unknown".into()
		} else {
			gpus.join(" + ")
		};
		format!(
			"CPU {cpu}; GPU {graphics}; Bluetooth {}; battery {}",
			self.bluetooth.label(),
			self.battery.label()
		)
	}
}

const fn presence(found: bool) -> Presence {
	if found {
		Presence::Present
	} else {
		Presence::Absent
	}
}

const fn gpu_presence(pc_cpu: bool, found: bool) -> Presence {
	if found {
		Presence::Present
	} else if pc_cpu {
		Presence::Absent
	} else {
		Presence::Unknown
	}
}

fn hex(path: &Path) -> Option<u32> {
	let value = fs::read_to_string(path).ok()?;
	u32::from_str_radix(value.trim().trim_start_matches("0x"), 16).ok()
}

fn pci_devices(root: &Path) -> Option<Vec<(u32, u32)>> {
	fs::read_dir(root)
		.ok()?
		.map(|entry| {
			let path = entry.ok()?.path();
			Some((hex(&path.join("class"))?, hex(&path.join("vendor"))?))
		})
		.collect()
}

fn bluetooth(sys: &Path, pci: Option<&[(u32, u32)]>, pc_cpu: bool) -> Presence {
	if fs::read_dir(sys.join("class/bluetooth"))
		.is_ok_and(|mut entries| entries.any(|entry| entry.is_ok()))
		|| pci.is_some_and(|devices| devices.iter().any(|(class, _)| class >> 8 == 0x0d11))
	{
		return Presence::Present;
	}
	let Ok(entries) = fs::read_dir(sys.join("bus/usb/devices")) else {
		return Presence::Unknown;
	};
	let mut complete = pc_cpu && pci.is_some_and(|devices| !devices.is_empty());
	for entry in entries {
		let Ok(entry) = entry else {
			complete = false;
			continue;
		};
		let path = entry.path();
		let interface = entry.file_name().to_string_lossy().contains(':');
		let prefix = if interface { "bInterface" } else { "bDevice" };
		let class = hex(&path.join(format!("{prefix}Class")));
		if class == Some(0xe0)
			&& hex(&path.join(format!("{prefix}SubClass"))) == Some(1)
			&& hex(&path.join(format!("{prefix}Protocol"))) == Some(1)
		{
			return Presence::Present;
		}
		// Vendor-specific USB interfaces can be Bluetooth without advertising
		// the standard class. Driver state alone cannot settle that case.
		if class.is_none() || class == Some(0xff) || class == Some(0xe0) {
			complete = false;
		}
	}
	if complete {
		Presence::Absent
	} else {
		Presence::Unknown
	}
}

fn battery(sys: &Path) -> Presence {
	let Ok(entries) = fs::read_dir(sys.join("class/power_supply")) else {
		return Presence::Unknown;
	};
	for entry in entries {
		let Some(kind) = entry
			.ok()
			.and_then(|entry| fs::read_to_string(entry.path().join("type")).ok())
		else {
			return Presence::Unknown;
		};
		if kind.trim() == "Battery" {
			return Presence::Present;
		}
	}
	// An empty power-supply class on a laptop may mean a missing driver.
	// Only infer no battery for known desktop/server chassis types.
	match fs::read_to_string(sys.join("class/dmi/id/chassis_type"))
		.ok()
		.and_then(|kind| kind.trim().parse::<u32>().ok())
	{
		Some(3..=7 | 13 | 15..=17 | 23 | 24) => Presence::Absent,
		_ => Presence::Unknown,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::atomic::{AtomicU64, Ordering};

	static NEXT: AtomicU64 = AtomicU64::new(0);

	struct Fixture(std::path::PathBuf);

	impl Fixture {
		fn new() -> Self {
			let root = std::env::temp_dir().join(format!(
				"nixbox-hardware-{}-{}",
				std::process::id(),
				NEXT.fetch_add(1, Ordering::Relaxed)
			));
			fs::create_dir_all(&root).unwrap();
			Self(root)
		}
		fn write(&self, path: &str, value: &str) {
			let path = self.0.join(path);
			fs::create_dir_all(path.parent().unwrap()).unwrap();
			fs::write(path, value).unwrap();
		}
		fn read(&self) -> HardwareInventory {
			HardwareInventory::read(&self.0, &self.0.join("cpuinfo"))
		}
	}

	impl Drop for Fixture {
		fn drop(&mut self) {
			let _ = fs::remove_dir_all(&self.0);
		}
	}

	#[test]
	fn detects_both_gpus_and_unbound_bluetooth_without_confusing_cpu_and_gpu() {
		let fixture = Fixture::new();
		fixture.write("cpuinfo", "vendor_id : AuthenticAMD\n");
		fixture.write("bus/pci/devices/0/class", "0x030000\n");
		fixture.write("bus/pci/devices/0/vendor", "0x8086\n");
		fixture.write("bus/pci/devices/1/class", "0x030200\n");
		fixture.write("bus/pci/devices/1/vendor", "0x10de\n");
		fixture.write("bus/usb/devices/1-1:1.0/bInterfaceClass", "e0");
		fixture.write("bus/usb/devices/1-1:1.0/bInterfaceSubClass", "01");
		fixture.write("bus/usb/devices/1-1:1.0/bInterfaceProtocol", "01");
		fixture.write("class/power_supply/BAT0/type", "Battery\n");
		let facts = fixture.read();
		assert_eq!(facts.amd_cpu, Presence::Present);
		assert_eq!(facts.intel_cpu, Presence::Absent);
		assert_eq!(facts.intel_gpu, Presence::Present);
		assert_eq!(facts.nvidia_gpu, Presence::Present);
		assert_eq!(facts.amd_gpu, Presence::Absent);
		assert_eq!(facts.bluetooth, Presence::Present);
		assert_eq!(facts.battery, Presence::Present);
	}

	#[test]
	fn failed_or_partial_probes_do_not_claim_absence() {
		let fixture = Fixture::new();
		assert_eq!(fixture.read(), HardwareInventory::default());
		fixture.write("bus/pci/devices/0/class", "0x030000");
		fixture.write("class/power_supply/AC/type", "Mains");
		fixture.write("class/dmi/id/chassis_type", "10");
		assert_eq!(fixture.read(), HardwareInventory::default());
		fixture.write("bus/pci/devices/0/vendor", "0x1002");
		fixture.write("bus/usb/devices/1-1/bDeviceClass", "ff");
		assert_eq!(fixture.read().bluetooth, Presence::Unknown);
		fixture.write("class/dmi/id/chassis_type", "3");
		assert_eq!(fixture.read().battery, Presence::Absent);
	}

	#[test]
	fn platform_hardware_is_not_excluded_by_a_pci_only_scan() {
		let fixture = Fixture::new();
		fixture.write("cpuinfo", "CPU architecture : 8\n");
		fixture.write("bus/pci/devices/0/class", "0x030000");
		fixture.write("bus/pci/devices/0/vendor", "0x1002");
		fs::create_dir_all(fixture.0.join("bus/usb/devices")).unwrap();
		let facts = fixture.read();
		assert_eq!(facts.amd_gpu, Presence::Present);
		assert_eq!(facts.nvidia_gpu, Presence::Unknown);
		assert_eq!(facts.intel_gpu, Presence::Unknown);
		assert_eq!(facts.bluetooth, Presence::Unknown);
	}
}
