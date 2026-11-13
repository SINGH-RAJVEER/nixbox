//! VCS tab. Follows the GUI's version control page: the repository header and
//! latest result, then the changed files and their diff beside the commit,
//! push and remote controls. Every action ends with a fresh review, so the
//! diff on screen is what a commit records. A control that cannot run says
//! why next to its key instead of failing when pressed.

use std::ops::Range;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nixbox_core::vcs::{Backend, Repository, Review, Vcs, Visibility};
use ratatui::{
	Frame,
	layout::{Constraint, Layout, Rect},
	style::{Color, Modifier, Style},
	text::{Line, Span},
	widgets::{Block, Borders, Paragraph, Wrap},
};
use tokio::sync::mpsc;

use crate::app::{App, AppEvent};

const PENDING: &str = "Waiting for queued changes and rebuilds to finish.";
const ACTIONS_WIDTH: u16 = 40;
const MESSAGE_ROWS: usize = 6;

/// A single-line or multiline text field with a byte cursor.
#[derive(Default)]
struct Editor {
	text: String,
	cursor: usize,
}

impl Editor {
	fn set(&mut self, text: impl Into<String>) {
		self.text = text.into();
		self.cursor = self.text.len();
	}

	fn edit(&mut self, key: KeyEvent, multiline: bool) {
		let previous = self.text[..self.cursor]
			.char_indices()
			.next_back()
			.map_or(0, |(index, _)| index);
		let next = self.text[self.cursor..]
			.chars()
			.next()
			.map_or(self.cursor, |ch| self.cursor + ch.len_utf8());
		match key.code {
			KeyCode::Left => self.cursor = previous,
			KeyCode::Right => self.cursor = next,
			KeyCode::Home => self.cursor = 0,
			KeyCode::End => self.cursor = self.text.len(),
			KeyCode::Backspace if self.cursor > 0 => {
				self.text.replace_range(previous..self.cursor, "");
				self.cursor = previous;
			}
			KeyCode::Delete => self.text.replace_range(self.cursor..next, ""),
			KeyCode::Enter if multiline => {
				self.text.insert(self.cursor, '\n');
				self.cursor += 1;
			}
			KeyCode::Char(ch)
				if !key
					.modifiers
					.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
			{
				self.text.insert(self.cursor, ch);
				self.cursor += ch.len_utf8();
			}
			_ => {}
		}
	}

	/// The text with a bar at the cursor, scrolled so the cursor stays in
	/// `rows` lines of `width` columns.
	fn lines(&self, rows: usize, width: usize) -> Vec<String> {
		let prefix = &self.text[..self.cursor];
		let row = prefix.matches('\n').count();
		let column = prefix
			.rsplit('\n')
			.next()
			.unwrap_or_default()
			.chars()
			.count();
		let first = row.saturating_sub(rows.saturating_sub(1));
		let left = column.saturating_sub(width.saturating_sub(1));
		let mut text = self.text.clone();
		text.insert(self.cursor, '|');
		text.split('\n')
			.skip(first)
			.take(rows)
			.map(|line| line.chars().skip(left).collect())
			.collect()
	}
}

#[derive(Default)]
pub(crate) struct Panel {
	loaded: bool,
	pub busy: bool,
	repository: Option<Repository>,
	origin: Option<String>,
	review: Option<Review>,
	diff: Diff,
	/// 0 shows every file's diff; `n` shows file `n - 1`.
	selected: usize,
	scroll: usize,
	horizontal: usize,
	show_status: bool,
	form: Option<Form>,
	message: Editor,
	/// The suggestion the message follows until the user edits it.
	generated: String,
	branch: Editor,
	remote: Editor,
	public: bool,
	create_bookmark: bool,
	confirmation: Option<Action>,
	notice: String,
	failed: bool,
}

impl Panel {
	fn jj(&self) -> bool {
		self.repository
			.as_ref()
			.is_some_and(|repo| repo.backend == Backend::Jj)
	}

	/// The configuration is a subdirectory of a larger repository.
	fn parent(&self) -> bool {
		self.repository
			.as_ref()
			.is_some_and(|repo| repo.root != repo.config_root)
	}

	fn follows_suggestion(&self) -> bool {
		self.message.text == self.generated || self.message.text.trim().is_empty()
	}

	fn set_review(&mut self, review: Option<Review>) {
		let diff = review
			.as_ref()
			.map_or_else(Diff::default, |review| Diff::parse(review.diff()));
		// Stay on the selected file while it still has changes.
		self.selected = self
			.selected
			.checked_sub(1)
			.and_then(|index| self.diff.files.get(index))
			.and_then(|file| diff.files.iter().position(|new| new.path == file.path))
			.map_or(0, |index| index + 1);
		self.diff = diff;
		self.review = review;
		self.scroll = 0;
		self.horizontal = 0;
	}

	fn select(&mut self, selected: usize) {
		self.selected = selected.min(self.diff.files.len());
		self.scroll = 0;
		self.horizontal = 0;
	}

