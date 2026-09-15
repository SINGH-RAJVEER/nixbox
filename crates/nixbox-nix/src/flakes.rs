use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;
use std::{fs, process::Stdio};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::timeout;

pub const MAX_FLAKE_RESULTS: usize = 20;
const MAX_FLAKE_CANDIDATES: usize = 12;
const FLAKE_EVALUATION_TIMEOUT: Duration = Duration::from_secs(15);
const UPSTREAM_FLAKE_EVALUATION_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Clone)]
pub struct FlakeHit {
    pub repo: String,
    pub repo_url: String,
    pub path: String,
    pub match_fragment: Option<String>,
    pub packages: Vec<FlakePackage>,
    pub nixos_module: Option<String>,
    pub home_manager_module: Option<String>,
    outputs: Vec<String>,
    content_url: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct FlakePackage {
    pub attr: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone)]
pub struct FlakeDetails {
    pub repo: String,
    pub repo_url: String,
    pub path: String,
    pub description: Option<String>,
    pub stars: u64,
    pub topics: Vec<String>,
    pub homepage: Option<String>,
    pub default_branch: String,
    pub pushed_at: Option<String>,
    pub archived: bool,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub packages: Vec<FlakePackage>,
    pub nixos_module: Option<String>,
    pub home_manager_module: Option<String>,
}

#[derive(Deserialize)]
struct CodeSearchResponse {
    items: Vec<CodeSearchItem>,
}

#[derive(Deserialize)]
struct CodeSearchItem {
    repository: SearchRepository,
    #[serde(default)]
    text_matches: Vec<TextMatch>,
}

#[derive(Deserialize)]
struct TextMatch {
    fragment: String,
}

#[derive(Deserialize)]
struct SearchRepository {
    full_name: String,
    html_url: String,
}

#[derive(Deserialize)]
struct RepositorySearchResponse {
    items: Vec<RepositorySearchItem>,
}

#[derive(Deserialize)]
struct RepositorySearchItem {
    full_name: String,
    stargazers_count: u64,
}

struct RankedHit {
    score: u64,
    hit: FlakeHit,
}

struct Candidate {
    score: u64,
    repo: String,
    repo_url: String,
    match_fragment: Option<String>,
    upstream_reference: bool,
}

#[derive(Clone)]
struct FlakeInspection {
    output_names: Vec<String>,
    packages: Vec<FlakePackage>,
    nixos_module: bool,
    home_manager_module: bool,
    home_module: bool,
}

#[derive(Deserialize)]
struct EvaluatedOutputs {
    output_names: Vec<String>,
    package_attrs: Vec<String>,
    nixos_module: bool,
    home_manager_module: bool,
    home_module: bool,
}

#[derive(Deserialize)]
struct GitHubCommit {
    sha: String,
}

#[derive(Deserialize)]
struct Repository {
    #[serde(default)]
    description: Option<String>,
    stargazers_count: u64,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    homepage: Option<String>,
    default_branch: String,
    #[serde(default)]
    pushed_at: Option<String>,
    archived: bool,
}

