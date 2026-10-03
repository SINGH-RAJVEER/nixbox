//! Package search as the interactive front-ends run it: against the local
//! catalog of the user's locked nixpkgs when it is ready, and through
//! `nix search` on the configured channel when it is not.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use directories::BaseDirs;
use nixbox_nix::search::{PackageCatalog, SearchHit};

/// Where the package catalog is cached between runs.
#[must_use]
pub fn catalog_cache_path() -> Option<PathBuf> {
	BaseDirs::new().map(|base| base.cache_dir().join("nixbox").join("package-catalog.json"))
}

/// Searches `catalog` when there is one, off the async threads because the
/// catalog is large, and falls back to a live `nix search` otherwise.
pub async fn search_packages(
	catalog: Option<Arc<PackageCatalog>>,
	channel: &str,
	query: String,
) -> Result<Vec<SearchHit>> {
	match catalog {
		Some(catalog) => tokio::task::spawn_blocking(move || catalog.search(&query))
			.await
			.map_err(|error| anyhow!("joining package catalog search: {error}")),
		None => nixbox_nix::search::search(channel, &query).await,
	}
}
