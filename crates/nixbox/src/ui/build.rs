//! The running or most recent rebuild as a list of steps, like a CI run:
//! each step with its state, nix's own counts, and its duration, and one
//! step open at a time to show what it printed.

use std::time::Instant;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
	AnyElement, App, Context, FontWeight, Hsla, InteractiveElement as _, IntoElement,
	ListHorizontalSizingBehavior, ParentElement as _, SharedString,
	StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
	prelude::FluentBuilder as _, px, rems, uniform_list,
};
use nixbox_core::progress::{BuildProgress, StepKind, StepState, duration, size};

use super::{empty, page_header};
use crate::app::NixboxApp;

pub fn render(app: &mut NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let building = app.session.is_building();
	let cancel = building.then(|| {
		Button::new("cancel-build")
			.danger()
			.outline()
			.label("Cancel")
			.on_click(cx.listener(|this, _, _, cx| this.cancel_build(cx)))
			.into_any_element()
	});
	let progress = &app.session.progress;
	let subtitle = if progress.is_empty() {
		"Output of the most recent rebuild.".to_string()
	} else {
		progress.label.clone()
	};
	let header = page_header("Build", subtitle, cancel, cx);

	let body = if !progress.is_empty() {
		steps(app, cx)
	} else if app.session.log.is_empty() {
		empty("No rebuild has run yet.", cx).into_any_element()
	} else {
		plain_log(app, cx)
	};
	v_flex()
		.size_full()
		.child(header)
		.when(!app.session.progress.is_empty(), |page| {
			page.child(overview(&app.session.progress, cx))
		})
		.child(
			v_flex()
				.flex_1()
				.min_h_0()
				.border_t_1()
				.border_color(cx.theme().border)
				.child(body),
		)
		.into_any_element()
}

/// Where the rebuild is, how long it has taken, and the bar. The bar only
/// moves with nix's own counts; evaluation and activation have none, so it
/// runs indeterminate through them rather than guess.
fn overview(progress: &BuildProgress, cx: &App) -> impl IntoElement {
	let theme = cx.theme();
	let now = Instant::now();
	let running = progress.is_running();
	let failed = progress.failed_step().is_some();
	let cancelled = progress
		.steps
		.iter()
		.any(|step| step.state == StepState::Cancelled);
	let fraction = progress.fraction();
	let (headline, color) = if running {
		let doing = match progress.current() {
			Some(StepKind::Prepare) | None => "Preparing",
			Some(StepKind::Evaluate) => "Evaluating configuration",
			Some(StepKind::Download | StepKind::Build) => match (
				progress.step(StepKind::Build).map(|step| step.state),
				progress.step(StepKind::Download).map(|step| step.state),
			) {
				(Some(StepState::Running), Some(StepState::Running)) => "Downloading and building",
				(Some(StepState::Running), _) => "Building",
				_ => "Downloading",
			},
			Some(StepKind::Activate) => "Activating",
		};
		(doing.to_string(), theme.foreground)
	} else if cancelled {
		("Cancelled".to_string(), theme.muted_foreground)
	} else if failed {
		("Failed".to_string(), theme.danger)
	} else {
		("Succeeded".to_string(), theme.success)
	};
	let percent = fraction
		.filter(|_| running)
		.map(|fraction| format!("{:.0}%", fraction * 100.));
	let elapsed = progress.started.map(|started| {
		duration(
			progress
				.ended
				.unwrap_or(now)
				.saturating_duration_since(started),
		)
	});

	let builds = progress.builds();
	let downloads = progress.downloads();
	let mut counts = Vec::new();
	if builds.expected > 0 {
		counts.push(format!("{} of {} built", builds.done, builds.expected));
	}
	if downloads.expected > 0 {
		let (done, expected) = progress.download_bytes();
		let mut text = format!(
			"{} of {} paths downloaded",
			downloads.done, downloads.expected
		);
		if expected > 0 {
			text.push_str(&format!(" ({} of {})", size(done), size(expected)));
		}
		counts.push(text);
	}

	let bar_color = if failed {
		theme.danger
	} else if cancelled {
		theme.muted_foreground
	} else if running {
		theme.progress_bar
	} else {
		theme.success
	};
	// A finished rebuild fills the bar; one that stopped short shows how far
	// nix's counts got.
	let value = if running || failed || cancelled {
		fraction.unwrap_or(0.) * 100.
	} else {
		100.
	};
	v_flex()
		.px_6()
		.pb_4()
		.gap_2()
		.child(
			h_flex()
				.gap_2()
				.text_sm()
				.child(
					div()
						.font_weight(FontWeight::MEDIUM)
						.text_color(color)
						.child(headline),
				)
				.children(
					percent.map(|percent| div().text_color(theme.muted_foreground).child(percent)),
				)
				.child(div().flex_1())
				.children(elapsed.map(|elapsed| {
					div()
						.font_family(theme.mono_font_family.clone())
						.text_color(theme.muted_foreground)
						.child(elapsed)
				})),
		)
		.child(
			Progress::new("build-progress")
				.small()
				.color(bar_color)
				.value(value)
				.loading(running && fraction.is_none()),
		)
		.when(!counts.is_empty(), |overview| {
			overview.child(
				div()
					.text_xs()
					.text_color(theme.muted_foreground)
					.child(counts.join("  ·  ")),
			)
		})
}

