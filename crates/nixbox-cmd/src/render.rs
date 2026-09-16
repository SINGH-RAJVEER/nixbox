//! Plain-text output helpers.
//!
//! Structured data goes to stdout, progress and notes go to stderr, so
//! `nixbox list | …` stays useful while a rebuild is streaming.

use std::fmt::Write as _;

/// Renders a left-aligned table with a header row. The last column is never
/// padded, so trailing whitespace never ends up in piped output.
#[must_use]
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row.iter()) {
            *width = (*width).max(cell.chars().count());
        }
    }

    let mut out = String::new();
    push_row(&mut out, headers.iter().map(|h| (*h).to_string()), &widths);
    for row in rows {
        push_row(&mut out, row.iter().cloned(), &widths);
    }
    out
}

fn push_row(out: &mut String, cells: impl Iterator<Item = String>, widths: &[usize]) {
    let cells: Vec<String> = cells.collect();
    let last = cells.len().saturating_sub(1);
    for (index, cell) in cells.iter().enumerate() {
        if index == last {
            out.push_str(cell);
        } else {
            let width = widths.get(index).copied().unwrap_or(0);
            let pad = width.saturating_sub(cell.chars().count());
            out.push_str(cell);
            for _ in 0..pad {
                out.push(' ');
            }
            out.push_str("  ");
        }
    }
    out.push('\n');
}

/// Renders aligned `key: value` lines.
#[must_use]
pub fn fields(rows: &[(&str, String)]) -> String {
    let width = rows
        .iter()
        .map(|(key, _)| key.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for (key, value) in rows {
        let pad = width.saturating_sub(key.chars().count());
        let _ = write!(out, "{key}");
        for _ in 0..pad {
            out.push(' ');
        }
        let _ = writeln!(out, "  {value}");
    }
    out
}

/// Shortens `text` to `max` characters, marking anything dropped.
#[must_use]
pub fn truncate(text: &str, max: usize) -> String {
    let flat = text.replace('\n', " ");
    if flat.chars().count() <= max {
        return flat;
    }
    let keep = max.saturating_sub(3);
    let mut out: String = flat.chars().take(keep).collect();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::{fields, table, truncate};

    #[test]
    fn table_pads_every_column_but_the_last() {
        let rendered = table(
            &["NAME", "VERSION"],
            &[
                vec!["ripgrep".into(), "14.1.1".into()],
                vec!["fd".into(), "10.2.0".into()],
            ],
        );

        assert_eq!(
            rendered,
            "NAME     VERSION\nripgrep  14.1.1\nfd       10.2.0\n"
        );
    }

    #[test]
    fn table_with_no_rows_still_prints_headers() {
        assert_eq!(table(&["NAME"], &[]), "NAME\n");
    }

    #[test]
    fn fields_align_on_the_longest_key() {
        assert_eq!(
            fields(&[("target", "nixos".into()), ("channel", "x".into())]),
            "target   nixos\nchannel  x\n"
        );
    }

    #[test]
    fn truncate_marks_what_it_dropped_and_flattens_newlines() {
        assert_eq!(truncate("hello world", 8), "hello...");
        assert_eq!(truncate("short", 8), "short");
        assert_eq!(truncate("two\nlines", 20), "two lines");
    }
}
