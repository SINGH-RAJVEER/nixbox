//! Repository commands run on the runtime's blocking pool, never during render.

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::{
	AppContext as _, Context, Entity, ScrollStrategy, SharedString, Task, UniformListScrollHandle,
	Window,
};
use nixbox_core::vcs::{Backend, Repository, Review, Vcs, Visibility};

use crate::app::NixboxApp;

pub struct RepositoryControls {
	/// Known before the first refresh, so navigation can show the right logo.
	pub backend: Option<Backend>,
	pub detected: Option<Repository>,
	pub origin: Option<String>,
	pub loaded: bool,
	pub busy: bool,
	pub review: Option<Review>,
	/// The review's diff split for display; only meaningful while `review` is set.
	pub diff: Arc<Diff>,
	/// The file the diff view is narrowed to.
	pub focused: Option<usize>,
	pub diff_scroll: UniformListScrollHandle,
	/// Whether the raw status output is shown under the file list.
	pub show_status: bool,
	pub message: Entity<TextareaState>,
	pub branch: Entity<InputState>,
	pub owner: Entity<InputState>,
	pub name: Entity<InputState>,
	pub public: bool,
	pub create_bookmark: bool,
	pub notice: String,
	generated: String,
	task: Option<Task<()>>,
}

impl RepositoryControls {
	pub fn new(window: &mut Window, cx: &mut Context<NixboxApp>) -> Self {
		Self {
			backend: None,
			detected: None,
			origin: None,
			loaded: false,
			busy: false,
			review: None,
			diff: Arc::default(),
			focused: None,
			diff_scroll: UniformListScrollHandle::new(),
			show_status: false,
			message: cx.new(|cx| {
				TextareaState::new(window, cx)
					.rows(5)
					.placeholder("Commit message")
			}),
			branch: cx.new(|cx| InputState::new(window, cx).placeholder("Branch or bookmark")),
			owner: cx.new(|cx| InputState::new(window, cx).placeholder("GitHub owner")),
			name: cx.new(|cx| InputState::new(window, cx).placeholder("Repository name")),
			public: false,
			create_bookmark: false,
			notice: "Refresh to detect Git or JJ and review configuration changes.".into(),
			generated: String::new(),
			task: None,
		}
	}

	/// Drops everything learned about the previous configuration's
	/// repository, after nixbox moves to `config_dir`.
	pub fn forget(&mut self, config_dir: &Path) {
		self.backend = nearest_backend(config_dir);
		self.detected = None;
		self.origin = None;
		self.loaded = false;
		self.review = None;
		self.diff = Arc::default();
		self.focused = None;
		self.notice = "Refresh to detect Git or JJ and review configuration changes.".into();
	}
}

/// Mirrors `Vcs::detect` with filesystem checks only, so it is cheap enough
/// for startup. The first refresh replaces it with the real answer.
pub fn nearest_backend(config_dir: &Path) -> Option<Backend> {
	config_dir.ancestors().find_map(|dir| {
		if dir.join(".jj").is_dir() {
			Some(Backend::Jj)
		} else if dir.join(".git").exists() {
			Some(Backend::Git)
		} else {
			None
		}
	})
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	Added,
	Deleted,
	Renamed,
	Modified,
}