	/// The diff lines of the selected file, or of every file.
	fn shown(&self) -> Range<usize> {
		self.selected
			.checked_sub(1)
			.and_then(|index| self.diff.files.get(index))
			.map_or(0..self.diff.lines.len(), |file| file.lines.clone())
	}
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Form {
	Message,
	Push,
	Remote,
}

#[derive(Clone)]
enum Action {
	Refresh,
	Init(Backend),
	Commit(Review),
	Push(String, bool),
	Remote(String, Visibility),
}

/// The action's result and a fresh look at the repository afterwards, kept
/// apart so an action's success stays visible even if the refresh fails.
#[derive(Debug, Clone)]
pub(crate) struct Outcome {
	notice: Result<String, String>,
	committed: bool,
	snapshot: Result<Snapshot, String>,
}

#[derive(Debug, Clone)]
struct Snapshot {
	repository: Option<Repository>,
	origin: Option<String>,
	suggestion: String,
	review: Result<Option<Review>, String>,
}

#[cfg(test)]
impl Outcome {
	/// A refresh that found no repository.
	pub(crate) fn nothing() -> Self {
		Self {
			notice: Ok(String::new()),
			committed: false,
			snapshot: Ok(Snapshot {
				repository: None,
				origin: None,
				suggestion: String::new(),
				review: Ok(None),
			}),
		}
	}
}

pub(crate) fn backend_name(backend: Backend) -> &'static str {
	match backend {
		Backend::Git => "Git",
		Backend::Jj => "Jujutsu",
	}
}

fn pending_writes(app: &App) -> bool {
	app.session.is_building() || !app.session.queue.is_empty()
}

pub(crate) fn ensure_loaded(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
	if !app.repository.loaded && !app.repository.busy {
		start(app, tx, Action::Refresh);
	}
}

fn start(app: &mut App, tx: &mpsc::Sender<AppEvent>, action: Action) {
	let pending = pending_writes(app);
	let panel = &mut app.repository;
	panel.confirmation = None;
	if panel.busy {
		return;
	}
	if pending {
		panel.set_review(None);
		panel.notice = PENDING.into();
		panel.failed = false;
		return;
	}
	let committing = matches!(action, Action::Commit(_));
	// After a commit the next message describes new work, so it follows the
	// suggestion again.
	let message = (!committing && !panel.follows_suggestion()).then(|| panel.message.text.clone());
	panel.busy = true;
	panel.loaded = true;
	panel.failed = false;
	panel.notice = match &action {
		Action::Refresh => "Reviewing the configuration's changes...",
		Action::Init(_) => "Initializing the repository...",
		Action::Commit(_) => "Committing...",
		Action::Push(..) => "Pushing to origin...",
		Action::Remote(..) => "Creating the GitHub repository...",
	}
	.into();
	let config = app.session.engine.config.clone();
	let tx = tx.clone();
	tokio::spawn(async move {
		let result = tokio::task::spawn_blocking(move || {
			let vcs = Vcs::new(&config).map_err(|error| format!("{error:#}"))?;
			let notice = run(&vcs, action).map_err(|error| format!("{error:#}"));
			let committed = committing && notice.is_ok();
			let snapshot = snapshot(&vcs, message.as_deref()).map_err(|error| format!("{error:#}"));
			Ok(Outcome {
				notice,
				committed,
				snapshot,
			})
		})
		.await
		.unwrap_or_else(|error| Err(format!("repository worker failed: {error}")));
		let _ = tx.send(AppEvent::Repository(result)).await;
	});
}

fn run(vcs: &Vcs, action: Action) -> anyhow::Result<String> {
	Ok(match action {
		Action::Refresh => "Review refreshed. Commit and push are separate steps.".into(),
		Action::Init(backend) => {
			vcs.init(backend)?;
			format!(
				"Initialized {}. Nothing committed or pushed.",
				backend_name(backend)
			)
		}
		Action::Commit(review) => {
			let outcome = vcs.commit(&review)?;
			let mut notice = format!("Committed {}. Nothing pushed.", outcome.revision);
			if let Some(warning) = outcome.warning {
				notice.push(' ');
				notice.push_str(&warning);
			}
			notice
		}
		Action::Push(name, create) => {
			vcs.push(&name, create)?;
			format!("Pushed {name} to origin.")
		}
		Action::Remote(name, visibility) => {
			vcs.create_remote(&name, visibility)?;
			format!("Created {name} and added it as origin. Nothing pushed.")
		}
	})
}

fn snapshot(vcs: &Vcs, message: Option<&str>) -> anyhow::Result<Snapshot> {
	let repository = vcs.detect()?;
	let suggestion = vcs.suggested_message()?;
	let (origin, review) = if repository.is_some() {
		(
			vcs.origin()?,
			vcs.review(message)
				.map(Some)
				.map_err(|error| format!("{error:#}")),
		)
	} else {
		(None, Ok(None))
	};
	Ok(Snapshot {
		repository,
		origin,
		suggestion,
		review,
	})
}

pub(crate) fn on_result(app: &mut App, result: Result<Outcome, String>) {
	let panel = &mut app.repository;
	panel.busy = false;
	panel.loaded = true;
	let outcome = match result {
		Ok(outcome) => outcome,
		Err(error) => {
			panel.notice = format!("Repository operation failed: {error}");
			panel.failed = true;
			panel.set_review(None);
			return;
		}
	};
	let succeeded = outcome.notice.is_ok();
	match outcome.notice {
		Ok(notice) => panel.notice = notice,
		Err(error) => {
			panel.notice = format!("Repository operation failed: {error}");
			panel.failed = true;
		}
	}
	match outcome.snapshot {
		Ok(snapshot) => {
			panel.repository = snapshot.repository;
			panel.origin = snapshot.origin;
			if outcome.committed || panel.follows_suggestion() {
				panel.message.set(snapshot.suggestion.clone());
			}
			panel.generated = snapshot.suggestion;
			match snapshot.review {
				Ok(review) if succeeded => panel.set_review(review),
				// A failed action, a commit above all, needs another review.
				Ok(_) => panel.set_review(None),
				Err(error) => {
					panel.set_review(None);
					panel.notice.push_str(&format!(" Review failed: {error}"));
					panel.failed = true;
				}
			}
		}
		Err(error) => {
			panel.set_review(None);
			panel.notice.push_str(&format!(" Refresh failed: {error}"));
			panel.failed = true;
		}
	}
}

