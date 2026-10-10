//! Saved desktop shortcuts and modal editing of the search/filter inputs.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{AnyInputState, InputState};
use gpui_kit::{
	AppContext as _, Context, Entity, FocusHandle, KeyBinding, Keystroke, KeystrokeEvent, Window,
};
use nixbox_config::{Config, InputMode};
use nixbox_core::text_motion;

use crate::app::{NixboxApp, Page, Quit};
use crate::model::{InstalledRows, target_of};
use crate::theme;

macro_rules! shortcuts {
	($( $action:ident, $id:literal, $label:literal, $group:literal, [$($key:literal),*]; )*) => {
		#[derive(Debug, Clone, Copy, PartialEq, Eq)]
		pub enum Action { $( $action, )* }
		pub const SHORTCUTS: &[Shortcut] = &[$(Shortcut {
			action: Action::$action, id: $id, label: $label, group: $group,
			defaults: &[$($key),*],
		}),*];
	};
}

pub struct Shortcut {
	pub action: Action,
	pub id: &'static str,
	pub label: &'static str,
	pub group: &'static str,
	pub defaults: &'static [&'static str],
}

shortcuts! {
	Quit, "quit", "Quit", "Window", ["ctrl-q", "ctrl-c"];
	FocusSearch, "focus-search", "Focus search / filter", "Window", ["ctrl-f"];
	Settings, "settings", "Open / close Settings", "Window", ["ctrl-s"];
	VersionControl, "version-control", "Open version control", "Window", ["ctrl-r"];
	NextTab, "next-tab", "Next tab", "Window", ["tab", "ctrl-tab"];
	PreviousTab, "previous-tab", "Previous tab", "Window", ["shift-tab", "ctrl-shift-tab"];
	Back, "back", "Normal mode / back", "Window", ["escape"];
	ZoomIn, "zoom-in", "Zoom in", "Window", ["ctrl-=", "ctrl-+"];
	ZoomOut, "zoom-out", "Zoom out", "Window", ["ctrl--"];
	ResetZoom, "reset-zoom", "Reset zoom", "Window", ["ctrl-0"];
	NextResult, "next-result", "Next result", "Lists", ["down", "j"];
	PreviousResult, "previous-result", "Previous result", "Lists", ["up", "k"];
	Activate, "activate", "Install / open options / edit", "Lists", ["enter"];
	Remove, "remove", "Remove selected package / flake", "Installed", ["delete"];
	Migrate, "migrate", "Migrate selected declaration", "Installed", ["m"];
	MigrateAll, "migrate-all", "Migrate all declarations", "Installed", ["M"];
	ApplyOptions, "apply-options", "Apply staged options", "Options", ["ctrl-w", "w"];
	UnsetOption, "unset-option", "Unset selected option", "Options", ["ctrl-u", "u"];
	DiscardOptions, "discard-options", "Discard staged options", "Options", ["ctrl-z", "U"];
	ReloadOptions, "reload-options", "Reload options", "Options", ["r"];
	CancelBuild, "cancel-build", "Cancel build", "Build", ["c"];
	ApplyQueue, "apply-queue", "Apply queued operations", "Queue", ["ctrl-enter"];
	DropQueued, "drop-queued", "Drop selected queued operation", "Queue", ["delete"];
	Review, "review", "Refresh review", "Version control", ["r"];
	EditMessage, "edit-message", "Edit commit message", "Version control", ["e"];
	InitGit, "init-git", "Initialize Git", "Version control", ["g"];
	InitJj, "init-jj", "Initialize Jujutsu with Git", "Version control", ["j"];
	Push, "push", "Edit push branch / bookmark", "Version control", ["p"];
	CreateRemote, "create-remote", "Edit GitHub repository details", "Version control", ["h"];
	ConfirmRepository, "confirm-repository", "Commit / push / create remote", "Version control", ["ctrl-enter"];
	ToggleBookmark, "toggle-bookmark", "Toggle nixbox bookmark", "Version control", ["ctrl-b"];
	ToggleStatus, "toggle-status", "Show / hide raw status", "Version control", ["ctrl-v"];
	PanLeft, "pan-left", "Pan diff left", "Version control", ["alt-left"];
	PanRight, "pan-right", "Pan diff right", "Version control", ["alt-right"];
	PageDown, "page-down", "Scroll down", "Scrolling", ["pagedown"];
	PageUp, "page-up", "Scroll up", "Scrolling", ["pageup"];
	Insert, "vim-insert", "Insert before cursor / search", "Vim editing", ["i", "/"];
	Append, "vim-append", "Append after cursor", "Vim editing", ["a"];
	InsertStart, "vim-insert-start", "Insert at start", "Vim editing", ["I"];
	AppendEnd, "vim-append-end", "Append at end", "Vim editing", ["A"];
	Visual, "vim-visual", "Visual selection", "Vim editing", ["v"];
	Left, "vim-left", "Move left", "Vim editing", ["h", "left"];
	Right, "vim-right", "Move right", "Vim editing", ["l", "right"];
	PreviousWord, "vim-previous-word", "Previous word", "Vim editing", ["b"];
	PreviousBigWord, "vim-previous-big-word", "Previous WORD", "Vim editing", ["B"];
	NextWord, "vim-next-word", "Next word", "Vim editing", ["w"];
	NextBigWord, "vim-next-big-word", "Next WORD", "Vim editing", ["W"];
	WordEnd, "vim-word-end", "End of word", "Vim editing", ["e"];
	BigWordEnd, "vim-big-word-end", "End of WORD", "Vim editing", ["E"];
	Start, "vim-start", "Start of input", "Vim editing", ["0"];
	End, "vim-end", "End of input", "Vim editing", ["$"];
	DeleteChar, "vim-delete", "Delete character / selection", "Vim editing", ["x"];
	DeleteSelection, "vim-delete-selection", "Delete visual selection", "Vim editing", ["d"];
	DeleteLine, "vim-delete-line", "Delete input / selection", "Vim editing", ["d d"];
	DeleteToEnd, "vim-delete-to-end", "Delete to end", "Vim editing", ["D"];
	ChangeSelection, "vim-change", "Change visual selection", "Vim editing", ["c"];
}

