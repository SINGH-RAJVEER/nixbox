use std::{collections::VecDeque, time::Duration};

use anyhow::{Result, anyhow};
use directories::BaseDirs;
use nixbox_config::Target;
use nixbox_core::{
    HOME_FALLBACK_NOTE, InstalledFlake, LogReporter, rebuild::resolve as resolve_rebuild,
};
use nixbox_nix::{
    build::{BuildEvent, rebuild},
    flakes::{fetch_flake_details, search_flakes},
    scan::ScanTarget,
    search::PackageCatalog,
};
use tokio::sync::{mpsc, oneshot};
use tokio::time::sleep;

use crate::app::{App, AppEvent, InstalledCursor, QueuedOp, Tab};
use crate::state::InProgress;

pub(crate) fn spawn_rebuild(
    app: &mut App,
    tx: &mpsc::Sender<AppEvent>,
    scope: Target,
    action_label: String,
) {
    app.build_in_progress = true;
    let (cancel_tx, mut cancel_rx) = oneshot::channel();
    app.build_cancel = Some(cancel_tx);
    app.current_op_label = Some(action_label.clone());
    app.in_progress_op = Some(InProgress {
        scope,
        label: action_label.clone(),
    });
    app.log.clear();
    app.status = format!("{}...", action_label);
    app.persist();

    let config_dir = app.engine.config.home_manager_dir();
    let app_tx = tx.clone();
    tokio::spawn(async move {
        let (build_tx, mut build_rx) = mpsc::channel::<BuildEvent>(64);
        let forward_tx = app_tx.clone();
        let forwarder = tokio::spawn(async move {
            while let Some(ev) = build_rx.recv().await {
                if forward_tx.send(AppEvent::Build(ev)).await.is_err() {
                    break;
                }
            }
        });
        // Resolving a home-manager rebuild evaluates the flake, which is slow
        // enough to be worth racing against the cancel signal.
        let command = tokio::select! {
            command = resolve_rebuild(&config_dir, scope) => command,
            _ = &mut cancel_rx => {
                let _ = build_tx.send(BuildEvent::Cancelled).await;
                drop(build_tx);
                let _ = forwarder.await;
                return;
            }
        };
        if command.via_nixos_fallback {
            let _ = app_tx
                .send(AppEvent::Build(BuildEvent::Line(HOME_FALLBACK_NOTE.into())))
                .await;
        }
        if let Err(e) = rebuild(
            &command.program,
            &command.arg_refs(),
            build_tx.clone(),
            cancel_rx,
        )
        .await
        {
            let _ = build_tx
                .send(BuildEvent::Finished(Err(e.to_string())))
                .await;
        }
        drop(build_tx);
        let _ = forwarder.await;
    });
}

pub(crate) fn cancel_build(app: &mut App) {
    if !app.build_in_progress {
        return;
    }
    let Some(cancel) = app.build_cancel.take() else {
        return;
    };
    if cancel.send(()).is_ok() {
        app.status = "Cancelling build...".into();
    }
}

pub(crate) fn drain_queue(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    if app.build_in_progress {
        return;
    }
    let Some(scope) = app.queue.front().map(QueuedOp::scope) else {
        app.persist();
        return;
    };

    let batch = take_queued_scope(app, scope);
    let mut applied = 0;
    for op in batch {
        let label = op.label();
        match apply_op_to_manifest(app, &op) {
            Ok(()) => applied += 1,
            Err(e) => app
                .log
                .push(format!("{}: failed to write manifest: {}", label, e)),
        }
    }
    if applied == 0 {
        app.status = format!("No queued {} changes could be written.", scope.label());
        app.persist();
        drain_queue(app, tx);
        return;
    }
    if !app.visible_tabs().contains(&app.tab) {
        app.tab = Tab::Search;
    }
    let label = format!("apply {} queued {} change(s)", applied, scope.label(),);
    spawn_rebuild(app, tx, scope, label);
}

