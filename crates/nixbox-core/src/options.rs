//! Package options: reading them from the user's flake, and writing the
//! ones nixbox sets into its settings module.

use std::time::Duration;

use anyhow::Result;
use nixbox_config::{Config, Target};
use nixbox_nix::build::flake_has_home_configuration;
use nixbox_nix::options::{
    OptionEntry, OptionKind, OptionQuery, OptionSet, OptionSource, fetch_options,
};
use nixbox_nix::settings::{
    OptionPath, SettingValue, SettingsFile, SettingsManifest, option_path_display,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::{Engine, git_track, note_import};
use crate::report::Reporter;
use crate::session::Session;

/// How long reading one package's options may take. The first evaluation
/// of a configuration can be slow; later ones hit Nix's caches.
const OPTIONS_TIMEOUT: Duration = Duration::from_secs(120);

/// The NixOS configuration nixbox rebuilds, and so the one it reads.
const NIXOS_HOST: &str = "nixos";

/// One option to set, or to stop setting when `value` is `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptionChange {
    pub path: OptionPath,
    pub value: Option<SettingValue>,
}

impl OptionChange {
    /// `programs.git.enable = true` or `unset programs.git.enable`.
    #[must_use]
    pub fn label(&self) -> String {
        let path = option_path_display(&self.path);
        match &self.value {
            Some(value) => format!("{path} = {}", value.to_nix()),
            None => format!("unset {path}"),
        }
    }
}

/// Reads the options `package` can be configured with in `scope`.
pub async fn load_options(config: Config, scope: Target, package: String) -> Result<OptionSet> {
    let root = config.home_manager_dir();
    let source = match scope {
        Target::NixosSystem => OptionSource::Nixos,
        Target::HomeManager if flake_has_home_configuration(&root).await => OptionSource::Home,
        Target::HomeManager => OptionSource::HomeInNixos,
    };
    let settings_file = config
        .settings_file_for(scope)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let query = OptionQuery {
        root,
        source,
        host: NIXOS_HOST.into(),
        user: std::env::var("USER").unwrap_or_else(|_| "user".into()),
        package,
        settings_file,
    };
    fetch_options(&query, OPTIONS_TIMEOUT).await
}

/// Why an option cannot be edited here, or `None` when it can.
pub fn locked_reason(entry: &OptionEntry) -> Option<String> {
    if entry.read_only {
        Some("read-only option".into())
    } else if let Some(file) = entry.defined_in.first() {
        Some(format!("set in {file}"))
    } else if entry.set_by_module {
        Some("set by another module".into())
    } else if !entry.kind.is_editable() {
        Some("type not editable here".into())
    } else {
        None
    }
}

/// The values a choice editor offers for `kind`, `null` first when the
/// option accepts it.
pub fn choices(kind: &OptionKind) -> Vec<SettingValue> {
    let mut values = match kind.inner() {
        OptionKind::Enum(values) => values.iter().cloned().map(SettingValue::Str).collect(),
        _ => vec![SettingValue::Bool(true), SettingValue::Bool(false)],
    };
    if matches!(kind, OptionKind::Nullable(_)) {
        values.insert(0, SettingValue::Null);
    }
    values
}

/// Reads typed text as a value of `kind`. Empty text means `null` for
/// nullable options and an empty list for lists.
pub fn parse_text(kind: &OptionKind, text: &str) -> Result<SettingValue, String> {
    let trimmed = text.trim();
    if let OptionKind::Nullable(inner) = kind {
        return if trimmed.is_empty() {
            Ok(SettingValue::Null)
        } else {
            parse_text(inner, text)
        };
    }
    match kind {
        OptionKind::Str => Ok(SettingValue::Str(text.to_string())),
        OptionKind::Int => trimmed
            .parse()
            .map(SettingValue::Int)
            .map_err(|_| format!("`{trimmed}` is not a whole number.")),
        OptionKind::Float => match trimmed.parse::<f64>() {
            Ok(value) if value.is_finite() => Ok(SettingValue::Float(value)),
            _ => Err(format!("`{trimmed}` is not a number.")),
        },
        OptionKind::StrList => Ok(SettingValue::StrList(
            text.split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect(),
        )),
        OptionKind::Enum(values) if values.iter().any(|value| value == trimmed) => {
            Ok(SettingValue::Str(trimmed.to_string()))
        }
        _ => Err("This option cannot be edited here.".into()),
    }
}

/// A short, single-line rendering of an evaluated value.
pub fn format_value(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return "unavailable".into();
    };
    match value {
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) if text.contains('\n') => {
            format!("{} lines", text.lines().count())
        }
        Value::String(text) => format!("{text:?}"),
        Value::Array(items) if items.is_empty() => "[ ]".into(),
        Value::Array(items) if items.iter().all(Value::is_string) => {
            let items: Vec<String> = items.iter().map(|item| format!("{item}")).collect();
            format!("[ {} ]", items.join(" "))
        }
        Value::Array(items) => format!("[ {} items ]", items.len()),
        Value::Object(map) => match map.get("_nixbox").and_then(Value::as_str) {
            Some("derivation") => format!(
                "<{}>",
                map.get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("derivation")
            ),
            Some("function") => "<function>".into(),
            Some(_) => "...".into(),
            None if map.is_empty() => "{ }".into(),
            None => format!("{{ {} attrs }}", map.len()),
        },
    }
}