pub fn keys(config: &Config, shortcut: &Shortcut) -> Vec<String> {
	config
		.gui_shortcuts
		.get(shortcut.id)
		.cloned()
		.unwrap_or_else(|| {
			shortcut
				.defaults
				.iter()
				.map(|key| (*key).to_string())
				.collect()
		})
}

fn binding(key: &str) -> Result<KeyBinding, String> {
	if key.trim().is_empty() {
		return Err("A shortcut cannot be empty.".into());
	}
	for stroke in key.split_whitespace() {
		let parsed = Keystroke::parse(stroke).map_err(|error| error.to_string())?;
		let named = [
			"escape",
			"tab",
			"enter",
			"space",
			"backspace",
			"delete",
			"up",
			"down",
			"left",
			"right",
			"home",
			"end",
			"pageup",
			"pagedown",
			"insert",
		];
		if parsed.key.chars().count() != 1
			&& !named.contains(&parsed.key.as_str())
			&& !(1..=24).any(|n| parsed.key == format!("f{n}"))
		{
			return Err(format!("Unknown key '{}'.", parsed.key));
		}
	}
	Ok(KeyBinding::new(key, Quit, None))
}

// These page-specific actions take priority over Vim motions, matching the TUI.
fn overlaps(a: &Shortcut, b: &Shortcut) -> bool {
	if a.group == "Scrolling" || b.group == "Scrolling" {
		return true;
	}
	if matches!(
		(a.action, b.action),
		(Action::DeleteSelection, Action::DeleteLine)
			| (Action::DeleteLine, Action::DeleteSelection)
	) {
		return false;
	}
	if a.group == "Vim editing" || b.group == "Vim editing" {
		return a.group == b.group
			|| a.group == "Window"
			|| b.group == "Window"
			|| a.group == "Lists"
			|| b.group == "Lists";
	}
	a.group == b.group
		|| a.group == "Window"
		|| b.group == "Window"
		|| (a.group == "Lists" && ["Installed", "Options", "Queue"].contains(&b.group))
		|| (b.group == "Lists" && ["Installed", "Options", "Queue"].contains(&a.group))
}