impl Change {
	pub fn letter(self) -> &'static str {
		match self {
			Change::Added => "A",
			Change::Deleted => "D",
			Change::Renamed => "R",
			Change::Modified => "M",
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
	pub path: String,
	pub change: Change,
	pub added: usize,
	pub removed: usize,
	/// This file's lines within `Diff::lines`.
	pub lines: Range<usize>,
	/// The longest line in `lines`, which sets the scrollable width.
	pub widest: usize,
}

/// How a diff line is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
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
pub struct DiffLine {
	pub kind: Line,
	pub text: SharedString,
	/// The line's number in the old and in the new file, as delta shows them.
	pub old: Option<u32>,
	pub new: Option<u32>,
	/// Byte ranges of `text` that changed within a paired removed and added line.
	pub emphasis: Vec<Range<usize>>,
}

/// A Git-format diff split into lines and per-file sections.
#[derive(Debug, Default)]
pub struct Diff {
	pub lines: Vec<DiffLine>,
	pub files: Vec<FileDiff>,
	pub widest: usize,
	/// Digits in the largest line number, for an aligned gutter.
	pub number_width: usize,
}

/// The old and new start lines of a `@@ -a,b +c,d @@` hunk header.
fn hunk_start(header: &str) -> Option<(u32, u32)> {
	let mut ranges = header.strip_prefix("@@ ")?.split_whitespace();
	let start = |range: &str, sign: char| -> Option<u32> {
		range.strip_prefix(sign)?.split(',').next()?.parse().ok()
	};
	Some((start(ranges.next()?, '-')?, start(ranges.next()?, '+')?))
}

impl Diff {
	/// Parsing, numbering, and within-line emphasis all happen here, on the
	/// repository worker, so rendering only paints prepared lines.
	pub fn parse(text: &str) -> Self {
		let mut lines: Vec<DiffLine> = Vec::new();
		let mut files: Vec<FileDiff> = Vec::new();
		let mut in_hunk = false;
		let (mut old, mut new) = (0, 0);
		let mut largest = 0;
		for (index, raw) in text.lines().enumerate() {
			let kind = if let Some(header) = raw.strip_prefix("diff --git ") {
				in_hunk = false;
				files.push(FileDiff {
					path: header
						.split_once(" b/")
						.map_or(header, |(_, path)| path)
						.to_owned(),
					change: Change::Modified,
					added: 0,
					removed: 0,
					lines: index..index,
					widest: index,
				});
				Line::File
			} else if raw.starts_with("@@") {
				in_hunk = true;
				(old, new) = hunk_start(raw).unwrap_or((0, 0));
				Line::Hunk
			} else if !in_hunk {
				Line::Header
			} else if raw.starts_with('+') {
				Line::Added
			} else if raw.starts_with('-') {
				Line::Removed
			} else {
				Line::Context
			};
			if let Some(file) = files.last_mut() {
				file.lines.end = index + 1;
				match kind {
					Line::Added => file.added += 1,
					Line::Removed => file.removed += 1,
					Line::Header if raw.starts_with("new file mode") => file.change = Change::Added,
					Line::Header if raw.starts_with("deleted file mode") => {
						file.change = Change::Deleted
					}
					Line::Header if raw.starts_with("rename from") => file.change = Change::Renamed,
					_ => {}
				}
			}
			// "\ No newline at end of file" belongs to neither side.
			let numbered = !raw.starts_with('\\');
			let (line_old, line_new) = match kind {
				Line::Context if numbered => (Some(old), Some(new)),
				Line::Removed => (Some(old), None),
				Line::Added => (None, Some(new)),
				_ => (None, None),
			};
			old = old.saturating_add(u32::from(line_old.is_some()));
			new = new.saturating_add(u32::from(line_new.is_some()));
			largest = largest
				.max(line_old.unwrap_or(0))
				.max(line_new.unwrap_or(0));
			lines.push(DiffLine {
				kind,
				text: raw.replace('\t', "    ").into(),
				old: line_old,
				new: line_new,
				emphasis: Vec::new(),
			});
		}
		emphasize(&mut lines);
		let widest_in = |range: Range<usize>| {
			let start = range.start;
			range
				.max_by_key(|index| lines[*index].text.len())
				.unwrap_or(start)
		};
		for file in &mut files {
			file.widest = widest_in(file.lines.clone());
		}
		let widest = widest_in(0..lines.len());
		Self {
			lines,
			files,
			widest,
			number_width: largest.to_string().len(),
		}
	}
}

/// Runs delta's within-line comparison over every change block: removed
/// lines directly followed by added lines.
fn emphasize(lines: &mut [DiffLine]) {
	let mut start = 0;
	while start < lines.len() {
		let run = |from: usize, kind: Line| {
			from + lines[from..]
				.iter()
				.take_while(|line| line.kind == kind || line.text.starts_with('\\'))
				.count()
		};
		let removed_end = run(start, Line::Removed);
		let added_end = run(removed_end, Line::Added);
		if removed_end == start || added_end == removed_end {
			start = added_end.max(start + 1);
			continue;
		}
		// Compare contents without the `-` or `+` prefix, then shift back.
		let content = |line: &DiffLine| line.text.get(1..).unwrap_or_default().to_owned();
		let minus: Vec<String> = lines[start..removed_end]
			.iter()
			.filter(|line| line.kind == Line::Removed)
			.map(content)
			.collect();
		let plus: Vec<String> = lines[removed_end..added_end]
			.iter()
			.filter(|line| line.kind == Line::Added)
			.map(content)
			.collect();
		let minus: Vec<&str> = minus.iter().map(String::as_str).collect();
		let plus: Vec<&str> = plus.iter().map(String::as_str).collect();
		let (removed, added) = crate::delta::emphasis(&minus, &plus);
		for (line, ranges) in lines[start..added_end]
			.iter_mut()
			.filter(|line| matches!(line.kind, Line::Removed | Line::Added))
			.zip(removed.into_iter().chain(added))
		{
			line.emphasis = ranges
				.into_iter()
				.map(|range| range.start + 1..range.end + 1)
				.collect();
		}
		start = added_end;
	}
}

pub enum RepositoryAction {
	Refresh,
	Init(Backend),
	Commit(Review),
	Push {
		name: String,
		create_bookmark: bool,
	},
	CreateRemote {
		name: String,
		visibility: Visibility,
	},
}

struct Snapshot {
	repository: Option<Repository>,
	origin: Option<String>,
	suggestion: String,
	review: std::result::Result<Option<Review>, String>,
	diff: Arc<Diff>,
}

fn snapshot(vcs: &Vcs, message: Option<&str>) -> Result<Snapshot> {
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
	let diff = match &review {
		Ok(Some(review)) => Arc::new(Diff::parse(review.diff())),
		_ => Arc::default(),
	};
	Ok(Snapshot {
		repository,
		origin,
		suggestion,
		review,
		diff,
	})
}

/// Follow updated suggestions only while the draft still matches generated text.
fn follows_suggestion(draft: &str, generated: &str) -> bool {
	draft == generated
}

impl NixboxApp {
	pub fn repository_pending_writes(&self) -> bool {
		self.session.is_building() || !self.session.queue.is_empty()
	}

