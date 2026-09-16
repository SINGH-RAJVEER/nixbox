//! Applies [`Op`]s to the user's configuration.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Result;
use nixbox_config::{Config, Target};
use nixbox_nix::{
    Manifest,
    flakes::{ensure_flake_input, remove_flake_input},
    manifest::{FlakeManifest, ImportStatus, ManagedFile, ManagedFlakeFile, ensure_imported},
    scan::{ExternalPackage, ScanTarget, remove_from_source, scan},
};

use crate::op::Op;
use crate::report::Reporter;

/// A package tracked by nixbox's manifest, tagged with the scope it lives in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedPackage {
    pub name: String,
    pub scope: Target,
}

/// The scan scope matching a rebuild target.
#[must_use]
pub fn scan_target(scope: Target) -> ScanTarget {
    match scope {
        Target::HomeManager => ScanTarget::HomeManager,
        Target::NixosSystem => ScanTarget::Nixos,
    }
}

/// Whether a scanned package belongs to `target`'s scope.
#[must_use]
pub fn scope_matches(scan_scope: ScanTarget, target: Target) -> bool {
    matches!(
        (scan_scope, target),
        (ScanTarget::HomeManager, Target::HomeManager) | (ScanTarget::Nixos, Target::NixosSystem)
    )
}

/// Holds the user's settings plus both manifests, and owns every mutation of
/// the files nixbox generates.
///
/// `external_packages` is maintained as "declared in a main config file and
/// *not* already tracked in the matching manifest" — [`Engine::apply`] keeps
/// that invariant as packages migrate in.
pub struct Engine {
    pub config: Config,
    /// Packages tracked in `nixbox-home-packages.nix`.
    pub home_manifest: Manifest,
    /// Packages tracked in `nixbox-system-packages.nix`.
    pub nixos_manifest: Manifest,
    /// Packages declared directly in the user's own config files.
    pub external_packages: Vec<ExternalPackage>,
}

impl Engine {
    /// Loads settings from disk, then both manifests, then scans for
    /// externally-declared packages.
    pub fn load() -> Result<Self> {
        Self::with_config(Config::load_or_default()?)
    }

    /// Same as [`Engine::load`] but with settings supplied by the caller —
    /// used when a front-end overrides the target or channel per invocation.
    pub fn with_config(config: Config) -> Result<Self> {
        let home_manifest =
            ManagedFile::new(config.managed_file_for(Target::HomeManager)).load()?;
        let nixos_manifest =
            ManagedFile::new(config.managed_file_for(Target::NixosSystem)).load()?;
        let external_packages = scan_externals(&config, &home_manifest, &nixos_manifest);
        Ok(Self {
            config,
            home_manifest,
            nixos_manifest,
            external_packages,
        })
    }

    /// Builds an engine from parts, without touching the filesystem.
    #[must_use]
    pub fn from_parts(
        config: Config,
        home_manifest: Manifest,
        nixos_manifest: Manifest,
        external_packages: Vec<ExternalPackage>,
    ) -> Self {
        Self {
            config,
            home_manifest,
            nixos_manifest,
            external_packages,
        }
    }

    #[must_use]
    pub fn manifest_for(&self, scope: Target) -> &Manifest {
        match scope {
            Target::HomeManager => &self.home_manifest,
            Target::NixosSystem => &self.nixos_manifest,
        }
    }

    pub fn manifest_for_mut(&mut self, scope: Target) -> &mut Manifest {
        match scope {
            Target::HomeManager => &mut self.home_manifest,
            Target::NixosSystem => &mut self.nixos_manifest,
        }
    }

    /// Every managed package from both scopes, home-manager entries first.
    #[must_use]
    pub fn managed_packages(&self) -> Vec<ManagedPackage> {
        let mut out: Vec<ManagedPackage> = Vec::new();
        for name in &self.home_manifest.packages {
            out.push(ManagedPackage {
                name: name.clone(),
                scope: Target::HomeManager,
            });
        }
        for name in &self.nixos_manifest.packages {
            out.push(ManagedPackage {
                name: name.clone(),
                scope: Target::NixosSystem,
            });
        }
        out
    }

    /// True if `name` is already tracked in `scope`'s manifest.
    #[must_use]
    pub fn is_tracked(&self, name: &str, scope: Target) -> bool {
        self.manifest_for(scope).packages.contains(name)
    }

    /// Reports whether `scope`'s managed file is imported, without writing
    /// anything — [`Engine::apply`] is what repairs a missing import.
    #[must_use]
    pub fn import_state(&self, scope: Target) -> ImportState {
        let main_file = self.config.main_file_for(scope);
        let managed_file = self.config.managed_file_for(scope);
        let Some(name) = managed_file.file_name().and_then(|n| n.to_str()) else {
            return ImportState::NotImported;
        };
        let Ok(source) = std::fs::read_to_string(&main_file) else {
            return ImportState::MainFileMissing;
        };
        if source.contains(name) {
            ImportState::Imported
        } else {
            ImportState::NotImported
        }
    }