/// Why commit cannot run now, if anything stops it.
fn commit_blocker(app: &App) -> Option<&'static str> {
	let panel = &app.repository;
	if pending_writes(app) {
		return Some("Finish or drop queued changes first.");
	}
	if panel.jj() && panel.parent() {
		return Some("Refused inside a larger JJ repository.");
	}
	match &panel.review {
		None => Some("Press r to review first."),
		Some(review) if review.message() != panel.message.text => {
			Some("The message changed. Press r to review it.")
		}
		Some(review) if review.diff().trim().is_empty() => Some("Nothing to commit."),
		Some(_) => None,
	}
}

/// Why push cannot run now, before a name is entered.
fn push_blocker(app: &App) -> Option<&'static str> {
	let panel = &app.repository;
	if pending_writes(app) {
		Some("Finish or drop queued changes first.")
	} else if panel.jj() && panel.parent() {
		Some("Refused inside a larger JJ repository.")
	} else if panel.origin.is_none() {
		Some("Add an origin remote to push.")
	} else {
		None
	}
}

/// GitHub creation is offered only for a repository rooted at the
/// configuration that has no origin yet.
fn offers_remote(panel: &Panel) -> bool {
	panel.repository.is_some() && panel.origin.is_none() && !panel.parent()
}

pub(crate) fn handle_key(app: &mut App, tx: &mpsc::Sender<AppEvent>, key: KeyEvent) {
	// Keys wait while a subprocess runs, so nothing conflicting can start.
	if app.repository.busy {
		return;
	}
	let control = key.modifiers.contains(KeyModifiers::CONTROL);
	if key.code == KeyCode::Char('c') && control {
		app.should_quit = true;
		return;
	}
	if let Some(action) = app.repository.confirmation.take() {
		match key.code {
			KeyCode::Char('y') => start(app, tx, action),
			KeyCode::Char('n') | KeyCode::Esc => {}
			_ => app.repository.confirmation = Some(action),
		}
		return;
	}
	if scroll(&mut app.repository, key) {
		return;
	}
	match app.repository.form {
		Some(form) => form_key(app, tx, form, key),
		None => command_key(app, tx, key),
	}
}

/// Diff scrolling and panning, available in every view.
fn scroll(panel: &mut Panel, key: KeyEvent) -> bool {
	let alt = key.modifiers.contains(KeyModifiers::ALT);
	match key.code {
		KeyCode::PageDown => {
			panel.scroll = panel
				.scroll
				.saturating_add(10)
				.min(panel.shown().len().saturating_sub(1));
		}
		KeyCode::PageUp => panel.scroll = panel.scroll.saturating_sub(10),
		KeyCode::Right if alt => panel.horizontal = panel.horizontal.saturating_add(10),
		KeyCode::Left if alt => panel.horizontal = panel.horizontal.saturating_sub(10),
		_ => return false,
	}
	true
}

fn command_key(app: &mut App, tx: &mpsc::Sender<AppEvent>, key: KeyEvent) {
	if key
		.modifiers
		.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
	{
		return;
	}
	let panel = &mut app.repository;
	let detected = panel.repository.is_some();
	match key.code {
		KeyCode::Esc => app.should_quit = true,
		KeyCode::Char('r') => start(app, tx, Action::Refresh),
		KeyCode::Char('g') if panel.loaded && !detected => {
			panel.confirmation = Some(Action::Init(Backend::Git))
		}
		KeyCode::Char('j') if panel.loaded && !detected => {
			panel.confirmation = Some(Action::Init(Backend::Jj))
		}
		_ if !detected => {}
		KeyCode::Down | KeyCode::Char('j') => {
			panel.select((panel.selected + 1) % (panel.diff.files.len() + 1))
		}
		KeyCode::Up | KeyCode::Char('k') => {
			let count = panel.diff.files.len() + 1;
			panel.select((panel.selected + count - 1) % count);
		}
		KeyCode::Char('s') => panel.show_status = !panel.show_status,
		KeyCode::Char('m') => {
			panel.message.cursor = panel.message.text.len();
			panel.form = Some(Form::Message);
		}
		KeyCode::Char('c') => match (commit_blocker(app), app.repository.review.clone()) {
			(None, Some(review)) => app.repository.confirmation = Some(Action::Commit(review)),
			(blocker, _) => {
				app.repository.notice = format!(
					"Cannot commit: {}",
					blocker.unwrap_or("press r to review first.")
				);
				app.repository.failed = false;
			}
		},
		KeyCode::Char('p') => match push_blocker(app) {
			None => app.repository.form = Some(Form::Push),
			Some(blocker) => {
				app.repository.notice = format!("Cannot push: {blocker}");
				app.repository.failed = false;
			}
		},
		KeyCode::Char('h') if offers_remote(panel) => {
			panel.public = false;
			panel.form = Some(Form::Remote);
		}
		_ => {}
	}
}

