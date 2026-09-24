//! Flake outputs in the configuration: wiring them in, listing them, and
//! taking them out again.

use std::path::PathBuf;

use anyhow::{Result, bail};
use nixbox_config::{Config, Target};
use nixbox_nix::{
    manifest::{FlakeOutput, ManagedFlakeFile},
    scan::{remove_flake_package_from_source, scan_flake_packages},
    wiring::{
        ensure_flake_input, ensure_home_manager_special_args, ensure_inputs_passed,
        input_referenced, input_repos, pass_inputs_through, remove_flake_input,
    },
};

use crate::engine::{Engine, git_track, note_import, scan_target};
use crate::report::Reporter;

/// One flake output in the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledFlake {
    /// The root flake input, as written in `flake.nix`.
    pub input: String,
    /// `owner/repo` when the input points at GitHub.
    pub repo: Option<String>,
    pub output: FlakeOutput,
    pub scope: Target,
    /// The list in the user's own config that declares this output, such as
    /// `home.packages`, or `None` when it lives in nixbox's generated flake
    /// module.
    pub declared_in: Option<String>,
    /// Whether nixbox can take the output out again. Hand-written entries
    /// sharing a line with other entries cannot be removed cleanly.
    pub removable: bool,
}

impl InstalledFlake {
    /// `input#package` or `input.<module path>`.
    #[must_use]
    pub fn name(&self) -> String {
        self.output.display(&self.input)
    }

    #[must_use]
    pub fn is_managed(&self) -> bool {
        self.declared_in.is_none()
    }
}

/// Every flake output nixbox wired in, followed by the flake packages the
/// user declared by hand in each target's main file. A hand-written package
/// that nixbox also lists is shown once, as the managed entry; removing it
/// removes both.
#[must_use]
pub fn scan_flakes(config: &Config) -> Vec<InstalledFlake> {
    let repos = input_repos(&config.flake_file());
    let mut out = Vec::new();
    for scope in [Target::HomeManager, Target::NixosSystem] {
        let managed = ManagedFlakeFile::new(config.flake_manifest_for(scope))
            .load()
            .unwrap_or_default();
        let first = out.len();
        for (repo, output) in managed.outputs() {
            out.push(InstalledFlake {
                input: managed.input_for(&repo),
                repo: Some(repo),
                output,
                scope,
                declared_in: None,
                removable: true,
            });
        }
        let Ok(found) = scan_flake_packages(&config.main_file_for(scope), scan_target(scope))
        else {
            continue;
        };
        for entry in found {
            let output = FlakeOutput::Package(entry.package);
            if out[first..]
                .iter()
                .any(|flake| flake.input == entry.input && flake.output == output)
            {
                continue;
            }
            out.push(InstalledFlake {
                repo: repos.get(&entry.input).cloned(),
                input: entry.input,
                output,
                scope,
                declared_in: Some(entry.source_attr),
                removable: entry.removable,
            });
        }
    }
    out
}

