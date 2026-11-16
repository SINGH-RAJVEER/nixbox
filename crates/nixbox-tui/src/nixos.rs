//! The NixOS tab: curated settings pages with their values inline, the
//! changed settings, and the whole schema as a namespace tree. Edits are
//! staged here and queued together as one NixOS rebuild. The tab loads on
//! first visit and never changes the package install target.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};
use nixbox_config::Target;
use nixbox_core::hardware::HardwareFilter;
use nixbox_core::options::{NIXOS_HOST, choices, edit_text, parse_text};
use nixbox_core::settings_pages::{
	SchemaIndex, SearchResults, SettingsPage, ShownSection, TreeRow, View, ancestors,
	changed_paths, curated, namespace_rows, page_sections, search, section_paths, stage_edit,
};
use nixbox_core::system_options::{Category, GROUP_LIMIT, default_host, edit_lock, filter_options};
use nixbox_core::{Enqueued, OptionChange};
use nixbox_nix::hardware::HardwareInventory;
use nixbox_nix::options::OptionKind;
use nixbox_nix::settings::{OptionPath, SettingValue, option_path_display};
use nixbox_nix::system_options::{
	EditableOption, NixosConfiguration, SystemOption, discover_hosts, discover_options,
	inspect_group,
};
use tokio::{sync::mpsc, task::JoinHandle};

use crate::app::{App, AppEvent, QueuedOp, Tab};

const EVALUATION_LIMIT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub(crate) enum Event {
	Hosts {
		epoch: u64,
		/// The hosts, and the one to open without asking.
		result: Result<(Vec<String>, Option<String>), String>,
	},
	Catalog {
		epoch: u64,
		result: Result<(Vec<SystemOption>, HardwareInventory), String>,
	},
	Group {
		epoch: u64,
		result: Result<Vec<EditableOption>, String>,
	},
}

/// Which part of the tab takes keys.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Focus {
	#[default]
	Pages,
	Content,
	/// The search bar; results replace the content while it holds text.
	Search,
}

/// A value being edited in a popup.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Edit {
	pub(crate) path: OptionPath,
	/// Typed text, for values without a fixed set of choices.
	pub(crate) text: String,
	/// The choices offered, empty for typed values.
	pub(crate) choices: Vec<SettingValue>,
	pub(crate) cursor: usize,
	pub(crate) hint: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Popup {
	Edit(Edit),
	Review,
}

/// One selectable row of the content pane.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Item {
	Page(SettingsPage),
	Setting {
		path: OptionPath,
		label: Option<&'static str>,
	},
	Tree(TreeRow),
}

#[derive(Default)]
pub(crate) struct Panel {
	pub(crate) loaded: bool,
	pub(crate) loading: bool,
	pub(crate) hosts: Vec<String>,
	pub(crate) configuration: Option<NixosConfiguration>,
	pub(crate) options: Vec<SystemOption>,
	pub(crate) index: SchemaIndex,
	pub(crate) view: View,
	pub(crate) sections: Vec<ShownSection>,
	pub(crate) focus: Focus,
	/// The highlighted entry of the pages pane, in [`View::all`] order.
	pub(crate) nav: usize,
	/// The highlighted content row.
	pub(crate) cursor: usize,
	/// The highlighted search result.
	pub(crate) result: usize,
	pub(crate) search: SearchResults,
	pub(crate) category: Category,
	pub(crate) visible: Vec<usize>,
	pub(crate) expanded: BTreeSet<OptionPath>,
	pub(crate) rows: Vec<TreeRow>,
	pub(crate) show_diagnostic: bool,
	pub(crate) hardware: HardwareFilter,
	local_hardware: HardwareInventory,
	hardware_configuration: Option<NixosConfiguration>,
	/// Every option evaluated since the schema loaded, by path.
	pub(crate) evaluated: HashMap<OptionPath, EditableOption>,
	/// Edits not yet queued; `None` unsets the option.
	pub(crate) staged: BTreeMap<OptionPath, Option<SettingValue>>,
	pub(crate) popup: Option<Popup>,
	/// Whether the details take the whole content pane.
	pub(crate) details_open: bool,
	pub(crate) inspecting: bool,
	pub(crate) error: Option<String>,
	pub(crate) detail_error: Option<String>,
	pub(crate) detail_scroll: u16,
	catalog_epoch: u64,
	detail_epoch: u64,
	catalog_task: Option<JoinHandle<()>>,
	detail_task: Option<JoinHandle<()>>,
}

impl Panel {
	/// Stops every evaluation in flight and drops its result.
	pub(crate) fn abort(&mut self) {
		self.catalog_epoch = self.catalog_epoch.wrapping_add(1);
		if let Some(task) = self.catalog_task.take() {
			task.abort();
		}
		self.loading = false;
		self.abort_inspection();
	}

	fn abort_inspection(&mut self) {
		self.detail_epoch = self.detail_epoch.wrapping_add(1);
		if let Some(task) = self.detail_task.take() {
			task.abort();
		}
		self.inspecting = false;
		self.detail_error = None;
	}

	fn clear_schema(&mut self) {
		self.options.clear();
		self.index = SchemaIndex::default();
		self.sections.clear();
		self.search = SearchResults::default();
		self.visible.clear();
		self.rows.clear();
		self.evaluated.clear();
		self.cursor = 0;
		self.result = 0;
		self.error = None;
	}

	/// Installs a freshly read schema.
	pub(crate) fn set_options(&mut self, options: Vec<SystemOption>) {
		self.index = SchemaIndex::new(&options);
		self.options = options;
		self.evaluated.clear();
		self.resolve_view();
	}