	pub(crate) fn repository_blocks_mutation(&mut self, cx: &mut Context<Self>) -> bool {
		if self.repository.busy {
			self.status = "Wait for the repository operation before changing configuration.".into();
			cx.notify();
			return true;
		}
		self.repository.review = None;
		false
	}

	pub fn refresh_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		self.run_repository(RepositoryAction::Refresh, window, cx);
	}

	pub fn commit_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		let Some(review) = self.repository.review.clone() else {
			return;
		};
		if review.message() != self.repository.message.read(cx).value().as_str() {
			self.repository.review = None;
			cx.notify();
			return;
		}
		self.run_repository(RepositoryAction::Commit(review), window, cx);
	}

	pub fn push_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		let name = self.repository.branch.read(cx).value().trim().to_owned();
		self.run_repository(
			RepositoryAction::Push {
				name,
				create_bookmark: self.repository.create_bookmark,
			},
			window,
			cx,
		);
	}

	/// Narrows the diff to one file, or shows every file again.
	pub fn focus_file(&mut self, file: Option<usize>, cx: &mut Context<Self>) {
		self.repository.focused = file;
		self.repository
			.diff_scroll
			.scroll_to_item(0, ScrollStrategy::Top);
		cx.notify();
	}

	/// Turning the bookmark on also names it, since that is the only bookmark
	/// it may push.
	pub fn toggle_bookmark(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		self.repository.create_bookmark = !self.repository.create_bookmark;
		if self.repository.create_bookmark {
			self.repository
				.branch
				.update(cx, |input, cx| input.set_value("nixbox", window, cx));
		}
		cx.notify();
	}

	pub fn create_repository_remote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		let owner = self.repository.owner.read(cx).value().trim().to_owned();
		let name = self.repository.name.read(cx).value().trim().to_owned();
		if owner.is_empty() || name.is_empty() || owner.contains('/') || name.contains('/') {
			self.repository.notice =
				"Enter an explicit owner and repository name, without slashes.".into();
			cx.notify();
			return;
		}
		let visibility = if self.repository.public {
			Visibility::Public
		} else {
			Visibility::Private
		};
		self.run_repository(
			RepositoryAction::CreateRemote {
				name: format!("{owner}/{name}"),
				visibility,
			},
			window,
			cx,
		);
	}

	pub fn run_repository(
		&mut self,
		action: RepositoryAction,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		if self.repository.busy {
			return;
		}
		if self.repository_pending_writes() {
			self.repository.review = None;
			self.repository.notice =
				"Finish or drop queued operations and wait for the rebuild, then review again."
					.into();
			cx.notify();
			return;
		}
		let draft = self.repository.message.read(cx).value().to_string();
		let generated = follows_suggestion(&draft, &self.repository.generated);
		let message = (!generated).then_some(draft.clone());
		let config = self.session.engine.config.clone();
		self.repository.busy = true;
		self.repository.review = None;
		self.repository.notice = "Working on repository...".into();
		cx.notify();
		let job = self.runtime.spawn_blocking(move || {
			let vcs = Vcs::new(&config).map_err(|error| format!("{error:#}"))?;
			let result: Result<String> = (|| {
				Ok(match action {
					RepositoryAction::Refresh => {
						"Review refreshed. Commit and push require separate clicks.".into()
					}
					RepositoryAction::Init(backend) => {
						vcs.init(backend)?;
						"Repository initialized. Nothing committed or pushed.".into()
					}
					RepositoryAction::Commit(review) => {
						let outcome = vcs.commit(&review)?;
						format!(
							"Committed {}. Nothing pushed. {}",
							outcome.revision,
							outcome.warning.unwrap_or_default()
						)
					}
					RepositoryAction::Push {
						name,
						create_bookmark,
					} => {
						vcs.push(&name, create_bookmark)?;
						format!("Pushed {name} to origin.")
					}
					RepositoryAction::CreateRemote { name, visibility } => {
						vcs.create_remote(&name, visibility)?;
						format!("Created {name} and added origin. Nothing pushed.")
					}
				})
			})();
			// Keep action success visible even if a subsequent refresh fails.
			let notice = result.map_err(|error| format!("{error:#}"));
			let snapshot = snapshot(&vcs, message.as_deref()).map_err(|error| format!("{error:#}"));
			Ok::<_, String>((notice, snapshot))
		});
		self.repository.task = Some(cx.spawn_in(window, async move |this, cx| {
			let result = match job.await {
				Ok(result) => result,
				Err(error) => Err(error.to_string()),
			};
			let _ = this.update_in(cx, |this, window, cx| {
				this.repository.busy = false;
				this.repository.task = None;
				match result {
					Ok((notice, snapshot)) => {
						let succeeded = notice.is_ok();
						this.repository.notice = notice.unwrap_or_else(|error| {
							format!("Repository operation failed: {error}")
						});
						match snapshot {
							Ok(snapshot) => {
								this.repository.loaded = true;
								this.repository.backend =
									snapshot.repository.as_ref().map(|repo| repo.backend);
								this.repository.detected = snapshot.repository;
								this.repository.origin = snapshot.origin;
								// An edit made while refreshing wins over the worker's result.
								if this.repository.message.read(cx).value().as_str() == draft {
									if generated {
										this.repository.message.update(cx, |input, cx| {
											input.set_value(snapshot.suggestion.clone(), window, cx)
										});
									}
									match snapshot.review {
										// A failed commit requires another explicit review.
										Ok(review) if succeeded => {
											// Stay on the focused file while it still has changes.
											let focused =
												this.repository.focused.and_then(|index| {
													let path = &this
														.repository
														.diff
														.files
														.get(index)?
														.path;
													snapshot
														.diff
														.files
														.iter()
														.position(|file| &file.path == path)
												});
											this.repository.review = review;
											this.repository.diff = snapshot.diff;
											this.repository.focused = focused;
										}
										Ok(_) => {}
										Err(error) => this
											.repository
											.notice
											.push_str(&format!(" Review failed: {error}")),
									}
								}
								this.repository.generated = snapshot.suggestion;
							}
							Err(error) => this
								.repository
								.notice
								.push_str(&format!(" Refresh failed: {error}")),
						}
					}
					Err(error) => {
						this.repository.notice = format!("Repository operation failed: {error}")
					}
				}
				cx.notify();
			});
		}));
	}
}

