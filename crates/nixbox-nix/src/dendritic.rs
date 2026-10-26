//! Conservative integration with import-tree/flake-parts configurations.
//!
//! Discovery never evaluates Nix or writes files. Only literal import trees
//! and statically connected classed modules are writable.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use rnix::{SyntaxKind as Kind, SyntaxNode};
use serde::Serialize;

use crate::manifest::ImportStatus;
use crate::wiring::attr_name;

const MARKER: &str = "# nixbox:layout dendritic";
mod migration;
pub use migration::SourceEdit;
mod managed_migration;
pub use managed_migration::FileChange;

#[derive(Debug, Clone)]
pub struct Layout {
	root: PathBuf,
	files: Vec<PathBuf>,
	pub tree: PathBuf,
	pub host: String,
	pub host_module: String,
	pub home_user: Option<String>,
	pub standalone: bool,
	predicates: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostInfo {
	pub output: String,
	pub module: String,
	pub users: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Discovery {
	pub root: PathBuf,
	pub tree: PathBuf,
	pub hosts: Vec<HostInfo>,
	pub homes: Vec<HomeInfo>,
	pub sources: Vec<SourceInfo>,
	pub generated_inputs: bool,
	#[serde(skip)]
	files: Vec<PathBuf>,
	#[serde(skip)]
	predicates: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SourceInfo {
	pub path: PathBuf,
	pub modules: Vec<String>,
	pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HomeInfo {
	pub output: String,
	pub module: String,
	pub user: Option<String>,
}

pub fn encode_name(name: &str) -> String {
	name.as_bytes()
		.iter()
		.map(|byte| format!("{byte:02x}"))
		.collect()
}

fn decode_name(name: &str) -> Result<String> {
	ensure!(name.len().is_multiple_of(2), "invalid encoded target name");
	let bytes = name
		.as_bytes()
		.as_chunks::<2>()
		.0
		.iter()
		.map(|pair| u8::from_str_radix(std::str::from_utf8(pair)?, 16).map_err(Into::into))
		.collect::<Result<Vec<_>>>()?;
	Ok(String::from_utf8(bytes)?)
}

fn parsed(source: &str) -> Result<SyntaxNode> {
	let parsed = rnix::Root::parse(source);
	ensure!(
		parsed.errors().is_empty(),
		"invalid Nix syntax: {:?}",
		parsed.errors()
	);
	parsed
		.syntax()
		.children()
		.next()
		.context("empty Nix expression")
}

fn unparen(mut node: SyntaxNode) -> SyntaxNode {
	while node.kind() == Kind::NODE_PAREN {
		node = node
			.children()
			.next()
			.expect("parsed parenthesized expression");
	}
	node
}

fn resolve(mut node: SyntaxNode) -> SyntaxNode {
	for _ in 0..32 {
		node = unparen(node);
		if node.kind() == Kind::NODE_LET_IN {
			node = node.children().last().expect("parsed let expression");
			continue;
		}
		if node.kind() != Kind::NODE_IDENT {
			break;
		}
		let name = node.to_string();
		let value = node
			.ancestors()
			.filter(|ancestor| ancestor.kind() == Kind::NODE_LET_IN)
			.find_map(|scope| {
				scope
					.children()
					.filter(|entry| entry.kind() == Kind::NODE_ATTRPATH_VALUE)
					.find_map(|entry| {
						let mut children = entry.children();
						let path = attrpath(&children.next()?)?;
						(path == [name.as_str()]).then(|| children.next()).flatten()
					})
			});
		let Some(value) = value else {
			break;
		};
		node = value;
	}
	node
}

fn body(node: SyntaxNode) -> SyntaxNode {
	let node = resolve(node);
	if node.kind() == Kind::NODE_LAMBDA {
		resolve(node.children().last().expect("parsed lambda body"))
	} else {
		node
	}
}

fn attrpath(node: &SyntaxNode) -> Option<Vec<String>> {
	node.children()
		.map(|attr| match attr.kind() {
			Kind::NODE_IDENT => Some(attr.to_string()),
			Kind::NODE_STRING if !attr.to_string().contains("${") => {
				serde_json::from_str(&attr.to_string()).ok()
			}
			_ => None,
		})
		.collect()
}

fn bindings(node: SyntaxNode, prefix: &[String], out: &mut Vec<(Vec<String>, SyntaxNode)>) {
	let node = body(node);
	if node.kind() != Kind::NODE_ATTR_SET {
		return;
	}
	for entry in node
		.children()
		.filter(|entry| entry.kind() == Kind::NODE_ATTRPATH_VALUE)
	{
		let mut children = entry.children();
		let Some(path) = children.next().and_then(|path| attrpath(&path)) else {
			continue;
		};
		let Some(value) = children.next() else {
			continue;
		};
		let path: Vec<_> = prefix.iter().cloned().chain(path).collect();
		out.push((path.clone(), value.clone()));
		bindings(value, &path, out);
	}
}

fn select(node: SyntaxNode) -> Option<Vec<String>> {
	let node = resolve(node);
	if node.kind() == Kind::NODE_IDENT {
		return Some(vec![node.to_string()]);
	}
	if node.kind() != Kind::NODE_SELECT || node.children().count() != 2 {
		return None;
	}
	let mut children = node.children();
	let mut path = select(children.next()?)?;
	path.extend(attrpath(&children.next()?)?);
	Some(path)
}

fn matches_path(path: &[String], expected: &[&str]) -> bool {
	path.iter().map(String::as_str).eq(expected.iter().copied())
}

fn root_module(source: &str) -> Result<Option<SyntaxNode>> {
	let mut entries = Vec::new();
	bindings(parsed(source)?, &[], &mut entries);
	let candidate = entries
		.iter()
		.any(|(path, _)| path.starts_with(&["inputs".into(), "import-tree".into()]));
	let Some((_, outputs)) = entries
		.iter()
		.find(|(path, _)| matches_path(path, &["outputs"]))
	else {
		return Ok(None);
	};
	let expression = body(outputs.clone());
	if expression.kind() != Kind::NODE_APPLY {
		ensure!(
			!candidate,
			"unresolved dendritic outputs expression; refusing conventional import edits"
		);
		return Ok(None);
	}
	let mut children = expression.children();
	let call = resolve(children.next().context("missing flake constructor")?);
	if call.kind() != Kind::NODE_APPLY {
		ensure!(
			!candidate,
			"unresolved dendritic constructor; refusing conventional import edits"
		);
		return Ok(None);
	}
	let constructor = call.children().next().and_then(select);
	if !constructor
		.as_ref()
		.is_some_and(|path| matches_path(path, &["inputs", "flake-parts", "lib", "mkFlake"]))
	{
		ensure!(
			!candidate,
			"unsupported dendritic flake-parts constructor alias"
		);
		return Ok(None);
	}
	ensure!(
		outputs.kind() == Kind::NODE_LAMBDA
			&& outputs.children().next().is_some_and(|param| param
				.descendants_with_tokens()
				.filter_map(|item| item.into_token())
				.any(|token| token.text() == "inputs")),
		"dendritic outputs must bind the input set as `inputs` for automatic integration"
	);
	Ok(Some(children.next().context("missing flake-parts module")?))
}

fn tree_path(node: SyntaxNode) -> Option<String> {
	let node = resolve(node);
	if node.kind() != Kind::NODE_APPLY {
		return None;
	}
	let mut children = node.children();
	tree_predicates(children.next()?)?;
	let path = children.next()?;
	(path.kind() == Kind::NODE_PATH_REL && path.children().count() == 0).then(|| path.to_string())
}

fn tree_predicates(node: SyntaxNode) -> Option<Vec<String>> {
	let node = resolve(node);
	if select(node.clone()).is_some_and(|path| matches_path(&path, &["inputs", "import-tree"])) {
		return Some(Vec::new());
	}
	if node.kind() != Kind::NODE_APPLY {
		return None;
	}
	let mut children = node.children();
	let function = resolve(children.next()?);
	if function.kind() != Kind::NODE_SELECT {
		return None;
	}
	let (mut predicates, method) = if let Some(path) = select(function.clone())
		.filter(|path| path.len() == 3 && matches_path(&path[..2], &["inputs", "import-tree"]))
	{
		(Vec::new(), vec![path[2].clone()])
	} else {
		let mut parts = function.children();
		(tree_predicates(parts.next()?)?, attrpath(&parts.next()?)?)
	};
	let argument = children.next()?.to_string();
	predicates.push(match method.as_slice() {
		[method] if method == "filter" => argument,
		[method] if method == "filterNot" => format!("path: !(({argument}) path)"),
		[method] if method == "match" => format!("path: builtins.match ({argument}) path != null"),
		[method] if method == "matchNot" => {
			format!("path: builtins.match ({argument}) path == null")
		}
		_ => return None,
	});
	Some(predicates)
}

fn accepted_paths(root: &Path, predicates: &[String], paths: &[String]) -> Result<Vec<String>> {
	if predicates.is_empty() {
		return Ok(paths.to_vec());
	}
	let expression = format!(
		"let inputs = (builtins.getFlake {}).inputs; lib = inputs.nixpkgs.lib; predicates = [ {} ]; paths = [ {} ]; in builtins.filter (path: builtins.all (f: f path) predicates) paths",
		serde_json::to_string(&root.canonicalize()?.to_string_lossy())?,
		predicates
			.iter()
			.map(|predicate| format!("({predicate})"))
			.collect::<Vec<_>>()
			.join(" "),
		paths
			.iter()
			.map(serde_json::to_string)
			.collect::<std::result::Result<Vec<_>, _>>()?
			.join(" ")
	);
	let output = std::process::Command::new("nix")
		.args(["eval", "--json", "--impure", "--expr", &expression])
		.output()
		.context("evaluating import-tree filters")?;
	ensure!(
		output.status.success(),
		"import-tree filter evaluation failed: {}",
		String::from_utf8_lossy(&output.stderr)
	);
	Ok(serde_json::from_slice(&output.stdout)?)
}

fn module_items(module: &SyntaxNode) -> Result<Vec<SyntaxNode>> {
	let module = body(module.clone());
	if tree_path(module.clone()).is_some() {
		return Ok(vec![module]);
	}
	ensure!(
		module.kind() == Kind::NODE_ATTR_SET,
		"unsupported flake-parts module expression; expected a literal import-tree or imports list"
	);
	let mut entries = Vec::new();
	bindings(module, &[], &mut entries);
	let imports: Vec<_> = entries
		.into_iter()
		.filter(|(path, _)| matches_path(path, &["imports"]))
		.collect();
	ensure!(
		imports.len() == 1 && imports[0].1.kind() == Kind::NODE_LIST,
		"flake-parts integration requires one literal imports list"
	);
	Ok(imports[0].1.children().collect())
}

fn collect(directory: &Path, root: &Path, files: &mut Vec<PathBuf>, depth: usize) -> Result<()> {
	ensure!(depth < 64, "dendritic import tree is too deep");
	ensure!(files.len() < 4096, "dendritic import tree is too large");
	ensure!(
		!fs::symlink_metadata(directory)?.is_symlink(),
		"symlinked import trees are not supported"
	);
	ensure!(
		directory.canonicalize()?.starts_with(root),
		"import tree escapes configuration root"
	);
	for entry in fs::read_dir(directory)? {
		let entry = entry?;
		if entry.file_name().to_string_lossy().starts_with('_') {
			continue;
		}
		let kind = entry.file_type()?;
		ensure!(
			!kind.is_symlink(),
			"symlinked import-tree entries are not supported: {}",
			entry.path().display()
		);
		if kind.is_dir() {
			collect(&entry.path(), root, files, depth + 1)?;
		} else if entry.path().extension().is_some_and(|ext| ext == "nix") {
			files.push(entry.path());
		}
	}
	Ok(())
}

impl Layout {
	pub fn inspect(root: &Path) -> Result<Option<Discovery>> {
		let flake = root.join("flake.nix");
		if !flake.exists() {
			return Ok(None);
		}
		let source = fs::read_to_string(&flake)?;
		let Some(module) = root_module(&source)? else {
			return Ok(None);
		};
		let items = module_items(&module)?;
		let trees: Vec<_> = items
			.iter()
			.filter_map(|item| tree_path(item.clone()))
			.collect();
		ensure!(
			trees.len() == 1,
			"automatic dendritic integration requires exactly one literal import-tree"
		);
		ensure!(
			root.join(&trees[0]).canonicalize()? != root.canonicalize()?,
			"an import-tree covering the configuration root would load generated modules twice; use a dedicated modules directory"
		);
		let mut files = Vec::new();
		collect(&root.join(&trees[0]), &root.canonicalize()?, &mut files, 0)?;
		let predicates = items
			.iter()
			.find(|item| tree_path((*item).clone()).is_some())
			.and_then(|item| resolve(item.clone()).children().next())
			.and_then(tree_predicates)
			.unwrap_or_default();
		let tree = root.join(&trees[0]);
		if !predicates.is_empty() {
			let relative: Vec<_> = files
				.iter()
				.map(|file| {
					format!(
						"/{}",
						file.strip_prefix(&tree)
							.expect("collected tree path")
							.display()
					)
				})
				.collect();
			let accepted = accepted_paths(root, &predicates, &relative)?;
			files.retain(|file| {
				accepted.contains(&format!(
					"/{}",
					file.strip_prefix(&tree)
						.expect("collected tree path")
						.display()
				))
			});
		}
		for item in &items {
			if tree_path(item.clone()).is_some() {
				continue;
			}
			if select(item.clone()).is_some_and(|path| {
				path.len() >= 4 && path[0] == "inputs" && path[2] == "flakeModules"
			}) {
				continue;
			}
			ensure!(
				item.kind() == Kind::NODE_PATH_REL && item.children().count() == 0,
				"computed root imports require an explicit discovery adapter"
			);
			let path = root.join(item.to_string());
			ensure!(
				!fs::symlink_metadata(&path)?.is_symlink()
					&& path.canonicalize()?.starts_with(root.canonicalize()?),
				"root import escapes the configuration or is symlinked"
			);
			files.push(path);
		}
		files.push(flake);
		files.retain(|file| {
			!fs::read_to_string(file).is_ok_and(|source| {
				source.starts_with("# Managed by nixbox. Do not edit by hand.\n")
					&& source.lines().any(|line| line == MARKER)
			})
		});
		files.sort();
		files.dedup();
		let mut entries = Vec::new();
		let mut sources = Vec::new();
		for file in &files {
			let source = fs::read_to_string(file)?;
			bindings(
				if file == &root.join("flake.nix") {
					module.clone()
				} else {
					parsed(&source).with_context(|| format!("reading {}", file.display()))?
				},
				&[],
				&mut entries,
			);
			let mut local = Vec::new();
			bindings(
				if file == &root.join("flake.nix") {
					module.clone()
				} else {
					parsed(&source)?
				},
				&[],
				&mut local,
			);
			let modules = local
				.iter()
				.filter(|(path, _)| {
					path.len() == 4 && matches_path(&path[..2], &["flake", "modules"])
				})
				.map(|(path, _)| path[2..].join("."))
				.collect();
			let diagnostics = local
				.iter()
				.filter(|(path, value)| {
					path.last().is_some_and(|name| name == "imports")
						&& resolve(value.clone()).kind() != Kind::NODE_LIST
				})
				.map(|(path, _)| {
					format!(
						"{} is computed; connected-source scanning is incomplete",
						path.join(".")
					)
				})
				.collect();
			sources.push(SourceInfo {
				path: file.strip_prefix(root)?.to_path_buf(),
				modules,
				diagnostics,
			});
		}
		let generated_inputs = source
			.lines()
			.take(5)
			.any(|line| line.starts_with('#') && line.contains("flake-file"))
			|| entries.iter().any(|(path, value)| {
				path.first().is_some_and(|name| name == "flake-file")
					|| value.descendants().filter_map(select).any(|path| {
						path.len() >= 3
							&& matches_path(&path[..3], &["inputs", "flake-file", "flakeModules"])
					})
			});
		ensure!(
			!entries.iter().any(|(path, _)| path.len() >= 4
				&& matches_path(&path[..2], &["flake", "modules"])
				&& ["nixboxPackages", "nixboxSettings", "nixboxHomeIntegration"]
					.iter()
					.any(|name| path[3] == *name || path[3].starts_with(&format!("{name}-")))),
			"the configuration already defines a reserved NixBox module name"
		);
		let outputs: Vec<_> = entries
			.iter()
			.filter(|(path, _)| {
				path.len() == 3 && matches_path(&path[..2], &["flake", "nixosConfigurations"])
			})
			.collect();
		let mut hosts = Vec::new();
		for (output, constructor) in outputs {
			let constructor = resolve(constructor.clone());
			ensure!(
				constructor.kind() == Kind::NODE_APPLY
					&& constructor
						.children()
						.next()
						.and_then(select)
						.is_some_and(|path| matches_path(
							&path,
							&["inputs", "nixpkgs", "lib", "nixosSystem"]
						)),
				"unsupported constructor for host {}",
				output[2]
			);
			let mut args = Vec::new();
			bindings(
				constructor
					.children()
					.nth(1)
					.context("missing nixosSystem arguments")?,
				&[],
				&mut args,
			);
			let modules = args
				.iter()
				.find(|(path, _)| matches_path(path, &["modules"]))
				.context("missing host modules")?;
			let references: Vec<_> = modules
				.1
				.children()
				.filter_map(select)
				.filter(|path| {
					path.len() == 5
						&& matches_path(&path[..4], &["config", "flake", "modules", "nixos"])
				})
				.collect();
			ensure!(
				modules.1.kind() == Kind::NODE_LIST && references.len() == 1,
				"host modules must contain exactly one host-specific classed reference"
			);
			let reference = &references[0];
			ensure!(
				reference.len() == 5
					&& matches_path(&reference[..4], &["config", "flake", "modules", "nixos"]),
				"unsupported host module reference"
			);
			let host_path = vec![
				"flake".into(),
				"modules".into(),
				"nixos".into(),
				reference[4].clone(),
			];
			ensure!(
				entries.iter().any(|(path, _)| path == &host_path),
				"host module is not defined in the import tree"
			);
			let users: BTreeSet<_> = entries
				.iter()
				.filter(|(path, _)| {
					path.len() >= 7
						&& path.starts_with(&host_path)
						&& matches_path(&path[4..6], &["home-manager", "users"])
				})
				.map(|(path, _)| path[6].clone())
				.collect();
			hosts.push(HostInfo {
				output: output[2].clone(),
				module: reference[4].clone(),
				users: users.into_iter().collect(),
			});
		}
		let mut homes = Vec::new();
		for (path, constructor) in &entries {
			if path.len() != 3 || !matches_path(&path[..2], &["flake", "homeConfigurations"]) {
				continue;
			}
			let constructor = resolve(constructor.clone());
			ensure!(
				constructor.kind() == Kind::NODE_APPLY
					&& constructor
						.children()
						.next()
						.and_then(select)
						.is_some_and(|path| matches_path(
							&path,
							&["inputs", "home-manager", "lib", "homeManagerConfiguration"]
						)),
				"unsupported Home Manager constructor for {}",
				path[2]
			);
			let mut args = Vec::new();
			bindings(
				constructor
					.children()
					.nth(1)
					.context("missing homeManagerConfiguration arguments")?,
				&[],
				&mut args,
			);
			let modules = args
				.iter()
				.find(|(path, _)| matches_path(path, &["modules"]))
				.context("missing Home Manager modules")?;
			let reference = modules
				.1
				.children()
				.next()
				.and_then(select)
				.context("missing Home Manager classed reference")?;
			ensure!(
				modules.1.kind() == Kind::NODE_LIST
					&& modules.1.children().count() == 1
					&& reference.len() == 5
					&& matches_path(
						&reference[..4],
						&["config", "flake", "modules", "homeManager"]
					),
				"Home Manager modules must contain one classed reference"
			);
			ensure!(
				entries
					.iter()
					.any(|(path, _)| path == &["flake", "modules", "homeManager", &reference[4]]),
				"Home Manager module is not defined in the import tree"
			);
			let mut home_entries = Vec::new();
			for (_, value) in entries
				.iter()
				.filter(|(path, _)| path == &["flake", "modules", "homeManager", &reference[4]])
			{
				bindings(value.clone(), &[], &mut home_entries);
			}
			let user = home_entries
				.iter()
				.find(|(path, _)| matches_path(path, &["home", "username"]))
				.and_then(|(_, value)| serde_json::from_str::<String>(&value.to_string()).ok());
			homes.push(HomeInfo {
				output: path[2].clone(),
				module: reference[4].clone(),
				user,
			});
		}
		ensure!(
			!hosts.is_empty() || !homes.is_empty(),
			"no supported configuration output was found"
		);
		hosts.sort_by(|left, right| left.output.cmp(&right.output));
		Ok(Some(Discovery {
			root: root.to_path_buf(),
			tree: root.join(&trees[0]),
			hosts,
			homes,
			sources,
			generated_inputs,
			predicates,
			files,
		}))
	}

	pub fn discover(root: &Path) -> Result<Option<Self>> {
		Self::select(root, None, None)
	}

	pub fn select(root: &Path, host: Option<&str>, user: Option<&str>) -> Result<Option<Self>> {
		Self::select_output(root, host, user, None)
	}

	pub fn select_output(
		root: &Path,
		host: Option<&str>,
		user: Option<&str>,
		home: Option<&str>,
	) -> Result<Option<Self>> {
		let Some(discovery) = Self::inspect(root)? else {
			return Ok(None);
		};
		if home.is_some() || (host.is_none() && discovery.hosts.is_empty()) {
			ensure!(
				host.is_none(),
				"select either a NixOS host or a standalone Home Manager output"
			);
			let selected = match home {
				Some(name) => discovery
					.homes
					.iter()
					.find(|entry| entry.output == name)
					.with_context(|| format!("Home Manager output {name} was not found"))?,
				None => {
					ensure!(
						discovery.homes.len() == 1,
						"multiple Home Manager outputs found; select --home-configuration"
					);
					&discovery.homes[0]
				}
			};
			ensure!(
				discovery
					.homes
					.iter()
					.filter(|entry| entry.module == selected.module)
					.count() == 1,
				"shared standalone Home Manager modules require a distinct output module before editing"
			);
			return Ok(Some(Self {
				root: discovery.root,
				tree: discovery.tree,
				files: discovery.files,
				host: selected.output.clone(),
				host_module: selected.module.clone(),
				home_user: selected.user.clone(),
				standalone: true,
				predicates: discovery.predicates,
			}));
		}
		let selected = match host {
			Some(name) => discovery
				.hosts
				.iter()
				.find(|host| host.output == name)
				.with_context(|| format!("host {name} was not found"))?,
			None => {
				ensure!(
					discovery.hosts.len() == 1,
					"multiple hosts found; select --host explicitly"
				);
				&discovery.hosts[0]
			}
		};
		ensure!(
			discovery
				.hosts
				.iter()
				.filter(|host| host.module == selected.module)
				.count() == 1,
			"the selected host module is shared by multiple outputs; host-specific attachment is not proven"
		);
		let home_user = match user {
			Some(name) => {
				ensure!(
					selected.users.iter().any(|user| user == name),
					"user {name} is not attached to host {}",
					selected.output
				);
				Some(name.to_string())
			}
			None if selected.users.len() == 1 => selected.users.first().cloned(),
			None => None,
		};
		Ok(Some(Self {
			root: discovery.root,
			files: discovery.files,
			tree: discovery.tree,
			host: selected.output.clone(),
			host_module: selected.module.clone(),
			home_user,
			standalone: false,
			predicates: discovery.predicates,
		}))
	}

	/// Native paths encode the target identity, so low-level writers cannot
	/// silently use another invocation's selection.
	pub fn for_file(file: &Path) -> Result<Option<Self>> {
		for root in file.ancestors().skip(1).take(64) {
			let flake = root.join("flake.nix");
			if !flake.is_file() {
				continue;
			}
			let source = fs::read_to_string(&flake)?;
			if root_module(&source)?.is_none() {
				let mut entries = Vec::new();
				bindings(parsed(&source)?, &[], &mut entries);
				if entries
					.iter()
					.any(|(path, _)| matches_path(path, &["outputs"]))
				{
					return Ok(None);
				}
				continue;
			}
			let discovery = Self::inspect(root)?.context("dendritic layout disappeared")?;
			if let Ok(relative) = file.strip_prefix(discovery.tree.join("nixbox/homes")) {
				let parts: Vec<_> = relative.components().collect();
				ensure!(parts.len() == 2, "invalid standalone managed path");
				let output = decode_name(&parts[0].as_os_str().to_string_lossy())?;
				return Self::select_output(root, None, None, Some(&output));
			}
			let native = discovery.tree.join("nixbox").join("hosts");
			if let Ok(relative) = file.strip_prefix(&native) {
				let parts: Vec<_> = relative
					.components()
					.map(|part| part.as_os_str().to_string_lossy().into_owned())
					.collect();
				ensure!(
					parts.len() == 2 || (parts.len() == 4 && parts[1] == "users"),
					"invalid native managed path"
				);
				let host = decode_name(&parts[0])?;
				let user = if parts.len() == 4 {
					Some(decode_name(&parts[2])?)
				} else {
					None
				};
				return Self::select(root, Some(&host), user.as_deref());
			}
			let source = fs::read_to_string(file).unwrap_or_default();
			let output = source
				.lines()
				.find_map(|line| line.strip_prefix("# nixbox:output "));
			let user = source.lines().find_map(|line| {
				line.strip_prefix("# nixbox:user ")
					.filter(|name| !name.is_empty())
			});
			let output = output.map(decode_name).transpose()?;
			let user = user.map(decode_name).transpose()?;
			return Self::select(root, output.as_deref(), user.as_deref());
		}
		Ok(None)
	}

	pub fn owns_root_file(&self, file: &Path) -> Result<bool> {
		if file.parent() != Some(self.root.as_path()) || !file.is_file() {
			return Ok(false);
		}
		let source = fs::read_to_string(file)?;
		if !source.starts_with("# Managed by nixbox. Do not edit by hand.\n")
			|| !source.lines().any(|line| line == MARKER)
		{
			return Ok(false);
		}
		let mut entries = Vec::new();
		bindings(parsed(&source)?, &[], &mut entries);
		let home = file.file_name().is_some_and(|name| {
			name == "nixbox-home-packages.nix" || name == "nixbox-home-settings.nix"
		});
		let mut path = vec![
			"flake".into(),
			"modules".into(),
			"nixos".into(),
			self.host_module.clone(),
		];
		if home && self.home_user.is_none() {
			return Ok(false);
		}
		if home {
			path.extend([
				"home-manager".into(),
				"users".into(),
				self.require_home()?.to_string(),
			]);
		}
		path.push("imports".into());
		Ok(entries.iter().any(|(entry, _)| entry == &path))
	}

	pub fn require_home(&self) -> Result<&str> {
		self.home_user
			.as_deref()
			.or_else(|| self.standalone.then_some(self.host.as_str()))
			.context("select --home-user: no unique Home Manager user was resolved for this host")
	}

	/// Read only the module contributions reachable through literal classed
	/// references. Source removal deliberately remains disabled.
	fn connected_sources(&self, home: bool) -> Result<Vec<(String, usize, String)>> {
		if self.standalone && !home {
			return Ok(Vec::new());
		}
		let class = if home { "homeManager" } else { "nixos" };
		let mut definitions = Vec::new();
		let mut roots = BTreeSet::new();
		for file in &self.files {
			let source = fs::read_to_string(file)?;
			let mut entries = Vec::new();
			bindings(
				if file == &self.root.join("flake.nix") {
					root_module(&source)?.context("root module disappeared")?
				} else {
					parsed(&source)?
				},
				&[],
				&mut entries,
			);
			for (path, value) in entries {
				if home
					&& path
						== [
							"flake",
							"modules",
							"nixos",
							&self.host_module,
							"home-manager",
							"users",
							self.require_home()?,
						] && let Some(reference) = select(value.clone())
					&& reference.len() == 5
					&& matches_path(
						&reference[..4],
						&["config", "flake", "modules", "homeManager"],
					) {
					roots.insert(reference[4].clone());
				}
				if path.len() == 4 && matches_path(&path[..3], &["flake", "modules", class]) {
					let offset = source[..usize::from(value.text_range().start())]
						.bytes()
						.filter(|byte| *byte == b'\n')
						.count();
					definitions.push((
						path[3].clone(),
						file.strip_prefix(&self.root)?.display().to_string(),
						offset,
						value,
					));
				}
			}
		}
		if !home || self.standalone {
			roots.insert(self.host_module.clone());
		}
		let mut pending: Vec<_> = roots.into_iter().collect();
		let mut visited = BTreeSet::new();
		let mut out = Vec::new();
		while let Some(name) = pending.pop() {
			if !visited.insert(name.clone()) {
				continue;
			}
			for (_, file, offset, value) in definitions.iter().filter(|(key, _, _, _)| key == &name)
			{
				let mut entries = Vec::new();
				bindings(value.clone(), &[], &mut entries);
				for (path, value) in entries {
					if matches_path(&path, &["imports"]) && value.kind() == Kind::NODE_LIST {
						for reference in value.children().filter_map(select) {
							if reference.len() == 5
								&& matches_path(
									&reference[..4],
									&["config", "flake", "modules", class],
								) {
								pending.push(reference[4].clone());
							}
						}
					}
				}
				out.push((file.clone(), *offset, value.to_string()));
			}
		}
		Ok(out)
	}

	pub fn scan_packages(&self, home: bool) -> Result<Vec<crate::scan::ExternalPackage>> {
		let target = if home {
			crate::scan::ScanTarget::HomeManager
		} else {
			crate::scan::ScanTarget::Nixos
		};
		let mut out = Vec::new();
		let shared = self.shared_sources(home)?;
		for (file, offset, source) in self.connected_sources(home)? {
			for mut entry in crate::scan::parse(&source, target) {
				entry.source_attr = format!("{file}: {}", entry.source_attr);
				entry.line += offset;
				entry.migratable &= !shared.contains(&(file.clone(), offset));
				out.push(entry);
			}
		}
		Ok(out)
	}

	pub fn scan_flake_packages(
		&self,
		home: bool,
	) -> Result<Vec<crate::scan::ExternalFlakePackage>> {
		let target = if home {
			crate::scan::ScanTarget::HomeManager
		} else {
			crate::scan::ScanTarget::Nixos
		};
		let mut out = Vec::new();
		let shared = self.shared_sources(home)?;
		for (file, offset, source) in self.connected_sources(home)? {
			for mut entry in crate::scan::parse_flake_packages(&source, target) {
				entry.source_attr = format!("{file}: {}", entry.source_attr);
				entry.line += offset;
				entry.migratable &= !shared.contains(&(file.clone(), offset));
				entry.removable &= !shared.contains(&(file.clone(), offset));
				out.push(entry);
			}
		}
		Ok(out)
	}

	fn updated_imports(&self, file: &Path) -> Result<Option<String>> {
		if file.starts_with(self.tree.join("nixbox/hosts"))
			|| file.starts_with(self.tree.join("nixbox/homes"))
		{
			ensure!(
				!file
					.strip_prefix(&self.tree)?
					.components()
					.any(|part| part.as_os_str().to_string_lossy().starts_with('_')),
				"generated path is excluded by import-tree"
			);
			return Ok(None);
		}
		ensure!(
			file.parent() == Some(self.root.as_path()),
			"managed dendritic modules must be at the configuration root"
		);
		let name = file
			.file_name()
			.and_then(|name| name.to_str())
			.context("invalid managed filename")?;
		let import = format!("./{name}");
		let source = fs::read_to_string(self.root.join("flake.nix"))?;
		let module = root_module(&source)?.context("flake-parts constructor changed")?;
		let items = module_items(&module)?;
		if items
			.iter()
			.any(|item| item.kind() == Kind::NODE_PATH_REL && item.to_string() == import)
		{
			return Ok(None);
		}
		let replacement = if tree_path(module.clone()).is_some() {
			format!("{{\n\t\timports = [\n\t\t\t{module}\n\t\t\t{import}\n\t\t];\n\t}}")
		} else {
			let mut entries = Vec::new();
			bindings(unparen(module.clone()), &[], &mut entries);
			let list = &entries
				.iter()
				.find(|(path, _)| matches_path(path, &["imports"]))
				.context("missing imports")?
				.1;
			let offset = usize::from(list.text_range().start() - module.text_range().start()) + 1;
			let mut replacement = module.to_string();
			let insertion = if list.to_string().contains('\n') {
				format!("\n\t\t\t{import}")
			} else {
				format!(" {import} ")
			};
			replacement.insert_str(offset, &insertion);
			replacement
		};
		let mut updated = source;
		updated.replace_range(
			std::ops::Range::<usize>::from(module.text_range()),
			&replacement,
		);
		parsed(&updated)?;
		Ok(Some(updated))
	}

	pub fn is_loaded(&self, file: &Path) -> Result<bool> {
		if let Ok(relative) = file.strip_prefix(&self.tree) {
			let candidate = format!("/{}", relative.display());
			if !accepted_paths(
				&self.root,
				&self.predicates,
				std::slice::from_ref(&candidate),
			)?
			.contains(&candidate)
			{
				return Ok(false);
			}
		}
		if self.updated_imports(file)?.is_some() {
			return Ok(false);
		}
		let Ok(source) = fs::read_to_string(file) else {
			return Ok(false);
		};
		if !source.lines().any(|line| line == MARKER) {
			return Ok(false);
		}
		let mut entries = Vec::new();
		bindings(parsed(&source)?, &[], &mut entries);
		Ok(entries
			.iter()
			.any(|(path, _)| path.len() == 4 && matches_path(&path[..2], &["flake", "modules"]))
			&& entries.iter().any(|(path, _)| {
				path.starts_with(&[
					"flake".into(),
					"modules".into(),
					if self.standalone {
						"homeManager"
					} else {
						"nixos"
					}
					.into(),
					self.host_module.clone(),
				]) && path.last().is_some_and(|last| last == "imports")
			}))
	}

	pub fn validate_import(&self, file: &Path) -> Result<()> {
		if let Ok(relative) = file.strip_prefix(&self.tree) {
			let candidate = format!("/{}", relative.display());
			ensure!(
				accepted_paths(
					&self.root,
					&self.predicates,
					std::slice::from_ref(&candidate)
				)?
				.contains(&candidate),
				"the import-tree filter excludes the proposed managed module {}",
				file.display()
			);
		}
		for path in [file.to_path_buf(), self.root.join("flake.nix")] {
			if let Ok(metadata) = fs::symlink_metadata(&path) {
				ensure!(
					metadata.is_file() && !metadata.is_symlink(),
					"expected regular file: {}",
					path.display()
				);
			}
		}
		let mut directory = file.parent();
		while let Some(path) = directory.filter(|path| *path != self.root) {
			if let Ok(metadata) = fs::symlink_metadata(path) {
				ensure!(
					!metadata.is_symlink(),
					"symlinked managed directory is not supported"
				);
			}
			directory = path.parent();
		}
		self.updated_imports(file)?;
		Ok(())
	}

	pub fn ensure_loaded(&self, file: &Path) -> Result<ImportStatus> {
		let Some(updated) = self.updated_imports(file)? else {
			return Ok(ImportStatus::AlreadyImported);
		};
		fs::write(self.root.join("flake.nix"), updated)?;
		Ok(ImportStatus::InsertedIntoList)
	}

	pub fn render(&self, path: &Path, content: &str) -> Result<String> {
		let name = path
			.file_name()
			.and_then(|name| name.to_str())
			.context("invalid managed filename")?;
		let (home, key) = match name {
			"nixbox-system-packages.nix" => (false, "nixboxPackages"),
			"nixbox-home-packages.nix" => (true, "nixboxPackages"),
			"nixbox-system-settings.nix" => (false, "nixboxSettings"),
			"nixbox-home-settings.nix" => (true, "nixboxSettings"),
			"nixbox-home-integration.nix" => (false, "nixboxHomeIntegration"),
			"nixbox-system-custom.nix" => (false, "nixboxCustom"),
			"nixbox-home-custom.nix" => (true, "nixboxCustom"),
			_ => bail!("unsupported dendritic managed file: {name}"),
		};
		let suffix = if home || key == "nixboxHomeIntegration" {
			format!(
				"h{}-u{}",
				encode_name(&self.host),
				encode_name(self.require_home()?)
			)
		} else {
			format!("h{}", encode_name(&self.host))
		};
		let key = format!("{key}-{suffix}");
		let expression = parsed(content)?;
		ensure!(
			expression.kind() == Kind::NODE_LAMBDA,
			"expected generated module function"
		);
		let mut inner = expression.to_string();
		// Generated renderers use this exact argument spelling. Capture inputs
		// lexically; the lower-level module system must never resolve them.
		let parameter = expression
			.children()
			.next()
			.context("missing module parameter")?;
		let parameter_text = parameter.to_string().replace("inputs, ", "");
		inner.replace_range(..usize::from(parameter.text_range().len()), &parameter_text);
		let header = &content[..usize::from(expression.text_range().start())];
		let class = if home { "homeManager" } else { "nixos" };
		let host = attr_name(&self.host_module);
		let attachment = if self.standalone {
			ensure!(home, "standalone Home Manager has no NixOS scope");
			format!("flake.modules.homeManager.{host}.imports")
		} else if home {
			format!(
				"flake.modules.nixos.{host}.home-manager.users.{}.imports",
				attr_name(self.require_home()?)
			)
		} else {
			format!("flake.modules.nixos.{host}.imports")
		};
		// Preserve the body verbatim, including indentation within Nix strings.
		let output = format!(
			"{header}{MARKER}\n# nixbox:output {}\n# nixbox:user {}\n{{ config, inputs, ... }}:\n{{\n\tflake.modules.{class}.{key} =\n{inner};\n\t{attachment} = [ config.flake.modules.{class}.{key} ];\n}}\n",
			encode_name(&self.host),
			self.home_user
				.as_deref()
				.map(encode_name)
				.unwrap_or_default()
		);
		parsed(&output)?;
		Ok(output)
	}
}

/// Write the generated module and its root import as one recoverable edit.
pub(crate) fn write(path: &Path, content: String) -> Result<()> {
	let parent = path.parent().context("managed file has no directory")?;
	let previous = fs::read(path).ok();
	if let Some(layout) = Layout::for_file(path)? {
		layout.validate_import(path)?;
		ensure!(
			previous
				.as_ref()
				.is_none_or(|bytes| String::from_utf8_lossy(bytes)
					.lines()
					.any(|line| line == MARKER)),
			"{} is a conventional module; automatic conversion of existing managed modules is not supported yet",
			path.display()
		);
		let rendered = layout.render(path, &content)?;
		let flake = layout.root.join("flake.nix");
		let old_flake = fs::read_to_string(&flake)?;
		let updated = layout.updated_imports(path)?;
		ensure!(
			fs::read_to_string(&flake)? == old_flake && fs::read(path).ok() == previous,
			"configuration changed during integration planning; retry the operation"
		);
		fs::create_dir_all(parent)?;
		fs::write(path, rendered)?;
		if let Some(updated) = updated
			&& let Err(error) = fs::write(&flake, updated)
		{
			match previous {
				Some(bytes) => fs::write(path, bytes)?,
				None => fs::remove_file(path)?,
			}
			if fs::read(&flake).ok().as_deref() != Some(old_flake.as_bytes()) {
				fs::write(&flake, old_flake)
					.context("restoring root flake after failed import write")?;
			}
			return Err(error.into());
		}
	} else {
		ensure!(
			!previous
				.as_ref()
				.is_some_and(|bytes| String::from_utf8_lossy(bytes).contains(MARKER)),
			"dendritic layout disappeared; refusing to replace a flake-parts module with a plain module"
		);
		fs::create_dir_all(parent)?;
		fs::write(path, content)?;
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn filtered_tree_calls_keep_their_predicates_and_reject_mapping() {
		for method in ["filter", "filterNot", "match", "matchNot"] {
			let source = format!("(inputs.import-tree.{method} (path: true) ./modules)");
			let expression = parsed(&source).unwrap();
			assert_eq!(tree_path(expression.clone()).as_deref(), Some("./modules"));
			let function = resolve(expression).children().next().unwrap();
			assert_eq!(tree_predicates(function).unwrap().len(), 1);
		}
		assert!(
			tree_path(parsed("(inputs.import-tree.map (path: path) ./modules)").unwrap()).is_none()
		);
	}
}