pub fn validate(config: &Config, id: &str, text: &str) -> Result<Vec<String>, String> {
	let shortcut = SHORTCUTS
		.iter()
		.find(|s| s.id == id)
		.ok_or("Unknown shortcut.")?;
	let proposed: Vec<String> = if text.trim().is_empty() {
		Vec::new()
	} else {
		text.split(',')
			.map(|key| key.split_whitespace().collect::<Vec<_>>().join(" "))
			.collect()
	};
	let parsed = proposed
		.iter()
		.map(|key| binding(key))
		.collect::<Result<Vec<_>, _>>()?;
	for (index, candidate) in parsed.iter().enumerate() {
		for alternative in proposed.iter().skip(index + 1) {
			let strokes = alternative
				.split_whitespace()
				.map(Keystroke::parse)
				.collect::<Result<Vec<_>, _>>()
				.map_err(|e| e.to_string())?;
			if proposed[index].split_whitespace().count() != strokes.len() {
				let current = proposed[index]
					.split_whitespace()
					.map(Keystroke::parse)
					.collect::<Result<Vec<_>, _>>()
					.map_err(|e| e.to_string())?;
				if candidate.match_keystrokes(&strokes).is_some()
					|| binding(alternative)?.match_keystrokes(&current).is_some()
				{
					return Err("One shortcut alternative is a prefix of another.".into());
				}
			}
		}
		for other in SHORTCUTS
			.iter()
			.filter(|other| other.id != id && overlaps(shortcut, other))
		{
			for key in keys(config, other) {
				let Ok(existing) = binding(&key) else {
					continue;
				};
				let strokes = proposed[index]
					.split_whitespace()
					.map(Keystroke::parse)
					.collect::<Result<Vec<_>, _>>()
					.map_err(|e| e.to_string())?;
				let other_strokes = key
					.split_whitespace()
					.map(Keystroke::parse)
					.collect::<Result<Vec<_>, _>>()
					.map_err(|e| e.to_string())?;
				if existing.match_keystrokes(&strokes).is_some()
					|| candidate.match_keystrokes(&other_strokes).is_some()
				{
					return Err(format!(
						"'{}' conflicts with {}. Change that binding first.",
						proposed[index], other.label
					));
				}
			}
		}
	}
	Ok(proposed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
	#[default]
	Normal,
	Insert,
	Visual,
}

pub struct Keyboard {
	pub mode: Mode,
	pub focus: FocusHandle,
	pub inputs: HashMap<&'static str, Entity<InputState>>,
	pub selected: usize,
	pub choice: usize,
	pub package_scroll: gpui_kit::UniformListScrollHandle,
	pub list_scroll: gpui_kit::ScrollHandle,
	pub flake_scroll: gpui_kit::ScrollHandle,
	pub details_scroll: gpui_kit::ScrollHandle,
	pub options_scroll: gpui_kit::ScrollHandle,
	pub queue_scroll: gpui_kit::ScrollHandle,
	pub settings_scroll: gpui_kit::ScrollHandle,
	repository_intent: Action,
	anchor: Option<usize>,
	cursor: usize,
	pending: Vec<Keystroke>,
	last_key: Instant,
}

impl Keyboard {
	pub fn new(config: &Config, window: &mut Window, cx: &mut Context<NixboxApp>) -> Self {
		Self {
			mode: if config.input_mode == InputMode::Vim {
				Mode::Normal
			} else {
				Mode::Insert
			},
			focus: cx.focus_handle(),
			inputs: SHORTCUTS
				.iter()
				.map(|s| {
					(
						s.id,
						cx.new(|cx| {
							let mut input = InputState::new(window, cx).placeholder("Unbound");
							input.set_value(keys(config, s).join(", "), window, cx);
							input
						}),
					)
				})
				.collect(),
			selected: 0,
			choice: 0,
			package_scroll: gpui_kit::UniformListScrollHandle::new(),
			list_scroll: gpui_kit::ScrollHandle::new(),
			flake_scroll: gpui_kit::ScrollHandle::new(),
			details_scroll: gpui_kit::ScrollHandle::new(),
			options_scroll: gpui_kit::ScrollHandle::new(),
			queue_scroll: gpui_kit::ScrollHandle::new(),
			settings_scroll: gpui_kit::ScrollHandle::new(),
			repository_intent: Action::EditMessage,
			anchor: None,
			cursor: 0,
			pending: Vec::new(),
			last_key: Instant::now(),
		}
	}
	pub fn reset_mode(&mut self, mode: InputMode) {
		self.mode = if mode == InputMode::Vim {
			Mode::Normal
		} else {
			Mode::Insert
		};
		self.anchor = None;
		self.pending.clear();
	}
}

impl NixboxApp {
	pub fn set_input_mode(&mut self, mode: InputMode, cx: &mut Context<Self>) {
		self.session.engine.config.input_mode = mode;
		self.keyboard.reset_mode(mode);
		self.save_config(format!("{} enabled.", mode.label()), cx);
	}

	pub fn save_shortcut(
		&mut self,
		id: &'static str,
		reset: bool,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		let mut config = self.session.engine.config.clone();
		if reset {
			config.gui_shortcuts.remove(id);
		} else {
			let text = self.keyboard.inputs[id].read(cx).value();
			match validate(&config, id, &text) {
				Ok(keys) => {
					config.gui_shortcuts.insert(id.to_string(), keys);
				}
				Err(error) => {
					self.status = error.into();
					cx.notify();
					return;
				}
			}
		}
		// Reset can conflict with another customized action too.
		let shortcut = SHORTCUTS
			.iter()
			.find(|s| s.id == id)
			.expect("known shortcut");
		let text = keys(&config, shortcut).join(", ");
		if let Err(error) = validate(&config, id, &text) {
			self.status = error.into();
			cx.notify();
			return;
		}
		if let Err(error) = config.save() {
			self.status = format!("Saving settings failed: {error}").into();
			cx.notify();
			return;
		}
		self.session.engine.config = config;
		self.keyboard.pending.clear();
		self.keyboard.inputs[id].update(cx, |input, cx| input.set_value(text, window, cx));
		self.status = format!("{} shortcut saved.", shortcut.label).into();
		cx.notify();
	}

	pub fn active_filter(&self) -> Option<&Entity<InputState>> {
		match self.page {
			Page::Packages => Some(&self.search_input),
			Page::Flakes => Some(&self.flake_input),
			Page::Installed => Some(&self.installed_input),
			Page::Options => Some(&self.options_filter),
			Page::Nixos => Some(&self.nixos_filter),
			_ => None,
		}
	}

	pub fn enter_insert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		self.keyboard.mode = Mode::Insert;
		self.keyboard.anchor = None;
		if let Some(input) = self.active_filter().cloned() {
			input.update(cx, |input, cx| input.focus(window, cx));
		}
		cx.notify();
	}

	pub fn select_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
		self.keyboard.selected = index;
		self.keyboard.focus.focus(window, cx);
		cx.notify();
	}

	pub fn filter_readonly(&self) -> bool {
		self.session.engine.config.input_mode == InputMode::Vim
			&& self.keyboard.mode != Mode::Insert
	}

	pub fn handle_key(
		&mut self,
		event: &KeystrokeEvent,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		if window.has_active_dialog(cx) {
			return;
		}
		let focused = window.focused_input(cx);
		let filter_focused = matches!(&focused, Some(AnyInputState::Input(input)) if self.active_filter() == Some(input));
		let other_input = focused.is_some() && !filter_focused;
		let editing = other_input || (filter_focused && self.keyboard.mode == Mode::Insert);
		if editing && event.keystroke.key_char.is_some() && event.keystroke.modifiers.alt {
			return;
		}
		if self.keyboard.last_key.elapsed() > Duration::from_secs(1) {
			self.keyboard.pending.clear();
		}
		self.keyboard.last_key = Instant::now();
		self.keyboard.pending.push(event.keystroke.clone());
		let mut matched = None;
		let mut prefix = false;
		for shortcut in SHORTCUTS {
			if !self.shortcut_enabled(shortcut.action, other_input) {
				continue;
			}
			for key in keys(&self.session.engine.config, shortcut) {
				if other_input && matches!(key.as_str(), "tab" | "shift-tab") {
					continue;
				}
				let Ok(binding) = binding(&key) else {
					continue;
				};
				// Letter shortcuts never take text from ordinary fields or insert mode.
				let first = key
					.split_whitespace()
					.next()
					.and_then(|key| Keystroke::parse(key).ok());
				let text_key = first.as_ref().is_some_and(|key| {
					key.key.chars().count() == 1
						&& !key.modifiers.control
						&& !key.modifiers.platform
						&& !key.modifiers.alt
				});
				if text_key
					&& (editing
						|| (self.session.engine.config.input_mode != InputMode::Vim
							&& filter_focused))
				{
					continue;
				}
				if editing
					&& key == "delete"
					&& matches!(shortcut.action, Action::Remove | Action::DropQueued)
				{
					continue;
				}
				// Ctrl-C remains Copy when a field has focus.
				if shortcut.action == Action::Quit && focused.is_some() && key == "ctrl-c" {
					continue;
				}
				match binding.match_keystrokes(&self.keyboard.pending) {
					Some(false) => {
						matched = Some(shortcut.action);
						break;
					}
					Some(true) => prefix = true,
					None => {}
				}
			}
			if matched.is_some() {
				break;
			}
		}
		if let Some(action) = matched {
			self.keyboard.pending.clear();
			cx.stop_propagation();
			window.prevent_default();
			self.keyboard_action(action, window, cx);
		} else if prefix {
			cx.stop_propagation();
			window.prevent_default();
		} else {
			if self.keyboard.pending.len() > 1 {
				self.keyboard.pending.clear();
				self.handle_key(event, window, cx);
				return;
			}
			self.keyboard.pending.clear();
			if filter_focused && self.filter_readonly() && event.keystroke.key.chars().count() == 1
			{
				cx.stop_propagation();
				window.prevent_default();
			}
		}
	}

	fn shortcut_enabled(&self, action: Action, other_input: bool) -> bool {
		use Action::*;
		let normal = self.keyboard.mode != Mode::Visual;
		match action {
			Quit | FocusSearch | Settings | VersionControl | NextTab | PreviousTab | ZoomIn
			| ZoomOut | ResetZoom => true,
			Back => true,
			NextResult | PreviousResult | Activate => {
				normal
					&& !other_input && matches!(
					self.page,
					Page::Packages | Page::Flakes | Page::Installed | Page::Options | Page::Queue
				)
			}
			Remove | Migrate | MigrateAll => {
				!other_input && self.page == Page::Installed && self.keyboard.mode != Mode::Visual
			}
			ApplyOptions | UnsetOption | DiscardOptions | ReloadOptions => {
				!other_input && normal && self.page == Page::Options
			}
			CancelBuild => self.page == Page::Build,
			ApplyQueue | DropQueued => self.page == Page::Queue,
			Review | EditMessage | Push | CreateRemote | InitGit | InitJj | ToggleBookmark
			| ToggleStatus | PanLeft | PanRight => !other_input && self.page == Page::VersionControl,
			ConfirmRepository => self.page == Page::VersionControl,
			PageDown | PageUp => !other_input && self.page != Page::Settings,
			DeleteSelection | ChangeSelection => {
				!other_input
					&& self.filter_readonly()
					&& self.keyboard.mode == Mode::Visual
					&& self.active_filter().is_some()
			}
			Insert | Append | InsertStart | AppendEnd | Visual | DeleteLine => {
				!other_input
					&& self.filter_readonly()
					&& self.keyboard.mode == Mode::Normal
					&& self.active_filter().is_some()
			}
			_ => !other_input && self.filter_readonly() && self.active_filter().is_some(),
		}
	}

	fn keyboard_action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
		use Action::*;
		match action {
			Quit => cx.quit(),
			FocusSearch => {
				let page = if self.active_filter().is_some() {
					self.page
				} else {
					Page::Packages
				};
				self.set_page(page, window, cx);
				self.enter_insert(window, cx);
			}
			Settings => {
				let page = if self.page == Page::Settings {
					self.keyboard_previous_page
				} else {
					self.keyboard_previous_page = self.page;
					Page::Settings
				};
				self.set_page(page, window, cx);
			}
			VersionControl => self.set_page(Page::VersionControl, window, cx),
			NextTab | PreviousTab => {
				let pages = [
					Page::Packages,
					Page::Flakes,
					Page::Installed,
					Page::Nixos,
					Page::Queue,
					Page::Build,
					Page::VersionControl,
					Page::Settings,
				];
				let current = if self.page == Page::Options {
					Page::Installed
				} else {
					self.page
				};
				let index = pages.iter().position(|page| *page == current).unwrap_or(0);
				let delta = if action == NextTab {
					1
				} else {
					pages.len() - 1
				};
				self.set_page(pages[(index + delta) % pages.len()], window, cx);
			}
			Back => {
				if window.focused_input(cx).is_some_and(|focused| !matches!(focused, AnyInputState::Input(input) if self.active_filter() == Some(&input))) { self.keyboard.focus.focus(window, cx); return; }
				if self.filter_readonly() && self.keyboard.mode == Mode::Visual
					|| self.session.engine.config.input_mode == InputMode::Vim
						&& self.keyboard.mode == Mode::Insert
						&& self.active_filter().is_some()
				{
					self.vim_normal(window, cx);
				} else if self.flake_picker_open {
					self.flake_picker_open = false;
				} else if self.page == Page::Options {
					self.close_options(window, cx);
				} else if self.page == Page::Settings {
					self.set_page(self.keyboard_previous_page, window, cx);
				}
			}
			ZoomIn => theme::set_zoom(theme::zoom(cx) + theme::ZOOM_STEP, cx),
			ZoomOut => theme::set_zoom(theme::zoom(cx).saturating_sub(theme::ZOOM_STEP), cx),
			ResetZoom => theme::set_zoom(theme::DEFAULT_ZOOM, cx),
			NextResult | PreviousResult => {
				self.move_result(if action == NextResult { 1 } else { -1 }, window, cx)
			}
			Activate => self.activate_result(window, cx),
			Remove | Migrate => self.installed_action(action, window, cx),
			MigrateAll => self.migrate_all(window, cx),
			ApplyOptions => self.apply_options(window, cx),
			UnsetOption => self.unset_option(cx),
			DiscardOptions => self.discard_options(cx),
			ReloadOptions => self.reload_options(cx),
			CancelBuild => self.cancel_build(cx),
			ApplyQueue => self.apply_queue(cx),
			DropQueued => self.dequeue(self.keyboard.selected, cx),
			Review => self.refresh_repository(window, cx),
			EditMessage => {
				self.keyboard.repository_intent = action;
				self.repository
					.message
					.update(cx, |input, cx| input.focus(window, cx));
			}
			Push => {
				self.keyboard.repository_intent = action;
				self.repository
					.branch
					.update(cx, |input, cx| input.focus(window, cx));
			}
			CreateRemote => {
				self.keyboard.repository_intent = action;
				self.repository
					.owner
					.update(cx, |input, cx| input.focus(window, cx));
			}
			InitGit | InitJj => {
				if self.repository.detected.is_none() {
					self.run_repository(
						crate::repository::RepositoryAction::Init(if action == InitGit {
							nixbox_core::vcs::Backend::Git
						} else {
							nixbox_core::vcs::Backend::Jj
						}),
						window,
						cx,
					);
				}
			}
			ConfirmRepository => {
				let intent = match window.focused_input(cx) {
					Some(AnyInputState::Input(input)) if input == self.repository.branch => Push,
					Some(AnyInputState::Input(input))
						if input == self.repository.owner || input == self.repository.name =>
					{
						CreateRemote
					}
					Some(AnyInputState::Textarea(input)) if input == self.repository.message => {
						EditMessage
					}
					_ => self.keyboard.repository_intent,
				};
				match intent {
					Push => self.push_repository(window, cx),
					CreateRemote => self.create_repository_remote(window, cx),
					_ => self.commit_repository(window, cx),
				}
			}
			ToggleBookmark => self.toggle_bookmark(window, cx),
			ToggleStatus => self.repository.show_status = !self.repository.show_status,
			PanLeft | PanRight => {
				let handle = &self.repository.diff_scroll.0.borrow().base_handle;
				let mut offset = handle.offset();
				offset.x += gpui_kit::px(if action == PanLeft { 80. } else { -80. });
				handle.set_offset(offset);
			}
			PageDown | PageUp => {
				self.keyboard_scroll(if action == PageDown { -300. } else { 300. }, cx)
			}
			_ => self.vim_action(action, window, cx),
		}
		cx.notify();
	}

	fn move_result(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
		if self.page == Page::Flakes && self.flake_picker_open {
			let len = self.flake_details.as_ref().map_or(0, |details| {
				nixbox_core::flake_choices(details, self.session.engine.config.target).len()
			});
			self.keyboard.choice = next_index(self.keyboard.choice, delta, len);
			return;
		}
		let len = match self.page {
			Page::Packages => self.results.len(),
			Page::Flakes => self.flake_results.len(),
			Page::Installed => {
				let rows = InstalledRows::new(
					&self.session.engine,
					&self.installed_input.read(cx).value(),
				);
				rows.managed.len() + rows.flakes.len() + rows.external.len()
			}
			Page::Options => self.options.as_ref().map_or(0, |view| {
				view.visible(&self.options_filter.read(cx).value()).len()
			}),
			Page::Queue => self.session.queue.len(),
			_ => 0,
		};
		let current = if self.page == Page::Flakes {
			self.flake_selected.unwrap_or(0)
		} else {
			self.keyboard.selected
		};
		self.keyboard.selected = next_index(current, delta, len);
		match self.page {
			Page::Packages => self
				.keyboard
				.package_scroll
				.scroll_to_item(self.keyboard.selected, gpui_kit::ScrollStrategy::Top),
			Page::Flakes if len > 0 => {
				self.select_flake(self.keyboard.selected, cx);
				self.keyboard
					.flake_scroll
					.scroll_to_item(self.keyboard.selected);
			}
			Page::Options => {
				let path = self.options.as_ref().and_then(|view| {
					view.visible(&self.options_filter.read(cx).value())
						.get(self.keyboard.selected)
						.map(|entry| entry.path.clone())
				});
				if let Some(path) = path {
					self.select_option(path, window, cx);
				}
				self.keyboard
					.options_scroll
					.scroll_to_item(self.keyboard.selected);
			}
			Page::Installed => {
				let rows = InstalledRows::new(
					&self.session.engine,
					&self.installed_input.read(cx).value(),
				);
				let headings = if self.keyboard.selected < rows.managed.len() {
					1
				} else if self.keyboard.selected < rows.managed.len() + rows.flakes.len() {
					2
				} else {
					3
				};
				self.keyboard
					.list_scroll
					.scroll_to_item(self.keyboard.selected + headings);
			}
			Page::Queue => self
				.keyboard
				.queue_scroll
				.scroll_to_item(self.keyboard.selected + usize::from(self.session.is_building())),
			_ => {}
		}
	}

	fn activate_result(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		match self.page {
			Page::Packages => self.install(self.keyboard.selected, window, cx),
			Page::Flakes => {
				if self.flake_picker_open {
					let path = self.flake_details.as_ref().and_then(|details| {
						nixbox_core::flake_choices(details, self.session.engine.config.target)
							.get(self.keyboard.choice)
							.map(|choice| choice.path.clone())
					});
					if let Some(path) = path {
						self.install_flake_choice(&path, window, cx);
					}
				} else {
					self.install_flake(window, cx);
				}
			}
			Page::Installed => self.installed_action(Action::Activate, window, cx),
			Page::Options => {
				if self
					.options
					.as_ref()
					.is_some_and(|view| view.selected.is_none())
				{
					self.move_result(0, window, cx);
				}
				self.option_input
					.update(cx, |input, cx| input.focus(window, cx));
			}
			_ => {}
		}
	}

	fn installed_action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
		let rows = InstalledRows::new(&self.session.engine, &self.installed_input.read(cx).value());
		let index = self.keyboard.selected;
		if let Some(package) = rows.managed.get(index) {
			match action {
				Action::Remove => {
					self.remove_managed(package.name.clone(), package.scope, window, cx)
				}
				Action::Activate => {
					self.open_options(package.name.clone(), package.scope, window, cx)
				}
				_ => {}
			}
		} else if let Some(flake) = rows.flakes.get(index.saturating_sub(rows.managed.len())) {
			match action {
				Action::Remove if flake.removable => self.remove_flake(flake.clone(), window, cx),
				Action::Migrate if flake.migratable() => {
					self.migrate_flake(flake.clone(), window, cx)
				}
				_ => {}
			}
		} else if let Some(package) = rows
			.external
			.get(index.saturating_sub(rows.managed.len() + rows.flakes.len()))
		{
			match action {
				Action::Migrate if package.migratable => self.migrate(package.clone(), window, cx),
				Action::Activate => {
					self.open_options(package.name.clone(), target_of(package.scope), window, cx)
				}
				_ => {}
			}
		}
	}

	fn keyboard_scroll(&mut self, delta: f32, cx: &mut Context<Self>) {
		let scroll = match self.page {
			Page::Installed => Some(&self.keyboard.list_scroll),
			Page::Flakes => Some(&self.keyboard.details_scroll),
			Page::Options => Some(&self.keyboard.options_scroll),
			Page::Queue => Some(&self.keyboard.queue_scroll),
			_ => None,
		};
		if let Some(scroll) = scroll {
			let mut offset = scroll.offset();
			offset.y += gpui_kit::px(delta);
			scroll.set_offset(offset);
		} else if self.page == Page::Build {
			let mut offset = self.log_scroll.0.borrow().base_handle.offset();
			offset.y += gpui_kit::px(delta);
			self.log_scroll.0.borrow().base_handle.set_offset(offset);
		} else if self.page == Page::VersionControl {
			let mut offset = self.repository.diff_scroll.0.borrow().base_handle.offset();
			offset.y += gpui_kit::px(delta);
			self.repository
				.diff_scroll
				.0
				.borrow()
				.base_handle
				.set_offset(offset);
		}
		cx.notify();
	}
}

