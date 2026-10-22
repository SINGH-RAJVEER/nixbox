use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use anyhow::{Context, Result};

use crate::{Config, TelemetrySettings};

pub(crate) fn load(path: &Path) -> Result<Config> {
	let raw = match fs::read_to_string(path) {
		Ok(raw) => raw,
		Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Config::default()),
		Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
	};
	serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
}

pub(crate) fn save(path: &Path, config: &Config) -> Result<()> {
	let path = write_path(path)?;
	let _lock = lock(&path)?;
	let mut saved = config.clone();
	// A theme/search save from a stale instance must never change consent.
	// A missing settings file is also an opt-out, even if the snapshot was enabled.
	saved.telemetry = load(&path)?.telemetry;
	write(&path, &saved)
}

pub(crate) fn set_telemetry_enabled(path: &Path, enabled: bool) -> Result<TelemetrySettings> {
	let path = write_path(path)?;
	let _lock = lock(&path)?;
	let mut saved = load(&path)?;
	saved.telemetry = saved.telemetry.changed(enabled)?;
	write(&path, &saved)?;
	Ok(saved.telemetry)
}

fn lock(path: &Path) -> Result<File> {
	let parent = path.parent().context("settings path has no parent")?;
	fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
	// Lock a stable sibling, not the settings inode replaced by each save.
	// Keep the lock file after closing it so waiting writers always use the same inode.
	let lock_path = path.with_extension("lock");
	let mut options = OpenOptions::new();
	options.read(true).write(true).create(true).truncate(false);
	#[cfg(unix)]
	options.mode(0o600);
	let file = options
		.open(&lock_path)
		.with_context(|| format!("opening {}", lock_path.display()))?;
	file.lock()
		.with_context(|| format!("locking {}", lock_path.display()))?;
	Ok(file)
}

fn write_path(path: &Path) -> Result<std::path::PathBuf> {
	// Preserve the link and use the same lock as writers addressing its target directly.
	Ok(
		if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_symlink()) {
			fs::canonicalize(path).with_context(|| format!("resolving {}", path.display()))?
		} else {
			path.to_owned()
		},
	)
}

fn write(path: &Path, config: &Config) -> Result<()> {
	let parent = path.parent().context("settings path has no parent")?;
	let raw = serde_json::to_vec_pretty(config)?;
	let mut temporary = tempfile::Builder::new()
		.prefix(".settings-")
		.tempfile_in(parent)
		.with_context(|| format!("creating temporary settings in {}", parent.display()))?;
	temporary
		.write_all(&raw)
		.context("writing temporary settings")?;
	temporary
		.as_file()
		.sync_all()
		.context("syncing temporary settings")?;
	temporary
		.persist(path)
		.map_err(|error| error.error)
		.with_context(|| format!("replacing {}", path.display()))?;
	Ok(())
}

#[cfg(test)]
mod tests {
	use std::sync::{Arc, Barrier};

	use super::*;

	#[test]
	fn ordinary_saves_cannot_enable_collection_or_restore_deleted_consent() {
		let directory = tempfile::tempdir().expect("test directory");
		let path = directory.path().join("settings.json");
		let config = Config {
			telemetry: TelemetrySettings::default()
				.changed(true)
				.expect("local consent"),
			..Config::default()
		};
		save(&path, &config).expect("save defaults");
		assert_eq!(
			load(&path).expect("load").telemetry,
			TelemetrySettings::default()
		);
		set_telemetry_enabled(&path, true).expect("enable");
		let stale = load(&path).expect("enabled snapshot");
		fs::remove_file(&path).expect("delete settings");
		save(&path, &stale).expect("save stale snapshot");
		assert!(!load(&path).expect("load").telemetry.enabled);
	}