fn form_key(app: &mut App, tx: &mpsc::Sender<AppEvent>, form: Form, key: KeyEvent) {
	let panel = &mut app.repository;
	let control = key.modifiers.contains(KeyModifiers::CONTROL);
	let submit = key.code == KeyCode::Char('s') && control;
	match form {
		Form::Message => {
			if key.code == KeyCode::Esc || submit {
				panel.form = None;
				// Commit uses only a reviewed message, so an edit is reviewed now.
				let reviewed = panel.review.as_ref().map(Review::message);
				if reviewed != Some(panel.message.text.as_str()) {
					start(app, tx, Action::Refresh);
				}
			} else {
				panel.message.edit(key, true);
			}
		}
		Form::Push => match key.code {
			KeyCode::Esc => panel.form = None,
			KeyCode::Char('b') if control && panel.jj() => {
				panel.create_bookmark = !panel.create_bookmark;
				if panel.create_bookmark {
					panel.branch.set("nixbox");
				}
			}
			KeyCode::Enter => {
				let name = panel.branch.text.trim().to_owned();
				let kind = if panel.jj() { "bookmark" } else { "branch" };
				if name.is_empty() {
					panel.notice = format!("Enter the {kind} to push.");
				} else if panel.create_bookmark && name != "nixbox" {
					panel.notice = "Advancing the nixbox bookmark pushes only nixbox.".into();
				} else {
					panel.form = None;
					panel.confirmation = Some(Action::Push(name, panel.create_bookmark));
				}
			}
			_ => panel.branch.edit(key, false),
		},
		Form::Remote => match key.code {
			KeyCode::Esc => panel.form = None,
			KeyCode::Char('v') if control => panel.public = !panel.public,
			KeyCode::Enter => {
				let name = panel.remote.text.trim().to_owned();
				let valid = name.split_once('/').is_some_and(|(owner, repo)| {
					!owner.is_empty() && !repo.is_empty() && !repo.contains('/')
				});
				if valid {
					panel.form = None;
					let visibility = if panel.public {
						Visibility::Public
					} else {
						Visibility::Private
					};
					panel.confirmation = Some(Action::Remote(name, visibility));
				} else {
					panel.notice = "Enter the repository as OWNER/NAME.".into();
				}
			}
			_ => panel.remote.edit(key, false),
		},
	}
}

/// Footer keys for the VCS tab's current state.
pub(crate) fn hints(app: &App) -> &'static str {
	let panel = &app.repository;
	if panel.busy {
		return "waiting for the repository operation";
	}
	if panel.confirmation.is_some() {
		return "y confirm  n/esc cancel";
	}
	match panel.form {
		Some(Form::Message) => "type  ↵ newline  ←→ home/end cursor  esc done",
		Some(Form::Push) if panel.jj() => {
			"type bookmark  ↵ push  ctrl-b nixbox bookmark  esc cancel"
		}
		Some(Form::Push) => "type branch  ↵ push  esc cancel",
		Some(Form::Remote) => "type OWNER/NAME  ↵ create  ctrl-v visibility  esc cancel",
		None if panel.repository.is_none() => {
			"g init Git  j init Jujutsu  r refresh  tab/shift-tab tabs  esc quit"
		}
		None => {
			"↑↓ files  m message  c commit  p push  r refresh  s status  pgup/pgdn scroll  tab/shift-tab tabs"
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
	Added,
	Deleted,
	Renamed,
	Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
	/// `diff --git`, which starts a file.
	File,
	/// Metadata between a file header and its first hunk.
	Header,
	Hunk,
	Added,
	Removed,
	Context,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileDiff {
	path: String,
	change: Change,
	added: usize,
	removed: usize,
	/// This file's lines within `Diff::lines`.
	lines: Range<usize>,
}

/// A Git-format diff split into kinds of lines and per-file sections.
#[derive(Debug, Default)]
struct Diff {
	lines: Vec<(Kind, String)>,
	files: Vec<FileDiff>,
}

impl Diff {
	fn parse(text: &str) -> Self {
		let mut diff = Self::default();
		let mut in_hunk = false;
		for (index, raw) in text.lines().enumerate() {
			let kind = if let Some(header) = raw.strip_prefix("diff --git ") {
				in_hunk = false;
				diff.files.push(FileDiff {
					path: header
						.split_once(" b/")
						.map_or(header, |(_, path)| path)
						.to_owned(),
					change: Change::Modified,
					added: 0,
					removed: 0,
					lines: index..index,
				});
				Kind::File
			} else if raw.starts_with("@@") {
				in_hunk = true;
				Kind::Hunk
			} else if !in_hunk {
				Kind::Header
			} else if raw.starts_with('+') {
				Kind::Added
			} else if raw.starts_with('-') {
				Kind::Removed
			} else {
				Kind::Context
			};
			if let Some(file) = diff.files.last_mut() {
				file.lines.end = index + 1;
				match kind {
					Kind::Added => file.added += 1,
					Kind::Removed => file.removed += 1,
					Kind::Header if raw.starts_with("new file mode") => file.change = Change::Added,
					Kind::Header if raw.starts_with("deleted file mode") => {
						file.change = Change::Deleted
					}
					Kind::Header if raw.starts_with("rename from") => file.change = Change::Renamed,
					_ => {}
				}
			}
			diff.lines.push((kind, raw.replace('\t', "    ")));
		}
		diff
	}
}

fn muted() -> Style {
	Style::default().fg(Color::DarkGray)
}

fn heading(title: &str) -> Line<'static> {
	Line::styled(title.to_uppercase(), muted().add_modifier(Modifier::BOLD))
}

fn counts(added: usize, removed: usize) -> Vec<Span<'static>> {
	let mut spans = Vec::new();
	if added > 0 {
		spans.push(Span::styled(
			format!(" +{added}"),
			Style::default().fg(Color::Green),
		));
	}
	if removed > 0 {
		spans.push(Span::styled(
			format!(" -{removed}"),
			Style::default().fg(Color::Red),
		));
	}
	spans
}

