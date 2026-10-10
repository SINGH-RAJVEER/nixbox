//! Maps the theme names stored in `settings.json` onto the GUI colors.
//!
//! `default` follows the desktop appearance. Named themes use the same
//! characteristic colors as the TUI, with surfaces for GUI controls.
//!
//! Zoom scales the theme's text sizes, which set the window's rem size, so
//! everything sized in rems grows and shrinks with the text.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Global, Hsla, Pixels, Window, rgb};

/// The zoom a fresh window starts at, as a percentage of the theme's text size.
pub const DEFAULT_ZOOM: u16 = 100;
/// How far one Ctrl-+ or Ctrl-- moves the zoom.
pub const ZOOM_STEP: u16 = 10;
const MIN_ZOOM: u16 = 50;
const MAX_ZOOM: u16 = 200;

/// The current zoom and the unzoomed text sizes it scales.
struct Zoom {
	percent: u16,
	font_size: Pixels,
	mono_font_size: Pixels,
}

impl Global for Zoom {}

struct Palette {
	name: &'static str,
	background: u32,
	surface: u32,
	foreground: u32,
	muted: u32,
	border: u32,
	accent: u32,
	accent_foreground: u32,
	title: u32,
}

const PALETTES: [Palette; 5] = [
	Palette {
		name: "dracula",
		background: 0x28_2a36,
		surface: 0x34_3646,
		foreground: 0xf8_f8f2,
		muted: 0xab_b2d0,
		border: 0x62_72a4,
		accent: 0xbd_93f9,
		accent_foreground: 0x28_2a36,
		title: 0xff_79c6,
	},
	Palette {
		name: "gruvbox",
		background: 0x28_2828,
		surface: 0x3c_3836,
		foreground: 0xeb_dbb2,
		muted: 0xa8_9984,
		border: 0x66_5c54,
		accent: 0xd7_9921,
		accent_foreground: 0x28_2828,
		title: 0xfa_bd2f,
	},
	Palette {
		name: "nord",
		background: 0x2e_3440,
		surface: 0x3b_4252,
		foreground: 0xec_eff4,
		muted: 0xb4_c0d0,
		border: 0x43_4c5e,
		accent: 0x81_a1c1,
		accent_foreground: 0x2e_3440,
		title: 0x88_c0d0,
	},
	Palette {
		name: "catppuccin",
		background: 0x1e_1e2e,
		surface: 0x31_3244,
		foreground: 0xcd_d6f4,
		muted: 0xa6_adc8,
		border: 0x58_5b70,
		accent: 0xcb_a6f7,
		accent_foreground: 0x1e_1e2e,
		title: 0x89_b4fa,
	},
	Palette {
		name: "monokai",
		background: 0x27_2822,
		surface: 0x3e_3d32,
		foreground: 0xf8_f8f2,
		muted: 0xb0_ad96,
		border: 0x75_715e,
		accent: 0xa6_e22e,
		accent_foreground: 0x27_2822,
		title: 0xf9_2672,
	},
];

/// Applies the theme called `name`, falling back to the desktop appearance.
pub fn apply(name: &str, mut window: Option<&mut Window>, cx: &mut App) {
	match PALETTES.iter().find(|palette| palette.name == name) {
		Some(palette) => paint(palette, cx),
		None => Theme::sync_system_appearance(window.as_deref_mut(), cx),
	}
	scale_text(cx);
	Theme::sync_base(cx);
	if let Some(window) = window {
		window.refresh();
	}
}

/// The current zoom, as a percentage of the theme's text size.
pub fn zoom(cx: &App) -> u16 {
	cx.try_global::<Zoom>()
		.map_or(DEFAULT_ZOOM, |zoom| zoom.percent)
}

/// Sets the zoom, clamped to 50-200%, and redraws every window.
pub fn set_zoom(percent: u16, cx: &mut App) {
	let percent = percent.clamp(MIN_ZOOM, MAX_ZOOM);
	if percent == zoom(cx) {
		return;
	}
	zoom_state(cx).percent = percent;
	scale_text(cx);
	Theme::sync_base(cx);
	cx.refresh_windows();
}

/// The zoom state, recording the theme's text sizes the first time.
fn zoom_state(cx: &mut App) -> &mut Zoom {
	if !cx.has_global::<Zoom>() {
		let theme = Theme::global(cx);
		let zoom = Zoom {
			percent: DEFAULT_ZOOM,
			font_size: theme.font_size,
			mono_font_size: theme.mono_font_size,
		};
		cx.set_global(zoom);
	}
	cx.global_mut::<Zoom>()
}

