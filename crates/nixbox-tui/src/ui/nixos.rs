//! The NixOS tab: a header naming the host and the staged edits, the pages
//! pane, and the open view. Settings pages list each setting with its value
//! inline above a details strip; All options pairs the namespace tree with
//! the highlighted option's details. Narrow terminals show one pane at a
//! time.

use nixbox_core::settings_pages::{
	PageGroup, SettingsPage, TreeRow, View, curated, documentation, label_for, tree_label,
};
use nixbox_core::system_options::{lock_note, option_tags, partial_note, value_text};
use nixbox_nix::options::OptionKind;
use nixbox_nix::settings::{SettingValue, option_path_display};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use super::options::{pad, push_block};
use super::{SPINNER, titled_panel};
use crate::app::App;
use crate::nixos::{Focus, Item, Panel, Popup, baseline, current, editable_host, lock};

/// Below this width only the focused pane shows.
const WIDE: u16 = 100;
const PAGES_WIDTH: u16 = 30;
const LABEL_WIDTH: usize = 34;
const VALUE_WIDTH: usize = 30;

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

	let content = if area.width >= WIDE {
		let columns = Layout::default()
			.direction(Direction::Horizontal)
			.constraints([Constraint::Length(PAGES_WIDTH), Constraint::Min(30)])
			.split(rows[1]);
		draw_pages(f, columns[0], app);
		columns[1]
	} else if panel.focus == Focus::Pages {
		draw_pages(f, rows[1], app);
		return;
	} else {
		rows[1]
	};
	if panel.focus == Focus::Search || !panel.search.is_empty() {
		draw_results(f, content, app);
	} else {
		match panel.view {
			View::Overview => draw_overview(f, content, app),
			View::Page(_) | View::Changed => draw_settings(f, content, app),
			View::AllOptions => draw_tree(f, content, app),
		}
	}
	match &panel.popup {
		Some(Popup::Edit(_)) => draw_edit(f, area, app),
		Some(Popup::Review) => draw_review(f, area, app),
		None => {}
	}
}

/// "NixOS  host nixos  in ~/.config/nixos" with the staged edits on the
/// right.
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
			if panel.hosts.len() > 1 {
				left.push(Span::styled(" (ctrl-t next)", dim));
			}
			if !editable_host(app) {
				left.push(Span::styled(" read-only", dim));
			}
		}
		None => left.push(Span::styled("no host selected", dim)),
	}
	left.push(Span::styled(
		format!("  in {}", app.session.config_dir().display()),
		dim,
	));
	let mut state = Vec::new();
	if panel.inspecting {
		state.push(format!(
			"{} reading values",
			SPINNER[app.spinner_frame % SPINNER.len()]
		));
	}
	if panel.hardware.inventory.is_some() {
		state.push("local hardware".to_string());
	}
	if panel.show_diagnostic {
		state.push("diagnostic shown".to_string());
	}
	match panel.staged.len() {
		0 => {}
		count => state.push(format!("{count} staged · R review · a apply")),
	}
	let right = format!("{} ", state.join("  ·  "));
	let width = right.chars().count() as u16;
	let split = Layout::default()
		.direction(Direction::Horizontal)
		.constraints([Constraint::Min(10), Constraint::Length(width)])
		.split(area);
	f.render_widget(Paragraph::new(Line::from(left)), split[0]);
	let style = if panel.staged.is_empty() {
		dim
	} else {
		t.version_style()
	};
	f.render_widget(Paragraph::new(Span::styled(right, style)), split[1]);
}

