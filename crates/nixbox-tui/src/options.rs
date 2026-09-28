//! The options panel: Enter on an Installed row lists the module options
//! for that package, read from the user's flake, and lets them be edited.
//!
//! Edits are staged in the panel and only queued, as one
//! [`QueuedOp::SetOptions`], when the user applies them, so a round of
//! changes costs one rebuild.

use std::collections::BTreeMap;
use std::sync::Arc;

use crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};
use nixbox_config::{InputMode, Target};
use nixbox_core::options::{choices, locked_reason, parse_text};
use nixbox_core::{Enqueued, OptionChange, load_options};
use nixbox_nix::options::{OptionEntry, OptionKind, OptionSet};
use nixbox_nix::scan::ScanTarget;
use nixbox_nix::settings::{OptionPath, SettingValue};
use tokio::sync::mpsc;

use crate::app::{App, AppEvent, InstalledCursor, QueuedOp, Tab};
use crate::vim::{VimInput, VimMode};

/// The package whose options the panel shows.
pub(crate) type PanelKey = (Target, String);

pub(crate) struct OptionsPanel {
    pub(crate) scope: Target,
    pub(crate) package: String,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
    pub(crate) set: Option<Arc<OptionSet>>,
    pub(crate) filter: VimInput,
    pub(crate) selected: usize,
    /// Edits not yet queued; `None` unsets the option.
    pub(crate) staged: BTreeMap<OptionPath, Option<SettingValue>>,
    pub(crate) editor: Option<OptionEditor>,
    /// Set after one Esc with staged edits; a second Esc discards them.
    pub(crate) confirm_close: bool,
}

pub(crate) enum OptionEditor {
    /// Free text for strings, numbers, and comma-separated string lists.
    Text { path: OptionPath, input: VimInput },
    /// A pick from fixed values, for enums and nullable booleans. Values
    /// are kept typed: an enum can allow the string `"true"`, which is not
    /// the boolean `true`.
    Choice {
        path: OptionPath,
        values: Vec<SettingValue>,
        cursor: usize,
    },
}

impl OptionsPanel {
    fn new(scope: Target, package: String, input_mode: InputMode) -> Self {
        let mut filter = VimInput::default();
        if input_mode == InputMode::Normal {
            filter.enter_insert_before();
        }
        Self {
            scope,
            package,
            loading: true,
            error: None,
            set: None,
            filter,
            selected: 0,
            staged: BTreeMap::new(),
            editor: None,
            confirm_close: false,
        }
    }

    pub(crate) fn key(&self) -> PanelKey {
        (self.scope, self.package.clone())
    }

    /// Options matching the filter, in evaluation order.
    pub(crate) fn visible(&self) -> Vec<&OptionEntry> {
        let Some(set) = &self.set else {
            return Vec::new();
        };
        let query = self.filter.value().trim().to_lowercase();
        set.entries
            .iter()
            .filter(|entry| {
                query.is_empty() || entry.path.join(".").to_lowercase().contains(&query)
            })
            .collect()
    }

    pub(crate) fn current(&self) -> Option<&OptionEntry> {
        let visible = self.visible();
        let index = self.selected.min(visible.len().checked_sub(1)?);
        visible.get(index).copied()
    }