    /// Re-reads both main config files and refreshes `external_packages`.
    pub fn refresh_externals(&mut self) {
        self.external_packages =
            scan_externals(&self.config, &self.home_manifest, &self.nixos_manifest);
    }

    /// Writes `scope`'s manifest to its managed file in the right format.
    pub fn write_manifest(&self, scope: Target) -> Result<ManagedFile> {
        let managed = ManagedFile::new(self.config.managed_file_for(scope));
        match scope {
            Target::HomeManager => managed.write_home_manager(self.manifest_for(scope))?,
            Target::NixosSystem => managed.write_nixos(self.manifest_for(scope))?,
        }
        Ok(managed)
    }

    /// Applies one op: updates the in-memory manifest, writes every file the
    /// op touches, makes sure the managed file is imported, and keeps git
    /// aware of what was generated.
    ///
    /// This does *not* rebuild. Writing first is deliberate — if the rebuild
    /// is interrupted the on-disk state already reflects the intent, so
    /// recovery is just re-running the rebuild.
    pub fn apply(&mut self, op: &Op, reporter: &mut dyn Reporter) -> Result<()> {
        let scope = op.scope();
        match op {
            Op::Install { hit, .. } => {
                self.manifest_for_mut(scope).add(&hit.attr);
            }
            Op::InstallFlake { repo, module, .. } => {
                return self.apply_flake_output(repo, scope, reporter, |manifest| {
                    manifest.add(repo.clone(), module.clone());
                });
            }
            Op::InstallFlakePackage { repo, package, .. } => {
                return self.apply_flake_output(repo, scope, reporter, |manifest| {
                    manifest.add_package(repo.clone(), package.clone());
                });
            }
            Op::UninstallFlake { repo, .. } => {
                return self.remove_flake(repo, scope, reporter);
            }
            Op::Uninstall { name, .. } => {
                self.manifest_for_mut(scope).remove(name);
            }
            Op::Migrate { names, .. } => {
                let source = self.config.main_file_for(scope);
                let removed = remove_from_source(&source, scan_target(scope), names)?;
                for name in &removed {
                    self.manifest_for_mut(scope).add(name);
                }
                // Drop any externals that just moved into the manifest.
                self.external_packages
                    .retain(|ep| !(scope_matches(ep.scope, scope) && removed.contains(&ep.name)));
            }
        }

        let managed = self.write_manifest(scope)?;
        reporter.info(format!(
            "Wrote {} ({}). {}...",
            managed.path().display(),
            scope.label(),
            op.label(),
        ));
        let main_file = self.config.main_file_for(scope);
        note_import(&main_file, managed.path(), reporter);
        // Flakes ignore untracked files — make sure git sees the managed file
        // (and the main config if we just touched it).
        git_track(managed.path(), reporter);
        if main_file.exists() {
            git_track(&main_file, reporter);
        }
        Ok(())
    }

    /// Adds `repo` as a flake input and wires one of its outputs into the
    /// managed flake file. The caller decides whether that output is a module
    /// or a package, since the file holds both.
    fn apply_flake_output(
        &mut self,
        repo: &str,
        scope: Target,
        reporter: &mut dyn Reporter,
        update: impl FnOnce(&mut FlakeManifest),
    ) -> Result<()> {
        let (special_args, constructor) = match scope {
            Target::HomeManager => ("extraSpecialArgs", "homeManagerConfiguration"),
            Target::NixosSystem => ("specialArgs", "nixosSystem"),
        };
        let flake_file = self.config.flake_file();
        ensure_flake_input(&flake_file, repo, special_args, constructor)?;

        let managed = ManagedFlakeFile::new(self.config.flake_manifest_for(scope));
        let mut manifest = managed.load()?;
        update(&mut manifest);
        match scope {
            Target::HomeManager => managed.write_home_manager(&manifest)?,
            Target::NixosSystem => managed.write_nixos(&manifest)?,
        }
        reporter.info(format!(
            "Added github:{repo} to {} and wired its selected output into {}.",
            flake_file.display(),
            scope.label(),
        ));

        let main_file = self.config.main_file_for(scope);
        note_import(&main_file, managed.path(), reporter);
        for path in [flake_file.as_path(), managed.path(), main_file.as_path()] {
            if path.exists() {
                git_track(path, reporter);
            }
        }
        Ok(())
    }
}

/// Whether a target's managed file is wired into the user's own config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportState {
    /// The main config file imports the managed file.
    Imported,
    /// The main config file exists but does not import the managed file.
    NotImported,
    /// There is no main config file to import from.
    MainFileMissing,
}

