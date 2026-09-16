use nixbox_config::InputMode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use super::{SPINNER, panel};
use crate::app::{App, Mode, SettingsPage, Tab};
use crate::vim::VimMode;

pub(super) fn draw_search_bar(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let dim = Style::default().add_modifier(Modifier::DIM);
    let (input, placeholder) = match app.tab {
        Tab::Flakes => (&app.flake_input, "search GitHub flake contents"),
        Tab::Installed => (&app.installed_input, "search installed applications"),
        _ => (&app.input, "search nixpkgs"),
    };
    let (mode_label, mode_style) = match (app.engine.config.input_mode, input.mode()) {
        (InputMode::Normal, _) => (" NORMAL ", t.title_style()),
        (InputMode::Vim, VimMode::Insert) => (" INSERT ", t.title_style()),
        (InputMode::Vim, VimMode::Normal) => (" NORMAL ", dim),
        (InputMode::Vim, VimMode::Visual) => (" VISUAL ", t.selection_style()),
    };

    f.render_widget(panel(t), area);
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    const PREFIX_WIDTH: u16 = 10;
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(PREFIX_WIDTH.min(inner.width)),
            Constraint::Min(0),
        ])
        .split(inner);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(mode_label, mode_style),
            Span::raw("  "),
        ])),
        chunks[0],
    );

    if chunks[1].width == 0 {
        return;
    }

    let selection = input.selection_range();
    let mut value_spans: Vec<Span> = input
        .value()
        .chars()
        .enumerate()
        .map(|(index, ch)| {
            let selected = match input.mode() {
                VimMode::Normal => index == input.cursor(),
                VimMode::Visual => {
                    selection.is_some_and(|(start, end)| (start..=end).contains(&index))
                }
                VimMode::Insert => false,
            };
            Span::styled(
                ch.to_string(),
                if selected {
                    t.selection_style()
                } else {
                    Style::default()
                },
            )
        })
        .collect();
    if input.value().is_empty() {
        if input.mode() == VimMode::Normal {
            value_spans.push(Span::styled(" ", t.selection_style()));
        } else {
            value_spans.push(Span::styled(placeholder, dim));
        }
    }

    let width = chunks[1].width as usize;
    let scroll = input.visual_scroll(width.saturating_sub(1));
    f.render_widget(
        Paragraph::new(Line::from(value_spans)).scroll((0, scroll as u16)),
        chunks[1],
    );

    if input.mode() == VimMode::Insert {
        let cursor_x = chunks[1]
            .x
            .saturating_add(input.visual_cursor().saturating_sub(scroll) as u16)
            .min(chunks[1].right().saturating_sub(1));
        f.set_cursor_position((cursor_x, chunks[1].y));
    }
}

pub(super) fn draw_tab_strip(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let dim = Style::default().add_modifier(Modifier::DIM);
    let tabs = app.visible_tabs();

    let context_width = context_pills_width(app).min(area.width as usize) as u16;
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(10), Constraint::Length(context_width)])
        .split(area);

    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    for tab in tabs.iter() {
        let is_active = *tab == app.tab;

        let mut label = tab.label().to_string();
        if matches!(tab, Tab::Building) && app.build_in_progress {
            let spin = SPINNER[app.spinner_frame % SPINNER.len()];
            label = format!("{} {}", spin, label);
        }
        if matches!(tab, Tab::Queue) && !app.queue.is_empty() {
            label = format!("{} ({})", label, app.queue.len());
        }

        let style = if is_active { t.selection_style() } else { dim };
        spans.push(Span::styled(format!(" {} ", label), style));
        spans.push(Span::raw("  "));
    }

    let context = Line::from(vec![
        Span::styled("channel ", dim),
        Span::styled(app.channel().to_string(), t.name_style()),
        Span::styled("  target ", dim),
        Span::styled(app.target_label().to_string(), t.title_style()),
        Span::raw(" "),
    ]);

    f.render_widget(Paragraph::new(Line::from(spans)), chunks[0]);
    f.render_widget(Paragraph::new(context), chunks[1]);
}

