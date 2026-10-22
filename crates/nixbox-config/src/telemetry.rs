use std::ffi::OsStr;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Version of the basic usage collection disclosure. New data or recipients require new consent.
pub const TELEMETRY_CONSENT_VERSION: u32 = 3;

/// Saved consent only. This value does not create a collector or perform network work.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TelemetrySettings {
	pub enabled: bool,
	pub consent_version: u32,
	/// Local consent generation. Never include it in reports.
	pub revision: u64,
}

/// Effective consent and the reason collection is suppressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TelemetryStatus {
	Enabled,
	Disabled,
	ConsentRequired,
	DisabledByEnvironment,
	DisabledByDoNotTrack,
	DevelopmentBuild,
	ContinuousIntegration,
	VirtualMachine,
	ReceiverNotConfigured,
}

impl TelemetrySettings {
	/// Resolves saved consent with the process's disabling overrides.
	pub fn status(self) -> TelemetryStatus {
		self.status_with(Overrides::from_env())
	}

	pub(crate) fn changed(self, enabled: bool) -> Result<Self> {
		Ok(Self {
			enabled,
			consent_version: if enabled {
				TELEMETRY_CONSENT_VERSION
			} else {
				self.consent_version
			},
			revision: self
				.revision
				.checked_add(1)
				.context("telemetry consent revision exhausted")?,
		})
	}

	fn status_with(self, overrides: Overrides) -> TelemetryStatus {
		if !self.enabled {
			TelemetryStatus::Disabled
		} else if self.consent_version != TELEMETRY_CONSENT_VERSION {
			TelemetryStatus::ConsentRequired
		} else if overrides.environment {
			TelemetryStatus::DisabledByEnvironment
		} else if overrides.do_not_track {
			TelemetryStatus::DisabledByDoNotTrack
		} else if overrides.development {
			TelemetryStatus::DevelopmentBuild
		} else if overrides.ci {
			TelemetryStatus::ContinuousIntegration
		} else if overrides.vm {
			TelemetryStatus::VirtualMachine
		} else {
			TelemetryStatus::Enabled
		}
	}
}

#[derive(Default, Clone, Copy)]
struct Overrides {
	environment: bool,
	do_not_track: bool,
	development: bool,
	ci: bool,
	vm: bool,
}

impl Overrides {
	fn from_env() -> Self {
		Self {
			environment: std::env::var_os("NIXBOX_NO_TELEMETRY").as_deref()
				== Some(OsStr::new("1")),
			do_not_track: std::env::var_os("DO_NOT_TRACK").as_deref() == Some(OsStr::new("1")),
			development: cfg!(debug_assertions) || cfg!(test),
			ci: std::env::var_os("CI").is_some(),
			vm: std::env::var_os("NIXBOX_VM_BUILD").is_some(),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_explicit_current_consent_enables_collection() {
		let settings = TelemetrySettings::default();
		assert_eq!(
			settings.status_with(Overrides::default()),
			TelemetryStatus::Disabled
		);
		let enabled = settings.changed(true).expect("enable consent");
		assert_eq!(
			enabled.status_with(Overrides::default()),
			TelemetryStatus::Enabled
		);
		for version in [0, 1, 2, TELEMETRY_CONSENT_VERSION + 1] {
			assert_eq!(
				TelemetrySettings {
					consent_version: version,
					..enabled
				}
				.status_with(Overrides::default()),
				TelemetryStatus::ConsentRequired
			);
		}
	}

	#[test]
	fn every_override_suppresses_valid_consent_and_cannot_enable_it() {
		let enabled = TelemetrySettings::default()
			.changed(true)
			.expect("enable consent");
		for (overrides, expected) in [
			(
				Overrides {
					environment: true,
					..Overrides::default()
				},
				TelemetryStatus::DisabledByEnvironment,
			),
			(
				Overrides {
					do_not_track: true,
					..Overrides::default()
				},
				TelemetryStatus::DisabledByDoNotTrack,
			),
			(
				Overrides {
					development: true,
					..Overrides::default()
				},
				TelemetryStatus::DevelopmentBuild,
			),
			(
				Overrides {
					ci: true,
					..Overrides::default()
				},
				TelemetryStatus::ContinuousIntegration,
			),
			(
				Overrides {
					vm: true,
					..Overrides::default()
				},
				TelemetryStatus::VirtualMachine,
			),
		] {
			assert_eq!(
				TelemetrySettings::default().status_with(overrides),
				TelemetryStatus::Disabled
			);
			assert_eq!(enabled.status_with(overrides), expected);
		}
	}

	#[test]
	fn disable_and_reenable_advance_the_local_revision() {
		let enabled = TelemetrySettings::default().changed(true).expect("enable");
		let disabled = enabled.changed(false).expect("disable");
		let reenabled = disabled.changed(true).expect("reenable");
		assert_eq!(enabled.revision, 1);
		assert!(!disabled.enabled);
		assert_eq!(disabled.revision, 2);
		assert_eq!(reenabled.revision, 3);
		assert_eq!(reenabled.consent_version, TELEMETRY_CONSENT_VERSION);
		assert!(
			TelemetrySettings {
				revision: u64::MAX,
				..reenabled
			}
			.changed(true)
			.is_err()
		);
	}

	#[test]
	fn incomplete_consent_stays_off_and_malformed_consent_is_rejected() {
		let incomplete: TelemetrySettings =
			serde_json::from_str(r#"{"enabled":true}"#).expect("partial consent");
		assert_eq!(
			incomplete.status_with(Overrides::default()),
			TelemetryStatus::ConsentRequired
		);
		assert!(serde_json::from_str::<TelemetrySettings>(r#"{"enabled":"true"}"#).is_err());
		assert!(
			serde_json::from_str::<TelemetrySettings>(r#"{"enabled":true,"extra":"field"}"#)
				.is_err()
		);
	}
}