impl Engine {
    /// Drops a flake module from the generated manifest and removes its input
    /// from the root flake.
    fn remove_flake(
        &mut self,
        repo: &str,
        scope: Target,
        reporter: &mut dyn Reporter,
    ) -> Result<()> {
        let managed = ManagedFlakeFile::new(self.config.flake_manifest_for(scope));
        let mut manifest = managed.load()?;
        if !manifest.remove(repo) {
            reporter.warn(format!(
                "{repo} is not one of the flakes nixbox manages for {}.",
                scope.label()
            ));
            return Ok(());
        }
        match scope {
            Target::HomeManager => managed.write_home_manager(&manifest)?,
            Target::NixosSystem => managed.write_nixos(&manifest)?,
        }

        let flake_file = self.config.flake_file();
        if remove_flake_input(&flake_file, repo)? {
            reporter.info(format!(
                "Removed github:{} from {}.",
                repo,
                flake_file.display()
            ));
        } else {
            reporter.warn(format!(
                "no input for {repo} found in {}; remove it by hand if it is still there",
                flake_file.display()
            ));
        }
        git_track(managed.path(), reporter);
        if flake_file.exists() {
            git_track(&flake_file, reporter);
        }
        Ok(())
    }

    /// The flake modules nixbox manages for `scope`, as (input, module) pairs.
    pub fn managed_flakes(&self, scope: Target) -> Result<Vec<(String, String)>> {
        let managed = ManagedFlakeFile::new(self.config.flake_manifest_for(scope));
        Ok(managed.load()?.modules.into_iter().collect())
    }
}

/// Scans both main config files and returns the packages declared in them,
/// scope-tagged, excluding anything already in the matching manifest.
///
/// The same name can appear twice when it is declared in both scopes — that
/// is intentional, they are two separate declarations.
#[must_use]
pub fn scan_externals(
    config: &Config,
    home_manifest: &Manifest,
    nixos_manifest: &Manifest,
) -> Vec<ExternalPackage> {
    let mut out: Vec<ExternalPackage> = Vec::new();

    if let Ok(found) = scan(
        &config.main_file_for(Target::HomeManager),
        ScanTarget::HomeManager,
    ) {
        out.extend(
            found
                .into_iter()
                .filter(|ep| !home_manifest.packages.contains(&ep.name)),
        );
    }
    if let Ok(found) = scan(
        &config.main_file_for(Target::NixosSystem),
        ScanTarget::Nixos,
    ) {
        out.extend(
            found
                .into_iter()
                .filter(|ep| !nixos_manifest.packages.contains(&ep.name)),
        );
    }

    out
}

/// Reports whatever `ensure_imported` had to change, and stays quiet when the
/// managed file was already imported.
fn note_import(main_file: &Path, managed_file: &Path, reporter: &mut dyn Reporter) {
    match ensure_imported(main_file, managed_file) {
        Ok(ImportStatus::AlreadyImported) => {}
        Ok(ImportStatus::InsertedIntoList) => reporter.info(format!(
            "Added import of {} to {}.",
            managed_file.display(),
            main_file.display(),
        )),
        Ok(ImportStatus::CreatedList) => reporter.info(format!(
            "Created imports list in {} and added {}.",
            main_file.display(),
            managed_file.display(),
        )),
        Ok(ImportStatus::MainFileMissing) => reporter.warn(format!(
            "{} not found; you must import {} manually.",
            main_file.display(),
            managed_file.display(),
        )),
        Err(e) => reporter.warn(format!(
            "could not auto-import {} into {}: {}",
            managed_file.display(),
            main_file.display(),
            e,
        )),
    }
}

