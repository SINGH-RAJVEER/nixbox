//! Version control page, with a complete review before the commit control.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::{Disableable as _, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
	AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
	StatefulInteractiveElement as _, Styled as _, div,
};
use nixbox_core::vcs::Backend;

use super::{muted, page_header};
use crate::app::NixboxApp;
use crate::repository::RepositoryAction;

pub fn render(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let controls = &app.repository;
	let pending = app.repository_pending_writes();
	let disabled = controls.busy || pending;
	let jj = controls
		.detected
		.as_ref()
		.is_some_and(|repo| repo.backend == Backend::Jj);
	let parent = controls
		.detected
		.as_ref()
		.is_some_and(|repo| repo.root != repo.config_root);
	let mut body = v_flex()
		.w_full()
		.min_w_0()
		.flex_shrink_0()
		.px_6()
		.py_4()
		.gap_3()
		.child(muted(
			format!("Configuration: {}", app.session.config_dir().display()),
			cx,
		))
		.child(
			h_flex()
				.gap_3()
				.child(
					Button::new("repository-refresh")
						.ghost()
						.small()
						.label("Refresh review")
						.disabled(disabled)
						.on_click(
							cx.listener(|this, _, window, cx| this.refresh_repository(window, cx)),
						),
				)
				.child(muted(
					if controls.busy {
						"Working on repository..."
					} else if pending {
						"Queued writes or a rebuild are pending. Finish or drop them before reviewing and committing."
					} else {
						"Repository commands run in the background."
					},
					cx,
				)),
		)
		.child(muted(controls.notice.clone(), cx));
	if let Some(repo) = &controls.detected {
		body = body.child(muted(
			format!(
				"{} repository: {}",
				if jj { "JJ" } else { "Git" },
				repo.root.display()
			),
			cx,
		));
		if parent {
			body = body.child(muted(if jj {
				"JJ parent repositories can be reviewed, but commit, push and remote creation are refused."
			} else {
				"Git commits cover only this configuration directory. Unrelated staged paths must be unstaged first. Remote creation requires a configuration-root repository."
			}, cx));
		}
	} else if controls.loaded {
		body = body
			.child(muted(
				"No repository detected. Choose how to initialize this configuration directory.",
				cx,
			))
			.child(
				h_flex()
					.gap_3()
					.child(
						Button::new("repository-init-git")
							.ghost()
							.small()
							.label("Initialize Git")
							.disabled(disabled)
							.on_click(cx.listener(|this, _, window, cx| {
								this.run_repository(
									RepositoryAction::Init(Backend::Git),
									window,
									cx,
								)
							})),
					)
					.child(
						Button::new("repository-init-jj")
							.ghost()
							.small()
							.label("Initialize JJ with Git")
							.disabled(disabled)
							.on_click(cx.listener(|this, _, window, cx| {
								this.run_repository(RepositoryAction::Init(Backend::Jj), window, cx)
							})),
					),
			);
	}

	body = body.child(div().font_weight(FontWeight::MEDIUM).child("Commit message"))
		.child(Textarea::new(&controls.message).disabled(controls.busy))
		.child(muted("Suggested from successful configuration operations. Edits are kept across refreshes. After editing, refresh the review before committing.", cx));
	if let Some(review) = &controls.review {
		body = body
			.child(div().font_weight(FontWeight::MEDIUM).child("Status"))
			.child(output(
				"repository-status",
				review.status(),
				"Clean working copy.",
			))
			.child(div().font_weight(FontWeight::MEDIUM).child("Full diff"))
			.child(output(
				"repository-diff",
				review.diff(),
				"No changes to commit.",
			))
			.child(
				div()
					.font_weight(FontWeight::MEDIUM)
					.child("Reviewed commit message"),
			)
			.child(output("repository-reviewed-message", review.message(), ""))
			.child(
				Button::new("repository-commit")
					.small()
					.label("Commit reviewed changes locally")
					.disabled(
						disabled
							|| (jj && parent) || review.diff().trim().is_empty()
							|| review.message() != controls.message.read(cx).value().as_str(),
					)
					.on_click(
						cx.listener(|this, _, window, cx| this.commit_repository(window, cx)),
					),
			);
	} else {
		body = body.child(muted(
			"Refresh to show status, the full diff and the final message before committing.",
			cx,
		));
	}

	if controls.detected.is_some() {
		body = body.child(div().font_weight(FontWeight::MEDIUM).child("Push to origin"))
			.child(muted(controls.origin.as_ref().map(|url| format!("origin: {url}")).unwrap_or_else(|| "origin is not configured.".into()), cx))
			.child(muted(if jj { "Enter the exact local bookmark to push. Opt in below to advance nixbox to the last committed change." }
				else { "Enter the exact local branch to push." }, cx))
			.child(Input::new(&controls.branch).disabled(controls.busy));
		if jj {
			body = body.child(
				Button::new("repository-create-bookmark")
					.ghost()
					.small()
					.label("Create/advance nixbox bookmark")
					.selected(controls.create_bookmark)
					.disabled(disabled)
					.on_click(cx.listener(|this, _, _, cx| {
						this.repository.create_bookmark = !this.repository.create_bookmark;
						cx.notify();
					})),
			);
		}
		let branch = controls.branch.read(cx).value();
		body = body.child(
			Button::new("repository-push")
				.small()
				.label("Push named branch/bookmark to origin")
				.disabled(
					disabled
						|| controls.origin.is_none()
						|| branch.trim().is_empty()
						|| (jj && parent) || (jj
						&& controls.create_bookmark
						&& branch.trim() != "nixbox"),
				)
				.on_click(cx.listener(|this, _, window, cx| this.push_repository(window, cx))),
		);
		if controls.origin.is_none() && !parent {
			body = body.child(div().font_weight(FontWeight::MEDIUM).child("Create GitHub repository"))
				.child(muted("Optional. Requires gh installed and authenticated. Creates the repository and adds origin without pushing.", cx))
				.child(Input::new(&controls.owner).disabled(controls.busy))
				.child(Input::new(&controls.name).disabled(controls.busy))
				.child(h_flex().gap_3()
					.child(Button::new("repository-private").ghost().small().label("Private").selected(!controls.public).disabled(disabled)
						.on_click(cx.listener(|this, _, _, cx| { this.repository.public = false; cx.notify(); })))
					.child(Button::new("repository-public").ghost().small().label("Public").selected(controls.public).disabled(disabled)
						.on_click(cx.listener(|this, _, _, cx| { this.repository.public = true; cx.notify(); }))))
				.child(Button::new("repository-create-remote").small().label("Create repository and add origin")
					.disabled(disabled || controls.owner.read(cx).value().trim().is_empty() || controls.name.read(cx).value().trim().is_empty())
					.on_click(cx.listener(|this, _, window, cx| this.create_repository_remote(window, cx))));
		}
	}
	v_flex()
		.size_full()
		.min_w_0()
		.child(page_header(
			"Version control",
			"Review configuration changes. Commit locally, then push separately.",
			None,
			cx,
		))
		.child(
			v_flex()
				.id("repository-scroll")
				.flex_1()
				.min_h_0()
				.min_w_0()
				.overflow_y_scroll()
				.child(body),
		)
		.into_any_element()
}

fn output(id: &'static str, text: &str, empty: &str) -> impl IntoElement {
	// Keep every line reachable vertically and long diff lines reachable horizontally.
	div()
		.id(id)
		.w_full()
		.min_w_0()
		.flex_shrink_0()
		.overflow_x_scroll()
		.text_sm()
		.font_family("monospace")
		.child(if text.is_empty() {
			empty.to_owned()
		} else {
			text.to_owned()
		})
}