/// The steps in order. The open one fills the space left with its log.
fn steps(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let progress = &app.session.progress;
	let expanded = app.expanded_step();
	let now = Instant::now();
	let mut list = v_flex().id("build-steps").size_full();
	for step in progress.steps.iter().filter(|step| step.shown()) {
		let open = expanded == Some(step.kind);
		list = list.child(step_row(progress, step.kind, open, now, cx));
		if step.state == StepState::Failed
			&& let Some(error) = &progress.error
		{
			list = list.child(
				div()
					.pl(rems(4.25))
					.pr_6()
					.pb_2()
					.text_sm()
					.text_color(cx.theme().danger)
					.child(error.lines().next().unwrap_or_default().to_string()),
			);
		}
		if open {
			list = list.child(step_log(app, step.kind, cx));
		}
	}
	list.into_any_element()
}

fn step_row(
	progress: &BuildProgress,
	kind: StepKind,
	open: bool,
	now: Instant,
	cx: &mut Context<NixboxApp>,
) -> impl IntoElement {
	let theme = cx.theme();
	let Some(step) = progress.step(kind) else {
		return div().into_any_element();
	};
	let mut summary = progress.summary(kind);
	if kind == StepKind::Build {
		let building: Vec<String> = progress
			.building()
			.into_iter()
			.map(|(name, phase)| match phase {
				Some(phase) => format!("{name} ({phase})"),
				None => name.to_string(),
			})
			.collect();
		if !building.is_empty() {
			summary = Some(format!(
				"{}, building {}",
				summary.unwrap_or_default(),
				building.join(", ")
			));
		}
	}
	let fraction = match kind {
		StepKind::Build => progress.builds().fraction(),
		StepKind::Download => {
			let (done, expected) = progress.download_bytes();
			if expected > 0 {
				Some(done as f32 / expected as f32)
			} else {
				progress.downloads().fraction()
			}
		}
		_ => None,
	};
	let hover = theme.list_hover;
	h_flex()
		.id(("build-step", kind as usize))
		.flex_shrink_0()
		.px_6()
		.py_2()
		.gap_3()
		.text_sm()
		.cursor_pointer()
		.when(open, |row| row.bg(theme.list_active))
		.hover(move |row| row.bg(hover))
		.on_click(cx.listener(move |this, _, _, cx| this.toggle_build_step(kind, cx)))
		.child(
			Icon::new(if open {
				IconName::ChevronDown
			} else {
				IconName::ChevronRight
			})
			.size_3p5()
			.text_color(theme.muted_foreground),
		)
		.child(state_icon(step.state, cx))
		.child(
			div()
				.flex_shrink_0()
				.font_weight(FontWeight::MEDIUM)
				.when(step.state == StepState::Pending, |name| {
					name.text_color(theme.muted_foreground)
				})
				.child(kind.label()),
		)
		.child(
			div()
				.flex_1()
				.min_w_0()
				.truncate()
				.text_color(theme.muted_foreground)
				.children(summary),
		)
		.when_some(
			fraction.filter(|_| step.state == StepState::Running),
			|row, fraction| {
				row.child(
					div().w(rems(7.5)).flex_shrink_0().child(
						Progress::new(("build-step-progress", kind as usize))
							.xsmall()
							.value(fraction * 100.),
					),
				)
			},
		)
		.child(
			div()
				.w(rems(4.5))
				.flex_shrink_0()
				.text_right()
				.font_family(theme.mono_font_family.clone())
				.text_color(theme.muted_foreground)
				.children(step.elapsed(now).map(duration)),
		)
		.test_support()
		.into_any_element()
}