/// Removes every pending operation for `scope`, preserving the order of work
/// for any other scope that requires a separate rebuild command.
fn take_queued_scope(app: &mut App, scope: Target) -> Vec<QueuedOp> {
    let mut batch = Vec::new();
    let mut remaining = VecDeque::new();
    while let Some(op) = app.queue.pop_front() {
        if op.scope() == scope {
            batch.push(op);
        } else {
            remaining.push_back(op);
        }
    }
    app.queue = remaining;
    batch
}

/// Hands one queued op to the engine and folds whatever it reports into the
/// build log, then keeps the Installed cursor inside the new bounds.
fn apply_op_to_manifest(app: &mut App, op: &QueuedOp) -> Result<()> {
    let mut reporter = LogReporter::new();
    let result = app.engine.apply(op, &mut reporter);
    app.log.extend(reporter.into_lines());
    result?;

    let total = app.installed_total();
    if total == 0 {
        app.installed_selected = 0;
    } else if app.installed_selected >= total {
        app.installed_selected = total - 1;
    }
    Ok(())
}

pub(crate) async fn install_selected(app: &mut App, tx: &mpsc::Sender<AppEvent>) -> Result<()> {
    if app.package_catalog.as_ref().is_some_and(|catalog| {
        !catalog
            .is_current_for(&app.engine.config.home_manager_dir())
            .unwrap_or(false)
    }) {
        app.package_catalog = None;
        prepare_package_catalog(app, tx.clone());
        app.status = "The nixpkgs lock changed; refreshing the package catalog first.".into();
        return Ok(());
    }

    let Some(hit) = app.results.get(app.selected).cloned() else {
        app.status = "No selection.".into();
        return Ok(());
    };

    let scope = app.engine.config.target;
    let already_tracked = app.manifest_for(scope).packages.contains(&hit.attr);
    let already_queued = app.queue.iter().any(|op| match op {
        QueuedOp::Install { hit: h, scope: s } => *s == scope && h.attr == hit.attr,
        _ => false,
    });
    if already_tracked || already_queued {
        app.status = format!("{} already tracked or queued.", hit.attr);
        return Ok(());
    }

    let attr = hit.attr.clone();
    app.queue.push_back(QueuedOp::Install { hit, scope });
    app.persist();
    if app.build_in_progress {
        app.status = format!("Queued install: {}.", attr);
        app.tab = Tab::Queue;
    } else {
        app.tab = Tab::Building;
        drain_queue(app, tx);
    }
    Ok(())
}

pub(crate) async fn install_selected_flake(
    app: &mut App,
    tx: &mpsc::Sender<AppEvent>,
) -> Result<()> {
    let Some(details) = app.flake_details.clone() else {
        app.status = "Wait for flake details before installing.".into();
        return Ok(());
    };
    let scope = app.engine.config.target;
    if app.queue.iter().any(|op| {
        matches!(
            op,
            QueuedOp::InstallFlake { repo, scope: queued_scope, .. }
                if repo == &details.repo && *queued_scope == scope
        ) || matches!(
            op,
            QueuedOp::InstallFlakePackage { repo, scope: queued_scope, .. }
                if repo == &details.repo && *queued_scope == scope
        )
    }) {
        app.status = format!("{} is already queued.", details.repo);
        return Ok(());
    }

    let package = details.packages.first().map(|package| package.attr.clone());
    let op = match scope {
        // A Home Manager module does nothing until its options are set, so
        // the package goes straight into `home.packages` whenever there is one.
        Target::HomeManager => package
            .map(|package| QueuedOp::InstallFlakePackage {
                repo: details.repo.clone(),
                package,
                scope,
            })
            .or_else(|| {
                details
                    .home_manager_module
                    .clone()
                    .map(|module| QueuedOp::InstallFlake {
                        repo: details.repo.clone(),
                        module,
                        scope,
                    })
            }),
        Target::NixosSystem => details
            .nixos_module
            .clone()
            .map(|module| QueuedOp::InstallFlake {
                repo: details.repo.clone(),
                module,
                scope,
            })
            .or_else(|| {
                package.map(|package| QueuedOp::InstallFlakePackage {
                    repo: details.repo.clone(),
                    package,
                    scope,
                })
            }),
    };
    let Some(op) = op else {
        app.status = format!(
            "{} has no installable package for this system or default {} module.",
            details.repo,
            scope.label()
        );
        return Ok(());
    };
    app.queue.push_back(op);
    app.persist();
    if app.build_in_progress {
        app.status = format!("Queued flake install: {}.", details.repo);
        app.tab = Tab::Queue;
    } else {
        app.tab = Tab::Building;
        drain_queue(app, tx);
    }
    Ok(())
}