/// A command key followed by its label, or by why it cannot run.
fn command(key: &'static str, label: String, blocker: Option<&'static str>) -> Line<'static> {
	match blocker {
		None => Line::from(vec![
			Span::styled(key, Style::default().add_modifier(Modifier::BOLD)),
			Span::raw(format!(" {label}")),
		]),
		Some(blocker) => Line::from(vec![
			Span::styled(format!("{key} {label}"), muted()),
			Span::styled(
				format!("  {blocker}"),
				muted().add_modifier(Modifier::ITALIC),
			),
		]),
	}
}

pub(crate) fn draw(f: &mut Frame, area: Rect, app: &App) {
	let panel = &app.repository;
	let block = crate::ui::panel(app.theme()).title(" Version control ");
	let inner = block.inner(area);
	f.render_widget(block, area);
	let [header_area, notice_area, body] = Layout::vertical([
		Constraint::Length(1),
		Constraint::Length(2),
		Constraint::Min(1),
	])
	.areas(inner);
	draw_header(f, header_area, app);
	draw_notice(f, notice_area, app);
	if panel.repository.is_some() {
		let [changes, actions] = Layout::horizontal([
			Constraint::Min(20),
			Constraint::Length(ACTIONS_WIDTH.min(body.width * 2 / 5)),
		])
		.areas(body);
		draw_changes(f, changes, app);
		draw_actions(f, actions, app);
	} else {
		draw_empty(f, body, app);
	}
}

