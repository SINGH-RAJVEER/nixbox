//! The NixOS page's state: the host's option schema, the group of options
//! around the selected one with their values and owners, and the edits
//! staged for one queued rebuild.

use std::collections::BTreeMap;
use std::time::Duration;

use gpui_kit::{Context, Task, Window};
use nixbox_config::Target;
use nixbox_core::hardware::HardwareFilter;
use nixbox_core::options::parse_text;
use nixbox_core::system_options::{
	Category, default_host, edit_lock, filter_options, host_is_editable, sibling_group,
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
	pub initialized: bool,
	pub hosts: Vec<String>,
	pub configuration: Option<NixosConfiguration>,
	pub options: Vec<SystemOption>,
	pub visible: Vec<usize>,
	/// Matches per category for the current search, in [`Category::ALL`] order.
	pub counts: [usize; Category::ALL.len()],
	pub category: Category,
	pub show_diagnostic: bool,
	pub hardware: HardwareFilter,
	pub local_hardware: HardwareInventory,
	hardware_configuration: Option<NixosConfiguration>,
	pub selected: Option<OptionPath>,
	/// The selected option and its siblings, as evaluated, in schema order.
	pub group: Vec<EditableOption>,
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
		self.group.clear();
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
		self.options.clear();
		self.visible.clear();
		self.counts = Default::default();
		self.selected = None;
		self.error = None;
	}

	pub fn filter(&mut self, text: &str) {
		self.hardware.retained = self.staged.keys().cloned().collect();
		let filtered = filter_options(
			&self.options,
			text,
			self.category,
			self.show_diagnostic,
			&self.hardware,
		);
		self.visible = filtered.visible;
		self.counts = filtered.counts;
	}

	pub fn selected_option(&self) -> Option<&SystemOption> {
		let path = self.selected.as_ref()?;
		self.option(path)
	}

	pub fn option(&self, path: &[String]) -> Option<&SystemOption> {
		self.options.iter().find(|option| option.path == path)
	}

	/// The evaluated option at `path`, once its group has been read.
	pub fn editing(&self, path: &[String]) -> Option<&EditableOption> {
		self.group.iter().find(|option| option.details.path == path)
	}

	pub fn selected_editing(&self) -> Option<&EditableOption> {
		self.editing(self.selected.as_deref()?)
	}

	/// Whether the open host is the one nixbox's NixOS rebuild applies.
	pub fn editable_host(&self) -> bool {
		self.configuration
			.as_ref()
			.is_some_and(|configuration| host_is_editable(&configuration.host))
	}

	/// Why `option` cannot be changed here, or `None` when it can.
	pub fn lock(&self, option: &EditableOption) -> Option<String> {
		if self.editable_host() {
			edit_lock(option)
		} else {
			Some("host not rebuilt by nixbox".into())
		}
	}
}

impl Drop for NixosView {
	fn drop(&mut self) {
		self.cancel();
	}
}