fn next_index(current: usize, delta: i32, len: usize) -> usize {
	if len == 0 {
		return 0;
	}
	(current.min(len - 1) as i64 + i64::from(delta)).rem_euclid(len as i64) as usize
}

impl NixboxApp {
	fn vim_normal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		if let Some(input) = self.active_filter().cloned() {
			let value = input.read(cx).value();
			let mut cursor = value[..input.read(cx).cursor().min(value.len())]
				.chars()
				.count();
			if self.keyboard.mode == Mode::Insert {
				cursor = cursor.saturating_sub(1);
			}
			cursor = cursor.min(value.chars().count().saturating_sub(1));
			let offset = byte_offset(&value, cursor);
			input.update(cx, |input, cx| {
				input.set_selected_range(offset..offset, cx);
				input.focus(window, cx);
			});
			self.keyboard.cursor = cursor;
		}
		self.keyboard.mode = Mode::Normal;
		self.keyboard.anchor = None;
	}

	fn vim_action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
		use Action::*;
		let Some(input) = self.active_filter().cloned() else {
			return;
		};
		let value = input.read(cx).value();
		let chars: Vec<char> = value.chars().collect();
		let last = chars.len().saturating_sub(1);
		let current = if self.keyboard.mode == Mode::Visual {
			self.keyboard.cursor.min(last)
		} else {
			value[..input.read(cx).cursor().min(value.len())]
				.chars()
				.count()
				.min(last)
		};
		let target = match action {
			Left => current.saturating_sub(1),
			Right => (current + 1).min(last),
			PreviousWord => text_motion::prev_word_start(&chars, current),
			PreviousBigWord => text_motion::prev_big_word_start(&chars, current),
			NextWord => text_motion::next_word_start(&chars, current),
			NextBigWord => text_motion::next_big_word_start(&chars, current),
			WordEnd => text_motion::word_end(&chars, current),
			BigWordEnd => text_motion::big_word_end(&chars, current),
			Start | InsertStart => 0,
			End => last,
			AppendEnd => chars.len(),
			Append => {
				if chars.is_empty() {
					0
				} else {
					current + 1
				}
			}
			_ => current,
		};
		match action {
			Insert | Append | InsertStart | AppendEnd => {
				self.enter_insert(window, cx);
				let offset = byte_offset(&value, target);
				input.update(cx, |input, cx| input.set_selected_range(offset..offset, cx));
				return;
			}
			Visual if !chars.is_empty() => {
				self.keyboard.mode = Mode::Visual;
				self.keyboard.anchor = Some(current);
			}
			DeleteChar | DeleteSelection | DeleteLine | DeleteToEnd | ChangeSelection => {
				if action == ChangeSelection && self.keyboard.mode != Mode::Visual {
					return;
				}
				let (start, end) = if let Some(anchor) = self.keyboard.anchor {
					(anchor.min(current), anchor.max(current) + 1)
				} else if action == DeleteLine {
					(0, chars.len())
				} else {
					(
						current,
						if action == DeleteToEnd {
							chars.len()
						} else {
							(current + 1).min(chars.len())
						},
					)
				};
				let range = byte_offset(&value, start)..byte_offset(&value, end);
				input.update(cx, |input, cx| {
					input.set_selected_range(range, cx);
					input.replace("", window, cx);
				});
				self.keyboard.mode = if action == ChangeSelection {
					Mode::Insert
				} else {
					Mode::Normal
				};
				self.keyboard.anchor = None;
				let value = input.read(cx).value();
				let cursor = start.min(if self.keyboard.mode == Mode::Insert {
					value.chars().count()
				} else {
					value.chars().count().saturating_sub(1)
				});
				let offset = byte_offset(&value, cursor);
				input.update(cx, |input, cx| input.set_selected_range(offset..offset, cx));
				return;
			}
			_ => {}
		}
		self.keyboard.cursor = target;
		let range = if let Some(anchor) = self.keyboard.anchor {
			byte_offset(&value, anchor.min(target))..byte_offset(&value, anchor.max(target) + 1)
		} else {
			let offset = byte_offset(&value, target);
			offset..offset
		};
		input.update(cx, |input, cx| input.set_selected_range(range, cx));
	}
}

