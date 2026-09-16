use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState};

use super::titled_panel;
use crate::app::{App, CHANNELS, INPUT_MODES, SettingsPage, TARGETS};
use crate::theme;

pub(super) fn draw_settings_popup(f: &mut Frame, app: &App) {
    let t = app.theme();
    let area = f.area();
    let popup_width: u16 = 42;
    let (title, labels, selected) = settings_content(app);
    let popup_height = labels.len() as u16 + 2;
    let x = area.x + area.width.saturating_sub(popup_width) / 2;
    let y = area.y + area.height.saturating_sub(popup_height) / 2;
    let popup_area = Rect::new(
        x,
        y,
        popup_width.min(area.width),
        popup_height.min(area.height),
    );

    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = labels
        .into_iter()
        .map(|label| ListItem::new(Line::from(Span::styled(label, t.name_style()))))
        .collect();

    let list = List::new(items)
        .block(titled_panel(t, Span::styled(title, t.title_style())))
        .highlight_style(t.selection_style())
        .highlight_symbol("❯");

    let mut state = ListState::default();
    state.select(Some(selected));
    f.render_stateful_widget(list, popup_area, &mut state);
}

fn settings_content(app: &App) -> (&'static str, Vec<String>, usize) {
    let selected = app.settings_cursor;
    match app.settings_page {
        SettingsPage::Main => (
            " Settings ",
            vec![
                format!(
                    "  {:<12} {}  ›",
                    "Input mode",
                    app.engine.config.input_mode.label()
                ),
                format!("  {:<12} {}  ›", "Theme", theme::ALL[app.theme_index].name),
                format!("  {:<12} {}  ›", "Target", app.engine.config.target.label()),
                format!("  {:<12} {}  ›", "Channel", app.engine.config.channel),
            ],
            selected,
        ),
        SettingsPage::InputMode => (
            " Input Mode ",
            INPUT_MODES
                .iter()
                .map(|mode| option_label(mode.label(), *mode == app.engine.config.input_mode))
                .collect(),
            selected,
        ),
        SettingsPage::Theme => (
            " Theme ",
            theme::ALL
                .iter()
                .enumerate()
                .map(|(index, value)| option_label(value.name, index == app.theme_index))
                .collect(),
            selected,
        ),
        SettingsPage::Target => (
            " Target ",
            TARGETS
                .iter()
                .map(|target| option_label(target.label(), *target == app.engine.config.target))
                .collect(),
            selected,
        ),
        SettingsPage::Channel => (
            " Channel ",
            CHANNELS
                .iter()
                .map(|channel| option_label(channel, *channel == app.engine.config.channel))
                .collect(),
            selected,
        ),
    }
}

fn option_label(label: &str, selected: bool) -> String {
    format!("  {}{}", label, if selected { "  ✓" } else { "" })
}
