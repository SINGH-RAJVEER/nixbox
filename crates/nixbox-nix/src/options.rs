//! Reading the module options an installed package can be configured with,
//! such as `programs.git.*`, from the user's own flake.
//!
//! The evaluation lives in `options.nix`. It returns every visible option
//! under the package's namespaces with its type, documentation, current
//! value, and the files that define it; this module turns that into an
//! [`OptionSet`] and decides which options nixbox may edit.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;
use tokio::process::Command;
use tokio::time::timeout;

use crate::build::find_in_nix_profiles;
use crate::settings::{OptionPath, SettingValue};

const OPTIONS_EXPRESSION: &str = include_str!("options.nix");
const QUERY_ENV: &str = "NIXBOX_OPTIONS_QUERY";

/// Which evaluated configuration holds the options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionSource {
    /// `nixosConfigurations.<host>`.
    Nixos,
    /// A standalone `homeConfigurations.<user>`.
    Home,
    /// `home-manager.users.<user>` inside `nixosConfigurations.<host>`.
    HomeInNixos,
}

impl OptionSource {
    fn as_str(self) -> &'static str {
        match self {
            OptionSource::Nixos => "nixos",
            OptionSource::Home => "home",
            OptionSource::HomeInNixos => "home-in-nixos",
        }
    }
}

/// What to evaluate: one package in one configuration.
#[derive(Debug, Clone)]
pub struct OptionQuery {
    /// The directory holding the root `flake.nix`.
    pub root: PathBuf,
    pub source: OptionSource,
    pub host: String,
    pub user: String,
    /// The package attribute, such as `git` or `kdePackages.kate`.
    pub package: String,
    /// File name of nixbox's settings module for this target. Options it
    /// defines stay editable.
    pub settings_file: String,
}

/// The shape of an option's type, as far as nixbox can edit it.
#[derive(Debug, Clone, PartialEq)]
pub enum OptionKind {
    Bool,
    Int,
    Float,
    Str,
    Enum(Vec<String>),
    StrList,
    Nullable(Box<OptionKind>),
    /// Anything else, described by the module system.
    Other(String),
}

impl OptionKind {
    /// Whether nixbox can write a value of this kind.
    #[must_use]
    pub fn is_editable(&self) -> bool {
        match self {
            OptionKind::Other(_) => false,
            OptionKind::Nullable(inner) => inner.is_editable(),
            _ => true,
        }
    }

    /// The kind with any `null or` wrapper removed.
    #[must_use]
    pub fn inner(&self) -> &OptionKind {
        match self {
            OptionKind::Nullable(inner) => inner.inner(),
            kind => kind,
        }
    }

    /// Converts an evaluated value into one nixbox can write back, if it
    /// fits this kind.
    #[must_use]
    pub fn setting(&self, value: &Value) -> Option<SettingValue> {
        match (self, value) {
            (OptionKind::Nullable(_), Value::Null) => Some(SettingValue::Null),
            (OptionKind::Nullable(inner), value) => inner.setting(value),
            (OptionKind::Bool, Value::Bool(value)) => Some(SettingValue::Bool(*value)),
            (OptionKind::Int, Value::Number(number)) => number.as_i64().map(SettingValue::Int),
            (OptionKind::Float, Value::Number(number)) => number.as_f64().map(SettingValue::Float),
            (OptionKind::Str, Value::String(value)) => Some(SettingValue::Str(value.clone())),
            (OptionKind::Enum(values), Value::String(value)) if values.contains(value) => {
                Some(SettingValue::Str(value.clone()))
            }
            (OptionKind::StrList, Value::Array(items)) => items
                .iter()
                .map(|item| item.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
                .map(SettingValue::StrList),
            _ => None,
        }
    }
}

/// One module option.
#[derive(Debug, Clone)]
pub struct OptionEntry {
    pub path: OptionPath,
    pub kind: OptionKind,
    /// The module system's own description of the type.
    pub type_description: String,
    pub description: Option<String>,
    pub default: Option<String>,
    pub example: Option<String>,
    /// The evaluated value, or `None` when evaluating it failed.
    pub value: Option<Value>,
    /// Files under the configuration root, other than nixbox's settings
    /// module, that define this option.
    pub defined_in: Vec<String>,
    /// True when nixbox's settings module defines this option.
    pub set_by_nixbox: bool,
    /// True when an upstream module, not the user, defines this option at
    /// normal priority. A second definition would conflict unless the type
    /// merges.
    pub set_by_module: bool,
    pub read_only: bool,
}

impl OptionEntry {
    /// Whether nixbox may write this option. Anything the user defines in
    /// their own files is left alone: a second definition of a value that
    /// does not merge would fail the rebuild.
    #[must_use]
    pub fn editable(&self) -> bool {
        !self.read_only
            && self.defined_in.is_empty()
            && !self.set_by_module
            && self.kind.is_editable()
    }