impl Engine {
    /// Adds `repo` as a flake input and wires `output` into the managed flake
    /// file, unless the user already declares that package by hand.
    pub(crate) fn apply_flake_output(
        &mut self,
        repo: &str,
        output: &FlakeOutput,
        scope: Target,
        reporter: &mut dyn Reporter,
    ) -> Result<()> {
        let flake_file = self.config.flake_file();
        let input = ensure_flake_input(&flake_file, repo)?;
        let main_file = self.config.main_file_for(scope);

        if let FlakeOutput::Package(package) = output
            && scan_flake_packages(&main_file, scan_target(scope))?
                .iter()
                .any(|entry| entry.input == input && &entry.package == package)
        {
            reporter.info(format!(
                "{} is already declared in {}; nothing added.",
                output.display(&input),
                main_file.display()
            ));
            self.refresh_externals();
            return Ok(());
        }

        let wired = self.pass_inputs(scope)?;
        let managed = ManagedFlakeFile::new(self.config.flake_manifest_for(scope));
        let mut manifest = managed.load()?;
        manifest.inputs.insert(repo.to_string(), input.clone());
        match output {
            FlakeOutput::Module(module) => manifest.add(repo.to_string(), module.clone()),
            FlakeOutput::Package(package) => {
                manifest.add_package(repo.to_string(), package.clone())
            }
        };
        match scope {
            Target::HomeManager => managed.write_home_manager(&manifest)?,
            Target::NixosSystem => managed.write_nixos(&manifest)?,
        }
        reporter.info(format!(
            "Wired github:{repo} into {} as input `{input}` and added {} to {}.",
            flake_file.display(),
            output.display(&input),
            scope.label(),
        ));

        note_import(&main_file, managed.path(), reporter);
        for path in [flake_file.as_path(), managed.path(), main_file.as_path()]
            .into_iter()
            .chain(wired.as_deref())
        {
            if path.exists() {
                git_track(path, reporter);
            }
        }
        self.refresh_externals();
        Ok(())
    }

    /// Makes the root flake's `inputs` reachable from `scope`'s modules, and
    /// returns the file outside `flake.nix` it had to touch, if any.
    ///
    /// Home Manager is either a standalone `homeManagerConfiguration`, which
    /// takes `extraSpecialArgs` directly, or a NixOS module, where `inputs`
    /// goes through `specialArgs` into the system configuration and on from
    /// there through `home-manager.extraSpecialArgs`.
    fn pass_inputs(&self, scope: Target) -> Result<Option<PathBuf>> {
        let flake_file = self.config.flake_file();
        let source = std::fs::read_to_string(&flake_file)?;
        if scope == Target::NixosSystem {
            ensure_inputs_passed(&flake_file, "nixosSystem", "specialArgs")?;
            return Ok(None);
        }
        if source.contains("homeManagerConfiguration") {
            ensure_inputs_passed(&flake_file, "homeManagerConfiguration", "extraSpecialArgs")?;
            return Ok(None);
        }
        if !source.contains("nixosSystem") {
            bail!(
                "{} has neither a `homeManagerConfiguration` nor a `nixosSystem` to pass `inputs` \
                 to Home Manager through",
                flake_file.display()
            );
        }
        ensure_inputs_passed(&flake_file, "nixosSystem", "specialArgs")?;
        if pass_inputs_through(&flake_file, "extraSpecialArgs")? {
            return Ok(None);
        }
        let system = self.config.main_file_for(Target::NixosSystem);
        ensure_home_manager_special_args(&system)?;
        Ok(Some(system))
    }

    /// Drops every output nixbox manages for `repo`, and its input from the
    /// root flake unless something else still references it.
    pub(crate) fn remove_flake(
        &mut self,
        repo: &str,
        scope: Target,
        reporter: &mut dyn Reporter,
    ) -> Result<()> {
        let managed = ManagedFlakeFile::new(self.config.flake_manifest_for(scope));
        let mut manifest = managed.load()?;
        let input = manifest.input_for(repo);
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
        git_track(managed.path(), reporter);
        self.release_input(&input, reporter)?;
        self.refresh_externals();
        Ok(())
    }

