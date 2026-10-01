use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::cursor::SetCursorStyle;
use crossterm::event::EventStream;
use crossterm::execute;
use crossterm::terminal::{
	EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures::StreamExt;
use nixbox_config::{DEFAULT_CHANNEL, InputMode, Target};
use nixbox_core::{Engine, InstalledFlake, ManagedPackage, Session};
use nixbox_nix::{
	Manifest,
	build::BuildEvent,
	flakes::{FlakeDetails, FlakeHit},
	options::OptionSet,
	scan::ExternalPackage,
	search::{PackageCatalog, SearchHit},
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::{sync::mpsc, task::JoinHandle};

use crate::handlers::{handle_app_event, handle_terminal_event};
use crate::ops::prepare_package_catalog;
use crate::options::{OptionsPanel, PanelKey};
use crate::state;
use crate::theme;
use crate::ui;
use crate::vim::VimInput;

pub(crate) const CHANNELS: &[&str] = &[DEFAULT_CHANNEL, "nixpkgs-unstable"];
pub(crate) const INPUT_MODES: &[InputMode] = &[InputMode::Vim, InputMode::Normal];
pub(crate) const TARGETS: &[Target] = &[Target::HomeManager, Target::NixosSystem];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mode {
	Browsing,
	SettingsSelect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsPage {
	Main,
	InputMode,
	Theme,
	Target,
	Channel,
}

/// The queue holds core ops verbatim, so what the TUI schedules is exactly
/// what the engine applies — and what `state.json` round-trips.
pub(crate) use nixbox_core::Op as QueuedOp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
	Search,
	Flakes,
	Installed,
	Vcs,
	Building,
	Queue,
}

impl Tab {
	pub(crate) fn label(self) -> &'static str {
		match self {
			Tab::Search => "nixpkgs",
			Tab::Flakes => "Flakes",
			Tab::Installed => "Installed",
			Tab::Vcs => "VCS",
			Tab::Building => "Building",
			Tab::Queue => "Queue",
		}
	}
}

#[derive(Debug, Clone)]
pub(crate) enum AppEvent {
	Repository(std::result::Result<crate::repository::Outcome, String>),
	SearchDone {
		epoch: u64,
		hits: Vec<SearchHit>,
	},
	SearchFailed {
		epoch: u64,
		error: String,
	},
	CatalogReady(Arc<PackageCatalog>),
	CatalogFailed(String),
	FlakeSearchDone {
		epoch: u64,
		hits: Vec<FlakeHit>,
	},
	FlakeSearchFailed {
		epoch: u64,
		error: String,
	},
	FlakeDetailsDone {
		epoch: u64,
		details: Box<FlakeDetails>,
	},
	FlakeDetailsFailed {
		epoch: u64,
		error: String,
	},
	OptionsLoaded {
		epoch: u64,
		key: PanelKey,
		set: Arc<OptionSet>,
	},
	OptionsFailed {
		epoch: u64,
		error: String,
	},
	Build(BuildEvent),
}

/// The row currently under the cursor in the Installed tab.
#[derive(Debug, Clone)]
pub(crate) enum InstalledCursor {
	Managed(ManagedPackage),
	Flake(InstalledFlake),
	External(ExternalPackage),
}

pub(crate) struct App {
	pub(crate) repository: crate::repository::Panel,
	/// The engine, the op queue, the running rebuild, and its log.
	pub(crate) session: Session,
	pub(crate) input: VimInput,
	pub(crate) results: Vec<SearchHit>,
	pub(crate) selected: usize,
	pub(crate) flake_input: VimInput,
	pub(crate) flake_results: Vec<FlakeHit>,
	pub(crate) flake_selected: usize,
	pub(crate) flake_details: Option<FlakeDetails>,
	pub(crate) flake_details_scroll: u16,
	pub(crate) flake_picker: Option<usize>,
	pub(crate) flake_search_epoch: u64,
	pub(crate) flake_detail_epoch: u64,
	pub(crate) flake_query: String,
	pub(crate) flake_searching: bool,
	pub(crate) flake_detail_loading: bool,
	pub(crate) flake_search_task: Option<JoinHandle<()>>,
	pub(crate) flake_detail_task: Option<JoinHandle<()>>,
	pub(crate) installed_input: VimInput,
	pub(crate) installed_selected: usize,
	pub(crate) status: String,
	pub(crate) mode: Mode,
	pub(crate) tab: Tab,
	pub(crate) search_epoch: u64,
	pub(crate) latest_query: String,
	pub(crate) should_quit: bool,
	pub(crate) theme_index: usize,
	pub(crate) settings_page: SettingsPage,
	pub(crate) settings_cursor: usize,
	pub(crate) searching: bool,
	pub(crate) search_task: Option<JoinHandle<()>>,
	pub(crate) package_catalog: Option<Arc<PackageCatalog>>,
	pub(crate) catalog_loading: bool,
	pub(crate) catalog_task: Option<JoinHandle<()>>,
	pub(crate) spinner_frame: usize,
	/// The package options panel, open over the Installed tab.
	pub(crate) options_panel: Option<OptionsPanel>,
	pub(crate) options_cache: HashMap<PanelKey, Arc<OptionSet>>,
	pub(crate) options_epoch: u64,
	pub(crate) options_task: Option<JoinHandle<()>>,
}

impl App {
	/// Builds an app around an engine assembled by the caller. Only tests
	/// use it directly; `run` goes through [`Engine::load`].
	#[cfg(test)]
	pub(crate) fn new(
		config: nixbox_config::Config,
		home_manifest: Manifest,
		nixos_manifest: Manifest,
		external_packages: Vec<ExternalPackage>,
	) -> Self {
		let (session, _build_rx) = Session::new(Engine::from_parts(
			config,
			home_manifest,
			nixos_manifest,
			external_packages,
		));
		Self::from_session(session.without_persistence())
	}

	pub(crate) fn from_session(session: Session) -> Self {
		let engine = &session.engine;
		let managed = engine.home_manifest.packages.len() + engine.nixos_manifest.packages.len();
		let external = engine.external_packages.len();
		let theme_index = theme::ALL
			.iter()
			.position(|t| t.name == engine.config.theme)
			.unwrap_or(0);
		let status = if external == 0 {
			format!("{} packages tracked.", managed)
		} else {
			format!("{} managed  ·  {} external.", managed, external)
		};
		let input_mode = engine.config.input_mode;
		let mut input = VimInput::default();
		let mut flake_input = VimInput::default();
		let mut installed_input = VimInput::default();
		if input_mode == InputMode::Normal {
			input.enter_insert_before();
			flake_input.enter_insert_before();
			installed_input.enter_insert_before();
		}
		Self {
			repository: crate::repository::Panel::default(),
			session,
			input,
			results: Vec::new(),
			selected: 0,
			flake_input,
			flake_results: Vec::new(),
			flake_selected: 0,
			flake_details: None,
			flake_picker: None,
			flake_details_scroll: 0,
			flake_search_epoch: 0,
			flake_detail_epoch: 0,
			flake_query: String::new(),
			flake_searching: false,
			flake_detail_loading: false,
			flake_search_task: None,
			flake_detail_task: None,
			installed_input,
			installed_selected: 0,
			status,
			mode: Mode::Browsing,
			tab: Tab::Search,
			search_epoch: 0,
			latest_query: String::new(),
			should_quit: false,
			theme_index,
			settings_page: SettingsPage::Main,
			settings_cursor: 0,
			searching: false,
			search_task: None,
			package_catalog: None,
			catalog_loading: false,
			catalog_task: None,
			spinner_frame: 0,
			options_panel: None,
			options_cache: HashMap::new(),
			options_epoch: 0,
			options_task: None,
		}
	}

	pub(crate) fn visible_tabs(&self) -> Vec<Tab> {
		let mut tabs = vec![Tab::Search, Tab::Flakes, Tab::Installed, Tab::Vcs];
		if self.session.is_building() || !self.session.log.is_empty() {
			tabs.push(Tab::Building);
		}
		if !self.session.queue.is_empty() {
			tabs.push(Tab::Queue);
		}
		tabs
	}

	pub(crate) fn channel(&self) -> &str {
		&self.session.engine.config.channel
	}

	pub(crate) fn target_label(&self) -> &'static str {
		self.session.engine.config.target.label()
	}

	pub(crate) fn theme(&self) -> &'static theme::Theme {
		let idx = if matches!(self.mode, Mode::SettingsSelect)
			&& self.settings_page == SettingsPage::Theme
		{
			self.settings_cursor
		} else {
			self.theme_index
		};
		&theme::ALL[idx]
	}

	pub(crate) fn apply_input_mode(&mut self, mode: InputMode) {
		self.session.engine.config.input_mode = mode;
		match mode {
			InputMode::Vim => {
				self.input.enter_normal();
				self.installed_input.enter_normal();
			}
			InputMode::Normal => {
				self.input.enter_insert_before();
				self.installed_input.enter_insert_before();
			}
		}
	}

	pub(crate) fn manifest_for(&self, scope: Target) -> &Manifest {
		self.session.engine.manifest_for(scope)
	}

	/// Returns all managed packages from both scopes, home-manager entries
	/// first.
	pub(crate) fn managed_packages(&self) -> Vec<ManagedPackage> {
		self.session.engine.managed_packages()
	}

	pub(crate) fn installed_filter(&self) -> Option<String> {
		let q = self.installed_input.value().trim().to_lowercase();
		if q.is_empty() { None } else { Some(q) }
	}

	pub(crate) fn filtered_managed_packages(&self) -> Vec<ManagedPackage> {
		let filter = self.installed_filter();
		self.managed_packages()
			.into_iter()
			.filter(|p| match &filter {
				None => true,
				Some(q) => p.name.to_lowercase().contains(q),
			})
			.collect()
	}

	pub(crate) fn filtered_external_packages(&self) -> Vec<ExternalPackage> {
		let filter = self.installed_filter();
		self.session
			.engine
			.external_packages
			.iter()
			.filter(|ep| match &filter {
				None => true,
				Some(q) => ep.name.to_lowercase().contains(q),
			})
			.cloned()
			.collect()
	}

	/// Flake outputs matching the filter by input, output, or repository.
	pub(crate) fn filtered_flakes(&self) -> Vec<InstalledFlake> {
		let filter = self.installed_filter();
		self.session
			.engine
			.flakes
			.iter()
			.filter(|flake| match &filter {
				None => true,
				Some(q) => {
					flake.name().to_lowercase().contains(q)
						|| flake
							.repo
							.as_ref()
							.is_some_and(|repo| repo.to_lowercase().contains(q))
				}
			})
			.cloned()
			.collect()
	}

	pub(crate) fn installed_total(&self) -> usize {
		self.filtered_managed_packages().len()
			+ self.filtered_flakes().len()
			+ self.filtered_external_packages().len()
	}

	/// Returns the row currently under the cursor in the Installed tab.
	pub(crate) fn installed_cursor(&self) -> Option<InstalledCursor> {
		let managed = self.filtered_managed_packages();
		let flakes = self.filtered_flakes();
		let external = self.filtered_external_packages();
		let total = managed.len() + flakes.len() + external.len();
		if total == 0 {
			return None;
		}
		let idx = self.installed_selected.min(total - 1);
		if idx < managed.len() {
			Some(InstalledCursor::Managed(managed[idx].clone()))
		} else if idx < managed.len() + flakes.len() {
			Some(InstalledCursor::Flake(flakes[idx - managed.len()].clone()))
		} else {
			Some(InstalledCursor::External(
				external[idx - managed.len() - flakes.len()].clone(),
			))
		}
	}

	/// Whether the options panel is waiting on an evaluation.
	pub(crate) fn options_loading(&self) -> bool {
		self.options_panel
			.as_ref()
			.is_some_and(|panel| panel.loading)
	}

	/// Whether keys and drawing go to the options panel.
	pub(crate) fn options_panel_active(&self) -> bool {
		self.tab == Tab::Installed && self.options_panel.is_some()
	}

	/// Keeps the Installed cursor inside the list after it changes size.
	pub(crate) fn clamp_installed_selection(&mut self) {
		let total = self.installed_total();
		if total == 0 {
			self.installed_selected = 0;
		} else if self.installed_selected >= total {
			self.installed_selected = total - 1;
		}
	}
}

