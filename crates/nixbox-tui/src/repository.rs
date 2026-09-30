use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nixbox_core::vcs::{Backend, Repository, Review, Vcs, Visibility};
use ratatui::{
	Frame,
	layout::{Constraint, Layout, Rect},
	text::{Line, Span},
	widgets::{Paragraph, Wrap},
};
use tokio::sync::mpsc;

use crate::app::{App, AppEvent};

#[derive(Default)]
pub(crate) struct Panel {
	loaded: bool,
	pub busy: bool,
	repository: Option<Repository>,
	origin: Option<String>,
	review: Option<Review>,
	page: Page,
	text: String,
	cursor: usize,
	scroll: usize,
	horizontal: usize,
	public: bool,
	create_bookmark: bool,
	confirmation: Option<Action>,
	notice: String,
}

#[derive(Default, PartialEq, Eq)]
enum Page {
	#[default]
	Menu,
	Message,
	Commit,
	Push,
	Remote,
}

#[derive(Clone)]
enum Action {
	Detect,
	Init(Backend),
	Review(Option<String>),
	Commit(Review),
	Push(String, bool),
	Remote(String, Visibility),
}

#[derive(Debug, Clone)]
pub(crate) enum Outcome {
	Detected(Option<Repository>, Option<String>),
	Reviewed(Review, bool),
	Done(String),
}

pub(crate) fn ensure_loaded(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
	if !app.repository.loaded {
		app.repository.loaded = true;
		start(app, tx, Action::Detect);
	}
}

fn start(app: &mut App, tx: &mpsc::Sender<AppEvent>, action: Action) {
	if app.repository.busy {
		app.repository.notice = "Repository operation is busy.".into();
		return;
	}
	if matches!(action, Action::Commit(_))
		&& (app.session.is_building() || !app.session.queue.is_empty())
	{
		app.repository.notice =
			"Commit refused: finish the build and pending operations first.".into();
		return;
	}
	app.repository.busy = true;
	app.repository.notice = "Repository operation running. Please wait.".into();
	let config = app.session.engine.config.clone();
	let tx = tx.clone();
	tokio::spawn(async move {
		let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Outcome> {
			let vcs = Vcs::new(&config)?;
			Ok(match action {
				Action::Detect => {
					let repo = vcs.detect()?;
					let origin = if repo.is_some() { vcs.origin()? } else { None };
					Outcome::Detected(repo, origin)
				}
				Action::Init(backend) => Outcome::Detected(Some(vcs.init(backend)?), None),
				Action::Review(message) => {
					Outcome::Reviewed(vcs.review(message.as_deref())?, message.is_some())
				}
				Action::Commit(review) => {
					let committed = vcs.commit(&review)?;
					Outcome::Done(format!(
						"Committed {}. {}",
						committed.revision,
						committed.warning.unwrap_or_default()
					))
				}
				Action::Push(name, create) => {
					vcs.push(&name, create)?;
					Outcome::Done(format!("Pushed {name} to origin."))
				}
				Action::Remote(name, visibility) => {
					vcs.create_remote(&name, visibility)?;
					Outcome::Detected(Some(vcs.repository()?), vcs.origin()?)
				}
			})
		})
		.await;
		let result = match result {
			Ok(result) => result.map_err(|error| format!("{error:#}")),
			Err(error) => Err(format!("Repository worker failed: {error}")),
		};
		let _ = tx.send(AppEvent::Repository(result)).await;
	});
}