fn byte_offset(value: &str, index: usize) -> usize {
	value
		.char_indices()
		.nth(index)
		.map_or(value.len(), |(offset, _)| offset)
}

#[cfg(test)]
mod tests {
	use super::*;
	use gpui_kit::component::Root;
	use gpui_kit::test::TestWindowExt as _;
	use gpui_kit::{AnyWindowHandle, TestAppContext, px, size};
	use nixbox_core::{Engine, Session};
	use nixbox_nix::{manifest::Manifest, search::SearchHit};

	fn open(
		cx: &mut TestAppContext,
		mode: InputMode,
	) -> (Entity<NixboxApp>, AnyWindowHandle, tokio::runtime::Runtime) {
		cx.update(gpui_kit::init);
		cx.update(crate::app::bind_keys);
		let runtime = tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.expect("runtime");
		let (session, rx) = Session::new(Engine::from_parts(
			Config {
				input_mode: mode,
				..Config::default()
			},
			Manifest::default(),
			Manifest::default(),
			Vec::new(),
		));
		let session = session
			.without_persistence()
			.with_runtime(runtime.handle().clone());
		let mut entity = None;
		let window = cx
			.open_window(size(px(1180.), px(760.)), |window, cx| {
				let app =
					cx.new(|cx| NixboxApp::new(session, rx, runtime.handle().clone(), window, cx));
				entity = Some(app.clone());
				Root::new(app, window, cx)
			})
			.into();
		cx.update_window(window, |_, window, cx| window.render_frame(cx))
			.expect("render");
		(entity.expect("app"), window, runtime)
	}

