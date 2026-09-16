pub mod build;
pub mod flakes;
pub mod manifest;
pub mod scan;
pub mod search;

pub use build::{
    BuildEvent, flake_has_home_configuration, home_manager_switch_cmd, nixos_rebuild_switch_cmd,
    rebuild,
};
pub use flakes::{
    FlakeDetails, FlakeHit, ensure_flake_input, fetch_flake_details, remove_flake_input,
    search_flakes,
};
pub use manifest::{
    FlakeManifest, ImportStatus, ManagedFile, ManagedFlakeFile, Manifest, ensure_home_nix,
    ensure_imported,
};
pub use scan::{ExternalPackage, ScanTarget, remove_from_source, scan};
pub use search::{SearchHit, search};
