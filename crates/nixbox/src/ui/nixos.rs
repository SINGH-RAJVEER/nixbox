//! The NixOS page: a sidebar with the host, search, and the settings
//! pages; the open page's settings, the changed settings, or the whole
//! schema as a namespace tree; and a status bar holding the staged edits'
//! Review and Apply. Narrow windows fold the sidebar into a page menu and
//! open option details as a full-width subpage.

use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::description_list::{DescriptionItem, DescriptionList};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::sidebar::{Sidebar, SidebarGroup, SidebarMenu, SidebarMenuItem};
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
	ActiveTheme as _, Disableable as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
	AnyElement, App, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
	ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
	TestSupportExt as _, WeakEntity, Window, div, prelude::FluentBuilder as _, px, rems,
	uniform_list,
};
use nixbox_config::Target;
use nixbox_core::options::{NIXOS_HOST, choices};
use nixbox_core::settings_pages::{
	PageGroup, SettingsPage, TreeRow, View, curated, documentation, label_for, tree_label,
};
use nixbox_core::system_options::{Category, lock_note, option_tags, partial_note, value_text};
use nixbox_nix::options::OptionKind;
use nixbox_nix::settings::{OptionPath, SettingValue, option_path_display};
use nixbox_nix::system_options::{EditableOption, SystemOption};

use super::logo::nixos_logo;
use super::{empty, muted};
use crate::app::NixboxApp;

/// Below this width the sidebar folds into a menu and details replace the
/// tree.
const NARROW: f32 = 900.;
const SIDEBAR_WIDTH: f32 = 264.;
/// The widest a settings page grows.
const PAGE_WIDTH: f32 = 860.;

pub fn render(app: &mut NixboxApp, window: &Window, cx: &mut Context<NixboxApp>) -> AnyElement {
	if app.nixos_input_dirty {
		app.nixos_input_dirty = false;
		cx.defer_in(window, |this, window, cx| this.sync_nixos_input(window, cx));
	}
	let narrow = window.viewport_size().width < px(NARROW);
	let ready =
		!app.nixos.loading && app.nixos.error.is_none() && app.nixos.configuration.is_some();
	let content = if let Some(message) = placeholder(app, cx) {
		message
	} else if !app.nixos.query.is_empty() {
		search_results(app, cx)
	} else {
		match app.nixos.view {
			View::Overview => overview(app, cx),
			View::Page(page) => settings_page(app, page, cx),
			View::Changed => changed(app, cx),
			View::AllOptions => all_options(app, narrow, cx),
		}
	};
	let body = if narrow {
		v_flex()
			.size_full()
			.child(narrow_header(app, cx))
			.child(
				div()
					.flex_1()
					.min_h_0()
					.border_t_1()
					.border_color(cx.theme().border)
					.child(content),
			)
			.into_any_element()
	} else {
		h_flex()
			.size_full()
			.child(sidebar(app, cx))
			.child(div().flex_1().h_full().min_w_0().child(content))
			.into_any_element()
	};
	v_flex()
		.size_full()
		.child(div().flex_1().min_h_0().child(body))
		.when(ready, |page| page.child(status_bar(app, cx)))
		.into_any_element()
}

