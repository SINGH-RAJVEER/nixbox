//! The NixOS page: a branded header with the host picker, one search row,
//! then categories, the virtualized option list, and the selected option's
//! details side by side. Narrow windows fold categories into a wrapped row
//! and open details as a full-width subpage.

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
	ActiveTheme as _, IconName, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::{
	AnyElement, App, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
	SharedString, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div,
	prelude::FluentBuilder as _, px, rems, uniform_list,
};
use nixbox_core::options::plain_markdown;
use nixbox_core::system_options::{Category, access_note, option_tags, partial_note, value_text};
use nixbox_nix::settings::option_path_display;

use super::logo::nixos_logo;
use super::{empty, muted};
use crate::app::NixboxApp;

/// Below this width categories wrap above the list and details replace it.
const NARROW: f32 = 900.;
const SIDEBAR_WIDTH: f32 = 240.;

pub fn render(app: &mut NixboxApp, window: &Window, cx: &mut Context<NixboxApp>) -> AnyElement {
	let narrow = window.viewport_size().width < px(NARROW);
	let ready =
		!app.nixos.loading && app.nixos.error.is_none() && app.nixos.configuration.is_some();
	let body = if app.nixos.loading {
		v_flex()
			.size_full()
			.items_center()
			.justify_center()
			.gap_3()
			.child(Spinner::new())
			.child(muted("Reading the selected host's option schema...", cx))
			.into_any_element()
	} else if let Some(error) = &app.nixos.error {
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
			.into_any_element()
	} else if app.nixos.hosts.is_empty() {
		empty(
			"This flake has no nixosConfigurations outputs. Select a NixOS configuration directory in Settings.",
			cx,
		)
		.into_any_element()
	} else if app.nixos.configuration.is_none() {
		empty(
			"Choose a host above. This does not change the package install target.",
			cx,
		)
		.into_any_element()
	} else if narrow {
		if app.nixos.selected.is_some() {
			v_flex()
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
				.into_any_element()
		} else {
			option_column(app, cx)
		}
	} else {
		h_flex()
			.size_full()
			.child(
				div()
					.w(px(SIDEBAR_WIDTH))
					.h_full()
					.flex_shrink_0()
					.border_r_1()
					.border_color(cx.theme().border)
					.child(category_list(app, cx)),
			)
			.child(
				div()
					.flex_1()
					.h_full()
					.min_w_0()
					.child(option_column(app, cx)),
			)
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
	};
	v_flex()
		.size_full()
		.child(header(app, cx))
		.child(toolbar(app, cx))
		.when(ready && narrow, |page| page.child(category_chips(app, cx)))
		.child(
			div()
				.flex_1()
				.min_h_0()
				.border_t_1()
				.border_color(cx.theme().border)
				.child(body),
		)
		.into_any_element()
}