	fn resolve_view(&mut self) {
		self.sections = match self.view {
			View::Page(page) => page_sections(page, &self.options, &self.index),
			_ => Vec::new(),
		};
	}

	pub(crate) fn option(&self, path: &[String]) -> Option<&SystemOption> {
		self.options.get(self.index.get(path)?)
	}

	pub(crate) fn editing(&self, path: &[String]) -> Option<&EditableOption> {
		self.evaluated.get(path)
	}

	/// The content pane's selectable rows.
	pub(crate) fn items(&self, session: &nixbox_core::Session) -> Vec<Item> {
		match self.view {
			View::Overview => SettingsPage::ALL.map(Item::Page).to_vec(),
			View::Page(_) => self
				.sections
				.iter()
				.flat_map(|section| {
					section.entries.iter().map(|entry| Item::Setting {
						path: entry.path.clone(),
						label: Some(entry.label),
					})
				})
				.collect(),
			View::Changed => changed_paths(&self.staged, session)
				.into_iter()
				.filter(|path| self.index.get(path).is_some())
				.take(GROUP_LIMIT)
				.map(|path| Item::Setting {
					label: curated(&path).map(|(_, _, entry)| entry.label),
					path,
				})
				.collect(),
			View::AllOptions => self.rows.iter().cloned().map(Item::Tree).collect(),
		}
	}

	/// The option the highlighted row stands for, when it stands for one.
	pub(crate) fn current_path(&self, session: &nixbox_core::Session) -> Option<OptionPath> {
		if self.focus == Focus::Search {
			return self
				.search
				.path_at(&self.options, self.result)
				.map(<[String]>::to_vec);
		}
		match self.items(session).into_iter().nth(self.cursor)? {
			Item::Setting { path, .. } => Some(path),
			Item::Tree(TreeRow::Option { index, .. }) => {
				self.options.get(index).map(|option| option.path.clone())
			}
			Item::Page(_) | Item::Tree(TreeRow::Namespace { .. }) => None,
		}
	}

	/// How many of `page`'s settings this host declares.
	pub(crate) fn page_size(&self, page: SettingsPage) -> usize {
		page_sections(page, &self.options, &self.index)
			.iter()
			.map(|section| section.entries.len())
			.sum()
	}
}

/// Whether nixbox's NixOS rebuild applies the open host.
pub(crate) fn editable_host(app: &App) -> bool {
	app.nixos
		.configuration
		.as_ref()
		.is_some_and(|configuration| {
			configuration.host
				== app
					.session
					.engine
					.config
					.nixos_host
					.as_deref()
					.unwrap_or(NIXOS_HOST)
		})
}

/// Why `option` cannot be changed here, or `None` when it can.
pub(crate) fn lock(app: &App, option: &EditableOption) -> Option<String> {
	if editable_host(app) {
		edit_lock(option)
	} else {
		Some("host not rebuilt by nixbox".into())
	}
}

/// What `option` is set to once queued work applies.
pub(crate) fn baseline(app: &App, option: &EditableOption) -> Option<SettingValue> {
	match app
		.session
		.queued_option(Target::NixosSystem, &option.entry.path)
	{
		Some(queued) => queued,
		None => option
			.entry
			.value
			.as_ref()
			.and_then(|value| option.entry.kind.setting(value)),
	}
}

/// The value an edit starts from: a staged one, else the baseline.
pub(crate) fn current(app: &App, option: &EditableOption) -> Option<SettingValue> {
	match app.nixos.staged.get(&option.entry.path) {
		Some(staged) => staged.clone(),
		None => baseline(app, option),
	}
}

/// Loads the host list the first time the tab opens, and again after a
/// rebuild invalidated it.
pub(crate) fn ensure_loaded(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
	if !app.nixos.loaded {
		reload(app, tx);
	}
}

pub(crate) fn reload(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
	let panel = &mut app.nixos;
	panel.abort();
	let previous = panel
		.configuration
		.take()
		.map(|configuration| configuration.host);
	panel.loaded = true;
	panel.loading = true;
	panel.hosts.clear();
	panel.clear_schema();
	let epoch = panel.catalog_epoch;
	let root = app.session.config_dir();
	let tx = tx.clone();
	app.nixos.catalog_task = Some(tokio::spawn(async move {
		let result = discover_hosts(&root, EVALUATION_LIMIT)
			.await
			.map_err(|error| format!("{error:#}"))
			.map(|hosts| {
				let host = default_host(&hosts, previous.as_deref());
				(hosts, host)
			});
		let _ = tx
			.send(AppEvent::Nixos(Event::Hosts { epoch, result }))
			.await;
	}));
	app.status = "Reading NixOS hosts…".into();
}

fn select_host(app: &mut App, tx: &mpsc::Sender<AppEvent>, host: String) {
	if !app.nixos.hosts.contains(&host) {
		return;
	}
	let panel = &mut app.nixos;
	panel.abort();
	panel.clear_schema();
	if panel
		.configuration
		.as_ref()
		.is_some_and(|configuration| configuration.host != host)
	{
		panel.staged.clear();
	}
	let configuration = NixosConfiguration {
		root: app.session.config_dir(),
		host,
	};
	if panel.hardware_configuration.as_ref() != Some(&configuration) {
		panel.hardware.inventory = None;
		panel.hardware_configuration = None;
	}
	panel.configuration = Some(configuration.clone());
	panel.loading = true;
	let epoch = panel.catalog_epoch;
	let tx = tx.clone();
	app.status = format!("Reading the option schema of {}…", configuration.host);
	app.nixos.catalog_task = Some(tokio::spawn(async move {
		let (catalog, hardware) = tokio::join!(
			discover_options(&configuration, EVALUATION_LIMIT),
			tokio::task::spawn_blocking(HardwareInventory::detect)
		);
		let result = catalog
			.map(|catalog| (catalog.options, hardware.unwrap_or_default()))
			.map_err(|error| format!("{error:#}"));
		let _ = tx
			.send(AppEvent::Nixos(Event::Catalog { epoch, result }))
			.await;
	}));
}