/// Sets the theme's text sizes from the unzoomed ones and the zoom.
fn scale_text(cx: &mut App) {
	let zoom = zoom_state(cx);
	let scale = f32::from(zoom.percent) / 100.;
	let font_size = zoom.font_size * scale;
	let mono_font_size = zoom.mono_font_size * scale;
	let theme = Theme::global_mut(cx);
	theme.font_size = font_size;
	theme.mono_font_size = mono_font_size;
}

fn paint(palette: &Palette, cx: &mut App) {
	Theme::change(ThemeMode::Dark, None, cx);
	let background = color(palette.background);
	let surface = color(palette.surface);
	let foreground = color(palette.foreground);
	let muted_foreground = color(palette.muted);
	let border = color(palette.border);
	let accent = color(palette.accent);
	let accent_foreground = color(palette.accent_foreground);
	let title = color(palette.title);
	let theme = Theme::global_mut(cx);

	theme.background = background;
	theme.foreground = foreground;
	theme.muted_foreground = muted_foreground;
	theme.border = border;
	theme.input = border;
	theme.caret = accent;
	theme.selection = accent.opacity(0.35);
	theme.ring = accent;
	theme.link = title;
	theme.link_hover = accent;
	theme.link_active = accent;

	theme.sidebar = background;
	theme.sidebar_foreground = foreground;
	theme.sidebar_border = border;
	theme.sidebar_accent = surface;
	theme.sidebar_accent_foreground = foreground;
	theme.sidebar_primary = accent;
	theme.sidebar_primary_foreground = accent_foreground;

	theme.colors.list = background;
	theme.list_even = background;
	theme.list_head = surface;
	theme.list_hover = surface;
	theme.list_active = accent.opacity(0.22);
	theme.list_active_border = accent;
	theme.popover = surface;
	theme.popover_foreground = foreground;
	theme.muted = surface;
	theme.secondary = surface;
	theme.secondary_hover = border;
	theme.secondary_active = border;
	theme.secondary_foreground = foreground;

	theme.primary = accent;
	theme.primary_foreground = accent_foreground;
	theme.primary_hover = accent.opacity(0.9);
	theme.primary_active = accent.opacity(0.8);
	theme.button_primary = accent;
	theme.button_primary_foreground = accent_foreground;
	theme.button_primary_hover = accent.opacity(0.9);
	theme.button_primary_active = accent.opacity(0.8);
	theme.button_secondary = surface;
	theme.button_secondary_foreground = foreground;
	theme.button_secondary_hover = border;
	theme.button_secondary_active = border;
	theme.accent = surface;
	theme.accent_foreground = foreground;
	theme.progress_bar = accent;
	theme.scrollbar = background;
	theme.scrollbar_thumb = border;
	theme.scrollbar_thumb_hover = accent;
	theme.tokens = (&theme.colors).into();
}

fn color(value: u32) -> Hsla {
	rgb(value).into()
}

#[cfg(test)]
mod tests {
	use super::{PALETTES, apply, color};
	use gpui_kit::TestAppContext;
	use gpui_kit::component::Theme;

	#[test]
	fn every_stored_theme_name_has_a_mapping() {
		let mapped: Vec<&str> = std::iter::once("default")
			.chain(PALETTES.iter().map(|palette| palette.name))
			.collect();

		assert_eq!(mapped, nixbox_config::THEMES);
	}

	#[gpui_kit::test]
	fn selected_theme_updates_surfaces_text_and_controls(cx: &mut TestAppContext) {
		cx.update(gpui_kit::init);
		cx.update(|cx| {
			apply("dracula", None, cx);
			let theme = Theme::global(cx);
			assert_eq!(theme.background, color(0x28_2a36));
			assert_eq!(theme.foreground, color(0xf8_f8f2));
			assert_eq!(theme.border, color(0x62_72a4));
			assert_eq!(theme.list_hover, color(0x34_3646));
			assert_eq!(theme.button_primary_foreground, color(0x28_2a36));
			assert_eq!(theme.tokens.scrollbar_thumb.color, color(0x62_72a4));

			apply("nord", None, cx);
			let theme = Theme::global(cx);
			assert_eq!(theme.background, color(0x2e_3440));
			assert_eq!(theme.sidebar_foreground, color(0xec_eff4));
			assert_eq!(theme.button_primary_foreground, color(0x2e_3440));

			apply("default", None, cx);
			assert_ne!(Theme::global(cx).background, color(0x2e_3440));
		});
	}
}