pub(crate) async fn uninstall_selected(app: &mut App, tx: &mpsc::Sender<AppEvent>) -> Result<()> {
    let cursor = match app.installed_cursor() {
        Some(c) => c,
        None => {
            app.status = "No selection.".into();
            return Ok(());
        }
    };
    let pkg = match cursor {
        InstalledCursor::Managed(p) => p,
        InstalledCursor::Flake(flake) => return uninstall_flake(app, tx, flake),
        InstalledCursor::External(ep) => {
            app.status = format!(
                "{} is external (in {}) — press m to migrate first.",
                ep.name, ep.source_attr,
            );
            return Ok(());
        }
    };

    let scope = pkg.scope;
    if app.queue.iter().any(|op| match op {
        QueuedOp::Uninstall { name, scope: s } => *s == scope && *name == pkg.name,
        _ => false,
    }) {
        app.status = format!("{} already queued for removal.", pkg.name);
        return Ok(());
    }

    let name = pkg.name.clone();
    app.queue.push_back(QueuedOp::Uninstall {
        name: name.clone(),
        scope,
    });
    app.persist();
    if app.build_in_progress {
        app.status = format!("Queued remove: {} [{}].", name, scope.tag());
        app.tab = Tab::Queue;
    } else {
        app.tab = Tab::Building;
        drain_queue(app, tx);
    }
    Ok(())
}

/// Queues removal of one flake output. The engine takes it out of whichever
/// file declares it and drops the flake's input once nothing uses it.
fn uninstall_flake(
    app: &mut App,
    tx: &mpsc::Sender<AppEvent>,
    flake: InstalledFlake,
) -> Result<()> {
    let name = flake.name();
    if !flake.removable {
        app.status = format!(
            "{} shares a line in {} with other entries; remove it manually.",
            name,
            flake.declared_in.as_deref().unwrap_or("your config"),
        );
        return Ok(());
    }
    let scope = flake.scope;
    if app.queue.iter().any(|op| {
        matches!(
            op,
            QueuedOp::UninstallFlakeOutput { input, output, scope: s }
                if *s == scope && *input == flake.input && *output == flake.output
        )
    }) {
        app.status = format!("{} already queued for removal.", name);
        return Ok(());
    }

    app.queue.push_back(QueuedOp::UninstallFlakeOutput {
        input: flake.input,
        output: flake.output,
        scope,
    });
    app.persist();
    if app.build_in_progress {
        app.status = format!("Queued remove: {} [{}].", name, scope.tag());
        app.tab = Tab::Queue;
    } else {
        app.tab = Tab::Building;
        drain_queue(app, tx);
    }
    Ok(())
}

