//! Preferences saved immediately.

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::{
	Disableable as _, IconName, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::{
	AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
	SharedString, StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
};
use nixbox_config::{
	DEFAULT_CHANNEL, InputMode, THEMES, TabLabels, Target, config_dir_from_env, default_config_dir,
};
use nixbox_core::telemetry;

use super::{muted, page_header};
use crate::app::NixboxApp;

const CHANNELS: [&str; 2] = [DEFAULT_CHANNEL, "nixpkgs-unstable"];
const TARGETS: [Target; 2] = [Target::HomeManager, Target::NixosSystem];

pub fn render(app: &mut NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let config = &app.session.engine.config;
	let busy = app.repository.busy;

	let targets = ButtonGroup::new("target")
		.outline()
		.small()
		.children(TARGETS.iter().map(|target| {
			Button::new(target.label())
				.label(target.label())
				.selected(*target == config.target)
				.disabled(busy)
		}))
		.on_click(cx.listener(|this, clicked: &Vec<usize>, _, cx| {
			if let Some(target) = clicked.first().and_then(|index| TARGETS.get(*index)) {
				this.set_target(*target, cx);
			}
		}));

	let channels = ButtonGroup::new("channel")
		.outline()
		.small()
		.children(CHANNELS.iter().map(|channel| {
			Button::new(*channel)
				.label(*channel)
				.selected(*channel == config.channel)
				.disabled(busy)
		}))
		.on_click(cx.listener(|this, clicked: &Vec<usize>, _, cx| {
			if let Some(channel) = clicked.first().and_then(|index| CHANNELS.get(*index)) {
				this.set_channel(channel, cx);
			}
		}));

	let themes = ButtonGroup::new("theme")
		.outline()
		.small()
		.children(THEMES.iter().map(|theme| {
			Button::new(*theme)
				.label(*theme)
				.selected(*theme == config.theme)
				.disabled(busy)
		}))
		.on_click(cx.listener(|this, clicked: &Vec<usize>, window, cx| {
			if let Some(theme) = clicked.first().and_then(|index| THEMES.get(*index)) {
				this.set_theme(theme, window, cx);
			}
		}));

	let tab_labels = ButtonGroup::new("tab-labels")
		.outline()
		.small()
		.children(TabLabels::ALL.iter().map(|labels| {
			Button::new(labels.name())
				.label(labels.label())
				.selected(*labels == config.tab_labels)
		}))
		.on_click(cx.listener(|this, clicked: &Vec<usize>, _, cx| {
			if let Some(labels) = clicked.first().and_then(|index| TabLabels::ALL.get(*index)) {
				this.set_tab_labels(*labels, cx);
			}
		}));

	let location = configuration(app, cx);
	let input_modes = ButtonGroup::new("input-mode")
		.outline()
		.small()
		.children([InputMode::Normal, InputMode::Vim].map(|mode| {
			Button::new(mode.label())
				.label(mode.label())
				.selected(mode == app.session.engine.config.input_mode)
		}))
		.on_click(cx.listener(|this, clicked: &Vec<usize>, _, cx| {
			if let Some(mode) = clicked
				.first()
				.and_then(|index| [InputMode::Normal, InputMode::Vim].get(*index).copied())
			{
				this.set_input_mode(mode, cx);
			}
		}));
	let shortcuts = keyboard_settings(app, cx);
	let privacy = telemetry_settings(app, cx);
	let terminal_apps = super::terminal_apps::render(cx);
	v_flex()
		.size_full()
		.child(page_header(
			"Settings",
			"Shared with the terminal UI and the command line.",
			None,
			cx,
		))
		.child(
			v_flex()
				.id("settings-scroll")
				.track_scroll(&app.keyboard.settings_scroll)
				.flex_1()
				.min_h_0()
				.overflow_y_scroll()
				.child(terminal_apps)
				.child(setting(
					"Input mode",
					"Normal types directly. Vim adds Normal, Insert and Visual modes to search and filter fields. Shared with the TUI.",
					input_modes,
					cx,
				))
				.child(privacy)
				.child(setting(
					"Install target",
					"Where new packages and flakes are added.",
					targets,
					cx,
				))
				.child(setting(
					"Channel",
					"Used by live search when the package catalog is unavailable.",
					channels,
					cx,
				))
				.child(setting(
					"Theme",
					"Default follows your desktop; the others are dark.",
					themes,
					cx,
				))
				.child(setting(
					"Tab labels",
					"What the navigation tabs show. Icon-only tabs name themselves on hover.",
					tab_labels,
					cx,
				))
				.child(setting(
					"Configuration",
					"The flake nixbox edits and rebuilds. Packages, flakes, options and version control all use it.",
					location,
					cx,
				))
				.child(shortcuts),
		)
		.into_any_element()
}

fn telemetry_settings(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let saved = app.session.engine.config.telemetry;
	let buttons = ButtonGroup::new("telemetry-choice")
		.outline()
		.small()
		.children([false, true].map(|enabled| {
			Button::new(if enabled {
				"telemetry-on"
			} else {
				"telemetry-off"
			})
			.label(if enabled { "On" } else { "Off" })
			.selected(enabled == saved.enabled)
			.disabled(enabled && telemetry::RECIPIENT.is_none())
		}))
		.on_click(cx.listener(|this, clicked: &Vec<usize>, _, cx| {
			if let Some(index) = clicked.first() {
				this.set_telemetry(*index == 1, cx);
			}
		}));
	v_flex()
		.flex_shrink_0()
		.px_6()
		.py_4()
		.gap_2()
		.child(
			div()
				.font_weight(FontWeight::MEDIUM)
				.child("Share basic usage data"),
		)
		.child(muted(telemetry::DISCLOSURE, cx))
		.child(muted(
			format!(
				"{} · Recipient: {}",
				telemetry::status_label(saved),
				telemetry::RECIPIENT.unwrap_or("not configured; enabling is unavailable")
			),
			cx,
		))
		.child(
			h_flex().items_center().gap_3().child(buttons).child(
				Button::new("telemetry-preview")
					.small()
					.label("Copy example report")
					.on_click(cx.listener(|this, _, _, cx| {
						this.copy_telemetry_preview(cx);
					})),
			),
		)
		.into_any_element()
}

fn keyboard_settings(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let mut rows = v_flex().flex_shrink_0().px_6().py_4().gap_2()
		.child(div().font_weight(FontWeight::MEDIUM).child("Keyboard shortcuts"))
		.child(muted("Separate alternatives with commas, e.g. ctrl-f, f2. Spaces make sequences, e.g. d d. Clear a field to disable it. Enter or Apply saves immediately; Reset restores the default. Letter shortcuts stay inactive while typing.", cx));
	let mut group = "";
	for shortcut in crate::keyboard::SHORTCUTS {
		if group != shortcut.group {
			group = shortcut.group;
			rows = rows.child(
				div()
					.pt_4()
					.text_sm()
					.font_weight(FontWeight::MEDIUM)
					.child(group),
			);
		}
		let id = shortcut.id;
		let input = &app.keyboard.inputs[id];
		rows = rows.child(
			h_flex()
				.id(format!("shortcut-row-{id}"))
				.items_center()
				.gap_3()
				.child(div().w_1_3().text_sm().child(shortcut.label))
				.child(div().flex_1().min_w_0().child(Input::new(input).small()))
				.child(
					Button::new(format!("shortcut-apply-{id}"))
						.small()
						.outline()
						.label("Apply")
						.on_click(cx.listener(move |this, _, window, cx| {
							this.save_shortcut(id, false, window, cx)
						})),
				)
				.child(
					Button::new(format!("shortcut-reset-{id}"))
						.small()
						.ghost()
						.label("Reset")
						.on_click(cx.listener(move |this, _, window, cx| {
							this.save_shortcut(id, true, window, cx)
						})),
				),
		);
	}
	rows.into_any_element()
}

fn setting(
	title: impl Into<SharedString>,
	description: impl Into<SharedString>,
	control: impl IntoElement,
	cx: &Context<NixboxApp>,
) -> impl IntoElement {
	h_flex()
		.flex_shrink_0()
		.px_6()
		.py_4()
		.gap_6()
		.child(
			v_flex()
				.w_1_3()
				.gap_1()
				.child(div().font_weight(FontWeight::MEDIUM).child(title.into()))
				.child(muted(description, cx)),
		)
		.child(div().flex_1().child(control))
}

/// The configuration directory, editable unless the environment names it or
/// queued writes still target the current one.
fn configuration(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let config = &app.session.engine.config;
	let from_env = config_dir_from_env().is_some();
	let pending = app.repository_pending_writes();
	let locked = from_env || pending || app.repository.busy;
	let current = app.session.config_dir().display().to_string();
	let typed = app.config_dir_input.read(cx).value();
	let unchanged = typed.trim() == current;
	let default = default_config_dir().display().to_string();
	let note = if from_env {
		"Set by NIXBOX_CONFIG_DIR, which overrides this setting.".to_string()
	} else if pending {
		"Finish or drop queued changes before moving the configuration.".to_string()
	} else if config.config_dir.is_some() {
		format!("Custom location. The default is {default}.")
	} else {
		"The default location. Type a path or browse for another directory.".to_string()
	};
	v_flex()
		.gap_2()
		.child(
			h_flex()
				.gap_2()
				.child(
					div()
						.flex_1()
						.min_w_0()
						.child(Input::new(&app.config_dir_input).disabled(locked)),
				)
				.child(
					Button::new("config-dir-browse")
						.outline()
						.small()
						.icon(IconName::FolderOpen)
						.label("Browse")
						.disabled(locked)
						.on_click(
							cx.listener(|this, _, window, cx| this.browse_config_dir(window, cx)),
						),
				)
				.child(
					Button::new("config-dir-apply")
						.primary()
						.small()
						.label("Apply")
						.disabled(locked || unchanged)
						.on_click(cx.listener(|this, _, window, cx| {
							this.apply_config_dir_input(window, cx)
						})),
				),
		)
		.child(h_flex().gap_3().child(muted(note, cx)).when(
			config.config_dir.is_some() && !locked,
			|row| {
				row.child(
					Button::new("config-dir-default")
						.ghost()
						.xsmall()
						.label("Use default")
						.on_click(
							cx.listener(|this, _, window, cx| {
								this.set_config_dir(None, window, cx)
							}),
						),
				)
			},
		))
		.into_any_element()
}
