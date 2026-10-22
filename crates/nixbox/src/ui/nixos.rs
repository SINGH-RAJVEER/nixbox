//! The NixOS page: a branded header with the host picker and the staged
//! edits' Apply, one search row, then categories, the virtualized option
//! list, and a settings editor for the selected option's group side by
//! side. Narrow windows fold categories into a wrapped row and open the
//! editor as a full-width subpage.

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
	ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::{
	AnyElement, App, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
	SharedString, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div,
	prelude::FluentBuilder as _, px, rems, uniform_list,
};
use nixbox_config::Target;
use nixbox_core::options::{NIXOS_HOST, choices, plain_markdown};
use nixbox_core::system_options::{Category, option_tags, partial_note, value_text};
use nixbox_nix::options::OptionKind;
use nixbox_nix::settings::{SettingValue, option_path_display};
use nixbox_nix::system_options::{EditableOption, SystemOption};

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
	let root = app.session.config_dir();
	let subtitle = match &host {
		Some(host) if app.nixos.editable_host() => format!("Host {host} in {}", root.display()),
		Some(host) => format!(
			"Host {host} in {}. Read-only: nixbox rebuilds the {NIXOS_HOST} host.",
			root.display()
		),
		None => format!("System settings in {}", root.display()),
	};
	let staged = app.nixos.staged.len();
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
		.when(app.nixos.editable_host(), |row| {
			row.child(
				Button::new("nixos-discard")
					.outline()
					.label("Discard")
					.disabled(staged == 0)
					.on_click(cx.listener(|this, _, window, cx| this.discard_nixos(window, cx))),
			)
			.child(
				Button::new("nixos-apply")
					.primary()
					.label(if staged == 0 {
						"Apply".to_string()
					} else {
						format!("Apply {staged}")
					})
					.disabled(staged == 0)
					.tooltip("Queue the staged changes as one NixOS rebuild")
					.on_click(cx.listener(|this, _, window, cx| this.apply_nixos(window, cx))),
			)
		})
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
					let staged = this.nixos.staged.contains_key(&path);
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
							.on_click(cx.listener(move |this, _, window, cx| {
								this.select_nixos_option(path.clone(), window, cx)
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
							.when(staged, |row| {
								row.child(Tag::primary().xsmall().child("staged"))
							})
							.test_support(),
					)
				})
				.collect::<Vec<_>>()
		}),
	)
	.size_full()
	.into_any_element()
}

/// The selected option's group, one block per option in the style of a
/// settings editor: a bar marking staged, queued, or nixbox-set values,
/// the name, a clamped description, and the control for its type. The
/// selected block opens up with its default, example, and sources.
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
	if let Some(note) = partial_note(option) {
		body = body.child(div().px_6().pb_2().child(muted(note, cx)));
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
				.child(name_line(option, None, cx))
				.children(
					option
						.description
						.as_ref()
						.map(|text| div().text_sm().child(plain_markdown(text))),
				)
				.child(meta("Declared in", option.declarations.join("\n"), cx)),
		);
	} else {
		for (index, editing) in app.nixos.group.iter().enumerate() {
			body = body.child(setting(app, index, editing, cx));
		}
	}
	body.test_support().into_any_element()
}

/// The option's last path segment with its type and schema tags.
fn name_line(
	option: &SystemOption,
	state: Option<&'static str>,
	cx: &Context<NixboxApp>,
) -> impl IntoElement {
	let name = option.path.last().cloned().unwrap_or_default();
	h_flex()
		.gap_2()
		.items_center()
		.flex_wrap()
		.child(div().font_weight(FontWeight::SEMIBOLD).child(name))
		.child(
			div()
				.text_xs()
				.text_color(cx.theme().muted_foreground)
				.child(option.type_description.clone()),
		)
		.children(
			option_tags(option)
				.into_iter()
				.filter(|tag| *tag != "diagnostic")
				.map(|tag| Tag::secondary().xsmall().child(tag)),
		)
		.children(state.map(|state| Tag::primary().xsmall().child(state)))
}

