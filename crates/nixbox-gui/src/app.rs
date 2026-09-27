//! The window's state and everything that changes it.
//!
//! Work that touches nix runs on the tokio runtime `main` starts; gpui tasks
//! await its join handles and fold the results back in here. Queued ops and
//! rebuilds go through the shared [`Session`], exactly as in the TUI.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Root, WindowExt as _};
use gpui_kit::{
    App, AppContext as _, Context, Entity, KeyBinding, SharedString, Subscription, Task,
    UniformListScrollHandle, Window, actions,
};
use nixbox_config::Target;
use nixbox_core::search::{catalog_cache_path, search_packages};
use nixbox_core::{BuildEnded, Enqueued, InstalledFlake, Op, Restored, Session};
use nixbox_nix::build::BuildEvent;
use nixbox_nix::flakes::{FlakeDetails, FlakeHit, fetch_flake_details, search_flakes};
use nixbox_nix::scan::ExternalPackage;
use nixbox_nix::search::{MAX_SEARCH_RESULTS, PackageCatalog, SearchHit};
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::model::target_of;
use crate::theme;

actions!(nixbox, [Quit, FocusSearch]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("ctrl-f", FocusSearch, None),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Packages,
    Flakes,
    Installed,
    Queue,
    Build,
    Settings,
}

pub struct NixboxApp {
    pub session: Session,
    pub page: Page,
    pub status: SharedString,
    runtime: Handle,

    pub search_input: Entity<InputState>,
    pub results: Vec<SearchHit>,
    pub searching: bool,
    search_epoch: u64,
    search_task: Option<Task<()>>,
    catalog: Option<Arc<PackageCatalog>>,
    pub catalog_loading: bool,
    catalog_task: Option<Task<()>>,

    pub flake_input: Entity<InputState>,
    pub flake_results: Vec<FlakeHit>,
    pub flake_selected: Option<usize>,
    pub flake_details: Option<FlakeDetails>,
    pub flake_searching: bool,
    pub flake_detail_loading: bool,
    flake_epoch: u64,
    flake_task: Option<Task<()>>,
    flake_detail_task: Option<Task<()>>,

    pub installed_input: Entity<InputState>,
    pub log_scroll: UniformListScrollHandle,

    _build_events: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl NixboxApp {
    pub fn new(
        session: Session,
        mut build_rx: UnboundedReceiver<BuildEvent>,
        runtime: Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        theme::apply(&session.engine.config.theme, Some(window), cx);

        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search nixpkgs, e.g. ripgrep"));
        let flake_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search GitHub for flakes by name or content")
        });
        let installed_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter installed packages"));
        search_input.update(cx, |input, cx| input.focus(window, cx));

        let subscriptions = vec![
            cx.subscribe_in(&search_input, window, |this, _, event, _, cx| {
                if let InputEvent::Change = event {
                    this.schedule_search(cx);
                }
            }),
            cx.subscribe_in(&flake_input, window, |this, _, event, _, cx| {
                if let InputEvent::Change = event {
                    this.schedule_flake_search(cx);
                }
            }),
            cx.subscribe_in(&installed_input, window, |_, _, event, _, cx| {
                if let InputEvent::Change = event {
                    cx.notify();
                }
            }),
        ];