/// Evaluates every path in `paths` not evaluated yet, in one request.
fn evaluate(app: &mut App, tx: &mpsc::Sender<AppEvent>, paths: Vec<OptionPath>) {
	let Some(configuration) = app.nixos.configuration.clone() else {
		return;
	};
	let missing: Vec<OptionPath> = paths
		.into_iter()
		.filter(|path| !app.nixos.evaluated.contains_key(path))
		.filter(|path| app.nixos.index.get(path).is_some())
		.collect();
	if missing.is_empty() {
		return;
	}
	app.nixos.abort_inspection();
	app.nixos.inspecting = true;
	let settings_file = app
		.session
		.engine
		.config
		.settings_file_for(Target::NixosSystem)
		.file_name()
		.map(|name| name.to_string_lossy().into_owned())
		.unwrap_or_default();
	let epoch = app.nixos.detail_epoch;
	let tx = tx.clone();
	app.nixos.detail_task = Some(tokio::spawn(async move {
		let result = inspect_group(&configuration, &missing, &settings_file, EVALUATION_LIMIT)
			.await
			.map_err(|error| format!("{error:#}"));
		let _ = tx
			.send(AppEvent::Nixos(Event::Group { epoch, result }))
			.await;
	}));
}

/// Reads the values the open view shows.
fn evaluate_view(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
	let paths = match app.nixos.view {
		View::Page(_) => section_paths(&app.nixos.sections),
		View::Changed => app
			.nixos
			.items(&app.session)
			.into_iter()
			.filter_map(|item| match item {
				Item::Setting { path, .. } => Some(path),
				_ => None,
			})
			.collect(),
		View::Overview | View::AllOptions => Vec::new(),
	};
	evaluate(app, tx, paths);
}

pub(crate) fn on_event(app: &mut App, tx: &mpsc::Sender<AppEvent>, event: Event) {
	match event {
		Event::Hosts { epoch, result } => {
			if epoch != app.nixos.catalog_epoch {
				return;
			}
			app.nixos.catalog_task = None;
			app.nixos.loading = false;
			match result {
				Ok((hosts, chosen)) => {
					app.nixos.hosts = hosts;
					match chosen {
						Some(host) => select_host(app, tx, host),
						None if app.nixos.hosts.is_empty() => {
							app.status = "No nixosConfigurations outputs found.".into();
						}
						None => app.status = "Choose a NixOS host with ctrl-t.".into(),
					}
				}
				Err(error) => {
					app.status = "Could not read NixOS hosts.".into();
					app.nixos.error = Some(error);
				}
			}
		}
		Event::Catalog { epoch, result } => {
			if epoch != app.nixos.catalog_epoch {
				return;
			}
			app.nixos.catalog_task = None;
			app.nixos.loading = false;
			match result {
				Ok((options, hardware)) => {
					app.nixos.set_options(options);
					app.nixos.local_hardware = hardware.clone();
					if app.nixos.hardware_configuration == app.nixos.configuration {
						app.nixos.hardware.inventory = Some(hardware);
					}
					refilter(app);
					evaluate_view(app, tx);
					app.status = format!("{} NixOS options.", app.nixos.options.len());
				}
				Err(error) => {
					app.status = "Could not read the NixOS schema.".into();
					app.nixos.error = Some(error);
				}
			}
		}
		Event::Group { epoch, result } => {
			if epoch != app.nixos.detail_epoch {
				return;
			}
			app.nixos.detail_task = None;
			app.nixos.inspecting = false;
			match result {
				Ok(group) => app.nixos.evaluated.extend(
					group
						.into_iter()
						.map(|option| (option.entry.path.clone(), option)),
				),
				Err(error) => app.nixos.detail_error = Some(error),
			}
		}
	}
}

/// Recomputes the tree and the search results, keeping the cursor in range.
pub(crate) fn refilter(app: &mut App) {
	let panel = &mut app.nixos;
	panel.hardware.retained = panel.staged.keys().cloned().collect();
	panel.visible = filter_options(
		&panel.options,
		"",
		panel.category,
		panel.show_diagnostic,
		&panel.hardware,
	)
	.visible;
	panel.rows = namespace_rows(&panel.options, &panel.visible, &panel.expanded);
	panel.search = search(
		&panel.options,
		&panel.index,
		app.nixos_input.value(),
		panel.show_diagnostic,
		&panel.hardware,
	);
	panel.result = panel.result.min(panel.search.len().saturating_sub(1));
	let len = panel.items(&app.session).len();
	app.nixos.cursor = app.nixos.cursor.min(len.saturating_sub(1));
}

/// Shows `view`, reading the values of the settings it lists.
pub(crate) fn open_view(app: &mut App, tx: &mpsc::Sender<AppEvent>, view: View) {
	let panel = &mut app.nixos;
	panel.view = view;
	panel.nav = View::all()
		.iter()
		.position(|item| *item == view)
		.unwrap_or(0);
	panel.cursor = 0;
	panel.detail_scroll = 0;
	panel.details_open = false;
	panel.resolve_view();
	evaluate_view(app, tx);
}

