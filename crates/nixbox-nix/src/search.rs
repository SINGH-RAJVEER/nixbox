use std::collections::{BTreeMap, BinaryHeap};
use std::fmt;
use std::fs;
use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};
use serde::de::{Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

pub const MAX_SEARCH_RESULTS: usize = 200;
const MAX_SEARCH_OUTPUT_BYTES: usize = 50 * 1024 * 1024;
const MAX_CATALOG_OUTPUT_BYTES: usize = 256 * 1024 * 1024;
const MAX_SEARCH_ERROR_BYTES: usize = 64 * 1024;
const CATALOG_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub attr: String,
    pub pname: String,
    pub version: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CatalogSource {
    flake_ref: String,
    revision: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CatalogSnapshot {
    format_version: u32,
    source: CatalogSource,
    hits: Vec<SearchHit>,
}

#[derive(Debug, Clone)]
struct SearchDocument {
    hit: SearchHit,
    attr: String,
    pname: String,
    description: String,
}

impl From<SearchHit> for SearchDocument {
    fn from(hit: SearchHit) -> Self {
        Self {
            attr: hit.attr.to_lowercase(),
            pname: hit.pname.to_lowercase(),
            description: hit.description.to_lowercase(),
            hit,
        }
    }
}

/// A revision-pinned nixpkgs catalog. Building it evaluates nixpkgs once;
/// every subsequent query is handled in-process.
#[derive(Debug, Clone)]
pub struct PackageCatalog {
    source: CatalogSource,
    documents: Vec<SearchDocument>,
}

impl PackageCatalog {
    /// Loads the cached catalog for the nixpkgs revision pinned by `flake.lock`,
    /// or evaluates that exact revision and stores a new cache atomically.
    pub async fn load_or_build(config_dir: &Path, cache_path: &Path) -> Result<Self> {
        let lock_path = config_dir.join("flake.lock");
        let lock =
            fs::read(&lock_path).with_context(|| format!("reading {}", lock_path.display()))?;
        let source = catalog_source_from_lock(&lock).context("resolving locked nixpkgs input")?;

        if let Some(catalog) = Self::load_current(cache_path, &source) {
            return Ok(catalog);
        }

        let stdout = run_nix_search(&source.flake_ref, "^", MAX_CATALOG_OUTPUT_BYTES).await?;
        let mut hits = parse_all_hits(&stdout).context("parsing nixpkgs catalog JSON")?;
        hits.sort_by(|left, right| left.attr.cmp(&right.attr));
        hits.dedup_by(|left, right| left.attr == right.attr);

        let snapshot = CatalogSnapshot {
            format_version: CATALOG_FORMAT_VERSION,
            source: source.clone(),
            hits,
        };
        save_snapshot(cache_path, &snapshot)?;
        Ok(Self::from_snapshot(snapshot))
    }

    /// Searches the already-loaded catalog without invoking Nix.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<SearchHit> {
        let query = ranking_term(query);
        if query.is_empty() {
            return Vec::new();
        }

        let mut best = BinaryHeap::with_capacity(MAX_SEARCH_RESULTS + 1);
        for (index, document) in self.documents.iter().enumerate() {
            let relevance = relevance_fields(
                &document.attr,
                &document.pname,
                &document.description,
                &query,
            );
            if relevance.tier == 6 {
                continue;
            }

            let candidate = RankedIndex { relevance, index };
            if best.len() < MAX_SEARCH_RESULTS {
                best.push(candidate);
            } else if best.peek().is_some_and(|worst| candidate < *worst) {
                best.pop();
                best.push(candidate);
            }
        }

        let mut ranked = best.into_vec();
        ranked.sort();
        ranked
            .into_iter()
            .filter_map(|item| self.documents.get(item.index))
            .map(|document| document.hit.clone())
            .collect()
    }

    #[must_use]
    pub fn revision(&self) -> &str {
        &self.source.revision
    }

    /// Returns whether the target flake still pins the revision represented by
    /// this catalog.
    pub fn is_current_for(&self, config_dir: &Path) -> Result<bool> {
        let lock_path = config_dir.join("flake.lock");
        let lock =
            fs::read(&lock_path).with_context(|| format!("reading {}", lock_path.display()))?;
        let source = catalog_source_from_lock(&lock).context("resolving locked nixpkgs input")?;
        Ok(source == self.source)
    }

    fn load_current(cache_path: &Path, source: &CatalogSource) -> Option<Self> {
        let raw = fs::read(cache_path).ok()?;
        let snapshot: CatalogSnapshot = serde_json::from_slice(&raw).ok()?;
        (snapshot.format_version == CATALOG_FORMAT_VERSION && snapshot.source == *source)
            .then(|| Self::from_snapshot(snapshot))
    }

    fn from_snapshot(snapshot: CatalogSnapshot) -> Self {
        Self {
            source: snapshot.source,
            documents: snapshot
                .hits
                .into_iter()
                .map(SearchDocument::from)
                .collect(),
        }
    }

    #[cfg(test)]
    fn from_hits_for_test(hits: Vec<SearchHit>) -> Self {
        Self::from_snapshot(CatalogSnapshot {
            format_version: CATALOG_FORMAT_VERSION,
            source: CatalogSource {
                flake_ref: "github:NixOS/nixpkgs/test".into(),
                revision: "test".into(),
            },
            hits,
        })
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct RankedIndex {
    relevance: Relevance,
    index: usize,
}

#[derive(Debug, Deserialize)]
struct RawHit {
    pname: String,
    version: String,
    #[serde(default)]
    description: String,
}

/// Resolves a channel shortname to a flake ref. Bare names like `nixpkgs-unstable`
/// aren't always in the user's registry, so map known ones explicitly.
fn resolve_channel(channel: &str) -> &str {
    match channel {
        "nixpkgs" | "nixpkgs-26.05" => "github:NixOS/nixpkgs/nixos-26.05",
        "nixpkgs-unstable" => "github:NixOS/nixpkgs/nixos-unstable",
        other => other,
    }
}

pub async fn search(channel: &str, query: &str) -> Result<Vec<SearchHit>> {
    let query = if query.trim().is_empty() { "^" } else { query };
    let resolved = resolve_channel(channel);

    let stdout = run_nix_search(resolved, query, MAX_SEARCH_OUTPUT_BYTES).await?;
    parse_hits(&stdout, query).context("parsing nix search JSON")
}

async fn run_nix_search(resolved: &str, query: &str, max_output_bytes: usize) -> Result<Vec<u8>> {
    let mut child = Command::new("nix")
        .args([
            "--quiet",
            "search",
            "--json",
            "--extra-experimental-features",
            "nix-command flakes",
            resolved,
            query,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("invoking `nix search` (is nix installed and on PATH?)")?;

    let stdout = child.stdout.take().context("capturing nix search stdout")?;
    let stderr = child.stderr.take().context("capturing nix search stderr")?;
    let stderr_task = tokio::spawn(read_truncated(stderr, MAX_SEARCH_ERROR_BYTES));

    let stdout = match read_limited(stdout, max_output_bytes).await {
        Ok(stdout) => stdout,
        Err(error) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = stderr_task.await;
            return Err(error);
        }
    };

    let status = child.wait().await.context("waiting for nix search")?;
    let stderr = stderr_task
        .await
        .context("joining nix search stderr reader")??;

    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        anyhow::bail!("nix search failed: {}", stderr.trim());
    }

    Ok(stdout)
}

fn catalog_source_from_lock(input: &[u8]) -> Result<CatalogSource> {
    let lock: serde_json::Value = serde_json::from_slice(input).context("parsing flake.lock")?;
    let root_id = lock
        .get("root")
        .and_then(serde_json::Value::as_str)
        .context("flake.lock has no root node")?;
    let nixpkgs_id = lock
        .get("nodes")
        .and_then(|nodes| nodes.get(root_id))
        .and_then(|root| root.get("inputs"))
        .and_then(|inputs| inputs.get("nixpkgs"))
        .and_then(serde_json::Value::as_str)
        .context("root flake has no direct nixpkgs input")?;
    let locked = lock
        .get("nodes")
        .and_then(|nodes| nodes.get(nixpkgs_id))
        .and_then(|node| node.get("locked"))
        .context("nixpkgs input is not locked")?;

    let input_type = locked
        .get("type")
        .and_then(serde_json::Value::as_str)
        .context("locked nixpkgs input has no type")?;
    let revision = locked
        .get("rev")
        .and_then(serde_json::Value::as_str)
        .context("locked nixpkgs input has no revision")?;
    let owner = locked
        .get("owner")
        .and_then(serde_json::Value::as_str)
        .context("locked nixpkgs input has no owner")?;
    let repo = locked
        .get("repo")
        .and_then(serde_json::Value::as_str)
        .context("locked nixpkgs input has no repository")?;

    let flake_ref = match input_type {
        "github" => format!("github:{owner}/{repo}/{revision}"),
        "gitlab" => format!("gitlab:{owner}/{repo}/{revision}"),
        other => anyhow::bail!("unsupported locked nixpkgs input type `{other}`"),
    };

    Ok(CatalogSource {
        flake_ref,
        revision: revision.to_string(),
    })
}

fn save_snapshot(path: &Path, snapshot: &CatalogSnapshot) -> Result<()> {
    let parent = path
        .parent()
        .context("package catalog cache path has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let bytes = serde_json::to_vec(snapshot).context("serializing package catalog")?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes).with_context(|| format!("writing {}", temporary.display()))?;
    fs::rename(&temporary, path)
        .with_context(|| format!("installing package catalog at {}", path.display()))?;
    Ok(())
}

fn parse_all_hits(input: &[u8]) -> Result<Vec<SearchHit>, serde_json::Error> {
    let raw: BTreeMap<String, RawHit> = serde_json::from_slice(input)?;
    Ok(raw
        .into_iter()
        .map(|(attr, raw)| SearchHit {
            attr: install_attr(&attr).to_string(),
            pname: raw.pname,
            version: raw.version,
            description: raw.description,
        })
        .collect())
}

async fn read_limited<R>(mut reader: R, max_bytes: usize) -> Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut out = Vec::new();
    let mut buf = [0; 8192];
    loop {
        let n = reader
            .read(&mut buf)
            .await
            .context("reading nix search output")?;
        if n == 0 {
            break;
        }
        if out.len() + n > max_bytes {
            anyhow::bail!(
                "nix search output exceeded {} MiB; refine the query",
                max_bytes / 1024 / 1024
            );
        }
        out.extend_from_slice(&buf[..n]);
    }
    Ok(out)
}

async fn read_truncated<R>(mut reader: R, max_bytes: usize) -> Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut out = Vec::new();
    let mut buf = [0; 8192];
    loop {
        let n = reader
            .read(&mut buf)
            .await
            .context("reading nix search stderr")?;
        if n == 0 {
            break;
        }
        let remaining = max_bytes.saturating_sub(out.len());
        if remaining > 0 {
            out.extend_from_slice(&buf[..n.min(remaining)]);
        }
    }
    Ok(out)
}

fn parse_hits(input: &[u8], query: &str) -> Result<Vec<SearchHit>, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    deserializer.deserialize_map(SearchHitsVisitor {
        query: ranking_term(query),
    })
}

struct SearchHitsVisitor {
    query: String,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Relevance {
    tier: u8,
    position: usize,
    name_len: usize,
    name: String,
}

struct RankedHit {
    relevance: Relevance,
    hit: SearchHit,
}

impl<'de> Visitor<'de> for SearchHitsVisitor {
    type Value = Vec<SearchHit>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a nix search JSON object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut hits: Vec<RankedHit> = Vec::new();
        while let Some(attr) = map.next_key::<String>()? {
            let raw = map.next_value::<RawHit>()?;
            let hit = SearchHit {
                attr: install_attr(&attr).to_string(),
                pname: raw.pname,
                version: raw.version,
                description: raw.description,
            };
            let ranked = RankedHit {
                relevance: relevance(&hit, &self.query),
                hit,
            };

            if hits.len() < MAX_SEARCH_RESULTS {
                hits.push(ranked);
            } else {
                let (worst_index, worst) = hits
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, ranked)| &ranked.relevance)
                    .expect("bounded result set is not empty");
                if ranked.relevance < worst.relevance {
                    hits[worst_index] = ranked;
                }
            }
        }
        hits.sort_by(|a, b| a.relevance.cmp(&b.relevance));
        Ok(hits.into_iter().map(|ranked| ranked.hit).collect())
    }
}

