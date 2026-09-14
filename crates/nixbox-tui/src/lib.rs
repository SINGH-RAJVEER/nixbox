mod app;
mod handlers;
mod nav;
mod ops;
mod state;
mod theme;
mod ui;
mod vim;

pub use app::run;

/// Names of the themes the TUI ships with, in the order the settings screen
/// cycles them. Exposed so the CLI can validate `nixbox config set theme`.
#[must_use]
pub fn theme_names() -> Vec<&'static str> {
    theme::ALL.iter().map(|t| t.name).collect()
}
