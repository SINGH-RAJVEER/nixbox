use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Wrap};
use serde_json::Value;

use super::{SPINNER, titled_panel};
use crate::app::App;
use crate::options::{OptionEditor, OptionsPanel};
use nixbox_core::options::{format_value, locked_reason, plain_markdown};
use nixbox_nix::options::OptionKind;
use nixbox_nix::settings::SettingValue;

/// Widest option name before the value column starts.
const NAME_WIDTH: usize = 34;
/// Lines of a long value shown in the details pane.
const VALUE_LINES: usize = 12;

pub(super) fn draw_options_body(f: &mut Frame, area: Rect, app: &App) {
	let Some(panel) = &app.options_panel else {
		return;
	};
	let split = Layout::default()
		.direction(Direction::Horizontal)
		.constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
		.split(area);
	draw_list(f, split[0], app, panel);

	let editor_height = match &panel.editor {
		Some(OptionEditor::Text { .. }) => 4,
		Some(OptionEditor::Choice { values, .. }) => values.len().min(10) as u16 + 2,
		None => 0,
	};
	if editor_height == 0 {
		draw_details(f, split[1], app, panel);
		return;
	}
	let right = Layout::default()
		.direction(Direction::Vertical)
		.constraints([Constraint::Min(4), Constraint::Length(editor_height)])
		.split(split[1]);
	draw_details(f, right[0], app, panel);
	draw_editor(f, right[1], app, panel);
}

fn title(panel: &OptionsPanel) -> String {
	let namespaces: Vec<String> = panel
		.set
		.as_ref()
		.map(|set| set.namespaces.iter().map(|path| path.join(".")).collect())
		.unwrap_or_default();
	let mut title = format!("{} [{}]", panel.package, panel.scope.tag());
	if !namespaces.is_empty() {
		title = format!("{}  ·  {title}", namespaces.join(", "));
	}
	if !panel.staged.is_empty() {
		title.push_str(&format!("  ·  {} staged", panel.staged.len()));
	}
	title
}

fn draw_list(f: &mut Frame, area: Rect, app: &App, panel: &OptionsPanel) {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let block = titled_panel(t, Span::styled(title(panel), t.title_style()));

	if panel.loading {
		let spinner = SPINNER[app.spinner_frame % SPINNER.len()];
		f.render_widget(
			Paragraph::new(Line::from(vec![
				Span::styled(format!(" {spinner} "), t.version_style()),
				Span::styled("Evaluating your configuration's options…", dim),
			]))
			.block(block),
			area,
		);
		return;
	}
	if let Some(error) = &panel.error {
		let lines = vec![
			Line::from(Span::styled("Could not read options.", t.title_style())),
			Line::raw(""),
			Line::raw(error.clone()),
			Line::raw(""),
			Line::from(Span::styled("r / ctrl-r retry    esc close", dim)),
		];
		f.render_widget(
			Paragraph::new(lines)
				.block(block)
				.wrap(Wrap { trim: false }),
			area,
		);
		return;
	}
	let visible = panel.visible();
	if visible.is_empty() {
		let message = if panel
			.set
			.as_ref()
			.is_some_and(|set| set.namespaces.is_empty())
		{
			format!(
				"No module options found for {}.\n\nOnly packages with a programs.* or services.* module in your {} configuration have options to set.",
				panel.package,
				panel.scope.label()
			)
		} else {
			format!("No options match \"{}\".", panel.filter.value())
		};
		f.render_widget(
			Paragraph::new(message)
				.block(block)
				.wrap(Wrap { trim: false }),
			area,
		);
		return;
	}

	let items: Vec<ListItem> = visible
		.iter()
		.map(|entry| {
			let name = panel.display_name(entry);
			let locked = locked_reason(entry);
			let staged = panel.staged.get(&entry.path);
			let queued = app.session.queued_option(panel.scope, &entry.path);
			let (marker, value) = match (staged, &queued) {
				(Some(value), _) => ("* ", setting_text(value.as_ref())),
				(None, Some(value)) => ("~ ", setting_text(value.as_ref())),
				(None, None) => ("  ", format_value(entry.value.as_ref())),
			};
			let name_style = if locked.is_some() {
				t.name_style().add_modifier(Modifier::DIM)
			} else {
				t.name_style()
			};
			let value_style = if staged.is_some() || queued.is_some() {
				t.title_style()
			} else {
				dim
			};
			let mut spans = vec![
				Span::styled(marker, t.title_style()),
				Span::styled(pad(&name, NAME_WIDTH), name_style),
				Span::raw("  "),
				Span::styled(value, value_style),
			];
			if let Some(reason) = locked {
				spans.push(Span::styled(format!("  · {reason}"), dim));
			} else if app.session.engine.sets_option(panel.scope, entry) {
				spans.push(Span::styled("  · nixbox", dim));
			}
			ListItem::new(Line::from(spans))
		})
		.collect();
	let list = List::new(items)
		.block(block)
		.highlight_style(t.selection_style())
		.highlight_symbol("❯ ");
	let mut state = ListState::default();
	state.select(Some(panel.selected.min(visible.len() - 1)));
	f.render_stateful_widget(list, area, &mut state);
}

