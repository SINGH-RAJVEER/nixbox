use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::titled_panel;
use crate::app::App;

pub(super) fn draw_build_body(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let title = match app.session.build_label() {
        Some(label) => format!("Build output  ·  {}", label),
        None => "Build output".to_string(),
    };
    let block = titled_panel(t, Span::styled(title, t.title_style()));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let log_height = inner.height as usize;
    let start = app.session.log.len().saturating_sub(log_height);
    let text: Vec<Line> = app.session.log[start..]
        .iter()
        .map(|l| Line::from(l.clone()))
        .collect();
    f.render_widget(Paragraph::new(text), inner);
}
