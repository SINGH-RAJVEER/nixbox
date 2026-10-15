//! The NixOS tab: a read-only browser over the selected host's option
//! schema. It loads on first visit, inspects an option only on request, and
//! never changes the package install target.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nixbox_config::InputMode;
use nixbox_core::system_options::{Category, Filtered, default_host, filter_options};
use nixbox_nix::system_options::{
	NixosConfiguration, SystemOption, SystemOptionDetails, discover_hosts, discover_options,
	inspect_option,
};
use tokio::{sync::mpsc, task::JoinHandle};

use crate::app::{App, AppEvent, Tab};
use crate::vim::VimMode;

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
		result: Result<Vec<SystemOption>, String>,
	},
	Details {
		epoch: u64,
		result: Result<SystemOptionDetails, String>,
	},
}

#[derive(Default)]
pub(crate) struct Panel {
	pub(crate) loaded: bool,
	pub(crate) loading: bool,
	pub(crate) hosts: Vec<String>,
	pub(crate) configuration: Option<NixosConfiguration>,
	pub(crate) options: Vec<SystemOption>,
	pub(crate) filtered: Filtered,
	pub(crate) category: Category,
	pub(crate) show_diagnostic: bool,
	/// Position in `filtered.visible`.
	pub(crate) cursor: usize,
	pub(crate) details: Option<SystemOptionDetails>,
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
	/// The option under the cursor.
	pub(crate) fn current(&self) -> Option<&SystemOption> {
		self.options.get(*self.filtered.visible.get(self.cursor)?)
	}

	/// The inspection of the option under the cursor, once it has arrived.
	pub(crate) fn current_details(&self) -> Option<&SystemOptionDetails> {
		let option = self.current()?;
		self.details
			.as_ref()
			.filter(|details| details.path == option.path)
	}

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
		self.details = None;
		self.detail_error = None;
		self.detail_scroll = 0;
	}

	fn clear_schema(&mut self) {
		self.options.clear();
		self.filtered = Filtered::default();
		self.cursor = 0;
		self.error = None;
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
	let configuration = NixosConfiguration {
		root: app.session.config_dir(),
		host,
	};
	panel.configuration = Some(configuration.clone());
	panel.loading = true;
	let epoch = panel.catalog_epoch;
	let tx = tx.clone();
	app.status = format!("Reading the option schema of {}…", configuration.host);
	app.nixos.catalog_task = Some(tokio::spawn(async move {
		let result = discover_options(&configuration, EVALUATION_LIMIT)
			.await
			.map(|catalog| catalog.options)
			.map_err(|error| format!("{error:#}"));
		let _ = tx
			.send(AppEvent::Nixos(Event::Catalog { epoch, result }))
			.await;
	}));
}

/// Evaluates the value and definitions of the option under the cursor.
fn inspect(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
	let Some(configuration) = app.nixos.configuration.clone() else {
		return;
	};
	let Some(path) = app.nixos.current().map(|option| option.path.clone()) else {
		return;
	};
	app.nixos.abort_inspection();
	app.nixos.inspecting = true;
	let epoch = app.nixos.detail_epoch;
	let tx = tx.clone();
	app.nixos.detail_task = Some(tokio::spawn(async move {
		let result = inspect_option(&configuration, &path, EVALUATION_LIMIT)
			.await
			.map_err(|error| format!("{error:#}"));
		let _ = tx
			.send(AppEvent::Nixos(Event::Details { epoch, result }))
			.await;
	}));
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
				Ok(options) => {
					app.nixos.options = options;
					refilter(app);
					app.status = format!("{} NixOS options.", app.nixos.options.len());
				}
				Err(error) => {
					app.status = "Could not read the NixOS schema.".into();
					app.nixos.error = Some(error);
				}
			}
		}
		Event::Details { epoch, result } => {
			if epoch != app.nixos.detail_epoch {
				return;
			}
			app.nixos.detail_task = None;
			app.nixos.inspecting = false;
			match result {
				Ok(details) => app.nixos.details = Some(details),
				Err(error) => app.nixos.detail_error = Some(error),
			}
		}
	}
}

/// Reapplies the search, category, and diagnostic filter, keeping the cursor
/// on the same option when it is still visible.
pub(crate) fn refilter(app: &mut App) {
	let panel = &mut app.nixos;
	let current = panel.current().map(|option| option.path.clone());
	panel.filtered = filter_options(
		&panel.options,
		app.nixos_input.value(),
		panel.category,
		panel.show_diagnostic,
	);
	panel.cursor = current
		.and_then(|path| {
			panel
				.filtered
				.visible
				.iter()
				.position(|index| panel.options[*index].path == path)
		})
		.unwrap_or(0);
}