	#[test]
	fn bindings_reject_invalid_keys_and_conflicts_but_allow_disjoint_pages() {
		let config = Config::default();
		for shortcut in SHORTCUTS {
			assert!(
				validate(&config, shortcut.id, &shortcut.defaults.join(", ")).is_ok(),
				"{}",
				shortcut.id
			);
		}
		assert!(validate(&config, "focus-search", "ctrl-ctrl-f2").is_ok());
		assert!(validate(&config, "focus-search", "ctrl-potato").is_err());
		assert!(
			validate(&config, "focus-search", "ctrl-s")
				.unwrap_err()
				.contains("Settings")
		);
		assert!(validate(&config, "vim-insert", "d").is_err());
		assert!(validate(&config, "focus-search", "ctrl-f,").is_err());
		assert!(validate(&config, "focus-search", "f2, f2 f3").is_err());
		assert!(validate(&config, "focus-search", "f2 f3, f2").is_err());
		assert!(validate(&config, "drop-queued", "j").is_err());
		assert!(validate(&config, "remove", "pagedown").is_err());
		assert!(validate(&config, "vim-left", "pageup").is_err());
		assert_eq!(
			validate(&config, "focus-search", "").unwrap(),
			Vec::<String>::new()
		);
		assert!(validate(&config, "remove", "r").is_ok());
	}