fn ranking_term(query: &str) -> String {
    let query = query.trim();
    let query = query.strip_prefix('^').unwrap_or(query);
    query.strip_suffix('$').unwrap_or(query).to_lowercase()
}

fn relevance(hit: &SearchHit, query: &str) -> Relevance {
    let attr = hit.attr.to_lowercase();
    let pname = hit.pname.to_lowercase();
    let description = hit.description.to_lowercase();
    relevance_fields(&attr, &pname, &description, query)
}

fn relevance_fields(attr: &str, pname: &str, description: &str, query: &str) -> Relevance {
    let names = [pname, attr];

    let (tier, position) = if query.is_empty() {
        (6, usize::MAX)
    } else if names.contains(&query) {
        (0, 0)
    } else if names.iter().any(|name| name.starts_with(query)) {
        (1, 0)
    } else if let Some(position) = names
        .iter()
        .filter_map(|name| token_position(name, query))
        .min()
    {
        (2, position)
    } else if let Some(position) = names.iter().filter_map(|name| name.find(query)).min() {
        (3, position)
    } else if let Some(position) = token_position(description, query) {
        (4, position)
    } else if let Some(position) = description.find(query) {
        (5, position)
    } else {
        (6, usize::MAX)
    };

    Relevance {
        tier,
        position,
        name_len: pname.chars().count(),
        name: pname.to_string(),
    }
}