/// The backend, the repository root and a warning when the configuration is
/// only part of it.
fn draw_header(f: &mut Frame, area: Rect, app: &App) {
	let panel = &app.repository;
	let bold = Style::default().add_modifier(Modifier::BOLD);
	let mut spans = match &panel.repository {
		Some(repo) => vec![
			Span::styled(backend_name(repo.backend), bold.fg(app.theme().title_color)),
			Span::raw("  "),
			Span::raw(repo.root.display().to_string()),
		],
		None => vec![
			Span::styled("No repository", bold),
			Span::raw("  "),
			Span::raw(app.session.config_dir().display().to_string()),
		],
	};
	if panel.parent() {
		spans.push(Span::styled(
			if panel.jj() {
				"  subdirectory: review only, commit and push are refused"
			} else {
				"  subdirectory: commits cover only the configuration"
			},
			Style::default().fg(Color::Yellow),
		));
	}
	f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// A pending confirmation, otherwise the latest result.
fn draw_notice(f: &mut Frame, area: Rect, app: &App) {
	let panel = &app.repository;
	let warning = Style::default().fg(Color::Yellow);
	let line = if let Some(action) = &panel.confirmation {
		let question = match action {
			Action::Init(backend) => format!(
				"Initialize {} in the configuration directory? Nothing is committed.",
				backend_name(*backend)
			),
			Action::Commit(_) => {
				let files = panel.diff.files.len();
				format!(
					"Commit {files} file{} locally? Nothing is pushed.",
					if files == 1 { "" } else { "s" }
				)
			}
			Action::Push(name, true) => {
				format!("Advance the nixbox bookmark and push {name} to origin?")
			}
			Action::Push(name, false) => format!("Push {name} to origin?"),
			Action::Remote(name, visibility) => format!(
				"Create the {} GitHub repository {name} and add it as origin? Nothing is pushed.",
				match visibility {
					Visibility::Private => "private",
					Visibility::Public => "public",
				}
			),
			Action::Refresh => String::new(),
		};
		Line::from(vec![
			Span::styled(question, warning.add_modifier(Modifier::BOLD)),
			Span::styled("  y confirm  n cancel", muted()),
		])
	} else if panel.busy {
		let spinner = crate::ui::SPINNER[app.spinner_frame % crate::ui::SPINNER.len()];
		Line::styled(format!("{spinner} {}", panel.notice), warning)
	} else if pending_writes(app) {
		Line::styled(PENDING, warning)
	} else if panel.failed {
		Line::styled(panel.notice.clone(), Style::default().fg(Color::Red))
	} else {
		Line::styled(panel.notice.clone(), muted())
	};
	f.render_widget(Paragraph::new(line).wrap(Wrap { trim: true }), area);
}

fn draw_empty(f: &mut Frame, area: Rect, app: &App) {
	let panel = &app.repository;
	let lines = if !panel.loaded || panel.busy {
		vec![Line::styled("Looking for a repository...", muted())]
	} else {
		vec![
			Line::styled(
				"Not under version control",
				Style::default().add_modifier(Modifier::BOLD),
			),
			Line::styled(
				"Initialize a repository to review and commit nixbox's changes.",
				muted(),
			),
			Line::default(),
			command("g", "initialize Git".into(), None),
			command("j", "initialize Jujutsu, colocated with Git".into(), None),
		]
	};
	let [_, middle, _] = Layout::vertical([
		Constraint::Fill(1),
		Constraint::Length(lines.len() as u16),
		Constraint::Fill(1),
	])
	.areas(area);
	f.render_widget(
		Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
		middle,
	);
}

/// The changed files, the optional status output, then the diff of the
/// selected file or of all of them.
fn draw_changes(f: &mut Frame, area: Rect, app: &App) {
	let panel = &app.repository;
	let theme = app.theme();
	let Some(review) = &panel.review else {
		let text = if panel.busy {
			"Reviewing changes..."
		} else if pending_writes(app) {
			"The review waits until queued changes and rebuilds finish."
		} else {
			"Press r to review the configuration's changes."
		};
		f.render_widget(Paragraph::new(Line::styled(text, muted())), area);
		return;
	};
	let files = &panel.diff.files;
	let list_rows = (files.len() + 1).min(8) as u16;
	let status_rows = if panel.show_status {
		review.status().lines().count().clamp(1, 6) as u16
	} else {
		0
	};
	let [title, list, status, diff] = Layout::vertical([
		Constraint::Length(1),
		Constraint::Length(list_rows),
		Constraint::Length(status_rows),
		Constraint::Min(1),
	])
	.areas(area);
	f.render_widget(
		Paragraph::new(Line::from(vec![
			Span::styled("CHANGES ", muted().add_modifier(Modifier::BOLD)),
			Span::styled(files.len().to_string(), muted()),
		])),
		title,
	);

	let total = files.iter().fold((0, 0), |(added, removed), file| {
		(added + file.added, removed + file.removed)
	});
	let first = panel.selected.saturating_sub(list_rows as usize - 1);
	let rows: Vec<Line> = std::iter::once((Span::raw(" "), "All files".to_owned(), total))
		.chain(files.iter().map(|file| {
			let (letter, color) = match file.change {
				Change::Added => ("A", Color::Green),
				Change::Deleted => ("D", Color::Red),
				Change::Renamed => ("R", Color::Cyan),
				Change::Modified => ("M", Color::Yellow),
			};
			(
				Span::styled(
					letter,
					Style::default().fg(color).add_modifier(Modifier::BOLD),
				),
				file.path.clone(),
				(file.added, file.removed),
			)
		}))
		.enumerate()
		.skip(first)
		.take(list_rows as usize)
		.map(|(index, (letter, path, (added, removed)))| {
			let mut spans = vec![Span::raw(" "), letter, Span::raw(" "), Span::raw(path)];
			spans.extend(counts(added, removed));
			let line = Line::from(spans);
			if index == panel.selected {
				line.style(theme.selection_style())
			} else {
				line
			}
		})
		.collect();
	f.render_widget(Paragraph::new(rows), list);

	if panel.show_status {
		let lines: Vec<Line> = if review.status().trim().is_empty() {
			vec![Line::styled(" Clean working copy.", muted())]
		} else {
			review
				.status()
				.lines()
				.map(|line| Line::styled(format!(" {line}"), muted()))
				.collect()
		};
		f.render_widget(Paragraph::new(lines), status);
	}

	let block = Block::default()
		.borders(Borders::TOP)
		.border_style(theme.border_style());
	let inner = block.inner(diff);
	f.render_widget(block, diff);
	let shown = panel.shown();
	if shown.is_empty() {
		f.render_widget(
			Paragraph::new(Line::styled(
				"No changes to commit. The working copy is clean.",
				muted(),
			)),
			inner,
		);
		return;
	}
	let lines: Vec<Line> = panel.diff.lines[shown]
		.iter()
		.skip(panel.scroll)
		.take(inner.height as usize)
		.map(|(kind, text)| {
			let style = match kind {
				Kind::File => Style::default().add_modifier(Modifier::BOLD),
				Kind::Header => muted(),
				Kind::Hunk => Style::default().fg(Color::Cyan),
				Kind::Added => Style::default().fg(Color::Green),
				Kind::Removed => Style::default().fg(Color::Red),
				Kind::Context => Style::default(),
			};
			Line::styled(
				text.chars().skip(panel.horizontal).collect::<String>(),
				style,
			)
		})
		.collect();
	f.render_widget(Paragraph::new(lines), inner);
}

/// Commit, push and remote controls, each with its key or the reason it
/// cannot run.
fn draw_actions(f: &mut Frame, area: Rect, app: &App) {
	let panel = &app.repository;
	let block = Block::default()
		.borders(Borders::LEFT)
		.border_style(app.theme().border_style());
	let inner = block.inner(area);
	f.render_widget(block, area);
	let width = inner.width.saturating_sub(1) as usize;
	let field = |label: &'static str, value: Vec<Span<'static>>| {
		let mut spans = vec![Span::styled(format!(" {label:<9}"), muted())];
		spans.extend(value);
		Line::from(spans)
	};
	let input = |editor: &Editor, editing: bool, placeholder: &'static str| -> Span<'static> {
		if editing {
			let line = editor.lines(1, width.saturating_sub(10)).remove(0);
			Span::styled(line, Style::default().add_modifier(Modifier::UNDERLINED))
		} else if editor.text.is_empty() {
			Span::styled(placeholder, muted())
		} else {
			Span::raw(editor.text.clone())
		}
	};

	let mut lines = vec![heading("Commit")];
	let editing = panel.form == Some(Form::Message);
	let message: Vec<String> = if editing {
		panel.message.lines(MESSAGE_ROWS, width)
	} else {
		panel
			.message
			.text
			.lines()
			.take(MESSAGE_ROWS)
			.map(|line| line.chars().take(width).collect())
			.collect()
	};
	let style = if editing {
		Style::default().add_modifier(Modifier::UNDERLINED)
	} else {
		Style::default()
	};
	let shown = message.len();
	lines.extend(
		message
			.into_iter()
			.map(|line| Line::styled(format!(" {line}"), style)),
	);
	if shown == 0 {
		lines.push(Line::styled(" No message yet.", muted()));
	}
	lines.push(command("m", "edit message".into(), None));
	let files = panel.diff.files.len();
	lines.push(command(
		"c",
		match files {
			0 => "commit".into(),
			1 => "commit 1 file".into(),
			files => format!("commit {files} files"),
		},
		commit_blocker(app),
	));

	lines.push(Line::default());
	lines.push(heading("Push"));
	lines.push(field(
		"origin",
		vec![match &panel.origin {
			Some(origin) => Span::raw(origin.clone()),
			None => Span::styled("none", muted()),
		}],
	));
	let jj = panel.jj();
	lines.push(field(
		if jj { "bookmark" } else { "branch" },
		vec![input(
			&panel.branch,
			panel.form == Some(Form::Push),
			"press p to enter",
		)],
	));
	if jj {
		lines.push(field(
			"advance",
			vec![Span::raw(if panel.create_bookmark {
				"nixbox bookmark (ctrl-b)"
			} else {
				"off (ctrl-b)"
			})],
		));
	}
	lines.push(command("p", "push to origin".into(), push_blocker(app)));

	if offers_remote(panel) {
		lines.push(Line::default());
		lines.push(heading("GitHub remote"));
		lines.push(field(
			"name",
			vec![input(
				&panel.remote,
				panel.form == Some(Form::Remote),
				"OWNER/NAME",
			)],
		));
		lines.push(field(
			"access",
			vec![Span::raw(if panel.public {
				"public (ctrl-v)"
			} else {
				"private (ctrl-v)"
			})],
		));
		lines.push(command(
			"h",
			"create and add as origin".into(),
			pending_writes(app).then_some("Finish or drop queued changes first."),
		));
	}
	f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::app::QueuedOp;
	use nixbox_config::{Config, Target};
	use nixbox_nix::Manifest;

	fn app() -> App {
		App::new(
			Config::default(),
			Manifest::default(),
			Manifest::default(),
			Vec::new(),
		)
	}

	fn key(code: KeyCode) -> KeyEvent {
		KeyEvent::new(code, KeyModifiers::NONE)
	}

	fn detected(app: &mut App, root: &std::path::Path, origin: Option<&str>) {
		app.repository.loaded = true;
		app.repository.repository = Some(Repository {
			backend: Backend::Git,
			root: root.into(),
			config_root: root.into(),
		});
		app.repository.origin = origin.map(Into::into);
	}

	#[tokio::test]
	async fn worker_returns_errors_through_app_events_without_blocking_input() {
		let mut app = app();
		let (tx, mut rx) = mpsc::channel(8);
		start(&mut app, &tx, Action::Push("missing".into(), false));
		assert!(app.repository.busy);
		let event = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
			.await
			.unwrap()
			.unwrap();
		crate::handlers::handle_app_event(&mut app, &tx, event);
		assert!(!app.repository.busy);
		assert!(app.repository.failed);
		assert!(
			app.repository
				.notice
				.contains("Repository operation failed")
		);
	}

	#[test]
	fn mutations_require_confirmation_and_escape_cancels() {
		let mut app = app();
		app.repository.loaded = true;
		let (tx, mut rx) = mpsc::channel(8);
		handle_key(&mut app, &tx, key(KeyCode::Char('g')));
		assert!(matches!(
			app.repository.confirmation,
			Some(Action::Init(Backend::Git))
		));
		handle_key(&mut app, &tx, key(KeyCode::Esc));
		assert!(app.repository.confirmation.is_none());
		assert!(!app.should_quit);
		assert!(rx.try_recv().is_err());

		let root = std::env::temp_dir();
		detected(&mut app, &root, None);
		handle_key(&mut app, &tx, key(KeyCode::Char('h')));
		assert!(app.repository.form == Some(Form::Remote));
		app.repository.remote.set("owner/config");
		handle_key(&mut app, &tx, key(KeyCode::Enter));
		assert!(matches!(
			app.repository.confirmation,
			Some(Action::Remote(_, Visibility::Private))
		));
		handle_key(&mut app, &tx, key(KeyCode::Char('n')));
		assert!(app.repository.confirmation.is_none());
		assert!(!app.repository.busy);
	}

	#[test]
	fn push_is_separate_and_preserves_explicit_input() {
		let mut app = app();
		app.tab = crate::app::Tab::Vcs;
		let (tx, _rx) = mpsc::channel(8);
		detected(
			&mut app,
			&std::env::temp_dir(),
			Some("git@example:me/config"),
		);
		handle_key(&mut app, &tx, key(KeyCode::Char('p')));
		assert!(app.repository.form == Some(Form::Push));
		for ch in "my-branch".chars() {
			handle_key(&mut app, &tx, key(KeyCode::Char(ch)));
		}
		crate::nav::cycle_tab(&mut app);
		assert_eq!(app.tab, crate::app::Tab::Search);
		crate::nav::cycle_tab_back(&mut app);
		assert_eq!(app.tab, crate::app::Tab::Vcs);
		assert!(app.repository.form == Some(Form::Push));
		handle_key(&mut app, &tx, key(KeyCode::Enter));
		assert!(
			matches!(&app.repository.confirmation, Some(Action::Push(name, false)) if name == "my-branch")
		);
		assert!(!app.repository.busy);
	}

	#[test]
	fn editor_keeps_unicode_boundaries() {
		let mut editor = Editor::default();
		editor.set("é");
		editor.edit(key(KeyCode::Enter), true);
		editor.edit(key(KeyCode::Char('界')), true);
		editor.edit(key(KeyCode::Left), true);
		editor.edit(key(KeyCode::Delete), true);
		assert_eq!(editor.text, "é\n");
		editor.edit(key(KeyCode::Backspace), true);
		editor.edit(key(KeyCode::Backspace), true);
		assert_eq!(editor.text, "");
	}

	#[test]
	fn diffs_split_into_files_with_counts_and_kinds() {
		let diff = Diff::parse(
			"diff --git a/home.nix b/home.nix\n\
			 --- a/home.nix\n\
			 +++ b/home.nix\n\
			 @@ -1,2 +1,2 @@\n\
			 -  old\n\
			 +  new\n\
			 diff --git a/nixbox/packages.nix b/nixbox/packages.nix\n\
			 new file mode 100644\n\
			 @@ -0,0 +1 @@\n\
			 +[ ]\n",
		);
		assert_eq!(diff.files.len(), 2);
		assert_eq!(diff.files[0].path, "home.nix");
		assert_eq!(diff.files[0].change, Change::Modified);
		assert_eq!((diff.files[0].added, diff.files[0].removed), (1, 1));
		assert_eq!(diff.files[0].lines, 0..6);
		assert_eq!(diff.lines[2].0, Kind::Header);
		assert_eq!(diff.files[1].change, Change::Added);
		assert_eq!(diff.files[1].lines, 6..10);
	}

	#[test]
	fn commit_follows_the_review_and_refuses_queue_or_build() {
		let root = std::env::temp_dir().join(format!("nixbox-tui-review-{}", std::process::id()));
		std::fs::create_dir_all(&root).unwrap();
		let vcs = Vcs::for_root(&root).unwrap();
		vcs.init(Backend::Git).unwrap();
		std::fs::write(root.join("home.nix"), "manual configuration\n").unwrap();
		let mut app = app();
		let (tx, _rx) = mpsc::channel(8);
		// A fresh load reviews with the generated message, which must match the draft.
		on_result(
			&mut app,
			Ok(Outcome {
				notice: Ok("Review refreshed.".into()),
				committed: false,
				snapshot: Ok(snapshot(&vcs, None).unwrap()),
			}),
		);
		let generated = app.repository.message.text.clone();
		assert!(!generated.trim().is_empty());
		assert_eq!(app.repository.diff.files[0].path, "home.nix");
		assert!(commit_blocker(&app).is_none());

		app.tab = crate::app::Tab::Vcs;
		let mut terminal =
			ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 40)).unwrap();
		terminal.draw(|frame| crate::ui::draw(frame, &app)).unwrap();
		let screen = terminal.backend().buffer();
		let row = |y| {
			(0..140)
				.map(|x| screen[(x, y)].symbol())
				.collect::<String>()
		};
		assert!(row(0).contains("VCS"));
		assert!(row(1).contains("Version control"));
		assert!(row(39).contains("tab/shift-tab tabs"));
		assert!((2..38).any(|y| row(y).contains("+manual configuration")));
		assert!((2..38).any(|y| row(y).contains("c commit 1 file")));

		// Editing the message invalidates the review until it is reviewed again.
		app.repository.message.set("Other");
		assert!(commit_blocker(&app).is_some());
		app.repository.message.set(generated);

		app.session.queue.push_back(QueuedOp::Uninstall {
			name: "fd".into(),
			scope: Target::HomeManager,
		});
		handle_key(&mut app, &tx, key(KeyCode::Char('c')));
		assert!(app.repository.confirmation.is_none());
		assert!(app.repository.notice.contains("queued changes"));
		app.session.queue.clear();
		let _cancel = app.session.fake_build(Target::HomeManager, "build");
		handle_key(&mut app, &tx, key(KeyCode::Char('c')));
		assert!(app.repository.confirmation.is_none());
		assert!(!app.repository.busy);

		let repository = app.repository.repository.clone();
		on_result(
			&mut app,
			Ok(Outcome {
				notice: Err("configuration changed since review; review again".into()),
				committed: false,
				snapshot: Ok(Snapshot {
					repository,
					origin: None,
					suggestion: String::new(),
					review: Ok(None),
				}),
			}),
		);
		assert!(app.repository.review.is_none());
		assert!(app.repository.failed);
		std::fs::remove_dir_all(root).unwrap();
	}
}