fn draw_details(f: &mut Frame, area: Rect, app: &App, panel: &OptionsPanel) {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let block = titled_panel(t, Span::styled("Option", t.title_style()));
	let Some(entry) = panel.current() else {
		let lines = vec![
			Line::from(Span::styled("Package options", t.title_style())),
			Line::raw(""),
			Line::from(Span::styled(
				"Options come from the programs.* and services.* modules your configuration evaluates.",
				dim,
			)),
			Line::from(Span::styled(
				"Changes are written to nixbox's settings module and applied with one rebuild.",
				dim,
			)),
		];
		f.render_widget(
			Paragraph::new(lines)
				.block(block)
				.wrap(Wrap { trim: false }),
			area,
		);
		return;
	};

	let mut lines = vec![
		Line::from(Span::styled(entry.path.join("."), t.name_style())),
		Line::from(Span::styled(
			entry.type_description.clone(),
			t.version_style(),
		)),
		Line::raw(""),
	];
	push_block(
		&mut lines,
		"value    ",
		value_lines(entry.value.as_ref()),
		dim,
	);
	if let Some(staged) = panel.staged.get(&entry.path) {
		push_block(
			&mut lines,
			"staged   ",
			vec![setting_text(staged.as_ref())],
			dim,
		);
	}
	if let Some(queued) = app.session.queued_option(panel.scope, &entry.path) {
		push_block(
			&mut lines,
			"queued   ",
			vec![setting_text(queued.as_ref())],
			dim,
		);
	}
	if let Some(default) = &entry.default {
		push_block(&mut lines, "default  ", text_lines(default), dim);
	}
	if let Some(example) = &entry.example {
		push_block(&mut lines, "example  ", text_lines(example), dim);
	}
	let status = match locked_reason(entry) {
		Some(reason) => format!("locked, {reason}"),
		None if app.session.engine.sets_option(panel.scope, entry) => {
			"set by nixbox, editable".into()
		}
		None => "editable".into(),
	};
	push_block(&mut lines, "status   ", vec![status], dim);
	if let Some(description) = &entry.description {
		lines.push(Line::raw(""));
		for line in description.lines() {
			lines.push(Line::raw(plain_markdown(line)));
		}
	}
	f.render_widget(
		Paragraph::new(lines)
			.block(block)
			.wrap(Wrap { trim: false }),
		area,
	);
}