    /// The option's name without its namespace, such as `lfs.enable`.
    #[must_use]
    pub fn short_name(&self, namespace_len: usize) -> String {
        self.path
            .get(namespace_len..)
            .unwrap_or(&self.path)
            .join(".")
    }
}

/// Every option found for one package.
#[derive(Debug, Clone, Default)]
pub struct OptionSet {
    /// The namespaces the options came from, such as `programs.git`.
    pub namespaces: Vec<OptionPath>,
    pub entries: Vec<OptionEntry>,
}

/// Evaluates the options for `query`, giving up after `limit`.
pub async fn fetch_options(query: &OptionQuery, limit: Duration) -> Result<OptionSet> {
    let request = serde_json::json!({
        "root": query.root,
        "source": query.source.as_str(),
        "host": query.host,
        "user": query.user,
        "package": query.package,
        "candidates": candidate_namespaces(&query.package),
    });
    let nix = find_in_nix_profiles("nix").unwrap_or_else(|| PathBuf::from("nix"));
    let command = Command::new(nix)
        .args(["eval", "--impure", "--json", "--expr", OPTIONS_EXPRESSION])
        .env(QUERY_ENV, request.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();
    let output = timeout(limit, command)
        .await
        .with_context(|| format!("timed out reading options for {}", query.package))??;
    if !output.status.success() {
        bail!(
            "evaluating options for {} failed: {}",
            query.package,
            last_error_line(&String::from_utf8_lossy(&output.stderr))
        );
    }
    let raw: RawSet = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("parsing options for {}", query.package))?;
    Ok(raw.into_set(&query.settings_file))
}

/// Namespaces to try before searching by package: `programs.<name>` and
/// `services.<name>`, for the attribute's last segment with and without a
/// trailing version, so `nodejs_22` also tries `nodejs`.
#[must_use]
pub fn candidate_namespaces(package: &str) -> Vec<OptionPath> {
    let name = package.rsplit('.').next().unwrap_or(package);
    let mut names = vec![name.to_string()];
    let unversioned = name
        .trim_end_matches(|ch: char| ch.is_ascii_digit())
        .trim_end_matches(['_', '-']);
    if !unversioned.is_empty() && unversioned != name {
        names.push(unversioned.to_string());
    }
    ["programs", "services"]
        .iter()
        .flat_map(|group| {
            names
                .iter()
                .map(move |name| vec![(*group).to_string(), name.clone()])
        })
        .collect()
}

/// Nix prints a trace before the error it ends on; the last `error:` line
/// is the part worth showing in a status bar.
fn last_error_line(stderr: &str) -> String {
    stderr
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with("error:"))
        .or_else(|| stderr.lines().rev().find(|line| !line.trim().is_empty()))
        .unwrap_or("no output")
        .trim()
        .to_string()
}

