//! The unit of work both front-ends schedule and the engine applies.

use nixbox_config::Target;
use nixbox_nix::search::SearchHit;
use serde::{Deserialize, Serialize};

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
    Uninstall {
        name: String,
        scope: Target,
    },
    Migrate {
        names: Vec<String>,
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
            | Op::Uninstall { scope, .. }
            | Op::Migrate { scope, .. } => *scope,
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
            Op::Uninstall { name, .. } => format!("remove {} [{}]", name, tag),
            Op::Migrate { names, .. } => match names.as_slice() {
                [only] => format!("migrate {} [{}]", only, tag),
                rest => format!("migrate {} packages [{}]", rest.len(), tag),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Op;
    use nixbox_config::Target;
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
}
