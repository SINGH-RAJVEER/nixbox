//! The NixOS browser owns inspection state, independently of package options.

use std::time::Duration;

use gpui_kit::{Context, Task};
use nixbox_core::system_options::{Category, default_host, filter_options};
use nixbox_nix::settings::OptionPath;
use nixbox_nix::system_options::{
	NixosConfiguration, SystemOption, SystemOptionDetails, discover_hosts, discover_options,
	inspect_option,
};
use tokio::task::AbortHandle;

use crate::app::{NixboxApp, Page};

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
	pub selected: Option<OptionPath>,
	pub details: Option<SystemOptionDetails>,
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
		self.details = None;
		self.detail_error = None;
	}

	pub fn reset(&mut self) {
		self.invalidate();
		self.hosts.clear();
		self.configuration = None;
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
		let filtered = filter_options(&self.options, text, self.category, self.show_diagnostic);
		self.visible = filtered.visible;
		self.counts = filtered.counts;
	}

	pub fn selected_option(&self) -> Option<&SystemOption> {
		let path = self.selected.as_ref()?;
		self.options.iter().find(|option| &option.path == path)
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
		let configuration = NixosConfiguration {
			root: self.session.config_dir(),
			host: host.into(),
		};
		self.nixos.configuration = Some(configuration.clone());
		self.nixos.loading = true;
		self.nixos.error = None;
		self.nixos.options.clear();
		self.nixos.visible.clear();
		self.nixos.counts = Default::default();
		self.nixos.selected = None;
		let epoch = self.nixos.catalog_epoch;
		let job = self
			.runtime
			.spawn(async move { discover_options(&configuration, EVALUATION_LIMIT).await });
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
					Ok(catalog) => {
						this.nixos.options = catalog.options;
						this.refresh_nixos_filter(cx);
					}
					Err(error) => this.nixos.error = Some(error),
				}
				cx.notify();
			});
		}));
		cx.notify();
	}

	pub fn select_nixos_option(&mut self, path: OptionPath, cx: &mut Context<Self>) {
		let Some(configuration) = self.nixos.configuration.clone() else {
			return;
		};
		if !self.nixos.options.iter().any(|option| option.path == path) {
			return;
		}
		self.nixos.cancel_inspection();
		self.nixos.selected = Some(path.clone());
		self.nixos.inspecting = true;
		let epoch = self.nixos.detail_epoch;
		let job = self
			.runtime
			.spawn(async move { inspect_option(&configuration, &path, EVALUATION_LIMIT).await });
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
					Ok(details) => this.nixos.details = Some(details),
					Err(error) => this.nixos.detail_error = Some(error),
				}
				cx.notify();
			});
		}));
		cx.notify();
	}

	pub fn refresh_nixos_filter(&mut self, cx: &mut Context<Self>) {
		self.nixos
			.filter(self.nixos_filter.read(cx).value().as_ref());
		cx.notify();
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
	use gpui_kit::test::TestWindowExt as _;
	use gpui_kit::{AppContext as _, TestAppContext, px, size};
	use nixbox_config::{Config, Target};
	use nixbox_core::{Engine, Session};
	use nixbox_nix::manifest::Manifest;
	use nixbox_nix::system_options::InspectedValue;

	fn option(path: &str, description: &str, diagnostic: bool) -> SystemOption {
		SystemOption {
			path: path.split('.').map(str::to_string).collect(),
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
				app.nixos.details = Some(SystemOptionDetails {
					path,
					value: InspectedValue::Available(serde_json::Value::Bool(true)),
					default: None,
					example: None,
					definitions: Some(vec!["configuration.nix".into()]),
				});
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
				assert!(app.nixos.details.is_some());
			});
			window.render_frame(cx);
			assert!(window.find("nixos-details").visible());
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
}
