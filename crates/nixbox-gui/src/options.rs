//! The options page: a package's module options, read from the user's
//! flake, with simple values staged here and applied as one queued op.

use std::collections::BTreeMap;
use std::sync::Arc;

use gpui_kit::{Context, Window};
use nixbox_config::Target;
use nixbox_core::options::{locked_reason, parse_text};
use nixbox_core::{Op, OptionChange, load_options};
use nixbox_nix::options::{OptionEntry, OptionSet};
use nixbox_nix::settings::{OptionPath, SettingValue};

use crate::app::{NixboxApp, Page};

/// The package the options page shows, and what has been staged for it.
pub struct OptionsView {
    pub scope: Target,
    pub package: String,
    pub set: Option<Arc<OptionSet>>,
    pub loading: bool,
    pub error: Option<String>,
    pub selected: Option<OptionPath>,
    /// Edits not yet queued; `None` unsets the option.
    pub staged: BTreeMap<OptionPath, Option<SettingValue>>,
    /// Set after one Back with staged edits; a second Back discards them.
    confirm_close: bool,
}

impl OptionsView {
    fn new(scope: Target, package: String) -> Self {
        Self {
            scope,
            package,
            set: None,
            loading: true,
            error: None,
            selected: None,
            staged: BTreeMap::new(),
            confirm_close: false,
        }
    }

