use nixbox_config::Target;
use nixbox_nix::scan::ScanTarget;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Wrap};

use super::titled_panel;
use crate::app::App;

const TAG_WIDTH: usize = 7; // "[nixos]"

pub(super) fn draw_installed_body(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let managed = app.filtered_managed_packages();
    let flakes = app.filtered_flakes();
    let external = app.filtered_external_packages();
    let filter = app.installed_filter();

    if managed.is_empty() && flakes.is_empty() && external.is_empty() {
        let block = titled_panel(t, Span::styled("Installed packages", t.title_style()));
        let body = if filter.is_some() {
            format!(
                "No installed packages match \"{}\".",
                app.installed_input.value()
            )
        } else {
            "No packages tracked yet.\n\nSwitch to the Search tab and press ↵ on a result to install.".to_string()
        };
        f.render_widget(
            Paragraph::new(body).block(block).wrap(Wrap { trim: false }),
            area,
        );
        return;
    }

    let dim = Style::default().add_modifier(Modifier::DIM);
    let hm_tag_style = Style::default().fg(Color::Cyan);
    let nixos_tag_style = Style::default().fg(Color::Magenta);

    let mut items: Vec<ListItem> = Vec::new();
    let mut pkg_to_row: Vec<usize> = Vec::new();

    // Managed section ----------------------------------------------------
    if !managed.is_empty() {
        let hm_count = managed
            .iter()
            .filter(|p| p.scope == Target::HomeManager)
            .count();
        let nx_count = managed.len() - hm_count;
        let header = format!(
            " Managed  ({} · {} hm, {} nixos) ",
            managed.len(),
            hm_count,
            nx_count,
        );
        items.push(ListItem::new(Line::from(Span::styled(header, dim))));
        for p in &managed {
            pkg_to_row.push(items.len());
            let (tag, tag_style) = scope_tag(p.scope);
            items.push(ListItem::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(pad_tag(&tag), tag_style),
                Span::raw("  "),
                Span::styled(p.name.clone(), t.name_style()),
            ])));
        }
    }

    // Flake section ------------------------------------------------------
    if !flakes.is_empty() {
        if !items.is_empty() {
            items.push(ListItem::new(Line::raw("")));
        }
        let managed_count = flakes.iter().filter(|flake| flake.is_managed()).count();
        let header = format!(
            " Flakes  ({} · {} managed, {} in your config) ",
            flakes.len(),
            managed_count,
            flakes.len() - managed_count,
        );
        items.push(ListItem::new(Line::from(Span::styled(header, dim))));
        for flake in &flakes {
            pkg_to_row.push(items.len());
            let (tag, tag_style) = scope_tag(flake.scope);
            let origin = match (&flake.repo, &flake.declared_in) {
                (Some(repo), None) => format!("(github:{repo})"),
                (Some(repo), Some(list)) => format!("({list} · github:{repo})"),
                (None, Some(list)) => format!("({list})"),
                (None, None) => String::new(),
            };
            let mut spans = vec![
                Span::raw("  "),
                Span::styled(pad_tag(&tag), tag_style),
                Span::raw("  "),
                Span::styled(flake.name(), t.name_style()),
                Span::raw("  "),
                Span::styled(origin, dim),
            ];
            if !flake.removable {
                spans.push(Span::styled("  · inline", dim));
            }
            items.push(ListItem::new(Line::from(spans)));
        }
    }

    // External section ---------------------------------------------------
    if !external.is_empty() {
        if !items.is_empty() {
            items.push(ListItem::new(Line::raw("")));
        }
        let hm_count = external
            .iter()
            .filter(|ep| matches!(ep.scope, ScanTarget::HomeManager))
            .count();
        let nx_count = external.len() - hm_count;
        let migratable_count = external.iter().filter(|ep| ep.migratable).count();
        let header = format!(
            " External  ({} · {} hm, {} nixos · {} migratable) ",
            external.len(),
            hm_count,
            nx_count,
            migratable_count,
        );
        items.push(ListItem::new(Line::from(vec![
            Span::styled(header, dim),
            Span::styled("  m migrate  M migrate all", dim),
        ])));
        for ep in &external {
            pkg_to_row.push(items.len());
            let scope = match ep.scope {
                ScanTarget::HomeManager => Target::HomeManager,
                ScanTarget::Nixos => Target::NixosSystem,
            };
            let (tag, tag_style) = scope_tag(scope);
            let tag_style = if ep.migratable {
                tag_style
            } else {
                tag_style.add_modifier(Modifier::DIM)
            };
            let name_style = if ep.migratable {
                t.name_style()
            } else {
                t.name_style().add_modifier(Modifier::DIM)
            };
            let mut spans = vec![
                Span::raw("  "),
                Span::styled(pad_tag(&tag), tag_style),
                Span::raw("  "),
                Span::styled(ep.name.clone(), name_style),
                Span::raw("  "),
                Span::styled(format!("({})", ep.source_attr), dim),
            ];
            if !ep.migratable {
                spans.push(Span::styled("  · inline", dim));
            }
            items.push(ListItem::new(Line::from(spans)));
        }
    }

    // Render --------------------------------------------------------------
    let title = Span::styled("Installed packages", t.title_style());
    let list = List::new(items)
        .block(titled_panel(t, title))
        .highlight_style(t.selection_style())
        .highlight_symbol("❯ ");

    let selected_row = if pkg_to_row.is_empty() {
        None
    } else {
        let clamped = app.installed_selected.min(pkg_to_row.len() - 1);
        Some(pkg_to_row[clamped])
    };

    let mut state = ListState::default();
    state.select(selected_row);
    let _ = hm_tag_style; // keep variables in scope for readability
    let _ = nixos_tag_style;
    f.render_stateful_widget(list, area, &mut state);
}

fn scope_tag(target: Target) -> (String, Style) {
    match target {
        Target::HomeManager => ("[hm]".to_string(), Style::default().fg(Color::Cyan)),
        Target::NixosSystem => ("[nixos]".to_string(), Style::default().fg(Color::Magenta)),
    }
}

fn pad_tag(tag: &str) -> String {
    if tag.chars().count() >= TAG_WIDTH {
        tag.to_string()
    } else {
        let pad = TAG_WIDTH - tag.chars().count();
        format!("{}{}", tag, " ".repeat(pad))
    }
}
