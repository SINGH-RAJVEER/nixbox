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
	let popup_width: u16 = if matches!(
		app.settings_page,
		SettingsPage::Telemetry | SettingsPage::TelemetryPreview
	) {
		76
	} else {
		42
	};
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
					app.session.engine.config.input_mode.label()
				),
				format!("  {:<12} {}  ›", "Theme", theme::ALL[app.theme_index].name),
				format!(
					"  {:<12} {}  ›",
					"Target",
					app.session.engine.config.target.label()
				),
				format!(
					"  {:<12} {}  ›",
					"Channel", app.session.engine.config.channel
				),
				format!(
					"  {:<12} {}  >",
					"Usage data",
					if app.session.engine.config.telemetry.enabled {
						"On"
					} else {
						"Off"
					}
				),
				format!(
					"  {:<12} {}  >",
					"Configuration",
					app.session
						.engine
						.config
						.nixos_host
						.as_deref()
						.or(app.session.engine.config.home_configuration.as_deref())
						.unwrap_or("Select output")
				),
			],
			selected,
		),
		SettingsPage::InputMode => (
			" Input Mode ",
			INPUT_MODES
				.iter()
				.map(|mode| {
					option_label(mode.label(), *mode == app.session.engine.config.input_mode)
				})
				.collect(),
			selected,
		),
		SettingsPage::Configurations => (
			" Configuration ",
			if app.configuration_choices.is_empty() {
				vec!["No supported configuration outputs discovered".into()]
			} else {
				app.configuration_choices
					.iter()
					.map(|choice| {
						option_label(&choice.label, choice.selected(&app.session.engine.config))
					})
					.collect()
			},
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
				.map(|target| {
					option_label(target.label(), *target == app.session.engine.config.target)
				})
				.collect(),
			selected,
		),
		SettingsPage::Channel => (
			" Channel ",
			CHANNELS
				.iter()
				.map(|channel| option_label(channel, *channel == app.session.engine.config.channel))
				.collect(),
			selected,
		),
		SettingsPage::Telemetry => {
			use nixbox_core::telemetry;
			let consent = app.session.engine.config.telemetry;
			let mut labels = vec![
				option_label("Off", !consent.enabled),
				option_label(
					if telemetry::RECIPIENT.is_some() {
						"On"
					} else {
						"On unavailable: receiver not configured"
					},
					consent.enabled,
				),
				"  View synthetic example report".into(),
				String::new(),
			];
			labels.extend(wrap(telemetry::DISCLOSURE, 68));
			labels.extend(wrap(
				&format!(
					"Status: {}. Recipient: {}.",
					telemetry::status_label(consent),
					telemetry::RECIPIENT.unwrap_or("not configured")
				),
				68,
			));
			(" Share basic usage data ", labels, selected)
		}
		SettingsPage::TelemetryPreview => (
			" Synthetic example, nothing sent ",
			nixbox_core::telemetry::preview()
				.unwrap_or_default()
				.lines()
				.map(str::to_owned)
				.collect(),
			selected,
		),
	}
}

fn wrap(text: &str, width: usize) -> Vec<String> {
	let mut lines = Vec::new();
	let mut line = String::new();
	for word in text.split_whitespace() {
		if !line.is_empty() && line.len() + 1 + word.len() > width {
			lines.push(std::mem::take(&mut line));
		}
		if !line.is_empty() {
			line.push(' ');
		}
		line.push_str(word);
	}
	if !line.is_empty() {
		lines.push(line);
	}
	lines
}

fn option_label(label: &str, selected: bool) -> String {
	format!("  {}{}", label, if selected { "  ✓" } else { "" })
}