    /// Takes one flake output out of `scope`, wherever it is declared: the
    /// generated flake module, the user's main file, or both. The input goes
    /// too once nothing references it, so a flake that still provides other
    /// packages keeps its input until the last one is removed.
    pub(crate) fn remove_flake_output(
        &mut self,
        input: &str,
        output: &FlakeOutput,
        scope: Target,
        reporter: &mut dyn Reporter,
    ) -> Result<()> {
        let name = output.display(input);
        let managed = ManagedFlakeFile::new(self.config.flake_manifest_for(scope));
        let mut manifest = managed.load()?;
        let repos: Vec<String> = manifest
            .outputs()
            .into_iter()
            .filter(|(repo, found)| found == output && manifest.input_for(repo) == input)
            .map(|(repo, _)| repo)
            .collect();
        let mut removed_managed = false;
        for repo in &repos {
            removed_managed |= manifest.remove_output(repo, output);
        }
        if removed_managed {
            match scope {
                Target::HomeManager => managed.write_home_manager(&manifest)?,
                Target::NixosSystem => managed.write_nixos(&manifest)?,
            }
            reporter.info(format!("Removed {name} from {}.", managed.path().display()));
            git_track(managed.path(), reporter);
        }

        let main_file = self.config.main_file_for(scope);
        let removed_by_hand = match output {
            FlakeOutput::Package(package) => {
                remove_flake_package_from_source(&main_file, scan_target(scope), input, package)?
            }
            FlakeOutput::Module(_) => false,
        };
        if removed_by_hand {
            reporter.info(format!("Removed {name} from {}.", main_file.display()));
            git_track(&main_file, reporter);
        }

        if !removed_managed && !removed_by_hand {
            reporter.warn(format!(
                "{name} is not declared on a line of its own in {} or nixbox's flake module; \
                 remove it by hand.",
                main_file.display()
            ));
            return Ok(());
        }
        self.release_input(input, reporter)?;
        self.refresh_externals();
        Ok(())
    }

    /// Removes the root input `input` when nothing references it any more.
    fn release_input(&self, input: &str, reporter: &mut dyn Reporter) -> Result<()> {
        let flake_file = self.config.flake_file();
        if input_referenced(&self.config.home_manager_dir(), &flake_file, input)? {
            reporter.info(format!(
                "Kept input `{input}` in {}: other parts of your configuration still use it.",
                flake_file.display()
            ));
        } else if remove_flake_input(&flake_file, input)? {
            reporter.info(format!(
                "Removed input `{input}` from {}.",
                flake_file.display()
            ));
            git_track(&flake_file, reporter);
        } else {
            reporter.warn(format!(
                "no input `{input}` found in {}; remove it by hand if it is still there",
                flake_file.display()
            ));
        }
        Ok(())
    }

    /// The flake outputs nixbox manages for `scope`, as `(repo, output)`.
    pub fn managed_flakes(&self, scope: Target) -> Result<Vec<(String, FlakeOutput)>> {
        let managed = ManagedFlakeFile::new(self.config.flake_manifest_for(scope));
        Ok(managed.load()?.outputs())
    }
}

#[cfg(test)]
mod tests {
    use nixbox_config::Target;
    use nixbox_nix::{Manifest, manifest::FlakeOutput};

    use crate::{Engine, Op, SilentReporter};

    const FLAKE: &str = "{\n\tinputs = {\n\t\tnixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\";\n\n\t\tllm-agents.url = \"github:numtide/llm-agents.nix\";\n\t};\n\n\toutputs = { self, nixpkgs, ... }@inputs: {\n\t\tnixosConfigurations.nixos = nixpkgs.lib.nixosSystem {\n\t\t\tspecialArgs = { inherit inputs; };\n\t\t\tmodules = [ ./configuration.nix ];\n\t\t};\n\t};\n}\n";
    const SYSTEM: &str = "{ inputs, ... }:\n{\n\thome-manager = {\n\t\textraSpecialArgs = { inherit inputs; };\n\t\tusers.me = import ./home.nix;\n\t};\n}\n";
    const HOME: &str = "{ inputs, pkgs, ... }:\n{\n\timports = [ ./nixbox-home-packages.nix ];\n\thome.packages = with pkgs; [\n\t\tinputs.llm-agents.packages.${pkgs.stdenv.hostPlatform.system}.chatgpt\n\t\tinputs.llm-agents.packages.${pkgs.stdenv.hostPlatform.system}.claude-code\n\t];\n}\n";

