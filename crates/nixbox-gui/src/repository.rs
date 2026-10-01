//! Repository commands run on the runtime's blocking pool, never during render.

use anyhow::Result;
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::{AppContext as _, Context, Entity, Task, Window};
use nixbox_core::vcs::{Backend, Repository, Review, Vcs, Visibility};

use crate::app::NixboxApp;

pub struct RepositoryControls {
	pub detected: Option<Repository>,
	pub origin: Option<String>,
	pub loaded: bool,
	pub busy: bool,
	pub review: Option<Review>,
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
			detected: None,
			origin: None,
			loaded: false,
			busy: false,
			review: None,
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
	Ok(Snapshot {
		repository,
		origin,
		suggestion,
		review,
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
										Ok(review) if succeeded => this.repository.review = review,
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
	use super::follows_suggestion;

	#[test]
	fn refresh_follows_generated_text_but_preserves_edits_and_cleared_drafts() {
		assert!(follows_suggestion("", ""));
		assert!(follows_suggestion("generated", "generated"));
		assert!(!follows_suggestion("my message", "generated"));
		assert!(!follows_suggestion("", "generated"));
	}
}