pub(crate) async fn migrate_selected(app: &mut App, tx: &mpsc::Sender<AppEvent>) -> Result<()> {
    let cursor = match app.installed_cursor() {
        Some(c) => c,
        None => {
            app.status = "No selection.".into();
            return Ok(());
        }
    };
    let ep = match cursor {
        InstalledCursor::External(ep) => ep,
        InstalledCursor::Managed(p) => {
            app.status = format!("{} is already managed — press d to uninstall.", p.name);
            return Ok(());
        }
        InstalledCursor::Flake(flake) => {
            app.status = format!(
                "{} comes from a flake and is not migrated — press d to uninstall.",
                flake.name()
            );
            return Ok(());
        }
    };
    if !ep.migratable {
        app.status = format!(
            "{} is on a same-line list in {}; remove it manually.",
            ep.name, ep.source_attr,
        );
        return Ok(());
    }
    let scope = match ep.scope {
        ScanTarget::HomeManager => Target::HomeManager,
        ScanTarget::Nixos => Target::NixosSystem,
    };
    let name = ep.name.clone();
    if app.queue.iter().any(|op| match op {
        QueuedOp::Migrate { names, scope: s } => *s == scope && names.contains(&name),
        _ => false,
    }) {
        app.status = format!("{} already queued for migration.", name);
        return Ok(());
    }
    app.queue.push_back(QueuedOp::Migrate {
        names: vec![name.clone()],
        scope,
    });
    app.persist();
    if app.build_in_progress {
        app.status = format!("Queued migrate: {} [{}].", name, scope.tag());
        app.tab = Tab::Queue;
    } else {
        app.tab = Tab::Building;
        drain_queue(app, tx);
    }
    Ok(())
}

pub(crate) async fn migrate_all(app: &mut App, tx: &mpsc::Sender<AppEvent>) -> Result<()> {
    let mut hm: Vec<String> = Vec::new();
    let mut nx: Vec<String> = Vec::new();
    for ep in &app.engine.external_packages {
        if !ep.migratable {
            continue;
        }
        match ep.scope {
            ScanTarget::HomeManager => hm.push(ep.name.clone()),
            ScanTarget::Nixos => nx.push(ep.name.clone()),
        }
    }
    if hm.is_empty() && nx.is_empty() {
        app.status = "No migratable external packages found.".into();
        return Ok(());
    }
    let hm_count = hm.len();
    let nx_count = nx.len();
    if !hm.is_empty() {
        app.queue.push_back(QueuedOp::Migrate {
            names: hm,
            scope: Target::HomeManager,
        });
    }
    if !nx.is_empty() {
        app.queue.push_back(QueuedOp::Migrate {
            names: nx,
            scope: Target::NixosSystem,
        });
    }
    app.persist();
    let summary = format!("Queued migrate-all ({} hm, {} nixos).", hm_count, nx_count,);
    if app.build_in_progress {
        app.status = summary;
        app.tab = Tab::Queue;
    } else {
        app.status = summary;
        app.tab = Tab::Building;
        drain_queue(app, tx);
    }
    Ok(())
}

pub(crate) fn prepare_package_catalog(app: &mut App, tx: mpsc::Sender<AppEvent>) {
    if let Some(task) = app.catalog_task.take() {
        task.abort();
    }

    let Some(base_dirs) = BaseDirs::new() else {
        app.catalog_loading = false;
        return;
    };
    let config_dir = app.engine.config.home_manager_dir();
    let cache_path = base_dirs
        .cache_dir()
        .join("nixbox")
        .join("package-catalog.json");
    app.catalog_loading = true;
    app.catalog_task = Some(tokio::spawn(async move {
        match PackageCatalog::load_or_build(&config_dir, &cache_path).await {
            Ok(catalog) => {
                let _ = tx
                    .send(AppEvent::CatalogReady(std::sync::Arc::new(catalog)))
                    .await;
            }
            Err(error) => {
                let _ = tx.send(AppEvent::CatalogFailed(error.to_string())).await;
            }
        }
    }));
}