    fn engine(dir: &crate::tests::TempConfigDir) -> Engine {
        let config = dir.config();
        std::fs::write(config.flake_file(), FLAKE).unwrap();
        std::fs::write(config.main_file_for(Target::NixosSystem), SYSTEM).unwrap();
        std::fs::write(config.main_file_for(Target::HomeManager), HOME).unwrap();
        let mut engine =
            Engine::from_parts(config, Manifest::default(), Manifest::default(), Vec::new());
        engine.refresh_externals();
        engine
    }

    fn remove(input: &str, package: &str) -> Op {
        Op::UninstallFlakeOutput {
            input: input.into(),
            output: FlakeOutput::Package(package.into()),
            scope: Target::HomeManager,
        }
    }

    #[test]
    fn lists_hand_written_flake_packages_with_their_repository() {
        let dir = crate::tests::temp_dir("flakes-list");
        let engine = engine(&dir);

        let names: Vec<(String, Option<String>)> = engine
            .flakes
            .iter()
            .map(|flake| (flake.name(), flake.repo.clone()))
            .collect();
        let repo = Some("numtide/llm-agents.nix".to_string());
        assert_eq!(
            names,
            vec![
                ("llm-agents#chatgpt".into(), repo.clone()),
                ("llm-agents#claude-code".into(), repo),
            ]
        );
        assert!(engine.flakes.iter().all(|flake| !flake.is_managed()));
        drop(dir);
    }

    #[test]
    fn a_flake_input_stays_until_its_last_package_is_removed() {
        let dir = crate::tests::temp_dir("flakes-last");
        let mut engine = engine(&dir);
        let flake_file = engine.config.flake_file();

        engine
            .apply(&remove("llm-agents", "chatgpt"), &mut SilentReporter)
            .unwrap();
        let home =
            std::fs::read_to_string(engine.config.main_file_for(Target::HomeManager)).unwrap();
        assert!(!home.contains(".chatgpt"));
        assert!(
            std::fs::read_to_string(&flake_file)
                .unwrap()
                .contains("llm-agents.url")
        );
        assert_eq!(engine.flakes.len(), 1);

        engine
            .apply(&remove("llm-agents", "claude-code"), &mut SilentReporter)
            .unwrap();
        assert!(
            !std::fs::read_to_string(&flake_file)
                .unwrap()
                .contains("llm-agents")
        );
        assert!(engine.flakes.is_empty());
        drop(dir);
    }

    #[test]
    fn installing_a_package_already_declared_by_hand_adds_nothing() {
        let dir = crate::tests::temp_dir("flakes-duplicate");
        let mut engine = engine(&dir);

        engine
            .apply(
                &Op::InstallFlakePackage {
                    repo: "numtide/llm-agents.nix".into(),
                    package: "chatgpt".into(),
                    scope: Target::HomeManager,
                },
                &mut SilentReporter,
            )
            .unwrap();

        assert!(
            !engine
                .config
                .flake_manifest_for(Target::HomeManager)
                .exists()
        );
        assert_eq!(
            std::fs::read_to_string(engine.config.flake_file()).unwrap(),
            FLAKE
        );
        drop(dir);
    }

    #[test]
    fn a_managed_package_round_trips_through_install_and_removal() {
        let dir = crate::tests::temp_dir("flakes-managed");
        let mut engine = engine(&dir);

        engine
            .apply(
                &Op::InstallFlakePackage {
                    repo: "oxcl/nix-flake-helium-browser".into(),
                    package: "default".into(),
                    scope: Target::HomeManager,
                },
                &mut SilentReporter,
            )
            .unwrap();
        let managed = engine
            .flakes
            .iter()
            .find(|flake| flake.is_managed())
            .expect("managed flake listed");
        assert_eq!(managed.name(), "helium-browser#default");

        engine
            .apply(&remove("helium-browser", "default"), &mut SilentReporter)
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(engine.config.flake_file()).unwrap(),
            FLAKE
        );
        assert!(engine.flakes.iter().all(|flake| !flake.is_managed()));
        drop(dir);
    }
}
