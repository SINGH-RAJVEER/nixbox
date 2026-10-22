//! Upgrade the two generated modules without discarding unrecognised Nix.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, ensure};
use rnix::{SyntaxKind as Kind, SyntaxNode};

use super::{
	FlakeManifest, Manifest, fs, parse, parse_flakes, pathdiff_relative, render_home_manager,
	render_legacy_flakes, render_nixos,
};

/// Merge a previous release's flake module into its package module, update
/// literal imports under `sources` (directories or individual entry files),
/// then delete the obsolete module. No rebuild or configuration switch runs.
/// All contents and edits are checked before writing. On an I/O error the
/// original files are restored. A leftover legacy file can be merged again
/// without duplicating outputs.
pub fn migrate_flake_file(
	packages: &Path,
	legacy: &Path,
	sources: &[PathBuf],
	home_manager: bool,
) -> Result<bool> {
	if !legacy.try_exists()? {
		return Ok(false);
	}
	ensure!(packages != legacy, "package and legacy paths must differ");
	for path in [packages, legacy] {
		if let Ok(metadata) = fs::symlink_metadata(path) {
			ensure!(
				metadata.is_file() && !metadata.is_symlink(),
				"cannot migrate {}: expected a regular generated file",
				path.display()
			);
		}
	}
	let old = fs::read_to_string(legacy)?;
	let previous = if packages.try_exists()? {
		Some(fs::read_to_string(packages)?)
	} else {
		None
	};
	let manifest = previous.as_deref().map(parse).unwrap_or_default();
	let mut flakes = previous.as_deref().map(parse_flakes).unwrap_or_default();
	let incoming = parse_flakes(&old);
	let option = if home_manager {
		"home.packages"
	} else {
		"environment.systemPackages"
	};
	let old_signature = signature(&old).with_context(|| format!("parsing {}", legacy.display()))?;
	ensure!(
		[option, "nixbox.packages"].into_iter().any(|option| {
			signature(&render_legacy_flakes(&incoming, option))
				.ok()
				.as_ref() == Some(&old_signature)
		}),
		"cannot automatically merge {}: it contains unrecognised edits or incomplete markers; originals were left in place",
		legacy.display()
	);
	if let Some(previous) = &previous {
		ensure!(
			signature(previous)? == signature(&render(&manifest, &flakes, home_manager))?,
			"cannot automatically merge {}: it contains unrecognised edits or incomplete markers; originals were left in place",
			packages.display()
		);
	}
	for (repo, input) in &incoming.inputs {
		ensure!(
			!flakes.contains(repo) || flakes.input_for(repo) == *input,
			"cannot merge {repo}: the package and legacy files reference different inputs"
		);
		flakes.inputs.insert(repo.clone(), input.clone());
	}
	for (repo, outputs) in incoming.modules {
		flakes.modules.entry(repo).or_default().extend(outputs);
	}
	for (repo, outputs) in incoming.packages {
		flakes.packages.entry(repo).or_default().extend(outputs);
	}
	let combined = render(&manifest, &flakes, home_manager);
	ensure!(
		rnix::Root::parse(&combined).errors().is_empty(),
		"merged package module is invalid"
	);

	let mut files = BTreeSet::new();
	for source in sources {
		collect_sources(source, &mut files)?;
	}
	let mut changes = BTreeMap::new();
	for file in files {
		if file == packages || file == legacy {
			continue;
		}
		let source = fs::read_to_string(&file)?;
		// Other broken or unfinished files have no bearing on this migration.
		if !source.contains(
			legacy
				.file_name()
				.and_then(|name| name.to_str())
				.unwrap_or_default(),
		) {
			continue;
		}
		let updated = replace_imports(&source, &file, legacy, packages)
			.with_context(|| format!("updating imports in {}", file.display()))?;
		if updated != source {
			changes.insert(file, (Some(source), updated));
		}
	}
	// Write the combined content before any import starts referring to it.
	let mut written = Vec::new();
	let result = (|| -> Result<()> {
		atomic_write(packages, &combined)?;
		written.push((packages.to_path_buf(), previous.clone()));
		for (file, (original, updated)) in &changes {
			atomic_write(file, updated)?;
			written.push((file.clone(), original.clone()));
		}
		fs::remove_file(legacy).with_context(|| format!("removing {}", legacy.display()))?;
		Ok(())
	})();
	if let Err(error) = result {
		let mut failures = Vec::new();
		for (file, original) in written.into_iter().rev() {
			let restored = match original {
				Some(original) => atomic_write(&file, &original),
				None => fs::remove_file(&file).map_err(Into::into),
			};
			if let Err(error) = restored {
				failures.push(format!("{}: {error}", file.display()));
			}
		}
		return Err(error.context(format!(
			"migrating {}; rollback errors: {failures:?}",
			legacy.display()
		)));
	}
	Ok(true)
}