#[cfg(test)]
mod tests {
	use nixbox_core::vcs::Backend;

	use super::{Change, Diff, Line, follows_suggestion, nearest_backend};

	#[test]
	fn diffs_split_into_files_with_counts_and_kinds() {
		let diff = Diff::parse(
			"diff --git a/home.nix b/home.nix\n\
			 index 1..2 100644\n\
			 --- a/home.nix\n\
			 +++ b/home.nix\n\
			 @@ -1,2 +1,2 @@\n\
			 -  old\n\
			 +  new\n\
			 +\tadded with a much longer line\n\
			 diff --git a/nixbox/packages.nix b/nixbox/packages.nix\n\
			 new file mode 100644\n\
			 --- /dev/null\n\
			 +++ b/nixbox/packages.nix\n\
			 @@ -0,0 +1 @@\n\
			 +[ ]\n",
		);
		assert_eq!(diff.files.len(), 2);
		let home = &diff.files[0];
		assert_eq!(home.path, "home.nix");
		assert_eq!(home.change, Change::Modified);
		assert_eq!((home.added, home.removed), (2, 1));
		assert_eq!(home.lines, 0..8);
		assert_eq!(home.widest, 7);
		assert_eq!(diff.lines[7].text, "+    added with a much longer line");
		let kinds: Vec<Line> = diff.lines.iter().map(|line| line.kind).collect();
		assert_eq!(
			kinds[..8],
			[
				Line::File,
				Line::Header,
				Line::Header,
				Line::Header,
				Line::Hunk,
				Line::Removed,
				Line::Added,
				Line::Added,
			]
		);
		let packages = &diff.files[1];
		assert_eq!(packages.path, "nixbox/packages.nix");
		assert_eq!(packages.change, Change::Added);
		assert_eq!((packages.added, packages.removed), (1, 0));
		assert_eq!(packages.lines, 8..14);
		assert!(Diff::parse("").files.is_empty());
	}