/// The message that replaces the panes while there is nothing to browse.
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
			Line::from(Span::styled("r reload", dim)),
		]);
	}
	if panel.hosts.is_empty() {
		return Some(vec![
			Line::raw("This flake has no nixosConfigurations outputs."),
			Line::from(Span::styled(
				"Point nixbox at a NixOS configuration directory, then r to reload.",
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

fn draw_pages(f: &mut Frame, area: Rect, app: &App) {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let panel = &app.nixos;
	let focused = panel.focus == Focus::Pages;
	let block = titled_panel(t, Span::styled(" Pages ", t.title_style()));
	let width = (area.width as usize).saturating_sub(2);
	let changed = nixbox_core::settings_pages::changed_paths(&panel.staged, &app.session).len();
	let mut lines = vec![
		Line::from(Span::styled(" / search all settings", dim)),
		Line::raw(""),
	];
	let mut group = None;
	for (position, view) in View::all().into_iter().enumerate() {
		if let View::Page(page) = view
			&& group != Some(page.group())
		{
			group = Some(page.group());
			lines.push(Line::from(Span::styled(
				format!(" {}", page.group().label().to_uppercase()),
				dim,
			)));
		}
		if view == View::Changed {
			lines.push(Line::from(Span::styled(
				format!(" {}", "─".repeat(width.saturating_sub(2))),
				dim,
			)));
		}
		let count = match view {
			View::Page(page) => panel
				.staged
				.keys()
				.filter(|path| {
					nixbox_core::settings_pages::curated(path)
						.is_some_and(|(found, _, _)| found == page)
				})
				.count(),
			View::Changed => changed,
			View::AllOptions => panel.options.len(),
			View::Overview => 0,
		};
		let count = if count == 0 {
			String::new()
		} else {
			count.to_string()
		};
		let marker = if panel.view == view { "❯ " } else { "  " };
		let label_width = width.saturating_sub(2 + count.len() + 1);
		let text = format!("{marker}{}{count} ", pad(view.label(), label_width));
		let style = if focused && panel.nav == position {
			t.selection_style()
		} else if panel.view == view {
			t.name_style().add_modifier(Modifier::BOLD)
		} else {
			Style::default()
		};
		lines.push(Line::from(Span::styled(text, style)));
	}
	f.render_widget(
		Paragraph::new(lines).block(if focused {
			block.border_style(t.selection_style())
		} else {
			block
		}),
		area,
	);
}

fn content_block<'a>(app: &'a App, title: String) -> ratatui::widgets::Block<'a> {
	let t = app.theme();
	let block = titled_panel(t, Span::styled(format!(" {title} "), t.title_style()));
	if app.nixos.focus == Focus::Content {
		block.border_style(t.selection_style())
	} else {
		block
	}
}

/// Scrolls `lines` so `selected` stays in view, inside `area`'s border.
fn render_scrolled(
	f: &mut Frame,
	area: Rect,
	lines: Vec<Line<'static>>,
	selected: Option<usize>,
	block: ratatui::widgets::Block<'_>,
) {
	let height = area.height.saturating_sub(2) as usize;
	let offset = selected.map_or(0, |line| (line + 2).saturating_sub(height));
	f.render_widget(
		Paragraph::new(lines)
			.block(block)
			.scroll((offset as u16, 0)),
		area,
	);
}

fn draw_overview(f: &mut Frame, area: Rect, app: &App) {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let panel = &app.nixos;
	let mut lines = vec![
		Line::from(Span::styled(
			" Choose a page. Changes are staged and applied together in one rebuild.",
			dim,
		)),
		Line::raw(""),
	];
	let mut selected = None;
	let mut group = None;
	for (position, page) in SettingsPage::ALL.into_iter().enumerate() {
		if group != Some(page.group()) {
			group = Some(page.group());
			if page.group() == PageGroup::Software {
				lines.push(Line::raw(""));
			}
			lines.push(Line::from(Span::styled(
				format!(" {}", page.group().label()),
				t.title_style(),
			)));
		}
		let chosen = panel.focus == Focus::Content && panel.cursor == position;
		if chosen {
			selected = Some(lines.len());
		}
		let count = match panel.page_size(page) {
			1 => "1 setting".to_string(),
			size => format!("{size} settings"),
		};
		let style = if chosen {
			t.selection_style()
		} else {
			Style::default()
		};
		lines.push(Line::from(vec![
			Span::styled(
				format!(
					"{} {}",
					if chosen { "❯" } else { " " },
					pad(page.label(), 24)
				),
				style,
			),
			Span::styled(format!("{count:>13}   "), dim),
			Span::styled(page.description(), dim),
		]));
	}
	lines.push(Line::raw(""));
	lines.push(Line::from(Span::styled(
		format!(
			" / searches all {} options. All options browses them by namespace.",
			panel.options.len()
		),
		dim,
	)));
	render_scrolled(
		f,
		area,
		lines,
		selected,
		content_block(app, "Overview".into()),
	);
}

/// A setting's value as its row shows it, with the style and state tag.
fn value_cell(app: &App, path: &[String]) -> (String, Style, &'static str) {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let panel = &app.nixos;
	let Some(option) = panel.editing(path) else {
		let text = if panel.inspecting {
			SPINNER[app.spinner_frame % SPINNER.len()].to_string()
		} else {
			String::new()
		};
		return (text, dim, "");
	};
	if lock(app, option).is_some() {
		return (value_text(&option.details.value), dim, "locked");
	}
	let show = |value: Option<&SettingValue>| match (value, option.entry.kind.inner()) {
		(Some(SettingValue::Bool(true)), OptionKind::Bool) => "[x] on".to_string(),
		(Some(SettingValue::Bool(false)), OptionKind::Bool) => "[ ] off".to_string(),
		(Some(SettingValue::Str(text)), _) => text.clone(),
		(Some(value), _) => value.to_nix(),
		(None, _) => "unset".to_string(),
	};
	let before = baseline(app, option);
	if let Some(staged) = panel.staged.get(path) {
		let text = format!("{} → {}", show(before.as_ref()), show(staged.as_ref()));
		return (text, t.version_style(), "staged");
	}
	let queued = app
		.session
		.queued_option(nixbox_config::Target::NixosSystem, path)
		.is_some();
	let ours = app
		.session
		.engine
		.sets_option(nixbox_config::Target::NixosSystem, &option.entry);
	let state = if queued {
		"queued"
	} else if ours {
		"nixbox"
	} else {
		""
	};
	let style = match before {
		Some(SettingValue::Bool(true)) => t.name_style(),
		Some(SettingValue::Bool(false)) | None => dim,
		_ => Style::default(),
	};
	(show(before.as_ref()), style, state)
}

fn setting_line(app: &App, path: &[String], label: &str, chosen: bool) -> Line<'static> {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let (value, value_style, state) = value_cell(app, path);
	let marker = match state {
		"staged" => Span::styled(" ● ", t.version_style()),
		"queued" => Span::styled(" ◆ ", t.version_style()),
		_ => Span::raw("   "),
	};
	let label_style = if chosen {
		t.selection_style()
	} else {
		Style::default()
	};
	Line::from(vec![
		Span::styled(if chosen { "❯" } else { " " }, t.title_style()),
		marker,
		Span::styled(pad(label, LABEL_WIDTH), label_style),
		Span::raw(" "),
		Span::styled(pad(&value, VALUE_WIDTH), value_style),
		Span::raw(" "),
		Span::styled(
			state,
			if state == "staged" {
				t.version_style()
			} else {
				dim
			},
		),
	])
}

/// A settings page, or the changed settings, above the highlighted
/// setting's details.
fn draw_settings(f: &mut Frame, area: Rect, app: &App) {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let panel = &app.nixos;
	let (title, description) = match panel.view {
		View::Page(page) => (page.label(), page.description()),
		_ => (
			"Changed",
			"Staged edits, and edits queued for the next NixOS rebuild.",
		),
	};
	let strip = if panel.details_open { 22 } else { 7 };
	let block = content_block(app, title.into());
	let inner = block.inner(area);
	f.render_widget(block, area);
	let parts = Layout::default()
		.direction(Direction::Vertical)
		.constraints([Constraint::Min(3), Constraint::Length(strip)])
		.split(inner);

	let mut lines = vec![
		Line::from(Span::styled(format!(" {description}"), dim)),
		Line::raw(""),
	];
	let mut selected = None;
	let items = panel.items(&app.session);
	if items.is_empty() {
		lines.push(Line::from(Span::styled(
			match panel.view {
				View::Changed => " Nothing is staged or queued.",
				_ => {
					" This host does not declare any of this page's settings. / searches everything it does."
				}
			},
			dim,
		)));
	}
	let chosen = |position: usize| panel.focus == Focus::Content && panel.cursor == position;
	if let View::Page(_) = panel.view {
		let mut position = 0;
		for section in &panel.sections {
			lines.push(Line::from(Span::styled(
				format!(" {}", section.title),
				t.title_style(),
			)));
			for entry in &section.entries {
				if chosen(position) {
					selected = Some(lines.len());
				}
				lines.push(setting_line(
					app,
					&entry.path,
					entry.label,
					chosen(position),
				));
				position += 1;
			}
			lines.push(Line::raw(""));
		}
	} else {
		for (position, item) in items.iter().enumerate() {
			if let Item::Setting { path, label } = item {
				if chosen(position) {
					selected = Some(lines.len());
				}
				let label = label.map_or_else(|| option_path_display(path), str::to_string);
				lines.push(setting_line(app, path, &label, chosen(position)));
			}
		}
	}
	let height = parts[0].height as usize;
	let offset = selected.map_or(0, |line| (line + 2).saturating_sub(height));
	f.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), parts[0]);
	draw_strip(f, parts[1], app);
}