pub(crate) fn move_cursor(app: &mut App, delta: i32) {
	let panel = &mut app.nixos;
	let len = panel.filtered.visible.len();
	if len == 0 {
		return;
	}
	panel.cursor = (panel.cursor as i64 + i64::from(delta)).rem_euclid(len as i64) as usize;
	panel.detail_scroll = 0;
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

/// Handles the tab's own keys. Returns false for keys that belong to the
/// search input or the rest of the app.
pub(crate) fn handle_key(app: &mut App, tx: &mpsc::Sender<AppEvent>, key: KeyEvent) -> bool {
	let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
	let alt = key.modifiers.contains(KeyModifiers::ALT);
	let normal = app.session.engine.config.input_mode == InputMode::Vim
		&& app.nixos_input.mode() == VimMode::Normal;
	match key.code {
		KeyCode::Enter if app.nixos_input.mode() != VimMode::Visual => inspect(app, tx),
		KeyCode::PageDown => {
			app.nixos.detail_scroll = app.nixos.detail_scroll.saturating_add(10);
		}
		KeyCode::PageUp => {
			app.nixos.detail_scroll = app.nixos.detail_scroll.saturating_sub(10);
		}
		KeyCode::Right if alt => cycle_category(app, 1),
		KeyCode::Left if alt => cycle_category(app, -1),
		KeyCode::Char(']') if normal => cycle_category(app, 1),
		KeyCode::Char('[') if normal => cycle_category(app, -1),
		KeyCode::Char('t') if ctrl => cycle_host(app, tx),
		KeyCode::Char('d') if ctrl => {
			app.nixos.show_diagnostic = !app.nixos.show_diagnostic;
			refilter(app);
			app.status = if app.nixos.show_diagnostic {
				"Showing diagnostic options.".into()
			} else {
				"Hiding diagnostic options.".into()
			};
		}
		KeyCode::Char('l') if ctrl => reload(app, tx),
		KeyCode::Char('r') if normal => reload(app, tx),
		_ => return false,
	}
	true
}

fn cycle_category(app: &mut App, delta: isize) {
	app.nixos.category = app.nixos.category.cycle(delta);
	refilter(app);
	app.status = format!("Category: {}.", app.nixos.category.label());
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

#[cfg(test)]
mod tests {
	use super::*;
	use nixbox_config::Config;
	use nixbox_nix::Manifest;
	use nixbox_nix::system_options::InspectedValue;

	fn option(path: &str) -> SystemOption {
		SystemOption {
			path: path.split('.').map(str::to_string).collect(),
			description: Some(format!("{path} description")),
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

	fn loaded_app() -> App {
		let mut app = App::new(
			Config::default(),
			Manifest::default(),
			Manifest::default(),
			Vec::new(),
		);
		app.nixos.loaded = true;
		app.nixos.hosts = vec!["workstation".into()];
		app.nixos.configuration = Some(NixosConfiguration {
			root: app.session.config_dir(),
			host: "workstation".into(),
		});
		app.nixos.options = vec![
			option("networking.hostName"),
			option("services.pipewire.enable"),
			option("custom.welcome"),
		];
		refilter(&mut app);
		app
	}

	fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
		KeyEvent::new(code, modifiers)
	}

	#[tokio::test]
	async fn categories_and_search_keep_the_cursor_on_its_option() {
		let (tx, _rx) = mpsc::channel(4);
		let mut app = loaded_app();
		move_cursor(&mut app, 2);
		assert_eq!(app.nixos.current().unwrap().path, ["custom", "welcome"]);

		assert!(handle_key(
			&mut app,
			&tx,
			key(KeyCode::Right, KeyModifiers::ALT)
		));
		assert_eq!(app.nixos.category, Category::General);
		assert_eq!(app.nixos.filtered.visible, [0]);
		assert_eq!(app.nixos.filtered.count(Category::All), 3);

		app.nixos.category = Category::All;
		refilter(&mut app);
		move_cursor(&mut app, 1);
		assert_eq!(
			app.nixos.current().unwrap().path,
			["services", "pipewire", "enable"]
		);
		app.nixos_input = crate::vim::VimInput::default();
		refilter(&mut app);
		assert_eq!(
			app.nixos.current().unwrap().path,
			["services", "pipewire", "enable"],
			"Refiltering keeps the selected option"
		);
		assert_eq!(app.session.engine.config.target, Config::default().target);
		assert!(
			app.session.queue.is_empty(),
			"The browser never queues work"
		);
	}

	#[tokio::test]
	async fn stale_and_foreign_inspections_are_not_shown() {
		let (tx, _rx) = mpsc::channel(4);
		let mut app = loaded_app();
		let epoch = app.nixos.detail_epoch;
		let details = |path: &str| SystemOptionDetails {
			path: path.split('.').map(str::to_string).collect(),
			value: InspectedValue::Available(serde_json::Value::Bool(true)),
			default: None,
			example: None,
			definitions: Some(Vec::new()),
		};
		on_event(
			&mut app,
			&tx,
			Event::Details {
				epoch: epoch.wrapping_add(1),
				result: Ok(details("networking.hostName")),
			},
		);
		assert!(app.nixos.details.is_none(), "Stale results are dropped");

		on_event(
			&mut app,
			&tx,
			Event::Details {
				epoch,
				result: Ok(details("networking.hostName")),
			},
		);
		assert!(app.nixos.current_details().is_some());
		move_cursor(&mut app, 1);
		assert!(
			app.nixos.current_details().is_none(),
			"Details follow the option they describe"
		);
	}

	#[tokio::test]
	async fn rebuilds_invalidate_the_schema() {
		let (tx, _rx) = mpsc::channel(4);
		let mut app = loaded_app();
		app.nixos.loading = true;
		on_build_ended(&mut app, &tx);
		assert!(!app.nixos.loaded, "Revisiting the tab reloads the schema");
		assert_eq!(app.nixos.options.len(), 3, "Away from the tab nothing runs");
		assert!(!app.nixos.loading, "An aborted load stops the spinner");
	}
}