fn render(manifest: &Manifest, flakes: &FlakeManifest, home_manager: bool) -> String {
	if home_manager {
		render_home_manager(manifest, flakes)
	} else {
		render_nixos(manifest, flakes)
	}
}

/// Compare the generated program, allowing formatting, comments, quoted
/// attribute names, the old system selector, and obsolete empty lists.
/// Everything executable must still be accounted for by the manifests.
fn signature(source: &str) -> Result<Vec<(Kind, String)>> {
	for section in ["packages", "flakes", "flake-packages"] {
		let start = format!("# nixbox:{section}:start");
		let end = format!("# nixbox:{section}:end");
		let starts: Vec<_> = source
			.lines()
			.enumerate()
			.filter(|(_, line)| line.trim() == start)
			.map(|(index, _)| index)
			.collect();
		let ends: Vec<_> = source
			.lines()
			.enumerate()
			.filter(|(_, line)| line.trim() == end)
			.map(|(index, _)| index)
			.collect();
		ensure!(
			(starts.is_empty() && ends.is_empty())
				|| (starts.len() == 1 && ends.len() == 1 && starts[0] < ends[0]),
			"incomplete or duplicate {section} markers; originals were left in place"
		);
	}
	let source = source.replace("${pkgs.system}", "${pkgs.stdenv.hostPlatform.system}");
	let parsed = rnix::Root::parse(&source);
	ensure!(
		parsed.errors().is_empty(),
		"invalid Nix syntax: {:?}",
		parsed.errors()
	);
	let root = parsed.syntax();
	let lambda = root.children().next().context("missing module function")?;
	ensure!(
		lambda.kind() == Kind::NODE_LAMBDA,
		"expected a generated module function"
	);
	let parameter = lambda
		.children()
		.next()
		.context("missing module arguments")?;
	ensure!(
		parameter.kind() == Kind::NODE_PATTERN,
		"expected generated module arguments"
	);
	for (kind, token) in rnix::tokenize(&parameter.to_string()) {
		ensure!(
			kind.is_trivia()
				|| ["{", "}", ",", "...", "inputs", "pkgs", "config", "lib"].contains(&token),
			"unrecognised module argument"
		);
	}
	let body = lambda.children().last().context("missing module body")?;
	let mut edits = Vec::new();
	for node in body.descendants() {
		if node.kind() == Kind::NODE_ATTRPATH_VALUE {
			let children: Vec<_> = node.children().collect();
			if children.len() == 2
				&& [
					"imports",
					"home.packages",
					"environment.systemPackages",
					"nixbox.packages",
				]
				.contains(&children[0].to_string().as_str())
				&& children[1].kind() == Kind::NODE_LIST
				&& children[1].children().next().is_none()
			{
				edits.push((node.text_range(), String::new()));
			}
		} else if node.kind() == Kind::NODE_STRING
			&& node
				.parent()
				.is_some_and(|parent| parent.kind() == Kind::NODE_ATTRPATH)
		{
			let text = node.to_string();
			let bare = super::unquote(&text);
			if super::attr_name(&bare) == bare {
				edits.push((node.text_range(), bare));
			}
		}
	}
	let mut normalized = body.to_string();
	let base = usize::from(body.text_range().start());
	edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start()));
	for (range, replacement) in edits {
		normalized.replace_range(
			usize::from(range.start()) - base..usize::from(range.end()) - base,
			&replacement,
		);
	}
	Ok(rnix::tokenize(&normalized)
		.filter(|(kind, _)| !kind.is_trivia())
		.map(|(kind, text)| (kind, text.to_string()))
		.collect())
}

