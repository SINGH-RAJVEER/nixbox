//! Preferences saved immediately.

use gpui_kit::component::button::{Button, ButtonGroup};
use gpui_kit::component::{Disableable as _, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
	AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
	SharedString, StatefulInteractiveElement as _, Styled as _, div,
};
use nixbox_config::{DEFAULT_CHANNEL, THEMES, Target};

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

	let config_dir = app.session.config_dir().display().to_string();
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
					"Configuration",
					"The flake nixbox edits and rebuilds.",
					muted(config_dir, cx),
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
