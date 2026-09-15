use std::collections::VecDeque;
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
use nixbox_config::{Config, DEFAULT_CHANNEL, InputMode, Target};
use nixbox_nix::{
    Manifest,
    build::BuildEvent,
    flakes::{FlakeDetails, FlakeHit},
    manifest::ManagedFile,
    scan::{ExternalPackage, ScanTarget, scan},
    search::{PackageCatalog, SearchHit},
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::handlers::{handle_app_event, handle_terminal_event};
use crate::ops::prepare_package_catalog;
use crate::state::{self, InProgress, PersistedState};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum QueuedOp {
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

impl QueuedOp {
    pub(crate) fn scope(&self) -> Target {
        match self {
            QueuedOp::Install { scope, .. }
            | QueuedOp::InstallFlake { scope, .. }
            | QueuedOp::InstallFlakePackage { scope, .. }
            | QueuedOp::Uninstall { scope, .. }
            | QueuedOp::Migrate { scope, .. } => *scope,
        }
    }

    pub(crate) fn label(&self) -> String {
        let tag = self.scope().tag();
        match self {
            QueuedOp::Install { hit, .. } => format!("install {} [{}]", hit.attr, tag),
            QueuedOp::InstallFlake { repo, .. } => format!("install flake {} [{}]", repo, tag),
            QueuedOp::InstallFlakePackage { repo, package, .. } => {
                format!("install {repo}#{package} [{tag}]")
            }
            QueuedOp::Uninstall { name, .. } => format!("remove {} [{}]", name, tag),
            QueuedOp::Migrate { names, .. } => match names.len() {
                1 => format!("migrate {} [{}]", names[0], tag),
                n => format!("migrate {} packages [{}]", n, tag),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Search,
    Flakes,
    Installed,
    Building,
    Queue,
}

impl Tab {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Tab::Search => "nixpkgs",
            Tab::Flakes => "Flakes",
            Tab::Installed => "Installed",
            Tab::Building => "Building",
            Tab::Queue => "Queue",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum AppEvent {
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
    Build(BuildEvent),
}

/// A package tracked by nixbox's manifest (managed) — tagged with the
/// target/scope it belongs to.
#[derive(Debug, Clone)]
pub(crate) struct ManagedPackage {
    pub name: String,
    pub scope: Target,
}

/// The row currently under the cursor in the Installed tab.
#[derive(Debug, Clone)]
pub(crate) enum InstalledCursor {
    Managed(ManagedPackage),
    External(ExternalPackage),
}

pub(crate) struct App {
    pub(crate) config: Config,
    /// Manifest of packages tracked in `nixbox-home-packages.nix`.
    pub(crate) home_manifest: Manifest,
    /// Manifest of packages tracked in `nixbox-system-packages.nix`.
    pub(crate) nixos_manifest: Manifest,
    /// Packages found declared directly in the user's main config files
    /// (both home.nix and configuration.nix), tagged with their scope.
    pub(crate) external_packages: Vec<ExternalPackage>,
    pub(crate) input: VimInput,
    pub(crate) results: Vec<SearchHit>,
    pub(crate) selected: usize,
    pub(crate) flake_input: VimInput,
    pub(crate) flake_results: Vec<FlakeHit>,
    pub(crate) flake_selected: usize,
    pub(crate) flake_details: Option<FlakeDetails>,
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
    pub(crate) log: Vec<String>,
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
    pub(crate) build_in_progress: bool,
    pub(crate) build_cancel: Option<oneshot::Sender<()>>,
    pub(crate) spinner_frame: usize,
    pub(crate) queue: VecDeque<QueuedOp>,
    pub(crate) current_op_label: Option<String>,
    /// Set while a rebuild is in flight; mirrors what gets persisted so the
    /// next launch can detect an interrupted operation. Cleared when the
    /// rebuild finishes (success or failure).
    pub(crate) in_progress_op: Option<InProgress>,
    /// Error message from the previous run that was never acknowledged.
    /// Survives across launches until the next successful build clears it.
    pub(crate) last_error: Option<String>,
}

impl App {
    pub(crate) fn new(
        config: Config,
        home_manifest: Manifest,
        nixos_manifest: Manifest,
        external_packages: Vec<ExternalPackage>,
    ) -> Self {
        let managed = home_manifest.packages.len() + nixos_manifest.packages.len();
        let external = external_packages.len();
        let theme_index = theme::ALL
            .iter()
            .position(|t| t.name == config.theme)
            .unwrap_or(0);
        let status = if external == 0 {
            format!("{} packages tracked.", managed)
        } else {
            format!("{} managed  ·  {} external.", managed, external)
        };
        let input_mode = config.input_mode;
        let mut input = VimInput::default();
        let mut flake_input = VimInput::default();
        let mut installed_input = VimInput::default();
        if input_mode == InputMode::Normal {
            input.enter_insert_before();
            flake_input.enter_insert_before();
            installed_input.enter_insert_before();
        }
        Self {
            config,
            home_manifest,
            nixos_manifest,
            external_packages,
            input,
            results: Vec::new(),
            selected: 0,
            flake_input,
            flake_results: Vec::new(),
            flake_selected: 0,
            flake_details: None,
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
            log: Vec::new(),
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
            build_in_progress: false,
            build_cancel: None,
            spinner_frame: 0,
            queue: VecDeque::new(),
            current_op_label: None,
            in_progress_op: None,
            last_error: None,
        }
    }

    pub(crate) fn visible_tabs(&self) -> Vec<Tab> {
        let mut tabs = vec![Tab::Search, Tab::Flakes, Tab::Installed];
        if self.build_in_progress || !self.log.is_empty() {
            tabs.push(Tab::Building);
        }
        if !self.queue.is_empty() {
            tabs.push(Tab::Queue);
        }
        tabs
    }

    pub(crate) fn channel(&self) -> &str {
        &self.config.channel
    }

    pub(crate) fn target_label(&self) -> &'static str {
        self.config.target.label()
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
        self.config.input_mode = mode;
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
        match scope {
            Target::HomeManager => &self.home_manifest,
            Target::NixosSystem => &self.nixos_manifest,
        }
    }

    pub(crate) fn manifest_for_mut(&mut self, scope: Target) -> &mut Manifest {
        match scope {
            Target::HomeManager => &mut self.home_manifest,
            Target::NixosSystem => &mut self.nixos_manifest,
        }
    }

    /// Returns all managed packages from both scopes, sorted by (scope, name)
    /// so HM entries appear before NixOS entries.
    pub(crate) fn managed_packages(&self) -> Vec<ManagedPackage> {
        let mut out: Vec<ManagedPackage> = Vec::new();
        for p in &self.home_manifest.packages {
            out.push(ManagedPackage {
                name: p.clone(),
                scope: Target::HomeManager,
            });
        }
        for p in &self.nixos_manifest.packages {
            out.push(ManagedPackage {
                name: p.clone(),
                scope: Target::NixosSystem,
            });
        }
        out
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
        self.external_packages
            .iter()
            .filter(|ep| match &filter {
                None => true,
                Some(q) => ep.name.to_lowercase().contains(q),
            })
            .cloned()
            .collect()
    }

    pub(crate) fn installed_total(&self) -> usize {
        self.filtered_managed_packages().len() + self.filtered_external_packages().len()
    }

    /// Returns the row currently under the cursor in the Installed tab.
    pub(crate) fn installed_cursor(&self) -> Option<InstalledCursor> {
        let managed = self.filtered_managed_packages();
        let external = self.filtered_external_packages();
        let total = managed.len() + external.len();
        if total == 0 {
            return None;
        }
        let idx = self.installed_selected.min(total - 1);
        if idx < managed.len() {
            Some(InstalledCursor::Managed(managed[idx].clone()))
        } else {
            Some(InstalledCursor::External(
                external[idx - managed.len()].clone(),
            ))
        }
    }

    /// Writes the current persistable slice of state to disk so the next
    /// launch can recover from a crash, kill, or interrupted rebuild. Errors
    /// are swallowed because failing to persist must never break the TUI.
    pub(crate) fn persist(&self) {
        let snapshot = PersistedState {
            pending_queue: self.queue.iter().cloned().collect(),
            in_progress: self.in_progress_op.clone(),
            last_error: self.last_error.clone(),
        };
        let _ = snapshot.save();
    }

    /// Re-reads both main config files and refreshes `external_packages`,
    /// excluding anything already tracked in either manifest.
    pub(crate) fn refresh_external_packages(&mut self) {
        self.external_packages =
            read_external_packages(&self.config, &self.home_manifest, &self.nixos_manifest);
        let total = self.installed_total();
        if total == 0 {
            self.installed_selected = 0;
        } else if self.installed_selected >= total {
            self.installed_selected = total - 1;
        }
    }
}

/// Scans both home.nix and configuration.nix and returns external packages
/// from each, scope-tagged, excluding anything already in the matching
/// manifest. The same package name can appear twice if declared in both
/// scopes — that's intentional.
pub(crate) fn read_external_packages(
    config: &Config,
    home_manifest: &Manifest,
    nixos_manifest: &Manifest,
) -> Vec<ExternalPackage> {
    let mut out: Vec<ExternalPackage> = Vec::new();

    if let Ok(found) = scan(
        &config.main_file_for(Target::HomeManager),
        ScanTarget::HomeManager,
    ) {
        out.extend(
            found
                .into_iter()
                .filter(|ep| !home_manifest.packages.contains(&ep.name)),
        );
    }
    if let Ok(found) = scan(
        &config.main_file_for(Target::NixosSystem),
        ScanTarget::Nixos,
    ) {
        out.extend(
            found
                .into_iter()
                .filter(|ep| !nixos_manifest.packages.contains(&ep.name)),
        );
    }

    out
}

pub async fn run() -> Result<()> {
    let config = Config::load_or_default()?;
    let home_manifest = ManagedFile::new(config.managed_file_for(Target::HomeManager)).load()?;
    let nixos_manifest = ManagedFile::new(config.managed_file_for(Target::NixosSystem)).load()?;
    let externals = read_external_packages(&config, &home_manifest, &nixos_manifest);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, SetCursorStyle::BlinkingBar)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(
        &mut terminal,
        config,
        home_manifest,
        nixos_manifest,
        externals,
    )
    .await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        SetCursorStyle::DefaultUserShape,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    result
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
            vec![Tab::Search, Tab::Flakes, Tab::Installed]
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
        app.build_in_progress = true;
        assert_eq!(
            app.visible_tabs(),
            vec![Tab::Search, Tab::Flakes, Tab::Installed, Tab::Building]
        );

        app.queue.push_back(QueuedOp::Uninstall {
            name: "ripgrep".into(),
            scope: Target::HomeManager,
        });
        assert_eq!(
            app.visible_tabs(),
            vec![
                Tab::Search,
                Tab::Flakes,
                Tab::Installed,
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

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    config: Config,
    home_manifest: Manifest,
    nixos_manifest: Manifest,
    externals: Vec<ExternalPackage>,
) -> Result<()> {
    let mut app = App::new(config, home_manifest, nixos_manifest, externals);
    let (tx, mut rx) = mpsc::channel::<AppEvent>(128);
    state::restore(&mut app, &tx);
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
            break;
        }

        tokio::select! {
            Some(Ok(ev)) = term_events.next() => {
                handle_terminal_event(&mut app, &tx, ev).await?;
            }
            Some(app_ev) = rx.recv() => {
                handle_app_event(&mut app, &tx, app_ev);
            }
            _ = spinner_tick.tick(), if app.searching || app.catalog_loading || app.flake_searching || app.flake_detail_loading || app.build_in_progress => {
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
    Ok(())
}
