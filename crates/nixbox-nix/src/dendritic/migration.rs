use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct SourceEdit {
	pub path: PathBuf,
	pub before: String,
	pub after: String,
}

impl SourceEdit {
	pub fn apply_all(edits: &[Self]) -> Result<()> {
		for edit in edits {
			ensure!(
				fs::read_to_string(&edit.path)? == edit.before,
				"{} changed after migration was planned",
				edit.path.display()
			);
			parsed(&edit.after).with_context(|| format!("validating {}", edit.path.display()))?;
		}
		for (index, edit) in edits.iter().enumerate() {
			if let Err(error) = fs::write(&edit.path, &edit.after) {
				for previous in &edits[..index] {
					fs::write(&previous.path, &previous.before)?;
				}
				return Err(error.into());
			}
		}
		Ok(())
	}
}

impl Layout {
	pub fn source_files(&self) -> &[PathBuf] {
		&self.files
	}

	pub(super) fn shared_sources(&self, home: bool) -> Result<BTreeSet<(String, usize)>> {
		let discovery = Self::inspect(&self.root)?.context("layout disappeared")?;
		let mut others = Vec::new();
		for host in &discovery.hosts {
			if home {
				for user in &host.users {
					if !self.standalone
						&& host.output == self.host
						&& Some(user) == self.home_user.as_ref()
					{
						continue;
					}
					others.push(
						Self::select(&self.root, Some(&host.output), Some(user))?
							.context("host disappeared")?,
					);
				}
			} else if self.standalone || host.output != self.host {
				others.push(
					Self::select(&self.root, Some(&host.output), None)?
						.context("host disappeared")?,
				);
			}
		}
		if home {
			for output in &discovery.homes {
				if self.standalone && output.output == self.host {
					continue;
				}
				others.push(
					Self::select_output(&self.root, None, None, Some(&output.output))?
						.context("home disappeared")?,
				);
			}
		}
		let mut shared = BTreeSet::new();
		for layout in others {
			for (file, offset, _) in layout.connected_sources(home)? {
				shared.insert((file, offset));
			}
		}
		Ok(shared)
	}

	pub fn plan_remove_packages(
		&self,
		home: bool,
		names: &[String],
	) -> Result<(Vec<SourceEdit>, Vec<String>)> {
		self.plan_remove(home, |source, target| {
			crate::scan::parse(source, target)
				.into_iter()
				.filter(|entry| names.contains(&entry.name))
				.map(|entry| (entry.line, entry.name, entry.migratable))
				.collect()
		})
	}

	pub fn plan_remove_flake_package(
		&self,
		home: bool,
		input: &str,
		package: &str,
	) -> Result<Vec<SourceEdit>> {
		let (edits, _) = self.plan_remove(home, |source, target| {
			crate::scan::parse_flake_packages(source, target)
				.into_iter()
				.filter(|entry| entry.input == input && entry.package == package)
				.map(|entry| (entry.line, entry.package, entry.removable))
				.collect()
		})?;
		Ok(edits)
	}

	fn plan_remove(
		&self,
		home: bool,
		entries: impl Fn(&str, crate::scan::ScanTarget) -> Vec<(usize, String, bool)>,
	) -> Result<(Vec<SourceEdit>, Vec<String>)> {
		let target = if home {
			crate::scan::ScanTarget::HomeManager
		} else {
			crate::scan::ScanTarget::Nixos
		};
		let shared = self.shared_sources(home)?;
		let mut removals: BTreeMap<PathBuf, BTreeSet<usize>> = BTreeMap::new();
		let mut removed = BTreeSet::new();
		for (file, offset, source) in self.connected_sources(home)? {
			for (line, name, standalone_line) in entries(&source, target) {
				ensure!(
					standalone_line,
					"{name} is inline or computed in {file}; put it on its own package-list line before migration"
				);
				ensure!(
					!shared.contains(&(file.clone(), offset)),
					"{name} is declared in a module shared with another configuration; split that declaration before migration"
				);
				removals
					.entry(self.root.join(&file))
					.or_default()
					.insert(line + offset);
				removed.insert(name);
			}
		}
		let mut edits = Vec::new();
		for (path, lines) in removals {
			let before = fs::read_to_string(&path)?;
			let mut after = before
				.split_inclusive('\n')
				.enumerate()
				.filter(|(line, _)| !lines.contains(line))
				.map(|(_, text)| text)
				.collect::<String>();
			if !before.ends_with('\n') && after.ends_with('\n') {
				after.pop();
			}
			parsed(&after)?;
			edits.push(SourceEdit {
				path,
				before,
				after,
			});
		}
		Ok((edits, removed.into_iter().collect()))
	}
}
