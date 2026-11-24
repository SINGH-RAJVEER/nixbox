//! The NixOS page's state: the host's option schema, the view on screen
//! (a curated page, the changed settings, or the whole schema), the values
//! evaluated so far, and the edits staged for one queued rebuild.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use gpui_kit::{Context, Task, Window};
use nixbox_config::Target;
use nixbox_core::hardware::HardwareFilter;
use nixbox_core::options::parse_text;
use nixbox_core::settings_pages::{
	SchemaIndex, SearchResults, SettingsPage, ShownSection, TreeRow, View, ancestors,
	changed_paths, namespace_rows, page_sections, search, section_paths, stage_edit,
};
use nixbox_core::system_options::{
	Category, GROUP_LIMIT, default_host, edit_lock, filter_options, sibling_group,
};
use nixbox_core::{Op, OptionChange};
use nixbox_nix::hardware::HardwareInventory;
use nixbox_nix::settings::{OptionPath, SettingValue, option_path_display};
use nixbox_nix::system_options::{
	EditableOption, NixosConfiguration, SystemOption, discover_hosts, discover_options,
	inspect_group,
};
use tokio::task::AbortHandle;

use crate::app::{NixboxApp, Page};
use crate::options::edit_text;

const EVALUATION_LIMIT: Duration = Duration::from_secs(120);

#[derive(Default)]
pub struct NixosView {
	pub applied_host: Option<String>,
	pub initialized: bool,
	pub hosts: Vec<String>,
	pub configuration: Option<NixosConfiguration>,
	pub options: Vec<SystemOption>,
	pub index: SchemaIndex,
	pub view: View,
	/// The open page's sections, resolved against the schema.
	pub sections: Vec<ShownSection>,
	/// The search typed into the sidebar; results replace the view.
	pub query: String,
	pub search: SearchResults,
	/// Options the tree lists: every option for All, configured ones for
	/// Configured.
	pub category: Category,
	pub visible: Vec<usize>,
	pub expanded: BTreeSet<OptionPath>,
	pub rows: Vec<TreeRow>,
	pub show_diagnostic: bool,
	pub hardware: HardwareFilter,
	pub local_hardware: HardwareInventory,
	hardware_configuration: Option<NixosConfiguration>,
	pub selected: Option<OptionPath>,
	/// The selected option's siblings, which All options edits together.
	pub group: Vec<OptionPath>,
	/// Every option evaluated since the schema loaded, by path.
	pub evaluated: HashMap<OptionPath, EditableOption>,
	/// Edits not yet queued; `None` unsets the option.
	pub staged: BTreeMap<OptionPath, Option<SettingValue>>,
	pub loading: bool,
	pub inspecting: bool,
	pub error: Option<String>,
	pub detail_error: Option<String>,
	catalog_epoch: u64,
	detail_epoch: u64,
	catalog_abort: Option<AbortHandle>,
	detail_abort: Option<AbortHandle>,
	catalog_task: Option<Task<()>>,
	detail_task: Option<Task<()>>,
}

impl NixosView {
	fn cancel(&mut self) {
		self.catalog_epoch = self.catalog_epoch.wrapping_add(1);
		if let Some(job) = self.catalog_abort.take() {
			job.abort();
		}
		self.cancel_inspection();
	}

	fn cancel_inspection(&mut self) {
		self.detail_epoch = self.detail_epoch.wrapping_add(1);
		if let Some(job) = self.detail_abort.take() {
			job.abort();
		}
		self.inspecting = false;
		self.detail_error = None;
	}

	pub fn reset(&mut self) {
		self.invalidate();
		self.hosts.clear();
		self.configuration = None;
		self.staged.clear();
		self.hardware.inventory = None;
		self.hardware_configuration = None;
	}

	fn invalidate(&mut self) {
		self.cancel();
		self.initialized = false;
		self.loading = false;
		self.clear_schema();
	}

	fn clear_schema(&mut self) {
		self.options.clear();
		self.index = SchemaIndex::default();
		self.sections.clear();
		self.search = SearchResults::default();
		self.visible.clear();
		self.rows.clear();
		self.selected = None;
		self.group.clear();
		self.evaluated.clear();
		self.error = None;
	}

	/// Installs a freshly read schema and resolves the open page against it.
	pub fn set_options(&mut self, options: Vec<SystemOption>) {
		self.index = SchemaIndex::new(&options);
		self.options = options;
		self.evaluated.clear();
		self.resolve_view();
	}