pub(crate) fn schedule_search(app: &mut App, tx: mpsc::Sender<AppEvent>) {
    if let Some(task) = app.search_task.take() {
        task.abort();
    }

    let query = app.input.value().to_string();
    if query.is_empty() {
        app.search_epoch += 1;
        app.searching = false;
        app.results.clear();
        app.selected = 0;
        app.latest_query = String::new();
        app.status = "Type to search packages.".into();
        return;
    }
    app.searching = true;
    app.search_epoch += 1;
    let epoch = app.search_epoch;
    let channel = app.engine.config.channel.clone();
    app.latest_query = query.clone();

    if app.catalog_loading {
        app.status = "Preparing package catalog for your locked nixpkgs revision...".into();
        return;
    }

    if app.package_catalog.as_ref().is_some_and(|catalog| {
        !catalog
            .is_current_for(&app.engine.config.home_manager_dir())
            .unwrap_or(false)
    }) {
        app.package_catalog = None;
        prepare_package_catalog(app, tx);
        app.status = "The nixpkgs lock changed; refreshing the package catalog...".into();
        return;
    }

    let catalog = app.package_catalog.clone();

    app.search_task = Some(tokio::spawn(async move {
        sleep(Duration::from_millis(180)).await;
        let result = if let Some(catalog) = catalog {
            tokio::task::spawn_blocking(move || catalog.search(&query))
                .await
                .map_err(|error| anyhow!("joining package catalog search: {error}"))
        } else {
            nixbox_nix::search::search(&channel, &query).await
        };
        match result {
            Ok(hits) => {
                let _ = tx.send(AppEvent::SearchDone { epoch, hits }).await;
            }
            Err(e) => {
                let _ = tx
                    .send(AppEvent::SearchFailed {
                        epoch,
                        error: e.to_string(),
                    })
                    .await;
            }
        }
    }));
}

pub(crate) fn schedule_flake_search(app: &mut App, tx: mpsc::Sender<AppEvent>) {
    if let Some(task) = app.flake_search_task.take() {
        task.abort();
    }
    if let Some(task) = app.flake_detail_task.take() {
        task.abort();
    }

    let query = app.flake_input.value().trim().to_string();
    if query.is_empty() {
        app.flake_search_epoch += 1;
        app.flake_detail_epoch += 1;
        app.flake_searching = false;
        app.flake_detail_loading = false;
        app.flake_results.clear();
        app.flake_details = None;
        app.flake_selected = 0;
        app.flake_query.clear();
        app.status = "Search GitHub for flakes by content or project name.".into();
        return;
    }

    app.flake_searching = true;
    app.flake_detail_loading = false;
    app.flake_search_epoch += 1;
    let epoch = app.flake_search_epoch;
    app.flake_query = query.clone();
    app.flake_search_task = Some(tokio::spawn(async move {
        sleep(Duration::from_millis(250)).await;
        match search_flakes(&query).await {
            Ok(hits) => {
                let _ = tx.send(AppEvent::FlakeSearchDone { epoch, hits }).await;
            }
            Err(error) => {
                let _ = tx
                    .send(AppEvent::FlakeSearchFailed {
                        epoch,
                        error: error.to_string(),
                    })
                    .await;
            }
        }
    }));
}