/// A paragraph's lines joined into one, up to the first blank line.
fn first_paragraph(text: &str) -> String {
	text.lines()
		.map(str::trim)
		.take_while(|line| !line.is_empty())
		.collect::<Vec<_>>()
		.join(" ")
}

/// `text` cut to `limit` characters, ending in an ellipsis when cut.
fn clip(text: &str, limit: usize) -> String {
	if text.chars().count() <= limit {
		return text.to_string();
	}
	let mut out: String = text.chars().take(limit.saturating_sub(1)).collect();
	out.push('…');
	out
}

/// The highlighted setting's path, type, documentation, and its keys; with
/// `d` it grows to show the full documentation, default, example, and
/// sources.
fn draw_strip(f: &mut Frame, area: Rect, app: &App) {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let panel = &app.nixos;
	let mut lines = vec![Line::from(Span::styled(
		"─".repeat(area.width as usize),
		dim,
	))];
	let Some(path) = panel.current_path(&app.session) else {
		f.render_widget(Paragraph::new(lines), area);
		return;
	};
	let Some(option) = panel.option(&path) else {
		f.render_widget(Paragraph::new(lines), area);
		return;
	};
	lines.push(Line::from(vec![
		Span::styled(format!(" {}", label_for(&path)), t.title_style()),
		Span::styled(format!("   {}", option_path_display(&path)), t.name_style()),
		Span::styled(format!("   {}", option.type_description), dim),
	]));
	let curated = curated(&path).map(|(_, _, entry)| entry.description);
	if let Some(text) = documentation(option, curated) {
		if panel.details_open {
			lines.extend(text.lines().map(|line| Line::raw(format!(" {line}"))));
		} else {
			// The first paragraph, with slack for word wrapping to fit two rows; `d` shows the rest.
			let width = (area.width as usize).saturating_sub(12) * 2;
			lines.push(Line::raw(format!(
				" {}",
				clip(&first_paragraph(&text), width)
			)));
		}
	}
	let editing = panel.editing(&path);
	if let Some(editing) = editing
		&& let Some(reason) = lock(app, editing)
	{
		lines.push(Line::from(Span::styled(
			format!(" {}", lock_note(editing, &reason)),
			t.version_style(),
		)));
	}
	if panel.details_open {
		if let Some(editing) = editing {
			lines.push(Line::raw(""));
			if let Some(text) = &editing.details.default {
				push_block(&mut lines, " default   ", text_lines(text), dim);
			}
			if let Some(text) = &editing.details.example {
				push_block(&mut lines, " example   ", text_lines(text), dim);
			}
			if let Some(files) = &editing.details.definitions {
				let files = if files.is_empty() {
					vec!["No definitions.".to_string()]
				} else {
					files.clone()
				};
				push_block(&mut lines, " defined   ", files, dim);
			}
		}
		push_block(&mut lines, " declared  ", option.declarations.clone(), dim);
	}
	lines.push(Line::from(Span::styled(
		" ↵ edit  space toggle  u unset  d details  / search",
		dim,
	)));
	f.render_widget(
		Paragraph::new(lines)
			.wrap(Wrap { trim: false })
			.scroll((panel.detail_scroll, 0)),
		area,
	);
}