pub(crate) fn on_result(app: &mut App, result: Result<Outcome, String>) {
	let panel = &mut app.repository;
	panel.loaded = true;
	panel.busy = false;
	panel.confirmation = None;
	panel.scroll = 0;
	panel.horizontal = 0;
	match result {
		Ok(Outcome::Detected(repo, origin)) => {
			panel.repository = repo;
			panel.origin = origin;
			panel.page = Page::Menu;
			panel.review = None;
			panel.notice = "Repository detection complete. No commit or push performed.".into();
		}
		Ok(Outcome::Reviewed(review, edited)) => {
			panel.text = review.message().into();
			panel.cursor = panel.text.len();
			panel.review = Some(review);
			panel.page = if edited { Page::Commit } else { Page::Message };
			panel.notice = "Review all status and diff lines below before committing.".into();
		}
		Ok(Outcome::Done(message)) => {
			panel.notice = message;
			panel.review = None;
			panel.page = Page::Menu;
		}
		Err(error) => {
			panel.notice = format!("Repository error: {error}");
			// A failed commit must be reviewed again, never retried blindly.
			if panel.page == Page::Commit {
				panel.page = Page::Menu;
				panel.review = None;
			}
		}
	}
}

pub(crate) fn handle_key(app: &mut App, tx: &mpsc::Sender<AppEvent>, key: KeyEvent) {
	if app.repository.busy {
		app.repository.notice =
			"Repository operation is busy. Wait before closing or starting another action.".into();
		return;
	}
	let panel = &mut app.repository;
	if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
		app.should_quit = true;
		return;
	}
	if let Some(action) = panel.confirmation.take() {
		if key.code == KeyCode::Char('y') {
			start(app, tx, action);
		} else if !matches!(key.code, KeyCode::Esc | KeyCode::Char('n')) {
			panel.confirmation = Some(action);
		}
		return;
	}
	if key.code == KeyCode::Esc {
		if panel.page == Page::Menu {
			app.should_quit = true;
		} else {
			panel.page = Page::Menu;
			panel.review = None;
		}
		return;
	}
	match key.code {
		KeyCode::PageDown => {
			panel.scroll = panel
				.scroll
				.saturating_add(10)
				.min(review_lines(panel).saturating_sub(1));
			return;
		}
		KeyCode::PageUp => {
			panel.scroll = panel.scroll.saturating_sub(10);
			return;
		}
		KeyCode::Right if key.modifiers.contains(KeyModifiers::ALT) => {
			panel.horizontal = panel.horizontal.saturating_add(10);
			return;
		}
		KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) => {
			panel.horizontal = panel.horizontal.saturating_sub(10);
			return;
		}
		_ => {}
	}
	if panel.page == Page::Commit {
		match key.code {
			KeyCode::Char('y') => {
				if let Some(review) = panel.review.clone() {
					start(app, tx, Action::Commit(review));
				}
			}
			KeyCode::Char('e') => panel.page = Page::Message,
			_ => {}
		}
		return;
	}
	if panel.page != Page::Menu {
		if key.code == KeyCode::Char('v')
			&& key.modifiers.contains(KeyModifiers::CONTROL)
			&& panel.page == Page::Remote
		{
			panel.public = !panel.public;
			return;
		}
		if key.code == KeyCode::Char('b')
			&& key.modifiers.contains(KeyModifiers::CONTROL)
			&& panel.page == Page::Push
		{
			panel.create_bookmark = !panel.create_bookmark;
			return;
		}
		if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
			if panel.text.trim().is_empty() {
				panel.notice = "Enter a nonblank value first.".into();
				return;
			}
			match panel.page {
				Page::Message => {
					let text = panel.text.clone();
					start(app, tx, Action::Review(Some(text)));
				}
				Page::Push => {
					panel.confirmation =
						Some(Action::Push(panel.text.clone(), panel.create_bookmark))
				}
				Page::Remote => {
					if panel.text.split('/').count() != 2 {
						panel.notice = "Enter explicit OWNER/NAME.".into();
						return;
					}
					panel.confirmation = Some(Action::Remote(
						panel.text.clone(),
						if panel.public {
							Visibility::Public
						} else {
							Visibility::Private
						},
					));
				}
				_ => {}
			}
			return;
		}
		edit(panel, key);
		return;
	}
	match key.code {
		KeyCode::Char('d') => start(app, tx, Action::Detect),
		KeyCode::Char('g') => panel.confirmation = Some(Action::Init(Backend::Git)),
		KeyCode::Char('j') => panel.confirmation = Some(Action::Init(Backend::Jj)),
		KeyCode::Char('r') => start(app, tx, Action::Review(None)),
		KeyCode::Char('p') => {
			panel.page = Page::Push;
			panel.text.clear();
			panel.cursor = 0;
			panel.create_bookmark = false;
		}
		KeyCode::Char('h') => {
			panel.page = Page::Remote;
			panel.text.clear();
			panel.cursor = 0;
			panel.public = false;
		}
		_ => {}
	}
}

