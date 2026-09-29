//! The unit of work both front-ends schedule and the engine applies.

use nixbox_config::Target;
use nixbox_nix::flakes::FlakeDetails;
use nixbox_nix::manifest::FlakeOutput;
use nixbox_nix::search::SearchHit;
use nixbox_nix::wiring::attr_name;
use serde::{Deserialize, Serialize};

use crate::options::OptionChange;

/// One output the selected flake can install for the active target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlakeChoice {
    pub path: String,
    pub output: FlakeOutput,
}

impl FlakeChoice {
    #[must_use]
    pub fn operation(&self, repo: String, scope: Target) -> Op {
        match &self.output {
            FlakeOutput::Package(package) => Op::InstallFlakePackage {
                repo,
                package: package.clone(),
                scope,
            },
            FlakeOutput::Module(module) => Op::InstallFlake {
                repo,
                module: module.clone(),
                scope,
            },
        }
    }
}

/// Complete list of package and module outputs suitable for `scope`.
#[must_use]
pub fn flake_choices(details: &FlakeDetails, scope: Target) -> Vec<FlakeChoice> {
    let mut choices: Vec<FlakeChoice> = details
        .packages
        .iter()
        .map(|package| FlakeChoice {
            path: format!("packages.{}.{}", details.system, attr_name(&package.attr)),
            output: FlakeOutput::Package(package.attr.clone()),
        })
        .collect();
    choices.extend(
        details
            .modules
            .iter()
            .filter(|path| match scope {
                Target::NixosSystem => path.starts_with("nixosModules."),
                Target::HomeManager => {
                    path.starts_with("homeManagerModules.") || path.starts_with("homeModules.")
                }
            })
            .map(|path| FlakeChoice {
                path: path.clone(),
                output: FlakeOutput::Module(path.clone()),
            }),
    );
    choices
}

/// A single pending change to the user's configuration.
///
/// This is persisted verbatim in `state.json`, so variant names and field
/// names are part of an on-disk format — renaming one strands queued work
/// from an older release.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Op {
    Install {
        hit: SearchHit,
        scope: Target,
    },
    InstallFlake {
        repo: String,
        module: String,
        scope: Target,
    },
    InstallFlakePackage {
        repo: String,
        package: String,
        scope: Target,
    },
    UninstallFlake {
        repo: String,
        scope: Target,
    },
    /// Removes one flake output wherever it is declared, nixbox's flake
    /// module or the user's own main file.
    UninstallFlakeOutput {
        input: String,
        output: FlakeOutput,
        scope: Target,
    },
    Uninstall {
        name: String,
        scope: Target,
    },
    Migrate {
        names: Vec<String>,
        scope: Target,
    },
    /// Moves a hand-declared flake package into nixbox's generated module.
    MigrateFlakePackage {
        input: String,
        package: String,
        scope: Target,
    },
    /// Sets or unsets package options in nixbox's settings module. Queuing
    /// another for the same scope merges into the waiting one, so staged
    /// edits share a rebuild.
    SetOptions {
        changes: Vec<OptionChange>,
        scope: Target,
    },
}

impl Op {
    /// Which target this op rebuilds. Ops of different scopes cannot share a
    /// rebuild, so this is what batching keys on.
    #[must_use]
    pub fn scope(&self) -> Target {
        match self {
            Op::Install { scope, .. }
            | Op::InstallFlake { scope, .. }
            | Op::InstallFlakePackage { scope, .. }
            | Op::UninstallFlake { scope, .. }
            | Op::UninstallFlakeOutput { scope, .. }
            | Op::Uninstall { scope, .. }
            | Op::Migrate { scope, .. }
            | Op::MigrateFlakePackage { scope, .. }
            | Op::SetOptions { scope, .. } => *scope,
        }
    }

