//! Maps the theme names stored in `settings.json` onto gpui-component.
//!
//! The TUI's palettes were chosen for a terminal, so they are not copied
//! whole. `default` follows the desktop's light or dark appearance; every
//! other name is dark, with the TUI palette's background and accent.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, Rgba, Window, rgb};

/// Background and accent for each named theme, in `THEMES` order after
/// `default`.
const PALETTES: [(&str, u32, u32); 5] = [
    ("dracula", 0x28_2a36, 0xbd_93f9),
    ("gruvbox", 0x28_2828, 0xd7_9921),
    ("nord", 0x2e_3440, 0x5e_81ac),
    ("catppuccin", 0x1e_1e2e, 0xcb_a6f7),
    ("monokai", 0x27_2822, 0xa6_e22e),
];

/// Applies the theme called `name`, falling back to `default`.
pub fn apply(name: &str, window: Option<&mut Window>, cx: &mut App) {
    let Some(&(_, background, accent)) = PALETTES.iter().find(|(n, _, _)| *n == name) else {
        Theme::sync_system_appearance(window, cx);
        return;
    };
    Theme::change(ThemeMode::Dark, None, cx);
    let background = hsla(rgb(background));
    let accent = hsla(rgb(accent));
    let theme = Theme::global_mut(cx);
    theme.background = background;
    theme.sidebar = background;
    theme.primary = accent;
    theme.primary_hover = accent.opacity(0.9);
    theme.primary_active = accent.opacity(0.8);
    theme.button_primary = accent;
    theme.button_primary_hover = accent.opacity(0.9);
    theme.button_primary_active = accent.opacity(0.8);
    theme.ring = accent;
    theme.sidebar_primary = accent;
    theme.list_active_border = accent;
    theme.progress_bar = accent;
    if let Some(window) = window {
        window.refresh();
    }
}

fn hsla(color: Rgba) -> Hsla {
    color.into()
}

#[cfg(test)]
mod tests {
    use super::PALETTES;

    #[test]
    fn every_stored_theme_name_has_a_mapping() {
        let mapped: Vec<&str> = std::iter::once("default")
            .chain(PALETTES.iter().map(|(name, _, _)| *name))
            .collect();

        assert_eq!(mapped, nixbox_config::THEMES);
    }
}