	/// Resolves the open page's sections against the schema.
	pub fn resolve_view(&mut self) {
		self.sections = match self.view {
			View::Page(page) => page_sections(page, &self.options, &self.index),
			_ => Vec::new(),
		};
	}

	/// Recomputes the tree and the search results.
	pub fn filter(&mut self, text: &str) {
		self.hardware.retained = self.staged.keys().cloned().collect();
		self.query = text.trim().to_string();
		self.visible = filter_options(
			&self.options,
			"",
			self.category,
			self.show_diagnostic,
			&self.hardware,
		)
		.visible;
		self.rows = namespace_rows(&self.options, &self.visible, &self.expanded);
		self.search = search(
			&self.options,
			&self.index,
			&self.query,
			self.show_diagnostic,
			&self.hardware,
		);
	}

	/// Opens or closes a namespace in the tree.
	pub fn toggle_namespace(&mut self, prefix: &[String]) {
		if !self.expanded.remove(prefix) {
			self.expanded.insert(prefix.to_vec());
		}
		self.rows = namespace_rows(&self.options, &self.visible, &self.expanded);
	}

	/// Expands the tree down to `path`.
	pub fn reveal(&mut self, path: &[String]) {
		self.expanded.extend(ancestors(path));
		self.rows = namespace_rows(&self.options, &self.visible, &self.expanded);
	}

	/// The options the open view edits, in display order.
	pub fn view_paths(&self, session: &nixbox_core::Session) -> Vec<OptionPath> {
		match self.view {
			View::Page(_) => section_paths(&self.sections),
			View::Changed => self.changed(session),
			View::Overview | View::AllOptions => self.group.clone(),
		}
	}

	/// Staged and queued paths this schema declares.
	pub fn changed(&self, session: &nixbox_core::Session) -> Vec<OptionPath> {
		changed_paths(&self.staged, session)
			.into_iter()
			.filter(|path| self.index.get(path).is_some())
			.take(GROUP_LIMIT)
			.collect()
	}

	pub fn selected_option(&self) -> Option<&SystemOption> {
		let path = self.selected.as_ref()?;
		self.option(path)
	}

	pub fn option(&self, path: &[String]) -> Option<&SystemOption> {
		self.options.get(self.index.get(path)?)
	}

	/// The evaluated option at `path`, once it has been read.
	pub fn editing(&self, path: &[String]) -> Option<&EditableOption> {
		self.evaluated.get(path)
	}

	pub fn selected_editing(&self) -> Option<&EditableOption> {
		self.editing(self.selected.as_deref()?)
	}

	/// Whether the open host is the one nixbox's NixOS rebuild applies.
	pub fn editable_host(&self) -> bool {
		self.configuration.as_ref().is_some_and(|configuration| {
			configuration.host
				== self
					.applied_host
					.as_deref()
					.unwrap_or(nixbox_core::options::NIXOS_HOST)
		})
	}

	/// Why `option` cannot be changed here, or `None` when it can.
	pub fn lock(&self, option: &EditableOption) -> Option<String> {
		if self.editable_host() {
			edit_lock(option)
		} else {
			Some("host not rebuilt by nixbox".into())
		}
	}

	/// How many of `page`'s settings this host declares.
	pub fn page_size(&self, page: SettingsPage) -> usize {
		page_sections(page, &self.options, &self.index)
			.iter()
			.map(|section| section.entries.len())
			.sum()
	}
}

impl Drop for NixosView {
	fn drop(&mut self) {
		self.cancel();
	}
}

impl NixboxApp {
	pub fn reload_nixos(&mut self, cx: &mut Context<Self>) {
		self.nixos.applied_host = self.session.engine.config.nixos_host.clone();
		self.nixos.cancel();
		let previous = self
			.nixos
			.configuration
			.as_ref()
			.map(|configuration| configuration.host.clone())
			.or_else(|| self.nixos.applied_host.clone());
		self.nixos.initialized = true;
		self.nixos.loading = true;
		self.nixos.clear_schema();
		self.nixos.hosts.clear();
		self.nixos.configuration = None;
		let epoch = self.nixos.catalog_epoch;
		let root = self.session.config_dir();
		let job = self
			.runtime
			.spawn(async move { discover_hosts(&root, EVALUATION_LIMIT).await });
		self.nixos.catalog_abort = Some(job.abort_handle());
		self.nixos.catalog_task = Some(cx.spawn(async move |this, cx| {
			let result = match job.await {
				Ok(result) => result.map_err(|error| format!("{error:#}")),
				Err(error) => Err(error.to_string()),
			};
			let _ = this.update(cx, |this, cx| {
				if epoch != this.nixos.catalog_epoch {
					return;
				}
				this.nixos.loading = false;
				this.nixos.catalog_abort = None;
				match result {
					Ok(hosts) => {
						let host = default_host(&hosts, previous.as_deref());
						if host != previous {
							this.nixos.staged.clear();
						}
						this.nixos.hosts = hosts;
						if let Some(host) = host {
							this.select_nixos_host(&host, cx);
						}
					}
					Err(error) => this.nixos.error = Some(error),
				}
				cx.notify();
			});
		}));
		cx.notify();
	}