pub(crate) fn schedule_flake_details(app: &mut App, tx: mpsc::Sender<AppEvent>) {
    let Some(hit) = app.flake_results.get(app.flake_selected).cloned() else {
        app.flake_details = None;
        app.flake_detail_loading = false;
        return;
    };
    if let Some(task) = app.flake_detail_task.take() {
        task.abort();
    }

    app.flake_detail_epoch += 1;
    let epoch = app.flake_detail_epoch;
    app.flake_detail_loading = true;
    app.flake_details = None;
    app.flake_detail_task = Some(tokio::spawn(async move {
        match fetch_flake_details(&hit).await {
            Ok(details) => {
                let _ = tx
                    .send(AppEvent::FlakeDetailsDone {
                        epoch,
                        details: Box::new(details),
                    })
                    .await;
            }
            Err(error) => {
                let _ = tx
                    .send(AppEvent::FlakeDetailsFailed {
                        epoch,
                        error: error.to_string(),
                    })
                    .await;
            }
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::vim::VimInput;
    use nixbox_config::Config;
    use nixbox_nix::{
        flakes::{FlakeDetails, FlakePackage},
        manifest::Manifest,
        search::SearchHit,
    };
    use tokio::sync::{mpsc, oneshot};
    use tokio::time::timeout;

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

    #[tokio::test]
    async fn empty_query_clears_results_and_does_not_spawn_search() {
        let (tx, _rx) = mpsc::channel(1);
        let mut app = test_app();
        app.input = VimInput::new(String::new());
        app.results = vec![hit("ripgrep")];
        app.selected = 4;
        app.searching = true;
        app.latest_query = "ripgrep".into();

        schedule_search(&mut app, tx);

        assert_eq!(app.search_epoch, 1);
        assert!(!app.searching);
        assert!(app.results.is_empty());
        assert_eq!(app.selected, 0);
        assert_eq!(app.latest_query, "");
        assert_eq!(app.status, "Type to search packages.");
        assert!(app.search_task.is_none());
    }

    #[tokio::test]
    async fn new_query_aborts_previous_pending_search_task() {
        struct NotifyOnDrop(Option<oneshot::Sender<()>>);

        impl Drop for NotifyOnDrop {
            fn drop(&mut self) {
                if let Some(tx) = self.0.take() {
                    let _ = tx.send(());
                }
            }
        }

        let (tx, _rx) = mpsc::channel(1);
        let (dropped_tx, dropped_rx) = oneshot::channel();
        let mut app = test_app();
        app.input = VimInput::new("ripgrep".into());
        app.search_task = Some(tokio::spawn(async move {
            let _guard = NotifyOnDrop(Some(dropped_tx));
            std::future::pending::<()>().await;
        }));
        tokio::task::yield_now().await;

        schedule_search(&mut app, tx);

        timeout(Duration::from_secs(1), dropped_rx)
            .await
            .expect("old search task should be aborted")
            .expect("drop notification should be sent");
        assert_eq!(app.search_epoch, 1);
        assert!(app.searching);
        assert_eq!(app.latest_query, "ripgrep");
        assert!(app.search_task.is_some());
    }

    #[tokio::test]
    async fn cancel_build_signals_active_rebuild() {
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let mut app = test_app();
        app.build_in_progress = true;
        app.build_cancel = Some(cancel_tx);

        cancel_build(&mut app);

        cancel_rx.await.expect("build should be signalled");
        assert!(app.build_cancel.is_none());
        assert!(app.build_in_progress);
        assert_eq!(app.status, "Cancelling build...");
    }

    #[tokio::test]
    async fn install_flake_queues_its_preferred_package_when_no_module_exists() {
        let (tx, _rx) = mpsc::channel(1);
        let mut app = test_app();
        app.build_in_progress = true;
        app.flake_details = Some(FlakeDetails {
            repo: "alleneubank/bun-overlay".into(),
            repo_url: "https://github.com/alleneubank/bun-overlay".into(),
            path: "flake.nix".into(),
            description: None,
            stars: 0,
            topics: Vec::new(),
            homepage: None,
            default_branch: "main".into(),
            pushed_at: None,
            archived: false,
            inputs: Vec::new(),
            outputs: vec!["packages".into()],
            packages: vec![FlakePackage {
                attr: "default".into(),
                name: "bun".into(),
                version: "1.4.2".into(),
            }],
            nixos_module: None,
            home_manager_module: None,
        });

        install_selected_flake(&mut app, &tx).await.unwrap();

        assert!(matches!(
            app.queue.front(),
            Some(QueuedOp::InstallFlakePackage { repo, package, scope: Target::NixosSystem })
                if repo == "alleneubank/bun-overlay" && package == "default"
        ));
    }

    #[test]
    fn taking_a_scope_batches_all_of_its_pending_operations() {
        let mut app = test_app();
        app.queue.push_back(QueuedOp::Install {
            hit: hit("ripgrep"),
            scope: Target::HomeManager,
        });
        app.queue.push_back(QueuedOp::Install {
            hit: hit("fd"),
            scope: Target::NixosSystem,
        });
        app.queue.push_back(QueuedOp::Uninstall {
            name: "neovim".into(),
            scope: Target::HomeManager,
        });

        let batch = take_queued_scope(&mut app, Target::HomeManager);

        assert_eq!(batch.len(), 2);
        assert!(batch.iter().all(|op| op.scope() == Target::HomeManager));
        assert_eq!(app.queue.len(), 1);
        assert!(matches!(
            app.queue.front(),
            Some(QueuedOp::Install { hit, scope: Target::NixosSystem }) if hit.attr == "fd"
        ));
    }
}