    /// The name shown in the list: relative to the namespace when there is
    /// only one, the full path otherwise.
    pub(crate) fn display_name(&self, entry: &OptionEntry) -> String {
        match self.set.as_deref().map(|set| set.namespaces.as_slice()) {
            Some([only]) => entry.short_name(only.len()),
            _ => entry.path.join("."),
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let len = self.visible().len();
        if len == 0 {
            return;
        }
        self.selected = (self.selected as i32 + delta).rem_euclid(len as i32) as usize;
    }

    fn clamp_selection(&mut self) {
        let len = self.visible().len();
        self.selected = self.selected.min(len.saturating_sub(1));
    }

    /// The value an edit starts from: a staged one, else what evaluated.
    fn starting_value(&self, entry: &OptionEntry) -> Option<SettingValue> {
        match self.staged.get(&entry.path) {
            Some(staged) => staged.clone(),
            None => entry
                .value
                .as_ref()
                .and_then(|value| entry.kind.setting(value)),
        }
    }

    fn stage(&mut self, path: OptionPath, value: Option<SettingValue>) {
        self.staged.insert(path, value);
        self.confirm_close = false;
    }
}

/// Opens the panel for the Installed row under the cursor.
pub(crate) fn open_options_panel(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    let (scope, package) = match app.installed_cursor() {
        Some(InstalledCursor::Managed(package)) => (package.scope, package.name),
        Some(InstalledCursor::External(package)) => {
            let scope = match package.scope {
                ScanTarget::HomeManager => Target::HomeManager,
                ScanTarget::Nixos => Target::NixosSystem,
            };
            (scope, package.name)
        }
        Some(InstalledCursor::Flake(_)) => {
            app.status = "Flake outputs have no options panel yet.".into();
            return;
        }
        None => return,
    };
    let input_mode = app.session.engine.config.input_mode;
    app.options_panel = Some(OptionsPanel::new(scope, package, input_mode));
    load_panel(app, tx);
}

/// Fills the open panel from the cache, or starts evaluating its options.
pub(crate) fn load_panel(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    let Some(panel) = app.options_panel.as_mut() else {
        return;
    };
    let key = panel.key();
    if let Some(set) = app.options_cache.get(&key) {
        panel.set = Some(Arc::clone(set));
        panel.loading = false;
        panel.error = None;
        panel.clamp_selection();
        return;
    }
    panel.loading = true;
    panel.error = None;
    if let Some(task) = app.options_task.take() {
        task.abort();
    }
    app.options_epoch += 1;
    let epoch = app.options_epoch;
    let config = app.session.engine.config.clone();
    let tx = tx.clone();
    app.status = format!("Reading options for {}...", key.1);
    app.options_task = Some(tokio::spawn(async move {
        let event = match load_options(config, key.0, key.1.clone()).await {
            Ok(set) => AppEvent::OptionsLoaded {
                epoch,
                key,
                set: Arc::new(set),
            },
            Err(error) => AppEvent::OptionsFailed {
                epoch,
                error: format!("{error:#}"),
            },
        };
        let _ = tx.send(event).await;
    }));
}

/// Takes a finished evaluation, ignoring ones a newer request replaced.
pub(crate) fn on_options_loaded(app: &mut App, epoch: u64, key: PanelKey, set: Arc<OptionSet>) {
    if epoch != app.options_epoch {
        return;
    }
    app.options_task = None;
    app.options_cache.insert(key.clone(), Arc::clone(&set));
    let Some(panel) = app.options_panel.as_mut() else {
        return;
    };
    if panel.key() != key {
        return;
    }
    // A reload after a rebuild lands while the Building tab shows how the
    // rebuild went; that status matters more than this one.
    if app.tab == Tab::Installed {
        app.status = match set.namespaces.as_slice() {
            [] => format!("No module options found for {}.", key.1),
            namespaces => {
                let names: Vec<String> = namespaces.iter().map(|path| path.join(".")).collect();
                format!("{} options in {}.", set.entries.len(), names.join(", "))
            }
        };
    }
    panel.set = Some(set);
    panel.loading = false;
    panel.error = None;
    panel.clamp_selection();
}

pub(crate) fn on_options_failed(app: &mut App, epoch: u64, error: String) {
    if epoch != app.options_epoch {
        return;
    }
    app.options_task = None;
    if let Some(panel) = app.options_panel.as_mut() {
        panel.loading = false;
        panel.error = Some(error);
        app.status = "Reading options failed.".into();
    }
}

/// Values change once a rebuild ends, so cached evaluations are dropped and
/// an open panel reads its package again.
pub(crate) fn on_build_ended(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    app.options_cache.clear();
    if app
        .options_panel
        .as_ref()
        .is_some_and(|panel| !panel.loading)
    {
        load_panel(app, tx);
    }
}

/// Handles a key while the panel is open. Returns false for keys the panel
/// leaves to the rest of the app, such as switching tabs.
pub(crate) fn handle_panel_key(app: &mut App, tx: &mpsc::Sender<AppEvent>, key: KeyEvent) -> bool {
    if matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
        || (key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s'))
    {
        return false;
    }
    let normal_input = app.session.engine.config.input_mode == InputMode::Normal;
    let Some(panel) = app.options_panel.as_mut() else {
        return false;
    };

    if panel.editor.is_some() {
        handle_editor_key(app, key);
        return true;
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let typing = normal_input || panel.filter.mode() == VimMode::Insert;
    match key.code {
        KeyCode::Esc if !normal_input && panel.filter.mode() != VimMode::Normal => {
            panel.filter.enter_normal();
        }
        KeyCode::Esc => close_panel(app),
        KeyCode::Down => panel.move_selection(1),
        KeyCode::Up => panel.move_selection(-1),
        KeyCode::Enter => begin_edit(app),
        KeyCode::Char('w') if ctrl || !typing => apply_staged(app),
        KeyCode::Char('u') if ctrl || !typing => unset_current(app),
        KeyCode::Char('z') if ctrl && normal_input => discard_staged(app),
        KeyCode::Char('U') if !typing => discard_staged(app),
        KeyCode::Char('r') if ctrl || !typing => {
            let key = panel.key();
            app.options_cache.remove(&key);
            load_panel(app, tx);
        }
        KeyCode::Char('j') if !typing => panel.move_selection(1),
        KeyCode::Char('k') if !typing => panel.move_selection(-1),
        KeyCode::Char('/') if !typing => {
            panel.filter = VimInput::default();
            panel.filter.enter_insert_before();
            panel.selected = 0;
        }
        KeyCode::Char('i') if !typing => panel.filter.enter_insert_end(),
        _ if typing && !ctrl && panel.filter.handle_insert_event(&CtEvent::Key(key)) => {
            panel.selected = 0;
        }
        _ => {}
    }
    true
}

fn close_panel(app: &mut App) {
    let Some(panel) = app.options_panel.as_mut() else {
        return;
    };
    if !panel.staged.is_empty() && !panel.confirm_close {
        panel.confirm_close = true;
        app.status = format!(
            "{} staged change(s) not applied. Esc again to discard, w to apply.",
            panel.staged.len()
        );
        return;
    }
    app.options_panel = None;
    if let Some(task) = app.options_task.take() {
        task.abort();
    }
    app.status = "Closed options.".into();
}

fn begin_edit(app: &mut App) {
    let Some(panel) = app.options_panel.as_mut() else {
        return;
    };
    let Some(entry) = panel.current().cloned() else {
        return;
    };
    if let Some(reason) = locked_reason(&entry) {
        app.status = format!("{} is not editable: {reason}.", entry.path.join("."));
        return;
    }
    let start = panel.starting_value(&entry);
    let nullable = matches!(entry.kind, OptionKind::Nullable(_));
    match entry.kind.inner() {
        OptionKind::Bool if !nullable => {
            let flipped = !matches!(start, Some(SettingValue::Bool(true)));
            panel.stage(entry.path.clone(), Some(SettingValue::Bool(flipped)));
            app.status = format!("Staged {} = {flipped}.", entry.path.join("."));
        }
        OptionKind::Bool | OptionKind::Enum(_) => {
            let values = choices(&entry.kind);
            let cursor = values
                .iter()
                .position(|value| Some(value) == start.as_ref())
                .unwrap_or(0);
            panel.editor = Some(OptionEditor::Choice {
                path: entry.path.clone(),
                values,
                cursor,
            });
        }
        _ => {
            let text = match &start {
                Some(SettingValue::Str(value)) => value.clone(),
                Some(SettingValue::Int(value)) => value.to_string(),
                Some(SettingValue::Float(value)) => value.to_string(),
                Some(SettingValue::StrList(items)) => items.join(", "),
                _ => String::new(),
            };
            let mut input = VimInput::new(text);
            input.enter_insert_end();
            panel.editor = Some(OptionEditor::Text {
                path: entry.path.clone(),
                input,
            });
        }
    }
}

fn handle_editor_key(app: &mut App, key: KeyEvent) {
    let Some(panel) = app.options_panel.as_mut() else {
        return;
    };
    let Some(editor) = panel.editor.as_mut() else {
        return;
    };
    match editor {
        OptionEditor::Choice { values, cursor, .. } => match key.code {
            KeyCode::Down | KeyCode::Char('j') => *cursor = (*cursor + 1) % values.len(),
            KeyCode::Up | KeyCode::Char('k') => {
                *cursor = (*cursor + values.len() - 1) % values.len();
            }
            KeyCode::Esc => panel.editor = None,
            KeyCode::Enter => {
                let Some(OptionEditor::Choice {
                    path,
                    values,
                    cursor,
                }) = panel.editor.take()
                else {
                    return;
                };
                let Some(value) = values.into_iter().nth(cursor) else {
                    return;
                };
                app.status = format!("Staged {} = {}.", path.join("."), value.to_nix());
                panel.stage(path, Some(value));
            }
            _ => {}
        },
        OptionEditor::Text { input, path } => match key.code {
            KeyCode::Esc => panel.editor = None,
            KeyCode::Enter => {
                let kind = panel
                    .set
                    .as_ref()
                    .and_then(|set| set.entries.iter().find(|entry| &entry.path == path))
                    .map(|entry| entry.kind.clone());
                let Some(kind) = kind else {
                    panel.editor = None;
                    return;
                };
                match parse_text(&kind, input.value()) {
                    Ok(value) => {
                        let path = path.clone();
                        panel.editor = None;
                        app.status = format!("Staged {} = {}.", path.join("."), value.to_nix());
                        panel.stage(path, Some(value));
                    }
                    Err(message) => app.status = message,
                }
            }
            _ => {
                input.handle_insert_event(&CtEvent::Key(key));
            }
        },
    }
}

fn unset_current(app: &mut App) {
    let Some(entry) = app
        .options_panel
        .as_ref()
        .and_then(OptionsPanel::current)
        .cloned()
    else {
        return;
    };
    let ours = app
        .options_panel
        .as_ref()
        .is_some_and(|panel| app.session.engine.sets_option(panel.scope, &entry));
    let Some(panel) = app.options_panel.as_mut() else {
        return;
    };
    if ours {
        panel.stage(entry.path.clone(), None);
        app.status = format!("Staged unset {}.", entry.path.join("."));
    } else if panel.staged.remove(&entry.path).is_some() {
        app.status = format!("Dropped the staged change to {}.", entry.path.join("."));
    } else {
        app.status = format!("nixbox does not set {}.", entry.path.join("."));
    }
}

fn discard_staged(app: &mut App) {
    let Some(panel) = app.options_panel.as_mut() else {
        return;
    };
    let count = panel.staged.len();
    panel.staged.clear();
    panel.confirm_close = false;
    app.status = format!("Discarded {count} staged change(s).");
}

/// Queues every staged edit as one op.
fn apply_staged(app: &mut App) {
    let Some(panel) = app.options_panel.as_mut() else {
        return;
    };
    if panel.staged.is_empty() {
        app.status = "No staged changes to apply.".into();
        return;
    }
    let scope = panel.scope;
    let changes: Vec<OptionChange> = std::mem::take(&mut panel.staged)
        .into_iter()
        .map(|(path, value)| OptionChange { path, value })
        .collect();
    panel.confirm_close = false;
    let op = QueuedOp::SetOptions { changes, scope };
    let label = op.label();
    app.status = match app.session.enqueue(op) {
        Enqueued::Started(_) => {
            app.tab = Tab::Building;
            format!("Applying {label}...")
        }
        Enqueued::Queued => {
            app.tab = Tab::Queue;
            format!("Queued {label}.")
        }
        Enqueued::NotWritten => format!("Could not write {label}; see the build log."),
        Enqueued::Duplicate => format!("{label} is already queued."),
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use nixbox_config::Config;
    use nixbox_nix::Manifest;
    use serde_json::Value;

    fn entry(path: &str, kind: OptionKind, value: Value) -> OptionEntry {
        OptionEntry {
            path: path.split('.').map(str::to_string).collect(),
            kind,
            type_description: String::new(),
            description: None,
            default: None,
            example: None,
            value: Some(value),
            defined_in: Vec::new(),
            set_by_nixbox: false,
            set_by_module: false,
            read_only: false,
        }
    }

    fn app_with_panel(entries: Vec<OptionEntry>) -> App {
        let mut manifest = Manifest::default();
        manifest.add("git");
        let mut app = App::new(Config::default(), manifest, Manifest::default(), Vec::new());
        app.tab = Tab::Installed;
        let mut panel = OptionsPanel::new(Target::HomeManager, "git".into(), InputMode::Vim);
        panel.loading = false;
        panel.set = Some(Arc::new(OptionSet {
            namespaces: vec![vec!["programs".into(), "git".into()]],
            entries,
        }));
        app.options_panel = Some(panel);
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        let (tx, _rx) = mpsc::channel(4);
        handle_panel_key(app, &tx, KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn enter_toggles_a_bool_and_esc_needs_confirming_with_staged_edits() {
        let mut app = app_with_panel(vec![entry(
            "programs.git.lfs.enable",
            OptionKind::Bool,
            Value::Bool(false),
        )]);

        press(&mut app, KeyCode::Enter);
        let panel = app.options_panel.as_ref().unwrap();
        assert_eq!(
            panel.staged.values().next(),
            Some(&Some(SettingValue::Bool(true)))
        );

        press(&mut app, KeyCode::Esc);
        assert!(app.options_panel.is_some());
        assert!(!app.should_quit);
        press(&mut app, KeyCode::Esc);
        assert!(app.options_panel.is_none());
        assert!(!app.should_quit);
    }

    #[test]
    fn locked_options_are_not_staged() {
        let mut locked = entry("programs.git.enable", OptionKind::Bool, Value::Bool(true));
        locked.defined_in = vec!["configuration.nix".into()];
        let mut app = app_with_panel(vec![locked]);

        press(&mut app, KeyCode::Enter);
        assert!(app.options_panel.as_ref().unwrap().staged.is_empty());
        assert!(app.status.contains("set in configuration.nix"));
    }

    #[test]
    fn enum_strings_that_read_like_booleans_stay_strings() {
        let dnssec = OptionKind::Enum(vec![
            "true".into(),
            "allow-downgrade".into(),
            "false".into(),
        ]);
        let mut app = app_with_panel(vec![entry(
            "services.resolved.dnssec",
            dnssec.clone(),
            Value::String("allow-downgrade".into()),
        )]);

        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('k'));
        press(&mut app, KeyCode::Enter);

        let panel = app.options_panel.as_ref().unwrap();
        assert_eq!(
            panel.staged.values().next(),
            Some(&Some(SettingValue::Str("true".into())))
        );
        assert_eq!(
            choices(&OptionKind::Nullable(Box::new(OptionKind::Bool))),
            [
                SettingValue::Null,
                SettingValue::Bool(true),
                SettingValue::Bool(false)
            ]
        );
    }

    #[test]
    fn applying_queues_every_staged_change_as_one_op() {
        let mut app = app_with_panel(vec![
            entry(
                "programs.git.lfs.enable",
                OptionKind::Bool,
                Value::Bool(false),
            ),
            entry(
                "programs.git.ignores",
                OptionKind::StrList,
                Value::Array(Vec::new()),
            ),
        ]);
        let _cancel = app.session.fake_build(Target::NixosSystem, "busy");
        let panel = app.options_panel.as_mut().unwrap();
        panel.stage(
            vec![
                "programs".into(),
                "git".into(),
                "lfs".into(),
                "enable".into(),
            ],
            Some(SettingValue::Bool(true)),
        );
        panel.stage(
            vec!["programs".into(), "git".into(), "ignores".into()],
            None,
        );

        press(&mut app, KeyCode::Char('w'));

        assert_eq!(app.session.queue.len(), 1);
        assert!(matches!(
            app.session.queue.front(),
            Some(QueuedOp::SetOptions { changes, scope: Target::HomeManager }) if changes.len() == 2
        ));
        assert!(app.options_panel.as_ref().unwrap().staged.is_empty());
        assert_eq!(app.tab, Tab::Queue);
    }

    #[test]
    fn stale_option_results_are_ignored() {
        let mut app = app_with_panel(Vec::new());
        app.options_epoch = 2;
        let set = Arc::new(OptionSet::default());
        on_options_loaded(&mut app, 1, (Target::HomeManager, "git".into()), set);
        assert!(app.options_cache.is_empty());
    }
}
