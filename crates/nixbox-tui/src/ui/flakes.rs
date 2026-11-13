use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph, Wrap};

use super::{SPINNER, titled_panel};
use crate::app::App;
use nixbox_core::{FlakeOutputGroup, flake_output_groups};
use nixbox_nix::flakes::FlakeOutputKind;

pub(super) fn draw_output_picker(f: &mut Frame, app: &App) {
	let (Some(index), Some(details)) = (app.flake_picker, app.flake_details.as_ref()) else {
		return;
	};
	let groups = flake_output_groups(details, app.session.engine.config.target);
	// Listed in choice order, so the picker index is the `flake_choices` index.
	let mut rows: Vec<_> = groups
		.iter()
		.flat_map(|group| {
			group
				.items
				.iter()
				.filter_map(move |item| item.choice.map(|choice| (choice, item, group)))
		})
		.collect();
	rows.sort_by_key(|(choice, _, _)| *choice);
	let width = rows
		.iter()
		.map(|(_, item, _)| item.name.chars().count())
		.max()
		.unwrap_or(0);

	let screen = f.area();
	let popup_width = screen.width.min(76);
	let height = screen
		.height
		.saturating_sub(2)
		.min(rows.len().saturating_add(2) as u16);
	let area = Rect::new(
		screen.x + screen.width.saturating_sub(popup_width) / 2,
		screen.y + screen.height.saturating_sub(height) / 2,
		popup_width,
		height,
	);
	f.render_widget(Clear, area);
	let theme = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let items: Vec<ListItem> = rows
		.into_iter()
		.map(|(_, item, group)| {
			let mut spans = vec![
				Span::styled(format!("{:<width$}", item.name), theme.name_style()),
				Span::styled(format!("  {}", group.title), dim),
			];
			if let Some(detail) = &item.detail {
				spans.push(Span::styled(format!("  {detail}"), dim));
			}
			ListItem::new(Line::from(spans))
		})
		.collect();
	let list = List::new(items)
		.block(titled_panel(
			theme,
			Span::styled(
				" Choose output (Enter install, Esc close) ",
				theme.title_style(),
			),
		))
		.highlight_style(theme.selection_style())
		.highlight_symbol("❯ ");
	let mut state = ListState::default();
	state.select(Some(index));
	f.render_stateful_widget(list, area, &mut state);
}
pub(super) fn draw_flakes_body(f: &mut Frame, area: Rect, app: &App) {
	let split = Layout::default()
		.direction(Direction::Horizontal)
		.constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
		.split(area);
	draw_results(f, split[0], app);
	draw_details(f, split[1], app);
}

fn draw_results(f: &mut Frame, area: Rect, app: &App) {
	let t = app.theme();
	let block = titled_panel(t, Span::styled("GitHub flakes", t.title_style()));
	let dim = Style::default().add_modifier(Modifier::DIM);
	if app.flake_searching && app.flake_results.is_empty() {
		let spinner = SPINNER[app.spinner_frame % SPINNER.len()];
		f.render_widget(
			Paragraph::new(Line::from(vec![
				Span::styled(format!(" {} ", spinner), t.version_style()),
				Span::styled("Finding and checking flake outputs…", dim),
			]))
			.block(block),
			area,
		);
		return;
	}
	if app.flake_results.is_empty() {
		let message = if app.flake_query.is_empty() {
			"Type a project name, input, or output to search root flake.nix files."
		} else {
			"No installable flake outputs matched this query."
		};
		f.render_widget(
			Paragraph::new(message)
				.block(block)
				.wrap(Wrap { trim: false }),
			area,
		);
		return;
	}

	let items: Vec<ListItem> = app
		.flake_results
		.iter()
		.map(|hit| ListItem::new(Line::from(Span::styled(hit.repo.clone(), t.name_style()))))
		.collect();
	let list = List::new(items)
		.block(block)
		.highlight_style(t.selection_style())
		.highlight_symbol("❯ ");
	let mut state = ListState::default();
	state.select(Some(app.flake_selected));
	f.render_stateful_widget(list, area, &mut state);
}

