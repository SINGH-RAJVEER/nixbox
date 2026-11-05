use crate::{Engine, Op, Reporter};
use anyhow::{Result, ensure};
use nixbox_config::Target;
use nixbox_nix::{
	FlakeOutput,
	dendritic::{Layout, SourceEdit},
};

/// Applies the reviewed file transaction while holding the configuration's
/// writer lock, then makes new modules visible to Git-backed Nix flakes.
pub fn apply_managed_migration(
	config: &nixbox_config::Config,
	changes: &[nixbox_nix::dendritic::FileChange],
) -> Result<()> {
	ensure!(
		config.dendritic_detection,
		"enable dendritic detection before migrating dendritic modules"
	);
	let _guard = crate::vcs::AppliedJournal::acquire(config)?;
	nixbox_nix::dendritic::FileChange::apply_all(changes)?;
	for change in changes.iter().filter(|change| change.after.is_some()) {
		crate::engine::git_track(&change.path, &mut crate::SilentReporter);
	}
	Ok(())
}

impl Engine {
	pub(crate) fn dendritic_source_edits(
		&self,
		layout: &Layout,
		op: &Op,
	) -> Result<Vec<SourceEdit>> {
		let home = op.scope() == Target::HomeManager;
		Ok(match op {
			Op::Migrate { names, .. } => layout.plan_remove_packages(home, names)?.0,
			Op::MigrateAcross { names, from, .. } => {
				layout
					.plan_remove_packages(*from == Target::HomeManager, names)?
					.0
			}
			Op::Uninstall { name, .. } => {
				layout
					.plan_remove_packages(home, std::slice::from_ref(name))?
					.0
			}
			Op::MigrateFlakePackage { input, package, .. }
			| Op::UninstallFlakeOutput {
				input,
				output: FlakeOutput::Package(package),
				..
			} => layout.plan_remove_flake_package(home, input, package)?,
			Op::UninstallFlake { repo, .. } => {
				let mut edits = Vec::new();
				for entry in &self.flakes {
					if entry.scope == op.scope()
						&& entry.repo.as_deref() == Some(repo)
						&& let FlakeOutput::Package(package) = &entry.output
					{
						edits.extend(layout.plan_remove_flake_package(
							home,
							&entry.input,
							package,
						)?);
					}
				}
				edits
			}
			_ => Vec::new(),
		})
	}
	pub(crate) fn apply_dendritic_sources(
		&mut self,
		layout: &Layout,
		op: &Op,
		reporter: &mut dyn Reporter,
	) -> Result<bool> {
		let scope = op.scope();
		if let Op::MigrateFlakePackage { input, package, .. } = op {
			let edits =
				layout.plan_remove_flake_package(scope == Target::HomeManager, input, package)?;
			ensure!(
				!edits.is_empty(),
				"{input}#{package} has no exclusively owned, removable source declaration"
			);
			let repo = nixbox_nix::wiring::input_repos(&self.config.flake_file())
				.remove(input)
				.ok_or_else(|| anyhow::anyhow!("input {input} is not a GitHub flake"))?;
			self.flakes.retain(|entry| {
				entry.is_managed()
					|| entry.scope != scope
					|| entry.input != *input
					|| entry.output != FlakeOutput::Package(package.clone())
			});
			self.apply_flake_output(
				&repo,
				&FlakeOutput::Package(package.clone()),
				scope,
				reporter,
			)?;
			SourceEdit::apply_all(&edits)?;
			self.refresh_externals();
			return Ok(true);
		}
		let (names, from, remove) = match op {
			Op::Migrate { names, .. } => (names.clone(), scope, false),
			Op::MigrateAcross { names, from, .. } => (names.clone(), *from, false),
			Op::Uninstall { name, .. } => (vec![name.clone()], scope, true),
			_ => return Ok(false),
		};
		ensure!(
			self.has_main_file(scope),
			"the destination scope is not configured"
		);
		let (edits, moved) = layout.plan_remove_packages(from == Target::HomeManager, &names)?;
		if !remove {
			ensure!(
				names.iter().all(|name| moved.contains(name)),
				"some requested packages have no removable source declaration"
			);
		}
		for name in if remove { &names } else { &moved } {
			if remove {
				self.manifest_for_mut(scope).remove(name);
			} else {
				self.manifest_for_mut(scope).add(name);
			}
		}
		self.write_manifest(scope)?;
		SourceEdit::apply_all(&edits)?;
		reporter.info(format!(
			"{} {} package declaration(s) for the selected configuration.",
			if remove { "Removed" } else { "Migrated" },
			moved.len()
		));
		self.refresh_externals();
		Ok(true)
	}
}