fn state_icon(state: StepState, cx: &App) -> AnyElement {
	let theme = cx.theme();
	let icon = |name: IconName, color: Hsla| {
		Icon::new(name)
			.size_4()
			.text_color(color)
			.into_any_element()
	};
	match state {
		StepState::Running => Spinner::new().small().into_any_element(),
		StepState::Succeeded => icon(IconName::CircleCheck, theme.success),
		StepState::Failed => icon(IconName::CircleX, theme.danger),
		StepState::Cancelled => icon(IconName::CircleX, theme.muted_foreground),
		StepState::Skipped => icon(IconName::Minus, theme.muted_foreground),
		// An empty ring: still to come.
		StepState::Pending => div()
			.size_4()
			.flex_shrink_0()
			.p(px(1.5))
			.child(
				div()
					.size_full()
					.rounded_full()
					.border_1()
					.border_color(theme.muted_foreground),
			)
			.into_any_element(),
	}
}

/// What one step printed, numbered from its first line.
fn step_log(app: &NixboxApp, kind: StepKind, cx: &mut Context<NixboxApp>) -> AnyElement {
	let theme = cx.theme();
	let Some(step) = app.session.progress.step(kind) else {
		return div().into_any_element();
	};
	if step.lines.is_empty() {
		return div()
			.flex_shrink_0()
			.pl(rems(4.25))
			.py_2()
			.text_sm()
			.text_color(theme.muted_foreground)
			.child(if step.state == StepState::Running {
				"Nothing printed yet."
			} else {
				"Nothing printed."
			})
			.into_any_element();
	}
	let widest = step
		.lines
		.iter()
		.enumerate()
		.max_by_key(|(_, line)| line.len())
		.map_or(0, |(index, _)| index);
	let digits = (step.dropped + step.lines.len()).to_string().len();
	let first = step.dropped;
	div()
		.id("build-step-output")
		.flex_1()
		.min_h_0()
		.py_1()
		.bg(theme.muted.opacity(0.5))
		.font_family(theme.mono_font_family.clone())
		.text_xs()
		.child(
			uniform_list(
				"build-step-log",
				step.lines.len(),
				cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
					let Some(step) = this.session.progress.step(kind) else {
						return Vec::new();
					};
					let muted = cx.theme().muted_foreground;
					range
						.filter_map(|index| {
							let line = step.lines.get(index)?;
							Some(
								h_flex()
									.min_w_full()
									.pl(rems(2.5))
									.pr_6()
									.gap_4()
									.whitespace_nowrap()
									.child(
										div()
											.w(rems(0.5 * digits as f32 + 0.5))
											.flex_shrink_0()
											.text_right()
											.text_color(muted)
											.child((first + index + 1).to_string()),
									)
									.child(SharedString::from(line.clone())),
							)
						})
						.collect::<Vec<_>>()
				}),
			)
			.with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
			.with_width_from_item(Some(widest))
			.track_scroll(&app.log_scroll)
			.size_full(),
		)
		.test_support()
		.into_any_element()
}

/// Engine notes with no rebuild behind them, such as a manifest that could
/// not be written.
fn plain_log(app: &NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	div()
		.size_full()
		.py_2()
		.font_family(cx.theme().mono_font_family.clone())
		.text_sm()
		.child(
			uniform_list(
				"build-log",
				app.session.log.len(),
				cx.processor(|this, range: std::ops::Range<usize>, _, _| {
					range
						.filter_map(|index| {
							let line = this.session.log.get(index)?;
							Some(div().px_6().whitespace_nowrap().child(line.clone()))
						})
						.collect::<Vec<_>>()
				}),
			)
			.track_scroll(&app.log_scroll)
			.size_full(),
		)
		.into_any_element()
}
