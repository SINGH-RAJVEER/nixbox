use super::*;
use crate::manifest::{ManagedFile, ManagedFlakeFile};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct FileChange {
	pub path: PathBuf,
	pub before: Option<String>,
	pub after: Option<String>,
}

impl FileChange {
	pub fn apply_all(changes: &[Self]) -> Result<()> {
		for change in changes {
			for path in change.path.ancestors() {
				if let Ok(metadata) = fs::symlink_metadata(path) {
					ensure!(
						!metadata.is_symlink(),
						"migration path became symlinked: {}",
						path.display()
					);
				}
			}
			ensure!(
				fs::read_to_string(&change.path).ok() == change.before,
				"{} changed after the migration preview",
				change.path.display()
			);
			if let Some(after) = &change.after {
				parsed(after)?;
			}
		}
		for (index, change) in changes.iter().enumerate() {
			let result = match &change.after {
				Some(after) => fs::create_dir_all(
					change
						.path
						.parent()
						.context("migration path has no parent")?,
				)
				.and_then(|()| fs::write(&change.path, after)),
				None => fs::remove_file(&change.path),
			};
			if let Err(error) = result {
				for previous in changes[..index].iter().rev() {
					if let Some(before) = &previous.before {
						fs::write(&previous.path, before)?;
					} else if previous.path.exists() {
						fs::remove_file(&previous.path)?;
					}
				}
				return Err(error.into());
			}
		}
		Ok(())
	}
}

fn normalized(source: &str) -> Result<Vec<String>> {
	Ok(parsed(source)?
		.descendants_with_tokens()
		.filter_map(|item| item.into_token())
		.filter(|token| !matches!(token.kind(), Kind::TOKEN_WHITESPACE | Kind::TOKEN_COMMENT))
		.map(|token| token.to_string())
		.collect())
}

fn cleared_blocks(source: &str) -> Result<String> {
	let mut inside = false;
	let mut out = String::new();
	for line in source.split_inclusive('\n') {
		let marker = line.trim();
		if marker.starts_with("# nixbox:") && marker.ends_with(":start") {
			ensure!(!inside, "nested managed markers need manual repair");
			inside = true;
			continue;
		}
		if marker.starts_with("# nixbox:") && marker.ends_with(":end") {
			ensure!(inside, "unmatched managed marker");
			inside = false;
			continue;
		}
		if !inside {
			out.push_str(line);
		}
	}
	ensure!(!inside, "unterminated managed marker");
	parsed(&out)?;
	Ok(out)
}

fn relative(from: &Path, to: &Path) -> String {
	let from: Vec<_> = from.components().collect();
	let to: Vec<_> = to.components().collect();
	let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
	let mut path = PathBuf::from(".");
	for _ in common..from.len() {
		path.push("..");
	}
	for component in &to[common..] {
		path.push(component.as_os_str());
	}
	path.to_string_lossy().into_owned()
}

fn absolute(path: &Path) -> PathBuf {
	let mut result = PathBuf::new();
	for component in path.components() {
		match component {
			std::path::Component::ParentDir => {
				result.pop();
			}
			std::path::Component::CurDir => {}
			_ => result.push(component.as_os_str()),
		}
	}
	result
}

fn edit_paths(
	source: &str,
	original: &Path,
	destination: &Path,
	removed: &[PathBuf],
) -> Result<String> {
	let mut edits = Vec::new();
	for node in parsed(source)?
		.descendants()
		.filter(|node| matches!(node.kind(), Kind::NODE_PATH_REL | Kind::NODE_PATH_ABS))
	{
		ensure!(
			node.children().count() == 0,
			"interpolated relative paths need manual migration"
		);
		let path = absolute(
			&original
				.parent()
				.context("source has no parent")?
				.join(node.to_string()),
		);
		if removed.contains(&path) {
			ensure!(
				node.parent()
					.filter(|parent| parent.kind() == Kind::NODE_LIST)
					.and_then(|list| list.parent())
					.filter(|entry| entry.kind() == Kind::NODE_ATTRPATH_VALUE)
					.and_then(|entry| entry.children().next())
					.and_then(|path| attrpath(&path))
					.is_some_and(|path| path.last().is_some_and(|name| name == "imports")),
				"managed module is referenced outside a literal import list"
			);
			edits.push((
				std::ops::Range::<usize>::from(node.text_range()),
				String::new(),
			));
		} else if original != destination && node.kind() == Kind::NODE_PATH_REL {
			edits.push((
				std::ops::Range::<usize>::from(node.text_range()),
				relative(
					destination.parent().context("destination has no parent")?,
					&path,
				),
			));
		}
	}
	edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
	let mut out = source.to_string();
	for (range, replacement) in edits {
		out.replace_range(range, &replacement);
	}
	parsed(&out)?;
	Ok(out)
}