fn collect_sources(path: &Path, files: &mut BTreeSet<PathBuf>) -> Result<()> {
	let metadata = match fs::symlink_metadata(path) {
		Ok(metadata) => metadata,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
		Err(error) => return Err(error.into()),
	};
	if metadata.is_symlink() {
		if path.extension().is_some_and(|extension| extension == "nix") {
			// Replacing a symlink or skipping one of its imports could leave
			// a configuration referring to the file about to be deleted.
			let source = fs::read_to_string(path)?;
			ensure!(
				!source.contains("nixbox-home-flakes.nix")
					&& !source.contains("nixbox-system-flakes.nix"),
				"cannot automatically migrate imports through symlink {}; originals were left in place",
				path.display()
			);
		}
		return Ok(());
	}
	if metadata.is_dir() {
		for entry in fs::read_dir(path)? {
			let entry = entry?;
			if !entry.file_name().to_string_lossy().starts_with('.') {
				collect_sources(&entry.path(), files)?;
			}
		}
	} else if path.extension().is_some_and(|extension| extension == "nix") {
		files.insert(path.to_path_buf());
	}
	Ok(())
}

fn resolved(path: &Path) -> PathBuf {
	let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
	path.canonicalize().unwrap_or_else(|_| {
		let mut normalized = PathBuf::new();
		for component in path.components() {
			match component {
				Component::CurDir => {}
				Component::ParentDir => {
					normalized.pop();
				}
				component => normalized.push(component.as_os_str()),
			}
		}
		normalized
	})
}

fn literal_path(node: &SyntaxNode, directory: &Path) -> Option<PathBuf> {
	matches!(node.kind(), Kind::NODE_PATH_REL | Kind::NODE_PATH_ABS)
		.then(|| directory.join(node.to_string()))
		.map(|path| resolved(&path))
}

fn replace_imports(source: &str, file: &Path, legacy: &Path, packages: &Path) -> Result<String> {
	let parsed = rnix::Root::parse(source);
	ensure!(
		parsed.errors().is_empty(),
		"cannot update imports in invalid Nix syntax"
	);
	let directory = file.parent().context("missing source directory")?;
	let old = resolved(legacy);
	let new = resolved(packages);
	let name = legacy
		.file_name()
		.and_then(|name| name.to_str())
		.context("missing legacy filename")?;
	for node in parsed.syntax().descendants() {
		if !node.to_string().contains(name) {
			continue;
		}
		let computed_path = matches!(node.kind(), Kind::NODE_PATH_REL | Kind::NODE_PATH_ABS)
			&& node.to_string().contains("${");
		let string_import = node.kind() == Kind::NODE_STRING
			&& node
				.ancestors()
				.find(|ancestor| ancestor.kind() == Kind::NODE_ATTRPATH_VALUE)
				.and_then(|binding| binding.children().next())
				.is_some_and(|attribute| {
					["imports", "modules"].contains(&attribute.to_string().as_str())
				});
		ensure!(
			!computed_path && !string_import,
			"obsolete module uses a computed path or string import; originals were left in place"
		);
	}
	let paths: Vec<_> = parsed
		.syntax()
		.descendants()
		.filter(|node| literal_path(node, directory).is_some())
		.collect();
	let mut replacements = Vec::new();
	let mut used = BTreeSet::new();
	for node in &paths {
		if literal_path(node, directory).as_ref() != Some(&old) {
			continue;
		}
		let parent = node.parent().context("missing path parent")?;
		ensure!(
			parent.kind() == Kind::NODE_LIST,
			"obsolete module is referenced outside a literal import list; originals were left in place"
		);
		// A binding can combine lists with ++. Deduplicate within the whole
		// binding, but never across separate Home Manager users or hosts.
		let group = parent
			.ancestors()
			.find(|node| node.kind() == Kind::NODE_ATTRPATH_VALUE)
			.unwrap_or(parent.clone());
		let has_new = group
			.descendants()
			.any(|other| literal_path(&other, directory).as_ref() == Some(&new));
		let replacement = if has_new || !used.insert(group.text_range().start()) {
			String::new()
		} else {
			let relative = pathdiff_relative(&resolved(packages), directory);
			if relative.starts_with('.') || relative.starts_with('/') {
				relative
			} else {
				format!("./{relative}")
			}
		};
		replacements.push((node.text_range(), replacement));
	}
	let mut updated = source.to_string();
	for (range, replacement) in replacements.into_iter().rev() {
		updated.replace_range(
			usize::from(range.start())..usize::from(range.end()),
			&replacement,
		);
	}
	ensure!(
		rnix::Root::parse(&updated).errors().is_empty(),
		"updated imports would be invalid"
	);
	Ok(updated)
}

