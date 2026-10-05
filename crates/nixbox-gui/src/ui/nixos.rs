//! Flat, virtualized schema browser with lazy option inspection.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
	ActiveTheme as _, IconName, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::{
	AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
	StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div,
	prelude::FluentBuilder as _, px, uniform_list,
};
use nixbox_core::options::plain_markdown;
use nixbox_nix::settings::option_path_display;
use nixbox_nix::system_options::{InspectedValue, format_preview};

use super::{empty, muted, page_header};
use crate::app::NixboxApp;
use crate::nixos::Category;

pub fn render(app: &mut NixboxApp, window: &Window, cx: &mut Context<NixboxApp>) -> AnyElement {
	let header = page_header(
		"NixOS",
		format!(
			"Inspect system settings in {}. System settings are read-only in this version.",
			app.session.config_dir().display()
		),
		Some(
			Button::new("nixos-reload")
				.ghost()
				.label("Reload")
				.on_click(cx.listener(|this, _, _, cx| this.reload_nixos(cx)))
				.into_any_element(),
		),
		cx,
	);
	let hosts = h_flex()
		.px_6()
		.pb_3()
		.gap_2()
		.flex_wrap()
		.items_center()
		.child(muted("Host", cx))
		.children(app.nixos.hosts.iter().enumerate().map(|(index, host)| {
			let selected = app
				.nixos
				.configuration
				.as_ref()
				.is_some_and(|configuration| &configuration.host == host);
			let host = host.clone();
			Button::new(("nixos-host", index))
				.outline()
				.small()
				.label(host.clone())
				.selected(selected)
				.on_click(cx.listener(move |this, _, _, cx| this.select_nixos_host(&host, cx)))
		}));
	let filters = h_flex()
		.px_6()
		.pb_3()
		.gap_3()
		.items_center()
		.child(
			div().flex_1().min_w_0().child(
				Input::new(&app.nixos_filter)
					.prefix(IconName::Search)
					.cleanable(true),
			),
		)
		.child(
			Switch::new("nixos-diagnostic")
				.label("Diagnostic options")
				.checked(app.nixos.show_diagnostic)
				.on_click(cx.listener(|this, checked: &bool, _, cx| {
					this.nixos.show_diagnostic = *checked;
					this.refresh_nixos_filter(cx);
				})),
		);
	let categories = h_flex().px_6().pb_3().gap_1().flex_wrap().children(
		Category::ALL
			.into_iter()
			.enumerate()
			.map(|(index, category)| {
				Button::new(("nixos-category", index))
					.ghost()
					.xsmall()
					.label(category.label())
					.selected(app.nixos.category == category)
					.on_click(cx.listener(move |this, _, _, cx| {
						this.nixos.category = category;
						this.refresh_nixos_filter(cx);
					}))
			}),
	);
	let count = format!(
		"{} matching / {} schema entries",
		app.nixos.visible.len(),
		app.nixos.options.len()
	);
	let body = if app.nixos.loading {
		empty("Reading the selected host's option schema...", cx).into_any_element()
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
		empty("This flake has no nixosConfigurations outputs. Select a NixOS configuration directory in Settings.", cx).into_any_element()
	} else if app.nixos.configuration.is_none() {
		empty(
			"Choose the host to inspect. This does not change the package install target.",
			cx,
		)
		.into_any_element()
	} else if window.viewport_size().width < px(900.) {
		if app.nixos.selected.is_some() {
			v_flex()
				.size_full()
				.child(
					Button::new("nixos-back")
						.ghost()
						.label("Back to options")
						.on_click(cx.listener(|this, _, _, cx| {
							this.nixos.selected = None;
							cx.notify();
						})),
				)
				.child(div().flex_1().min_h_0().child(details(app, cx)))
				.into_any_element()
		} else {
			option_list(app, cx)
		}
	} else {
		h_flex()
			.size_full()
			.child(div().w_1_2().h_full().min_w_0().child(option_list(app, cx)))
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
		.child(header)
		.child(hosts)
		.child(filters)
		.child(categories)
		.child(div().px_6().pb_3().child(muted(count, cx)))
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
							.when(option.nested, |row| {
								row.child(Tag::secondary().xsmall().child("schema"))
							})
							.when(option.read_only, |row| {
								row.child(Tag::secondary().xsmall().child("read-only"))
							})
							.when(
								option.children_truncated || option.children_unavailable,
								|row| row.child(Tag::secondary().xsmall().child("partial")),
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
		return empty("Select an option to inspect its value and declaration.", cx)
			.into_any_element();
	};
	let mut body = v_flex()
		.id("nixos-details")
		.size_full()
		.overflow_y_scroll()
		.px_6()
		.py_4()
		.gap_3()
		.child(
			div()
				.font_weight(FontWeight::SEMIBOLD)
				.child(option_path_display(&option.path)),
		)
		.child(muted(option.type_description.clone(), cx))
		.child(muted(
			if option.read_only {
				"Declared read-only. This setting cannot be changed here."
			} else {
				"Inspection only. Editing system settings is not available in this version."
			},
			cx,
		));
	if let Some(description) = &option.description {
		body = body.child(div().text_sm().child(plain_markdown(description)));
	}
	if option.children_unavailable || option.children_truncated {
		body = body.child(muted(if option.children_unavailable { "This submodule's child schema could not be evaluated. Its declaration files remain available below." }
			else { "This branch embeds a complete system or exceeds the four-submodule expansion limit. Its deeper fields are not indexed; consult the declaration files below." }, cx));
	}
	if app.nixos.inspecting {
		body = body.child(muted("Reading value and definition locations...", cx));
	}
	if let Some(error) = &app.nixos.detail_error {
		body = body.child(muted(error.clone(), cx));
	}
	if let Some(details) = &app.nixos.details {
		let value = match &details.value {
			InspectedValue::Available(value) => format_preview(value),
			InspectedValue::Unavailable => "Unavailable. This value could not be evaluated.".into(),
			InspectedValue::Sensitive => {
				"Withheld. Credential values and their defaults are not evaluated.".into()
			}
			InspectedValue::Template => {
				"Nested schema. Concrete instance inspection is not implemented yet.".into()
			}
		};
		body = body.child(field("Effective value", value, cx));
		if let Some(value) = &details.default {
			body = body.child(field("Default", value.clone(), cx));
		}
		if let Some(value) = &details.example {
			body = body.child(field("Example", value.clone(), cx));
		}
		if let Some(files) = &details.definitions {
			body = body.child(field(
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
	body.child(field("Declared in", option.declarations.join("\n"), cx))
		.test_support()
		.into_any_element()
}

fn field(label: &'static str, value: String, cx: &Context<NixboxApp>) -> impl IntoElement {
	v_flex()
		.gap_1()
		.child(muted(label, cx))
		.child(div().text_sm().child(value))
}