pub async fn run() -> Result<()> {
	let engine = Engine::load()?;

	enable_raw_mode()?;
	let mut stdout = io::stdout();
	execute!(stdout, EnterAlternateScreen, SetCursorStyle::BlinkingBar)?;
	let backend = CrosstermBackend::new(stdout);
	let mut terminal = Terminal::new(backend)?;

	let result = event_loop(&mut terminal, engine).await;

	disable_raw_mode()?;
	execute!(
		terminal.backend_mut(),
		SetCursorStyle::DefaultUserShape,
		LeaveAlternateScreen
	)?;
	terminal.show_cursor()?;

	result
}

async fn event_loop(
	terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
	engine: Engine,
) -> Result<()> {
	let (session, mut build_rx) = Session::new(engine);
	let mut app = App::from_session(session);
	let (tx, mut rx) = mpsc::channel::<AppEvent>(128);
	state::restore(&mut app);
	prepare_package_catalog(&mut app, tx.clone());
	let mut term_events = EventStream::new();
	let mut spinner_tick = tokio::time::interval(Duration::from_millis(80));
	spinner_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

	loop {
		terminal.draw(|f| ui::draw(f, &app))?;
		if app.should_quit {
			if let Some(task) = app.search_task.take() {
				task.abort();
			}
			if let Some(task) = app.flake_search_task.take() {
				task.abort();
			}
			if let Some(task) = app.flake_detail_task.take() {
				task.abort();
			}
			if let Some(task) = app.catalog_task.take() {
				task.abort();
			}
			if let Some(task) = app.options_task.take() {
				task.abort();
			}
			break;
		}

		tokio::select! {
			Some(Ok(ev)) = term_events.next() => {
				handle_terminal_event(&mut app, &tx, ev).await?;
			}
			Some(app_ev) = rx.recv() => {
				handle_app_event(&mut app, &tx, app_ev);
			}
			Some(build_ev) = build_rx.recv() => {
				handle_app_event(&mut app, &tx, AppEvent::Build(build_ev));
			}
			_ = spinner_tick.tick(), if app.searching || app.catalog_loading || app.flake_searching || app.flake_detail_loading || app.options_loading() || app.session.is_building() => {
				app.spinner_frame = app.spinner_frame.wrapping_add(1);
			}
		}
	}
	if let Some(handle) = app.search_task.take() {
		handle.abort();
	}
	if let Some(handle) = app.flake_search_task.take() {
		handle.abort();
	}
	if let Some(handle) = app.flake_detail_task.take() {
		handle.abort();
	}
	if let Some(handle) = app.catalog_task.take() {
		handle.abort();
	}
	if let Some(handle) = app.options_task.take() {
		handle.abort();
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::handlers::handle_app_event;
	use nixbox_config::{Config, Target};
	use tokio::sync::mpsc;

	fn hit(name: &str) -> SearchHit {
		SearchHit {
			attr: name.into(),
			pname: name.into(),
			version: "1.0".into(),
			description: String::new(),
		}
	}

	fn test_app() -> App {
		App::new(
			Config::default(),
			Manifest::default(),
			Manifest::default(),
			Vec::new(),
		)
	}

	#[test]
	fn app_initializes_with_expected_defaults() {
		let app = test_app();

		assert_eq!(app.tab, Tab::Search);
		assert_eq!(app.mode, Mode::Browsing);
		assert_eq!(app.search_epoch, 0);
		assert!(!app.searching);
		assert!(app.results.is_empty());
		assert_eq!(app.manifest_for(Target::HomeManager).packages.len(), 0);
		assert_eq!(app.manifest_for(Target::NixosSystem).packages.len(), 0);
		assert_eq!(
			app.visible_tabs(),
			vec![Tab::Search, Tab::Flakes, Tab::Installed, Tab::Vcs]
		);
	}

	#[test]
	fn normal_input_preference_starts_all_inputs_in_insert_mode() {
		let app = App::new(
			Config {
				input_mode: InputMode::Normal,
				..Config::default()
			},
			Manifest::default(),
			Manifest::default(),
			Vec::new(),
		);

		assert_eq!(app.input.mode(), crate::vim::VimMode::Insert);
		assert_eq!(app.flake_input.mode(), crate::vim::VimMode::Insert);
		assert_eq!(app.installed_input.mode(), crate::vim::VimMode::Insert);
		assert_eq!(app.settings_page, SettingsPage::Main);
		assert_eq!(app.settings_cursor, 0);
	}

	#[test]
	fn visible_tabs_include_build_and_queue_only_when_needed() {
		let mut app = test_app();
		let _cancel = app.session.fake_build(Target::HomeManager, "build");
		assert_eq!(
			app.visible_tabs(),
			vec![
				Tab::Search,
				Tab::Flakes,
				Tab::Installed,
				Tab::Vcs,
				Tab::Building
			]
		);

		app.session.queue.push_back(QueuedOp::Uninstall {
			name: "ripgrep".into(),
			scope: Target::HomeManager,
		});
		assert_eq!(
			app.visible_tabs(),
			vec![
				Tab::Search,
				Tab::Flakes,
				Tab::Installed,
				Tab::Vcs,
				Tab::Building,
				Tab::Queue
			]
		);
	}

	#[test]
	fn stale_search_results_are_ignored() {
		let (tx, _rx) = mpsc::channel(1);
		let mut app = test_app();
		app.search_epoch = 2;
		app.searching = true;
		app.latest_query = "fd".into();

		handle_app_event(
			&mut app,
			&tx,
			AppEvent::SearchDone {
				epoch: 1,
				hits: vec![hit("wrong")],
			},
		);

		assert!(app.searching);
		assert!(app.results.is_empty());
		assert_eq!(app.status, "0 packages tracked.");
	}

	#[test]
	fn current_search_results_replace_previous_results() {
		let (tx, _rx) = mpsc::channel(1);
		let mut app = test_app();
		app.search_epoch = 7;
		app.searching = true;
		app.latest_query = "rg".into();
		app.results = vec![hit("old")];
		app.selected = 10;

		handle_app_event(
			&mut app,
			&tx,
			AppEvent::SearchDone {
				epoch: 7,
				hits: vec![hit("ripgrep"), hit("ripgrep-all")],
			},
		);

		assert!(!app.searching);
		assert_eq!(app.results.len(), 2);
		assert_eq!(app.results[0].attr, "ripgrep");
		assert_eq!(app.selected, 0);
		assert_eq!(app.status, "2 matches for `rg`");
	}

	#[test]
	fn current_search_failure_clears_results() {
		let (tx, _rx) = mpsc::channel(1);
		let mut app = test_app();
		app.search_epoch = 3;
		app.searching = true;
		app.results = vec![hit("old")];

		handle_app_event(
			&mut app,
			&tx,
			AppEvent::SearchFailed {
				epoch: 3,
				error: "boom".into(),
			},
		);

		assert!(!app.searching);
		assert!(app.results.is_empty());
		assert_eq!(app.status, "search failed: boom");
	}
}