/// Drops the module system's role markers, such as `{option}` and
/// `{manpage}`, that prefix a backticked name in descriptions.
pub fn plain_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let role = after.find('}').filter(|&close| {
            close > 0
                && after[..close].chars().all(|ch| ch.is_ascii_alphabetic())
                && after[close + 1..].starts_with('`')
        });
        match role {
            Some(close) => rest = &after[close + 1..],
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

impl Engine {
    /// Options nixbox sets for `scope`.
    #[must_use]
    pub fn settings_for(&self, scope: Target) -> &SettingsManifest {
        match scope {
            Target::HomeManager => &self.home_settings,
            Target::NixosSystem => &self.nixos_settings,
        }
    }

    /// Whether nixbox's settings module sets `entry` in `scope`. The
    /// engine's record is current even before the next evaluation reflects
    /// a value it just wrote.
    #[must_use]
    pub fn sets_option(&self, scope: Target, entry: &OptionEntry) -> bool {
        entry.set_by_nixbox || self.settings_for(scope).values.contains_key(&entry.path)
    }

    fn settings_for_mut(&mut self, scope: Target) -> &mut SettingsManifest {
        match scope {
            Target::HomeManager => &mut self.home_settings,
            Target::NixosSystem => &mut self.nixos_settings,
        }
    }

    /// Writes `changes` into `scope`'s settings module and makes sure the
    /// main file imports it.
    pub(crate) fn apply_settings(
        &mut self,
        changes: &[OptionChange],
        scope: Target,
        reporter: &mut dyn Reporter,
    ) -> Result<()> {
        for change in changes {
            self.settings_for_mut(scope)
                .apply(&change.path, change.value.as_ref());
        }
        let file = SettingsFile::new(self.config.settings_file_for(scope));
        file.write(self.settings_for(scope))?;
        for change in changes {
            reporter.info(format!("Set {} ({}).", change.label(), scope.label()));
        }
        reporter.info(format!(
            "Wrote {} ({}).",
            file.path().display(),
            scope.label()
        ));
        let main_file = self.config.main_file_for(scope);
        note_import(&main_file, file.path(), reporter);
        git_track(file.path(), reporter);
        if main_file.exists() {
            git_track(&main_file, reporter);
        }
        Ok(())
    }
}

impl Session {
    /// The value waiting in the queue for `path` in `scope`: `Some(None)`
    /// for a queued unset, `None` when nothing is queued for it.
    #[must_use]
    pub fn queued_option(&self, scope: Target, path: &[String]) -> Option<Option<SettingValue>> {
        self.queue.iter().rev().find_map(|op| match op {
            crate::Op::SetOptions {
                changes,
                scope: queued,
            } if *queued == scope => changes
                .iter()
                .find(|change| change.path == path)
                .map(|change| change.value.clone()),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SilentReporter;
    use crate::op::Op;
    use crate::tests::temp_dir;
    use nixbox_nix::Manifest;

    fn path(raw: &str) -> OptionPath {
        raw.split('.').map(str::to_string).collect()
    }

    #[test]
    fn setting_options_writes_the_settings_module_and_imports_it() {
        let dir = temp_dir("settings-apply");
        let config = dir.config();
        std::fs::write(
            config.main_file_for(Target::HomeManager),
            "{ pkgs, ... }:\n{\n  imports = [\n    ./other.nix\n  ];\n}\n",
        )
        .unwrap();
        let mut engine = Engine::from_parts(
            config.clone(),
            Manifest::default(),
            Manifest::default(),
            Vec::new(),
        );

        let set = Op::SetOptions {
            changes: vec![
                OptionChange {
                    path: path("programs.git.lfs.enable"),
                    value: Some(SettingValue::Bool(true)),
                },
                OptionChange {
                    path: path("programs.git.ignores"),
                    value: Some(SettingValue::StrList(vec![".direnv".into()])),
                },
            ],
            scope: Target::HomeManager,
        };
        engine.apply(&set, &mut SilentReporter).unwrap();
        let unset = Op::SetOptions {
            changes: vec![OptionChange {
                path: path("programs.git.ignores"),
                value: None,
            }],
            scope: Target::HomeManager,
        };
        engine.apply(&unset, &mut SilentReporter).unwrap();

        let written =
            std::fs::read_to_string(config.settings_file_for(Target::HomeManager)).unwrap();
        assert!(written.contains("\tprograms.git.lfs.enable = true;\n"));
        assert!(!written.contains("ignores"));
        let reloaded = SettingsFile::new(config.settings_file_for(Target::HomeManager))
            .load()
            .unwrap();
        assert_eq!(&reloaded, engine.settings_for(Target::HomeManager));
        let main = std::fs::read_to_string(config.main_file_for(Target::HomeManager)).unwrap();
        assert!(main.contains("./nixbox-home-settings.nix"));
    }

    #[test]
    fn text_edits_are_parsed_by_kind() {
        let list = OptionKind::StrList;
        assert_eq!(
            parse_text(&list, " .direnv, *.swp ,,"),
            Ok(SettingValue::StrList(vec![
                ".direnv".into(),
                "*.swp".into()
            ]))
        );
        let nullable = OptionKind::Nullable(Box::new(OptionKind::Int));
        assert_eq!(parse_text(&nullable, " "), Ok(SettingValue::Null));
        assert_eq!(parse_text(&nullable, "42"), Ok(SettingValue::Int(42)));
        assert!(parse_text(&OptionKind::Int, "4.2").is_err());
        assert!(parse_text(&OptionKind::Float, "inf").is_err());
    }

    #[test]
    fn descriptions_lose_role_markers() {
        assert_eq!(
            plain_markdown("Alias of {option}`programs.git.settings.alias`."),
            "Alias of `programs.git.settings.alias`."
        );
        assert_eq!(
            plain_markdown("See {manpage}`gitattributes(5)`, not { x } or {a}b."),
            "See `gitattributes(5)`, not { x } or {a}b."
        );
    }
}
