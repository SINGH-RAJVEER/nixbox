//! The NixOS tab, laid out like the desktop page: a header line naming the
//! host, then categories, options, and the selected option's details.
//! Narrow terminals drop the category column and name the category in the
//! option list's title.

use nixbox_core::options::plain_markdown;
use nixbox_core::system_options::{Category, access_note, option_tags, partial_note, value_text};
use nixbox_nix::settings::option_path_display;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Wrap};

use super::options::{pad, push_block};
use super::{SPINNER, titled_panel};
use crate::app::App;
use crate::nixos::Panel;

/// Below this width the category column is hidden.
const WIDE: u16 = 110;
const CATEGORY_WIDTH: u16 = 34;

pub(super) fn draw_nixos_body(f: &mut Frame, area: Rect, app: &App) {
	let rows = Layout::default()
		.direction(Direction::Vertical)
		.constraints([Constraint::Length(1), Constraint::Min(4)])
		.split(area);
	draw_header(f, rows[0], app);

	let panel = &app.nixos;
	if let Some(message) = placeholder(app, panel) {
		let t = app.theme();
		let block = titled_panel(t, Span::styled(" NixOS ", t.title_style()));
		f.render_widget(
			Paragraph::new(message)
				.block(block)
				.wrap(Wrap { trim: false }),
			rows[1],
		);
		return;
	}

	let columns = if area.width >= WIDE {
		Layout::default()
			.direction(Direction::Horizontal)
			.constraints([
				Constraint::Length(CATEGORY_WIDTH),
				Constraint::Min(30),
				Constraint::Percentage(40),
			])
			.split(rows[1])
	} else {
		Layout::default()
			.direction(Direction::Horizontal)
			.constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
			.split(rows[1])
	};
	let (list_area, details_area) = if columns.len() == 3 {
		draw_categories(f, columns[0], app, panel);
		(columns[1], columns[2])
	} else {
		(columns[0], columns[1])
	};
	draw_options(f, list_area, app, panel);
	draw_details(f, details_area, app, panel);
}

/// "NixOS  host workstation  in ~/.config/nixos" with the browser's state on
/// the right.
fn draw_header(f: &mut Frame, area: Rect, app: &App) {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let panel = &app.nixos;
	let mut left = vec![
		Span::styled(" NixOS ", t.selection_style()),
		Span::raw("  "),
	];
	match &panel.configuration {
		Some(configuration) => {
			left.push(Span::styled("host ", dim));
			left.push(Span::styled(configuration.host.clone(), t.name_style()));
			let position = panel
				.hosts
				.iter()
				.position(|host| *host == configuration.host);
			if let (true, Some(position)) = (panel.hosts.len() > 1, position) {
				left.push(Span::styled(
					format!(" ({} of {}, ctrl-t next)", position + 1, panel.hosts.len()),
					dim,
				));
			}
		}
		None => left.push(Span::styled("no host selected", dim)),
	}
	left.push(Span::styled(
		format!("  in {}", app.session.config_dir().display()),
		dim,
	));
	let mut state = vec!["read-only"];
	if panel.show_diagnostic {
		state.push("diagnostic shown");
	}
	let right = format!("{} ", state.join("  ·  "));
	let width = right.chars().count() as u16;
	let split = Layout::default()
		.direction(Direction::Horizontal)
		.constraints([Constraint::Min(10), Constraint::Length(width)])
		.split(area);
	f.render_widget(Paragraph::new(Line::from(left)), split[0]);
	f.render_widget(Paragraph::new(Span::styled(right, dim)), split[1]);
}

/// The message that replaces the columns while there is nothing to browse.
fn placeholder(app: &App, panel: &Panel) -> Option<Vec<Line<'static>>> {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	if panel.loading {
		let spinner = SPINNER[app.spinner_frame % SPINNER.len()];
		return Some(vec![Line::from(vec![
			Span::styled(format!(" {spinner} "), t.version_style()),
			Span::styled("Reading the selected host's option schema…", dim),
		])]);
	}
	if let Some(error) = &panel.error {
		return Some(vec![
			Line::from(Span::styled(
				"Could not read the NixOS schema.",
				t.title_style(),
			)),
			Line::raw(""),
			Line::raw(error.clone()),
			Line::raw(""),
			Line::from(Span::styled("ctrl-l reload", dim)),
		]);
	}
	if panel.hosts.is_empty() {
		return Some(vec![
			Line::raw("This flake has no nixosConfigurations outputs."),
			Line::from(Span::styled(
				"Point nixbox at a NixOS configuration directory, then ctrl-l to reload.",
				dim,
			)),
		]);
	}
	if panel.configuration.is_none() {
		return Some(vec![
			Line::raw(format!("Hosts: {}", panel.hosts.join(", "))),
			Line::from(Span::styled(
				"ctrl-t chooses a host. This does not change the package install target.",
				dim,
			)),
		]);
	}
	None
}

fn draw_categories(f: &mut Frame, area: Rect, app: &App, panel: &Panel) {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let block = titled_panel(t, Span::styled(" Categories ", t.title_style()));
	let label_width = (area.width as usize).saturating_sub(2 + 2 + 5);
	let items: Vec<ListItem> = Category::ALL
		.into_iter()
		.map(|category| {
			let count = panel.filtered.count(category);
			let style = if count == 0 { dim } else { Style::default() };
			ListItem::new(Line::from(vec![
				Span::styled(pad(category.label(), label_width), style),
				Span::styled(format!("{count:>5}"), dim),
			]))
		})
		.collect();
	let mut state = ListState::default();
	state.select(Some(panel.category.index()));
	f.render_stateful_widget(
		List::new(items)
			.block(block)
			.highlight_style(t.selection_style())
			.highlight_symbol("❯ "),
		area,
		&mut state,
	);
}