/// Opens a search result: a curated setting on its page, anything else in
/// All options with the tree expanded to it.
fn open_result(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
	let Some(path) = app.nixos.current_path(&app.session) else {
		return;
	};
	let page = app
		.nixos
		.search
		.settings
		.get(app.nixos.result)
		.map(|(page, _)| *page);
	close_search(app);
	match page {
		Some(page) => open_view(app, tx, View::Page(page)),
		None => {
			open_view(app, tx, View::AllOptions);
			app.nixos.expanded.extend(ancestors(&path));
			refilter(app);
		}
	}
	let at = app
		.nixos
		.items(&app.session)
		.iter()
		.position(|item| match item {
			Item::Setting { path: shown, .. } => *shown == path,
			Item::Tree(TreeRow::Option { index, .. }) => {
				app.nixos.options.get(*index).map(|option| &option.path) == Some(&path)
			}
			_ => false,
		})
		.unwrap_or(0);
	app.nixos.cursor = at;
	app.nixos.focus = Focus::Content;
	if app.nixos.view == View::AllOptions {
		evaluate(app, tx, vec![path]);
	}
}

fn close_search(app: &mut App) {
	app.nixos_input = crate::vim::VimInput::default();
	app.nixos.focus = Focus::Content;
	app.nixos.result = 0;
	refilter(app);
}

/// Stages `value` for `path`, or an unset when `value` is `None`.
pub(crate) fn stage(app: &mut App, path: &[String], value: Option<SettingValue>) {
	let Some(option) = app.nixos.editing(path) else {
		app.status = "Wait for the value to load before editing it.".into();
		return;
	};
	if let Some(reason) = lock(app, option) {
		app.status = format!("{} is not editable: {reason}.", option_path_display(path));
		return;
	}
	let ours = app
		.session
		.engine
		.sets_option(Target::NixosSystem, &option.entry);
	let baseline = baseline(app, option);
	app.status = stage_edit(&mut app.nixos.staged, path, value, baseline.as_ref(), ours);
}

/// Toggles a boolean, or opens the editor for anything else.
fn edit_current(app: &mut App, tx: &mpsc::Sender<AppEvent>, toggle_only: bool) {
	let Some(path) = app.nixos.current_path(&app.session) else {
		return;
	};
	let Some(option) = app.nixos.editing(&path) else {
		evaluate(app, tx, vec![path]);
		app.status = "Reading the value…".into();
		return;
	};
	if let Some(reason) = lock(app, option) {
		app.status = format!("{} is not editable: {reason}.", option_path_display(&path));
		return;
	}
	let kind = option.entry.kind.clone();
	let value = current(app, option);
	let nullable = matches!(kind, OptionKind::Nullable(_));
	match kind.inner() {
		OptionKind::Bool if !nullable => {
			let on = matches!(value, Some(SettingValue::Bool(true)));
			stage(app, &path, Some(SettingValue::Bool(!on)));
		}
		_ if toggle_only => app.status = "Space toggles on/off settings; ↵ edits this one.".into(),
		OptionKind::Bool | OptionKind::Enum(_) => {
			let choices = choices(&kind);
			let cursor = choices
				.iter()
				.position(|choice| Some(choice) == value.as_ref())
				.unwrap_or(0);
			app.nixos.popup = Some(Popup::Edit(Edit {
				path,
				text: String::new(),
				choices,
				cursor,
				hint: "↑↓ choose  ↵ stage  esc cancel",
			}));
		}
		inner if kind.is_editable() => {
			let hint = match inner {
				OptionKind::StrList => "Comma-separated; empty for [ ].  ↵ stage  esc cancel",
				OptionKind::Int | OptionKind::Float if nullable => {
					"A number, or empty for null.  ↵ stage  esc cancel"
				}
				OptionKind::Int | OptionKind::Float => "A number.  ↵ stage  esc cancel",
				_ if nullable => "Empty for null.  ↵ stage  esc cancel",
				_ => "↵ stage  esc cancel",
			};
			app.nixos.popup = Some(Popup::Edit(Edit {
				path,
				text: edit_text(value.as_ref()),
				choices: Vec::new(),
				cursor: 0,
				hint,
			}));
		}
		_ => app.status = "This type cannot be edited here.".into(),
	}
}

/// Queues every staged edit as one NixOS rebuild.
fn apply(app: &mut App) {
	if app.nixos.staged.is_empty() {
		app.status = "No staged changes to apply.".into();
		return;
	}
	let changes: Vec<OptionChange> = std::mem::take(&mut app.nixos.staged)
		.into_iter()
		.map(|(path, value)| OptionChange { path, value })
		.collect();
	let op = QueuedOp::SetOptions {
		changes,
		scope: Target::NixosSystem,
	};
	let label = op.label();
	app.nixos.popup = None;
	app.status = match app.session.enqueue(op) {
		Enqueued::Started(_) => {
			app.tab = Tab::Building;
			format!("Applying {label}...")
		}
		Enqueued::Queued => format!("Queued {label}."),
		Enqueued::NotWritten => format!("Could not write {label}; see the build log."),
		Enqueued::Duplicate => format!("{label} is already queued."),
	};
}

pub(crate) fn on_build_ended(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
	if !app.nixos.loaded {
		return;
	}
	app.nixos.abort();
	app.nixos.loaded = false;
	if app.tab == Tab::Nixos {
		reload(app, tx);
	}
}

fn move_cursor(app: &mut App, delta: i32) {
	let step = |at: usize, len: usize| {
		if len == 0 {
			0
		} else {
			(at as i64 + i64::from(delta)).rem_euclid(len as i64) as usize
		}
	};
	match app.nixos.focus {
		Focus::Pages => app.nixos.nav = step(app.nixos.nav, View::all().len()),
		Focus::Search => app.nixos.result = step(app.nixos.result, app.nixos.search.len()),
		Focus::Content => {
			let len = app.nixos.items(&app.session).len();
			app.nixos.cursor = step(app.nixos.cursor, len);
		}
	}
	app.nixos.detail_scroll = 0;
}