	#[test]
	fn hunks_number_lines_and_emphasize_changed_words_like_delta() {
		let diff = Diff::parse(
			"diff --git a/home.nix b/home.nix\n\
			 @@ -9,3 +9,3 @@ {\n\
			  context\n\
			 -  enable = false;\n\
			 +  enable = true;\n\
			  tail\n\
			 \\ No newline at end of file\n",
		);
		let numbers: Vec<_> = diff.lines.iter().map(|line| (line.old, line.new)).collect();
		assert_eq!(
			numbers,
			[
				(None, None),
				(None, None),
				(Some(9), Some(9)),
				(Some(10), None),
				(None, Some(10)),
				(Some(11), Some(11)),
				(None, None),
			]
		);
		assert_eq!(diff.number_width, 2);
		let emphasized = |index: usize| -> Vec<&str> {
			let line = &diff.lines[index];
			line.emphasis
				.iter()
				.map(|range| &line.text[range.clone()])
				.collect()
		};
		assert_eq!(emphasized(3), ["false"]);
		assert_eq!(emphasized(4), ["true"]);
		assert!(diff.lines[2].emphasis.is_empty());
	}

	#[test]
	fn no_newline_markers_preserve_pairing_and_hunks_restart_numbering() {
		let diff = Diff::parse(
			"diff --git a/home.nix b/home.nix\n\
			 @@ -1 +1 @@\n\
			 -enable = false;\n\
			 \\ No newline at end of file\n\
			 +enable = true;\n\
			 \\ No newline at end of file\n\
			 @@ -100,0 +101 @@\n\
			 +new line\n",
		);
		for (index, expected) in [(2, "false"), (4, "true")] {
			let line = &diff.lines[index];
			let changed: Vec<_> = line
				.emphasis
				.iter()
				.map(|range| &line.text[range.clone()])
				.collect();
			assert_eq!(changed, [expected]);
		}
		assert_eq!((diff.lines[7].old, diff.lines[7].new), (None, Some(101)));
		assert!(diff.lines[7].emphasis.is_empty());
		for index in [3, 5] {
			assert_eq!((diff.lines[index].old, diff.lines[index].new), (None, None));
			assert!(diff.lines[index].emphasis.is_empty());
		}
		assert_eq!(diff.number_width, 3);
	}

	#[test]
	fn the_nearest_repository_marker_names_the_backend() {
		let root = std::env::temp_dir().join(format!("nixbox-backend-{}", std::process::id()));
		let config = root.join("config");
		std::fs::create_dir_all(config.join(".jj")).expect("jj marker");
		std::fs::create_dir_all(root.join(".git")).expect("git marker");
		assert_eq!(nearest_backend(&config), Some(Backend::Jj));
		std::fs::remove_dir_all(config.join(".jj")).expect("remove jj marker");
		assert_eq!(nearest_backend(&config), Some(Backend::Git));
		std::fs::remove_dir_all(&root).expect("cleanup");
	}

	#[test]
	fn refresh_follows_generated_text_but_preserves_edits_and_cleared_drafts() {
		assert!(follows_suggestion("", ""));
		assert!(follows_suggestion("generated", "generated"));
		assert!(!follows_suggestion("my message", "generated"));
		assert!(!follows_suggestion("", "generated"));
	}
}