fn context_pills_width(app: &App) -> usize {
    // Keep channel/target visible without dedicating a full top bar to them.
    18 + app.channel().chars().count() + app.target_label().chars().count()
}

pub(super) fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let status = app.status.as_str();
    let keys = context_keys(app);

    let inner_width = area.width as usize;
    let keys_len = keys.chars().count();
    let status_chars = status.chars().count();
    let pad = inner_width.saturating_sub(status_chars + keys_len + 2);

    let line = Line::from(vec![
        Span::raw(status.to_string()),
        Span::raw(format!("  {:pad$}", "", pad = pad)),
        Span::styled(keys, Style::default().add_modifier(Modifier::DIM)),
    ]);

    let block = Block::default()
        .borders(Borders::TOP)
        .border_type(BorderType::Plain)
        .border_style(t.border_style());

    f.render_widget(Paragraph::new(line).block(block), area);
}

fn context_keys(app: &App) -> &'static str {
    match app.mode {
        Mode::SettingsSelect
            if app.settings_page == SettingsPage::Main
                && app.engine.config.input_mode == InputMode::Normal =>
        {
            "↑↓ select  ↵ open  esc close"
        }
        Mode::SettingsSelect if app.engine.config.input_mode == InputMode::Normal => {
            "↑↓ select  ↵ confirm  esc back"
        }
        Mode::SettingsSelect if app.settings_page == SettingsPage::Main => {
            "↑↓/j/k select  ↵ open  esc close"
        }
        Mode::SettingsSelect => "↑↓/j/k select  ↵ confirm  esc back",
        Mode::Browsing if app.engine.config.input_mode == InputMode::Normal => match app.tab {
            Tab::Search => {
                "type  ←→ cursor  ↑↓ results  tab/shift-tab tabs  ↵ install  ctrl-s settings"
            }
            Tab::Flakes => "type  ←→ cursor  ↑↓ results  Enter install  tab/shift-tab tabs",
            Tab::Installed => {
                "type to filter  ←→ cursor  ↑↓ results  tab/shift-tab tabs  ctrl-s settings"
            }
            Tab::Building if app.build_in_progress => {
                "c cancel build  tab/shift-tab tabs  ctrl-s settings  esc quit"
            }
            Tab::Building | Tab::Queue => "tab/shift-tab tabs  ctrl-s settings  esc quit",
        },
        Mode::Browsing => match app.tab {
            Tab::Search => match app.input.mode() {
                VimMode::Insert => {
                    "type  ←→ cursor  ↑↓ results  Enter install  tab/shift-tab tabs  esc normal"
                }
                VimMode::Normal => {
                    "h/l/←→ cursor  v visual  i/a insert  tab/shift-tab tabs  ctrl-s settings"
                }
                VimMode::Visual => {
                    "h/l/w/b select  d/x delete  c change  esc normal  ctrl-s settings"
                }
            },
            Tab::Flakes => match app.flake_input.mode() {
                VimMode::Insert => {
                    "type  ←→ cursor  ↑↓ results  tab/shift-tab tabs  esc normal  ctrl-s settings"
                }
                VimMode::Normal => {
                    "h/l/←→ cursor  v visual  i/a insert  j/k results  Enter install"
                }
                VimMode::Visual => {
                    "h/l/w/b select  d/x delete  c change  esc normal  ctrl-s settings"
                }
            },
            Tab::Installed => match app.installed_input.mode() {
                VimMode::Insert => {
                    "type to filter  ←→ cursor  ↑↓ results  tab/shift-tab tabs  ctrl-s settings"
                }
                VimMode::Normal => {
                    "h/l cursor  i/a filter  tab/shift-tab tabs  d uninstall  ctrl-s settings"
                }
                VimMode::Visual => {
                    "h/l/w/b select  d/x delete  c change  esc normal  ctrl-s settings"
                }
            },
            Tab::Building if app.build_in_progress => {
                "c cancel build  tab/shift-tab tabs  ctrl-s settings  esc quit"
            }
            Tab::Building | Tab::Queue => "tab/shift-tab tabs  ctrl-s settings  esc quit",
        },
    }
}