	pub fn select_nixos_host(&mut self, host: &str, cx: &mut Context<Self>) {
		if !self.nixos.hosts.iter().any(|known| known == host) {
			return;
		}
		self.nixos.cancel();
		// Staged edits belong to the host they were made for.
		if self
			.nixos
			.configuration
			.as_ref()
			.is_some_and(|configuration| configuration.host != host)
		{
			self.nixos.staged.clear();
		}
		let configuration = NixosConfiguration {
			root: self.session.config_dir(),
			host: host.into(),
		};
		if self.nixos.hardware_configuration.as_ref() != Some(&configuration) {
			self.nixos.hardware.inventory = None;
			self.nixos.hardware_configuration = None;
		}
		self.nixos.configuration = Some(configuration.clone());
		self.nixos.loading = true;
		self.nixos.clear_schema();
		let epoch = self.nixos.catalog_epoch;
		let job = self.runtime.spawn(async move {
			let (catalog, hardware) = tokio::join!(
				discover_options(&configuration, EVALUATION_LIMIT),
				tokio::task::spawn_blocking(HardwareInventory::detect)
			);
			catalog.map(|catalog| (catalog, hardware.unwrap_or_default()))
		});
		self.nixos.catalog_abort = Some(job.abort_handle());
		self.nixos.catalog_task = Some(cx.spawn(async move |this, cx| {
			let result = match job.await {
				Ok(result) => result.map_err(|error| format!("{error:#}")),
				Err(error) => Err(error.to_string()),
			};
			let _ = this.update(cx, |this, cx| {
				if epoch != this.nixos.catalog_epoch {
					return;
				}
				this.nixos.loading = false;
				this.nixos.catalog_abort = None;
				match result {
					Ok((catalog, hardware)) => {
						this.nixos.set_options(catalog.options);
						this.nixos.local_hardware = hardware.clone();
						if this.nixos.hardware_configuration == this.nixos.configuration {
							this.nixos.hardware.inventory = Some(hardware);
						}
						this.refresh_nixos_filter(cx);
						this.evaluate_nixos_view(cx);
					}
					Err(error) => this.nixos.error = Some(error),
				}
				cx.notify();
			});
		}));
		cx.notify();
	}

	/// Shows `view`, reading the values of the settings it lists.
	pub fn open_nixos_view(&mut self, view: View, window: &mut Window, cx: &mut Context<Self>) {
		self.nixos.view = view;
		self.nixos.resolve_view();
		if view != View::AllOptions {
			self.nixos.selected = None;
		}
		if !self.nixos.query.is_empty() {
			self.nixos_filter
				.update(cx, |input, cx| input.set_value("", window, cx));
			self.refresh_nixos_filter(cx);
		}
		self.evaluate_nixos_view(cx);
		self.sync_nixos_input(window, cx);
		cx.notify();
	}

	/// Reads the values of the open view's settings that are not read yet.
	pub fn evaluate_nixos_view(&mut self, cx: &mut Context<Self>) {
		let paths = self.nixos.view_paths(&self.session);
		self.evaluate_nixos(paths, cx);
	}