fn atomic_write(path: &Path, content: &str) -> Result<()> {
	use std::sync::atomic::{AtomicU64, Ordering};
	static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
	let parent = path.parent().context("missing destination directory")?;
	let temporary = parent.join(format!(
		".{}.nixbox-migrate-{}-{}",
		path.file_name()
			.context("missing filename")?
			.to_string_lossy(),
		std::process::id(),
		NEXT_FILE.fetch_add(1, Ordering::Relaxed)
	));
	let mut file = fs::OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(&temporary)
		.with_context(|| format!("creating migration file for {}", path.display()))?;
	let result = (|| -> Result<()> {
		use std::io::Write;
		if let Ok(metadata) = fs::metadata(path) {
			file.set_permissions(metadata.permissions())?;
		}
		file.write_all(content.as_bytes())?;
		file.sync_all()?;
		fs::rename(&temporary, path)?;
		Ok(())
	})();
	if result.is_err() {
		let _ = fs::remove_file(&temporary);
	}
	result.with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
	use super::super::{ManagedFile, ManagedFlakeFile, PackageMode};
	use super::*;

	struct Fixture(PathBuf);
	impl Fixture {
		fn new(label: &str) -> Self {
			let path =
				std::env::temp_dir().join(format!("nixbox-merge-{label}-{}", std::process::id()));
			let _ = fs::remove_dir_all(&path);
			fs::create_dir_all(&path).unwrap();
			Self(path)
		}
	}
	impl Drop for Fixture {
		fn drop(&mut self) {
			let _ = fs::remove_dir_all(&self.0);
		}
	}

	fn outputs() -> FlakeManifest {
		let mut flakes = FlakeManifest::default();
		flakes.add("owner/module".into(), "homeModules.default".into());
		flakes.add_package("owner/tools".into(), "default".into());
		flakes.add_package("owner/tools".into(), "extra".into());
		flakes.inputs.insert("owner/tools".into(), "tools".into());
		flakes
	}

	#[test]
	fn upgrades_both_targets_and_modes_and_preserves_outputs_on_later_writes() {
		let fixture = Fixture::new("targets");
		for home in [true, false] {
			for mode in [PackageMode::Configuration, PackageMode::Profile] {
				let dir = fixture.0.join(format!("{home}-{mode:?}"));
				fs::create_dir_all(&dir).unwrap();
				let packages = dir.join("nixbox-packages.nix");
				let legacy = dir.join("nixbox-flakes.nix");
				let main = dir.join("main.nix");
				let mut manifest = Manifest {
					mode,
					..Manifest::default()
				};
				manifest.add("fd");
				let flakes = outputs();
				fs::write(
					&packages,
					render(&manifest, &FlakeManifest::default(), home),
				)
				.unwrap();
				let option = match (home, mode) {
					(_, PackageMode::Profile) => "nixbox.packages",
					(true, _) => "home.packages",
					(false, _) => "environment.systemPackages",
				};
				fs::write(&legacy, render_legacy_flakes(&flakes, option)).unwrap();
				fs::write(
					&main,
					"{ ... }: { imports = [ ./nixbox-packages.nix ./nixbox-flakes.nix ]; }",
				)
				.unwrap();
				assert!(
					migrate_flake_file(&packages, &legacy, std::slice::from_ref(&dir), home)
						.unwrap()
				);
				assert!(!legacy.exists());
				assert_eq!(ManagedFile::new(&packages).load().unwrap().mode, mode);
				assert_eq!(
					ManagedFlakeFile::new(&packages).load().unwrap().outputs(),
					flakes.outputs()
				);
				assert_eq!(
					fs::read_to_string(&main).unwrap(),
					"{ ... }: { imports = [ ./nixbox-packages.nix  ]; }"
				);
				let combined = fs::read_to_string(&packages).unwrap();
				assert!(
					!migrate_flake_file(&packages, &legacy, std::slice::from_ref(&dir), home)
						.unwrap()
				);
				assert_eq!(fs::read_to_string(&packages).unwrap(), combined);

				manifest.add("ripgrep");
				let file = ManagedFile::new(&packages);
				if home {
					file.write_home_manager(&manifest).unwrap();
				} else {
					file.write_nixos(&manifest).unwrap();
				}
				assert_eq!(
					ManagedFlakeFile::new(&packages).load().unwrap().outputs(),
					flakes.outputs()
				);
				let mut trimmed = flakes.clone();
				trimmed.remove("owner/tools");
				let file = ManagedFlakeFile::new(&packages);
				if home {
					file.write_home_manager(&trimmed, mode).unwrap();
				} else {
					file.write_nixos(&trimmed, mode).unwrap();
				}
				assert_eq!(
					ManagedFile::new(&packages).load().unwrap().packages,
					manifest.packages
				);
				assert_eq!(ManagedFile::new(&packages).load().unwrap().mode, mode);
				// A leftover legacy file after an interrupted upgrade merges once.
				fs::write(&legacy, render_legacy_flakes(&flakes, option)).unwrap();
				assert!(migrate_flake_file(&packages, &legacy, &[dir], home).unwrap());
				assert_eq!(
					ManagedFlakeFile::new(&packages).load().unwrap().outputs(),
					flakes.outputs()
				);
			}
		}
	}

	#[test]
	fn accepts_old_selectors_quoted_attributes_empty_blocks_and_missing_package_file() {
		let fixture = Fixture::new("old-syntax");
		let packages = fixture.0.join("nixbox-packages.nix");
		let legacy = fixture.0.join("nixbox-flakes.nix");
		let main = fixture.0.join("main.nix");
		fs::write(&legacy, "{ inputs, pkgs, ... }: {\n\timports = [\n\t\t# nixbox:flakes:start\n\t\t# nixbox:flakes:end\n\t];\n\thome.packages = [\n\t\t# nixbox:flake-packages:start\n\t\tinputs.\"owner/tools\".packages.${pkgs.system}.\"bun-latest\"\n\t\t# nixbox:flake-packages:end\n\t];\n}").unwrap();
		fs::write(&main, "{ ... }: { imports = [ ./nixbox-flakes.nix ]; }").unwrap();
		migrate_flake_file(&packages, &legacy, std::slice::from_ref(&fixture.0), true).unwrap();
		assert!(
			fs::read_to_string(&main)
				.unwrap()
				.contains("[ ./nixbox-packages.nix ]")
		);
		assert!(
			ManagedFlakeFile::new(&packages).load().unwrap().packages["owner/tools"]
				.contains("bun-latest")
		);
	}

	#[test]
	fn edits_only_actual_import_paths_and_keeps_each_host_wired() {
		let fixture = Fixture::new("imports");
		let packages = fixture.0.join("nixbox-packages.nix");
		let legacy = fixture.0.join("nixbox-flakes.nix");
		let nested = fixture.0.join("hosts");
		fs::create_dir_all(&nested).unwrap();
		let main = nested.join("host.nix");
		fs::write(&legacy, render_legacy_flakes(&outputs(), "home.packages")).unwrap();
		let raw = "{ ... }: {\n\t# keep ./nixbox-flakes.nix\n\tx = \"../nixbox-flakes.nix\";\n\timports = [ ../nixbox-flakes.nix ../nixbox-flakes.nix.bak ] ++ [ ../nixbox-packages.nix ];\n\tother.imports = [ ../nixbox-flakes.nix ];\n}";
		fs::write(&main, raw).unwrap();
		migrate_flake_file(&packages, &legacy, std::slice::from_ref(&fixture.0), true).unwrap();
		let updated = fs::read_to_string(&main).unwrap();
		assert!(updated.contains("# keep ./nixbox-flakes.nix"));
		assert!(updated.contains("x = \"../nixbox-flakes.nix\";"));
		assert!(
			updated
				.contains("imports = [  ../nixbox-flakes.nix.bak ] ++ [ ../nixbox-packages.nix ];")
		);
		assert!(updated.contains("other.imports = [ ../nixbox-packages.nix ];"));
	}

	#[test]
	fn relative_configuration_paths_work_before_the_package_file_exists() {
		let fixture = Fixture::new("relative-config");
		let legacy = fixture.0.join("nixbox-flakes.nix");
		let main = fixture.0.join("main.nix");
		fs::write(&legacy, render_legacy_flakes(&outputs(), "home.packages")).unwrap();
		fs::write(
			&main,
			"{ ... }: { imports = [ ./nixbox-flakes.nix ./nixbox-packages.nix ]; }",
		)
		.unwrap();
		let relative = PathBuf::from(pathdiff_relative(
			&legacy,
			&std::env::current_dir().unwrap(),
		))
		.with_file_name("nixbox-packages.nix");
		migrate_flake_file(&relative, &legacy, std::slice::from_ref(&fixture.0), true).unwrap();
		assert_eq!(
			fs::read_to_string(&main).unwrap(),
			"{ ... }: { imports = [  ./nixbox-packages.nix ]; }"
		);
		assert!(relative.exists());
	}

	#[test]
	fn updates_an_overridden_entry_file_outside_the_configuration_root() {
		let fixture = Fixture::new("override");
		let root = fixture.0.join("config");
		fs::create_dir_all(&root).unwrap();
		let packages = root.join("nixbox-packages.nix");
		let legacy = root.join("nixbox-flakes.nix");
		let main = fixture.0.join("external.nix");
		fs::write(&legacy, render_legacy_flakes(&outputs(), "home.packages")).unwrap();
		fs::write(
			&main,
			format!("{{ ... }}: {{ imports = [ {} ]; }}", legacy.display()),
		)
		.unwrap();
		migrate_flake_file(&packages, &legacy, &[root, main.clone()], true).unwrap();
		assert_eq!(
			fs::read_to_string(&main).unwrap(),
			"{ ... }: { imports = [ ./config/nixbox-packages.nix ]; }"
		);
	}

	#[cfg(unix)]
	#[test]
	fn an_import_write_failure_restores_the_package_file_and_keeps_the_legacy_file() {
		use std::os::unix::fs::PermissionsExt;
		let fixture = Fixture::new("rollback");
		let packages = fixture.0.join("nixbox-packages.nix");
		let legacy = fixture.0.join("nixbox-flakes.nix");
		let locked = fixture.0.join("locked");
		fs::create_dir_all(&locked).unwrap();
		let main = locked.join("main.nix");
		let original = render(&Manifest::default(), &FlakeManifest::default(), true);
		let old = render_legacy_flakes(&outputs(), "home.packages");
		let entry = "{ ... }: { imports = [ ../nixbox-flakes.nix ]; }";
		fs::write(&packages, &original).unwrap();
		fs::write(&legacy, &old).unwrap();
		fs::write(&main, entry).unwrap();
		fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
		// Root ignores directory permissions; keep the test meaningful when
		// a downstream builder happens to run the test suite as root.
		if fs::write(locked.join("probe"), "").is_ok() {
			fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
			return;
		}
		let result = migrate_flake_file(&packages, &legacy, std::slice::from_ref(&fixture.0), true);
		fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
		assert!(result.is_err());
		assert_eq!(fs::read_to_string(&packages).unwrap(), original);
		assert_eq!(fs::read_to_string(&legacy).unwrap(), old);
		assert_eq!(fs::read_to_string(&main).unwrap(), entry);
	}

	#[test]
	fn unknown_generated_content_or_unsupported_imports_leave_all_files_untouched() {
		let fixture = Fixture::new("refuse");
		let packages = fixture.0.join("nixbox-packages.nix");
		let legacy = fixture.0.join("nixbox-flakes.nix");
		let main = fixture.0.join("main.nix");
		let package_source = render(&Manifest::default(), &FlakeManifest::default(), true);
		let legacy_source = render_legacy_flakes(&outputs(), "home.packages");
		for (package_source, legacy_source, main_source) in [
			(
				package_source.clone(),
				legacy_source.replace("\n}", "\n custom = true;\n}"),
				"{ ... }: { imports = [ ./nixbox-flakes.nix ]; }",
			),
			(
				package_source.replace("\n}", "\n custom = true;\n}"),
				legacy_source.clone(),
				"{ ... }: { imports = [ ./nixbox-flakes.nix ]; }",
			),
			(
				package_source.clone(),
				legacy_source.replace("# nixbox:flake-packages:start", "# missing marker"),
				"{ ... }: { imports = [ ./nixbox-flakes.nix ]; }",
			),
			(
				package_source.clone(),
				legacy_source.replace("# nixbox:flake-packages:end", "# missing marker"),
				"{ ... }: { imports = [ ./nixbox-flakes.nix ]; }",
			),
			(
				package_source.clone(),
				legacy_source.clone(),
				"{ ... }: { imports = [ ./some-${\"directory\"}/nixbox-flakes.nix ]; }",
			),
			(
				package_source.clone(),
				legacy_source.clone(),
				"{ ... }: { imports = [ \"/somewhere/nixbox-flakes.nix\" ]; }",
			),
			(
				package_source,
				legacy_source,
				"{ ... }: import ./nixbox-flakes.nix",
			),
		] {
			fs::write(&packages, &package_source).unwrap();
			fs::write(&legacy, &legacy_source).unwrap();
			fs::write(&main, main_source).unwrap();
			assert!(
				migrate_flake_file(&packages, &legacy, std::slice::from_ref(&fixture.0), true)
					.is_err()
			);
			assert_eq!(fs::read_to_string(&packages).unwrap(), package_source);
			assert_eq!(fs::read_to_string(&legacy).unwrap(), legacy_source);
			assert_eq!(fs::read_to_string(&main).unwrap(), main_source);
		}
	}
}
