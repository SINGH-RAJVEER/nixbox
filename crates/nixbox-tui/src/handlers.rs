use anyhow::Result;
use crossterm::event::{Event as CtEvent, KeyCode, KeyEventKind, KeyModifiers};
use nixbox_config::InputMode;
use nixbox_nix::build::BuildEvent;
use nixbox_nix::search::MAX_SEARCH_RESULTS;
use tokio::sync::mpsc;

use crate::app::{App, AppEvent, CHANNELS, INPUT_MODES, Mode, SettingsPage, TARGETS, Tab};
use crate::nav::{
    cycle_tab, cycle_tab_back, move_flake_selection, move_installed_selection, move_selection,
    open_settings,
};
use crate::ops::{
    cancel_build, drain_queue, install_selected, install_selected_flake, migrate_all,
    migrate_selected, schedule_flake_details, schedule_flake_search, schedule_search,
    uninstall_selected,
};
use crate::theme;
use crate::vim::VimMode;

pub(crate) async fn handle_terminal_event(
    app: &mut App,
    tx: &mpsc::Sender<AppEvent>,
    ev: CtEvent,
) -> Result<()> {
    let CtEvent::Key(key) = ev else { return Ok(()) };
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        app.should_quit = true;
        return Ok(());
    }

    if let Mode::SettingsSelect = app.mode {
        handle_settings_select(app, key.code, key.modifiers);
        return Ok(());
    }

    if matches!(key.code, KeyCode::Esc)
        && app.config.input_mode == InputMode::Vim
        && matches!(app.tab, Tab::Search)
        && !matches!(app.input.mode(), VimMode::Normal)
    {
        app.input.enter_normal();
        return Ok(());
    }

    if matches!(key.code, KeyCode::Esc)
        && app.config.input_mode == InputMode::Vim
        && matches!(app.tab, Tab::Flakes)
        && !matches!(app.flake_input.mode(), VimMode::Normal)
    {
        app.flake_input.enter_normal();
        return Ok(());
    }
    if matches!(key.code, KeyCode::Esc)
        && app.config.input_mode == InputMode::Vim
        && matches!(app.tab, Tab::Installed)
        && !matches!(app.installed_input.mode(), VimMode::Normal)
    {
        app.installed_input.enter_normal();
        return Ok(());
    }

    match key.code {
        KeyCode::Tab => {
            cycle_tab(app);
            return Ok(());
        }
        KeyCode::BackTab => {
            cycle_tab_back(app);
            return Ok(());
        }
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            open_settings(app);
            return Ok(());
        }
        KeyCode::Esc => {
            app.should_quit = true;
            return Ok(());
        }
        _ => {}
    }

    if app.config.input_mode == InputMode::Normal {
        match app.tab {
            Tab::Search => match key.code {
                KeyCode::Down => move_selection(app, 1),
                KeyCode::Up => move_selection(app, -1),
                KeyCode::Enter => install_selected(app, tx).await?,
                _ => {
                    if app.input.handle_insert_event(&CtEvent::Key(key)) {
                        schedule_search(app, tx.clone());
                    }
                }
            },
            Tab::Flakes => match key.code {
                KeyCode::Down => {
                    move_flake_selection(app, 1);
                    schedule_flake_details(app, tx.clone());
                }
                KeyCode::Up => {
                    move_flake_selection(app, -1);
                    schedule_flake_details(app, tx.clone());
                }
                KeyCode::Enter => install_selected_flake(app, tx).await?,
                _ => {
                    if app.flake_input.handle_insert_event(&CtEvent::Key(key)) {
                        schedule_flake_search(app, tx.clone());
                    }
                }
            },
            Tab::Installed => match key.code {
                KeyCode::Down => move_installed_selection(app, 1),
                KeyCode::Up => move_installed_selection(app, -1),
                _ => {
                    if app.installed_input.handle_insert_event(&CtEvent::Key(key)) {
                        clamp_installed_selection(app);
                    }
                }
            },
            Tab::Building => {
                if matches!(key.code, KeyCode::Char('c')) {
                    cancel_build(app);
                }
            }
            Tab::Queue => {}
        }
        return Ok(());
    }

    match app.tab {
        Tab::Search => match app.input.mode() {
            VimMode::Insert => match key.code {
                KeyCode::Down => move_selection(app, 1),
                KeyCode::Up => move_selection(app, -1),
                KeyCode::Enter => install_selected(app, tx).await?,
                _ => {
                    if app.input.handle_insert_event(&CtEvent::Key(key)) {
                        schedule_search(app, tx.clone());
                    }
                }
            },
            VimMode::Normal => {
                if !matches!(key.code, KeyCode::Char('d')) {
                    app.input.clear_pending_d();
                }
                match key.code {
                    KeyCode::Down | KeyCode::Char('j') => move_selection(app, 1),
                    KeyCode::Up | KeyCode::Char('k') => move_selection(app, -1),
                    KeyCode::Left | KeyCode::Char('h') => app.input.move_left(),
                    KeyCode::Right | KeyCode::Char('l') => app.input.move_right(),
                    KeyCode::Char('b') => app.input.move_prev_word(),
                    KeyCode::Char('B') => app.input.move_prev_big_word(),
                    KeyCode::Char('w') => app.input.move_next_word(),
                    KeyCode::Char('W') => app.input.move_next_big_word(),
                    KeyCode::Char('e') => app.input.move_word_end(),
                    KeyCode::Char('E') => app.input.move_big_word_end(),
                    KeyCode::Char('0') => app.input.move_start(),
                    KeyCode::Char('$') => app.input.move_end(),
                    KeyCode::Char('v') => app.input.enter_visual(),
                    KeyCode::Char('x') => {
                        if app.input.delete_char() {
                            schedule_search(app, tx.clone());
                        }
                    }
                    KeyCode::Char('d') => {
                        if app.input.has_pending_d() {
                            app.input.clear_pending_d();
                            if app.input.delete_line() {
                                schedule_search(app, tx.clone());
                            }
                        } else {
                            app.input.set_pending_d();
                        }
                    }
                    KeyCode::Char('D') => {
                        if app.input.delete_to_end() {
                            schedule_search(app, tx.clone());
                        }
                    }
                    KeyCode::Enter => install_selected(app, tx).await?,
                    KeyCode::Char('i') | KeyCode::Char('/') => app.input.enter_insert_before(),
                    KeyCode::Char('a') => app.input.enter_insert_after(),
                    KeyCode::Char('I') => app.input.enter_insert_start(),
                    KeyCode::Char('A') => app.input.enter_insert_end(),
                    _ => {}
                }
            }
            VimMode::Visual => match key.code {
                KeyCode::Left | KeyCode::Char('h') => app.input.move_left(),
                KeyCode::Right | KeyCode::Char('l') => app.input.move_right(),
                KeyCode::Char('b') => app.input.move_prev_word(),
                KeyCode::Char('B') => app.input.move_prev_big_word(),
                KeyCode::Char('w') => app.input.move_next_word(),
                KeyCode::Char('W') => app.input.move_next_big_word(),
                KeyCode::Char('e') => app.input.move_word_end(),
                KeyCode::Char('E') => app.input.move_big_word_end(),
                KeyCode::Char('0') => app.input.move_start(),
                KeyCode::Char('$') => app.input.move_end(),
                KeyCode::Char('d') | KeyCode::Char('x') => {
                    if app.input.delete_selection(false) {
                        schedule_search(app, tx.clone());
                    }
                }
                KeyCode::Char('c') => {
                    app.input.delete_selection(true);
                    schedule_search(app, tx.clone());
                }
                _ => {}
            },
        },
        Tab::Flakes => match app.flake_input.mode() {
            VimMode::Insert => match key.code {
                KeyCode::Down => {
                    move_flake_selection(app, 1);
                    schedule_flake_details(app, tx.clone());
                }
                KeyCode::Up => {
                    move_flake_selection(app, -1);
                    schedule_flake_details(app, tx.clone());
                }
                KeyCode::Enter => install_selected_flake(app, tx).await?,
                _ => {
                    if app.flake_input.handle_insert_event(&CtEvent::Key(key)) {
                        schedule_flake_search(app, tx.clone());
                    }
                }
            },
            VimMode::Normal => {
                if !matches!(key.code, KeyCode::Char('d')) {
                    app.flake_input.clear_pending_d();
                }
                match key.code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        move_flake_selection(app, 1);
                        schedule_flake_details(app, tx.clone());
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        move_flake_selection(app, -1);
                        schedule_flake_details(app, tx.clone());
                    }
                    KeyCode::Left | KeyCode::Char('h') => app.flake_input.move_left(),
                    KeyCode::Right | KeyCode::Char('l') => app.flake_input.move_right(),
                    KeyCode::Char('b') => app.flake_input.move_prev_word(),
                    KeyCode::Char('B') => app.flake_input.move_prev_big_word(),
                    KeyCode::Char('w') => app.flake_input.move_next_word(),
                    KeyCode::Char('W') => app.flake_input.move_next_big_word(),
                    KeyCode::Char('e') => app.flake_input.move_word_end(),
                    KeyCode::Char('E') => app.flake_input.move_big_word_end(),
                    KeyCode::Char('0') => app.flake_input.move_start(),
                    KeyCode::Char('$') => app.flake_input.move_end(),
                    KeyCode::Char('v') => app.flake_input.enter_visual(),
                    KeyCode::Char('x') => {
                        if app.flake_input.delete_char() {
                            schedule_flake_search(app, tx.clone());
                        }
                    }
                    KeyCode::Char('d') => {
                        if app.flake_input.has_pending_d() {
                            app.flake_input.clear_pending_d();
                            if app.flake_input.delete_line() {
                                schedule_flake_search(app, tx.clone());
                            }
                        } else {
                            app.flake_input.set_pending_d();
                        }
                    }
                    KeyCode::Char('D') => {
                        if app.flake_input.delete_to_end() {
                            schedule_flake_search(app, tx.clone());
                        }
                    }
                    KeyCode::Enter => install_selected_flake(app, tx).await?,
                    KeyCode::Char('i') | KeyCode::Char('/') => {
                        app.flake_input.enter_insert_before();
                    }
                    KeyCode::Char('a') => app.flake_input.enter_insert_after(),
                    KeyCode::Char('I') => app.flake_input.enter_insert_start(),
                    KeyCode::Char('A') => app.flake_input.enter_insert_end(),
                    _ => {}
                }
            }
            VimMode::Visual => match key.code {
                KeyCode::Left | KeyCode::Char('h') => app.flake_input.move_left(),
                KeyCode::Right | KeyCode::Char('l') => app.flake_input.move_right(),
                KeyCode::Char('b') => app.flake_input.move_prev_word(),
                KeyCode::Char('B') => app.flake_input.move_prev_big_word(),
                KeyCode::Char('w') => app.flake_input.move_next_word(),
                KeyCode::Char('W') => app.flake_input.move_next_big_word(),
                KeyCode::Char('e') => app.flake_input.move_word_end(),
                KeyCode::Char('E') => app.flake_input.move_big_word_end(),
                KeyCode::Char('0') => app.flake_input.move_start(),
                KeyCode::Char('$') => app.flake_input.move_end(),
                KeyCode::Char('d') | KeyCode::Char('x') => {
                    if app.flake_input.delete_selection(false) {
                        schedule_flake_search(app, tx.clone());
                    }
                }
                KeyCode::Char('c') if app.flake_input.delete_selection(true) => {
                    schedule_flake_search(app, tx.clone());
                }
                _ => {}
            },
        },
        Tab::Installed => match app.installed_input.mode() {
            VimMode::Insert => match key.code {
                KeyCode::Down => move_installed_selection(app, 1),
                KeyCode::Up => move_installed_selection(app, -1),
                _ => {
                    if app.installed_input.handle_insert_event(&CtEvent::Key(key)) {
                        clamp_installed_selection(app);
                    }
                }
            },
            VimMode::Normal => match key.code {
                KeyCode::Down | KeyCode::Char('j') => move_installed_selection(app, 1),
                KeyCode::Up | KeyCode::Char('k') => move_installed_selection(app, -1),
                KeyCode::Char('h') => app.installed_input.move_left(),
                KeyCode::Char('l') => app.installed_input.move_right(),
                KeyCode::Char('b') => app.installed_input.move_prev_word(),
                KeyCode::Char('B') => app.installed_input.move_prev_big_word(),
                KeyCode::Char('w') => app.installed_input.move_next_word(),
                KeyCode::Char('W') => app.installed_input.move_next_big_word(),
                KeyCode::Char('e') => app.installed_input.move_word_end(),
                KeyCode::Char('E') => app.installed_input.move_big_word_end(),
                KeyCode::Char('0') => app.installed_input.move_start(),
                KeyCode::Char('$') => app.installed_input.move_end(),
                KeyCode::Char('v') => app.installed_input.enter_visual(),
                KeyCode::Char('x') => {
                    if app.installed_input.delete_char() {
                        clamp_installed_selection(app);
                    }
                }
                KeyCode::Char('D') => {
                    if app.installed_input.delete_to_end() {
                        clamp_installed_selection(app);
                    }
                }
                KeyCode::Delete | KeyCode::Char('d') => uninstall_selected(app, tx).await?,
                KeyCode::Char('m') => migrate_selected(app, tx).await?,
                KeyCode::Char('M') => migrate_all(app, tx).await?,
                KeyCode::Char('i') | KeyCode::Char('/') => {
                    app.installed_input.enter_insert_before();
                }
                KeyCode::Char('a') => app.installed_input.enter_insert_after(),
                KeyCode::Char('I') => app.installed_input.enter_insert_start(),
                KeyCode::Char('A') => app.installed_input.enter_insert_end(),
                _ => {}
            },
            VimMode::Visual => match key.code {
                KeyCode::Char('h') => app.installed_input.move_left(),
                KeyCode::Char('l') => app.installed_input.move_right(),
                KeyCode::Char('b') => app.installed_input.move_prev_word(),
                KeyCode::Char('B') => app.installed_input.move_prev_big_word(),
                KeyCode::Char('w') => app.installed_input.move_next_word(),
                KeyCode::Char('W') => app.installed_input.move_next_big_word(),
                KeyCode::Char('e') => app.installed_input.move_word_end(),
                KeyCode::Char('E') => app.installed_input.move_big_word_end(),
                KeyCode::Char('0') => app.installed_input.move_start(),
                KeyCode::Char('$') => app.installed_input.move_end(),
                KeyCode::Char('d') | KeyCode::Char('x') => {
                    if app.installed_input.delete_selection(false) {
                        clamp_installed_selection(app);
                    }
                }
                KeyCode::Char('c') => {
                    app.installed_input.delete_selection(true);
                    clamp_installed_selection(app);
                }
                _ => {}
            },
        },
        Tab::Building => {
            if let KeyCode::Char('c') = key.code {
                cancel_build(app);
            }
        }
        Tab::Queue => {}
    }
    Ok(())
}