fn setting(
	app: &NixboxApp,
	index: usize,
	editing: &EditableOption,
	cx: &mut Context<NixboxApp>,
) -> AnyElement {
	let path = editing.entry.path.clone();
	let Some(option) = app.nixos.option(&path) else {
		return div().into_any_element();
	};
	let selected = app.nixos.selected.as_ref() == Some(&path);
	let staged = app.nixos.staged.contains_key(&path);
	let queued = app
		.session
		.queued_option(Target::NixosSystem, &path)
		.is_some();
	let ours = app
		.session
		.engine
		.sets_option(Target::NixosSystem, &editing.entry);
	let (state, bar) = if staged {
		(Some("staged"), Some(cx.theme().primary))
	} else if queued {
		(Some("queued"), Some(cx.theme().warning))
	} else if ours {
		(Some("nixbox"), Some(cx.theme().muted_foreground))
	} else {
		(None, None)
	};
	let lock = app.nixos.lock(editing);
	let hover = cx.theme().list_hover;
	let mut content = v_flex()
		.flex_1()
		.min_w_0()
		.gap_1p5()
		.child(name_line(option, state, cx))
		.children(option.description.as_ref().map(|text| {
			div()
				.text_sm()
				.text_color(cx.theme().muted_foreground)
				.when(!selected, |text| text.line_clamp(2))
				.child(plain_markdown(text))
		}))
		.child(control(app, index, editing, lock.as_deref(), selected, cx));
	if lock.is_none() && (ours || staged) {
		let reset = path.clone();
		content = content.child(
			h_flex().child(
				Button::new(("nixos-reset", index))
					.ghost()
					.xsmall()
					.icon(IconName::Undo2)
					.label(if ours { "Unset" } else { "Reset" })
					.on_click(cx.listener(move |this, _, window, cx| {
						this.stage_nixos(&reset, None, cx);
						this.sync_nixos_input(window, cx);
					})),
			),
		);
	}
	if selected {
		content = content.child(
			v_flex()
				.pt_1()
				.gap_1()
				.when_some(editing.details.default.clone(), |meta_rows, text| {
					meta_rows.child(meta("Default", text, cx))
				})
				.when_some(editing.details.example.clone(), |meta_rows, text| {
					meta_rows.child(meta("Example", text, cx))
				})
				.when_some(editing.details.definitions.as_ref(), |meta_rows, files| {
					meta_rows.child(meta(
						"Defined in",
						if files.is_empty() {
							"No definitions".to_string()
						} else {
							files.join("\n")
						},
						cx,
					))
				})
				.child(meta("Declared in", option.declarations.join("\n"), cx)),
		);
	}
	h_flex()
		.id(("nixos-setting", index))
		.items_stretch()
		.when(selected, |block| block.bg(cx.theme().list_active))
		.when(!selected, |block| {
			block
				.cursor_pointer()
				.hover(move |block| block.bg(hover))
				.on_click(cx.listener(move |this, _, window, cx| {
					this.select_nixos_option(path.clone(), window, cx)
				}))
		})
		.child(
			div()
				.w(px(3.))
				.flex_shrink_0()
				.when_some(bar, |edge, color| edge.bg(color)),
		)
		.child(content.px_6().py_3())
		.test_support()
		.into_any_element()
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
	if let Some(reason) = lock {
		return v_flex()
			.gap_0p5()
			.child(code(value_text(&editing.details.value), cx))
			.when(app.nixos.editable_host(), |column| {
				column.child(
					h_flex()
						.gap_1()
						.items_center()
						.text_xs()
						.text_color(cx.theme().muted_foreground)
						.child(IconName::EyeOff)
						.child(format!("Read-only, {reason}")),
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
		OptionKind::Bool | OptionKind::Enum(_) if choices(kind).len() <= CHOICE_BUTTONS => {
			let values = choices(kind);
			h_flex()
				.child(
					ButtonGroup::new(("nixos-choice", index))
						.outline()
						.small()
						.children(values.iter().enumerate().map(|(choice, value)| {
							Button::new(("nixos-choice-value", choice))
								.label(choice_label(value))
								.selected(current.as_ref() == Some(value))
						}))
						.on_click(cx.listener(move |this, clicked: &Vec<usize>, _, cx| {
							if let Some(value) =
								clicked.first().and_then(|choice| values.get(*choice))
							{
								this.stage_nixos(&path, Some(value.clone()), cx);
							}
						})),
				)
				.into_any_element()
		}
		OptionKind::Bool | OptionKind::Enum(_) => {
			let values = choices(kind);
			let label = current
				.as_ref()
				.map_or_else(|| "Choose".to_string(), choice_label);
			let app_handle = cx.entity().downgrade();
			h_flex()
				.child(
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
												this.stage_nixos(
													&path,
													Some(value_for_click.clone()),
													cx,
												);
											});
										}),
								);
							}
							menu
						}),
				)
				.into_any_element()
		}
		inner if selected => {
			let hint = match inner {
				OptionKind::StrList => "Comma-separated; empty for [ ]. Press Enter to stage.",
				OptionKind::Int | OptionKind::Float if nullable => {
					"A number, or empty for null. Press Enter to stage."
				}
				OptionKind::Int | OptionKind::Float => "A number. Press Enter to stage.",
				_ if nullable => "Empty for null. Press Enter to stage.",
				_ => "Press Enter to stage.",
			};
			v_flex()
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
		_ => code(
			current
				.as_ref()
				.map_or_else(|| "unset".to_string(), SettingValue::to_nix),
			cx,
		)
		.into_any_element(),
	}
}

/// Choices up to this many show as buttons; more open a menu.
const CHOICE_BUTTONS: usize = 4;

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
		.font_family(cx.theme().mono_font_family.clone())
		.child(text)
}

/// A label beside its value, for the selected option's sources.
fn meta(label: &'static str, value: impl Into<SharedString>, cx: &App) -> impl IntoElement {
	h_flex()
		.gap_3()
		.items_start()
		.text_xs()
		.child(
			div()
				.w(px(80.))
				.flex_shrink_0()
				.text_color(cx.theme().muted_foreground)
				.child(label),
		)
		.child(
			div()
				.flex_1()
				.min_w_0()
				.font_family(cx.theme().mono_font_family.clone())
				.child(value.into()),
		)
}