/// The namespace tree beside the highlighted option's details.
fn draw_tree(f: &mut Frame, area: Rect, app: &App) {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let panel = &app.nixos;
	let columns = if area.width >= WIDE {
		Layout::default()
			.direction(Direction::Horizontal)
			.constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
			.split(area)
			.to_vec()
	} else {
		vec![area]
	};
	let title = if panel.category == nixbox_core::system_options::Category::Configured {
		format!("Configured options · {}", panel.visible.len())
	} else {
		format!("All options · {}", panel.visible.len())
	};
	let block = content_block(app, title);
	let width = (columns[0].width as usize).saturating_sub(4);
	let height = (columns[0].height as usize).saturating_sub(2).max(1);
	let start = panel
		.cursor
		.saturating_sub(height / 2)
		.min(panel.rows.len().saturating_sub(height));
	let lines: Vec<Line> = if panel.rows.is_empty() {
		vec![Line::from(Span::styled(
			" No options match the filters.",
			dim,
		))]
	} else {
		panel.rows[start..(start + height).min(panel.rows.len())]
			.iter()
			.enumerate()
			.map(|(offset, row)| {
				let chosen = panel.focus == Focus::Content && start + offset == panel.cursor;
				let indent = "  ".repeat(row.depth());
				let label = tree_label(&panel.options, row);
				let (glyph, right, style) = match row {
					TreeRow::Namespace {
						count, expanded, ..
					} => (
						if *expanded { "▾ " } else { "▸ " },
						count.to_string(),
						Style::default(),
					),
					TreeRow::Option { index, .. } => {
						let option = &panel.options[*index];
						let staged = panel.staged.contains_key(&option.path);
						let tags = option_tags(option)
							.into_iter()
							.filter(|tag| *tag != "diagnostic")
							.collect::<Vec<_>>()
							.join(" ");
						let right = if staged { "staged".to_string() } else { tags };
						(
							"  ",
							right,
							if option.read_only {
								dim
							} else {
								t.name_style()
							},
						)
					}
				};
				let name_width = width.saturating_sub(indent.len() + 2 + right.len() + 1);
				let text = format!("{indent}{glyph}{} {right}", pad(&label, name_width));
				if chosen {
					Line::from(Span::styled(format!(" {text}"), t.selection_style()))
				} else {
					Line::from(vec![Span::raw(" "), Span::styled(text, style)])
				}
			})
			.collect()
	};
	f.render_widget(Paragraph::new(lines).block(block), columns[0]);
	if let Some(details) = columns.get(1) {
		draw_option(f, *details, app);
	}
}