/// Searches GitHub's code index for flakes at the root of their repositories.
/// Authentication is delegated to the user's existing `gh auth login` session.
pub async fn search_flakes(query: &str) -> Result<Vec<FlakeHit>> {
    let query = query.trim();
    let (code_items, repositories) =
        tokio::try_join!(search_code(query), search_repositories(query))?;
    let mut candidates: BTreeMap<String, Candidate> = BTreeMap::new();

    for item in code_items {
        let name_score = repository_name_score(query, &item.repository.full_name);
        insert_candidate(
            &mut candidates,
            Candidate {
                score: 100 + name_score,
                repo: item.repository.full_name.clone(),
                repo_url: item.repository.html_url,
                match_fragment: item
                    .text_matches
                    .first()
                    .map(|matched| compact_fragment(&matched.fragment)),
                upstream_reference: false,
            },
        );
        for reference in item
            .text_matches
            .iter()
            .flat_map(|matched| github_references(&matched.fragment))
        {
            let score = repository_name_score(query, &reference);
            if score > 0 {
                insert_candidate(
                    &mut candidates,
                    Candidate {
                        score: 1_000 + score,
                        repo_url: format!("https://github.com/{reference}"),
                        repo: reference,
                        match_fragment: None,
                        upstream_reference: true,
                    },
                );
            }
        }
    }

    for repository in repositories {
        let score = repository_name_score(query, &repository.full_name);
        insert_candidate(
            &mut candidates,
            Candidate {
                score: 500 + score + repository.stargazers_count.min(10_000) / 1_000,
                repo_url: format!("https://github.com/{}", repository.full_name),
                repo: repository.full_name,
                match_fragment: None,
                upstream_reference: false,
            },
        );
    }

    let mut candidates: Vec<Candidate> = candidates.into_values().collect();
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.repo.cmp(&b.repo)));
    candidates.truncate(MAX_FLAKE_CANDIDATES);

    let mut tasks = tokio::task::JoinSet::new();
    for candidate in candidates {
        let query = query.to_string();
        tasks.spawn(async move { inspect_candidate(&query, candidate).await });
    }
    let mut ranked = Vec::new();
    while let Some(result) = tasks.join_next().await {
        if let Ok(Ok(Some(hit))) = result {
            ranked.push(hit);
        }
    }

    ranked.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.hit.repo.cmp(&b.hit.repo))
    });
    ranked.truncate(MAX_FLAKE_RESULTS);
    Ok(ranked.into_iter().map(|candidate| candidate.hit).collect())
}

async fn search_code(query: &str) -> Result<Vec<CodeSearchItem>> {
    let query = format!("{query} in:file filename:flake.nix path:/");
    let raw = gh_api(vec![
        "--method".into(),
        "GET".into(),
        "-H".into(),
        "Accept: application/vnd.github.text-match+json".into(),
        "-H".into(),
        "X-GitHub-Api-Version: 2026-03-10".into(),
        "search/code".into(),
        "-f".into(),
        format!("q={query}"),
        "-f".into(),
        "per_page=20".into(),
    ])
    .await?;
    let response: CodeSearchResponse =
        serde_json::from_slice(&raw).context("parsing GitHub code search response")?;
    Ok(response.items)
}

async fn search_repositories(query: &str) -> Result<Vec<RepositorySearchItem>> {
    let raw = gh_api(vec![
        "--method".into(),
        "GET".into(),
        "-H".into(),
        "Accept: application/vnd.github+json".into(),
        "search/repositories".into(),
        "-f".into(),
        format!("q={}", repository_search_query(query)),
        "-f".into(),
        "per_page=20".into(),
    ])
    .await?;
    let response: RepositorySearchResponse =
        serde_json::from_slice(&raw).context("parsing GitHub repository search response")?;
    Ok(response.items)
}

fn repository_search_query(query: &str) -> String {
    format!("{query} flake in:name,description,topics archived:false")
}

fn insert_candidate(candidates: &mut BTreeMap<String, Candidate>, candidate: Candidate) {
    match candidates.get_mut(&candidate.repo) {
        Some(existing) => {
            let upstream_reference = existing.upstream_reference || candidate.upstream_reference;
            if candidate.score > existing.score {
                *existing = candidate;
            }
            existing.upstream_reference = upstream_reference;
        }
        None => {
            candidates.insert(candidate.repo.clone(), candidate);
        }
    }
}

async fn inspect_candidate(query: &str, candidate: Candidate) -> Result<Option<RankedHit>> {
    let evaluation_timeout = if candidate.upstream_reference {
        UPSTREAM_FLAKE_EVALUATION_TIMEOUT
    } else {
        FLAKE_EVALUATION_TIMEOUT
    };
    let mut inspection = inspect_flake(&candidate.repo, evaluation_timeout).await?;
    if inspection.packages.is_empty()
        && !inspection.nixos_module
        && !inspection.home_manager_module
        && !inspection.home_module
    {
        return Ok(None);
    }
    sort_packages(query, &mut inspection.packages);
    let package_score = inspection
        .packages
        .first()
        .map_or(0, |package| flake_package_score(query, package));
    let outputs = classify_output_names(&inspection);
    let home_manager_module = if inspection.home_manager_module {
        Some("homeManagerModules.default".into())
    } else if inspection.home_module {
        Some("homeModules.default".into())
    } else {
        None
    };
    let nixos_module = inspection
        .nixos_module
        .then(|| "nixosModules.default".into());
    let repo = candidate.repo;

    Ok(Some(RankedHit {
        score: candidate.score + package_score,
        hit: FlakeHit {
            repo_url: candidate.repo_url,
            path: "flake.nix".into(),
            match_fragment: candidate.match_fragment,
            packages: inspection.packages,
            nixos_module,
            home_manager_module,
            outputs,
            content_url: format!("repos/{repo}/contents/flake.nix"),
            repo,
        },
    }))
}