/// Logo, title, what is being inspected, the host picker, and reload.
fn header(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> impl IntoElement {
	let host = app
		.nixos
		.configuration
		.as_ref()
		.map(|configuration| configuration.host.clone());
	let subtitle = match &host {
		Some(host) => format!(
			"Host {host} in {}. Read-only in this version.",
			app.session.config_dir().display()
		),
		None => format!(
			"System settings in {}. Read-only in this version.",
			app.session.config_dir().display()
		),
	};
	let hosts = (app.nixos.hosts.len() > 1 || host.is_none()) && !app.nixos.hosts.is_empty();
	h_flex()
		.px_6()
		.pt_5()
		.pb_4()
		.gap_4()
		.items_center()
		.child(nixos_logo(rems(2.75)))
		.child(
			v_flex()
				.flex_1()
				.min_w_0()
				.gap_0p5()
				.child(
					div()
						.text_xl()
						.font_weight(FontWeight::SEMIBOLD)
						.text_color(cx.theme().link)
						.child("NixOS"),
				)
				.child(muted(subtitle, cx).truncate()),
		)
		.when(hosts, |row| {
			row.child(
				ButtonGroup::new("nixos-hosts")
					.outline()
					.small()
					.children(app.nixos.hosts.iter().enumerate().map(|(index, name)| {
						Button::new(("nixos-host", index))
							.label(name.clone())
							.selected(host.as_ref() == Some(name))
					}))
					.on_click(cx.listener(|this, clicked: &Vec<usize>, _, cx| {
						let host = clicked
							.first()
							.and_then(|index| this.nixos.hosts.get(*index))
							.cloned();
						if let Some(host) = host {
							this.select_nixos_host(&host, cx);
						}
					})),
			)
		})
		.child(
			Button::new("nixos-reload")
				.ghost()
				.icon(IconName::RotateCw)
				.label("Reload")
				.on_click(cx.listener(|this, _, _, cx| this.reload_nixos(cx))),
		)
}

fn toolbar(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> impl IntoElement {
	h_flex()
		.px_6()
		.pb_3()
		.gap_4()
		.items_center()
		.child(
			div()
				.flex_1()
				.min_w_0()
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
		.child(
			Switch::new("nixos-diagnostic")
				.label("Diagnostic options")
				.checked(app.nixos.show_diagnostic)
				.on_click(cx.listener(|this, checked: &bool, _, cx| {
					this.nixos.show_diagnostic = *checked;
					this.refresh_nixos_filter(cx);
				})),
		)
}

fn choose_category(this: &mut NixboxApp, category: Category, cx: &mut Context<NixboxApp>) {
	this.nixos.category = category;
	this.refresh_nixos_filter(cx);
}

/// The wide layout's category column: one row per category with its count.
fn category_list(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> impl IntoElement {
	v_flex()
		.id("nixos-categories")
		.size_full()
		.overflow_y_scroll()
		.py_2()
		.children(Category::ALL.into_iter().map(|category| {
			let selected = app.nixos.category == category;
			let count = app.nixos.counts[category.index()];
			let hover = cx.theme().list_hover;
			h_flex()
				.id(("nixos-category", category.index()))
				.px_4()
				.py_1p5()
				.gap_2()
				.items_center()
				.text_sm()
				.cursor_pointer()
				.when(selected, |row| {
					row.bg(cx.theme().list_active)
						.font_weight(FontWeight::MEDIUM)
				})
				.when(!selected, |row| row.hover(move |row| row.bg(hover)))
				.when(count == 0 && !selected, |row| {
					row.text_color(cx.theme().muted_foreground)
				})
				.on_click(cx.listener(move |this, _, _, cx| choose_category(this, category, cx)))
				.child(div().flex_1().min_w_0().truncate().child(category.label()))
				.child(
					div()
						.text_xs()
						.text_color(cx.theme().muted_foreground)
						.child(count.to_string()),
				)
		}))
}

/// The narrow layout's categories, wrapped above the list.
fn category_chips(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> impl IntoElement {
	h_flex()
		.px_6()
		.pb_3()
		.gap_1()
		.flex_wrap()
		.children(Category::ALL.into_iter().map(|category| {
			Button::new(("nixos-category", category.index()))
				.ghost()
				.xsmall()
				.label(format!(
					"{} {}",
					category.label(),
					app.nixos.counts[category.index()]
				))
				.selected(app.nixos.category == category)
				.on_click(cx.listener(move |this, _, _, cx| choose_category(this, category, cx)))
		}))
}

/// The match count over the virtualized option rows.
fn option_column(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let summary = format!(
		"{} of {} options",
		app.nixos.visible.len(),
		app.nixos.options.len()
	);
	v_flex()
		.size_full()
		.child(
			h_flex()
				.px_6()
				.py_2()
				.gap_2()
				.border_b_1()
				.border_color(cx.theme().border)
				.text_xs()
				.text_color(cx.theme().muted_foreground)
				.child(
					div()
						.flex_1()
						.truncate()
						.font_weight(FontWeight::SEMIBOLD)
						.child(app.nixos.category.label().to_uppercase()),
				)
				.child(summary),
		)
		.child(div().flex_1().min_h_0().child(option_list(app, cx)))
		.into_any_element()
}

fn option_list(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	if app.nixos.visible.is_empty() {
		return empty("No options match this category and search.", cx).into_any_element();
	}
	uniform_list(
		"nixos-options",
		app.nixos.visible.len(),
		cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
			range
				.filter_map(|index| {
					let option = this.nixos.options.get(*this.nixos.visible.get(index)?)?;
					let path = option.path.clone();
					let selected = this.nixos.selected.as_ref() == Some(&path);
					let hover = cx.theme().list_hover;
					Some(
						h_flex()
							.id(("nixos-option", index))
							.px_6()
							.py_2()
							.gap_2()
							.items_center()
							.border_b_1()
							.border_color(cx.theme().border)
							.cursor_pointer()
							.when(selected, |row| row.bg(cx.theme().list_active))
							.hover(move |row| row.bg(hover))
							.on_click(cx.listener(move |this, _, _, cx| {
								this.select_nixos_option(path.clone(), cx)
							}))
							.child(
								v_flex()
									.flex_1()
									.min_w_0()
									.child(
										div()
											.font_weight(FontWeight::MEDIUM)
											.truncate()
											.child(option_path_display(&option.path)),
									)
									.child(muted(option.type_description.clone(), cx).truncate()),
							)
							.children(
								option_tags(option)
									.into_iter()
									.filter(|tag| *tag != "diagnostic")
									.map(|tag| Tag::secondary().xsmall().child(tag)),
							)
							.test_support(),
					)
				})
				.collect::<Vec<_>>()
		}),
	)
	.size_full()
	.into_any_element()
}

fn details(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let Some(option) = app.nixos.selected_option() else {
		return v_flex()
			.size_full()
			.items_center()
			.justify_center()
			.gap_3()
			.px_6()
			.child(div().opacity(0.35).child(nixos_logo(rems(3.5))))
			.child(muted(
				"Select an option to inspect its value and where it is defined.",
				cx,
			))
			.into_any_element();
	};
	let tags = option_tags(option);
	let mut body = v_flex()
		.id("nixos-details")
		.size_full()
		.overflow_y_scroll()
		.px_6()
		.py_4()
		.gap_4()
		.child(
			v_flex()
				.gap_1()
				.child(
					div()
						.text_lg()
						.font_weight(FontWeight::SEMIBOLD)
						.font_family(cx.theme().mono_font_family.clone())
						.child(option_path_display(&option.path)),
				)
				.child(
					h_flex()
						.gap_2()
						.flex_wrap()
						.items_center()
						.child(muted(option.type_description.clone(), cx))
						.children(
							tags.into_iter()
								.map(|tag| Tag::secondary().xsmall().child(tag)),
						),
				),
		);
	if let Some(description) = &option.description {
		body = body.child(div().text_sm().child(plain_markdown(description)));
	}
	body = body.child(muted(access_note(option), cx));
	if let Some(note) = partial_note(option) {
		body = body.child(muted(note, cx));
	}

	let mut value = v_flex().gap_3().child(heading("Value", cx));
	if app.nixos.inspecting {
		value = value.child(
			h_flex()
				.gap_2()
				.items_center()
				.child(Spinner::new().small())
				.child(muted("Reading value and definition locations...", cx)),
		);
	}
	if let Some(error) = &app.nixos.detail_error {
		value = value.child(
			div()
				.text_sm()
				.text_color(cx.theme().danger)
				.child(error.clone()),
		);
	}
	let mut source = v_flex().gap_3().child(heading("Source", cx));
	if let Some(details) = &app.nixos.details {
		value = value.child(field("Effective", value_text(&details.value), cx));
		if let Some(text) = &details.default {
			value = value.child(field("Default", text.clone(), cx));
		}
		if let Some(text) = &details.example {
			value = value.child(field("Example", text.clone(), cx));
		}
		if let Some(files) = &details.definitions {
			source = source.child(field(
				"Defined in",
				if files.is_empty() {
					"No definitions.".into()
				} else {
					files.join("\n")
				},
				cx,
			));
		}
	}
	source = source.child(field("Declared in", option.declarations.join("\n"), cx));
	body.child(value)
		.child(source)
		.test_support()
		.into_any_element()
}

/// A small uppercase label that opens a group of fields.
fn heading(title: &'static str, cx: &App) -> impl IntoElement {
	div()
		.pt_1()
		.pb_1()
		.border_b_1()
		.border_color(cx.theme().border)
		.text_xs()
		.font_weight(FontWeight::SEMIBOLD)
		.text_color(cx.theme().muted_foreground)
		.child(title.to_uppercase())
}

fn field(label: &'static str, value: impl Into<SharedString>, cx: &App) -> impl IntoElement {
	v_flex().gap_1().child(muted(label, cx)).child(
		div()
			.text_sm()
			.font_family(cx.theme().mono_font_family.clone())
			.child(value.into()),
	)
}