    /// True when queueing `self` next to `other` would do the same work
    /// twice: the same package, flake, or output in the same scope.
    #[must_use]
    pub fn duplicates(&self, other: &Op) -> bool {
        if self.scope() != other.scope() {
            return false;
        }
        match (self, other) {
            (Op::Install { hit: a, .. }, Op::Install { hit: b, .. }) => a.attr == b.attr,
            (
                Op::InstallFlake {
                    repo: a,
                    module: a_module,
                    ..
                },
                Op::InstallFlake {
                    repo: b,
                    module: b_module,
                    ..
                },
            ) => a == b && a_module == b_module,
            (
                Op::InstallFlakePackage {
                    repo: a,
                    package: a_package,
                    ..
                },
                Op::InstallFlakePackage {
                    repo: b,
                    package: b_package,
                    ..
                },
            ) => a == b && a_package == b_package,
            (Op::UninstallFlake { repo: a, .. }, Op::UninstallFlake { repo: b, .. })
            | (Op::Uninstall { name: a, .. }, Op::Uninstall { name: b, .. }) => a == b,
            (
                Op::UninstallFlakeOutput {
                    input: a_input,
                    output: a_output,
                    ..
                },
                Op::UninstallFlakeOutput {
                    input: b_input,
                    output: b_output,
                    ..
                },
            ) => a_input == b_input && a_output == b_output,
            (Op::Migrate { names: a, .. }, Op::Migrate { names: b, .. }) => {
                a.iter().any(|name| b.contains(name))
            }
            (
                Op::MigrateFlakePackage {
                    input: a_input,
                    package: a_package,
                    ..
                },
                Op::MigrateFlakePackage {
                    input: b_input,
                    package: b_package,
                    ..
                },
            ) => a_input == b_input && a_package == b_package,
            _ => false,
        }
    }

    /// The op that installs the flake in `details` for `scope`, or `None`
    /// when it has nothing that target can use.
    ///
    /// A Home Manager module does nothing until its options are set, so for
    /// home the first package wins; NixOS prefers the module, which is how
    /// flakes usually expect to be added to a system.
    #[must_use]
    pub fn install_flake(details: &FlakeDetails, scope: Target) -> Option<Op> {
        let repo = details.repo.clone();
        let package = details.packages.first().map(|package| package.attr.clone());
        let as_package = |package| Op::InstallFlakePackage {
            repo: repo.clone(),
            package,
            scope,
        };
        let as_module = |module| Op::InstallFlake {
            repo: repo.clone(),
            module,
            scope,
        };
        let home_module = details.home_manager_module.clone().or_else(|| {
            details
                .modules
                .iter()
                .find(|path| {
                    path.starts_with("homeManagerModules.") || path.starts_with("homeModules.")
                })
                .cloned()
        });
        let nixos_module = details.nixos_module.clone().or_else(|| {
            details
                .modules
                .iter()
                .find(|path| path.starts_with("nixosModules."))
                .cloned()
        });
        match scope {
            Target::HomeManager => package
                .map(as_package)
                .or_else(|| home_module.map(as_module)),
            Target::NixosSystem => nixos_module
                .map(as_module)
                .or_else(|| package.map(as_package)),
        }
    }