fn clamp_installed_selection(app: &mut App) {
    let total = app.installed_total();
    if total == 0 {
        app.installed_selected = 0;
    } else if app.installed_selected >= total {
        app.installed_selected = total - 1;
    }
}

fn handle_settings_select(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    if matches!(code, KeyCode::Char('s')) && modifiers.contains(KeyModifiers::CONTROL) {
        app.mode = Mode::Browsing;
        app.status = "Settings closed.".into();
        return;
    }

    match code {
        KeyCode::Up | KeyCode::Char('k')
            if matches!(code, KeyCode::Up) || app.config.input_mode == InputMode::Vim =>
        {
            let n = settings_option_count(app.settings_page);
            app.settings_cursor = app.settings_cursor.checked_sub(1).unwrap_or(n - 1);
        }
        KeyCode::Down | KeyCode::Char('j')
            if matches!(code, KeyCode::Down) || app.config.input_mode == InputMode::Vim =>
        {
            app.settings_cursor =
                (app.settings_cursor + 1) % settings_option_count(app.settings_page);
        }
        KeyCode::Enter => select_setting(app),
        KeyCode::Esc => close_settings_page(app),
        _ => {}
    }
}

fn settings_option_count(page: SettingsPage) -> usize {
    match page {
        SettingsPage::Main => 4,
        SettingsPage::InputMode => INPUT_MODES.len(),
        SettingsPage::Theme => theme::ALL.len(),
        SettingsPage::Target => TARGETS.len(),
        SettingsPage::Channel => CHANNELS.len(),
    }
}