	#[gpui_kit::test]
	async fn tab_navigation_and_settings_work_from_focused_inputs(cx: &mut TestAppContext) {
		let (app, window, _runtime) = open(cx, InputMode::Normal);
		cx.simulate_keystrokes(window, "tab");
		app.read_with(cx, |app, _| assert_eq!(app.page, Page::Flakes));
		cx.simulate_keystrokes(window, "shift-tab");
		app.read_with(cx, |app, _| assert_eq!(app.page, Page::Packages));
		cx.simulate_keystrokes(window, "ctrl-s");
		app.read_with(cx, |app, _| assert_eq!(app.page, Page::Settings));
		cx.update_window(window, |_, window, cx| {
			window.render_frame(cx);
			assert!(window.within("input-mode").find("Normal mode").visible());
			assert!(window.try_find("shortcut-apply-focus-search").is_some());
		})
		.expect("settings");
		cx.simulate_keystrokes(window, "ctrl-s");
		app.read_with(cx, |app, _| assert_eq!(app.page, Page::Packages));
		app.update(cx, |app, _| app.nixos.initialized = true);
		cx.simulate_keystrokes(window, "tab tab tab");
		app.read_with(cx, |app, _| assert_eq!(app.page, Page::Nixos));
		cx.simulate_keystrokes(window, "ctrl-f");
		app.read_with(cx, |app, _| assert_eq!(app.page, Page::Nixos));
		cx.simulate_input(window, "services.pipewire");
		app.read_with(cx, |app, cx| {
			assert_eq!(
				app.nixos_filter.read(cx).value().as_str(),
				"services.pipewire"
			);
		});
		cx.simulate_keystrokes(window, "ctrl-s escape");
		app.read_with(cx, |app, _| assert_eq!(app.page, Page::Nixos));
		cx.simulate_keystrokes(window, "tab");
		app.read_with(cx, |app, _| assert_eq!(app.page, Page::Queue));
	}

	#[gpui_kit::test]
	async fn vim_edits_unicode_and_visual_selections_and_normal_mode_types(
		cx: &mut TestAppContext,
	) {
		let (app, window, _runtime) = open(cx, InputMode::Vim);
		cx.update_window(window, |_, window, cx| {
			app.update(cx, |app, cx| {
				app.search_input
					.update(cx, |input, cx| input.set_value("aé word", window, cx));
			})
		})
		.expect("value");
		cx.simulate_keystrokes(window, "0 l v l x");
		app.read_with(cx, |app, cx| {
			assert_eq!(app.search_input.read(cx).value().as_str(), "aword");
			assert_eq!(app.keyboard.mode, Mode::Normal);
		});
		cx.simulate_keystrokes(window, "A");
		app.read_with(cx, |app, _| assert_eq!(app.keyboard.mode, Mode::Insert));
		cx.simulate_input(window, "!");
		cx.simulate_keystrokes(window, "escape 0 w");
		app.read_with(cx, |app, cx| {
			assert_eq!(app.search_input.read(cx).value().as_str(), "aword!");
			assert_eq!(app.search_input.read(cx).cursor(), 5);
		});
		cx.simulate_keystrokes(window, "d d");
		app.read_with(cx, |app, cx| {
			assert!(app.search_input.read(cx).value().is_empty())
		});
		app.update(cx, |app, _| {
			app.session.engine.config.input_mode = InputMode::Normal;
			app.keyboard.reset_mode(InputMode::Normal);
		});
		cx.simulate_input(window, "jkm");
		app.read_with(cx, |app, cx| {
			assert_eq!(app.search_input.read(cx).value().as_str(), "jkm")
		});
	}