    /// Options whose path contains `filter`, ignoring case.
    pub fn visible(&self, filter: &str) -> Vec<&OptionEntry> {
        let filter = filter.trim().to_lowercase();
        self.set
            .as_deref()
            .map(|set| {
                set.entries
                    .iter()
                    .filter(|entry| {
                        filter.is_empty() || entry.path.join(".").to_lowercase().contains(&filter)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn entry(&self, path: &[String]) -> Option<&OptionEntry> {
        self.set
            .as_deref()?
            .entries
            .iter()
            .find(|entry| entry.path == path)
    }

    pub fn selected_entry(&self) -> Option<&OptionEntry> {
        self.entry(self.selected.as_deref()?)
    }

    /// The name shown in the list: relative to the namespace when there is
    /// only one, the full path otherwise.
    pub fn display_name(&self, entry: &OptionEntry) -> String {
        match self.set.as_deref().map(|set| set.namespaces.as_slice()) {
            Some([only]) => entry.short_name(only.len()),
            _ => entry.path.join("."),
        }
    }

    /// The value an edit starts from: a staged one, else what evaluated.
    pub fn current(&self, entry: &OptionEntry) -> Option<SettingValue> {
        match self.staged.get(&entry.path) {
            Some(staged) => staged.clone(),
            None => entry
                .value
                .as_ref()
                .and_then(|value| entry.kind.setting(value)),
        }
    }
}

/// How a value reads in the text field.
fn edit_text(value: Option<&SettingValue>) -> String {
    match value {
        Some(SettingValue::Str(text)) => text.clone(),
        Some(SettingValue::Int(number)) => number.to_string(),
        Some(SettingValue::Float(number)) => number.to_string(),
        Some(SettingValue::StrList(items)) => items.join(", "),
        _ => String::new(),
    }
}

impl NixboxApp {
    /// Opens the options page for an installed package.
    pub fn open_options(
        &mut self,
        package: String,
        scope: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.options = Some(OptionsView::new(scope, package));
        self.options_filter
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.set_page(Page::Options, window, cx);
        self.load_options(cx);
    }

    /// Fills the page from the cache, or starts evaluating its options.
    pub fn load_options(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.options.as_mut() else {
            return;
        };
        let key = (view.scope, view.package.clone());
        if let Some(set) = self.options_cache.get(&key) {
            view.set = Some(Arc::clone(set));
            view.loading = false;
            view.error = None;
            cx.notify();
            return;
        }
        view.loading = true;
        view.error = None;
        self.options_epoch = self.options_epoch.wrapping_add(1);
        let epoch = self.options_epoch;
        let config = self.session.engine.config.clone();
        let (scope, package) = key.clone();
        self.status = format!("Reading options for {package}...").into();
        let job = self
            .runtime
            .spawn(async move { load_options(config, scope, package).await });
        self.options_task = Some(cx.spawn(async move |this, cx| {
            let result = match job.await {
                Ok(result) => result.map_err(|error| format!("{error:#}")),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                if epoch != this.options_epoch {
                    return;
                }
                this.options_task = None;
                let set = result.map(Arc::new);
                if let Ok(set) = &set {
                    this.options_cache.insert(key.clone(), Arc::clone(set));
                }
                let Some(view) = this.options.as_mut() else {
                    return;
                };
                if (view.scope, view.package.clone()) != key {
                    return;
                }
                view.loading = false;
                let on_page = this.page == Page::Options;
                match set {
                    Ok(set) => {
                        if on_page {
                            this.status = match set.namespaces.as_slice() {
                                [] => format!("No module options found for {}.", key.1),
                                namespaces => {
                                    let names: Vec<String> =
                                        namespaces.iter().map(|path| path.join(".")).collect();
                                    format!(
                                        "{} options in {}.",
                                        set.entries.len(),
                                        names.join(", ")
                                    )
                                }
                            }
                            .into();
                        }
                        view.set = Some(set);
                        view.error = None;
                    }
                    Err(error) => {
                        if on_page {
                            this.status = "Reading options failed.".into();
                        }
                        view.error = Some(error);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Reads the open package's options again, skipping the cache.
    pub fn reload_options(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = &self.options {
            self.options_cache
                .remove(&(view.scope, view.package.clone()));
        }
        self.load_options(cx);
    }

    /// Values change once a rebuild ends, so cached evaluations are dropped
    /// and an open page reads its package again.
    pub(crate) fn options_after_build(&mut self, cx: &mut Context<Self>) {
        self.options_cache.clear();
        if self.options.as_ref().is_some_and(|view| !view.loading) {
            self.load_options(cx);
        }
    }

    pub fn select_option(&mut self, path: OptionPath, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.options.as_mut() else {
            return;
        };
        let text = view
            .entry(&path)
            .map(|entry| edit_text(view.current(entry).as_ref()))
            .unwrap_or_default();
        view.selected = Some(path);
        self.option_input
            .update(cx, |input, cx| input.set_value(text, window, cx));
        cx.notify();
    }

    pub fn stage_option(&mut self, value: SettingValue, cx: &mut Context<Self>) {
        let Some(view) = self.options.as_mut() else {
            return;
        };
        let Some(entry) = view.selected_entry() else {
            return;
        };
        if let Some(reason) = locked_reason(entry) {
            self.status = format!("{} is not editable: {reason}.", entry.path.join(".")).into();
            cx.notify();
            return;
        }
        let path = entry.path.clone();
        self.status = format!("Staged {} = {}.", path.join("."), value.to_nix()).into();
        view.staged.insert(path, Some(value));
        view.confirm_close = false;
        cx.notify();
    }

    /// Stages what is typed in the value field, if it reads as the
    /// selected option's type.
    pub fn stage_text(&mut self, cx: &mut Context<Self>) {
        let text = self.option_input.read(cx).value().to_string();
        let Some(kind) = self
            .options
            .as_ref()
            .and_then(OptionsView::selected_entry)
            .map(|entry| entry.kind.clone())
        else {
            return;
        };
        match parse_text(&kind, &text) {
            Ok(value) => self.stage_option(value, cx),
            Err(message) => {
                self.status = message.into();
                cx.notify();
            }
        }
    }

    /// Stages removing nixbox's value for the selected option, or drops its
    /// staged change when nixbox does not set it.
    pub fn unset_option(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self
            .options
            .as_ref()
            .and_then(OptionsView::selected_entry)
            .cloned()
        else {
            return;
        };
        let Some(view) = self.options.as_mut() else {
            return;
        };
        let path = entry.path.join(".");
        self.status = if self.session.engine.sets_option(view.scope, &entry) {
            view.staged.insert(entry.path, None);
            format!("Staged unset {path}.")
        } else if view.staged.remove(&entry.path).is_some() {
            format!("Dropped the staged change to {path}.")
        } else {
            format!("nixbox does not set {path}.")
        }
        .into();
        cx.notify();
    }

    pub fn discard_options(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = self.options.as_mut() {
            let count = view.staged.len();
            view.staged.clear();
            view.confirm_close = false;
            self.status = format!("Discarded {count} staged change(s).").into();
            cx.notify();
        }
    }

    /// Queues every staged edit as one op.
    pub fn apply_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.options.as_mut() else {
            return;
        };
        if view.staged.is_empty() {
            self.status = "No staged changes to apply.".into();
            cx.notify();
            return;
        }
        let changes: Vec<OptionChange> = std::mem::take(&mut view.staged)
            .into_iter()
            .map(|(path, value)| OptionChange { path, value })
            .collect();
        view.confirm_close = false;
        let op = Op::SetOptions {
            changes,
            scope: view.scope,
        };
        let duplicate = format!("{} is already queued.", op.label());
        self.enqueue(op, duplicate, window, cx);
    }

    /// Back to the Installed page. With staged edits the first call warns
    /// and a second discards them.
    pub fn close_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.options.as_mut()
            && !view.staged.is_empty()
            && !view.confirm_close
        {
            view.confirm_close = true;
            self.status = format!(
                "{} staged change(s) not applied. Press Back again to discard them.",
                view.staged.len()
            )
            .into();
            cx.notify();
            return;
        }
        self.options = None;
        self.options_task = None;
        self.set_page(Page::Installed, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, TestAppContext, px, size};
    use nixbox_config::{Config, Target};
    use nixbox_core::{Engine, Op, Session};
    use nixbox_nix::manifest::Manifest;
    use nixbox_nix::options::{OptionEntry, OptionKind, OptionSet};
    use nixbox_nix::settings::SettingValue;
    use serde_json::Value;

    use super::OptionsView;
    use crate::app::{NixboxApp, Page};

    fn entry(path: &str, kind: OptionKind, value: Value) -> OptionEntry {
        OptionEntry {
            path: path.split('.').map(str::to_string).collect(),
            kind,
            type_description: String::new(),
            description: Some("See {manpage}`git(1)`.".into()),
            default: None,
            example: None,
            value: Some(value),
            defined_in: Vec::new(),
            set_by_nixbox: false,
            set_by_module: false,
            read_only: false,
        }
    }

    #[gpui_kit::test]
    async fn options_page_renders_and_applies_staged_edits_as_one_op(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (session, build_rx) = Session::new(Engine::from_parts(
            Config::default(),
            Manifest::default(),
            Manifest::default(),
            Vec::new(),
        ));
        let session = session
            .without_persistence()
            .with_runtime(runtime.handle().clone());

        let mut app = None;
        let handle = cx.open_window(size(px(1180.), px(760.)), |window, cx| {
            let view = cx
                .new(|cx| NixboxApp::new(session, build_rx, runtime.handle().clone(), window, cx));
            app = Some(view.clone());
            Root::new(view, window, cx)
        });
        let app = app.expect("view");

        let mut locked = entry("programs.git.enable", OptionKind::Bool, Value::Bool(true));
        locked.defined_in = vec!["configuration.nix".into()];
        let set = OptionSet {
            namespaces: vec![vec!["programs".into(), "git".into()]],
            entries: vec![
                locked,
                entry(
                    "programs.git.lfs.enable",
                    OptionKind::Bool,
                    Value::Bool(false),
                ),
                entry(
                    "programs.git.signing.format",
                    OptionKind::Nullable(Box::new(OptionKind::Enum(vec![
                        "openpgp".into(),
                        "ssh".into(),
                    ]))),
                    Value::Null,
                ),
                entry(
                    "programs.git.ignores",
                    OptionKind::StrList,
                    Value::Array(Vec::new()),
                ),
            ],
        };

        cx.update_window(handle.into(), |_, window, cx| {
            app.update(cx, |app, cx| {
                let _cancel = app.session.fake_build(Target::NixosSystem, "busy");
                let mut view = OptionsView::new(Target::HomeManager, "git".into());
                view.loading = false;
                view.set = Some(Arc::new(set));
                app.options = Some(view);
                app.set_page(Page::Options, window, cx);

                // A locked option refuses a value.
                app.select_option(
                    vec!["programs".into(), "git".into(), "enable".into()],
                    window,
                    cx,
                );
                app.stage_option(SettingValue::Bool(false), cx);
                assert!(app.options.as_ref().unwrap().staged.is_empty());

                app.select_option(
                    vec![
                        "programs".into(),
                        "git".into(),
                        "lfs".into(),
                        "enable".into(),
                    ],
                    window,
                    cx,
                );
                app.stage_option(SettingValue::Bool(true), cx);
                app.select_option(
                    vec!["programs".into(), "git".into(), "ignores".into()],
                    window,
                    cx,
                );
                app.option_input.update(cx, |input, cx| {
                    input.set_value(".direnv, *.swp", window, cx)
                });
                app.stage_text(cx);
            });
            window.render_frame(cx);
        })
        .expect("window");

        cx.update_window(handle.into(), |_, window, cx| {
            app.update(cx, |app, cx| app.apply_options(window, cx));
            window.render_frame(cx);
        })
        .expect("window");

        app.read_with(cx, |app, _| {
            assert_eq!(app.session.queue.len(), 1);
            let Some(Op::SetOptions { changes, scope }) = app.session.queue.front() else {
                panic!("expected one settings op");
            };
            assert_eq!(*scope, Target::HomeManager);
            assert_eq!(changes.len(), 2);
            assert!(changes.iter().any(|change| change.value
                == Some(SettingValue::StrList(vec![
                    ".direnv".into(),
                    "*.swp".into()
                ]))));
            assert!(app.options.as_ref().unwrap().staged.is_empty());
        });
    }
}
