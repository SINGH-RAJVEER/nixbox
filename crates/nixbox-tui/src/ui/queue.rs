use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::{SPINNER, titled_panel};
use crate::app::App;

pub(super) fn draw_queue_body(f: &mut Frame, area: Rect, app: &App) {
	let t = app.theme();
	let title = format!("Queue  ·  {} pending", app.session.queue.len());
	let block = titled_panel(t, Span::styled(title, t.title_style()));

	if app.session.queue.is_empty() {
		f.render_widget(Paragraph::new("Queue is empty.").block(block), area);
		return;
	}

	let dim = Style::default().add_modifier(Modifier::DIM);
	let mut lines: Vec<Line> = Vec::new();

	if let Some(running) = app.session.build_label() {
		let spin = SPINNER[app.spinner_frame % SPINNER.len()];
		lines.push(Line::from(vec![
			Span::styled(format!("{}  ", spin), t.title_style()),
			Span::styled(running.to_string(), t.name_style()),
			Span::styled("  running", dim),
		]));
		lines.push(Line::raw(""));
	}

	for (i, op) in app.session.queue.iter().enumerate() {
		lines.push(Line::from(vec![
			Span::styled(format!("{:>2}.  ", i + 1), dim),
			Span::styled(op.label(), t.name_style()),
		]));
	}

	f.render_widget(
		Paragraph::new(lines)
			.block(block)
			.wrap(Wrap { trim: false }),
		area,
	);
}