fn edit(panel: &mut Panel, key: KeyEvent) {
	let previous = panel.text[..panel.cursor]
		.char_indices()
		.next_back()
		.map_or(0, |(index, _)| index);
	let next = panel.text[panel.cursor..]
		.chars()
		.next()
		.map_or(panel.cursor, |ch| panel.cursor + ch.len_utf8());
	match key.code {
		KeyCode::Left => panel.cursor = previous,
		KeyCode::Right => panel.cursor = next,
		KeyCode::Home => panel.cursor = 0,
		KeyCode::End => panel.cursor = panel.text.len(),
		KeyCode::Backspace if panel.cursor > 0 => {
			panel.text.replace_range(previous..panel.cursor, "");
			panel.cursor = previous;
		}
		KeyCode::Delete => {
			panel.text.replace_range(panel.cursor..next, "");
		}
		KeyCode::Enter if panel.page == Page::Message => {
			panel.text.insert(panel.cursor, '\n');
			panel.cursor += 1;
		}
		KeyCode::Char(ch)
			if !key
				.modifiers
				.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
		{
			panel.text.insert(panel.cursor, ch);
			panel.cursor += ch.len_utf8();
		}
		_ => {}
	}
}

fn review_lines(panel: &Panel) -> usize {
	review_content(panel).lines().count()
}

fn review_content(panel: &Panel) -> String {
	panel.review.as_ref().map_or_else(
		|| panel.notice.clone(),
		|review| {
			format!(
				"Message\n{}\n\nStatus\n{}\nFull diff\n{}",
				review.message(),
				review.status(),
				review.diff()
			)
		},
	)
}