	/// Evaluates every path in `paths` not evaluated yet, in one request.
	fn evaluate_nixos(&mut self, paths: Vec<OptionPath>, cx: &mut Context<Self>) {
		let Some(configuration) = self.nixos.configuration.clone() else {
			return;
		};
		let missing: Vec<OptionPath> = paths
			.into_iter()
			.filter(|path| !self.nixos.evaluated.contains_key(path))
			.filter(|path| self.nixos.index.get(path).is_some())
			.collect();
		if missing.is_empty() {
			return;
		}
		self.nixos.cancel_inspection();
		self.nixos.inspecting = true;
		let settings_file = self
			.session
			.engine
			.config
			.settings_file_for(Target::NixosSystem)
			.file_name()
			.map(|name| name.to_string_lossy().into_owned())
			.unwrap_or_default();
		let epoch = self.nixos.detail_epoch;
		let job = self.runtime.spawn(async move {
			inspect_group(&configuration, &missing, &settings_file, EVALUATION_LIMIT).await
		});
		self.nixos.detail_abort = Some(job.abort_handle());
		self.nixos.detail_task = Some(cx.spawn(async move |this, cx| {
			let result = match job.await {
				Ok(result) => result.map_err(|error| format!("{error:#}")),
				Err(error) => Err(error.to_string()),
			};
			let _ = this.update(cx, |this, cx| {
				if epoch != this.nixos.detail_epoch {
					return;
				}
				this.nixos.inspecting = false;
				this.nixos.detail_abort = None;
				match result {
					Ok(group) => {
						// The text field shows the selected value once it arrives.
						let selected = this.nixos.selected.as_ref();
						this.nixos_input_dirty |= group
							.iter()
							.any(|option| Some(&option.entry.path) == selected);
						this.nixos.evaluated.extend(
							group
								.into_iter()
								.map(|option| (option.entry.path.clone(), option)),
						);
					}
					Err(error) => this.nixos.detail_error = Some(error),
				}
				cx.notify();
			});
		}));
		cx.notify();
	}

	/// Selects `path`. In All options this reads its siblings too, which the
	/// details pane edits together.
	pub fn select_nixos_option(
		&mut self,
		path: OptionPath,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		if self.nixos.option(&path).is_none() {
			return;
		}
		self.nixos.selected = Some(path.clone());
		let paths = if self.nixos.view == View::AllOptions {
			self.nixos.group = sibling_group(
				&self.nixos.options,
				&path,
				self.nixos.show_diagnostic,
				&self.nixos.hardware,
			);
			self.nixos.group.clone()
		} else {
			vec![path]
		};
		self.evaluate_nixos(paths, cx);
		self.sync_nixos_input(window, cx);
		cx.notify();
	}

	/// Opens a search result: a curated setting on its page, anything else
	/// in All options with the tree expanded to it.
	pub fn open_nixos_result(
		&mut self,
		path: OptionPath,
		page: Option<SettingsPage>,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		match page {
			Some(page) => self.open_nixos_view(View::Page(page), window, cx),
			None => {
				self.open_nixos_view(View::AllOptions, window, cx);
				self.nixos.reveal(&path);
			}
		}
		self.select_nixos_option(path, window, cx);
	}

	/// What `option` is set to once queued work applies: a queued value,
	/// else what evaluated. `None` is an unset or unreadable value.
	pub(crate) fn nixos_baseline(&self, option: &EditableOption) -> Option<SettingValue> {
		match self
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
	pub(crate) fn nixos_current(&self, option: &EditableOption) -> Option<SettingValue> {
		match self.nixos.staged.get(&option.entry.path) {
			Some(staged) => staged.clone(),
			None => self.nixos_baseline(option),
		}
	}

	/// Puts the selected option's value into the text editor.
	pub(crate) fn sync_nixos_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		self.nixos_input_dirty = false;
		let text = self
			.nixos
			.selected_editing()
			.map(|option| edit_text(self.nixos_current(option).as_ref()))
			.unwrap_or_default();
		self.nixos_input
			.update(cx, |input, cx| input.set_value(text, window, cx));
	}

	/// Stages `value` for `path`, or unsetting it when `value` is `None`.
	/// Staging what is already in effect drops the staged change instead.
	pub fn stage_nixos(
		&mut self,
		path: &[String],
		value: Option<SettingValue>,
		cx: &mut Context<Self>,
	) {
		let Some(option) = self.nixos.editing(path) else {
			return;
		};
		if let Some(reason) = self.nixos.lock(option) {
			self.status =
				format!("{} is not editable: {reason}.", option_path_display(path)).into();
			cx.notify();
			return;
		}
		let ours = self
			.session
			.engine
			.sets_option(Target::NixosSystem, &option.entry);
		let baseline = self.nixos_baseline(option);
		self.status =
			stage_edit(&mut self.nixos.staged, path, value, baseline.as_ref(), ours).into();
		cx.notify();
	}

