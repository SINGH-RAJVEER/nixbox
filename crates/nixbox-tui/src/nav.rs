use nixbox_config::InputMode;

use crate::app::{App, Mode, SettingsPage, Tab};

pub(crate) fn move_selection(app: &mut App, delta: i32) {
    if app.results.is_empty() {
        return;
    }
    let len = app.results.len() as i32;
    let next = (app.selected as i32 + delta).rem_euclid(len);
    app.selected = next as usize;
}

pub(crate) fn move_flake_selection(app: &mut App, delta: i32) {
    if app.flake_results.is_empty() {
        return;
    }
    let len = app.flake_results.len() as i32;
    let next = (app.flake_selected as i32 + delta).rem_euclid(len);
    app.flake_selected = next as usize;
}

pub(crate) fn move_installed_selection(app: &mut App, delta: i32) {
    let len = app.installed_total();
    if len == 0 {
        return;
    }
    let next = (app.installed_selected as i32 + delta).rem_euclid(len as i32);
    app.installed_selected = next as usize;
}

pub(crate) fn cycle_tab(app: &mut App) {
    let tabs = app.visible_tabs();
    let cur = tabs.iter().position(|t| *t == app.tab).unwrap_or(0);
    let next = tabs[(cur + 1) % tabs.len()];
    set_tab(app, next);
}

pub(crate) fn cycle_tab_back(app: &mut App) {
    let tabs = app.visible_tabs();
    let cur = tabs.iter().position(|t| *t == app.tab).unwrap_or(0);
    let prev = tabs[(cur + tabs.len() - 1) % tabs.len()];
    set_tab(app, prev);
}

pub(crate) fn set_tab(app: &mut App, tab: Tab) {
    app.tab = tab;
    if app.engine.config.input_mode == InputMode::Vim {
        match tab {
            Tab::Search => app.input.enter_normal(),
            Tab::Flakes => app.flake_input.enter_normal(),
            Tab::Installed => app.installed_input.enter_normal(),
            _ => {}
        }
    }
    app.status = format!("{} tab", tab.label());
}

pub(crate) fn open_settings(app: &mut App) {
    app.settings_page = SettingsPage::Main;
    app.settings_cursor = 0;
    app.mode = Mode::SettingsSelect;
    app.status = "↑/↓ select  ·  Enter open  ·  Esc close".into();
}