fn select_setting(app: &mut App) {
    if app.settings_page == SettingsPage::Main {
        app.settings_page = match app.settings_cursor {
            0 => SettingsPage::InputMode,
            1 => SettingsPage::Theme,
            2 => SettingsPage::Target,
            _ => SettingsPage::Channel,
        };
        app.settings_cursor = match app.settings_page {
            SettingsPage::InputMode => INPUT_MODES
                .iter()
                .position(|mode| *mode == app.config.input_mode)
                .unwrap_or(0),
            SettingsPage::Theme => app.theme_index,
            SettingsPage::Target => TARGETS
                .iter()
                .position(|target| *target == app.config.target)
                .unwrap_or(0),
            SettingsPage::Channel => CHANNELS
                .iter()
                .position(|channel| *channel == app.config.channel)
                .unwrap_or(0),
            SettingsPage::Main => 0,
        };
        app.status = "↑/↓ select  ·  Enter confirm  ·  Esc back".into();
        return;
    }

    let page = app.settings_page;
    app.status = match page {
        SettingsPage::InputMode => {
            let mode = INPUT_MODES[app.settings_cursor];
            app.apply_input_mode(mode);
            format!("Input set to {}.", mode.label())
        }
        SettingsPage::Theme => {
            app.theme_index = app.settings_cursor;
            app.config.theme = theme::ALL[app.theme_index].name.to_string();
            format!("Theme set to {}.", theme::ALL[app.theme_index].name)
        }
        SettingsPage::Target => {
            app.config.target = TARGETS[app.settings_cursor];
            format!("Target set to {}.", app.config.target.label())
        }
        SettingsPage::Channel => {
            app.config.channel = CHANNELS[app.settings_cursor].to_string();
            format!("Channel set to {}.", app.config.channel)
        }
        SettingsPage::Main => unreachable!(),
    };
    let _ = app.config.save();
    app.settings_page = SettingsPage::Main;
    app.settings_cursor = settings_main_index(page);
}