fn draw_options(f: &mut Frame, area: Rect, app: &App, panel: &Panel) {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let visible = &panel.filtered.visible;
	let title = format!(
		" {}  ·  {} of {} ",
		panel.category.label(),
		visible.len(),
		panel.options.len()
	);
	let block = titled_panel(t, Span::styled(title, t.title_style()));
	if visible.is_empty() {
		f.render_widget(
			Paragraph::new(Span::styled(
				"No options match this category and search.",
				dim,
			))
			.block(block),
			area,
		);
		return;
	}

	// Only the rows on screen are built; a schema has tens of thousands.
	let height = (area.height as usize).saturating_sub(2).max(1);
	let cursor = panel.cursor.min(visible.len() - 1);
	let start = cursor
		.saturating_sub(height / 2)
		.min(visible.len().saturating_sub(height));
	let path_width = (area.width as usize).saturating_sub(4).max(8);
	let items: Vec<ListItem> = visible[start..(start + height).min(visible.len())]
		.iter()
		.map(|index| {
			let option = &panel.options[*index];
			let tags = option_tags(option);
			let tag_text = if tags.is_empty() {
				String::new()
			} else {
				format!("  {}", tags.join(" · "))
			};
			let name_width = path_width.saturating_sub(tag_text.chars().count());
			let name_style = if option.read_only {
				t.name_style().add_modifier(Modifier::DIM)
			} else {
				t.name_style()
			};
			ListItem::new(Line::from(vec![
				Span::styled(
					pad(&option_path_display(&option.path), name_width),
					name_style,
				),
				Span::styled(tag_text, dim),
			]))
		})
		.collect();
	let mut state = ListState::default();
	state.select(Some(cursor - start));
	f.render_stateful_widget(
		List::new(items)
			.block(block)
			.highlight_style(t.selection_style())
			.highlight_symbol("❯ "),
		area,
		&mut state,
	);
}

fn draw_details(f: &mut Frame, area: Rect, app: &App, panel: &Panel) {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let block = titled_panel(t, Span::styled(" Option ", t.title_style()));
	let Some(option) = panel.current() else {
		f.render_widget(
			Paragraph::new(Span::styled(
				"Select an option to inspect its value and where it is defined.",
				dim,
			))
			.block(block)
			.wrap(Wrap { trim: false }),
			area,
		);
		return;
	};

	let tags = option_tags(option);
	let mut type_line = vec![Span::styled(
		option.type_description.clone(),
		t.version_style(),
	)];
	if !tags.is_empty() {
		type_line.push(Span::styled(format!("  {}", tags.join(" · ")), dim));
	}
	let mut lines = vec![
		Line::from(Span::styled(
			option_path_display(&option.path),
			t.name_style(),
		)),
		Line::from(type_line),
	];
	if let Some(description) = &option.description {
		lines.push(Line::raw(""));
		for line in description.lines() {
			lines.push(Line::raw(plain_markdown(line)));
		}
	}
	lines.push(Line::raw(""));
	lines.push(Line::from(Span::styled(access_note(option), dim)));
	if let Some(note) = partial_note(option) {
		lines.push(Line::from(Span::styled(note, dim)));
	}

	lines.push(Line::raw(""));
	lines.push(Line::from(Span::styled("Value", t.title_style())));
	if panel.inspecting {
		let spinner = SPINNER[app.spinner_frame % SPINNER.len()];
		lines.push(Line::from(vec![
			Span::styled(format!("{spinner} "), t.version_style()),
			Span::styled("Reading value and definition locations…", dim),
		]));
	} else if let Some(error) = &panel.detail_error {
		lines.push(Line::raw(error.clone()));
	}
	let details = panel.current_details();
	match details {
		Some(details) => {
			push_block(
				&mut lines,
				"effective ",
				text_lines(value_text(&details.value)),
				dim,
			);
			if let Some(text) = &details.default {
				push_block(&mut lines, "default   ", text_lines(text.clone()), dim);
			}
			if let Some(text) = &details.example {
				push_block(&mut lines, "example   ", text_lines(text.clone()), dim);
			}
		}
		None if !panel.inspecting => lines.push(Line::from(Span::styled(
			"↵ inspect to read the value and definition locations.",
			dim,
		))),
		None => {}
	}

	lines.push(Line::raw(""));
	lines.push(Line::from(Span::styled("Source", t.title_style())));
	if let Some(files) = details.and_then(|details| details.definitions.as_ref()) {
		let files = if files.is_empty() {
			vec!["No definitions.".to_string()]
		} else {
			files.clone()
		};
		push_block(&mut lines, "defined   ", files, dim);
	}
	push_block(&mut lines, "declared  ", option.declarations.clone(), dim);

	f.render_widget(
		Paragraph::new(lines)
			.block(block)
			.wrap(Wrap { trim: false })
			.scroll((panel.detail_scroll, 0)),
		area,
	);
}

fn text_lines(text: String) -> Vec<String> {
	text.lines().map(str::to_string).collect()
}