fn draw_details(f: &mut Frame, area: Rect, app: &App) {
	let t = app.theme();
	let block = titled_panel(t, Span::styled("Flake properties", t.title_style()));
	let dim = Style::default().add_modifier(Modifier::DIM);
	if app.flake_detail_loading {
		let spinner = SPINNER[app.spinner_frame % SPINNER.len()];
		f.render_widget(
			Paragraph::new(Line::from(vec![
				Span::styled(format!(" {} ", spinner), t.version_style()),
				Span::styled("Reading repository and flake.nix…", dim),
			]))
			.block(block),
			area,
		);
		return;
	}
	let Some(details) = &app.flake_details else {
		f.render_widget(
			Paragraph::new(vec![
				Line::from(Span::styled("GitHub flake browser", t.title_style())),
				Line::raw(""),
				Line::from(Span::styled("Searches only root flake.nix files.", dim)),
				Line::from(Span::styled("Results are not cloned or persisted.", dim)),
				Line::raw(""),
				Line::from(Span::styled(
					"i  search    j/k  select    PgUp/PgDn  details    Enter  install output",
					dim,
				)),
			])
			.block(block)
			.wrap(Wrap { trim: false }),
			area,
		);
		return;
	};

	let description = details.description.as_deref().unwrap_or("(no description)");
	let target = app.session.engine.config.target;
	let inputs = list_or_none(&details.inputs);
	let topics = list_or_none(&details.topics);
	let pushed = details.pushed_at.as_deref().unwrap_or("unknown");
	let homepage = details.homepage.as_deref().unwrap_or("none");
	let status = if details.archived {
		"archived"
	} else {
		"active"
	};
	let mut lines = vec![
		Line::from(Span::styled(details.repo.clone(), t.name_style())),
		Line::from(Span::styled(
			format!(
				"★ {}  ·  {}  ·  {}",
				details.stars, status, details.default_branch
			),
			t.version_style(),
		)),
		Line::raw(""),
		Line::from(Span::raw(description.to_string())),
		Line::raw(""),
	];

	let groups = flake_output_groups(details, target);
	let installable = groups
		.iter()
		.flat_map(|group| &group.items)
		.filter(|item| item.choice.is_some())
		.count();
	let summary = match installable {
		0 => format!("nothing installs into {}", target.title()),
		1 => format!("1 installs into {}", target.title()),
		n => format!("{n} install into {}", target.title()),
	};
	lines.push(Line::from(vec![
		Span::styled("OUTPUTS", t.title_style()),
		Span::styled(format!("  {summary}"), dim),
	]));
	for group in &groups {
		output_group_lines(&mut lines, group, app);
	}

	lines.extend([
		Line::raw(""),
		Line::from(Span::styled("REPOSITORY", t.title_style())),
		Line::from(vec![Span::styled("inputs    ", dim), Span::raw(inputs)]),
		Line::from(vec![Span::styled("topics    ", dim), Span::raw(topics)]),
		Line::from(vec![
			Span::styled("flake     ", dim),
			Span::raw(details.path.clone()),
		]),
		Line::from(vec![
			Span::styled("url       ", dim),
			Span::raw(details.repo_url.clone()),
		]),
		Line::from(vec![Span::styled("updated   ", dim), Span::raw(pushed)]),
		Line::from(vec![Span::styled("homepage  ", dim), Span::raw(homepage)]),
	]);
	f.render_widget(
		Paragraph::new(lines)
			.block(block)
			.scroll((app.flake_details_scroll, 0))
			.wrap(Wrap { trim: false }),
		area,
	);
}

/// A group header, then one aligned row per output. Families that cannot be
/// installed into the active target are dimmed, and the leftover top-level
/// outputs share a single line.
fn output_group_lines(lines: &mut Vec<Line<'_>>, group: &FlakeOutputGroup, app: &App) {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let mut header = vec![
		Span::styled(format!(" {}", group.title), t.version_style()),
		Span::styled(format!("  {}", group.items.len()), dim),
	];
	if let Some(note) = &group.unavailable {
		header.push(Span::styled(format!("  ·  {note}"), dim));
	}
	lines.push(Line::from(header));

	if group.kind == FlakeOutputKind::Other {
		let names: Vec<_> = group.items.iter().map(|item| item.name.as_str()).collect();
		lines.push(Line::from(Span::styled(
			format!("   {}", names.join("  ·  ")),
			dim,
		)));
		return;
	}

	let width = group
		.items
		.iter()
		.map(|item| item.name.chars().count())
		.max()
		.unwrap_or(0);
	for item in &group.items {
		let name_style = if item.choice.is_some() {
			t.name_style()
		} else {
			dim
		};
		let mut row = vec![Span::styled(
			format!("   {:<width$}", item.name),
			name_style,
		)];
		if let Some(detail) = &item.detail {
			row.push(Span::styled(format!("  {detail}"), dim));
		}
		lines.push(Line::from(row));
	}
}

fn list_or_none(values: &[String]) -> String {
	if values.is_empty() {
		"(not detected)".to_string()
	} else {
		values.join("  ·  ")
	}
}
