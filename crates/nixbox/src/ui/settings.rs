//! Preferences saved immediately, grouped into sections separated by rules.

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
	ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::{
	AnyElement, App, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
	SharedString, StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
	rems,
};
use nixbox_config::{
	DEFAULT_CHANNEL, InputMode, THEMES, TabLabels, Target, config_dir_from_env, default_config_dir,
};
use nixbox_core::telemetry;

use super::{muted, page_header};
use crate::app::NixboxApp;

const CHANNELS: [&str; 2] = [DEFAULT_CHANNEL, "nixpkgs-unstable"];
const TARGETS: [Target; 2] = [Target::HomeManager, Target::NixosSystem];
const INPUT_MODES: [InputMode; 2] = [InputMode::Normal, InputMode::Vim];

/// Order of the sections, which are the direct children of the scroll view.
#[cfg(test)]
pub(crate) const INTERFACE_SECTION: usize = 1;
#[cfg(test)]
pub(crate) const PRIVACY_SECTION: usize = 3;
#[cfg(test)]
pub(crate) const TERMINAL_SECTION: usize = 4;

pub fn render(app: &mut NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let nix = nix_section(app, cx);
	let interface = interface_section(app, cx);
	let window = window_section(app, cx);
	let privacy = privacy_section(app, cx);
	let terminal_apps = super::terminal_apps::render(cx);
	let shortcuts = keyboard_section(app, cx);
	v_flex()
		.size_full()
		.child(page_header(
			"Settings",
			"Changes save immediately.",
			None,
			cx,
		))
		.child(
			v_flex()
				.id("settings-scroll")
				.track_scroll(&app.keyboard.settings_scroll)
				.flex_1()
				.min_h_0()
				.pb_6()
				.overflow_y_scroll()
				.child(nix)
				.child(interface)
				.child(window)
				.child(privacy)
				.child(terminal_apps)
				.child(shortcuts),
		)
		.into_any_element()
}

