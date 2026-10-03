pub mod build;
pub mod flakes;
pub mod manifest;
pub mod options;
pub mod scan;
pub mod search;
pub mod settings;
pub mod wiring;

pub use build::{
	BuildEvent, flake_has_home_configuration, home_manager_switch_cmd, nixos_rebuild_switch_cmd,
	rebuild,
};
pub use flakes::{FlakeDetails, FlakeHit, fetch_flake_details, search_flakes};
pub use manifest::{
	FlakeManifest, FlakeOutput, ImportStatus, ManagedFile, ManagedFlakeFile, Manifest,
	ensure_home_nix, ensure_imported,
};
pub use options::{OptionEntry, OptionKind, OptionQuery, OptionSet, OptionSource, fetch_options};
pub use scan::{
	ExternalFlakePackage, ExternalPackage, ScanTarget, remove_flake_package_from_source,
	remove_from_source, scan, scan_flake_packages,
};
pub use search::{SearchHit, search};
pub use settings::{OptionPath, SettingValue, SettingsFile, SettingsManifest};