    /// Short description used in status lines, logs, and the resume banner.
    #[must_use]
    pub fn label(&self) -> String {
        let tag = self.scope().tag();
        match self {
            Op::Install { hit, .. } => format!("install {} [{}]", hit.attr, tag),
            Op::InstallFlake { repo, .. } => format!("install flake {} [{}]", repo, tag),
            Op::InstallFlakePackage { repo, package, .. } => {
                format!("install {repo}#{package} [{tag}]")
            }
            Op::UninstallFlake { repo, .. } => format!("remove flake {} [{}]", repo, tag),
            Op::UninstallFlakeOutput { input, output, .. } => {
                format!("remove {} [{tag}]", output.display(input))
            }
            Op::Uninstall { name, .. } => format!("remove {} [{}]", name, tag),
            Op::Migrate { names, .. } => match names.as_slice() {
                [only] => format!("migrate {} [{}]", only, tag),
                rest => format!("migrate {} packages [{}]", rest.len(), tag),
            },
            Op::MigrateFlakePackage { input, package, .. } => {
                format!("migrate {input}#{package} [{tag}]")
            }
            Op::SetOptions { changes, .. } => match changes.as_slice() {
                [only] => format!("set {} [{tag}]", only.label()),
                rest => format!("set {} options [{tag}]", rest.len()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Op, flake_choices};
    use nixbox_config::Target;
    use nixbox_nix::flakes::{FlakeDetails, FlakePackage};
    use nixbox_nix::search::SearchHit;

    fn hit(attr: &str) -> SearchHit {
        SearchHit {
            attr: attr.into(),
            pname: attr.into(),
            version: "1.0".into(),
            description: String::new(),
        }
    }

    #[test]
    fn labels_carry_the_scope_tag() {
        assert_eq!(
            Op::Install {
                hit: hit("ripgrep"),
                scope: Target::HomeManager,
            }
            .label(),
            "install ripgrep [hm]"
        );
        assert_eq!(
            Op::Uninstall {
                name: "fd".into(),
                scope: Target::NixosSystem,
            }
            .label(),
            "remove fd [nixos]"
        );
        assert_eq!(
            Op::InstallFlake {
                repo: "owner/repo".into(),
                module: "nixosModules.default".into(),
                scope: Target::NixosSystem,
            }
            .label(),
            "install flake owner/repo [nixos]"
        );
        assert_eq!(
            Op::InstallFlakePackage {
                repo: "owner/repo".into(),
                package: "default".into(),
                scope: Target::HomeManager,
            }
            .label(),
            "install owner/repo#default [hm]"
        );
    }

    #[test]
    fn removing_a_flake_reads_as_a_flake_removal() {
        assert_eq!(
            Op::UninstallFlake {
                repo: "owner/repo".into(),
                scope: Target::HomeManager,
            }
            .label(),
            "remove flake owner/repo [hm]"
        );
        assert_eq!(
            Op::UninstallFlakeOutput {
                input: "llm-agents".into(),
                output: nixbox_nix::manifest::FlakeOutput::Package("claude-code".into()),
                scope: Target::HomeManager,
            }
            .label(),
            "remove llm-agents#claude-code [hm]"
        );
    }

    #[test]
    fn migrate_labels_switch_between_singular_and_count() {
        assert_eq!(
            Op::Migrate {
                names: vec!["git".into()],
                scope: Target::HomeManager,
            }
            .label(),
            "migrate git [hm]"
        );
        assert_eq!(
            Op::Migrate {
                names: vec!["git".into(), "neovim".into()],
                scope: Target::HomeManager,
            }
            .label(),
            "migrate 2 packages [hm]"
        );
    }

    #[test]
    fn serialized_form_is_stable_for_persisted_queues() {
        let json = serde_json::to_string(&Op::Uninstall {
            name: "fd".into(),
            scope: Target::NixosSystem,
        })
        .expect("serialize");

        assert_eq!(
            json,
            r#"{"Uninstall":{"name":"fd","scope":"nixos-system"}}"#
        );
    }

    fn details(packages: &[&str], nixos_module: Option<&str>) -> FlakeDetails {
        FlakeDetails {
            repo: "alleneubank/bun-overlay".into(),
            repo_url: "https://github.com/alleneubank/bun-overlay".into(),
            path: "flake.nix".into(),
            description: None,
            stars: 0,
            topics: Vec::new(),
            homepage: None,
            default_branch: "main".into(),
            pushed_at: None,
            archived: false,
            inputs: Vec::new(),
            outputs: vec!["packages".into()],
            output_entries: Vec::new(),
            packages: packages
                .iter()
                .map(|attr| FlakePackage {
                    attr: (*attr).into(),
                    name: "bun".into(),
                    version: "1.4.2".into(),
                })
                .collect(),
            system: "x86_64-linux".into(),
            modules: nixos_module.into_iter().map(str::to_string).collect(),
            nixos_module: nixos_module.map(Into::into),
            home_manager_module: None,
        }
    }

    #[test]
    fn a_flake_without_a_module_installs_its_first_package_on_nixos() {
        let op = Op::install_flake(&details(&["default"], None), Target::NixosSystem);

        assert!(matches!(
            op,
            Some(Op::InstallFlakePackage { repo, package, scope: Target::NixosSystem })
                if repo == "alleneubank/bun-overlay" && package == "default"
        ));
    }

    #[test]
    fn choices_keep_exact_output_paths_and_filter_modules_by_target() {
        let mut flake = details(&["default", "my.tool"], Some("nixosModules.default"));
        flake.modules.push("nixosModules.server".into());
        flake
            .modules
            .push("homeManagerModules.\"my-module\"".into());

        let nixos = flake_choices(&flake, Target::NixosSystem);
        assert_eq!(
            nixos
                .iter()
                .map(|choice| choice.path.as_str())
                .collect::<Vec<_>>(),
            [
                "packages.x86_64-linux.default",
                "packages.x86_64-linux.\"my.tool\"",
                "nixosModules.default",
                "nixosModules.server",
            ]
        );
        assert!(
            matches!(nixos[3].operation(flake.repo.clone(), Target::NixosSystem),
            Op::InstallFlake { module, .. } if module == "nixosModules.server")
        );

        let home = flake_choices(&flake, Target::HomeManager);
        assert_eq!(
            home.last().map(|choice| choice.path.as_str()),
            Some("homeManagerModules.\"my-module\"")
        );
        assert!(
            !home
                .iter()
                .any(|choice| choice.path.starts_with("nixosModules."))
        );
    }

    #[test]
    fn nixos_prefers_the_module_and_home_prefers_the_package() {
        let both = details(&["default"], Some("default"));

        assert!(matches!(
            Op::install_flake(&both, Target::NixosSystem),
            Some(Op::InstallFlake { .. })
        ));
        assert!(matches!(
            Op::install_flake(&both, Target::HomeManager),
            Some(Op::InstallFlakePackage { .. })
        ));
        assert!(Op::install_flake(&details(&[], None), Target::HomeManager).is_none());
    }

    #[test]
    fn duplicates_match_the_same_work_in_the_same_scope_only() {
        let install = |attr, scope| Op::Install {
            hit: hit(attr),
            scope,
        };
        assert!(install("fd", Target::HomeManager).duplicates(&install("fd", Target::HomeManager)));
        assert!(
            !install("fd", Target::HomeManager).duplicates(&install("fd", Target::NixosSystem))
        );
        assert!(
            !install("fd", Target::HomeManager).duplicates(&install("rg", Target::HomeManager))
        );

        let module = Op::InstallFlake {
            repo: "a/b".into(),
            module: "default".into(),
            scope: Target::HomeManager,
        };
        let package = Op::InstallFlakePackage {
            repo: "a/b".into(),
            package: "default".into(),
            scope: Target::HomeManager,
        };
        assert!(!module.duplicates(&package));
        assert!(module.duplicates(&module));
        assert!(package.duplicates(&package));
        assert!(!module.duplicates(&Op::InstallFlake {
            repo: "a/b".into(),
            module: "another".into(),
            scope: Target::HomeManager,
        }));

        let migrate = |names: &[&str]| Op::Migrate {
            names: names.iter().map(|name| (*name).to_string()).collect(),
            scope: Target::HomeManager,
        };
        assert!(migrate(&["git", "fd"]).duplicates(&migrate(&["fd"])));
        assert!(!migrate(&["git"]).duplicates(&migrate(&["fd"])));
        assert!(!migrate(&["fd"]).duplicates(&install("fd", Target::HomeManager)));
    }
}
