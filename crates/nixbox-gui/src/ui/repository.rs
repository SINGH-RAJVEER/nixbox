//! Version control page. Changes fill the left; commit, push and remote
//! controls sit beside them, so the commit control never hides below a long
//! diff yet only enables for a complete, current review.

use std::ops::Range;

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
	ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::{
	AnyElement, App, Context, FontWeight, InteractiveElement as _, IntoElement,
	ListHorizontalSizingBehavior, ParentElement as _, SharedString,
	StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
	uniform_list,
};
use nixbox_core::vcs::Backend;

use super::logo::{backend_name, vcs_logo};
use super::{empty, muted, section};
use crate::app::NixboxApp;
use crate::repository::{Change, Line, RepositoryAction};

pub fn render(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let controls = &app.repository;
	let body = if controls.detected.is_some() {
		h_flex()
			.flex_1()
			.min_h_0()
			.items_start()
			.border_t_1()
			.border_color(cx.theme().border)
			.child(changes(app, cx))
			.child(actions(app, cx))
			.into_any_element()
	} else if controls.loaded {
		initialize(app, cx)
	} else {
		empty(
			if controls.busy {
				"Looking for a repository..."
			} else {
				"Refresh to look for a repository."
			},
			cx,
		)
		.into_any_element()
	};
	v_flex()
		.size_full()
		.min_w_0()
		.child(header(app, cx))
		.child(notices(app, cx))
		.child(body)
		.into_any_element()
}