/// Folds the tree with ←/→ in All options: → opens a namespace, ← closes
/// it or moves to its parent. Returns false when the key is not a fold.
fn fold(app: &mut App, open: bool) -> bool {
	if app.nixos.view != View::AllOptions || app.nixos.focus != Focus::Content {
		return false;
	}
	let Some(row) = app.nixos.rows.get(app.nixos.cursor).cloned() else {
		return false;
	};
	match (row, open) {
		(
			TreeRow::Namespace {
				prefix, expanded, ..
			},
			true,
		) if !expanded => {
			app.nixos.expanded.insert(prefix);
		}
		(
			TreeRow::Namespace {
				prefix, expanded, ..
			},
			false,
		) if expanded => {
			app.nixos.expanded.remove(&prefix);
		}
		(row, false) if row.depth() > 0 => {
			// Close the parent namespace and land on it.
			let parent = app.nixos.rows[..app.nixos.cursor]
				.iter()
				.rposition(|above| {
					matches!(above, TreeRow::Namespace { .. }) && above.depth() < row.depth()
				});
			if let Some(at) = parent {
				if let TreeRow::Namespace { prefix, .. } = &app.nixos.rows[at] {
					app.nixos.expanded.remove(prefix);
				}
				app.nixos.cursor = at;
			}
		}
		(_, false) => return false,
		_ => return true,
	}
	refilter(app);
	true
}

fn handle_popup(app: &mut App, key: KeyEvent) {
	match app.nixos.popup.clone() {
		Some(Popup::Review) => match key.code {
			KeyCode::Enter | KeyCode::Char('a') => apply(app),
			KeyCode::Esc | KeyCode::Char('q') => app.nixos.popup = None,
			_ => {}
		},
		Some(Popup::Edit(mut edit)) => {
			match key.code {
				KeyCode::Esc => {
					app.nixos.popup = None;
					return;
				}
				KeyCode::Enter => {
					app.nixos.popup = None;
					let value = if edit.choices.is_empty() {
						let Some(option) = app.nixos.editing(&edit.path) else {
							return;
						};
						match parse_text(&option.entry.kind, &edit.text) {
							Ok(value) => value,
							Err(message) => {
								app.status = message;
								app.nixos.popup = Some(Popup::Edit(edit));
								return;
							}
						}
					} else {
						edit.choices[edit.cursor].clone()
					};
					stage(app, &edit.path, Some(value));
					return;
				}
				KeyCode::Up | KeyCode::Char('k') if !edit.choices.is_empty() => {
					edit.cursor = (edit.cursor + edit.choices.len() - 1) % edit.choices.len();
				}
				KeyCode::Down | KeyCode::Char('j') if !edit.choices.is_empty() => {
					edit.cursor = (edit.cursor + 1) % edit.choices.len();
				}
				KeyCode::Backspace if edit.choices.is_empty() => {
					edit.text.pop();
				}
				KeyCode::Char(c)
					if edit.choices.is_empty()
						&& !key.modifiers.contains(KeyModifiers::CONTROL) =>
				{
					edit.text.push(c);
				}
				_ => {}
			}
			app.nixos.popup = Some(Popup::Edit(edit));
		}
		None => {}
	}
}

