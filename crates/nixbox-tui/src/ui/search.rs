use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Wrap};

use nixbox_config::Target;
use nixbox_nix::scan::ScanTarget;

use super::{SPINNER, titled_panel};
use crate::app::App;

struct HitStatus {
    managed_hm: bool,
    managed_nixos: bool,
    external_hm: bool,
    external_nixos: bool,
}

impl HitStatus {
    fn for_attr(app: &App, attr: &str) -> Self {
        let managed_hm = app.engine.home_manifest.packages.contains(attr);
        let managed_nixos = app.engine.nixos_manifest.packages.contains(attr);
        let external_hm = !managed_hm
            && app
                .engine
                .external_packages
                .iter()
                .any(|ep| ep.name == attr && ep.scope == ScanTarget::HomeManager);
        let external_nixos = !managed_nixos
            && app
                .engine
                .external_packages
                .iter()
                .any(|ep| ep.name == attr && ep.scope == ScanTarget::Nixos);
        Self {
            managed_hm,
            managed_nixos,
            external_hm,
            external_nixos,
        }
    }

    fn any_installed(&self) -> bool {
        self.managed_hm || self.managed_nixos || self.external_hm || self.external_nixos
    }

    fn installed_in(&self, target: Target) -> bool {
        match target {
            Target::HomeManager => self.managed_hm || self.external_hm,
            Target::NixosSystem => self.managed_nixos || self.external_nixos,
        }
    }
}

pub(super) fn draw_search_body(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    let results_block = titled_panel(t, Span::styled("Results", t.title_style()));

    let no_results = !app.searching && !app.latest_query.is_empty() && app.results.is_empty();

    if app.searching && app.results.is_empty() {
        let frame = SPINNER[app.spinner_frame % SPINNER.len()];
        let inner_h = split[0].height.saturating_sub(2) as usize;
        let pad = inner_h / 2;
        let mut lines: Vec<Line> = (0..pad).map(|_| Line::raw("")).collect();
        lines.push(Line::from(vec![
            Span::styled(format!("  {}  ", frame), t.version_style()),
            Span::styled(
                "Searching nixpkgs…",
                Style::default().add_modifier(Modifier::DIM),
            ),
        ]));
        f.render_widget(Paragraph::new(lines).block(results_block), split[0]);
    } else if no_results {
        let dim = Style::default().add_modifier(Modifier::DIM);
        let inner_h = split[0].height.saturating_sub(2) as usize;
        let pad = inner_h / 2;
        let mut lines: Vec<Line> = (0..pad).map(|_| Line::raw("")).collect();
        lines.push(Line::from(vec![
            Span::styled("No results for ", dim),
            Span::styled(format!("\"{}\"", app.latest_query), t.name_style()),
        ]));
        f.render_widget(Paragraph::new(lines).block(results_block), split[0]);
    } else {
        let items: Vec<ListItem> = app
            .results
            .iter()
            .map(|hit| {
                let status = HitStatus::for_attr(app, &hit.attr);
                let mut spans = vec![
                    Span::styled(hit.pname.clone(), t.name_style()),
                    Span::raw("  "),
                    Span::styled(hit.version.clone(), t.version_style()),
                ];
                if status.any_installed() {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled("✓", Style::default().fg(Color::Green)));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();

        let list = List::new(items)
            .block(results_block)
            .highlight_style(t.selection_style())
            .highlight_symbol("❯ ");

        let mut state = ListState::default();
        if !app.results.is_empty() {
            state.select(Some(app.selected));
        }
        f.render_stateful_widget(list, split[0], &mut state);
    }

    let detail_block = titled_panel(t, Span::styled("Details", t.title_style()));
    let dim = Style::default().add_modifier(Modifier::DIM);

    if let Some(hit) = app.results.get(app.selected) {
        let desc = if hit.description.is_empty() {
            "(no description)".to_string()
        } else {
            hit.description.clone()
        };
        let status = HitStatus::for_attr(app, &hit.attr);
        let green = Style::default().fg(Color::Green);

        let mut lines: Vec<Line> = vec![
            Line::from(Span::styled(hit.pname.clone(), t.name_style())),
            Line::from(Span::styled(
                format!("version {}", hit.version),
                t.version_style(),
            )),
            Line::raw(""),
            Line::from(vec![
                Span::styled("ABOUT", t.title_style()),
                Span::styled("  ", dim),
                Span::raw(desc),
            ]),
            Line::raw(""),
            Line::from(Span::styled("PACKAGE", t.title_style())),
            Line::from(vec![
                Span::styled("attribute  ", dim),
                Span::raw(hit.attr.clone()),
            ]),
        ];

        if status.any_installed() {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled("INSTALLED", t.title_style())));
            for (installed, managed, scope_label) in [
                (status.managed_hm, true, "home-manager"),
                (status.managed_nixos, true, "nixos"),
                (status.external_hm, false, "home-manager"),
                (status.external_nixos, false, "nixos"),
            ] {
                if installed {
                    let kind = if managed {
                        "nixbox-managed"
                    } else {
                        "in config"
                    };
                    lines.push(Line::from(vec![
                        Span::styled("✓ ", green),
                        Span::styled(kind, green),
                        Span::styled("  ·  ", dim),
                        Span::styled(scope_label, t.title_style()),
                    ]));
                }
            }
        }

        lines.push(Line::raw(""));
        let hint = if status.installed_in(app.engine.config.target) {
            format!("↵  installed for {}", app.target_label())
        } else {
            format!("↵  install for {}", app.target_label())
        };
        lines.push(Line::from(Span::styled(hint, dim)));

        f.render_widget(
            Paragraph::new(lines)
                .block(detail_block)
                .wrap(Wrap { trim: false }),
            split[1],
        );
    } else {
        let lines: Vec<Line> = vec![
            Line::from(Span::styled("nixpkgs search", t.title_style())),
            Line::raw(""),
            Line::from(Span::styled("/  type to search", dim)),
            Line::from(Span::styled("↑↓ j/k  navigate", dim)),
            Line::from(Span::styled("↵   install", dim)),
            Line::from(Span::styled("tab  switch tabs", dim)),
        ];
        f.render_widget(
            Paragraph::new(lines)
                .block(detail_block)
                .wrap(Wrap { trim: false }),
            split[1],
        );
    }
}