        let build_events = cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = build_rx.recv().await {
                let updated = this.update_in(cx, |this, window, cx| {
                    this.on_build_event(event, window, cx);
                });
                if updated.is_err() {
                    break;
                }
            }
        });

        let managed = session.engine.managed_packages().len();
        let external = session.engine.external_packages.len();
        let mut app = Self {
            session,
            page: Page::Packages,
            status: format!("{managed} managed, {external} external.").into(),
            runtime,
            search_input,
            results: Vec::new(),
            searching: false,
            search_epoch: 0,
            search_task: None,
            catalog: None,
            catalog_loading: false,
            catalog_task: None,
            flake_input,
            flake_results: Vec::new(),
            flake_selected: None,
            flake_details: None,
            flake_searching: false,
            flake_detail_loading: false,
            flake_epoch: 0,
            flake_task: None,
            flake_detail_task: None,
            installed_input,
            log_scroll: UniformListScrollHandle::new(),
            _build_events: build_events,
            _subscriptions: subscriptions,
        };
        app.restore();
        app
    }

    fn restore(&mut self) {
        let restored = self.session.restore();
        if let Some(error) = &self.session.last_error {
            self.status = format!("Previous run failed: {error}").into();
        }
        match restored {
            Restored::Nothing => {}
            Restored::Rebuild(label) => {
                self.page = Page::Build;
                self.status = format!(
                    "Resuming interrupted build: {}.",
                    label.trim_start_matches("resume ")
                )
                .into();
            }
            Restored::Queue(count) => {
                self.page = Page::Queue;
                self.status = format!("Resuming {count} queued op(s).").into();
            }
        }
    }

    pub fn set_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        let input = match page {
            Page::Packages => Some(&self.search_input),
            Page::Flakes => Some(&self.flake_input),
            Page::Installed => Some(&self.installed_input),
            Page::Queue | Page::Build | Page::Settings => None,
        };
        if let Some(input) = input {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        if page == Page::Build {
            self.scroll_log_to_end();
        }
        cx.notify();
    }

    pub fn focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        let page = match self.page {
            Page::Flakes | Page::Installed => self.page,
            _ => Page::Packages,
        };
        self.set_page(page, window, cx);
    }

    fn set_status(&mut self, status: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status = status.into();
        cx.notify();
    }

    // ── nixpkgs search ──────────────────────────────────────────────────

    /// Loads the package catalog for the locked nixpkgs, building it the
    /// first time, so searches stop shelling out to `nix search`.
    pub fn prepare_catalog(&mut self, cx: &mut Context<Self>) {
        let Some(cache_path) = catalog_cache_path() else {
            return;
        };
        let config_dir = self.session.config_dir();
        self.catalog_loading = true;
        let job = self
            .runtime
            .spawn(async move { PackageCatalog::load_or_build(&config_dir, &cache_path).await });
        self.catalog_task = Some(cx.spawn(async move |this, cx| {
            let result = match job.await {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                this.catalog_loading = false;
                this.catalog_task = None;
                match result {
                    Ok(catalog) => {
                        let revision: String = catalog.revision().chars().take(12).collect();
                        this.catalog = Some(Arc::new(catalog));
                        this.status = format!("Package catalog ready at {revision}.").into();
                    }
                    Err(error) => {
                        this.catalog = None;
                        this.status =
                            format!("Package catalog unavailable; using live search: {error}")
                                .into();
                    }
                }
                if this.searching {
                    this.schedule_search(cx);
                }
                cx.notify();
            });
        }));
    }

    fn schedule_search(&mut self, cx: &mut Context<Self>) {
        self.search_epoch = self.search_epoch.wrapping_add(1);
        let epoch = self.search_epoch;
        let query = self.search_input.read(cx).value().trim().to_string();
        if query.is_empty() {
            self.search_task = None;
            self.searching = false;
            self.results.clear();
            self.set_status("Type to search packages.", cx);
            return;
        }
        self.searching = true;
        if self.catalog_loading {
            self.set_status(
                "Preparing the package catalog for your locked nixpkgs revision...",
                cx,
            );
            return;
        }
        if self.catalog.as_ref().is_some_and(|catalog| {
            !catalog
                .is_current_for(&self.session.config_dir())
                .unwrap_or(false)
        }) {
            self.catalog = None;
            self.prepare_catalog(cx);
            self.set_status(
                "The nixpkgs lock changed; refreshing the package catalog...",
                cx,
            );
            return;
        }

        let catalog = self.catalog.clone();
        let channel = self.session.engine.config.channel.clone();
        let runtime = self.runtime.clone();
        cx.notify();
        self.search_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(180))
                .await;
            let label = query.clone();
            let job =
                runtime.spawn(async move { search_packages(catalog, &channel, query).await });
            let result = match job.await {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                if epoch != this.search_epoch {
                    return;
                }
                this.searching = false;
                match result {
                    Ok(hits) => {
                        this.status = if hits.len() == MAX_SEARCH_RESULTS {
                            format!(
                                "Showing the first {} matches for `{label}`; refine the search for more.",
                                hits.len()
                            )
                        } else {
                            format!("{} matches for `{label}`.", hits.len())
                        }
                        .into();
                        this.results = hits;
                    }
                    Err(error) => {
                        this.results.clear();
                        this.status = format!("Search failed: {error}").into();
                    }
                }
                cx.notify();
            });
        }));
    }

    pub fn install(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self.results.get(index).cloned() else {
            return;
        };
        let scope = self.session.engine.config.target;
        if self.session.engine.is_tracked(&hit.attr, scope) {
            self.set_status(format!("{} is already tracked.", hit.attr), cx);
            return;
        }
        let attr = hit.attr.clone();
        self.enqueue(
            Op::Install { hit, scope },
            format!("{attr} is already queued."),
            window,
            cx,
        );
    }

    // ── flakes ──────────────────────────────────────────────────────────

    fn schedule_flake_search(&mut self, cx: &mut Context<Self>) {
        self.flake_epoch = self.flake_epoch.wrapping_add(1);
        let epoch = self.flake_epoch;
        self.flake_detail_task = None;
        self.flake_detail_loading = false;
        let query = self.flake_input.read(cx).value().trim().to_string();
        if query.is_empty() {
            self.flake_task = None;
            self.flake_searching = false;
            self.flake_results.clear();
            self.flake_selected = None;
            self.flake_details = None;
            self.set_status("Search GitHub for flakes by content or project name.", cx);
            return;
        }
        self.flake_searching = true;
        let runtime = self.runtime.clone();
        cx.notify();
        self.flake_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            let label = query.clone();
            let job = runtime.spawn(async move { search_flakes(&query).await });
            let result = match job.await {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                if epoch != this.flake_epoch {
                    return;
                }
                this.flake_searching = false;
                this.flake_details = None;
                match result {
                    Ok(hits) => {
                        this.status =
                            format!("{} GitHub flake matches for `{label}`.", hits.len()).into();
                        this.flake_results = hits;
                        this.select_flake(0, cx);
                    }
                    Err(error) => {
                        this.flake_results.clear();
                        this.flake_selected = None;
                        this.status = format!("Flake search failed: {error}").into();
                    }
                }
                cx.notify();
            });
        }));
    }

    pub fn select_flake(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(hit) = self.flake_results.get(index).cloned() else {
            self.flake_selected = None;
            return;
        };
        self.flake_selected = Some(index);
        self.flake_details = None;
        self.flake_detail_loading = true;
        let epoch = self.flake_epoch;
        let runtime = self.runtime.clone();
        self.flake_detail_task = Some(cx.spawn(async move |this, cx| {
            let job = runtime.spawn(async move { fetch_flake_details(&hit).await });
            let result = match job.await {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                if epoch != this.flake_epoch || this.flake_selected != Some(index) {
                    return;
                }
                this.flake_detail_loading = false;
                match result {
                    Ok(details) => this.flake_details = Some(details),
                    Err(error) => this.status = format!("Flake details failed: {error}").into(),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub fn install_flake(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(details) = self.flake_details.as_ref() else {
            self.set_status("Wait for the flake details before installing.", cx);
            return;
        };
        let scope = self.session.engine.config.target;
        let repo = details.repo.clone();
        let Some(op) = Op::install_flake(details, scope) else {
            self.set_status(
                format!(
                    "{repo} has no installable package for this system or default {} module.",
                    scope.label()
                ),
                cx,
            );
            return;
        };
        self.enqueue(op, format!("{repo} is already queued."), window, cx);
    }

    // ── installed ───────────────────────────────────────────────────────

    pub fn remove_managed(
        &mut self,
        name: String,
        scope: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let duplicate = format!("{name} is already queued for removal.");
        self.enqueue(Op::Uninstall { name, scope }, duplicate, window, cx);
    }

    pub fn remove_flake(
        &mut self,
        flake: InstalledFlake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = flake.name();
        if !flake.removable {
            self.set_status(
                format!(
                    "{name} shares a line in {} with other entries; remove it by hand.",
                    flake.declared_in.as_deref().unwrap_or("your config")
                ),
                cx,
            );
            return;
        }
        self.enqueue(
            Op::UninstallFlakeOutput {
                input: flake.input,
                output: flake.output,
                scope: flake.scope,
            },
            format!("{name} is already queued for removal."),
            window,
            cx,
        );
    }

    pub fn migrate(
        &mut self,
        package: ExternalPackage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !package.migratable {
            self.set_status(
                format!(
                    "{} shares a line in {} with other entries; move it by hand.",
                    package.name, package.source_attr
                ),
                cx,
            );
            return;
        }
        let name = package.name;
        self.enqueue(
            Op::Migrate {
                names: vec![name.clone()],
                scope: target_of(package.scope),
            },
            format!("{name} is already queued for migration."),
            window,
            cx,
        );
    }

    pub fn migrate_flake(
        &mut self,
        flake: InstalledFlake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = flake.name();
        if !flake.migratable() {
            self.set_status(format!("{name} cannot be migrated automatically."), cx);
            return;
        }
        if let nixbox_nix::manifest::FlakeOutput::Package(package) = flake.output {
            self.enqueue(
                Op::MigrateFlakePackage {
                    input: flake.input,
                    package,
                    scope: flake.scope,
                },
                format!("{name} is already queued for migration."),
                window,
                cx,
            );
        }
    }

    pub fn migrate_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ops = self.session.migrate_all_ops();
        if ops.is_empty() {
            self.set_status("No external packages or flake outputs left to migrate.", cx);
            return;
        }
        let outcome = self.session.enqueue_all(ops);
        self.report_enqueued(outcome, "Nothing new to migrate.".into(), window, cx);
    }

    // ── queue and rebuilds ──────────────────────────────────────────────

    fn enqueue(&mut self, op: Op, duplicate: String, window: &mut Window, cx: &mut Context<Self>) {
        let outcome = self.session.enqueue(op);
        self.report_enqueued(outcome, duplicate, window, cx);
    }

    fn report_enqueued(
        &mut self,
        outcome: Enqueued,
        duplicate: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let status = match outcome {
            Enqueued::Duplicate => duplicate,
            Enqueued::Queued => format!(
                "Queued behind the running rebuild ({} waiting).",
                self.session.queue.len()
            ),
            Enqueued::Started(label) => {
                window.push_notification(Notification::info(format!("Started: {label}")), cx);
                format!("{label}...")
            }
            Enqueued::NotWritten => {
                "The queued changes could not be written; see the build log.".into()
            }
        };
        self.set_status(status, cx);
    }

    pub fn dequeue(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(op) = self.session.dequeue(index) {
            self.set_status(format!("Dropped {} from the queue.", op.label()), cx);
        }
    }

    pub fn apply_queue(&mut self, cx: &mut Context<Self>) {
        match self.session.drain() {
            Some(label) => self.set_status(format!("{label}..."), cx),
            None => cx.notify(),
        }
    }

    pub fn cancel_build(&mut self, cx: &mut Context<Self>) {
        if self.session.cancel_build() {
            self.set_status("Cancelling the rebuild...", cx);
        }
    }

    pub fn dismiss_error(&mut self, cx: &mut Context<Self>) {
        self.session.dismiss_error();
        cx.notify();
    }

    fn on_build_event(&mut self, event: BuildEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ended) = self.session.on_build_event(event) else {
            if self.page == Page::Build {
                self.scroll_log_to_end();
            }
            cx.notify();
            return;
        };
        let (status, note) = match ended {
            BuildEnded::Succeeded { label, next } => (
                next.map_or_else(|| format!("{label} done."), |next| format!("{next}...")),
                Notification::success(format!("{label} done.")),
            ),
            BuildEnded::Failed { label, error, next } => (
                next.map_or_else(
                    || format!("{label} failed: {error}."),
                    |next| format!("{next}..."),
                ),
                Notification::error(format!("{label} failed: {error}.")),
            ),
            BuildEnded::Cancelled { label, paused } => {
                let status = if paused == 0 {
                    format!("{label} cancelled.")
                } else {
                    format!("{label} cancelled; {paused} queued op(s) paused.")
                };
                (status.clone(), Notification::warning(status))
            }
        };
        window.push_notification(note, cx);
        self.set_status(status, cx);
    }

    fn scroll_log_to_end(&self) {
        if let Some(last) = self.session.log.len().checked_sub(1) {
            self.log_scroll
                .scroll_to_item(last, gpui_kit::ScrollStrategy::Bottom);
        }
    }

    // ── settings ────────────────────────────────────────────────────────

    pub fn set_target(&mut self, target: Target, cx: &mut Context<Self>) {
        self.session.engine.config.target = target;
        self.save_config(format!("New installs go to {}.", target.label()), cx);
    }

    pub fn set_channel(&mut self, channel: &str, cx: &mut Context<Self>) {
        self.session.engine.config.channel = channel.to_string();
        self.save_config(format!("Live search uses {channel}."), cx);
    }

    pub fn set_theme(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.session.engine.config.theme = name.to_string();
        theme::apply(name, Some(window), cx);
        self.save_config(format!("Theme set to {name}."), cx);
    }

    fn save_config(&mut self, done: String, cx: &mut Context<Self>) {
        let status = match self.session.engine.config.save() {
            Ok(()) => done,
            Err(error) => format!("Saving settings failed: {error}"),
        };
        self.set_status(status, cx);
    }

    /// Notification and dialog layers, drawn above the page.
    pub fn overlays(
        window: &mut Window,
        cx: &mut App,
    ) -> impl Iterator<Item = gpui_kit::AnyElement> {
        use gpui_kit::IntoElement;
        [
            Root::render_sheet_layer(window, cx).map(IntoElement::into_any_element),
            Root::render_dialog_layer(window, cx).map(IntoElement::into_any_element),
            Root::render_notification_layer(window, cx).map(IntoElement::into_any_element),
        ]
        .into_iter()
        .flatten()
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, TestAppContext, px, size};
    use nixbox_config::{Config, Target};
    use nixbox_core::{Engine, Session};
    use nixbox_nix::build::BuildEvent;
    use nixbox_nix::manifest::Manifest;
    use nixbox_nix::search::SearchHit;

    use super::{NixboxApp, Page};

    #[gpui_kit::test]
    async fn every_page_renders_with_data_and_while_building(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let mut home = Manifest::default();
        home.packages.insert("ripgrep".into());
        let (session, build_rx) = Session::new(Engine::from_parts(
            Config::default(),
            home,
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

        app.update(cx, |app, _| {
            app.results = vec![SearchHit {
                attr: "ripgrep".into(),
                pname: "ripgrep".into(),
                version: "14.1.1".into(),
                description: "Fast grep".into(),
            }];
            let _cancel = app
                .session
                .fake_build(Target::HomeManager, "apply 1 queued change(s)");
            app.session
                .on_build_event(BuildEvent::Line("building...".into()));
        });

        for page in [
            Page::Packages,
            Page::Flakes,
            Page::Installed,
            Page::Queue,
            Page::Build,
            Page::Settings,
        ] {
            cx.update_window(handle.into(), |_, window, cx| {
                app.update(cx, |app, cx| app.set_page(page, window, cx));
                window.render_frame(cx);
            })
            .expect("window");
        }

        // ripgrep is only tracked for home and the default target is NixOS, so
        // it is queued, and waits behind the running rebuild.
        cx.update_window(handle.into(), |_, window, cx| {
            app.update(cx, |app, cx| app.install(0, window, cx));
        })
        .expect("window");
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.queue.len(), 1);
            assert!(app.status.contains("Queued behind the running rebuild"));
        });
    }
}
