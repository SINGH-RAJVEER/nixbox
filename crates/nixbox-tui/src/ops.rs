use std::time::Duration;

use anyhow::Result;
use nixbox_config::Target;
use nixbox_core::{
	Enqueued, InstalledFlake, flake_choices,
	search::{catalog_cache_path, search_packages},
};
use nixbox_nix::{
	flakes::{fetch_flake_details, search_flakes},
	scan::ScanTarget,
	search::PackageCatalog,
};
use tokio::sync::mpsc;
use tokio::time::sleep;

use crate::app::{App, AppEvent, InstalledCursor, QueuedOp, Tab};

pub(crate) fn cancel_build(app: &mut App) {
	if app.session.cancel_build() {
		app.status = "Cancelling build...".into();
	}
}

/// Hands `op` to the session and reports what became of it: started, queued
/// behind the running rebuild, or already waiting.
fn enqueue(app: &mut App, op: QueuedOp, queued: String, duplicate: String) {
	match app.session.enqueue(op) {
		Enqueued::Duplicate => app.status = duplicate,
		Enqueued::Queued => {
			app.status = queued;
			app.tab = Tab::Queue;
		}
		Enqueued::Started(label) => {
			app.status = format!("{label}...");
			app.tab = Tab::Building;
		}
		Enqueued::NotWritten => {
			app.status = "The queued changes could not be written; see the build log.".into();
			app.tab = Tab::Building;
		}
	}
	app.clamp_installed_selection();
}