fn draw_editor(f: &mut Frame, area: Rect, app: &App, panel: &OptionsPanel) {
	let t = app.theme();
	let dim = Style::default().add_modifier(Modifier::DIM);
	let Some(editor) = &panel.editor else {
		return;
	};
	match editor {
		OptionEditor::Text { path, input } => {
			let kind = panel
				.set
				.as_ref()
				.and_then(|set| set.entries.iter().find(|entry| &entry.path == path))
				.map(|entry| &entry.kind);
			let block = titled_panel(
				t,
				Span::styled(format!("Edit {}", path.join(".")), t.title_style()),
			);
			let inner = block.inner(area);
			f.render_widget(block, area);
			let rows = Layout::default()
				.direction(Direction::Vertical)
				.constraints([Constraint::Length(1), Constraint::Length(1)])
				.split(inner);
			let width = rows[0].width as usize;
			let scroll = input.visual_scroll(width.saturating_sub(1));
			f.render_widget(
				Paragraph::new(input.value().to_string()).scroll((0, scroll as u16)),
				rows[0],
			);
			f.render_widget(
				Paragraph::new(Span::styled(editor_hint(kind), dim)),
				rows[1],
			);
			let cursor_x = rows[0]
				.x
				.saturating_add(input.visual_cursor().saturating_sub(scroll) as u16)
				.min(rows[0].right().saturating_sub(1));
			f.set_cursor_position((cursor_x, rows[0].y));
		}
		OptionEditor::Choice {
			path,
			values,
			cursor,
		} => {
			let block = titled_panel(
				t,
				Span::styled(format!("Set {}", path.join(".")), t.title_style()),
			);
			let items: Vec<ListItem> = values
				.iter()
				.map(|value| {
					ListItem::new(Line::from(Span::styled(value.to_nix(), t.name_style())))
				})
				.collect();
			let list = List::new(items)
				.block(block)
				.highlight_style(t.selection_style())
				.highlight_symbol("❯ ");
			let mut state = ListState::default();
			state.select(Some(*cursor));
			f.render_stateful_widget(list, area, &mut state);
		}
	}
}

fn editor_hint(kind: Option<&OptionKind>) -> &'static str {
	let nullable = matches!(kind, Some(OptionKind::Nullable(_)));
	match kind.map(OptionKind::inner) {
		Some(OptionKind::StrList) => "comma-separated; empty for [ ]",
		Some(OptionKind::Int | OptionKind::Float) if nullable => "a number; empty for null",
		Some(OptionKind::Int | OptionKind::Float) => "a number",
		_ if nullable => "empty for null",
		_ => "text is used as written",
	}
}

fn setting_text(value: Option<&SettingValue>) -> String {
	match value {
		Some(value) => value.to_nix(),
		None => "unset".into(),
	}
}

fn value_lines(value: Option<&Value>) -> Vec<String> {
	match value {
		Some(Value::String(text)) if text.contains('\n') => text_lines(text),
		Some(value @ (Value::Array(_) | Value::Object(_))) if !value_is_marker(value) => {
			let pretty = serde_json::to_string_pretty(value).unwrap_or_default();
			text_lines(&pretty)
		}
		other => vec![format_value(other)],
	}
}

fn value_is_marker(value: &Value) -> bool {
	value.get("_nixbox").is_some()
}

fn text_lines(text: &str) -> Vec<String> {
	let mut lines: Vec<String> = text.lines().take(VALUE_LINES).map(str::to_string).collect();
	let total = text.lines().count();
	if total > VALUE_LINES {
		lines.push(format!("… {} more lines", total - VALUE_LINES));
	}
	if lines.is_empty() {
		lines.push(String::new());
	}
	lines
}

fn push_block(lines: &mut Vec<Line<'static>>, label: &'static str, body: Vec<String>, dim: Style) {
	for (index, text) in body.into_iter().enumerate() {
		let prefix = if index == 0 { label } else { "         " };
		lines.push(Line::from(vec![Span::styled(prefix, dim), Span::raw(text)]));
	}
}

fn pad(text: &str, width: usize) -> String {
	let count = text.chars().count();
	if count >= width {
		let mut cut: String = text.chars().take(width.saturating_sub(1)).collect();
		cut.push('…');
		cut
	} else {
		format!("{text}{}", " ".repeat(width - count))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn long_names_are_cut_to_the_column() {
		assert_eq!(pad("abc", 5), "abc  ");
		assert_eq!(pad("abcdef", 5), "abcd…");
	}
}
