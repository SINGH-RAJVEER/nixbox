use std::time::Duration;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const MAX_REPORT_BYTES: usize = 8 * 1024;
pub const MAX_GROUPS: usize = 32;
pub const MAX_COUNT: u16 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Interface {
	Gui,
	Tui,
	Cli,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OsFamily {
	Linux,
	Macos,
	Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Architecture {
	X86_64,
	Aarch64,
	Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
	HomeManager,
	Nixos,
}

impl From<nixbox_config::Target> for Scope {
	fn from(value: nixbox_config::Target) -> Self {
		match value {
			nixbox_config::Target::HomeManager => Self::HomeManager,
			nixbox_config::Target::NixosSystem => Self::Nixos,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActivationKind {
	Configuration,
	Packages,
}

impl From<crate::Activation> for ActivationKind {
	fn from(value: crate::Activation) -> Self {
		match value {
			crate::Activation::Configuration => Self::Configuration,
			crate::Activation::Packages => Self::Packages,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
	Succeeded,
	Failed,
	Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DurationBucket {
	Under10Seconds,
	Under1Minute,
	Under5Minutes,
	Under15Minutes,
	AtLeast15Minutes,
}

impl From<Duration> for DurationBucket {
	fn from(value: Duration) -> Self {
		match value.as_secs() {
			0..10 => Self::Under10Seconds,
			10..60 => Self::Under1Minute,
			60..300 => Self::Under5Minutes,
			300..900 => Self::Under15Minutes,
			_ => Self::AtLeast15Minutes,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum Observation {
	AppStarted {},
	ActivationFinished {
		target: Scope,
		activation: ActivationKind,
		outcome: Outcome,
		duration: DurationBucket,
	},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Counter {
	pub observation: Observation,
	pub count: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
	pub schema_version: u8,
	pub app_version: String,
	pub interface: Interface,
	pub os: OsFamily,
	pub arch: Architecture,
	pub counters: Vec<Counter>,
}

impl Report {
	pub fn validate(&self) -> Result<()> {
		ensure!(self.schema_version == 1, "unsupported schema version");
		let parts: Vec<_> = self.app_version.split('.').collect();
		ensure!(
			parts.len() == 3
				&& parts.iter().all(|part| !part.is_empty()
					&& part.len() <= 5
					&& part.bytes().all(|byte| byte.is_ascii_digit())
					&& part
						.parse::<u16>()
						.is_ok_and(|number| number.to_string() == *part)),
			"invalid release version"
		);
		ensure!(
			!self.counters.is_empty() && self.counters.len() <= MAX_GROUPS,
			"invalid counter groups"
		);
		for (index, counter) in self.counters.iter().enumerate() {
			ensure!(
				(1..=MAX_COUNT).contains(&counter.count),
				"invalid counter value"
			);
			ensure!(
				!self
					.counters
					.iter()
					.take(index)
					.any(|previous| previous.observation == counter.observation),
				"duplicate counter group"
			);
		}
		ensure!(
			serde_json::to_vec(self)?.len() <= MAX_REPORT_BYTES,
			"report too large"
		);
		Ok(())
	}

	pub(super) fn current(interface: Interface) -> Self {
		Self {
			schema_version: 1,
			app_version: concat!(
				env!("CARGO_PKG_VERSION_MAJOR"),
				".",
				env!("CARGO_PKG_VERSION_MINOR"),
				".",
				env!("CARGO_PKG_VERSION_PATCH")
			)
			.into(),
			interface,
			os: match std::env::consts::OS {
				"linux" => OsFamily::Linux,
				"macos" => OsFamily::Macos,
				_ => OsFamily::Other,
			},
			arch: match std::env::consts::ARCH {
				"x86_64" => Architecture::X86_64,
				"aarch64" => Architecture::Aarch64,
				_ => Architecture::Other,
			},
			counters: Vec::new(),
		}
	}
}

/// An offline, synthetic example, using the same serializer as actual reports.
pub fn preview() -> Result<String> {
	let report = Report {
		schema_version: 1,
		app_version: "0.4.0".into(),
		interface: Interface::Gui,
		os: OsFamily::Linux,
		arch: Architecture::X86_64,
		counters: vec![
			Counter {
				observation: Observation::AppStarted {},
				count: 1,
			},
			Counter {
				observation: Observation::ActivationFinished {
					target: Scope::Nixos,
					activation: ActivationKind::Configuration,
					outcome: Outcome::Succeeded,
					duration: DurationBucket::Under5Minutes,
				},
				count: 1,
			},
		],
	};
	report.validate()?;
	Ok(serde_json::to_string_pretty(&report)?)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn schema_rejects_extra_sensitive_fields_in_every_object() {
		let value: serde_json::Value =
			serde_json::from_str(&preview().expect("preview")).expect("JSON");
		for pointer in [
			"",
			"/counters/0",
			"/counters/0/observation",
			"/counters/1/observation",
		] {
			let mut invalid = value.clone();
			invalid
				.pointer_mut(pointer)
				.expect("object")
				.as_object_mut()
				.expect("object")
				.insert("secret_path".into(), "/home/private/token".into());
			assert!(
				serde_json::from_value::<Report>(invalid).is_err(),
				"accepted extra field at {pointer}"
			);
		}
	}

	#[test]
	fn report_validation_rejects_versions_counts_and_duplicate_groups() {
		let original: Report = serde_json::from_str(&preview().expect("preview")).expect("report");
		for version in [
			"1.0.0+private-host",
			"1.2",
			"1.2.-3",
			"65536.1.1",
			"00001.0.0",
		] {
			let mut report = original.clone();
			report.app_version = version.into();
			assert!(report.validate().is_err());
		}
		for count in [0, MAX_COUNT + 1] {
			let mut report = original.clone();
			report.counters.first_mut().expect("counter").count = count;
			assert!(report.validate().is_err());
		}
		let mut report = original;
		report
			.counters
			.push(report.counters.first().expect("counter").clone());
		assert!(report.validate().is_err());
	}

	#[test]
	fn duration_boundaries_are_coarse_and_monotonic() {
		for (seconds, expected) in [
			(9, DurationBucket::Under10Seconds),
			(10, DurationBucket::Under1Minute),
			(60, DurationBucket::Under5Minutes),
			(300, DurationBucket::Under15Minutes),
			(900, DurationBucket::AtLeast15Minutes),
		] {
			assert_eq!(DurationBucket::from(Duration::from_secs(seconds)), expected);
		}
	}
}