fn repository_name_score(query: &str, repo: &str) -> u64 {
    let query = normalize(query);
    let name = repo.rsplit('/').next().map(normalize).unwrap_or_default();
    if query.is_empty() || name.is_empty() {
        0
    } else if name == query {
        500
    } else if name.starts_with(&query) {
        400
    } else if name.contains(&query) {
        300
    } else {
        0
    }
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn inspection_cache() -> &'static Mutex<BTreeMap<String, FlakeInspection>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, FlakeInspection>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

async fn inspect_flake(repo: &str, evaluation_timeout: Duration) -> Result<FlakeInspection> {
    if let Some(inspection) = inspection_cache().lock().await.get(repo).cloned() {
        return Ok(inspection);
    }
    if !repo
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/'))
    {
        bail!("invalid GitHub repository name: {repo}");
    }

    let revision = repository_revision(repo).await?;
    let system = nix_system().context("unsupported platform for flake package inspection")?;
    let expression = format!(
        r#"
let
	flake = builtins.getFlake "github:{repo}/{revision}";
	packages =
		if flake ? packages && builtins.hasAttr "{system}" flake.packages
		then builtins.getAttr "{system}" flake.packages
		else {{}};
	is_derivation = attr:
		let result = builtins.tryEval ((builtins.getAttr attr packages).type or null);
		in result.success && result.value == "derivation";
in {{
	output_names = builtins.attrNames flake;
	package_attrs = builtins.filter is_derivation (builtins.attrNames packages);
	nixos_module = flake ? nixosModules && builtins.hasAttr "default" flake.nixosModules;
	home_manager_module = flake ? homeManagerModules && builtins.hasAttr "default" flake.homeManagerModules;
	home_module = flake ? homeModules && builtins.hasAttr "default" flake.homeModules;
}}
"#
    );
    let command = Command::new("nix")
        .args(["eval", "--json", "--expr", &expression])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();
    let output = timeout(evaluation_timeout, command)
        .await
        .with_context(|| format!("timed out evaluating github:{repo}"))??;
    if !output.status.success() {
        bail!(
            "Nix flake evaluation failed for github:{repo}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let evaluated: EvaluatedOutputs = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("parsing evaluated outputs for github:{repo}"))?;
    let inspection = inspection_from_evaluation(evaluated);
    inspection_cache()
        .lock()
        .await
        .insert(repo.to_string(), inspection.clone());
    Ok(inspection)
}

async fn repository_revision(repo: &str) -> Result<String> {
    let raw = gh_api(vec![
        "-H".into(),
        "Accept: application/vnd.github+json".into(),
        format!("repos/{repo}/commits/HEAD"),
    ])
    .await?;
    let commit: GitHubCommit = serde_json::from_slice(&raw)
        .with_context(|| format!("parsing the HEAD revision for github:{repo}"))?;
    Ok(commit.sha)
}

fn nix_system() -> Option<&'static str> {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("x86_64", "linux") => Some("x86_64-linux"),
        ("aarch64", "linux") => Some("aarch64-linux"),
        ("x86_64", "macos") => Some("x86_64-darwin"),
        ("aarch64", "macos") => Some("aarch64-darwin"),
        _ => None,
    }
}

fn inspection_from_evaluation(evaluated: EvaluatedOutputs) -> FlakeInspection {
    FlakeInspection {
        output_names: evaluated.output_names,
        packages: evaluated
            .package_attrs
            .into_iter()
            .map(|attr| FlakePackage {
                name: attr.clone(),
                attr,
                version: String::new(),
            })
            .collect(),
        nixos_module: evaluated.nixos_module,
        home_manager_module: evaluated.home_manager_module,
        home_module: evaluated.home_module,
    }
}

fn flake_package_score(query: &str, package: &FlakePackage) -> u64 {
    let query = normalize(query);
    let attr = normalize(&package.attr);
    let name = normalize(&package.name);
    if attr == "default" && (name == query || name.starts_with(&query)) {
        450
    } else if attr == query {
        400
    } else if name == query {
        350
    } else if attr.starts_with(&query) || name.starts_with(&query) {
        250
    } else if attr.contains(&query) || name.contains(&query) {
        150
    } else {
        0
    }
}

fn sort_packages(query: &str, packages: &mut [FlakePackage]) {
    packages.sort_by(|a, b| {
        let a_default = a.attr == "default";
        let b_default = b.attr == "default";
        b_default
            .cmp(&a_default)
            .then_with(|| flake_package_score(query, b).cmp(&flake_package_score(query, a)))
            .then_with(|| b.version.cmp(&a.version))
            .then_with(|| a.attr.cmp(&b.attr))
    });
}

fn classify_output_names(inspection: &FlakeInspection) -> Vec<String> {
    const OUTPUTS: [(&str, &str); 7] = [
        ("nixosModules", "NixOS modules"),
        ("homeManagerModules", "Home Manager modules"),
        ("homeModules", "Home Manager modules"),
        ("overlays", "overlays"),
        ("devShells", "dev shells"),
        ("apps", "apps"),
        ("formatter", "formatter"),
    ];
    let mut outputs = Vec::new();
    if !inspection.packages.is_empty() {
        outputs.push("packages".to_string());
    }
    for (name, label) in OUTPUTS {
        if inspection.output_names.iter().any(|output| output == name)
            && !outputs.iter().any(|output| output == label)
        {
            outputs.push(label.to_string());
        }
    }
    outputs
}

fn github_references(fragment: &str) -> Vec<String> {
    let mut references = Vec::new();
    let mut remaining = fragment;
    while let Some(start) = remaining.find("github:") {
        remaining = &remaining[start + "github:".len()..];
        let path: String = remaining
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/'))
            .collect();
        let mut segments = path.split('/');
        let Some(owner) = segments.next() else {
            continue;
        };
        let Some(repo) = segments.next() else {
            continue;
        };
        if !owner.is_empty() && !repo.is_empty() {
            let reference = format!("{owner}/{repo}");
            if !references.contains(&reference) {
                references.push(reference);
            }
        }
    }
    references
}

fn compact_fragment(fragment: &str) -> String {
    fragment.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Fetches the selected repository and its flake source concurrently, then
/// extracts the public metadata and common flake input/output declarations.
pub async fn fetch_flake_details(hit: &FlakeHit) -> Result<FlakeDetails> {
    let metadata = gh_api(vec![
        "-H".into(),
        "Accept: application/vnd.github+json".into(),
        format!("repos/{}", hit.repo),
    ]);
    let source = gh_api(vec![
        "-H".into(),
        "Accept: application/vnd.github.raw+json".into(),
        hit.content_url.clone(),
    ]);
    let (metadata, source) = tokio::try_join!(metadata, source)?;
    let repository: Repository =
        serde_json::from_slice(&metadata).context("parsing GitHub repository response")?;
    let source = String::from_utf8(source).context("flake.nix was not valid UTF-8")?;

    Ok(FlakeDetails {
        repo: hit.repo.clone(),
        repo_url: hit.repo_url.clone(),
        path: hit.path.clone(),
        description: repository
            .description
            .filter(|value| !value.trim().is_empty()),
        stars: repository.stargazers_count,
        topics: repository.topics,
        homepage: repository.homepage.filter(|value| !value.trim().is_empty()),
        default_branch: repository.default_branch,
        pushed_at: repository.pushed_at,
        archived: repository.archived,
        inputs: classify_inputs(&source),
        outputs: hit.outputs.clone(),
        packages: hit.packages.clone(),
        nixos_module: hit.nixos_module.clone(),
        home_manager_module: hit.home_manager_module.clone(),
    })
}

/// Adds a GitHub flake as a root input and makes `inputs` available to the
/// selected configuration constructor.
pub fn ensure_flake_input(
    flake_file: &Path,
    repo: &str,
    special_args: &str,
    constructor: &str,
) -> Result<()> {
    let source = fs::read_to_string(flake_file)
        .with_context(|| format!("reading {}", flake_file.display()))?;
    if !source.contains("outputs = inputs@") {
        bail!(
            "{} must bind `inputs` in its outputs function before nixbox can install flake outputs",
            flake_file.display()
        );
    }

    let mut updated = source;
    let input = format!("\"{repo}\"");
    if !updated.contains(&format!("{input}.url")) {
        let inputs_pos = updated
            .find("inputs = {")
            .context("could not find an `inputs = { ... };` block")?;
        let open = inputs_pos + "inputs = ".len();
        let close = matching_brace(&updated, open)
            .context("could not find the end of the flake inputs block")?;
        updated.insert_str(close, &format!("\t{input}.url = \"github:{repo}\";\n"));
    }

    if !updated.contains(special_args) {
        let constructor_pos = updated
            .find(constructor)
            .with_context(|| format!("could not find `{constructor}`"))?;
        let open = updated[constructor_pos..]
            .find('{')
            .map(|offset| constructor_pos + offset)
            .context("could not find configuration arguments")?;
        updated.insert_str(
            open + 1,
            &format!("\n\t\t{special_args} = {{ inherit inputs; }};"),
        );
    }

    fs::write(flake_file, updated).with_context(|| format!("writing {}", flake_file.display()))
}

fn matching_brace(source: &str, open: usize) -> Option<usize> {
    let mut depth = 0;
    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

async fn gh_api(args: Vec<String>) -> Result<Vec<u8>> {
    let output = Command::new("gh")
        .arg("api")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .context("invoking `gh api` (run `gh auth login` to enable flake browsing)")?;
    if !output.status.success() {
        bail!(
            "GitHub API request failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

fn classify_inputs(source: &str) -> Vec<String> {
    let Some(start) = source.find("inputs") else {
        return Vec::new();
    };
    let Some(open) = source[start..].find('{').map(|offset| start + offset) else {
        return Vec::new();
    };
    let Some(block) = balanced_block(source, open) else {
        return Vec::new();
    };

    let mut inputs = Vec::new();
    for line in block.lines() {
        let line = line.trim_start();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let name: String = line
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
            .collect();
        if !name.is_empty() && !inputs.contains(&name) {
            inputs.push(name);
        }
    }
    inputs
}

fn balanced_block(source: &str, open: usize) -> Option<&str> {
    let mut depth = 0;
    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[open + 1..open + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        Candidate, EvaluatedOutputs, FlakeInspection, FlakePackage, classify_inputs,
        classify_output_names, compact_fragment, ensure_flake_input, github_references,
        insert_candidate, inspection_from_evaluation, repository_name_score,
        repository_search_query,
    };
    use std::collections::BTreeMap;
    use std::fs;

    #[test]
    fn compacts_github_match_fragments_for_result_rows() {
        assert_eq!(
            compact_fragment("  inputs.nixpkgs.url =\n  \"github:NixOS/nixpkgs\";  "),
            "inputs.nixpkgs.url = \"github:NixOS/nixpkgs\";"
        );
    }

    #[test]
    fn extracts_upstream_github_references_without_branch_suffixes() {
        assert_eq!(
            github_references(
                "zen.url = \"github:0xc000022070/zen-browser-flake/beta\"; other = \"github:NixOS/nixpkgs\";"
            ),
            ["0xc000022070/zen-browser-flake", "NixOS/nixpkgs"]
        );
    }

    #[test]
    fn ranks_canonical_repository_names_above_consumer_repositories() {
        assert!(
            repository_name_score("zen-browser", "0xc000022070/zen-browser-flake")
                > repository_name_score("zen-browser", "Baitinq/nixos-config")
        );
    }

    #[test]
    fn classifies_evaluated_flake_properties() {
        assert_eq!(
            classify_inputs(
                r#"{
	inputs = {
		nixpkgs.url = "github:NixOS/nixpkgs";
		home-manager.url = "github:nix-community/home-manager";
	};
}"#
            ),
            ["nixpkgs", "home-manager"]
        );
        let inspection = FlakeInspection {
            output_names: vec![
                "packages".into(),
                "nixosModules".into(),
                "homeManagerModules".into(),
                "devShells".into(),
            ],
            packages: vec![FlakePackage {
                attr: "default".into(),
                name: "bun".into(),
                version: "1.4.2".into(),
            }],
            nixos_module: true,
            home_manager_module: true,
            home_module: false,
        };

        assert_eq!(
            classify_output_names(&inspection),
            [
                "packages",
                "NixOS modules",
                "Home Manager modules",
                "dev shells"
            ]
        );
    }

    #[test]
    fn repository_search_prefers_flake_projects() {
        assert_eq!(
            repository_search_query("bun"),
            "bun flake in:name,description,topics archived:false"
        );
    }

    #[test]
    fn preserves_upstream_provenance_when_candidates_are_deduplicated() {
        let mut candidates = BTreeMap::new();
        insert_candidate(
            &mut candidates,
            Candidate {
                score: 1_010,
                repo: "owner/bun".into(),
                repo_url: "https://github.com/owner/bun".into(),
                match_fragment: None,
                upstream_reference: false,
            },
        );
        insert_candidate(
            &mut candidates,
            Candidate {
                score: 900,
                repo: "owner/bun".into(),
                repo_url: "https://github.com/owner/bun".into(),
                match_fragment: None,
                upstream_reference: true,
            },
        );

        let candidate = candidates.get("owner/bun").unwrap();
        assert_eq!(candidate.score, 1_010);
        assert!(candidate.upstream_reference);
    }

    #[test]
    fn converts_current_system_package_attributes_into_installable_outputs() {
        let inspection = inspection_from_evaluation(EvaluatedOutputs {
            output_names: vec!["devShells".into(), "packages".into()],
            package_attrs: vec!["bun".into(), "default".into()],
            nixos_module: false,
            home_manager_module: false,
            home_module: false,
        });

        assert_eq!(inspection.packages.len(), 2);
        assert!(
            inspection
                .packages
                .iter()
                .all(|package| package.name == package.attr)
        );
        assert!(!inspection.nixos_module);
        assert!(!inspection.home_manager_module);
    }

    #[test]
    fn adds_input_and_home_manager_special_args() {
        let dir = std::env::temp_dir().join(format!("nixbox-flake-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("flake.nix");
        fs::write(
            &file,
            "{\n  inputs = { nixpkgs.url = \"github:NixOS/nixpkgs\"; };\n  outputs = inputs@{ self, nixpkgs, ... }: {\n    homeConfigurations.user = inputs.home-manager.lib.homeManagerConfiguration {\n      modules = [ ./home.nix ];\n    };\n  };\n}\n",
        )
        .unwrap();

        ensure_flake_input(
            &file,
            "owner/module",
            "extraSpecialArgs",
            "homeManagerConfiguration",
        )
        .unwrap();

        let updated = fs::read_to_string(&file).unwrap();
        assert!(updated.contains("\"owner/module\".url = \"github:owner/module\";"));
        assert!(updated.contains("extraSpecialArgs = { inherit inputs; };"));
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    #[ignore = "requires authenticated gh access and consumes GitHub search quota"]
    async fn live_zen_browser_search_promotes_the_upstream_flake() {
        let hits = super::search_flakes("zen-browser").await.unwrap();

        assert_eq!(
            hits.first().map(|hit| hit.repo.as_str()),
            Some("0xc000022070/zen-browser-flake")
        );
    }

    #[tokio::test]
    #[ignore = "requires authenticated gh access, Nix evaluation, and GitHub search quota"]
    async fn live_bun_search_finds_an_installable_community_flake() {
        let hits = super::search_flakes("bun").await.unwrap();

        assert_eq!(
            hits.first().map(|hit| hit.repo.as_str()),
            Some("alleneubank/bun-overlay")
        );
        assert!(hits.iter().all(|hit| hit.repo != "oven-sh/bun"));
        assert!(
            hits.first()
                .and_then(|hit| hit.packages.first())
                .is_some_and(|package| package.attr == "default")
        );
        assert!(
            hits.first()
                .is_some_and(|hit| hit.packages.iter().any(|package| package.attr == "bun"))
        );
    }
}