#[derive(Deserialize)]
struct RawSet {
    root: String,
    namespaces: Vec<OptionPath>,
    options: Vec<RawOption>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawOption {
    path: OptionPath,
    description: Attempt<Option<String>>,
    #[serde(default)]
    read_only: bool,
    #[serde(rename = "type")]
    kind: Attempt<RawType>,
    default: Attempt<Option<String>>,
    example: Attempt<Option<String>>,
    value: Attempt<Value>,
    files: Attempt<Vec<String>>,
    priority: Attempt<i64>,
}

/// A value `options.nix` evaluated under `tryEval`.
#[derive(Deserialize)]
#[serde(untagged)]
enum Attempt<T> {
    Ok { ok: T },
    Failed {},
}

impl<T> Attempt<T> {
    fn ok(self) -> Option<T> {
        match self {
            Attempt::Ok { ok } => Some(ok),
            Attempt::Failed {} => None,
        }
    }
}

#[derive(Deserialize)]
struct RawType {
    name: String,
    description: String,
    elem: Option<Box<RawType>>,
    values: Option<Vec<Value>>,
}

impl RawType {
    fn kind(&self) -> OptionKind {
        let other = || OptionKind::Other(self.description.clone());
        let name = self.name.as_str();
        match name {
            "bool" => OptionKind::Bool,
            "float" => OptionKind::Float,
            "str" | "singleLineStr" | "nonEmptyStr" => OptionKind::Str,
            "int" | "intBetween" | "positiveInt" => OptionKind::Int,
            _ if name.starts_with("signedInt") || name.starts_with("unsignedInt") => {
                OptionKind::Int
            }
            _ if name.starts_with("strMatching") => OptionKind::Str,
            "enum" => self
                .values
                .as_ref()
                .and_then(|values| {
                    values
                        .iter()
                        .map(|value| value.as_str().map(str::to_string))
                        .collect::<Option<Vec<_>>>()
                })
                .map_or_else(other, OptionKind::Enum),
            "nullOr" => match self.elem.as_ref().map(|elem| elem.kind()) {
                Some(inner) if inner.is_editable() => OptionKind::Nullable(Box::new(inner)),
                _ => other(),
            },
            "listOf" => match self.elem.as_ref().map(|elem| elem.kind()) {
                Some(OptionKind::Str) => OptionKind::StrList,
                _ => other(),
            },
            _ => other(),
        }
    }
}

impl RawSet {
    fn into_set(self, settings_file: &str) -> OptionSet {
        let root = format!("{}/", self.root.trim_end_matches('/'));
        let ours =
            |file: &str| file == settings_file || file.ends_with(&format!("/{settings_file}"));
        let entries = self
            .options
            .into_iter()
            .map(|raw| {
                let (kind, type_description) = match raw.kind.ok() {
                    Some(raw_type) => (raw_type.kind(), raw_type.description),
                    None => (OptionKind::Other("unknown".into()), "unknown".into()),
                };
                let files = raw.files.ok().unwrap_or_default();
                let local: Vec<String> = files
                    .iter()
                    .filter_map(|file| file.strip_prefix(&root).map(str::to_string))
                    .collect();
                let set_by_nixbox = local.iter().any(|file| ours(file));
                let mut defined_in: Vec<String> =
                    local.into_iter().filter(|file| !ours(file)).collect();
                defined_in.sort();
                defined_in.dedup();
                let set_by_module = raw.priority.ok().is_some_and(|priority| priority <= 100)
                    && defined_in.is_empty()
                    && !set_by_nixbox
                    && kind != OptionKind::StrList;
                OptionEntry {
                    path: raw.path,
                    kind,
                    type_description,
                    description: raw.description.ok().flatten(),
                    default: raw.default.ok().flatten(),
                    example: raw.example.ok().flatten(),
                    value: raw.value.ok(),
                    defined_in,
                    set_by_nixbox,
                    set_by_module,
                    read_only: raw.read_only,
                }
            })
            .collect();
        OptionSet {
            namespaces: self.namespaces,
            entries,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
        "root": "/nix/store/abc-source",
        "namespaces": [["programs", "git"]],
        "options": [
            {
                "path": ["programs", "git", "enable"],
                "description": {"ok": "Whether to enable Git."},
                "readOnly": false,
                "type": {"ok": {"name": "bool", "description": "boolean", "elem": null, "values": null}},
                "default": {"ok": "false"},
                "example": {"ok": "true"},
                "value": {"ok": true},
                "files": {"ok": ["/nix/store/abc-source/configuration.nix"]},
                "priority": {"ok": 100}
            },
            {
                "path": ["programs", "git", "signing", "format"],
                "description": {"ok": null},
                "readOnly": false,
                "type": {"ok": {"name": "nullOr", "description": "null or one of \"openpgp\", \"ssh\"",
                    "elem": {"name": "enum", "description": "one of", "elem": null, "values": ["openpgp", "ssh"]},
                    "values": null}},
                "default": {"failed": true},
                "example": {"ok": null},
                "value": {"ok": null},
                "files": {"ok": ["/nix/store/hm-source/modules/programs/git.nix"]},
                "priority": {"ok": 1500}
            },
            {
                "path": ["programs", "git", "ignores"],
                "description": {"ok": null},
                "readOnly": false,
                "type": {"ok": {"name": "listOf", "description": "list of string",
                    "elem": {"name": "str", "description": "string", "elem": null, "values": null},
                    "values": null}},
                "default": {"ok": "[ ]"},
                "example": {"ok": null},
                "value": {"ok": [".direnv"]},
                "files": {"ok": ["/nix/store/abc-source/nixbox-home-settings.nix", "/nix/store/hm-source/modules/programs/git.nix"]},
                "priority": {"ok": 100}
            },
            {
                "path": ["programs", "git", "hooks"],
                "description": {"ok": null},
                "readOnly": false,
                "type": {"ok": {"name": "attrsOf", "description": "attribute set of absolute path", "elem": null, "values": null}},
                "default": {"ok": null},
                "example": {"ok": null},
                "value": {"failed": true},
                "files": {"failed": true},
                "priority": {"failed": true}
            },
            {
                "path": ["programs", "git", "lfs", "skipSmudge"],
                "description": {"ok": null},
                "readOnly": false,
                "type": {"ok": {"name": "bool", "description": "boolean", "elem": null, "values": null}},
                "default": {"ok": "false"},
                "example": {"ok": null},
                "value": {"ok": true},
                "files": {"ok": ["/nix/store/hm-source/modules/programs/git-lfs.nix"]},
                "priority": {"ok": 100}
            }
        ]
    }"#;

    fn fixture() -> OptionSet {
        serde_json::from_str::<RawSet>(FIXTURE)
            .expect("fixture parses")
            .into_set("nixbox-home-settings.nix")
    }

    #[test]
    fn evaluated_options_decode_into_kinds_and_ownership() {
        let set = fixture();
        assert_eq!(set.namespaces, vec![vec!["programs", "git"]]);
        let [enable, format, ignores, hooks, skip_smudge] = set.entries.as_slice() else {
            panic!("expected five options");
        };

        assert_eq!(enable.kind, OptionKind::Bool);
        assert_eq!(enable.defined_in, vec!["configuration.nix"]);
        assert!(!enable.editable());

        assert_eq!(
            format.kind,
            OptionKind::Nullable(Box::new(OptionKind::Enum(vec![
                "openpgp".into(),
                "ssh".into()
            ])))
        );
        assert!(format.defined_in.is_empty());
        assert!(format.editable());
        assert_eq!(format.kind.setting(&Value::Null), Some(SettingValue::Null));

        assert_eq!(ignores.kind, OptionKind::StrList);
        assert!(ignores.set_by_nixbox);
        assert!(ignores.editable());
        assert_eq!(
            ignores.kind.setting(ignores.value.as_ref().unwrap()),
            Some(SettingValue::StrList(vec![".direnv".into()]))
        );

        assert!(matches!(hooks.kind, OptionKind::Other(_)));
        assert!(hooks.value.is_none());
        assert!(!hooks.editable());
        assert_eq!(hooks.short_name(2), "hooks");

        assert!(format.default.is_none());
        assert!(!format.set_by_module);
        assert!(skip_smudge.set_by_module);
        assert!(!skip_smudge.editable());
    }

    #[test]
    fn candidates_try_the_name_with_and_without_a_version() {
        assert_eq!(
            candidate_namespaces("nodejs_22"),
            vec![
                vec!["programs", "nodejs_22"],
                vec!["programs", "nodejs"],
                vec!["services", "nodejs_22"],
                vec!["services", "nodejs"],
            ]
        );
        assert_eq!(
            candidate_namespaces("kdePackages.kate"),
            vec![vec!["programs", "kate"], vec!["services", "kate"]]
        );
    }
}