/// The highlighted option in full: type, description, value, and sources.
fn draw_option(f: &mut Frame, area: Rect, app: &App) {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let panel = &app.nixos;
	let block = titled_panel(t, Span::styled(" Option ", t.title_style()));
	let Some(option) = panel
		.current_path(&app.session)
		.and_then(|path| panel.option(&path))
	else {
		f.render_widget(
			Paragraph::new(Span::styled(
				" ←→ fold namespaces. Select an option to see it here.",
				dim,
			))
			.block(block)
			.wrap(Wrap { trim: false }),
			area,
		);
		return;
	};
	let tags = option_tags(option);
	let mut lines = vec![
		Line::from(Span::styled(
			option_path_display(&option.path),
			t.name_style(),
		)),
		Line::from(vec![
			Span::styled(option.type_description.clone(), t.version_style()),
			Span::styled(format!("  {}", tags.join(" · ")), dim),
		]),
	];
	let curated = curated(&option.path).map(|(_, _, entry)| entry.description);
	if let Some(text) = documentation(option, curated) {
		lines.push(Line::raw(""));
		lines.extend(text.lines().map(|line| Line::raw(line.to_string())));
	}
	if let Some(note) = partial_note(option) {
		lines.push(Line::from(Span::styled(note, dim)));
	}
	if let Some(reason) = panel.hardware.exclusion_reason(&option.path) {
		lines.push(Line::from(Span::styled(
			format!("{reason}. Retained by configuration or alt-a."),
			dim,
		)));
	}
	lines.push(Line::raw(""));
	lines.push(Line::from(Span::styled("Value", t.title_style())));
	if let Some(error) = &panel.detail_error {
		lines.push(Line::raw(error.clone()));
	}
	match panel.editing(&option.path) {
		Some(editing) => {
			push_block(
				&mut lines,
				"effective ",
				text_lines(&value_text(&editing.details.value)),
				dim,
			);
			if let Some(staged) = panel.staged.get(&option.path) {
				let text = staged
					.as_ref()
					.map_or_else(|| "unset".to_string(), SettingValue::to_nix);
				push_block(&mut lines, "staged    ", vec![text], dim);
			}
			if let Some(text) = &editing.details.default {
				push_block(&mut lines, "default   ", text_lines(text), dim);
			}
			if let Some(reason) = lock(app, editing) {
				push_block(&mut lines, "locked    ", vec![reason], dim);
			} else {
				let now = current(app, editing)
					.map_or_else(|| "unset".to_string(), |value| value.to_nix());
				push_block(&mut lines, "editing   ", vec![now], dim);
			}
			lines.push(Line::raw(""));
			lines.push(Line::from(Span::styled("Source", t.title_style())));
			if let Some(files) = &editing.details.definitions {
				let files = if files.is_empty() {
					vec!["No definitions.".to_string()]
				} else {
					files.clone()
				};
				push_block(&mut lines, "defined   ", files, dim);
			}
		}
		None if panel.inspecting => lines.push(Line::from(vec![
			Span::styled(
				format!("{} ", SPINNER[app.spinner_frame % SPINNER.len()]),
				t.version_style(),
			),
			Span::styled("Reading the value and definitions…", dim),
		])),
		None => {
			lines.push(Line::from(Span::styled(
				"↵ reads the value and definitions.",
				dim,
			)));
			lines.push(Line::raw(""));
			lines.push(Line::from(Span::styled("Source", t.title_style())));
		}
	}
	push_block(&mut lines, "declared  ", option.declarations.clone(), dim);
	if let Some((page, _, _)) = nixbox_core::settings_pages::curated(&option.path) {
		lines.push(Line::raw(""));
		lines.push(Line::from(vec![
			Span::styled("Shown on ", dim),
			Span::styled(page.label(), t.name_style()),
		]));
	}
	f.render_widget(
		Paragraph::new(lines)
			.block(block)
			.wrap(Wrap { trim: false })
			.scroll((panel.detail_scroll, 0)),
		area,
	);
}