impl Layout {
	/// Preview conversion and relocation. Custom declarations in configuration
	/// mode are kept in a separate, user-editable classed module.
	pub fn plan_managed_migration(&self, relocate: bool) -> Result<Vec<FileChange>> {
		let mut planned: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
		let mut migrations = Vec::new();
		let mut legacy_flakes = BTreeMap::new();
		for (home, suffix) in [(false, "system"), (true, "home")] {
			if self.standalone && !home {
				continue;
			}
			let directory = if self.standalone {
				self.tree.join("nixbox/homes").join(encode_name(&self.host))
			} else {
				let directory = self.tree.join("nixbox/hosts").join(encode_name(&self.host));
				if home && self.home_user.is_some() {
					directory
						.join("users")
						.join(encode_name(self.require_home()?))
				} else {
					directory
				}
			};
			let legacy = self.root.join(format!("nixbox-{suffix}-flakes.nix"));
			if legacy.exists() {
				let source = fs::read_to_string(&legacy)?;
				let manifest = ManagedFlakeFile::new(legacy.clone()).load()?;
				let canonical = crate::manifest::render_legacy_flakes(
					&manifest,
					if home {
						"home.packages"
					} else {
						"environment.systemPackages"
					},
				);
				ensure!(
					source.starts_with("# Managed by nixbox. Do not edit by hand.\n")
						&& normalized(&source)? == normalized(&canonical)?,
					"custom legacy flake declarations need manual extraction before conversion"
				);
				legacy_flakes.insert(home, manifest);
				migrations.push((
					legacy,
					directory.join(format!("nixbox-{suffix}-packages.nix")),
					home,
					"legacy".into(),
					source,
					false,
				));
			}
			for kind in ["packages", "settings", "integration"] {
				if kind == "integration" && !home {
					continue;
				}
				let name = format!("nixbox-{suffix}-{kind}.nix");
				let path = self.root.join(&name);
				if !path.exists() && !(kind == "packages" && legacy_flakes.contains_key(&home)) {
					continue;
				}
				if home {
					self.require_home()?;
				}
				let source = if path.exists() {
					fs::read_to_string(&path)?
				} else {
					crate::manifest::rendered_module(&Default::default(), &Default::default(), home)
				};
				ensure!(
					source.starts_with("# Managed by nixbox. Do not edit by hand.\n"),
					"{} is not an owned managed module",
					path.display()
				);
				let parts = source.lines().any(|line| line == MARKER);
				ensure!(
					!parts || !legacy_flakes.contains_key(&home),
					"legacy flake declarations coexist with a root flake-parts module; merge those outputs before relocation"
				);
				if parts && (!relocate || !self.owns_root_file(&path)?) {
					continue;
				}
				let destination = directory.join(&name);
				self.validate_import(&destination)?;
				ensure!(
					!destination.exists(),
					"{} already exists; merge it explicitly before relocation",
					destination.display()
				);
				migrations.push((path, destination, home, kind.to_string(), source, parts));
			}
		}
		let removed: Vec<_> = migrations.iter().map(|(path, ..)| absolute(path)).collect();
		for file in &self.files {
			let source = fs::read_to_string(file)?;
			for node in parsed(&source)?.descendants().filter(|node| {
				matches!(node.kind(), Kind::NODE_PATH_REL | Kind::NODE_PATH_ABS)
					&& node.children().count() == 0
			}) {
				let referenced = absolute(
					&file
						.parent()
						.context("source has no parent")?
						.join(node.to_string()),
				);
				let Some((_, _, home, kind, _, parts)) = migrations
					.iter()
					.find(|(path, ..)| absolute(path) == referenced)
				else {
					continue;
				};
				if *parts && file == &self.root.join("flake.nix") {
					continue;
				}
				let class_home = *home && kind != "integration";
				let sources = self.connected_sources(class_home)?;
				let shared = self.shared_sources(class_home)?;
				let line = source[..usize::from(node.text_range().start())]
					.bytes()
					.filter(|byte| *byte == b'\n')
					.count();
				ensure!(
					sources
						.iter()
						.any(|(name, offset, body)| self.root.join(name) == *file
							&& line >= *offset && line
							< offset + body.lines().count()
							&& !shared.contains(&(name.clone(), *offset))),
					"{} imports the managed file for an unselected or shared target; split that import before converting",
					file.display()
				);
			}
			let after = edit_paths(&source, file, file, &removed)?;
			if source != after {
				planned.insert(file.clone(), Some(after));
			}
		}
		for (path, destination, home, kind, source, parts) in migrations {
			if kind == "legacy" {
				planned.insert(path, None);
				continue;
			}
			if parts {
				planned.insert(
					destination.clone(),
					Some(edit_paths(&source, &path, &destination, &removed)?),
				);
			} else {
				let canonical_before = match kind.as_str() {
					"packages" => crate::manifest::rendered_module(
						&ManagedFile::new(path.clone()).load()?,
						&ManagedFlakeFile::new(path.clone()).load()?,
						home,
					),
					"settings" => crate::settings::render(
						&crate::settings::SettingsFile::new(path.clone()).load()?,
					),
					_ => {
						let enabled = normalized(&source)?
							== normalized(&crate::manifest::rendered_home_integration(true))?;
						let canonical = crate::manifest::rendered_home_integration(enabled);
						ensure!(
							normalized(&source)? == normalized(&canonical)?,
							"custom profile integration needs manual extraction before conversion"
						);
						canonical
					}
				};
				let canonical = if kind == "packages" && legacy_flakes.contains_key(&home) {
					let mut flakes = ManagedFlakeFile::new(path.clone()).load()?;
					for (repo, input) in &legacy_flakes[&home].inputs {
						ensure!(
							flakes
								.inputs
								.get(repo)
								.is_none_or(|previous| previous == input),
							"conflicting input aliases for {repo}"
						);
						flakes.inputs.insert(repo.clone(), input.clone());
					}
					for (repo, modules) in &legacy_flakes[&home].modules {
						flakes
							.modules
							.entry(repo.clone())
							.or_default()
							.extend(modules.iter().cloned());
					}
					for (repo, packages) in &legacy_flakes[&home].packages {
						flakes
							.packages
							.entry(repo.clone())
							.or_default()
							.extend(packages.iter().cloned());
					}
					crate::manifest::rendered_module(
						&ManagedFile::new(path.clone()).load()?,
						&flakes,
						home,
					)
				} else {
					canonical_before.clone()
				};
				if normalized(&source)? != normalized(&canonical_before)? {
					ensure!(
						!source.lines().any(|line| line == "# nixbox:mode profile"),
						"custom profile-mode declarations need manual extraction before conversion"
					);
					let custom = cleared_blocks(&source)?;
					let custom_path = destination.with_file_name(if home {
						"nixbox-home-custom.nix"
					} else {
						"nixbox-system-custom.nix"
					});
					ensure!(
						!custom_path.exists() && !planned.contains_key(&custom_path),
						"custom migration module already exists"
					);
					let custom = edit_paths(&custom, &path, &custom_path, &removed)?;
					let custom = self.render(&custom_path, &custom)?.replacen(
						"# Managed by nixbox. Do not edit by hand.",
						"# Preserved custom declarations. Editable by the user.",
						1,
					);
					planned.insert(custom_path, Some(custom));
				}
				planned.insert(
					destination.clone(),
					Some(self.render(&destination, &canonical)?),
				);
			}
			planned.insert(path, None);
		}
		let mut changes = Vec::new();
		for (path, after) in planned {
			let before = fs::read_to_string(&path).ok();
			if before != after {
				changes.push(FileChange {
					path,
					before,
					after,
				});
			}
		}
		// Create destination modules before removing old imports and sources.
		changes.sort_by_key(|change| (change.after.is_none(), change.before.is_some()));
		Ok(changes)
	}
}
