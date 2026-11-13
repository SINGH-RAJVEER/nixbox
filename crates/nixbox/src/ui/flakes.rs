//! Flake search: hits on the left, the selected flake's details on the right.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
	AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
	StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
};
use nixbox_config::Target;
use nixbox_core::{FlakeOutputGroup, flake_output_groups};
use nixbox_nix::flakes::{FlakeDetails, FlakeOutputKind};

use super::{empty, muted, page_header, section};
use crate::app::NixboxApp;

pub fn render(app: &mut NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let header = page_header(
		"Flakes",
		"Find flakes on GitHub and add their package or module to your configuration.",
		None,
		cx,
	);
	let search = div()
		.on_mouse_down(
			gpui_kit::MouseButton::Left,
			cx.listener(|this, _, window, cx| this.enter_insert(window, cx)),
		)
		.px_6()
		.pb_3()
		.child(super::cursor::filter_input(
			app,
			&app.flake_input,
			IconName::Github,
		));

	let hits = if app.flake_results.is_empty() {
		let text = if app.flake_searching {
			"Searching GitHub..."
		} else {
			"Search by project name or by what the flake contains."
		};
		empty(text, cx).into_any_element()
	} else {
		let (active, hover, border) = (
			cx.theme().list_active,
			cx.theme().list_hover,
			cx.theme().border,
		);
		v_flex()
			.id("flake-hits")
			.track_scroll(&app.keyboard.flake_scroll)
			.size_full()
			.overflow_y_scroll()
			.children(app.flake_results.iter().enumerate().map(|(index, hit)| {
				let selected = app.flake_selected == Some(index);
				let packages = hit.packages.len();
				v_flex()
					.id(("flake-hit", index))
					.px_6()
					.py_2()
					.gap_1()
					.border_b_1()
					.border_color(border)
					.cursor_pointer()
					.when(selected, |row| row.bg(active))
					.hover(move |row| row.bg(hover))
					.on_click(cx.listener(move |this, _, _, cx| this.open_flake(index, cx)))
					.child(
						div()
							.font_weight(FontWeight::MEDIUM)
							.child(hit.repo.clone()),
					)
					.child(
						h_flex()
							.gap_1()
							.when(packages > 0, |tags| {
								tags.child(Tag::secondary().xsmall().child(if packages == 1 {
									"1 package".to_string()
								} else {
									format!("{packages} packages")
								}))
							})
							.when(
								hit.modules
									.iter()
									.any(|path| path.starts_with("nixosModules.")),
								|tags| tags.child(Tag::secondary().xsmall().child("NixOS module")),
							)
							.when(
								hit.modules.iter().any(|path| {
									path.starts_with("homeManagerModules.")
										|| path.starts_with("homeModules.")
								}),
								|tags| {
									tags.child(
										Tag::secondary().xsmall().child("Home Manager module"),
									)
								},
							),
					)
			}))
			.into_any_element()
	};

	let details = match (&app.flake_details, app.flake_detail_loading) {
		(Some(details), _) => render_details(
			details,
			app.session.engine.config.target,
			app.flake_picker_open,
			&app.keyboard,
			cx,
		)
		.into_any_element(),
		(None, true) => v_flex()
			.size_full()
			.items_center()
			.justify_center()
			.child(Spinner::new())
			.into_any_element(),
		(None, false) => empty("Select a flake to see what it provides.", cx).into_any_element(),
	};

	v_flex()
		.size_full()
		.child(header)
		.child(search)
		.child(
			h_flex()
				.flex_1()
				.min_h_0()
				.border_t_1()
				.border_color(cx.theme().border)
				.child(div().w_2_5().h_full().child(hits))
				.child(
					div()
						.flex_1()
						.h_full()
						.border_l_1()
						.border_color(cx.theme().border)
						.child(details),
				),
		)
		.into_any_element()
}