	/// Stages what is typed in the selected option's field, if it reads as
	/// that option's type.
	pub fn stage_nixos_text(&mut self, cx: &mut Context<Self>) {
		let Some(option) = self.nixos.selected_editing() else {
			return;
		};
		let path = option.entry.path.clone();
		let text = self.nixos_input.read(cx).value().to_string();
		match parse_text(&option.entry.kind, &text) {
			Ok(value) => self.stage_nixos(&path, Some(value), cx),
			Err(message) => {
				self.status = message.into();
				cx.notify();
			}
		}
	}

	pub fn discard_nixos(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		let count = self.nixos.staged.len();
		self.nixos.staged.clear();
		self.sync_nixos_input(window, cx);
		self.status = format!("Discarded {count} staged change(s).").into();
		cx.notify();
	}

	/// Queues every staged edit as one NixOS rebuild.
	pub fn apply_nixos(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		if self.repository_blocks_mutation(cx) {
			return;
		}
		if self.nixos.staged.is_empty() {
			self.status = "No staged changes to apply.".into();
			cx.notify();
			return;
		}
		let changes: Vec<OptionChange> = std::mem::take(&mut self.nixos.staged)
			.into_iter()
			.map(|(path, value)| OptionChange { path, value })
			.collect();
		let op = Op::SetOptions {
			changes,
			scope: Target::NixosSystem,
		};
		let duplicate = format!("{} is already queued.", op.label());
		self.enqueue(op, duplicate, window, cx);
		self.sync_nixos_input(window, cx);
	}

	pub fn refresh_nixos_filter(&mut self, cx: &mut Context<Self>) {
		self.nixos
			.filter(self.nixos_filter.read(cx).value().as_ref());
		cx.notify();
	}

	pub fn use_local_nixos_hardware(
		&mut self,
		enabled: bool,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		self.nixos.hardware_configuration =
			enabled.then(|| self.nixos.configuration.clone()).flatten();
		self.nixos.hardware.inventory = self
			.nixos
			.hardware_configuration
			.as_ref()
			.map(|_| self.nixos.local_hardware.clone());
		self.refresh_nixos_hardware(window, cx);
	}

	/// Reapplies the tree's filters, keeping the selection while it stays
	/// visible.
	pub fn refresh_nixos_hardware(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		self.refresh_nixos_filter(cx);
		let hidden = self.nixos.view == View::AllOptions
			&& self.nixos.selected.as_ref().is_some_and(|path| {
				!self
					.nixos
					.visible
					.iter()
					.any(|index| self.nixos.options[*index].path == *path)
			});
		if hidden {
			self.nixos.selected = None;
			self.nixos.group.clear();
			self.sync_nixos_input(window, cx);
		}
	}