pub(crate) fn draw(f: &mut Frame, area: Rect, app: &App) {
	let panel = &app.repository;
	let block = crate::ui::panel(app.theme()).title(" Version control ");
	let inner = block.inner(area);
	f.render_widget(block, area);
	let chunks = Layout::vertical([
		Constraint::Length(3),
		Constraint::Length(6),
		Constraint::Length(
			if matches!(panel.page, Page::Message | Page::Push | Page::Remote) {
				5
			} else {
				0
			},
		),
		Constraint::Min(1),
	])
	.split(inner);
	let repo = panel.repository.as_ref().map_or_else(
		|| "No repository detected".into(),
		|repo| {
			format!(
				"{:?}: {}\nConfig scope: {}",
				repo.backend,
				repo.root.display(),
				repo.config_root.display()
			)
		},
	);
	f.render_widget(
		Paragraph::new(format!(
			"{repo}\norigin: {}",
			panel.origin.as_deref().unwrap_or("not configured")
		)),
		chunks[0],
	);
	let controls = if panel.busy {
		"Busy. Subprocess work is running.".into()
	} else if let Some(action) = &panel.confirmation {
		let description = match action {
			Action::Init(backend) => format!("Initialize {backend:?} in the configuration root?"),
			Action::Push(name, create) => format!(
				"Push {name} to origin {}? Create/advance nixbox bookmark: {create}",
				panel.origin.as_deref().unwrap_or("not configured")
			),
			Action::Remote(name, visibility) => format!(
				"Create GitHub {name} with {visibility:?} visibility and add origin? No push."
			),
			_ => String::new(),
		};
		format!("{description}\ny confirm | n/Esc cancel")
	} else {
		match panel.page {
		Page::Menu => "d detect | g initialize Git | j initialize JJ\nr review/commit | p push origin | h create GitHub repository".into(),
		Page::Message => "Edit multiline message: type, Enter newline, Left/Right, Home/End, Backspace/Delete\nCtrl-S prepare final review | PgUp/PgDn diff | Alt-Left/Right pan".into(),
		Page::Commit => "Final review: y explicitly commit | e edit message | Esc cancel\nPgUp/PgDn status/diff | Alt-Left/Right pan. Commit does not push.".into(),
		Page::Push => format!("Enter branch/bookmark | Ctrl-S request push confirmation\nCtrl-B toggle create/advance nixbox bookmark: {}", panel.create_bookmark),
		Page::Remote => format!("Enter OWNER/NAME | Ctrl-S request creation confirmation\nCtrl-V visibility: {}", if panel.public { "public" } else { "private" }),
	}
	};
	f.render_widget(
		Paragraph::new(format!("{}\n{controls}", panel.notice)).wrap(Wrap { trim: false }),
		chunks[1],
	);
	// Highlight the insertion point and keep it visible even in a long message.
	let prefix = &panel.text[..panel.cursor];
	let row = prefix.chars().filter(|ch| *ch == '\n').count();
	let first = row.saturating_sub(chunks[2].height.saturating_sub(1) as usize);
	let mut text = panel.text.clone();
	if matches!(panel.page, Page::Message | Page::Push | Page::Remote) {
		text.insert(panel.cursor, '|');
	}
	let column = prefix
		.rsplit('\n')
		.next()
		.unwrap_or_default()
		.chars()
		.count();
	let left = column.saturating_sub(chunks[2].width.saturating_sub(1) as usize);
	let lines: Vec<Line> = text
		.lines()
		.skip(first)
		.map(|line| Line::from(Span::raw(line.chars().skip(left).collect::<String>())))
		.collect();
	f.render_widget(Paragraph::new(lines), chunks[2]);
	{
		let content = review_content(panel);
		let lines: Vec<Line> = content
			.lines()
			.skip(panel.scroll)
			.take(chunks[3].height as usize)
			.map(|line| Line::raw(line.chars().skip(panel.horizontal).collect::<String>()))
			.collect();
		f.render_widget(Paragraph::new(lines), chunks[3]);
	}
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

	#[tokio::test]
	async fn worker_returns_errors_through_app_events_without_blocking_input() {
		let mut app = app();
		let (tx, mut rx) = mpsc::channel(8);
		start(&mut app, &tx, Action::Review(Some(String::new())));
		assert!(app.repository.busy);
		let event = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
			.await
			.unwrap()
			.unwrap();
		assert!(matches!(&event, AppEvent::Repository(Err(_))));
		crate::handlers::handle_app_event(&mut app, &tx, event);
		assert!(!app.repository.busy);
		assert!(app.repository.notice.contains("Repository error"));
	}

	#[test]
	fn mutations_require_confirmation_and_escape_cancels() {
		let mut app = app();
		let (tx, mut rx) = mpsc::channel(8);
		handle_key(&mut app, &tx, key(KeyCode::Char('g')));
		assert!(matches!(
			app.repository.confirmation,
			Some(Action::Init(Backend::Git))
		));
		assert!(!app.repository.busy);
		handle_key(&mut app, &tx, key(KeyCode::Esc));
		assert!(app.repository.confirmation.is_none());
		assert!(rx.try_recv().is_err());
		handle_key(&mut app, &tx, key(KeyCode::Char('h')));
		assert!(!app.repository.public);
		app.repository.text = "owner/config".into();
		handle_key(
			&mut app,
			&tx,
			KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
		);
		assert!(matches!(
			app.repository.confirmation,
			Some(Action::Remote(_, Visibility::Private))
		));
		handle_key(&mut app, &tx, key(KeyCode::Char('n')));
		assert!(!app.repository.busy);
	}

	#[test]
	fn push_is_separate_and_preserves_explicit_input() {
		let mut app = app();
		app.tab = crate::app::Tab::Vcs;
		let (tx, _rx) = mpsc::channel(8);
		handle_key(&mut app, &tx, key(KeyCode::Char('p')));
		assert!(!app.repository.create_bookmark);
		app.repository.text = "my-branch".into();
		crate::nav::cycle_tab(&mut app);
		assert_eq!(app.tab, crate::app::Tab::Search);
		crate::nav::cycle_tab_back(&mut app);
		assert_eq!(app.tab, crate::app::Tab::Vcs);
		assert!(app.repository.page == Page::Push);
		handle_key(
			&mut app,
			&tx,
			KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
		);
		assert!(
			matches!(&app.repository.confirmation, Some(Action::Push(name, false)) if name == "my-branch")
		);
		assert!(!app.repository.busy);
	}

	#[test]
	fn multiline_editor_keeps_unicode_boundaries() {
		let mut panel = Panel {
			page: Page::Message,
			text: "é".into(),
			cursor: 2,
			..Panel::default()
		};
		edit(&mut panel, key(KeyCode::Enter));
		edit(&mut panel, key(KeyCode::Char('界')));
		edit(&mut panel, key(KeyCode::Left));
		edit(&mut panel, key(KeyCode::Delete));
		assert_eq!(panel.text, "é\n");
		edit(&mut panel, key(KeyCode::Backspace));
		edit(&mut panel, key(KeyCode::Backspace));
		assert_eq!(panel.text, "");
	}

	#[test]
	fn commit_review_refuses_queue_or_build_and_errors_invalidate_it() {
		let root = std::env::temp_dir().join(format!("nixbox-tui-review-{}", std::process::id()));
		std::fs::create_dir_all(&root).unwrap();
		let vcs = Vcs::for_root(&root).unwrap();
		vcs.init(Backend::Git).unwrap();
		std::fs::write(root.join("home.nix"), "manual configuration\n").unwrap();
		let review = vcs.review(Some("Edited message\n\nDetails")).unwrap();
		let mut app = app();
		let (tx, _rx) = mpsc::channel(8);
		on_result(&mut app, Ok(Outcome::Reviewed(review.clone(), false)));
		assert!(app.repository.page == Page::Message);
		assert_eq!(app.repository.text, review.message());
		on_result(&mut app, Ok(Outcome::Reviewed(review, true)));
		assert!(app.repository.page == Page::Commit);
		assert!(review_content(&app.repository).contains("+manual configuration"));
		assert!(review_content(&app.repository).contains("Edited message\n\nDetails"));
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
		assert!(row(0).contains("nixpkgs"));
		assert!(row(0).contains("VCS"));
		assert!(row(1).contains("Version control"));
		assert!(row(39).contains("tab/shift-tab tabs"));
		assert!((2..38).any(|y| row(y).contains("+manual configuration")));
		app.session.queue.push_back(QueuedOp::Uninstall {
			name: "fd".into(),
			scope: Target::HomeManager,
		});
		handle_key(&mut app, &tx, key(KeyCode::Char('y')));
		assert!(!app.repository.busy);
		assert!(app.repository.notice.contains("pending operations"));
		app.session.queue.clear();
		let _cancel = app.session.fake_build(Target::HomeManager, "build");
		handle_key(&mut app, &tx, key(KeyCode::Char('y')));
		assert!(!app.repository.busy);
		on_result(
			&mut app,
			Err("configuration changed since review; review again".into()),
		);
		assert!(app.repository.review.is_none());
		assert!(app.repository.page == Page::Menu);
		std::fs::remove_dir_all(root).unwrap();
	}
}