/// Which configuration nixbox edits, and where new work goes.
fn nix_section(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let config = &app.session.engine.config;
	let busy = app.repository.busy;
	let queued = busy || app.session.is_building() || !app.session.queue.is_empty();
	let configuration_choices = app.configuration_choices.clone();
	let outputs = ButtonGroup::new("configuration-selection")
		.outline()
		.small()
		.children(
			configuration_choices
				.iter()
				.enumerate()
				.map(|(index, choice)| {
					Button::new(SharedString::from(format!("configuration-{index}")))
						.label(choice.label.clone())
						.selected(choice.selected(config))
						.disabled(queued)
				}),
		)
		.on_click(cx.listener(move |this, clicked: &Vec<usize>, window, cx| {
			if let Some(choice) = clicked
				.first()
				.and_then(|index| configuration_choices.get(*index))
			{
				this.select_configuration(choice, window, cx);
			}
		}));
	let outputs_note = app
		.configuration_diagnostic
		.clone()
		.or(queued.then(|| "Finish queued work before switching.".to_string()))
		.unwrap_or_else(|| "The host, attached user, or Home Manager output to manage.".into());

	let home_only = config.home_configuration.is_some();
	let targets = ButtonGroup::new("target")
		.outline()
		.small()
		.children(TARGETS.iter().map(|target| {
			Button::new(target.label())
				.label(target.label())
				.selected(*target == config.target)
				.disabled(busy || (home_only && *target == Target::NixosSystem))
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

	let dendritic = Switch::new("dendritic-detection")
		.checked(config.dendritic_detection)
		.disabled(queued)
		.on_click(cx.listener(|this, checked: &bool, _, cx| {
			this.set_dendritic_detection(*checked, cx);
		}));
	let dendritic_note = if queued {
		"Finish queued work before switching."
	} else {
		"Discover flake-parts hosts and Home Manager outputs."
	};

	let directory = configuration_directory(app, cx);
	section(
		"Nix configuration",
		"Shared with the terminal UI and the command line.",
		true,
		cx,
	)
	.child(setting(
		"Configuration directory",
		"The flake nixbox edits and rebuilds.",
		directory,
		cx,
	))
	.child(setting("Host or output", outputs_note, outputs, cx))
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
		"Detect dendritic configurations",
		dendritic_note,
		dendritic,
		cx,
	))
	.into_any_element()
}

/// Preferences the GUI shares with the terminal UI.
fn interface_section(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let config = &app.session.engine.config;
	let input_modes = ButtonGroup::new("input-mode")
		.outline()
		.small()
		.children(INPUT_MODES.map(|mode| {
			Button::new(mode.label())
				.label(mode.label())
				.selected(mode == config.input_mode)
		}))
		.on_click(cx.listener(|this, clicked: &Vec<usize>, _, cx| {
			if let Some(mode) = clicked
				.first()
				.and_then(|index| INPUT_MODES.get(*index).copied())
			{
				this.set_input_mode(mode, cx);
			}
		}));

	let themes = ButtonGroup::new("theme")
		.outline()
		.small()
		.children(THEMES.iter().map(|theme| {
			Button::new(*theme)
				.label(*theme)
				.selected(*theme == config.theme)
				.disabled(app.repository.busy)
		}))
		.on_click(cx.listener(|this, clicked: &Vec<usize>, window, cx| {
			if let Some(theme) = clicked.first().and_then(|index| THEMES.get(*index)) {
				this.set_theme(theme, window, cx);
			}
		}));

	section("Interface", "Shared with the terminal UI.", false, cx)
		.child(setting(
			"Input mode",
			"Vim adds Normal, Insert and Visual modes to search and filter fields.",
			input_modes,
			cx,
		))
		.child(setting(
			"Theme",
			"Default follows your desktop; the others are dark.",
			themes,
			cx,
		))
		.into_any_element()
}

/// Window chrome, which only the desktop app has.
fn window_section(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let config = &app.session.engine.config;
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
	let close_button = Switch::new("close-button")
		.checked(config.close_button)
		.on_click(cx.listener(|this, checked: &bool, _, cx| {
			this.set_close_button(*checked, cx);
		}));

	section("Desktop window", "Desktop app only.", false, cx)
		.child(setting(
			"Tab labels",
			"Icon-only tabs name themselves on hover.",
			tab_labels,
			cx,
		))
		.child(setting(
			"Close button",
			"Turn off if your window manager draws its own.",
			close_button,
			cx,
		))
		.into_any_element()
}

fn privacy_section(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
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
	let control = v_flex()
		.gap_2()
		.child(
			h_flex().items_center().gap_3().child(buttons).child(
				Button::new("telemetry-preview")
					.small()
					.ghost()
					.label("Copy example report")
					.on_click(cx.listener(|this, _, _, cx| {
						this.copy_telemetry_preview(cx);
					})),
			),
		)
		.child(muted(
			format!(
				"{} · Recipient: {}",
				telemetry::status_label(saved),
				telemetry::RECIPIENT.unwrap_or("not configured; enabling is unavailable")
			),
			cx,
		));

	section("Privacy", "Shared with the terminal UI.", false, cx)
		.child(setting(
			"Share basic usage data",
			telemetry::DISCLOSURE,
			control,
			cx,
		))
		.into_any_element()
}

fn keyboard_section(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let mut rows = section(
		"Keyboard shortcuts",
		"Desktop app only. Separate alternatives with commas (ctrl-f, f2) and sequence keys with spaces (d d). Clear a field to disable it; Enter or Apply saves.",
		false,
		cx,
	);
	let mut group = "";
	for shortcut in crate::keyboard::SHORTCUTS {
		if group != shortcut.group {
			group = shortcut.group;
			rows = rows.child(
				div()
					.px_6()
					.pt_3()
					.pb_1()
					.text_xs()
					.font_weight(FontWeight::SEMIBOLD)
					.text_color(cx.theme().muted_foreground)
					.child(group.to_uppercase()),
			);
		}
		let id = shortcut.id;
		let input = &app.keyboard.inputs[id];
		rows = rows.child(
			h_flex()
				.id(format!("shortcut-row-{id}"))
				.px_6()
				.py_1()
				.items_center()
				.gap_6()
				.child(div().w_1_3().text_sm().child(shortcut.label))
				.child(
					h_flex()
						.flex_1()
						.min_w_0()
						.items_center()
						.gap_2()
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
				),
		);
	}
	rows.into_any_element()
}

/// A titled group of settings. Every group after the first starts with a
/// rule; groups are never boxed.
pub(super) fn section(
	title: impl Into<SharedString>,
	scope: impl Into<SharedString>,
	first: bool,
	cx: &App,
) -> gpui_kit::Div {
	v_flex()
		.flex_shrink_0()
		.max_w(rems(60.))
		.pb_2()
		.when(!first, |group| {
			group.mt_4().border_t_1().border_color(cx.theme().border)
		})
		.child(
			v_flex()
				.px_6()
				.pt_5()
				.pb_2()
				.gap_1()
				.child(
					div()
						.font_weight(FontWeight::SEMIBOLD)
						.text_color(cx.theme().link)
						.child(title.into()),
				)
				.child(muted(scope, cx)),
		)
}

/// One labelled control: name and a short note on the left, control on the right.
pub(super) fn setting(
	title: impl Into<SharedString>,
	description: impl Into<SharedString>,
	control: impl IntoElement,
	cx: &App,
) -> impl IntoElement {
	h_flex()
		.flex_shrink_0()
		.px_6()
		.py_3()
		.gap_6()
		.items_start()
		.child(
			v_flex()
				.w_1_3()
				.gap_1()
				.child(
					div()
						.text_sm()
						.font_weight(FontWeight::MEDIUM)
						.child(title.into()),
				)
				.child(muted(description, cx)),
		)
		.child(div().flex_1().min_w_0().child(control))
}

/// The configuration directory, editable unless the environment names it or
/// queued writes still target the current one.
fn configuration_directory(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
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
		"The default location.".to_string()
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
						.child(Input::new(&app.config_dir_input).small().disabled(locked)),
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
		.child(h_flex().items_center().gap_3().child(muted(note, cx)).when(
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