	pub(crate) fn nixos_after_build(&mut self, cx: &mut Context<Self>) {
		if self.nixos.initialized {
			self.nixos.invalidate();
			if self.page == Page::Nixos {
				self.reload_nixos(cx);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use gpui_kit::component::Root;
	use gpui_kit::test::{TestAppContextExt as _, TestWindowExt as _};
	use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowHandle, px, size};
	use nixbox_config::{Config, Target};
	use nixbox_core::options::NIXOS_HOST;
	use nixbox_core::{Engine, Session};
	use nixbox_nix::manifest::Manifest;
	use nixbox_nix::options::{OptionEntry, OptionKind};
	use nixbox_nix::system_options::{InspectedValue, SystemOptionDetails};

	fn path(raw: &str) -> OptionPath {
		raw.split('.').map(str::to_string).collect()
	}

	fn editable(raw: &str, kind: OptionKind, value: serde_json::Value) -> EditableOption {
		let path = path(raw);
		EditableOption {
			details: SystemOptionDetails {
				path: path.clone(),
				value: InspectedValue::Available(value.clone()),
				default: None,
				example: None,
				definitions: Some(Vec::new()),
			},
			entry: OptionEntry {
				path,
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

	fn option(raw: &str, description: &str) -> SystemOption {
		SystemOption {
			path: path(raw),
			configured: Some(false),
			description: Some(description.into()),
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

	/// The app on the NixOS page of `host`, with `options` as its schema and
	/// `evaluated` already read, so no test runs an evaluation.
	fn open(
		cx: &mut TestAppContext,
		config: Config,
		host: &str,
		options: Vec<SystemOption>,
		evaluated: Vec<EditableOption>,
	) -> (Entity<NixboxApp>, WindowHandle<Root>) {
		cx.update(gpui_kit::init);
		let runtime = tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.expect("runtime");
		let (session, build_rx) = Session::new(Engine::from_parts(
			config,
			Manifest::default(),
			Manifest::default(),
			Vec::new(),
		));
		let session = session
			.without_persistence()
			.with_runtime(runtime.handle().clone());
		let mut app = None;
		let handle = cx.open_window(size(px(1280.), px(860.)), |window, cx| {
			let view = cx
				.new(|cx| NixboxApp::new(session, build_rx, runtime.handle().clone(), window, cx));
			app = Some(view.clone());
			Root::new(view, window, cx)
		});
		// The runtime is never driven, so evaluations a test does not
		// pre-fill stay pending instead of running Nix.
		std::mem::forget(runtime);
		let app = app.expect("view");
		cx.update_window(handle.into(), |_, window, cx| {
			app.update(cx, |app, cx| {
				app.nixos.initialized = true;
				app.nixos.hosts = vec![host.into()];
				app.nixos.configuration = Some(NixosConfiguration {
					root: app.session.config_dir(),
					host: host.into(),
				});
				app.nixos.set_options(options);
				app.nixos.evaluated = evaluated
					.into_iter()
					.map(|option| (option.entry.path.clone(), option))
					.collect();
				app.nixos.filter("");
				app.set_page(Page::Nixos, window, cx);
			});
			window.render_frame(cx);
		})
		.expect("window");
		(app, handle)
	}

	#[gpui_kit::test]
	async fn nixos_page_navigates_pages_tree_and_search(cx: &mut TestAppContext) {
		let config = Config {
			target: Target::HomeManager,
			..Config::default()
		};
		let mut configured = option("services.pipewire.enable", "Sound server");
		configured.configured = Some(true);
		let options = vec![
			configured,
			option("custom.welcome", "Welcome message"),
			option("services.printing.enable", "Printing"),
		];
		let evaluated = vec![
			editable(
				"services.pipewire.enable",
				OptionKind::Bool,
				serde_json::json!(true),
			),
			editable("custom.welcome", OptionKind::Bool, serde_json::json!(false)),
		];
		let (app, handle) = open(cx, config, "workstation", options, evaluated);
		cx.update_window(handle.into(), |_, window, cx| {
			assert_eq!(app.read(cx).nixos.view, View::Overview);
			window.click(("nixos-page", SettingsPage::Audio.index()), cx);
			window.render_frame(cx);
			app.read_with(cx, |app, _| {
				assert_eq!(app.nixos.view, View::Page(SettingsPage::Audio));
				assert_eq!(app.nixos.sections.len(), 1, "Only declared settings show");
				assert_eq!(app.nixos.sections[0].entries[0].label, "PipeWire");
			});
			assert!(window.find(("nixos-setting", 0_usize)).visible());

			app.update(cx, |app, cx| {
				app.open_nixos_view(View::AllOptions, window, cx)
			});
			window.render_frame(cx);
			app.read_with(cx, |app, _| {
				assert_eq!(
					app.nixos.rows.len(),
					2,
					"custom holds one option, services folds two"
				);
			});
			window.click(("nixos-option", 1_usize), cx);
			window.render_frame(cx);
			assert!(window.find(("nixos-option", 3_usize)).visible());
			app.update(cx, |app, cx| {
				app.nixos.category = Category::Configured;
				app.refresh_nixos_hardware(window, cx);
				assert_eq!(app.nixos.visible, [0]);
				app.nixos.category = Category::All;
				app.refresh_nixos_hardware(window, cx);
				app.nixos_filter
					.update(cx, |input, cx| input.set_value("welcome", window, cx));
				app.refresh_nixos_filter(cx);
				assert_eq!(app.nixos.search.options, [1]);
			});
			window.render_frame(cx);
			window.click(("nixos-result", 0_usize), cx);
			window.render_frame(cx);
			app.update(cx, |app, cx| {
				assert!(
					app.nixos.query.is_empty(),
					"Opening a result clears the search"
				);
				assert_eq!(app.nixos.view, View::AllOptions);
				assert_eq!(app.nixos.selected, Some(path("custom.welcome")));
				app.set_page(Page::Installed, window, cx);
				app.set_page(Page::Nixos, window, cx);
				app.focus_search(&crate::app::FocusSearch, window, cx);
				assert_eq!(app.page, Page::Nixos, "Ctrl-F must stay on the NixOS tab");
				assert_eq!(app.nixos.selected, Some(path("custom.welcome")));
				assert_eq!(app.session.engine.config.target, Target::HomeManager);
				assert!(app.session.queue.is_empty());
				assert!(
					app.options.is_none(),
					"Package options have separate view state"
				);
				assert!(
					app.nixos
						.lock(app.nixos.selected_editing().unwrap())
						.is_some(),
					"Only the host nixbox rebuilds is editable"
				);
			});
			window.render_frame(cx);
			assert!(window.find("nixos-details").visible());
			assert!(window.try_find("nixos-apply").is_none());
		})
		.expect("window");
		cx.simulate_window_resize(handle.into(), size(px(780.), px(640.)));
		cx.update_window(handle.into(), |_, window, cx| {
			window.render_frame(cx);
			assert!(window.find("nixos-back").visible());
			let bounds = window.find("nixos-details").bounds();
			assert!(
				bounds.size.width > px(700.),
				"Small windows show full-width details"
			);
			window.click("nixos-back", cx);
			window.render_frame(cx);
			assert!(window.find(("nixos-option", 0_usize)).visible());
			assert!(window.try_find("nixos-details").is_none());
			assert!(
				window.find("nixos-views").visible(),
				"The sidebar folds into a menu"
			);
		})
		.expect("small window");
	}

	#[gpui_kit::test]
	async fn nixos_switches_and_dropdowns_stage_values_from_clicks(cx: &mut TestAppContext) {
		let options = vec![
			option("custom.enable", "Enable the service"),
			option("custom.mode", "Service mode"),
			option("custom.optional", "Optional flag"),
		];
		let evaluated = vec![
			editable("custom.enable", OptionKind::Bool, serde_json::json!(false)),
			editable(
				"custom.mode",
				OptionKind::Enum(vec!["auto".into(), "manual".into()]),
				serde_json::json!("auto"),
			),
			editable(
				"custom.optional",
				OptionKind::Nullable(Box::new(OptionKind::Bool)),
				serde_json::Value::Null,
			),
		];
		let (app, handle) = open(cx, Config::default(), NIXOS_HOST, options, evaluated);
		let current = |app: &NixboxApp, raw: &str| {
			app.nixos_current(app.nixos.editing(&path(raw)).expect("evaluated"))
		};
		cx.update_window(handle.into(), |_, window, cx| {
			app.update(cx, |app, cx| {
				app.open_nixos_view(View::AllOptions, window, cx);
				app.select_nixos_option(path("custom.enable"), window, cx);
				assert_eq!(app.nixos.group.len(), 3, "Siblings are edited together");
			});
			window.render_frame(cx);
			window.click(("nixos-bool", 0_usize), cx);
			app.read_with(cx, |app, _| {
				assert_eq!(
					current(app, "custom.enable"),
					Some(SettingValue::Bool(true))
				);
				assert!(
					app.session.queue.is_empty(),
					"A toggle only stages the edit"
				);
			});
			window.click(("nixos-bool", 0_usize), cx);
			app.read_with(cx, |app, _| assert!(app.nixos.staged.is_empty()));
			window.click(("nixos-menu", 1_usize), cx);
			assert_eq!(
				window.within("popup-menu").find(1_usize).label(),
				Some("manual")
			);
			window.within("popup-menu").click(1_usize, cx);
			window.render_frame(cx);
			app.read_with(cx, |app, _| {
				assert_eq!(
					current(app, "custom.mode"),
					Some(SettingValue::Str("manual".into()))
				);
				assert!(
					app.session.queue.is_empty(),
					"A dropdown only stages the edit"
				);
			});
		})
		.expect("enum window");
		cx.wait_for(
			handle.into(),
			std::time::Duration::from_secs(1),
			|window, _| window.try_find("popup-menu").is_none(),
		)
		.await;
		cx.update_window(handle.into(), |_, window, cx| {
			window.click(("nixos-menu", 2_usize), cx);
			let labels: Vec<_> = (0_usize..3)
				.map(|index| {
					window
						.within("popup-menu")
						.find(index)
						.label()
						.map(str::to_owned)
				})
				.collect();
			let unset = labels
				.iter()
				.position(|label| label.as_deref() == Some("null"))
				.expect("Nullable flags retain an explicit null choice");
			let enabled = labels
				.iter()
				.position(|label| label.as_deref() == Some("true"))
				.expect("true choice");
			assert_ne!(unset, enabled);
			window.within("popup-menu").click(enabled, cx);
			window.render_frame(cx);
			app.read_with(cx, |app, _| {
				assert_eq!(
					current(app, "custom.optional"),
					Some(SettingValue::Bool(true))
				);
				assert_eq!(app.nixos.staged.len(), 2);
			});
		})
		.expect("nullable window");
	}

	#[gpui_kit::test]
	async fn nixos_edits_stage_review_and_queue_one_system_rebuild(cx: &mut TestAppContext) {
		let options = vec![
			option("networking.hostName", "Host name"),
			option("services.printing.enable", "Printing"),
			option("time.timeZone", "Time zone"),
		];
		let mut owned = editable(
			"time.timeZone",
			OptionKind::Nullable(Box::new(OptionKind::Str)),
			serde_json::json!("UTC"),
		);
		owned.entry.defined_in = vec!["configuration.nix".into()];
		let evaluated = vec![
			editable(
				"networking.hostName",
				OptionKind::Str,
				serde_json::json!("nixos"),
			),
			editable(
				"services.printing.enable",
				OptionKind::Bool,
				serde_json::json!(false),
			),
			owned,
		];
		let (app, handle) = open(cx, Config::default(), NIXOS_HOST, options, evaluated);
		cx.update_window(handle.into(), |_, window, cx| {
			app.update(cx, |app, cx| {
				let _cancel = app.session.fake_build(Target::HomeManager, "busy");
				app.open_nixos_view(View::Page(SettingsPage::General), window, cx);
				assert_eq!(
					section_paths(&app.nixos.sections),
					[path("networking.hostName"), path("time.timeZone")]
				);
				app.select_nixos_option(path("networking.hostName"), window, cx);
				assert_eq!(app.nixos_input.read(cx).value().as_ref(), "nixos");

				app.stage_nixos(&path("time.timeZone"), Some(SettingValue::Null), cx);
				assert!(app.nixos.staged.is_empty(), "User definitions stay locked");
				app.nixos_input
					.update(cx, |input, cx| input.set_value("desk", window, cx));
				app.stage_nixos_text(cx);
				app.stage_nixos(
					&path("services.printing.enable"),
					Some(SettingValue::Bool(true)),
					cx,
				);
				assert_eq!(app.nixos.staged.len(), 2);
				app.stage_nixos(
					&path("services.printing.enable"),
					Some(SettingValue::Bool(false)),
					cx,
				);
				assert_eq!(
					app.nixos.staged.len(),
					1,
					"Restoring the evaluated value drops the edit"
				);
			});
			window.render_frame(cx);
			assert!(window.find(("nixos-setting", 0_usize)).visible());
			assert!(window.find(("nixos-open-source", 1_usize)).visible());
			app.update(cx, |app, cx| app.open_nixos_view(View::Changed, window, cx));
			window.render_frame(cx);
			assert!(window.find(("nixos-setting", 0_usize)).visible());
			assert!(window.try_find(("nixos-setting", 1_usize)).is_none());
			window.click("nixos-review", cx);
			window.render_frame(cx);
		})
		.expect("window");
		// The sheet slides in; wait until its Apply button is on screen.
		cx.wait_for(
			handle.into(),
			std::time::Duration::from_secs(2),
			|window, _| {
				window.try_find("nixos-review-apply").is_some_and(|button| {
					let bounds = button.bounds();
					bounds.origin.x + bounds.size.width <= px(1280.)
				})
			},
		)
		.await;
		cx.update_window(handle.into(), |_, window, cx| {
			window.click("nixos-review-apply", cx);
			window.render_frame(cx);
		})
		.expect("review");
		app.read_with(cx, |app, _| {
			let Some(Op::SetOptions { changes, scope }) = app.session.queue.front() else {
				panic!("expected one settings op");
			};
			assert_eq!(*scope, Target::NixosSystem);
			assert_eq!(
				changes,
				&[OptionChange {
					path: path("networking.hostName"),
					value: Some(SettingValue::Str("desk".into())),
				}]
			);
			assert!(app.nixos.staged.is_empty());
			assert_eq!(
				app.nixos.changed(&app.session),
				[path("networking.hostName")],
				"Queued edits stay in the Changed view until the rebuild"
			);
			let name = app
				.nixos
				.editing(&path("networking.hostName"))
				.expect("host name");
			assert_eq!(
				app.nixos_current(name),
				Some(SettingValue::Str("desk".into())),
				"Queued values stay in the controls until the rebuild"
			);
		});
	}
}