impl NixboxApp {
	pub fn reload_nixos(&mut self, cx: &mut Context<Self>) {
		self.nixos.cancel();
		let previous = self
			.nixos
			.configuration
			.as_ref()
			.map(|configuration| configuration.host.clone());
		self.nixos.initialized = true;
		self.nixos.loading = true;
		self.nixos.error = None;
		self.nixos.options.clear();
		self.nixos.visible.clear();
		self.nixos.counts = Default::default();
		self.nixos.selected = None;
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
		self.nixos.error = None;
		self.nixos.options.clear();
		self.nixos.visible.clear();
		self.nixos.counts = Default::default();
		self.nixos.selected = None;
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
						this.nixos.options = catalog.options;
						this.nixos.local_hardware = hardware.clone();
						if this.nixos.hardware_configuration == this.nixos.configuration {
							this.nixos.hardware.inventory = Some(hardware);
						}
						this.refresh_nixos_filter(cx);
					}
					Err(error) => this.nixos.error = Some(error),
				}
				cx.notify();
			});
		}));
		cx.notify();
	}

	/// Selects `path`. An option outside the group on screen reads its
	/// siblings' values and owners in one evaluation.
	pub fn select_nixos_option(
		&mut self,
		path: OptionPath,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		let Some(configuration) = self.nixos.configuration.clone() else {
			return;
		};
		if self.nixos.option(&path).is_none() {
			return;
		}
		self.nixos.selected = Some(path.clone());
		if self.nixos.editing(&path).is_some() {
			self.sync_nixos_input(window, cx);
			cx.notify();
			return;
		}
		self.nixos.cancel_inspection();
		self.nixos.inspecting = true;
		let paths = sibling_group(
			&self.nixos.options,
			&path,
			self.nixos.show_diagnostic,
			&self.nixos.hardware,
		);
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
			inspect_group(&configuration, &paths, &settings_file, EVALUATION_LIMIT).await
		});
		self.nixos.detail_abort = Some(job.abort_handle());
		self.nixos.detail_task = Some(cx.spawn_in(window, async move |this, cx| {
			let result = match job.await {
				Ok(result) => result.map_err(|error| format!("{error:#}")),
				Err(error) => Err(error.to_string()),
			};
			let _ = this.update_in(cx, |this, window, cx| {
				if epoch != this.nixos.detail_epoch {
					return;
				}
				this.nixos.inspecting = false;
				this.nixos.detail_abort = None;
				match result {
					Ok(group) => {
						this.nixos.group = group;
						this.sync_nixos_input(window, cx);
					}
					Err(error) => this.nixos.detail_error = Some(error),
				}
				cx.notify();
			});
		}));
		cx.notify();
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
		let name = option_path_display(path);
		if let Some(reason) = self.nixos.lock(option) {
			self.status = format!("{name} is not editable: {reason}.").into();
			cx.notify();
			return;
		}
		let ours = self
			.session
			.engine
			.sets_option(Target::NixosSystem, &option.entry);
		let baseline = self.nixos_baseline(option);
		let path = path.to_vec();
		self.status = match value {
			Some(value) if Some(&value) == baseline.as_ref() => {
				self.nixos.staged.remove(&path);
				format!("{name} is back to {}.", value.to_nix())
			}
			Some(value) => {
				let text = format!("Staged {name} = {}.", value.to_nix());
				self.nixos.staged.insert(path, Some(value));
				text
			}
			None if ours => {
				self.nixos.staged.insert(path, None);
				format!("Staged unset {name}.")
			}
			None => {
				self.nixos.staged.remove(&path);
				format!("Dropped the staged change to {name}.")
			}
		}
		.into();
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

	pub fn refresh_nixos_hardware(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		let selected = self.nixos.selected.take();
		self.nixos.cancel_inspection();
		self.refresh_nixos_filter(cx);
		if let Some(path) = selected.filter(|path| {
			self.nixos
				.visible
				.iter()
				.any(|index| self.nixos.options[*index].path == *path)
		}) {
			self.select_nixos_option(path, window, cx);
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
	use gpui_kit::{AppContext as _, TestAppContext, px, size};
	use nixbox_config::{Config, Target};
	use nixbox_core::options::NIXOS_HOST;
	use nixbox_core::{Engine, Session};
	use nixbox_nix::manifest::Manifest;
	use nixbox_nix::options::{OptionEntry, OptionKind};
	use nixbox_nix::system_options::{InspectedValue, SystemOptionDetails};

	fn editable(path: &str, kind: OptionKind, value: serde_json::Value) -> EditableOption {
		let path: OptionPath = path.split('.').map(str::to_string).collect();
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

	fn option(path: &str, description: &str, diagnostic: bool) -> SystemOption {
		SystemOption {
			path: path.split('.').map(str::to_string).collect(),
			configured: Some(false),
			description: Some(description.into()),
			type_description: "boolean".into(),
			declarations: vec!["configuration.nix".into()],
			diagnostic,
			read_only: false,
			nested: false,
			template: false,
			children_unavailable: false,
			children_truncated: false,
		}
	}

	#[gpui_kit::test]
	async fn nixos_tab_renders_filters_and_preserves_inspection_across_tabs(
		cx: &mut TestAppContext,
	) {
		cx.update(gpui_kit::init);
		let runtime = tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.expect("runtime");
		let config = Config {
			target: Target::HomeManager,
			..Config::default()
		};
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
		let handle = cx.open_window(size(px(1180.), px(760.)), |window, cx| {
			let view = cx
				.new(|cx| NixboxApp::new(session, build_rx, runtime.handle().clone(), window, cx));
			app = Some(view.clone());
			Root::new(view, window, cx)
		});
		let app = app.expect("view");
		cx.update_window(handle.into(), |_, window, cx| {
			app.update(cx, |app, _| {
				app.nixos.initialized = true;
				app.nixos.hosts = vec!["workstation".into()];
				app.nixos.configuration = Some(NixosConfiguration {
					root: app.session.config_dir(),
					host: "workstation".into(),
				});
				app.nixos.options = vec![
					option("services.pipewire.enable", "Sound server", false),
					option("custom.welcome", "Welcome message", false),
				];
				app.nixos.filter("");
			});
			window.render_frame(cx);
			window.within("page-tabs").click(3_usize, cx);
			window.render_frame(cx);
			assert!(window.find(("nixos-option", 0_usize)).visible());
			assert!(window.find(("nixos-option", 1_usize)).visible());
			app.update(cx, |app, cx| {
				app.nixos
					.options
					.push(option("hardware.nvidia.open", "NVIDIA", false));
				app.nixos.local_hardware.nvidia_gpu = nixbox_nix::hardware::Presence::Absent;
				app.use_local_nixos_hardware(true, window, cx);
				assert_eq!(app.nixos.visible.len(), 2);
				app.nixos.hardware.show_all = true;
				app.refresh_nixos_hardware(window, cx);
				assert_eq!(app.nixos.visible.len(), 3);
				app.nixos.hardware.show_all = false;
				app.use_local_nixos_hardware(false, window, cx);
				assert_eq!(app.nixos.visible.len(), 3);
				app.nixos.options.pop();
				app.refresh_nixos_filter(cx);
			});
			window.render_frame(cx);
			let row = window.find(("nixos-option", 0_usize)).bounds();
			assert!(
				row.size.width > px(200.) && row.size.height > px(0.),
				"Schema rows need a real viewport"
			);
			app.update(cx, |app, cx| {
				assert_eq!(app.page, Page::Nixos);
				app.nixos_filter
					.update(cx, |input, cx| input.set_value("welcome", window, cx));
				app.refresh_nixos_filter(cx);
			});
			window.render_frame(cx);
			assert!(window.find(("nixos-option", 0_usize)).visible());
			assert!(window.try_find(("nixos-option", 1_usize)).is_none());
			app.update(cx, |app, cx| {
				let path = vec!["custom".into(), "welcome".into()];
				app.nixos.selected = Some(path.clone());
				app.nixos.group = vec![editable(
					&path.join("."),
					OptionKind::Bool,
					serde_json::Value::Bool(true),
				)];
				app.set_page(Page::Installed, window, cx);
				app.set_page(Page::Nixos, window, cx);
				app.focus_search(&crate::app::FocusSearch, window, cx);
				assert_eq!(app.page, Page::Nixos, "Ctrl-F must stay on the NixOS tab");
				assert_eq!(app.session.engine.config.target, Target::HomeManager);
				assert!(app.session.queue.is_empty());
				assert!(
					app.options.is_none(),
					"Package options have separate view state"
				);
				assert!(app.nixos.selected_editing().is_some());
				assert!(
					app.nixos.lock(&app.nixos.group[0]).is_some(),
					"Only the host nixbox rebuilds is editable"
				);
			});
			window.render_frame(cx);
			assert!(window.find("nixos-details").visible());
			assert!(window.try_find("nixos-apply").is_none());
			app.update(cx, |app, cx| {
				app.nixos_filter.update(cx, |input, cx| {
					input.set_value("does-not-exist", window, cx)
				});
				app.refresh_nixos_filter(cx);
			});
			window.render_frame(cx);
			assert!(window.try_find(("nixos-option", 0_usize)).is_none());
			app.update(cx, |app, cx| {
				app.nixos_filter
					.update(cx, |input, cx| input.set_value("", window, cx));
				app.refresh_nixos_filter(cx);
			});
			window.render_frame(cx);
			assert!(window.find(("nixos-option", 1_usize)).visible());
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
		})
		.expect("small window");
	}

	#[gpui_kit::test]
	async fn nixos_switches_and_dropdowns_stage_values_from_clicks(cx: &mut TestAppContext) {
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
		let handle = cx.open_window(size(px(1180.), px(900.)), |window, cx| {
			let view = cx
				.new(|cx| NixboxApp::new(session, build_rx, runtime.handle().clone(), window, cx));
			app = Some(view.clone());
			Root::new(view, window, cx)
		});
		let app = app.expect("view");
		cx.update_window(handle.into(), |_, window, cx| {
			app.update(cx, |app, cx| {
				app.nixos.initialized = true;
				app.nixos.hosts = vec![NIXOS_HOST.into()];
				app.nixos.configuration = Some(NixosConfiguration {
					root: app.session.config_dir(),
					host: NIXOS_HOST.into(),
				});
				app.nixos.options = vec![
					option("custom.enable", "Enable the service", false),
					option("custom.mode", "Service mode", false),
					option("custom.optional", "Optional flag", false),
				];
				app.nixos.filter("");
				app.nixos.group = vec![
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
				app.select_nixos_option(vec!["custom".into(), "enable".into()], window, cx);
				app.set_page(Page::Nixos, window, cx);
			});
			window.render_frame(cx);
			window.click(("nixos-bool", 0_usize), cx);
			app.read_with(cx, |app, _| {
				assert_eq!(
					app.nixos_current(&app.nixos.group[0]),
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
				window.within("popup-menu").find(0_usize).label(),
				Some("auto")
			);
			assert_eq!(
				window.within("popup-menu").find(1_usize).label(),
				Some("manual")
			);
			window.within("popup-menu").click(1_usize, cx);
			window.render_frame(cx);
			app.read_with(cx, |app, _| {
				assert_eq!(
					app.nixos_current(&app.nixos.group[1]),
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
		let mut unset = None;
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
			assert!(
				labels.contains(&Some("null".into())),
				"Nullable flags retain an explicit null choice"
			);
			let enabled = labels
				.iter()
				.position(|label| label.as_deref() == Some("true"))
				.expect("true choice");
			unset = Some(
				labels
					.iter()
					.position(|label| label.as_deref() == Some("null"))
					.expect("null choice"),
			);
			window.within("popup-menu").click(enabled, cx);
			window.render_frame(cx);
			app.read_with(cx, |app, _| {
				assert_eq!(
					app.nixos_current(&app.nixos.group[2]),
					Some(SettingValue::Bool(true))
				);
			});
		})
		.expect("nullable window");
		cx.wait_for(
			handle.into(),
			std::time::Duration::from_secs(1),
			|window, _| window.try_find("popup-menu").is_none(),
		)
		.await;
		cx.update_window(handle.into(), |_, window, cx| {
			window.click(("nixos-menu", 2_usize), cx);
			window
				.within("popup-menu")
				.click(unset.expect("null choice"), cx);
			app.read_with(cx, |app, _| {
				assert_eq!(
					app.nixos_current(&app.nixos.group[2]),
					Some(SettingValue::Null)
				);
				assert_eq!(app.nixos.staged.len(), 1, "Restoring null drops that edit");
			});
		})
		.expect("window");
	}

	#[gpui_kit::test]
	async fn nixos_edits_stage_and_queue_one_system_rebuild(cx: &mut TestAppContext) {
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
		let path = |raw: &str| -> OptionPath { raw.split('.').map(str::to_string).collect() };
		cx.update_window(handle.into(), |_, window, cx| {
			app.update(cx, |app, cx| {
				let _cancel = app.session.fake_build(Target::HomeManager, "busy");
				app.nixos.initialized = true;
				app.nixos.hosts = vec![NIXOS_HOST.into()];
				app.nixos.configuration = Some(NixosConfiguration {
					root: app.session.config_dir(),
					host: NIXOS_HOST.into(),
				});
				app.nixos.options = vec![
					option("custom.enable", "Turns it on", false),
					option("custom.greeting", "Greeting", false),
					option("custom.owned", "Set by the user", false),
				];
				app.nixos.filter("");
				let mut owned = editable(
					"custom.owned",
					OptionKind::Bool,
					serde_json::Value::Bool(true),
				);
				owned.entry.defined_in = vec!["configuration.nix".into()];
				app.nixos.group = vec![
					editable(
						"custom.enable",
						OptionKind::Bool,
						serde_json::Value::Bool(false),
					),
					editable(
						"custom.greeting",
						OptionKind::Str,
						serde_json::Value::String("hello".into()),
					),
					owned,
				];
				app.select_nixos_option(path("custom.greeting"), window, cx);
				assert_eq!(app.nixos_input.read(cx).value().as_ref(), "hello");
				app.set_page(Page::Nixos, window, cx);

				app.stage_nixos(&path("custom.owned"), Some(SettingValue::Bool(false)), cx);
				assert!(app.nixos.staged.is_empty(), "User definitions stay locked");
				app.stage_nixos(&path("custom.enable"), Some(SettingValue::Bool(true)), cx);
				app.nixos_input
					.update(cx, |input, cx| input.set_value("hi", window, cx));
				app.stage_nixos_text(cx);
				assert_eq!(app.nixos.staged.len(), 2);
				app.stage_nixos(&path("custom.enable"), Some(SettingValue::Bool(false)), cx);
				assert_eq!(
					app.nixos.staged.len(),
					1,
					"Restoring the evaluated value drops the edit"
				);
			});
			window.render_frame(cx);
			assert!(window.find(("nixos-setting", 1_usize)).visible());
			assert!(window.find("nixos-apply").visible());
			app.update(cx, |app, cx| app.apply_nixos(window, cx));
		})
		.expect("window");
		app.read_with(cx, |app, _| {
			let Some(Op::SetOptions { changes, scope }) = app.session.queue.front() else {
				panic!("expected one settings op");
			};
			assert_eq!(*scope, Target::NixosSystem);
			assert_eq!(
				changes,
				&[OptionChange {
					path: path("custom.greeting"),
					value: Some(SettingValue::Str("hi".into())),
				}]
			);
			assert!(app.nixos.staged.is_empty());
			let greeting = app
				.nixos
				.editing(&path("custom.greeting"))
				.expect("greeting");
			assert_eq!(
				app.nixos_current(greeting),
				Some(SettingValue::Str("hi".into())),
				"Queued values stay in the controls until the rebuild"
			);
		});
	}
}