/// The message shown while there is no schema to browse.
fn placeholder(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> Option<AnyElement> {
	if app.nixos.loading {
		return Some(
			v_flex()
				.size_full()
				.items_center()
				.justify_center()
				.gap_3()
				.child(Spinner::new())
				.child(muted("Reading the selected host's option schema...", cx))
				.into_any_element(),
		);
	}
	if let Some(error) = &app.nixos.error {
		return Some(
			v_flex()
				.id("nixos-error")
				.size_full()
				.overflow_y_scroll()
				.px_6()
				.py_4()
				.gap_3()
				.child(
					div()
						.font_weight(FontWeight::MEDIUM)
						.child("Could not read the NixOS schema."),
				)
				.child(muted(error.clone(), cx))
				.into_any_element(),
		);
	}
	if app.nixos.hosts.is_empty() {
		return Some(empty(
			"This flake has no nixosConfigurations outputs. Select a NixOS configuration directory in Settings.",
			cx,
		)
		.into_any_element());
	}
	if app.nixos.configuration.is_none() {
		return Some(
			empty(
				"Choose a host from the menu. This does not change the package install target.",
				cx,
			)
			.into_any_element(),
		);
	}
	None
}

/// Logo and title, the host menu, filters, reload, and the search field.
fn sidebar_header(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> impl IntoElement {
	v_flex()
		.w_full()
		.gap_3()
		.pb_1()
		.child(
			h_flex()
				.gap_2()
				.items_center()
				.child(nixos_logo(rems(1.5)))
				.child(
					div()
						.flex_1()
						.font_weight(FontWeight::SEMIBOLD)
						.text_color(cx.theme().link)
						.child("NixOS"),
				)
				.child(filters_menu(app, cx))
				.child(
					Button::new("nixos-reload")
						.ghost()
						.small()
						.icon(IconName::RotateCw)
						.tooltip("Reload hosts and the option schema")
						.on_click(cx.listener(|this, _, _, cx| this.reload_nixos(cx))),
				),
		)
		.child(host_menu(app, cx))
		.child(
			div()
				.w_full()
				.on_mouse_down(
					gpui_kit::MouseButton::Left,
					cx.listener(|this, _, window, cx| this.enter_insert(window, cx)),
				)
				.child(super::cursor::filter_input(
					app,
					&app.nixos_filter,
					IconName::Search,
				)),
		)
}

/// The host this page configures, as a menu when there is a choice.
fn host_menu(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> impl IntoElement {
	let host = app
		.nixos
		.configuration
		.as_ref()
		.map(|configuration| configuration.host.clone());
	let label = match &host {
		Some(host) if app.nixos.editable_host() => format!("Host {host}"),
		Some(host) => format!("Host {host}, read-only"),
		None => "Choose a host".into(),
	};
	let hosts = app.nixos.hosts.clone();
	let handle = cx.entity().downgrade();
	Button::new("nixos-host")
		.outline()
		.small()
		.w_full()
		.label(label)
		.icon(IconName::ChevronDown)
		.disabled(hosts.len() < 2 && host.is_some())
		.tooltip("The NixOS configuration this page reads and edits")
		.dropdown_menu(move |mut menu, _, _| {
			for name in &hosts {
				let (handle, name_for_click) = (handle.clone(), name.clone());
				menu = menu.item(
					PopupMenuItem::new(name.clone())
						.checked(host.as_ref() == Some(name))
						.on_click(move |_, _, cx| {
							let _ = handle.update(cx, |this, cx| {
								this.select_nixos_host(&name_for_click, cx);
							});
						}),
				);
			}
			menu
		})
}

/// The tree and search filters, as checkable menu items.
fn filters_menu(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> impl IntoElement {
	let handle = cx.entity().downgrade();
	let linked = app.nixos.hardware.inventory.is_some();
	let show_all = app.nixos.hardware.show_all;
	let diagnostic = app.nixos.show_diagnostic;
	let configured = app.nixos.category == Category::Configured;
	let summary = app.nixos.local_hardware.summary();
	Button::new("nixos-filters")
		.ghost()
		.small()
		.icon(gpui_kit::assets::IconName::ListFilter)
		.tooltip("Filter the option tree and search")
		.dropdown_menu(move |menu, _, _| {
			let toggle =
				|menu: PopupMenu,
				 label: &'static str,
				 checked: bool,
				 apply: fn(&mut NixboxApp, bool, &mut Window, &mut Context<NixboxApp>)| {
					let handle: WeakEntity<NixboxApp> = handle.clone();
					menu.item(PopupMenuItem::new(label).checked(checked).on_click(
						move |_, window, cx| {
							let _ = handle.update(cx, |this, cx| apply(this, !checked, window, cx));
						},
					))
				};
			let menu = toggle(
				menu,
				"Configured options only",
				configured,
				|this, on, window, cx| {
					this.nixos.category = if on {
						Category::Configured
					} else {
						Category::All
					};
					this.refresh_nixos_hardware(window, cx);
				},
			);
			let menu = toggle(
				menu,
				"Diagnostic options",
				diagnostic,
				|this, on, window, cx| {
					this.nixos.show_diagnostic = on;
					this.refresh_nixos_hardware(window, cx);
				},
			);
			let menu = menu.separator().label(if linked {
				summary.clone()
			} else {
				"Hardware is not linked to this host.".into()
			});
			let menu = toggle(
				menu,
				"Use this machine's hardware",
				linked,
				|this, on, window, cx| {
					this.use_local_nixos_hardware(on, window, cx);
				},
			);
			toggle(
				menu,
				"Show all hardware options",
				show_all,
				|this, on, window, cx| {
					this.nixos.hardware.show_all = on;
					this.refresh_nixos_hardware(window, cx);
				},
			)
		})
}

/// A navigation entry with an optional count beside it.
fn nav_item(
	app: &NixboxApp,
	view: View,
	count: Option<usize>,
	cx: &mut Context<NixboxApp>,
) -> SidebarMenuItem {
	let item = SidebarMenuItem::new(view.label())
		.active(app.nixos.view == view && app.nixos.query.is_empty())
		.on_click(cx.listener(move |this, _, window, cx| this.open_nixos_view(view, window, cx)));
	match count {
		Some(count) if count > 0 => item.suffix(move |_, cx| {
			div()
				.text_xs()
				.text_color(cx.theme().muted_foreground)
				.child(count.to_string())
		}),
		_ => item,
	}
}

/// How many staged edits sit on `page`.
fn staged_on(app: &NixboxApp, page: SettingsPage) -> usize {
	app.nixos
		.staged
		.keys()
		.filter(|path| curated(path).is_some_and(|(found, _, _)| found == page))
		.count()
}

fn sidebar(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> impl IntoElement {
	let pages = |group: PageGroup, cx: &mut Context<NixboxApp>| {
		SidebarMenu::new().children(
			SettingsPage::ALL
				.into_iter()
				.filter(|page| page.group() == group)
				.map(|page| nav_item(app, View::Page(page), Some(staged_on(app, page)), cx))
				.collect::<Vec<_>>(),
		)
	};
	let changed = app.nixos.changed(&app.session).len();
	Sidebar::new("nixos-nav")
		.w(px(SIDEBAR_WIDTH))
		.collapsible(false)
		.header(sidebar_header(app, cx))
		.child(
			SidebarGroup::new("Settings").child(SidebarMenu::new().child(nav_item(
				app,
				View::Overview,
				None,
				cx,
			))),
		)
		.child(SidebarGroup::new(PageGroup::System.label()).child(pages(PageGroup::System, cx)))
		.child(SidebarGroup::new(PageGroup::Software.label()).child(pages(PageGroup::Software, cx)))
		.child(
			SidebarGroup::new("Advanced").child(
				SidebarMenu::new()
					.child(nav_item(app, View::Changed, Some(changed), cx))
					.child(nav_item(
						app,
						View::AllOptions,
						Some(app.nixos.options.len()),
						cx,
					)),
			),
		)
}

/// The narrow layout's page menu and search, above the content.
fn narrow_header(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> impl IntoElement {
	let handle = cx.entity().downgrade();
	let current = app.nixos.view;
	v_flex()
		.px_4()
		.py_3()
		.gap_2()
		.child(
			h_flex()
				.gap_2()
				.items_center()
				.child(nixos_logo(rems(1.5)))
				.child(
					Button::new("nixos-views")
						.outline()
						.small()
						.label(current.label())
						.icon(IconName::ChevronDown)
						.dropdown_menu(move |mut menu, _, _| {
							for view in View::all() {
								let handle = handle.clone();
								menu = menu.item(
									PopupMenuItem::new(view.label())
										.checked(view == current)
										.on_click(move |_, window, cx| {
											let _ = handle.update(cx, |this, cx| {
												this.open_nixos_view(view, window, cx);
											});
										}),
								);
							}
							menu
						}),
				)
				.child(div().flex_1())
				.child(filters_menu(app, cx))
				.child(host_menu(app, cx)),
		)
		.child(
			div()
				.on_mouse_down(
					gpui_kit::MouseButton::Left,
					cx.listener(|this, _, window, cx| this.enter_insert(window, cx)),
				)
				.child(super::cursor::filter_input(
					app,
					&app.nixos_filter,
					IconName::Search,
				)),
		)
}

/// A page's breadcrumb, title, and description.
fn page_heading(
	crumb: &str,
	title: &str,
	description: impl Into<SharedString>,
	cx: &App,
) -> impl IntoElement {
	v_flex()
		.gap_1()
		.pb_5()
		.child(
			div()
				.text_xs()
				.text_color(cx.theme().muted_foreground)
				.child(format!("NixOS / {crumb}")),
		)
		.child(
			div()
				.text_xl()
				.font_weight(FontWeight::SEMIBOLD)
				.child(title.to_string()),
		)
		.child(muted(description, cx))
}

/// A scrolling column at the settings page width.
fn page_column(id: &'static str) -> gpui_kit::Stateful<gpui_kit::Div> {
	v_flex()
		.id(id)
		.size_full()
		.overflow_y_scroll()
		.px_8()
		.py_6()
}

fn overview(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let host = app
		.nixos
		.configuration
		.as_ref()
		.map(|configuration| configuration.host.clone())
		.unwrap_or_default();
	let description = if app.nixos.editable_host() {
		format!(
			"Settings for host {host}. Changes are staged here and applied together in one rebuild."
		)
	} else {
		let applied = app.nixos.applied_host.as_deref().unwrap_or(NIXOS_HOST);
		format!("Settings for host {host}, read-only: nixbox rebuilds the {applied} host.")
	};
	let mut column = page_column("nixos-overview").child(
		div()
			.max_w(px(PAGE_WIDTH))
			.child(page_heading("Overview", "Overview", description, cx)),
	);
	for group in [PageGroup::System, PageGroup::Software] {
		column = column.child(section_title(group.label(), cx));
		for page in SettingsPage::ALL
			.into_iter()
			.filter(|page| page.group() == group)
		{
			let count = app.nixos.page_size(page);
			let hover = cx.theme().list_hover;
			column = column.child(
				h_flex()
					.id(("nixos-page", page.index()))
					.max_w(px(PAGE_WIDTH))
					.px_3()
					.py_3()
					.gap_4()
					.items_center()
					.border_t_1()
					.border_color(cx.theme().border)
					.cursor_pointer()
					.hover(move |row| row.bg(hover))
					.on_click(cx.listener(move |this, _, window, cx| {
						this.open_nixos_view(View::Page(page), window, cx)
					}))
					.child(
						v_flex()
							.flex_1()
							.min_w_0()
							.child(div().font_weight(FontWeight::MEDIUM).child(page.label()))
							.child(muted(page.description(), cx)),
					)
					.child(muted(
						match count {
							1 => "1 setting".to_string(),
							count => format!("{count} settings"),
						},
						cx,
					))
					.child(IconName::ChevronRight)
					.test_support(),
			);
		}
	}
	column
		.child(div().pt_4().max_w(px(PAGE_WIDTH)).child(muted(
			format!(
				"Search finds any of this host's {} options. All options browses them by namespace.",
				app.nixos.options.len()
			),
			cx,
		)))
		.into_any_element()
}

fn section_title(title: &str, cx: &App) -> impl IntoElement {
	div()
		.pt_4()
		.pb_2()
		.max_w(px(PAGE_WIDTH))
		.text_sm()
		.font_weight(FontWeight::SEMIBOLD)
		.text_color(cx.theme().foreground)
		.child(title.to_string())
}

fn settings_page(app: &NixboxApp, page: SettingsPage, cx: &mut Context<NixboxApp>) -> AnyElement {
	let mut column = page_column("nixos-page-content").child(div().max_w(px(PAGE_WIDTH)).child(
		page_heading(page.group().label(), page.label(), page.description(), cx),
	));
	if let Some(error) = &app.nixos.detail_error {
		column = column.child(div().max_w(px(PAGE_WIDTH)).pb_3().child(Alert::error(
			"nixos-detail-error",
			format!("Could not read these settings. {error}"),
		)));
	}
	if app.nixos.sections.is_empty() {
		return column
			.child(muted(
				"This host does not declare any of this page's settings. Search or All options lists everything it does declare.",
				cx,
			))
			.into_any_element();
	}
	let mut index = 0;
	for section in &app.nixos.sections {
		column = column.child(section_title(section.title, cx));
		for entry in &section.entries {
			column = column.child(setting(
				app,
				index,
				&entry.path,
				Some((entry.label, entry.description)),
				cx,
			));
			index += 1;
		}
	}
	column.into_any_element()
}

fn changed(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let paths = app.nixos.changed(&app.session);
	let mut column =
		page_column("nixos-changed").child(div().max_w(px(PAGE_WIDTH)).child(page_heading(
			"Advanced",
			"Changed",
			"Settings with staged edits, and edits queued for the next NixOS rebuild.",
			cx,
		)));
	if paths.is_empty() {
		return column
			.child(muted("Nothing is staged or queued.", cx))
			.into_any_element();
	}
	for (index, path) in paths.iter().enumerate() {
		let entry = curated(path).map(|(_, _, entry)| (entry.label, entry.description));
		column = column.child(setting(app, index, path, entry, cx));
	}
	column.into_any_element()
}

/// One setting: a bar marking staged, queued, or nixbox-set values, its
/// name and description, and the control for its type. The selected one
/// opens up with its default, example, and sources.
fn setting(
	app: &NixboxApp,
	index: usize,
	path: &[String],
	label: Option<(&'static str, &'static str)>,
	cx: &mut Context<NixboxApp>,
) -> AnyElement {
	let Some(option) = app.nixos.option(path) else {
		return div().into_any_element();
	};
	let path: OptionPath = path.to_vec();
	let selected = app.nixos.selected.as_ref() == Some(&path);
	let editing = app.nixos.editing(&path);
	let staged = app.nixos.staged.contains_key(&path);
	let queued = app
		.session
		.queued_option(Target::NixosSystem, &path)
		.is_some();
	let ours = editing.is_some_and(|editing| {
		app.session
			.engine
			.sets_option(Target::NixosSystem, &editing.entry)
	});
	let (state, bar) = if staged {
		(Some("staged"), Some(cx.theme().primary))
	} else if queued {
		(Some("queued"), Some(cx.theme().warning))
	} else if ours {
		(Some("nixbox"), Some(cx.theme().muted_foreground))
	} else {
		(None, None)
	};
	let lock = editing.and_then(|editing| app.nixos.lock(editing));
	let description = documentation(option, label.map(|(_, description)| description));
	let mut text = v_flex()
		.flex_1()
		.min_w_0()
		.gap_0p5()
		.child(name_line(option, label.map(|(name, _)| name), state, cx))
		.children(description.map(|text| {
			div()
				.text_sm()
				.text_color(cx.theme().muted_foreground)
				.when(!selected, |text| text.line_clamp(2))
				.child(text)
		}))
		.when(label.is_some(), |text| {
			text.child(
				div()
					.text_xs()
					.font_family(cx.theme().mono_font_family.clone())
					.text_color(cx.theme().muted_foreground)
					.truncate()
					.child(option_path_display(&path)),
			)
		});
	if let (Some(editing), Some(reason)) = (editing, lock.as_deref())
		&& app.nixos.editable_host()
	{
		text = text.child(lock_alert(app, index, editing, reason, cx));
	}
	if selected && let Some(editing) = editing {
		text = text.child(sources(option, editing, cx));
	}
	let control = match editing {
		Some(editing) => control(app, index, editing, lock.as_deref(), selected, cx),
		None if app.nixos.inspecting => Skeleton::new().w(px(160.)).h(px(28.)).into_any_element(),
		None => muted("Not read", cx).into_any_element(),
	};
	let reset = (lock.is_none() && (ours || staged)).then(|| {
		let reset = path.clone();
		Button::new(("nixos-reset", index))
			.ghost()
			.xsmall()
			.icon(IconName::Undo2)
			.tooltip(if staged {
				"Drop the staged edit"
			} else {
				"Stop setting this value"
			})
			.on_click(cx.listener(move |this, _, window, cx| {
				this.stage_nixos(&reset, None, cx);
				this.sync_nixos_input(window, cx);
			}))
	});
	let hover = cx.theme().list_hover;
	let select = path.clone();
	h_flex()
		.id(("nixos-setting", index))
		.max_w(px(PAGE_WIDTH))
		.items_stretch()
		.border_t_1()
		.border_color(cx.theme().border)
		.when(selected, |block| block.bg(cx.theme().list_active))
		.when(!selected, |block| {
			block
				.cursor_pointer()
				.hover(move |block| block.bg(hover))
				.on_click(cx.listener(move |this, _, window, cx| {
					this.select_nixos_option(select.clone(), window, cx)
				}))
		})
		.child(
			div()
				.w(px(3.))
				.flex_shrink_0()
				.when_some(bar, |edge, color| edge.bg(color)),
		)
		.child(
			h_flex()
				.flex_1()
				.min_w_0()
				.px_4()
				.py_3()
				.gap_4()
				.items_start()
				.child(text)
				.child(
					h_flex()
						.flex_shrink_0()
						.gap_1()
						.items_center()
						.child(control)
						.children(reset),
				),
		)
		.test_support()
		.into_any_element()
}

/// Why a setting is locked, with a way to its definition when it lives in
/// a file.
fn lock_alert(
	app: &NixboxApp,
	index: usize,
	editing: &EditableOption,
	reason: &str,
	cx: &mut Context<NixboxApp>,
) -> impl IntoElement {
	let file = editing.entry.defined_in.first().cloned();
	let root = app.session.config_dir();
	h_flex()
		.pt_1()
		.gap_2()
		.items_center()
		.child(div().flex_1().min_w_0().child(Alert::warning(
			("nixos-lock", index),
			lock_note(editing, reason),
		)))
		.when_some(file, |row, file| {
			row.child(
				Button::new(("nixos-open-source", index))
					.outline()
					.small()
					.label("Open source")
					.on_click(cx.listener(move |_, _, _, cx| {
						let name = file.split(':').next().unwrap_or(&file);
						let path = std::path::Path::new(name);
						let path = if path.is_absolute() {
							path.to_path_buf()
						} else {
							root.join(path)
						};
						cx.open_with_system(&path);
					})),
			)
		})
}

/// The selected option's default, example, and where it is defined.
fn sources(option: &SystemOption, editing: &EditableOption, cx: &App) -> impl IntoElement {
	let code = |text: String| {
		div()
			.font_family(cx.theme().mono_font_family.clone())
			.child(text)
	};
	let mut list = DescriptionList::new().columns(1).bordered(false);
	if let Some(text) = &editing.details.default {
		list = list
			.child(DescriptionItem::new("Default").value(code(text.clone()).into_any_element()));
	}
	if let Some(text) = &editing.details.example {
		list = list
			.child(DescriptionItem::new("Example").value(code(text.clone()).into_any_element()));
	}
	if let Some(files) = &editing.details.definitions {
		let files = if files.is_empty() {
			"No definitions".to_string()
		} else {
			files.join("\n")
		};
		list = list.child(DescriptionItem::new("Defined in").value(code(files).into_any_element()));
	}
	list = list.child(
		DescriptionItem::new("Declared in")
			.value(code(option.declarations.join("\n")).into_any_element()),
	);
	div().pt_2().text_xs().child(list)
}

/// The option's name, its type and schema tags, and its edit state.
fn name_line(
	option: &SystemOption,
	label: Option<&str>,
	state: Option<&'static str>,
	cx: &App,
) -> impl IntoElement {
	let name = label.map_or_else(
		|| option.path.last().cloned().unwrap_or_default(),
		str::to_string,
	);
	h_flex()
		.gap_2()
		.items_center()
		.flex_wrap()
		.child(div().font_weight(FontWeight::MEDIUM).child(name))
		.when(label.is_none(), |line| {
			line.child(
				div()
					.text_xs()
					.text_color(cx.theme().muted_foreground)
					.child(option.type_description.clone()),
			)
		})
		.children(
			option_tags(option)
				.into_iter()
				.filter(|tag| *tag != "diagnostic" && (label.is_none() || *tag != "configured"))
				.map(|tag| Tag::secondary().xsmall().child(tag)),
		)
		.children(state.map(|state| Tag::primary().xsmall().child(state)))
}

/// Curated settings, then matching options, for the sidebar search.
fn search_results(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let results = &app.nixos.search;
	if results.is_empty() {
		return empty("Nothing matches this search.", cx).into_any_element();
	}
	let hover = cx.theme().list_hover;
	let mut column =
		page_column("nixos-search").child(div().max_w(px(PAGE_WIDTH)).child(page_heading(
			"Search",
			&format!("Results for \u{201c}{}\u{201d}", app.nixos.query),
			format!(
				"{} settings and {} options match.",
				results.settings.len(),
				results.total
			),
			cx,
		)));
	if !results.settings.is_empty() {
		column = column.child(section_title("Settings", cx));
		for (position, (page, shown)) in results.settings.iter().enumerate() {
			let (page, path) = (*page, shown.path.clone());
			column = column.child(
				h_flex()
					.id(("nixos-result", position))
					.max_w(px(PAGE_WIDTH))
					.px_3()
					.py_2()
					.gap_3()
					.items_center()
					.border_t_1()
					.border_color(cx.theme().border)
					.cursor_pointer()
					.hover(move |row| row.bg(hover))
					.on_click(cx.listener(move |this, _, window, cx| {
						this.open_nixos_result(path.clone(), Some(page), window, cx)
					}))
					.child(
						v_flex()
							.flex_1()
							.min_w_0()
							.child(div().font_weight(FontWeight::MEDIUM).child(shown.label))
							.children(
								app.nixos
									.option(&shown.path)
									.and_then(|option| {
										documentation(option, Some(shown.description))
									})
									.map(|text| muted(text, cx).line_clamp(2)),
							),
					)
					.child(Tag::secondary().xsmall().child(page.label()))
					.test_support(),
			);
		}
	}
	if !results.options.is_empty() {
		column = column.child(section_title("Options", cx));
		let offset = results.settings.len();
		for (position, index) in results.options.iter().enumerate() {
			let Some(option) = app.nixos.options.get(*index) else {
				continue;
			};
			let path = option.path.clone();
			column = column.child(
				h_flex()
					.id(("nixos-result", offset + position))
					.max_w(px(PAGE_WIDTH))
					.px_3()
					.py_2()
					.gap_3()
					.items_center()
					.border_t_1()
					.border_color(cx.theme().border)
					.cursor_pointer()
					.hover(move |row| row.bg(hover))
					.on_click(cx.listener(move |this, _, window, cx| {
						this.open_nixos_result(path.clone(), None, window, cx)
					}))
					.child(
						div()
							.flex_1()
							.min_w_0()
							.truncate()
							.font_family(cx.theme().mono_font_family.clone())
							.text_sm()
							.child(option_path_display(&option.path)),
					)
					.child(muted(option.type_description.clone(), cx).truncate())
					.test_support(),
			);
		}
		if results.total > results.options.len() {
			column = column.child(div().pt_3().child(muted(
				format!(
					"Showing the first {} of {} options. Refine the search to see the rest.",
					results.options.len(),
					results.total
				),
				cx,
			)));
		}
	}
	column.into_any_element()
}

/// The namespace tree beside the selected option's details. Narrow windows
/// show one or the other.
fn all_options(app: &NixboxApp, narrow: bool, cx: &mut Context<NixboxApp>) -> AnyElement {
	if narrow && app.nixos.selected.is_some() {
		return v_flex()
			.size_full()
			.child(
				div().px_4().py_2().child(
					Button::new("nixos-back")
						.ghost()
						.icon(IconName::ArrowLeft)
						.label("Back to options")
						.on_click(cx.listener(|this, _, _, cx| {
							this.nixos.selected = None;
							cx.notify();
						})),
				),
			)
			.child(div().flex_1().min_h_0().child(details(app, cx)))
			.into_any_element();
	}
	let tree = v_flex()
		.size_full()
		.child(
			h_flex()
				.px_4()
				.py_2()
				.gap_2()
				.border_b_1()
				.border_color(cx.theme().border)
				.text_xs()
				.text_color(cx.theme().muted_foreground)
				.child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child(
					if app.nixos.category == Category::Configured {
						"CONFIGURED OPTIONS"
					} else {
						"ALL OPTIONS"
					},
				))
				.child(format!(
					"{} of {}",
					app.nixos.visible.len(),
					app.nixos.options.len()
				)),
		)
		.child(div().flex_1().min_h_0().child(tree_list(app, cx)));
	if narrow {
		return tree.into_any_element();
	}
	h_flex()
		.size_full()
		.child(div().w(px(420.)).h_full().flex_shrink_0().child(tree))
		.child(
			div()
				.flex_1()
				.h_full()
				.min_w_0()
				.border_l_1()
				.border_color(cx.theme().border)
				.child(details(app, cx)),
		)
		.into_any_element()
}

fn tree_list(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	if app.nixos.rows.is_empty() {
		let message = if app.nixos.category == Category::Configured {
			"No configured options. Reload after saving configuration changes."
		} else {
			"No options match the filters."
		};
		return empty(message, cx).into_any_element();
	}
	uniform_list(
		"nixos-tree",
		app.nixos.rows.len(),
		cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
			range
				.filter_map(|position| {
					let row = this.nixos.rows.get(position)?.clone();
					let indent = px(16. + 16. * row.depth() as f32);
					let label = tree_label(&this.nixos.options, &row);
					let hover = cx.theme().list_hover;
					let base = h_flex()
						.id(("nixos-option", position))
						.pl(indent)
						.pr_4()
						.py_1p5()
						.gap_2()
						.items_center()
						.text_sm()
						.cursor_pointer()
						.hover(move |item| item.bg(hover));
					Some(match row {
						TreeRow::Namespace {
							prefix,
							count,
							expanded,
						} => base
							.on_click(cx.listener(move |this, _, _, cx| {
								this.nixos.toggle_namespace(&prefix);
								cx.notify();
							}))
							.child(if expanded {
								IconName::ChevronDown
							} else {
								IconName::ChevronRight
							})
							.child(
								div()
									.flex_1()
									.min_w_0()
									.truncate()
									.font_family(cx.theme().mono_font_family.clone())
									.child(label),
							)
							.child(muted(count.to_string(), cx))
							.test_support(),
						TreeRow::Option { index, .. } => {
							let option = this.nixos.options.get(index)?;
							let path = option.path.clone();
							let selected = this.nixos.selected.as_ref() == Some(&path);
							let staged = this.nixos.staged.contains_key(&path);
							base.when(selected, |item| item.bg(cx.theme().list_active))
								.on_click(cx.listener(move |this, _, window, cx| {
									this.select_nixos_option(path.clone(), window, cx)
								}))
								.child(div().w(px(16.)))
								.child(
									div()
										.flex_1()
										.min_w_0()
										.truncate()
										.font_family(cx.theme().mono_font_family.clone())
										.child(label),
								)
								.children(
									option_tags(option)
										.into_iter()
										.filter(|tag| *tag != "diagnostic")
										.map(|tag| Tag::secondary().xsmall().child(tag)),
								)
								.when(staged, |item| {
									item.child(Tag::primary().xsmall().child("staged"))
								})
								.test_support()
						}
					})
				})
				.collect::<Vec<_>>()
		}),
	)
	.size_full()
	.into_any_element()
}

/// The selected option and its siblings, edited together.
fn details(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let Some(option) = app.nixos.selected_option() else {
		return v_flex()
			.size_full()
			.items_center()
			.justify_center()
			.gap_3()
			.px_6()
			.child(div().opacity(0.35).child(nixos_logo(rems(3.5))))
			.child(muted("Select an option to see and change it.", cx))
			.into_any_element();
	};
	let parent = option
		.path
		.split_last()
		.map(|(_, parent)| option_path_display(parent))
		.unwrap_or_default();
	let shown_on = curated(&option.path).map(|(page, _, _)| page);
	let mut body = v_flex()
		.id("nixos-details")
		.size_full()
		.overflow_y_scroll()
		.pb_6()
		.child(
			h_flex()
				.px_6()
				.pt_4()
				.pb_2()
				.gap_2()
				.items_center()
				.child(
					div()
						.flex_1()
						.min_w_0()
						.truncate()
						.font_weight(FontWeight::SEMIBOLD)
						.font_family(cx.theme().mono_font_family.clone())
						.child(if parent.is_empty() {
							option_path_display(&option.path)
						} else {
							parent
						}),
				)
				.when(app.nixos.inspecting, |row| {
					row.child(Spinner::new().small())
				})
				.when(!app.nixos.group.is_empty(), |row| {
					row.child(muted(
						match app.nixos.group.len() {
							1 => "1 setting".to_string(),
							count => format!("{count} settings"),
						},
						cx,
					))
				}),
		);
	if let Some(page) = shown_on {
		body = body.child(
			h_flex().px_6().pb_2().child(
				Button::new("nixos-shown-on")
					.ghost()
					.xsmall()
					.icon(IconName::ArrowRight)
					.label(format!("Shown on {}", page.label()))
					.on_click(cx.listener(move |this, _, window, cx| {
						this.open_nixos_view(View::Page(page), window, cx)
					})),
			),
		);
	}
	if let Some(note) = partial_note(option) {
		body = body.child(div().px_6().pb_2().child(muted(note, cx)));
	}
	if let Some(reason) = app.nixos.hardware.exclusion_reason(&option.path) {
		body = body.child(div().px_6().pb_2().child(muted(
			format!("{reason}. Visible because it is configured, staged, or Show all is enabled."),
			cx,
		)));
	}
	if let Some(error) = &app.nixos.detail_error {
		body = body.child(
			div()
				.px_6()
				.py_2()
				.text_sm()
				.text_color(cx.theme().danger)
				.child(error.clone()),
		);
	}
	if app.nixos.group.is_empty() {
		// Until the group is read, the schema alone describes the selection.
		body = body.child(
			v_flex()
				.px_6()
				.py_3()
				.gap_1()
				.child(name_line(option, None, None, cx))
				.children(documentation(option, None).map(|text| div().text_sm().child(text))),
		);
	} else {
		for (index, path) in app.nixos.group.iter().enumerate() {
			body = body.child(setting(app, index, path, None, cx));
		}
	}
	body.test_support().into_any_element()
}

/// The editor for one option's type, or its value read-only when it is
/// locked. Text, number, and list values are typed into the selected block.
fn control(
	app: &NixboxApp,
	index: usize,
	editing: &EditableOption,
	lock: Option<&str>,
	selected: bool,
	cx: &mut Context<NixboxApp>,
) -> AnyElement {
	let path = editing.entry.path.clone();
	if lock.is_some() {
		return v_flex()
			.max_w(px(260.))
			.child(code(value_text(&editing.details.value), cx))
			.when(!app.nixos.editable_host(), |column| {
				column.child(
					h_flex()
						.gap_1()
						.items_center()
						.text_xs()
						.text_color(cx.theme().muted_foreground)
						.child(IconName::EyeOff)
						.child("Read-only host"),
				)
			})
			.into_any_element();
	}
	let current = app.nixos_current(editing);
	let kind = &editing.entry.kind;
	let nullable = matches!(kind, OptionKind::Nullable(_));
	match kind.inner() {
		OptionKind::Bool if !nullable => Switch::new(("nixos-bool", index))
			.checked(matches!(current, Some(SettingValue::Bool(true))))
			.on_click(cx.listener(move |this, checked: &bool, _, cx| {
				this.stage_nixos(&path, Some(SettingValue::Bool(*checked)), cx);
			}))
			.into_any_element(),
		OptionKind::Bool | OptionKind::Enum(_) => {
			let values = choices(kind);
			let label = current
				.as_ref()
				.map_or_else(|| "Choose".to_string(), choice_label);
			let app_handle = cx.entity().downgrade();
			Button::new(("nixos-menu", index))
				.outline()
				.small()
				.label(label)
				.icon(IconName::ChevronDown)
				.dropdown_menu(move |mut menu, _, _| {
					for value in &values {
						let (app_handle, path, value_for_click) =
							(app_handle.clone(), path.clone(), value.clone());
						menu = menu.item(
							PopupMenuItem::new(choice_label(value))
								.checked(current.as_ref() == Some(value))
								.on_click(move |_, _, cx| {
									let _ = app_handle.update(cx, |this, cx| {
										this.stage_nixos(&path, Some(value_for_click.clone()), cx);
									});
								}),
						);
					}
					menu
				})
				.into_any_element()
		}
		inner if selected => {
			let hint = match inner {
				OptionKind::StrList => "Comma-separated; empty for [ ]. Enter stages.",
				OptionKind::Int | OptionKind::Float if nullable => {
					"A number, or empty for null. Enter stages."
				}
				OptionKind::Int | OptionKind::Float => "A number. Enter stages.",
				_ if nullable => "Empty for null. Enter stages.",
				_ => "Enter stages.",
			};
			v_flex()
				.w(px(240.))
				.gap_1()
				.child(Input::new(&app.nixos_input).small())
				.child(
					div()
						.text_xs()
						.text_color(cx.theme().muted_foreground)
						.child(hint),
				)
				.into_any_element()
		}
		_ => div()
			.max_w(px(260.))
			.px_2()
			.py_1()
			.border_1()
			.border_color(cx.theme().border)
			.rounded(cx.theme().radius)
			.child(code(
				current
					.as_ref()
					.map_or_else(|| "unset".to_string(), SettingValue::to_nix),
				cx,
			))
			.into_any_element(),
	}
}

fn choice_label(value: &SettingValue) -> String {
	match value {
		SettingValue::Str(text) => text.clone(),
		value => value.to_nix(),
	}
}

/// A value as Nix, in the monospace font.
fn code(text: String, cx: &App) -> impl IntoElement {
	div()
		.text_sm()
		.truncate()
		.font_family(cx.theme().mono_font_family.clone())
		.child(text)
}

/// The host, the staged count, and Discard, Review, and Apply.
fn status_bar(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> impl IntoElement {
	let staged = app.nixos.staged.len();
	let host = app
		.nixos
		.configuration
		.as_ref()
		.map(|configuration| configuration.host.clone())
		.unwrap_or_default();
	let mut right = h_flex().gap_2().items_center().child(muted(
		match staged {
			0 => "No staged changes".to_string(),
			1 => "1 staged change".to_string(),
			count => format!("{count} staged changes"),
		},
		cx,
	));
	if app.nixos.editable_host() {
		right = right
			.child(
				Button::new("nixos-discard")
					.ghost()
					.small()
					.label("Discard")
					.disabled(staged == 0)
					.on_click(cx.listener(|this, _, window, cx| this.discard_nixos(window, cx))),
			)
			.child(
				Button::new("nixos-review")
					.outline()
					.small()
					.label("Review")
					.disabled(staged == 0)
					.on_click(cx.listener(|this, _, window, cx| open_review(this, window, cx))),
			)
			.child(
				Button::new("nixos-apply")
					.primary()
					.small()
					.label(if staged == 0 {
						"Apply".to_string()
					} else {
						format!("Apply {staged}")
					})
					.disabled(staged == 0)
					.tooltip("Queue the staged changes as one NixOS rebuild")
					.on_click(cx.listener(|this, _, window, cx| this.apply_nixos(window, cx))),
			);
	}
	StatusBar::new()
		.left(
			h_flex()
				.gap_3()
				.items_center()
				.text_xs()
				.text_color(cx.theme().muted_foreground)
				.child(div().size(px(7.)).rounded_full().bg(cx.theme().success))
				.child(host)
				.child(
					div()
						.font_family(cx.theme().mono_font_family.clone())
						.child(app.session.config_dir().display().to_string()),
				)
				.child(format!("{} options", app.nixos.options.len())),
		)
		.right(right)
}

/// One staged edit as the review lists it.
struct Change {
	label: String,
	path: String,
	before: String,
	after: String,
	nix: String,
}

fn review_changes(app: &NixboxApp) -> Vec<Change> {
	app.nixos
		.staged
		.iter()
		.map(|(path, value)| {
			let before = app
				.nixos
				.editing(path)
				.and_then(|option| app.nixos_baseline(option))
				.map_or_else(|| "unset".to_string(), |value| value.to_nix());
			let display = option_path_display(path);
			let (after, nix) = match value {
				Some(value) => (value.to_nix(), format!("{display} = {};", value.to_nix())),
				None => ("unset".to_string(), format!("# {display} is no longer set")),
			};
			Change {
				label: label_for(path),
				path: display,
				before,
				after,
				nix,
			}
		})
		.collect()
}

/// Opens the staged edits in a sheet, with the module they produce, before
/// they are applied.
fn open_review(app: &mut NixboxApp, window: &mut Window, cx: &mut Context<NixboxApp>) {
	if app.nixos.staged.is_empty() {
		return;
	}
	let handle: Entity<NixboxApp> = cx.entity();
	// The sheet renders inside the app's own frame, so it reads a snapshot
	// taken now rather than the app.
	let changes = std::rc::Rc::new(review_changes(app));
	let host = app
		.nixos
		.configuration
		.as_ref()
		.map(|configuration| configuration.host.clone())
		.unwrap_or_default();
	let file = app
		.session
		.engine
		.config
		.settings_file_for(Target::NixosSystem)
		.display()
		.to_string();
	window.open_sheet(cx, move |sheet, _, cx| {
		let count = changes.len();
		let mono = cx.theme().mono_font_family.clone();
		let muted_color = cx.theme().muted_foreground;
		let mut body = v_flex().id("nixos-review-body").gap_4().child(
			div()
				.text_sm()
				.text_color(muted_color)
				.child(format!("Host {host}. One rebuild writes these to {file}.")),
		);
		let mut module = String::from("{\n");
		for change in changes.iter() {
			module.push_str(&format!("  {}\n", change.nix));
			body = body.child(
				v_flex()
					.gap_1()
					.pb_3()
					.border_b_1()
					.border_color(cx.theme().border)
					.child(
						div()
							.font_weight(FontWeight::MEDIUM)
							.child(change.label.clone()),
					)
					.child(
						DescriptionList::new()
							.columns(1)
							.bordered(false)
							.child(
								DescriptionItem::new("Option").value(
									div()
										.font_family(mono.clone())
										.child(change.path.clone())
										.into_any_element(),
								),
							)
							.child(
								DescriptionItem::new("Change").value(
									div()
										.font_family(mono.clone())
										.child(format!(
											"{}  \u{2192}  {}",
											change.before, change.after
										))
										.into_any_element(),
								),
							),
					),
			);
		}
		module.push('}');
		body = body
			.child(
				div()
					.text_sm()
					.text_color(muted_color)
					.child("Generated settings"),
			)
			.child(
				div()
					.p_3()
					.rounded(cx.theme().radius)
					.bg(cx.theme().secondary)
					.font_family(mono)
					.text_sm()
					.child(module),
			);
		let apply = handle.clone();
		sheet
			.title(format!(
				"Review {count} change{}",
				if count == 1 { "" } else { "s" }
			))
			.size(px(560.))
			.child(body)
			.footer(
				h_flex()
					.w_full()
					.gap_2()
					.child(
						Button::new("nixos-review-back")
							.ghost()
							.label("Back")
							.on_click(move |_, window, cx| {
								window.close_sheet(cx);
							}),
					)
					.child(div().flex_1())
					.child(
						Button::new("nixos-review-apply")
							.primary()
							.label(format!("Apply {count}"))
							.on_click(move |_, window, cx| {
								window.close_sheet(cx);
								apply.update(cx, |this, cx| this.apply_nixos(window, cx));
							}),
					),
			)
	});
}