/// Handles the tab's keys. Returns false for keys that belong to the rest
/// of the app: switching tabs, settings, and quitting from the pages pane.
pub(crate) fn handle_key(app: &mut App, tx: &mpsc::Sender<AppEvent>, key: KeyEvent) -> bool {
	if app.nixos.popup.is_some() {
		handle_popup(app, key);
		return true;
	}
	let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
	let alt = key.modifiers.contains(KeyModifiers::ALT);
	if matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
		|| (ctrl && key.code == KeyCode::Char('s'))
	{
		return false;
	}
	if app.nixos.focus == Focus::Search {
		match key.code {
			KeyCode::Esc => close_search(app),
			KeyCode::Enter => open_result(app, tx),
			KeyCode::Up => move_cursor(app, -1),
			KeyCode::Down => move_cursor(app, 1),
			_ => {
				app.nixos_input.enter_insert_end();
				if app.nixos_input.handle_insert_event(&CtEvent::Key(key)) {
					app.nixos.result = 0;
					refilter(app);
				}
			}
		}
		return true;
	}
	let content = app.nixos.focus == Focus::Content;
	match key.code {
		KeyCode::Esc if content => app.nixos.focus = Focus::Pages,
		KeyCode::Esc => return false,
		KeyCode::Char('/') => {
			app.nixos.focus = Focus::Search;
			app.nixos_input.enter_insert_end();
		}
		KeyCode::Char('k') if ctrl => {
			app.nixos.focus = Focus::Search;
			app.nixos_input.enter_insert_end();
		}
		KeyCode::Up | KeyCode::Char('k') => move_cursor(app, -1),
		KeyCode::Down | KeyCode::Char('j') => move_cursor(app, 1),
		KeyCode::Right | KeyCode::Char('l') if !content && !ctrl => {
			let view = View::all()[app.nixos.nav];
			if view != app.nixos.view {
				open_view(app, tx, view);
			}
			app.nixos.focus = Focus::Content;
		}
		KeyCode::Enter if !content => {
			let view = View::all()[app.nixos.nav];
			open_view(app, tx, view);
			app.nixos.focus = Focus::Content;
		}
		KeyCode::Right | KeyCode::Char('l') if !ctrl => {
			fold(app, true);
		}
		KeyCode::Left | KeyCode::Char('h') if content && !alt => {
			if !fold(app, false) {
				app.nixos.focus = Focus::Pages;
			}
		}
		KeyCode::Enter => match app.nixos.items(&app.session).get(app.nixos.cursor).cloned() {
			Some(Item::Page(page)) => open_view(app, tx, View::Page(page)),
			Some(Item::Tree(TreeRow::Namespace { prefix, .. })) => {
				if !app.nixos.expanded.remove(&prefix) {
					app.nixos.expanded.insert(prefix);
				}
				refilter(app);
			}
			Some(_) => edit_current(app, tx, false),
			None => {}
		},
		KeyCode::Char(' ') if content => edit_current(app, tx, true),
		KeyCode::Char('u') if content => {
			if let Some(path) = app.nixos.current_path(&app.session) {
				stage(app, &path, None);
			}
		}
		KeyCode::Char('d') if !ctrl => app.nixos.details_open = !app.nixos.details_open,
		KeyCode::Char('R') => {
			if app.nixos.staged.is_empty() {
				app.status = "Nothing is staged.".into();
			} else {
				app.nixos.popup = Some(Popup::Review);
			}
		}
		KeyCode::Char('a') if !alt => {
			if editable_host(app) {
				apply(app);
			}
		}
		KeyCode::Char('x') => {
			let count = app.nixos.staged.len();
			app.nixos.staged.clear();
			app.status = format!("Discarded {count} staged change(s).");
		}
		KeyCode::Char('c') if !ctrl => {
			app.nixos.category = if app.nixos.category == Category::Configured {
				Category::All
			} else {
				Category::Configured
			};
			refilter(app);
			app.status = if app.nixos.category == Category::Configured {
				"All options shows configured options only.".into()
			} else {
				"All options shows every option.".into()
			};
		}
		KeyCode::PageDown => {
			app.nixos.detail_scroll = app.nixos.detail_scroll.saturating_add(10);
		}
		KeyCode::PageUp => {
			app.nixos.detail_scroll = app.nixos.detail_scroll.saturating_sub(10);
		}
		KeyCode::Char('t') if ctrl => cycle_host(app, tx),
		KeyCode::Char('h') if alt => toggle_hardware(app),
		KeyCode::Char('a') if alt => {
			app.nixos.hardware.show_all = !app.nixos.hardware.show_all;
			refilter(app);
			app.status = if app.nixos.hardware.show_all {
				"Showing all hardware options.".into()
			} else {
				"Showing relevant, configured, and unknown hardware options.".into()
			};
		}
		KeyCode::Char('d') => {
			app.nixos.show_diagnostic = !app.nixos.show_diagnostic;
			refilter(app);
			app.status = if app.nixos.show_diagnostic {
				"Showing diagnostic options.".into()
			} else {
				"Hiding diagnostic options.".into()
			};
		}
		KeyCode::Char('l') if ctrl => reload(app, tx),
		KeyCode::Char('r') => reload(app, tx),
		_ => {}
	}
	true
}

fn toggle_hardware(app: &mut App) {
	let panel = &mut app.nixos;
	if panel.configuration.is_none() || panel.loading {
		return;
	}
	if panel.hardware.inventory.is_some() {
		panel.hardware.inventory = None;
		panel.hardware_configuration = None;
		app.status = "Hardware unlinked. All hardware remains possible for this host.".into();
	} else {
		panel.hardware_configuration = panel.configuration.clone();
		panel.hardware.inventory = Some(panel.local_hardware.clone());
		app.status = format!(
			"Using this machine's hardware for this host. {}",
			panel.local_hardware.summary()
		);
	}
	refilter(app);
}

fn cycle_host(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
	let hosts = &app.nixos.hosts;
	if hosts.is_empty() || app.nixos.loading {
		return;
	}
	let current = app
		.nixos
		.configuration
		.as_ref()
		.and_then(|configuration| hosts.iter().position(|host| *host == configuration.host));
	let next = current.map_or(0, |index| (index + 1) % hosts.len());
	if current == Some(next) {
		return;
	}
	let host = hosts[next].clone();
	select_host(app, tx, host);
}