fn render_details(
	details: &FlakeDetails,
	target: Target,
	picker_open: bool,
	keyboard: &crate::keyboard::Keyboard,
	cx: &mut Context<NixboxApp>,
) -> impl IntoElement {
	let groups = flake_output_groups(details, target);
	let installable = groups
		.iter()
		.flat_map(|group| &group.items)
		.filter(|item| item.choice.is_some())
		.count();
	let action = match installable {
		0 => muted(format!("Nothing here installs into {}", target.title()), cx).into_any_element(),
		1 => Button::new("install-flake")
			.primary()
			.icon(IconName::Plus)
			.label(format!("Install into {}", target.label()))
			.on_click(cx.listener(|this, _, window, cx| this.install_flake(window, cx)))
			.into_any_element(),
		n => muted(format!("{n} outputs install into {}", target.title()), cx).into_any_element(),
	};
	let highlighted = picker_open.then_some(keyboard.choice);

	v_flex()
		.id("flake-details")
		.track_scroll(&keyboard.details_scroll)
		.size_full()
		.overflow_y_scroll()
		.pb_4()
		.child(
			h_flex()
				.px_6()
				.pt_4()
				.gap_3()
				.child(
					v_flex()
						.flex_1()
						.gap_1()
						.child(
							div()
								.text_lg()
								.font_weight(FontWeight::SEMIBOLD)
								.child(details.repo.clone()),
						)
						.child(muted(
							details
								.description
								.clone()
								.unwrap_or_else(|| "No description.".into()),
							cx,
						)),
				)
				.child(action),
		)
		.child(
			h_flex()
				.px_6()
				.pt_2()
				.gap_2()
				.flex_wrap()
				.child(
					Tag::secondary()
						.xsmall()
						.child(format!("{} stars", details.stars)),
				)
				.when(details.archived, |tags| {
					tags.child(Tag::warning().xsmall().child("archived"))
				})
				.children(
					details
						.topics
						.iter()
						.take(6)
						.map(|topic| Tag::secondary().outline().xsmall().child(topic.clone())),
				),
		)
		.children(
			groups
				.iter()
				.map(|group| render_group(group, highlighted, cx)),
		)
		.when(!details.inputs.is_empty(), |panel| {
			panel
				.child(section("Inputs", details.inputs.len(), cx))
				.child(
					h_flex().px_6().gap_1().flex_wrap().children(
						details
							.inputs
							.iter()
							.map(|input| Tag::secondary().outline().xsmall().child(input.clone())),
					),
				)
		})
}

/// One output family: a header, then a row per output with an Install
/// button when it fits the active target. Leftover top-level outputs, which
/// nothing can install, collapse into a line of tags.
fn render_group(
	group: &FlakeOutputGroup,
	highlighted: Option<usize>,
	cx: &mut Context<NixboxApp>,
) -> AnyElement {
	let header = section(group.title.clone(), group.items.len(), cx).when_some(
		group.unavailable.clone(),
		|header, note| {
			header.child(
				div()
					.font_weight(FontWeight::NORMAL)
					.child(format!("· {note}")),
			)
		},
	);
	if group.kind == FlakeOutputKind::Other {
		return v_flex()
			.child(header)
			.child(
				h_flex().px_6().gap_1().flex_wrap().children(
					group
						.items
						.iter()
						.map(|item| Tag::secondary().outline().xsmall().child(item.name.clone())),
				),
			)
			.into_any_element();
	}

	let (active, muted_color) = (cx.theme().list_active, cx.theme().muted_foreground);
	v_flex()
		.child(header)
		.children(group.items.iter().map(|item| {
			let choice = item.choice;
			h_flex()
				.px_6()
				.py_1()
				.gap_3()
				.items_center()
				.when(choice.is_some() && choice == highlighted, |row| {
					row.bg(active)
				})
				.child(
					h_flex()
						.flex_1()
						.min_w_0()
						.gap_2()
						.child(
							div()
								.font_weight(FontWeight::MEDIUM)
								.when(choice.is_none(), |name| name.text_color(muted_color))
								.child(item.name.clone()),
						)
						.when_some(item.detail.clone(), |row, detail| {
							row.child(muted(detail, cx))
						}),
				)
				.when_some(choice, |row, index| {
					let path = item.path.clone();
					row.child(
						Button::new(("install-flake-output", index))
							.small()
							.ghost()
							.icon(IconName::Plus)
							.label("Install")
							.tooltip(path.clone())
							.on_click(cx.listener(move |this, _, window, cx| {
								this.install_flake_choice(&path, window, cx);
							})),
					)
				})
		}))
		.into_any_element()
}