/// Runs `git add --intent-to-add` if `path` lives inside a git work tree.
/// Silent no-op when git isn't installed or the path isn't inside one. Nix
/// flakes refuse to see files that are present on disk but untracked, so this
/// keeps a freshly written managed file visible to the rebuild.
fn git_track(path: &Path, reporter: &mut dyn Reporter) {
    let Some(parent) = path.parent() else {
        return;
    };
    let inside = Command::new("git")
        .arg("-C")
        .arg(parent)
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let Ok(inside) = inside else {
        return;
    };
    if !inside.status.success() || std::str::from_utf8(&inside.stdout).map(str::trim) != Ok("true")
    {
        return;
    }

    let added = Command::new("git")
        .arg("-C")
        .arg(parent)
        .args(["add", "--intent-to-add", "--"])
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    let Ok(added) = added else {
        return;
    };
    if added.status.success() {
        reporter.info(format!("git add -N {}", path.display()));
    } else {
        reporter.warn(format!(
            "`git add` failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&added.stderr).trim()
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::{Engine, ManagedPackage, scan_target, scope_matches};
    use crate::op::Op;
    use crate::report::{LogReporter, SilentReporter};
    use nixbox_config::{Config, Target};
    use nixbox_nix::{Manifest, scan::ScanTarget};

    fn engine() -> Engine {
        Engine::from_parts(
            Config::default(),
            Manifest::default(),
            Manifest::default(),
            Vec::new(),
        )
    }

    #[test]
    fn scan_targets_and_scopes_line_up() {
        assert_eq!(scan_target(Target::HomeManager), ScanTarget::HomeManager);
        assert_eq!(scan_target(Target::NixosSystem), ScanTarget::Nixos);
        assert!(scope_matches(ScanTarget::Nixos, Target::NixosSystem));
        assert!(!scope_matches(ScanTarget::Nixos, Target::HomeManager));
    }

    #[test]
    fn managed_packages_lists_home_entries_before_system_entries() {
        let mut engine = engine();
        engine.manifest_for_mut(Target::NixosSystem).add("fd");
        engine.manifest_for_mut(Target::HomeManager).add("ripgrep");

        assert_eq!(
            engine.managed_packages(),
            vec![
                ManagedPackage {
                    name: "ripgrep".into(),
                    scope: Target::HomeManager,
                },
                ManagedPackage {
                    name: "fd".into(),
                    scope: Target::NixosSystem,
                },
            ]
        );
    }

    #[test]
    fn tracking_is_scoped() {
        let mut engine = engine();
        engine.manifest_for_mut(Target::HomeManager).add("ripgrep");

        assert!(engine.is_tracked("ripgrep", Target::HomeManager));
        assert!(!engine.is_tracked("ripgrep", Target::NixosSystem));
    }

    #[test]
    fn applying_an_install_writes_the_managed_file_and_reports_it() {
        let dir = crate::tests::temp_dir("apply-install");
        let mut engine = Engine::from_parts(
            dir.config(),
            Manifest::default(),
            Manifest::default(),
            Vec::new(),
        );
        let mut reporter = LogReporter::new();

        engine
            .apply(
                &Op::Install {
                    hit: crate::tests::hit("ripgrep"),
                    scope: Target::NixosSystem,
                },
                &mut reporter,
            )
            .expect("apply install");

        assert!(engine.is_tracked("ripgrep", Target::NixosSystem));
        let written = std::fs::read_to_string(engine.config.managed_file_for(Target::NixosSystem))
            .expect("managed file written");
        assert!(written.contains("ripgrep"));
        assert!(
            reporter
                .lines()
                .iter()
                .any(|line| line.contains("install ripgrep [nixos]"))
        );
        drop(dir);
    }

    #[test]
    fn uninstall_drops_the_package_from_the_managed_file() {
        let dir = crate::tests::temp_dir("apply-uninstall");
        let mut engine = Engine::from_parts(
            dir.config(),
            Manifest::default(),
            Manifest::default(),
            Vec::new(),
        );
        engine.manifest_for_mut(Target::NixosSystem).add("ripgrep");
        engine.manifest_for_mut(Target::NixosSystem).add("fd");

        engine
            .apply(
                &Op::Uninstall {
                    name: "fd".into(),
                    scope: Target::NixosSystem,
                },
                &mut SilentReporter,
            )
            .expect("apply uninstall");

        let written = std::fs::read_to_string(engine.config.managed_file_for(Target::NixosSystem))
            .expect("managed file written");
        assert!(written.contains("ripgrep"));
        assert!(!written.contains("fd"));
        drop(dir);
    }
}

#[cfg(test)]
mod import_state_tests {
    use super::{Engine, ImportState};
    use nixbox_config::Target;
    use nixbox_nix::Manifest;

    fn engine(dir: &crate::tests::TempConfigDir) -> Engine {
        Engine::from_parts(
            dir.config(),
            Manifest::default(),
            Manifest::default(),
            Vec::new(),
        )
    }

    #[test]
    fn a_missing_main_file_is_reported_as_such() {
        let dir = crate::tests::temp_dir("import-missing");
        let engine = engine(&dir);

        assert_eq!(
            engine.import_state(Target::NixosSystem),
            ImportState::MainFileMissing
        );
        drop(dir);
    }

    #[test]
    fn an_import_line_is_detected_and_its_absence_is_not() {
        let dir = crate::tests::temp_dir("import-present");
        let engine = engine(&dir);
        let main_file = engine.config.main_file_for(Target::NixosSystem);
        std::fs::write(&main_file, "{ imports = [ ./unrelated.nix ]; }\n")
            .expect("write main file");

        assert_eq!(
            engine.import_state(Target::NixosSystem),
            ImportState::NotImported
        );

        std::fs::write(
            &main_file,
            "{ imports = [ ./nixbox-system-packages.nix ]; }\n",
        )
        .expect("write main file");

        assert_eq!(
            engine.import_state(Target::NixosSystem),
            ImportState::Imported
        );
        drop(dir);
    }
}