/// The footer's key hints for the tab's current state.
pub(crate) fn hints(app: &App) -> &'static str {
	match (&app.nixos.popup, app.nixos.focus, app.nixos.view) {
		(Some(Popup::Review), _, _) => "↵ apply  esc back",
		(Some(Popup::Edit(_)), _, _) => "↵ stage  esc cancel",
		(None, Focus::Search, _) => "type to search  ↑↓ results  ↵ open  esc close",
		(None, Focus::Pages, _) => {
			"↑↓/j/k pages  ↵/→ open  / search  R review  a apply  ctrl-t host  tab tabs"
		}
		(None, Focus::Content, View::AllOptions) => {
			"←→ fold  ↵ edit  space toggle  c configured  alt-h hardware  ctrl-d diagnostic  esc pages"
		}
		(None, Focus::Content, View::Overview) => "↑↓ pages  ↵ open  / search  esc pages",
		(None, Focus::Content, _) => {
			"↵ edit  space toggle  u unset  d details  / search  R review  a apply  x discard  esc pages"
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use nixbox_config::Config;
	use nixbox_nix::Manifest;
	use nixbox_nix::options::OptionEntry;
	use nixbox_nix::system_options::{InspectedValue, SystemOptionDetails};

	fn path(raw: &str) -> OptionPath {
		raw.split('.').map(str::to_string).collect()
	}

	fn option(raw: &str) -> SystemOption {
		SystemOption {
			path: path(raw),
			configured: Some(false),
			description: Some(format!("{raw} description")),
			type_description: "boolean".into(),
			declarations: vec!["configuration.nix".into()],
			diagnostic: false,
			read_only: false,
			nested: false,
			template: false,
			children_unavailable: false,
			children_truncated: false,
		}
	}

	fn editable(raw: &str, kind: OptionKind, value: serde_json::Value) -> EditableOption {
		EditableOption {
			details: SystemOptionDetails {
				path: path(raw),
				value: InspectedValue::Available(value.clone()),
				default: None,
				example: None,
				definitions: Some(Vec::new()),
			},
			entry: OptionEntry {
				path: path(raw),
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
			},
		}
	}

	/// The tab on `host` with a small schema whose values are already read.
	fn loaded_app(host: &str) -> App {
		let mut app = App::new(
			Config::default(),
			Manifest::default(),
			Manifest::default(),
			Vec::new(),
		);
		app.tab = Tab::Nixos;
		app.nixos.loaded = true;
		app.nixos.hosts = vec![host.into()];
		app.nixos.configuration = Some(NixosConfiguration {
			root: app.session.config_dir(),
			host: host.into(),
		});
		app.nixos.set_options(vec![
			option("networking.hostName"),
			option("services.pipewire.enable"),
			option("time.timeZone"),
			option("custom.welcome"),
		]);
		let mut zone = editable(
			"time.timeZone",
			OptionKind::Nullable(Box::new(OptionKind::Str)),
			serde_json::json!("UTC"),
		);
		zone.entry.defined_in = vec!["configuration.nix".into()];
		app.nixos.evaluated = [
			editable(
				"networking.hostName",
				OptionKind::Str,
				serde_json::json!("nixos"),
			),
			editable(
				"services.pipewire.enable",
				OptionKind::Bool,
				serde_json::json!(false),
			),
			zone,
		]
		.into_iter()
		.map(|option| (option.entry.path.clone(), option))
		.collect();
		refilter(&mut app);
		app
	}

	fn press(app: &mut App, tx: &mpsc::Sender<AppEvent>, code: KeyCode) -> bool {
		handle_key(app, tx, KeyEvent::new(code, KeyModifiers::NONE))
	}

	#[tokio::test]
	async fn pages_edit_values_inline_and_queue_one_rebuild() {
		let (tx, _rx) = mpsc::channel(4);
		let mut app = loaded_app(NIXOS_HOST);
		let _cancel = app.session.fake_build(Target::HomeManager, "busy");
		assert_eq!(app.nixos.focus, Focus::Pages);
		press(&mut app, &tx, KeyCode::Down);
		press(&mut app, &tx, KeyCode::Enter);
		assert_eq!(app.nixos.view, View::Page(SettingsPage::General));
		assert_eq!(app.nixos.focus, Focus::Content);
		assert_eq!(
			app.nixos.items(&app.session).len(),
			2,
			"Only the settings this host declares are listed"
		);

		press(&mut app, &tx, KeyCode::Enter);
		let Some(Popup::Edit(edit)) = &app.nixos.popup else {
			panic!("expected the text editor");
		};
		assert_eq!(edit.text, "nixos");
		for _ in 0..5 {
			press(&mut app, &tx, KeyCode::Backspace);
		}
		for c in "desk".chars() {
			press(&mut app, &tx, KeyCode::Char(c));
		}
		press(&mut app, &tx, KeyCode::Enter);
		assert!(app.nixos.popup.is_none());
		assert_eq!(
			app.nixos.staged.get(&path("networking.hostName")),
			Some(&Some(SettingValue::Str("desk".into())))
		);

		press(&mut app, &tx, KeyCode::Down);
		press(&mut app, &tx, KeyCode::Enter);
		assert!(
			app.nixos.popup.is_none(),
			"Locked settings do not open an editor"
		);
		assert_eq!(app.nixos.staged.len(), 1);
		assert!(app.session.queue.is_empty(), "Editing only stages");

		press(&mut app, &tx, KeyCode::Char('R'));
		assert_eq!(app.nixos.popup, Some(Popup::Review));
		press(&mut app, &tx, KeyCode::Enter);
		assert!(app.nixos.staged.is_empty());
		assert!(matches!(
			app.session.queue.front(),
			Some(QueuedOp::SetOptions { changes, scope: Target::NixosSystem })
				if changes.len() == 1
		));
		assert_eq!(app.session.engine.config.target, Config::default().target);
	}

	#[tokio::test]
	async fn space_toggles_booleans_and_unset_drops_the_edit() {
		let (tx, _rx) = mpsc::channel(4);
		let mut app = loaded_app(NIXOS_HOST);
		open_view(&mut app, &tx, View::Page(SettingsPage::Audio));
		app.nixos.focus = Focus::Content;
		press(&mut app, &tx, KeyCode::Char(' '));
		assert_eq!(
			app.nixos.staged.get(&path("services.pipewire.enable")),
			Some(&Some(SettingValue::Bool(true)))
		);
		assert!(
			!press(&mut app, &tx, KeyCode::Tab),
			"Tab still switches tabs"
		);
		press(&mut app, &tx, KeyCode::Char('u'));
		assert!(app.nixos.staged.is_empty());
		press(&mut app, &tx, KeyCode::Char(' '));
		press(&mut app, &tx, KeyCode::Char('x'));
		assert!(app.nixos.staged.is_empty(), "x discards every staged edit");
	}

	#[tokio::test]
	async fn other_hosts_are_read_only() {
		let (tx, _rx) = mpsc::channel(4);
		let mut app = loaded_app("workstation");
		open_view(&mut app, &tx, View::Page(SettingsPage::Audio));
		app.nixos.focus = Focus::Content;
		press(&mut app, &tx, KeyCode::Char(' '));
		assert!(app.nixos.staged.is_empty());
		assert!(app.status.contains("host not rebuilt by nixbox"));
	}

	#[tokio::test]
	async fn search_opens_curated_settings_and_reveals_options_in_the_tree() {
		let (tx, _rx) = mpsc::channel(4);
		let mut app = loaded_app(NIXOS_HOST);
		press(&mut app, &tx, KeyCode::Char('/'));
		assert_eq!(app.nixos.focus, Focus::Search);
		for c in "welcome".chars() {
			press(&mut app, &tx, KeyCode::Char(c));
		}
		assert_eq!(app.nixos.search.options.len(), 1);
		press(&mut app, &tx, KeyCode::Enter);
		assert_eq!(app.nixos.view, View::AllOptions);
		assert!(
			app.nixos_input.value().is_empty(),
			"Opening a result clears the search"
		);
		assert_eq!(
			app.nixos.current_path(&app.session),
			Some(path("custom.welcome"))
		);

		press(&mut app, &tx, KeyCode::Char('/'));
		for c in "pipewire".chars() {
			press(&mut app, &tx, KeyCode::Char(c));
		}
		assert_eq!(
			app.nixos.search.settings.len(),
			1,
			"Curated settings come first"
		);
		press(&mut app, &tx, KeyCode::Enter);
		assert_eq!(app.nixos.view, View::Page(SettingsPage::Audio));
		assert_eq!(
			app.nixos.current_path(&app.session),
			Some(path("services.pipewire.enable"))
		);
	}

	#[tokio::test]
	async fn the_tree_folds_with_arrows_and_filters_configured_options() {
		let (tx, _rx) = mpsc::channel(4);
		let mut app = loaded_app(NIXOS_HOST);
		app.nixos.options[1].configured = Some(true);
		open_view(&mut app, &tx, View::AllOptions);
		app.nixos.focus = Focus::Content;
		refilter(&mut app);
		let top = app.nixos.rows.len();
		assert_eq!(
			top, 4,
			"Single-option namespaces collapse into their option"
		);
		press(&mut app, &tx, KeyCode::Char('c'));
		assert_eq!(app.nixos.visible, [1]);
		press(&mut app, &tx, KeyCode::Char('c'));
		app.nixos
			.options
			.push(option("services.pipewire.pulse.enable"));
		app.nixos.index = SchemaIndex::new(&app.nixos.options);
		refilter(&mut app);
		let services = app
			.nixos
			.rows
			.iter()
			.position(|row| matches!(row, TreeRow::Namespace { .. }))
			.expect("services namespace");
		app.nixos.cursor = services;
		press(&mut app, &tx, KeyCode::Right);
		assert_eq!(
			app.nixos.rows.len(),
			top + 1,
			"services opens onto its pipewire namespace"
		);
		press(&mut app, &tx, KeyCode::Down);
		press(&mut app, &tx, KeyCode::Left);
		assert_eq!(
			app.nixos.cursor, services,
			"← from a child closes its parent"
		);
		assert_eq!(app.nixos.rows.len(), top);
		press(&mut app, &tx, KeyCode::Left);
		assert_eq!(app.nixos.focus, Focus::Pages);
	}

	#[tokio::test]
	async fn stale_values_are_dropped_and_rebuilds_invalidate_the_schema() {
		let (tx, _rx) = mpsc::channel(4);
		let mut app = loaded_app(NIXOS_HOST);
		app.nixos.evaluated.clear();
		let epoch = app.nixos.detail_epoch;
		let group = || {
			Ok(vec![editable(
				"networking.hostName",
				OptionKind::Str,
				serde_json::json!("nixos"),
			)])
		};
		on_event(
			&mut app,
			&tx,
			Event::Group {
				epoch: epoch.wrapping_add(1),
				result: group(),
			},
		);
		assert!(app.nixos.evaluated.is_empty(), "Stale results are dropped");
		on_event(
			&mut app,
			&tx,
			Event::Group {
				epoch,
				result: group(),
			},
		);
		assert!(app.nixos.editing(&path("networking.hostName")).is_some());

		app.tab = Tab::Search;
		app.nixos.loading = true;
		on_build_ended(&mut app, &tx);
		assert!(!app.nixos.loaded, "Revisiting the tab reloads the schema");
		assert!(!app.nixos.loading, "An aborted load stops the spinner");
	}

	#[tokio::test]
	async fn hardware_controls_filter_and_unlink_on_host_changes() {
		use nixbox_nix::hardware::Presence;
		let (tx, _rx) = mpsc::channel(4);
		let mut app = loaded_app("workstation");
		app.nixos.options.push(option("hardware.nvidia.open"));
		app.nixos.index = SchemaIndex::new(&app.nixos.options);
		app.nixos.local_hardware.nvidia_gpu = Presence::Absent;
		refilter(&mut app);
		assert_eq!(app.nixos.visible.len(), 5);
		let alt = |code| KeyEvent::new(code, KeyModifiers::ALT);
		handle_key(&mut app, &tx, alt(KeyCode::Char('h')));
		assert_eq!(app.nixos.visible.len(), 4);
		handle_key(&mut app, &tx, alt(KeyCode::Char('a')));
		assert_eq!(app.nixos.visible.len(), 5);
		handle_key(&mut app, &tx, alt(KeyCode::Char('a')));
		assert_eq!(app.nixos.visible.len(), 4);
		app.nixos.hosts.push("other".into());
		select_host(&mut app, &tx, "other".into());
		app.nixos.abort();
		assert!(app.nixos.hardware.inventory.is_none());
		assert!(app.nixos.hardware_configuration.is_none());
	}
}
