use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Block;

use crate::app::{App, Mode, Tab};
use crate::theme;

mod bars;
mod build;
mod flakes;
mod installed;
mod options;
mod popups;
mod queue;
mod search;

pub(crate) const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

pub(crate) fn panel<'a>(t: &theme::Theme) -> Block<'a> {
	use ratatui::widgets::Borders;
	Block::default()
		.borders(Borders::ALL)
		.border_type(t.border_type())
		.border_style(t.border_style())
}

pub(crate) fn titled_panel<'a>(t: &theme::Theme, title: Span<'a>) -> Block<'a> {
	panel(t).title(title)
}

pub(crate) fn draw(f: &mut Frame, app: &App) {
	if let Some(bg) = app.theme().bg_color {
		f.render_widget(Block::default().style(Style::default().bg(bg)), f.area());
	}

	let show_search_bar = matches!(app.tab, Tab::Search | Tab::Flakes | Tab::Installed);

	let mut constraints = vec![Constraint::Length(1)];
	if show_search_bar {
		constraints.push(Constraint::Length(3));
	}
	constraints.push(Constraint::Min(8));
	constraints.push(Constraint::Length(2));

	let chunks = Layout::default()
		.direction(Direction::Vertical)
		.constraints(constraints)
		.split(f.area());

	let mut i = 0;
	bars::draw_tab_strip(f, chunks[i], app);
	i += 1;
	if show_search_bar {
		bars::draw_search_bar(f, chunks[i], app);
		i += 1;
	}
	let body_area = chunks[i];
	i += 1;
	let footer_area = chunks[i];

	match app.tab {
		Tab::Search => search::draw_search_body(f, body_area, app),
		Tab::Flakes => flakes::draw_flakes_body(f, body_area, app),
		Tab::Installed if app.options_panel.is_some() => {
			options::draw_options_body(f, body_area, app);
		}
		Tab::Installed => installed::draw_installed_body(f, body_area, app),
		Tab::Building => build::draw_build_body(f, body_area, app),
		Tab::Queue => queue::draw_queue_body(f, body_area, app),
		Tab::Vcs => crate::repository::draw(f, body_area, app),
	}

	bars::draw_footer(f, footer_area, app);

	if matches!(app.mode, Mode::SettingsSelect) {
		popups::draw_settings_popup(f, app);
	}
	if app.flake_picker.is_some() {
		flakes::draw_output_picker(f, app);
	}
}