pub(crate) async fn install_selected(app: &mut App, tx: &mpsc::Sender<AppEvent>) -> Result<()> {
	if app.package_catalog.as_ref().is_some_and(|catalog| {
		!catalog
			.is_current_for(&app.session.engine.config.home_manager_dir())
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

	let scope = app.session.engine.config.target;
	let attr = hit.attr.clone();
	if app.manifest_for(scope).packages.contains(&attr) {
		app.status = format!("{attr} already tracked or queued.");
		return Ok(());
	}
	enqueue(
		app,
		QueuedOp::Install { hit, scope },
		format!("Queued install: {attr}."),
		format!("{attr} already tracked or queued."),
	);
	Ok(())
}

pub(crate) async fn install_selected_flake(
	app: &mut App,
	_tx: &mpsc::Sender<AppEvent>,
) -> Result<()> {
	let Some(details) = app.flake_details.clone() else {
		app.status = "Wait for flake details before installing.".into();
		return Ok(());
	};
	let scope = app.session.engine.config.target;
	let choices = flake_choices(&details, scope);
	if choices.len() > 1 {
		app.flake_picker = Some(0);
		app.status = format!("Choose an output from {}.", details.repo);
		return Ok(());
	}
	let Some(choice) = choices.first() else {
		app.status = format!(
			"{} has no installable package for this system or compatible {} module.",
			details.repo,
			scope.label()
		);
		return Ok(());
	};
	enqueue(
		app,
		choice.operation(details.repo.clone(), scope),
		format!("Queued {}#{}.", details.repo, choice.path),
		format!("{}#{} is already queued.", details.repo, choice.path),
	);
	Ok(())
}

pub(crate) fn confirm_flake_choice(app: &mut App) {
	let Some(index) = app.flake_picker.take() else {
		return;
	};
	let Some(details) = app.flake_details.clone() else {
		return;
	};
	let scope = app.session.engine.config.target;
	let Some(choice) = flake_choices(&details, scope).get(index).cloned() else {
		return;
	};
	enqueue(
		app,
		choice.operation(details.repo.clone(), scope),
		format!("Queued {}#{}.", details.repo, choice.path),
		format!("{}#{} is already queued.", details.repo, choice.path),
	);
}

pub(crate) async fn uninstall_selected(app: &mut App, _tx: &mpsc::Sender<AppEvent>) -> Result<()> {
	let Some(cursor) = app.installed_cursor() else {
		app.status = "No selection.".into();
		return Ok(());
	};
	let pkg = match cursor {
		InstalledCursor::Managed(p) => p,
		InstalledCursor::Flake(flake) => {
			uninstall_flake(app, flake);
			return Ok(());
		}
		InstalledCursor::External(ep) => {
			app.status = format!(
				"{} is external (in {}) — press m to migrate first.",
				ep.name, ep.source_attr,
			);
			return Ok(());
		}
	};

	let scope = pkg.scope;
	let name = pkg.name;
	enqueue(
		app,
		QueuedOp::Uninstall {
			name: name.clone(),
			scope,
		},
		format!("Queued remove: {} [{}].", name, scope.tag()),
		format!("{name} already queued for removal."),
	);
	Ok(())
}

/// Queues removal of one flake output. The engine takes it out of whichever
/// file declares it and drops the flake's input once nothing uses it.
fn uninstall_flake(app: &mut App, flake: InstalledFlake) {
	let name = flake.name();
	if !flake.removable {
		app.status = format!(
			"{} shares a line in {} with other entries; remove it manually.",
			name,
			flake.declared_in.as_deref().unwrap_or("your config"),
		);
		return;
	}
	let scope = flake.scope;
	enqueue(
		app,
		QueuedOp::UninstallFlakeOutput {
			input: flake.input,
			output: flake.output,
			scope,
		},
		format!("Queued remove: {} [{}].", name, scope.tag()),
		format!("{name} already queued for removal."),
	);
}

pub(crate) async fn migrate_selected(app: &mut App, _tx: &mpsc::Sender<AppEvent>) -> Result<()> {
	let Some(cursor) = app.installed_cursor() else {
		app.status = "No selection.".into();
		return Ok(());
	};
	let ep = match cursor {
		InstalledCursor::External(ep) => ep,
		InstalledCursor::Managed(p) => {
			app.status = format!("{} is already managed — press d to uninstall.", p.name);
			return Ok(());
		}
		InstalledCursor::Flake(flake) => {
			let name = flake.name();
			if !flake.migratable() {
				app.status = format!(
					"{name} cannot be migrated automatically. It needs a dedicated package line and a GitHub input."
				);
				return Ok(());
			}
			if let nixbox_nix::manifest::FlakeOutput::Package(package) = flake.output {
				enqueue(
					app,
					QueuedOp::MigrateFlakePackage {
						input: flake.input,
						package,
						scope: flake.scope,
					},
					format!("Queued migrate: {name} [{}].", flake.scope.tag()),
					format!("{name} already queued for migration."),
				);
			}
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
	let name = ep.name;
	enqueue(
		app,
		QueuedOp::Migrate {
			names: vec![name.clone()],
			scope,
		},
		format!("Queued migrate: {} [{}].", name, scope.tag()),
		format!("{name} already queued for migration."),
	);
	Ok(())
}

pub(crate) async fn migrate_all(app: &mut App, _tx: &mpsc::Sender<AppEvent>) -> Result<()> {
	let ops = app.session.migrate_all_ops();
	if ops.is_empty() {
		app.status = "No external packages or flake outputs left to migrate.".into();
		return Ok(());
	}
	let count = |target: Target| {
		ops.iter()
			.filter_map(|op| match op {
				QueuedOp::Migrate { names, scope } if *scope == target => Some(names.len()),
				QueuedOp::MigrateFlakePackage { scope, .. } if *scope == target => Some(1),
				_ => None,
			})
			.sum::<usize>()
	};
	let summary = format!(
		"Queued migrate-all ({} hm, {} nixos).",
		count(Target::HomeManager),
		count(Target::NixosSystem),
	);
	let outcome = app.session.enqueue_all(ops);
	app.tab = if matches!(outcome, Enqueued::Queued) {
		Tab::Queue
	} else {
		Tab::Building
	};
	app.status = summary;
	app.clamp_installed_selection();
	Ok(())
}

pub(crate) fn prepare_package_catalog(app: &mut App, tx: mpsc::Sender<AppEvent>) {
	if let Some(task) = app.catalog_task.take() {
		task.abort();
	}

	let Some(cache_path) = catalog_cache_path() else {
		app.catalog_loading = false;
		return;
	};
	let config_dir = app.session.engine.config.home_manager_dir();
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
	let channel = app.session.engine.config.channel.clone();
	app.latest_query = query.clone();

	if app.catalog_loading {
		app.status = "Preparing package catalog for your locked nixpkgs revision...".into();
		return;
	}

	if app.package_catalog.as_ref().is_some_and(|catalog| {
		!catalog
			.is_current_for(&app.session.engine.config.home_manager_dir())
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
		match search_packages(catalog, &channel, query).await {
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
	app.flake_picker = None;
	app.flake_details_scroll = 0;
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
	use nixbox_core::Op;
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
		let mut app = test_app();
		let cancel_rx = app.session.fake_build(Target::HomeManager, "build");

		cancel_build(&mut app);

		cancel_rx.await.expect("build should be signalled");
		assert!(app.session.is_building());
		assert_eq!(app.status, "Cancelling build...");
	}

	#[tokio::test]
	async fn installing_during_a_rebuild_queues_behind_it() {
		let (tx, _rx) = mpsc::channel(1);
		let mut app = test_app();
		let _cancel = app.session.fake_build(Target::HomeManager, "build");
		app.results = vec![hit("ripgrep")];

		install_selected(&mut app, &tx).await.unwrap();
		install_selected(&mut app, &tx).await.unwrap();

		assert_eq!(app.session.queue.len(), 1);
		assert_eq!(app.tab, Tab::Queue);
		assert_eq!(app.status, "ripgrep already tracked or queued.");
	}

	#[tokio::test]
	async fn flake_picker_queues_only_the_confirmed_output() {
		let (tx, _rx) = mpsc::channel(1);
		let mut app = test_app();
		let _cancel = app.session.fake_build(Target::NixosSystem, "build");
		app.flake_details = Some(FlakeDetails {
			repo: "owner/repo".into(),
			repo_url: String::new(),
			path: "flake.nix".into(),
			description: None,
			stars: 0,
			topics: Vec::new(),
			homepage: None,
			default_branch: "main".into(),
			pushed_at: None,
			archived: false,
			inputs: Vec::new(),
			outputs: Vec::new(),
			output_entries: Vec::new(),
			system: "x86_64-linux".into(),
			packages: vec![FlakePackage {
				attr: "default".into(),
				name: "pkg".into(),
				version: String::new(),
			}],
			modules: vec!["nixosModules.server".into()],
			nixos_module: None,
			home_manager_module: None,
		});

		install_selected_flake(&mut app, &tx).await.unwrap();
		assert_eq!(app.flake_picker, Some(0));
		assert!(app.session.queue.is_empty());
		app.flake_picker = Some(1);
		confirm_flake_choice(&mut app);
		assert_eq!(app.flake_picker, None);
		assert_eq!(app.session.queue.len(), 1);
		assert!(
			matches!(app.session.queue.front(), Some(Op::InstallFlake { module, .. }) if module == "nixosModules.server")
		);
	}
}