fn header(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> impl IntoElement {
	let controls = &app.repository;
	let subtitle = match &controls.detected {
		Some(repo) => format!(
			"{} repository at {}",
			backend_name(repo.backend),
			repo.root.display()
		),
		None => format!("Configuration at {}", app.session.config_dir().display()),
	};
	h_flex()
		.px_6()
		.pt_5()
		.pb_3()
		.gap_3()
		.child(vcs_logo(controls.backend, px(28.)))
		.child(
			v_flex()
				.flex_1()
				.min_w_0()
				.gap_1()
				.child(
					div()
						.text_lg()
						.font_weight(FontWeight::SEMIBOLD)
						.text_color(cx.theme().link)
						.child("Version control"),
				)
				.child(muted(subtitle, cx).truncate()),
		)
		.child(
			Button::new("repository-refresh")
				.outline()
				.small()
				.icon(IconName::RotateCw)
				.label("Refresh review")
				.loading(controls.busy)
				.disabled(controls.busy || app.repository_pending_writes())
				.on_click(cx.listener(|this, _, window, cx| this.refresh_repository(window, cx))),
		)
}

/// The latest result, then anything that currently limits what can be done.
fn notices(app: &NixboxApp, cx: &App) -> impl IntoElement {
	let controls = &app.repository;
	let theme = cx.theme();
	let failed = controls.notice.contains("failed");
	let parent = controls
		.detected
		.as_ref()
		.filter(|repo| repo.root != repo.config_root)
		.map(|repo| match repo.backend {
			Backend::Jj => {
				"This configuration lives inside a larger JJ repository. It can be reviewed, but commit, push and remote creation are refused there."
			}
			Backend::Git => {
				"This configuration lives inside a larger Git repository. Commits cover only this directory, and unrelated staged paths must be unstaged first."
			}
		});
	v_flex()
		.px_6()
		.pb_3()
		.gap_1()
		.text_sm()
		.child(
			h_flex()
				.gap_2()
				.when(controls.busy, |line| line.child(Spinner::new().small()))
				.child(
					div()
						.min_w_0()
						.text_color(if failed {
							theme.danger
						} else {
							theme.muted_foreground
						})
						.child(controls.notice.clone()),
				),
		)
		.when(app.repository_pending_writes(), |notices| {
			notices.child(div().text_color(theme.warning).child(
				"Queued changes or a rebuild are pending. Finish or drop them before reviewing and committing.",
			))
		})
		.when_some(parent, |notices, parent| {
			notices.child(div().text_color(theme.muted_foreground).child(parent))
		})
}

fn initialize(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let disabled = app.repository.busy || app.repository_pending_writes();
	let choice = |id: &'static str, backend: Backend, label: &'static str| {
		Button::new(id)
			.outline()
			.disabled(disabled)
			.child(
				h_flex()
					.gap_2()
					.items_center()
					.child(vcs_logo(Some(backend), px(16.)))
					.child(label),
			)
			.on_click(cx.listener(move |this, _, window, cx| {
				this.run_repository(RepositoryAction::Init(backend), window, cx)
			}))
	};
	v_flex()
		.flex_1()
		.items_center()
		.justify_center()
		.gap_4()
		.border_t_1()
		.border_color(cx.theme().border)
		.child(div().font_weight(FontWeight::MEDIUM).child(
			"This configuration is not under version control.",
		))
		.child(muted(
			"Initialize a repository to review and commit what nixbox changes. Nothing is committed or pushed.",
			cx,
		))
		.child(
			h_flex()
				.gap_3()
				.child(choice("repository-init-git", Backend::Git, "Initialize Git"))
				.child(choice(
					"repository-init-jj",
					Backend::Jj,
					"Initialize Jujutsu with Git",
				)),
		)
		.into_any_element()
}

/// The changed files, then the diff of the focused file or of all of them.
fn changes(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let controls = &app.repository;
	let Some(review) = &controls.review else {
		return empty(
			if controls.busy {
				"Reviewing changes..."
			} else if app.repository_pending_writes() {
				"The review waits until queued changes and rebuilds finish."
			} else {
				"Refresh to review the configuration's changes."
			},
			cx,
		)
		.into_any_element();
	};
	let diff = controls.diff.clone();
	let theme = cx.theme();

	let files = diff.files.iter().enumerate().map(|(index, file)| {
		let selected = controls.focused == Some(index);
		let color = match file.change {
			Change::Added => theme.success,
			Change::Deleted => theme.danger,
			Change::Renamed => theme.info,
			Change::Modified => theme.warning,
		};
		h_flex()
			.id(("repository-file", index))
			.px_6()
			.py_1()
			.gap_3()
			.text_sm()
			.cursor_pointer()
			.when(selected, |row| row.bg(theme.list_active))
			.hover(|row| row.bg(theme.list_hover))
			.child(
				div()
					.w_3()
					.font_weight(FontWeight::SEMIBOLD)
					.text_color(color)
					.child(file.change.letter()),
			)
			.child(div().flex_1().min_w_0().truncate().child(file.path.clone()))
			.child(
				h_flex()
					.gap_2()
					.text_xs()
					.font_family(theme.mono_font_family.clone())
					.when(file.added > 0, |counts| {
						counts.child(
							div()
								.text_color(theme.success)
								.child(format!("+{}", file.added)),
						)
					})
					.when(file.removed > 0, |counts| {
						counts.child(
							div()
								.text_color(theme.danger)
								.child(format!("-{}", file.removed)),
						)
					}),
			)
			.on_click(cx.listener(move |this, _, _, cx| {
				let file = (!selected).then_some(index);
				this.focus_file(file, cx);
			}))
	});

	let focused = controls.focused.and_then(|index| diff.files.get(index));
	let (lines, widest): (Range<usize>, usize) = match focused {
		Some(file) => (file.lines.clone(), file.widest),
		None => (0..diff.lines.len(), diff.widest),
	};
	let start = lines.start;
	let diff_view = if lines.is_empty() {
		empty("No changes to commit. The working copy is clean.", cx).into_any_element()
	} else {
		uniform_list(
			"repository-diff",
			lines.len(),
			cx.processor(move |this, visible: Range<usize>, _, cx| {
				let diff = this.repository.diff.clone();
				visible
					.filter_map(|index| diff.lines.get(start + index))
					.map(|(kind, line)| diff_line(*kind, line.clone(), cx))
					.collect::<Vec<_>>()
			}),
		)
		.with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
		.with_width_from_item(Some(widest - start))
		.track_scroll(&controls.diff_scroll)
		.size_full()
		.font_family(cx.theme().mono_font_family.clone())
		.into_any_element()
	};

	let status_toggle = Button::new("repository-show-status")
		.ghost()
		.xsmall()
		.label(if controls.show_status {
			"Hide status"
		} else {
			"Show status"
		})
		.on_click(cx.listener(|this, _, _, cx| {
			this.repository.show_status = !this.repository.show_status;
			cx.notify();
		}));

	v_flex()
		.flex_1()
		.min_w_0()
		.h_full()
		.child(
			h_flex()
				.pr_4()
				.items_end()
				.child(section("Changes", diff.files.len(), cx).flex_1())
				.when(focused.is_some(), |bar| {
					bar.child(
						Button::new("repository-show-all")
							.ghost()
							.xsmall()
							.label("Show all files")
							.on_click(cx.listener(|this, _, _, cx| this.focus_file(None, cx))),
					)
				})
				.child(status_toggle),
		)
		.child(
			v_flex()
				.id("repository-files")
				.flex_shrink_0()
				.max_h(px(200.))
				.overflow_y_scroll()
				.children(files),
		)
		.when(controls.show_status, |pane| {
			pane.child(
				div()
					.id("repository-status")
					.flex_shrink_0()
					.max_h(px(160.))
					.px_6()
					.py_2()
					.overflow_scroll()
					.whitespace_nowrap()
					.text_xs()
					.font_family(theme.mono_font_family.clone())
					.text_color(theme.muted_foreground)
					.children(
						review
							.status()
							.lines()
							.map(|line| div().child(SharedString::from(line.to_owned())))
							.collect::<Vec<_>>(),
					)
					.when(review.status().trim().is_empty(), |status| {
						status.child("Clean working copy.")
					}),
			)
		})
		.child(
			div()
				.flex_1()
				.min_h_0()
				.mt_2()
				.py_2()
				.border_t_1()
				.border_color(theme.border)
				.text_sm()
				.child(diff_view),
		)
		.into_any_element()
}

fn diff_line(kind: Line, line: SharedString, cx: &App) -> gpui_kit::Div {
	let theme = cx.theme();
	let (color, background) = match kind {
		Line::File => (theme.foreground, Some(theme.muted)),
		Line::Header => (theme.muted_foreground, None),
		Line::Hunk => (theme.info, None),
		Line::Added => (theme.success, Some(theme.success.opacity(0.12))),
		Line::Removed => (theme.danger, Some(theme.danger.opacity(0.12))),
		Line::Context => (theme.foreground, None),
	};
	div()
		.px_4()
		.whitespace_nowrap()
		.text_color(color)
		.when(kind == Line::File, |line| {
			line.font_weight(FontWeight::SEMIBOLD)
		})
		.when_some(background, |line, background| line.bg(background))
		.child(line)
}

/// Commit, push and remote controls. A disabled control says why.
fn actions(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
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
	let message = controls.message.read(cx).value();

	let commit_blocker = if controls.busy {
		None
	} else if pending {
		Some("Finish or drop queued changes first.")
	} else if jj && parent {
		Some("Commits are refused inside a larger JJ repository.")
	} else {
		match &controls.review {
			None => Some("Refresh the review before committing."),
			Some(review) if review.message() != message.as_str() => {
				Some("The message changed. Refresh the review to commit it.")
			}
			Some(review) if review.diff().trim().is_empty() => Some("Nothing to commit."),
			Some(_) => None,
		}
	};
	let files = controls.diff.files.len();
	let commit = Button::new("repository-commit")
		.primary()
		.small()
		.w_full()
		.icon(IconName::Check)
		.label(match (&controls.review, files) {
			(Some(_), 1) => "Commit 1 file locally".to_string(),
			(Some(_), files) if files > 1 => format!("Commit {files} files locally"),
			_ => "Commit locally".to_string(),
		})
		.disabled(disabled || commit_blocker.is_some())
		.on_click(cx.listener(|this, _, window, cx| this.commit_repository(window, cx)));

	let mut panel = v_flex()
		.id("repository-scroll")
		.w(px(380.))
		.flex_shrink_0()
		.h_full()
		.pb_6()
		.border_l_1()
		.border_color(cx.theme().border)
		.overflow_y_scroll()
		.child(heading("Commit", cx))
		.child(
			v_flex()
				.px_6()
				.gap_2()
				.child(Textarea::new(&controls.message).disabled(controls.busy))
				.child(muted(
					"Suggested from nixbox's changes. Your edits are kept across refreshes.",
					cx,
				))
				.child(commit)
				.when_some(commit_blocker, |section, reason| {
					section.child(muted(reason, cx))
				}),
		);

	let branch = controls.branch.read(cx).value();
	let push_blocker = if controls.busy {
		None
	} else if pending {
		Some("Finish or drop queued changes first.")
	} else if jj && parent {
		Some("Pushes are refused inside a larger JJ repository.")
	} else if controls.origin.is_none() {
		Some("Add an origin remote to push.")
	} else if branch.trim().is_empty() {
		Some(if jj {
			"Enter the bookmark to push."
		} else {
			"Enter the branch to push."
		})
	} else if jj && controls.create_bookmark && branch.trim() != "nixbox" {
		Some("Advancing the nixbox bookmark pushes only nixbox.")
	} else {
		None
	};
	panel = panel.child(heading("Push", cx)).child(
		v_flex()
			.px_6()
			.gap_2()
			.child(match &controls.origin {
				Some(url) => h_flex()
					.gap_2()
					.text_sm()
					.child(
						div()
							.text_color(cx.theme().muted_foreground)
							.child("origin"),
					)
					.child(
						div()
							.min_w_0()
							.truncate()
							.font_family(cx.theme().mono_font_family.clone())
							.child(url.clone()),
					),
				None => h_flex().child(muted("No origin remote is configured.", cx)),
			})
			.child(Input::new(&controls.branch).disabled(controls.busy))
			.when(jj, |section| {
				section.child(
					Checkbox::new("repository-create-bookmark")
						.small()
						.label("Create or advance the nixbox bookmark to the last commit")
						.checked(controls.create_bookmark)
						.disabled(disabled)
						.on_click(cx.listener(|this, _: &bool, window, cx| {
							this.toggle_bookmark(window, cx)
						})),
				)
			})
			.child(
				Button::new("repository-push")
					.outline()
					.small()
					.w_full()
					.icon(IconName::ArrowUp)
					.label(if jj {
						"Push bookmark to origin"
					} else {
						"Push branch to origin"
					})
					.disabled(disabled || push_blocker.is_some())
					.on_click(cx.listener(|this, _, window, cx| this.push_repository(window, cx))),
			)
			.when_some(push_blocker, |section, reason| {
				section.child(muted(reason, cx))
			}),
	);

	if controls.origin.is_none() && !parent {
		let named = !controls.owner.read(cx).value().trim().is_empty()
			&& !controls.name.read(cx).value().trim().is_empty();
		panel = panel.child(heading("GitHub remote", cx)).child(
			v_flex()
				.px_6()
				.gap_2()
				.child(muted(
					"Optional. Creates the repository with an authenticated gh and adds it as origin, without pushing.",
					cx,
				))
				.child(
					h_flex()
						.gap_2()
						.child(div().flex_1().child(Input::new(&controls.owner).disabled(controls.busy)))
						.child(muted("/", cx))
						.child(div().flex_1().child(Input::new(&controls.name).disabled(controls.busy))),
				)
				.child(
					ButtonGroup::new("repository-visibility")
						.outline()
						.small()
						.child(
							Button::new("repository-private")
								.label("Private")
								.selected(!controls.public)
								.disabled(disabled),
						)
						.child(
							Button::new("repository-public")
								.label("Public")
								.selected(controls.public)
								.disabled(disabled),
						)
						.on_click(cx.listener(|this, clicked: &Vec<usize>, _, cx| {
							if let Some(index) = clicked.first() {
								this.repository.public = *index == 1;
								cx.notify();
							}
						})),
				)
				.child(
					Button::new("repository-create-remote")
						.outline()
						.small()
						.w_full()
						.icon(IconName::Github)
						.label("Create repository and add origin")
						.disabled(disabled || !named)
						.on_click(
							cx.listener(|this, _, window, cx| this.create_repository_remote(window, cx)),
						),
				),
		);
	}
	panel.into_any_element()
}

/// A section label without a count.
fn heading(title: &'static str, cx: &App) -> gpui_kit::Div {
	h_flex()
		.px_6()
		.pt_4()
		.pb_2()
		.text_xs()
		.font_weight(FontWeight::SEMIBOLD)
		.text_color(cx.theme().muted_foreground)
		.child(title.to_uppercase())
}