	#[gpui_kit::test]
	async fn customized_bindings_replace_defaults_and_results_follow_the_keyboard(
		cx: &mut TestAppContext,
	) {
		let (app, window, _runtime) = open(cx, InputMode::Vim);
		app.update(cx, |app, cx| {
			app.session
				.engine
				.config
				.gui_shortcuts
				.insert("settings".into(), vec!["f2".into()]);
			app.results = (0..30)
				.map(|index| SearchHit {
					attr: format!("pkg{index}"),
					pname: format!("pkg{index}"),
					version: "1".into(),
					description: String::new(),
				})
				.collect();
			cx.notify();
		});
		cx.simulate_keystrokes(window, "ctrl-s");
		app.read_with(cx, |app, _| assert_eq!(app.page, Page::Packages));
		cx.simulate_keystrokes(window, "f2 f2 j j");
		app.read_with(cx, |app, _| {
			assert_eq!(app.page, Page::Packages);
			assert_eq!(app.keyboard.selected, 2);
		});
		cx.update_window(window, |_, window, cx| {
			window.render_frame(cx);
			assert!(window.find(("install", 2_usize)).visible());
		})
		.expect("selected row");
		cx.simulate_keystrokes(window, "i");
		cx.simulate_input(window, "j");
		app.read_with(cx, |app, cx| {
			assert_eq!(app.keyboard.selected, 2);
			assert_eq!(app.search_input.read(cx).value().as_str(), "j");
		});
	}

	#[gpui_kit::test]
	async fn ordinary_setting_fields_keep_typing_and_conflicting_edits_do_not_apply(
		cx: &mut TestAppContext,
	) {
		let (app, window, _runtime) = open(cx, InputMode::Vim);
		cx.simulate_keystrokes(window, "ctrl-s");
		cx.update_window(window, |_, window, cx| {
			app.update(cx, |app, cx| {
				app.keyboard.inputs["focus-search"].update(cx, |input, cx| {
					input.set_value("", window, cx);
					input.focus(window, cx);
				});
			})
		})
		.expect("binding field");
		cx.simulate_input(window, "ctrl-s");
		cx.simulate_keystrokes(window, "enter");
		app.read_with(cx, |app, _| {
			assert!(
				!app.session
					.engine
					.config
					.gui_shortcuts
					.contains_key("focus-search")
			);
			assert!(app.status.contains("conflicts"));
			assert_eq!(app.page, Page::Settings);
		});
	}

	#[gpui_kit::test]
	async fn keyboard_package_actions_queue_the_selected_row_and_cancel_uses_the_session(
		cx: &mut TestAppContext,
	) {
		let (app, window, _runtime) = open(cx, InputMode::Vim);
		let mut cancel = app.update(cx, |app, cx| {
			app.results = (0..2)
				.map(|index| SearchHit {
					attr: format!("pkg{index}"),
					pname: format!("pkg{index}"),
					version: "1".into(),
					description: String::new(),
				})
				.collect();
			cx.notify();
			app.session
				.fake_build(nixbox_config::Target::NixosSystem, "held build")
		});
		cx.simulate_keystrokes(window, "j enter");
		app.read_with(cx, |app, _| {
			assert!(matches!(app.session.queue.front(), Some(nixbox_core::Op::Install { hit, .. }) if hit.attr == "pkg1"));
		});
		cx.update_window(window, |_, window, cx| {
			app.update(cx, |app, cx| {
				app.set_page(Page::Queue, window, cx);
			})
		})
		.expect("queue");
		cx.simulate_keystrokes(window, "delete");
		app.read_with(cx, |app, _| assert!(app.session.queue.is_empty()));
		cx.update_window(window, |_, window, cx| {
			app.update(cx, |app, cx| {
				app.session
					.engine
					.nixos_manifest
					.packages
					.insert("pkg1".into());
				app.set_page(Page::Installed, window, cx);
			})
		})
		.expect("installed");
		cx.simulate_keystrokes(window, "delete");
		app.read_with(cx, |app, _| assert!(matches!(app.session.queue.front(), Some(nixbox_core::Op::Uninstall { name, .. }) if name == "pkg1")));
		cx.update_window(window, |_, window, cx| {
			app.update(cx, |app, cx| app.set_page(Page::Build, window, cx))
		})
		.expect("build");
		cx.simulate_keystrokes(window, "c");
		assert_eq!(cancel.try_recv(), Ok(()));
	}

	#[gpui_kit::test]
	async fn clicking_the_search_field_enters_insert_and_dialogs_keep_their_keys(
		cx: &mut TestAppContext,
	) {
		let (app, window, _runtime) = open(cx, InputMode::Vim);
		cx.update_window(window, |_, window, cx| {
			window.render_frame(cx);
			let id = app.read(cx).search_input.entity_id().as_u64();
			window.click(("input", id), cx);
		})
		.expect("click field");
		cx.simulate_input(window, "vim");
		app.read_with(cx, |app, cx| {
			assert_eq!(app.keyboard.mode, Mode::Insert);
			assert_eq!(app.search_input.read(cx).value().as_str(), "vim");
		});
		cx.update_window(window, |_, window, cx| {
			window.open_dialog(cx, |dialog, _, _| dialog.title("Test dialog"));
			window.render_frame(cx);
		})
		.expect("dialog");
		cx.simulate_keystrokes(window, "ctrl-s");
		app.read_with(cx, |app, _| assert_eq!(app.page, Page::Packages));
	}
}