fn token_position(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .match_indices(needle)
        .find_map(|(position, matched)| {
            let before = haystack[..position].chars().next_back();
            let after = haystack[position + matched.len()..].chars().next();
            let starts_token = before.is_none_or(|ch| !ch.is_alphanumeric());
            let ends_token = after.is_none_or(|ch| !ch.is_alphanumeric());
            (starts_token && ends_token).then_some(position)
        })
}

fn install_attr(full: &str) -> &str {
    let mut segments = full.splitn(3, '.');
    match (segments.next(), segments.next(), segments.next()) {
        (Some("legacyPackages" | "packages"), Some(_system), Some(attr)) => attr,
        _ => full,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn resolves_known_channel_aliases() {
        assert_eq!(
            resolve_channel("nixpkgs-unstable"),
            "github:NixOS/nixpkgs/nixos-unstable"
        );
        assert_eq!(
            resolve_channel("nixpkgs-26.05"),
            "github:NixOS/nixpkgs/nixos-26.05"
        );
        assert_eq!(
            resolve_channel("nixpkgs"),
            "github:NixOS/nixpkgs/nixos-26.05"
        );
        assert_eq!(resolve_channel("github:owner/repo"), "github:owner/repo");
    }

    #[test]
    fn install_attr_strips_the_standard_flake_prefix() {
        assert_eq!(
            install_attr("legacyPackages.x86_64-linux.ripgrep"),
            "ripgrep"
        );
        assert_eq!(install_attr("firefox"), "firefox");
    }

    #[test]
    fn install_attr_preserves_nested_package_paths() {
        assert_eq!(
            install_attr("legacyPackages.x86_64-linux.python312Packages.black"),
            "python312Packages.black"
        );
        assert_eq!(install_attr("packages.aarch64-darwin.ripgrep"), "ripgrep");
        assert_eq!(install_attr("firefox"), "firefox");
    }

    #[test]
    fn resolves_nixpkgs_revision_from_root_flake_lock() {
        let lock = br#"{
            "root": "root",
            "nodes": {
                "root": { "inputs": { "nixpkgs": "nixpkgs" } },
                "nixpkgs": {
                    "locked": {
                        "type": "github",
                        "owner": "NixOS",
                        "repo": "nixpkgs",
                        "rev": "0123456789abcdef"
                    }
                }
            }
        }"#;

        let source = catalog_source_from_lock(lock).expect("locked nixpkgs source");

        assert_eq!(source.flake_ref, "github:NixOS/nixpkgs/0123456789abcdef");
        assert_eq!(source.revision, "0123456789abcdef");
    }

    #[test]
    fn catalog_searches_locally_and_keeps_relevance_order() {
        let catalog = PackageCatalog::from_hits_for_test(vec![
            SearchHit {
                attr: "air".into(),
                pname: "air".into(),
                version: "1".into(),
                description: "Live reload for Go apps".into(),
            },
            SearchHit {
                attr: "go-tools".into(),
                pname: "go-tools".into(),
                version: "1".into(),
                description: "Developer tools".into(),
            },
            SearchHit {
                attr: "go".into(),
                pname: "go".into(),
                version: "1".into(),
                description: "Compiler".into(),
            },
        ]);

        let hits = catalog.search("go");

        assert_eq!(
            hits.iter()
                .map(|hit| hit.pname.as_str())
                .collect::<Vec<_>>(),
            ["go", "go-tools", "air"]
        );
    }

    #[tokio::test]
    async fn matching_disk_catalog_avoids_nix_evaluation() {
        let base = std::env::temp_dir().join(format!("nixbox-catalog-test-{}", std::process::id()));
        let config_dir = base.join("config");
        let cache_path = base.join("cache/package-catalog.json");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("flake.lock"),
            br#"{
                "root": "root",
                "nodes": {
                    "root": { "inputs": { "nixpkgs": "nixpkgs" } },
                    "nixpkgs": {
                        "locked": {
                            "type": "github",
                            "owner": "NixOS",
                            "repo": "nixpkgs",
                            "rev": "cached-revision"
                        }
                    }
                }
            }"#,
        )
        .unwrap();
        save_snapshot(
            &cache_path,
            &CatalogSnapshot {
                format_version: CATALOG_FORMAT_VERSION,
                source: CatalogSource {
                    flake_ref: "github:NixOS/nixpkgs/cached-revision".into(),
                    revision: "cached-revision".into(),
                },
                hits: vec![SearchHit {
                    attr: "ripgrep".into(),
                    pname: "ripgrep".into(),
                    version: "14.1.1".into(),
                    description: "Fast recursive search".into(),
                }],
            },
        )
        .unwrap();

        let catalog = PackageCatalog::load_or_build(&config_dir, &cache_path)
            .await
            .unwrap();

        assert_eq!(catalog.revision(), "cached-revision");
        assert_eq!(catalog.search("ripgrep")[0].pname, "ripgrep");
        let updated_lock = fs::read_to_string(config_dir.join("flake.lock"))
            .unwrap()
            .replace("cached-revision", "updated-revision");
        fs::write(config_dir.join("flake.lock"), updated_lock).unwrap();
        assert!(!catalog.is_current_for(&config_dir).unwrap());
        fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn read_limited_allows_exact_limit() {
        let bytes = b"abcdef".to_vec();
        let out = read_limited(Cursor::new(bytes), 6).await.expect("read");
        assert_eq!(out, b"abcdef".to_vec());
    }

    #[tokio::test]
    async fn read_limited_rejects_payload_over_limit() {
        let bytes = b"abcdef".to_vec();
        let err = read_limited(Cursor::new(bytes), 5)
            .await
            .expect_err("payload should exceed limit");
        assert!(err.to_string().contains("exceeded"));
    }

    #[tokio::test]
    async fn read_truncated_drains_bytes_beyond_limit() {
        let bytes = b"abcdef".to_vec();
        let mut reader = Cursor::new(bytes);

        let out = read_truncated(&mut reader, 3).await.expect("read");

        assert_eq!(out, b"abc");
        assert_eq!(reader.position(), 6);
    }

    #[test]
    fn parse_hits_normalizes_attrs_and_caps_results() {
        let mut json = String::from("{");
        for i in 0..(MAX_SEARCH_RESULTS + 2) {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&format!(
                r#""legacyPackages.x86_64-linux.pkg{i}":{{"pname":"pkg{i}","version":"1.0","description":"desc"}}"#
            ));
        }
        json.push('}');

        let hits = parse_hits(json.as_bytes(), "pkg").unwrap();

        assert_eq!(hits.len(), MAX_SEARCH_RESULTS);
        assert_eq!(hits[0].attr, "pkg0");
    }

    #[test]
    fn parse_hits_accepts_missing_description() {
        let hits = parse_hits(
            br#"{"legacyPackages.x86_64-linux.ripgrep":{"pname":"ripgrep","version":"14.1.1"}}"#,
            "ripgrep",
        )
        .unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].attr, "ripgrep");
        assert_eq!(hits[0].description, "");
    }

    #[test]
    fn exact_match_displaces_early_weak_matches() {
        let mut json = String::from("{");
        for i in 0..MAX_SEARCH_RESULTS {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&format!(
                r#""legacyPackages.x86_64-linux.weak{i}":{{"pname":"weak{i}","version":"1.0","description":"A tool written in Go"}}"#
            ));
        }
        json.push_str(
            r#", "legacyPackages.x86_64-linux.go":{"pname":"go","version":"1.24","description":"The Go compiler"}}"#,
        );
        json.push('}');

        let hits = parse_hits(json.as_bytes(), "go").unwrap();

        assert_eq!(hits.len(), MAX_SEARCH_RESULTS);
        assert_eq!(hits[0].pname, "go");
        assert_eq!(
            hits.iter()
                .filter(|hit| hit.pname.starts_with("weak"))
                .count(),
            MAX_SEARCH_RESULTS - 1
        );
    }

    #[test]
    fn ranks_name_matches_before_description_matches() {
        let hits = parse_hits(
            br#"{
                "legacyPackages.x86_64-linux.air":{"pname":"air","version":"1","description":"Live reload for Go apps"},
                "legacyPackages.x86_64-linux.go-tools":{"pname":"go-tools","version":"1","description":"Developer tools"},
                "legacyPackages.x86_64-linux.go":{"pname":"go","version":"1","description":"Compiler"}
            }"#,
            "go",
        )
        .unwrap();

        assert_eq!(
            hits.iter()
                .map(|hit| hit.pname.as_str())
                .collect::<Vec<_>>(),
            ["go", "go-tools", "air"]
        );
    }

    #[tokio::test]
    #[ignore = "requires nix and a configured nixpkgs registry"]
    async fn live_go_search_ranks_exact_package_first() {
        let hits = search("nixpkgs", "go").await.expect("live nix search");

        assert_eq!(hits.first().map(|hit| hit.pname.as_str()), Some("go"));
    }
}