/// Search results: curated settings, then options.
fn draw_results(f: &mut Frame, area: Rect, app: &App) {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let panel = &app.nixos;
	let results = &panel.search;
	let mut lines = Vec::new();
	let mut selected = None;
	if app.nixos_input.value().trim().is_empty() {
		lines.push(Line::from(Span::styled(
			" Type to search settings and option paths. ↵ opens, esc closes.",
			dim,
		)));
	} else if results.is_empty() {
		lines.push(Line::from(Span::styled(
			" Nothing matches this search.",
			dim,
		)));
	}
	if !results.settings.is_empty() {
		lines.push(Line::from(Span::styled(" Settings", t.title_style())));
		for (position, (page, shown)) in results.settings.iter().enumerate() {
			let chosen = panel.result == position;
			if chosen {
				selected = Some(lines.len());
			}
			let style = if chosen {
				t.selection_style()
			} else {
				Style::default()
			};
			lines.push(Line::from(vec![
				Span::styled(format!(" {}", pad(shown.label, LABEL_WIDTH)), style),
				Span::styled(format!("  {}", page.label()), dim),
			]));
		}
		lines.push(Line::raw(""));
	}
	if !results.options.is_empty() {
		lines.push(Line::from(Span::styled(
			format!(" Options · {}", results.total),
			t.title_style(),
		)));
		let offset = results.settings.len();
		for (position, index) in results.options.iter().enumerate() {
			let option = &panel.options[*index];
			let chosen = panel.result == offset + position;
			if chosen {
				selected = Some(lines.len());
			}
			let style = if chosen {
				t.selection_style()
			} else {
				t.name_style()
			};
			lines.push(Line::from(vec![
				Span::styled(
					format!(" {}", pad(&option_path_display(&option.path), 52)),
					style,
				),
				Span::styled(format!("  {}", option.type_description), dim),
			]));
		}
	}
	let title = format!("Search · {}", app.nixos_input.value());
	render_scrolled(f, area, lines, selected, content_block(app, title));
}

