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
	DEFAULT_CHANNEL, THEMES, TabLabels, Target, config_dir_from_env, default_config_dir,
};

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
				.flex_1()
				.min_h_0()
				.overflow_y_scroll()
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
				)),
		)
		.into_any_element()
}

fn setting(
	title: impl Into<SharedString>,
	description: impl Into<SharedString>,
	control: impl IntoElement,
	cx: &Context<NixboxApp>,
) -> impl IntoElement {
	h_flex()
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