	#[test]
	fn a_stale_preference_save_preserves_opt_out_and_its_revision() {
		let directory = tempfile::tempdir().expect("test directory");
		let path = directory.path().join("settings.json");
		set_telemetry_enabled(&path, true).expect("enable");
		let mut stale = load(&path).expect("enabled snapshot");
		let disabled = set_telemetry_enabled(&path, false).expect("disable");
		stale.theme = "nord".into();
		save(&path, &stale).expect("save stale theme");
		let saved = load(&path).expect("load");
		assert_eq!(saved.theme, "nord");
		assert_eq!(saved.telemetry, disabled);
		assert!(!saved.telemetry.enabled);
	}

	#[test]
	fn explicit_consent_changes_preserve_other_saved_preferences() {
		let directory = tempfile::tempdir().expect("test directory");
		let path = directory.path().join("settings.json");
		let preferences = Config {
			theme: "dracula".into(),
			..Config::default()
		};
		save(&path, &preferences).expect("save preferences");
		let consent = set_telemetry_enabled(&path, true).expect("enable");
		let saved = load(&path).expect("load");
		assert_eq!(saved.theme, "dracula");
		assert_eq!(saved.telemetry, consent);
		assert_eq!(consent.revision, 1);
	}

	#[test]
	fn invalid_settings_are_never_replaced_by_a_save_or_consent_change() {
		let directory = tempfile::tempdir().expect("test directory");
		let path = directory.path().join("settings.json");
		for raw in [
			"{broken",
			r#"{"channel":"stable","target":"nixos-system","telemetry":{"enabled":"yes"}}"#,
		] {
			fs::write(&path, raw).expect("invalid settings");
			assert!(load(&path).is_err());
			assert!(save(&path, &Config::default()).is_err());
			assert!(set_telemetry_enabled(&path, true).is_err());
			assert_eq!(fs::read_to_string(&path).expect("unchanged settings"), raw);
		}
	}

	#[test]
	fn a_failed_consent_write_keeps_saved_consent_unchanged() {
		let directory = tempfile::tempdir().expect("test directory");
		let path = directory.path().join("settings.json");
		save(&path, &Config::default()).expect("initial settings");
		fs::remove_file(path.with_extension("lock")).expect("remove unlocked file");
		fs::create_dir(path.with_extension("lock")).expect("block writes");
		assert!(set_telemetry_enabled(&path, true).is_err());
		assert!(!load(&path).expect("load").telemetry.enabled);
	}

	#[test]
	fn concurrent_updates_serialize_read_modify_write_and_keep_json_complete() {
		let directory = tempfile::tempdir().expect("test directory");
		let path = directory.path().join("settings.json");
		save(&path, &Config::default()).expect("initial settings");
		let barrier = Arc::new(Barrier::new(5));
		std::thread::scope(|scope| {
			for worker in 0..4 {
				let path = &path;
				let barrier = Arc::clone(&barrier);
				scope.spawn(move || {
					barrier.wait();
					for iteration in 0..12 {
						set_telemetry_enabled(path, (worker + iteration) % 2 == 0)
							.expect("concurrent consent");
					}
				});
			}
			barrier.wait();
			for _ in 0..48 {
				load(&path).expect("readers always see complete JSON");
			}
		});
		let consent = set_telemetry_enabled(&path, false).expect("final opt out");
		assert_eq!(consent.revision, 49);
		assert!(!consent.enabled);
		assert_eq!(load(&path).expect("load").telemetry, consent);
	}

	#[cfg(unix)]
	#[test]
	fn saving_through_a_symlink_preserves_the_link_and_uses_private_permissions() {
		use std::os::unix::fs::{PermissionsExt, symlink};

		let directory = tempfile::tempdir().expect("test directory");
		let destination = directory.path().join("actual.json");
		save(&destination, &Config::default()).expect("initial settings");
		let path = directory.path().join("settings.json");
		symlink(&destination, &path).expect("settings link");
		set_telemetry_enabled(&path, true).expect("enable through link");
		assert!(
			fs::symlink_metadata(&path)
				.expect("link metadata")
				.is_symlink()
		);
		assert!(
			load(&destination)
				.expect("target settings")
				.telemetry
				.enabled
		);
		assert_eq!(
			fs::metadata(&destination)
				.expect("metadata")
				.permissions()
				.mode() & 0o777,
			0o600
		);
	}
}