fn close_settings_page(app: &mut App) {
    if app.settings_page == SettingsPage::Main {
        app.mode = Mode::Browsing;
        app.status = "Settings closed.".into();
    } else {
        let page = app.settings_page;
        app.settings_page = SettingsPage::Main;
        app.settings_cursor = settings_main_index(page);
        app.status = "↑/↓ select  ·  Enter open  ·  Esc close".into();
    }
}

fn settings_main_index(page: SettingsPage) -> usize {
    match page {
        SettingsPage::Main | SettingsPage::InputMode => 0,
        SettingsPage::Theme => 1,
        SettingsPage::Target => 2,
        SettingsPage::Channel => 3,
    }
}

pub(crate) fn handle_app_event(app: &mut App, tx: &mpsc::Sender<AppEvent>, ev: AppEvent) {
    match ev {
        AppEvent::SearchDone { epoch, hits } => {
            if epoch == app.search_epoch {
                app.search_task = None;
                app.searching = false;
                app.search_task = None;
                let count = hits.len();
                app.results = hits;
                app.selected = 0;
                app.status = if count == MAX_SEARCH_RESULTS {
                    format!(
                        "Showing first {} matches for `{}`; refine search for more.",
                        count, app.latest_query
                    )
                } else {
                    format!("{} matches for `{}`", count, app.latest_query)
                };
            }
        }
        AppEvent::SearchFailed { epoch, error } => {
            if epoch == app.search_epoch {
                app.search_task = None;
                app.searching = false;
                app.search_task = None;
                app.results.clear();
                app.status = format!("search failed: {}", error);
            }
        }
        AppEvent::CatalogReady(catalog) => {
            app.catalog_task = None;
            app.catalog_loading = false;
            let revision: String = catalog.revision().chars().take(12).collect();
            app.package_catalog = Some(catalog);
            if app.input.value().is_empty() && !app.build_in_progress && app.last_error.is_none() {
                app.status = format!("Package catalog ready at {revision}.");
            } else if !app.input.value().is_empty() {
                schedule_search(app, tx.clone());
            }
        }
        AppEvent::CatalogFailed(error) => {
            app.catalog_task = None;
            app.catalog_loading = false;
            app.package_catalog = None;
            if app.input.value().is_empty() && !app.build_in_progress && app.last_error.is_none() {
                app.status =
                    format!("Package catalog unavailable; live search will be used: {error}");
            } else if !app.input.value().is_empty() {
                app.status = format!("Package catalog unavailable; using live search: {error}");
                schedule_search(app, tx.clone());
            }
        }
        AppEvent::FlakeSearchDone { epoch, hits } => {
            if epoch == app.flake_search_epoch {
                app.flake_search_task = None;
                app.flake_searching = false;
                app.flake_selected = 0;
                app.flake_results = hits;
                app.flake_details = None;
                let count = app.flake_results.len();
                app.status = format!("{} GitHub flake matches for `{}`", count, app.flake_query);
                schedule_flake_details(app, tx.clone());
            }
        }
        AppEvent::FlakeSearchFailed { epoch, error } => {
            if epoch == app.flake_search_epoch {
                app.flake_search_task = None;
                app.flake_searching = false;
                app.flake_results.clear();
                app.flake_details = None;
                app.status = format!("flake search failed: {}", error);
            }
        }
        AppEvent::FlakeDetailsDone { epoch, details } => {
            if epoch == app.flake_detail_epoch {
                app.flake_detail_task = None;
                app.flake_detail_loading = false;
                app.flake_details = Some(*details);
            }
        }
        AppEvent::FlakeDetailsFailed { epoch, error } => {
            if epoch == app.flake_detail_epoch {
                app.flake_detail_task = None;
                app.flake_detail_loading = false;
                app.status = format!("flake details failed: {}", error);
            }
        }
        AppEvent::Build(BuildEvent::Line(line)) => {
            app.log.push(line);
            if app.log.len() > 1000 {
                let drop_to = app.log.len() - 1000;
                app.log.drain(0..drop_to);
            }
        }
        AppEvent::Build(BuildEvent::Finished(result)) => {
            let label = app
                .current_op_label
                .take()
                .unwrap_or_else(|| "build".into());
            app.build_in_progress = false;
            app.build_cancel = None;
            app.in_progress_op = None;
            match &result {
                Ok(()) => {
                    app.status = format!("{} done.", label);
                    app.last_error = None;
                }
                Err(err) => {
                    app.status = format!("{} failed: {}.", label, err);
                    app.last_error = Some(format!("{}: {}", label, err));
                }
            }
            if result.is_ok() {
                app.refresh_external_packages();
            }
            app.persist();
            drain_queue(app, tx);
            if !app.visible_tabs().contains(&app.tab) {
                app.tab = Tab::Search;
            }
        }
        AppEvent::Build(BuildEvent::Cancelled) => {
            let label = app
                .current_op_label
                .take()
                .unwrap_or_else(|| "build".into());
            app.build_in_progress = false;
            app.build_cancel = None;
            app.in_progress_op = None;
            app.status = if app.queue.is_empty() {
                format!("{} cancelled.", label)
            } else {
                format!(
                    "{} cancelled; {} queued operation(s) paused.",
                    label,
                    app.queue.len()
                )
            };
            app.persist();
            if !app.visible_tabs().contains(&app.tab) {
                app.tab = Tab::Search;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;
    use nixbox_config::Config;
    use nixbox_nix::Manifest;

    use crate::vim::VimInput;

    fn test_app() -> App {
        App::new(
            Config::default(),
            Manifest::default(),
            Manifest::default(),
            Vec::new(),
        )
    }

    async fn press(app: &mut App, code: KeyCode) {
        press_with_modifiers(app, code, KeyModifiers::NONE).await;
    }

    async fn press_with_modifiers(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
        let (tx, _rx) = mpsc::channel(1);
        let key = KeyEvent::new(code, modifiers);
        handle_terminal_event(app, &tx, CtEvent::Key(key))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn h_and_l_move_the_search_cursor_without_switching_tabs() {
        let mut app = test_app();
        app.input = VimInput::new("abc".into());

        press(&mut app, KeyCode::Char('h')).await;
        assert_eq!(app.input.cursor(), 1);
        assert_eq!(app.tab, Tab::Search);

        press(&mut app, KeyCode::Char('l')).await;
        assert_eq!(app.input.cursor(), 2);
        assert_eq!(app.tab, Tab::Search);
    }

    #[tokio::test]
    async fn only_tab_and_backtab_switch_tabs() {
        let mut app = test_app();
        app.input = VimInput::new("abc".into());

        press(&mut app, KeyCode::Left).await;
        assert_eq!(app.tab, Tab::Search);
        assert_eq!(app.input.cursor(), 1);

        press(&mut app, KeyCode::Right).await;
        assert_eq!(app.tab, Tab::Search);
        assert_eq!(app.input.cursor(), 2);

        press(&mut app, KeyCode::Tab).await;
        assert_eq!(app.tab, Tab::Flakes);

        press(&mut app, KeyCode::BackTab).await;
        assert_eq!(app.tab, Tab::Search);
    }

    #[tokio::test]
    async fn double_d_deletes_the_search_line() {
        let mut app = test_app();
        app.input = VimInput::new("ripgrep".into());

        press(&mut app, KeyCode::Char('d')).await;
        assert!(app.input.has_pending_d());
        assert_eq!(app.input.value(), "ripgrep");

        press(&mut app, KeyCode::Char('d')).await;
        assert_eq!(app.input.value(), "");
        assert!(!app.input.has_pending_d());
    }

    #[tokio::test]
    async fn pending_delete_cancels_on_other_keys() {
        let mut app = test_app();
        app.input = VimInput::new("ab".into());

        press(&mut app, KeyCode::Char('d')).await;
        assert!(app.input.has_pending_d());

        press(&mut app, KeyCode::Char('e')).await;
        assert!(!app.input.has_pending_d());
        assert_eq!(app.input.value(), "ab");
    }

    #[tokio::test]
    async fn flakes_tab_uses_vim_cursor_navigation() {
        let mut app = test_app();
        app.tab = Tab::Flakes;
        app.flake_input = VimInput::new("niri".into());

        press(&mut app, KeyCode::Char('h')).await;
        assert_eq!(app.flake_input.cursor(), 2);

        press(&mut app, KeyCode::Char('v')).await;
        press(&mut app, KeyCode::Char('l')).await;
        assert_eq!(app.flake_input.selection_range(), Some((2, 3)));
    }

    #[tokio::test]
    async fn ctrl_s_opens_and_closes_settings() {
        let mut app = test_app();

        press_with_modifiers(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL).await;
        assert_eq!(app.mode, Mode::SettingsSelect);
        assert_eq!(app.settings_page, SettingsPage::Main);
        assert_eq!(app.settings_cursor, 0);

        press(&mut app, KeyCode::Enter).await;
        assert_eq!(app.settings_page, SettingsPage::InputMode);
        assert_eq!(app.settings_cursor, 0);

        press(&mut app, KeyCode::Esc).await;
        assert_eq!(app.settings_page, SettingsPage::Main);
        assert_eq!(app.settings_cursor, 0);

        press(&mut app, KeyCode::Down).await;
        assert_eq!(app.settings_cursor, 1);

        press(&mut app, KeyCode::Enter).await;
        assert_eq!(app.settings_page, SettingsPage::Theme);

        press_with_modifiers(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL).await;
        assert_eq!(app.mode, Mode::Browsing);
        assert_eq!(app.config.input_mode, InputMode::Vim);
    }

    #[tokio::test]
    async fn normal_mode_settings_navigation_uses_only_arrow_keys() {
        let mut app = test_app();
        app.apply_input_mode(InputMode::Normal);

        press_with_modifiers(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL).await;
        press(&mut app, KeyCode::Char('j')).await;
        assert_eq!(app.settings_cursor, 0);

        press(&mut app, KeyCode::Down).await;
        assert_eq!(app.settings_cursor, 1);

        press(&mut app, KeyCode::Char('k')).await;
        assert_eq!(app.settings_cursor, 1);

        press(&mut app, KeyCode::Up).await;
        assert_eq!(app.settings_cursor, 0);
    }

    #[tokio::test]
    async fn former_settings_shortcuts_are_unbound() {
        let mut app = test_app();

        for code in ['t', 'g', 'n'] {
            press_with_modifiers(&mut app, KeyCode::Char(code), KeyModifiers::CONTROL).await;
        }

        assert_eq!(app.mode, Mode::Browsing);
        assert_eq!(app.config.target, Config::default().target);
        assert_eq!(app.config.channel, Config::default().channel);
        assert_eq!(app.config.theme, Config::default().theme);
    }

    #[test]
    fn every_settings_row_opens_its_own_page() {
        let mut app = test_app();
        let pages = [
            SettingsPage::InputMode,
            SettingsPage::Theme,
            SettingsPage::Target,
            SettingsPage::Channel,
        ];

        for (index, page) in pages.into_iter().enumerate() {
            app.settings_page = SettingsPage::Main;
            app.settings_cursor = index;
            select_setting(&mut app);
            assert_eq!(app.settings_page, page);

            close_settings_page(&mut app);
            assert_eq!(app.settings_page, SettingsPage::Main);
            assert_eq!(app.settings_cursor, index);
        }
    }

    #[tokio::test]
    async fn normal_input_mode_types_without_vim_transitions() {
        let mut app = test_app();
        app.apply_input_mode(InputMode::Normal);
        app.tab = Tab::Installed;

        press(&mut app, KeyCode::Char('h')).await;
        assert_eq!(app.installed_input.value(), "h");
        assert_eq!(app.installed_input.mode(), VimMode::Insert);

        press(&mut app, KeyCode::Esc).await;
        assert!(app.should_quit);
        assert_eq!(app.installed_input.mode(), VimMode::Insert);
    }
}