fn popup_area(area: Rect, width: u16, height: u16) -> Rect {
	let width = width.min(area.width.saturating_sub(4));
	let height = height.min(area.height.saturating_sub(2));
	Rect::new(
		area.x + (area.width - width) / 2,
		area.y + (area.height - height) / 2,
		width,
		height,
	)
}

fn draw_edit(f: &mut Frame, area: Rect, app: &App) {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let Some(Popup::Edit(edit)) = &app.nixos.popup else {
		return;
	};
	let height = if edit.choices.is_empty() {
		7
	} else {
		edit.choices.len() as u16 + 6
	};
	let rect = popup_area(area, 70, height);
	f.render_widget(Clear, rect);
	let title = format!(" Edit {} ", label_for(&edit.path));
	let mut lines = vec![
		Line::from(Span::styled(
			option_path_display(&edit.path),
			t.name_style(),
		)),
		Line::raw(""),
	];
	if edit.choices.is_empty() {
		lines.push(Line::from(vec![
			Span::styled("> ", t.title_style()),
			Span::raw(edit.text.clone()),
			Span::styled("█", t.title_style()),
		]));
	} else {
		for (position, choice) in edit.choices.iter().enumerate() {
			let text = match choice {
				SettingValue::Str(text) => text.clone(),
				value => value.to_nix(),
			};
			lines.push(Line::from(Span::styled(
				format!(
					" {} {text}",
					if position == edit.cursor { "❯" } else { " " }
				),
				if position == edit.cursor {
					t.selection_style()
				} else {
					Style::default()
				},
			)));
		}
	}
	lines.push(Line::raw(""));
	lines.push(Line::from(Span::styled(edit.hint, dim)));
	f.render_widget(
		Paragraph::new(lines)
			.block(titled_panel(t, Span::styled(title, t.title_style())))
			.wrap(Wrap { trim: false }),
		rect,
	);
}

fn draw_review(f: &mut Frame, area: Rect, app: &App) {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let panel = &app.nixos;
	let count = panel.staged.len();
	let rect = popup_area(area, 90, (count as u16) * 3 + count as u16 + 10);
	f.render_widget(Clear, rect);
	let mut lines = vec![
		Line::from(Span::styled(
			format!(
				"One rebuild writes these to {}.",
				app.session
					.engine
					.config
					.settings_file_for(nixbox_config::Target::NixosSystem)
					.display()
			),
			dim,
		)),
		Line::raw(""),
	];
	let mut module = vec!["{".to_string()];
	for (path, value) in &panel.staged {
		let before = panel
			.editing(path)
			.and_then(|option| baseline(app, option))
			.map_or_else(|| "unset".to_string(), |value| value.to_nix());
		let after = value
			.as_ref()
			.map_or_else(|| "unset".to_string(), SettingValue::to_nix);
		lines.push(Line::from(Span::styled(label_for(path), t.title_style())));
		lines.push(Line::from(Span::styled(
			format!("  {}", option_path_display(path)),
			t.name_style(),
		)));
		lines.push(Line::from(vec![
			Span::styled(format!("  {before}"), dim),
			Span::raw("  →  "),
			Span::styled(after.clone(), t.version_style()),
		]));
		module.push(match value {
			Some(_) => format!("  {} = {after};", option_path_display(path)),
			None => format!("  # {} is no longer set", option_path_display(path)),
		});
	}
	module.push("}".to_string());
	lines.push(Line::raw(""));
	lines.push(Line::from(Span::styled("Generated settings", dim)));
	lines.extend(module.into_iter().map(Line::raw));
	lines.push(Line::raw(""));
	lines.push(Line::from(Span::styled("↵ apply   esc back", dim)));
	f.render_widget(
		Paragraph::new(lines)
			.block(titled_panel(
				t,
				Span::styled(
					format!(
						" Review {count} change{} ",
						if count == 1 { "" } else { "s" }
					),
					t.title_style(),
				),
			))
			.wrap(Wrap { trim: false }),
		rect,
	);
}

fn text_lines(text: &str) -> Vec<String> {
	text.lines().map(str::to_string).collect()
}
