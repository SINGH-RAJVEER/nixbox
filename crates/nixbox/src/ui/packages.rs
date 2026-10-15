//! nixpkgs search: a search box over a virtualized list of hits.

use gpui_kit::component::button::Button;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
	AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
	StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, uniform_list,
};

use super::{empty, hover_row, muted, page_header};
use crate::app::NixboxApp;
use crate::model::installed_scopes;

pub fn render(app: &mut NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let target = app.session.engine.config.target.label();
	let header = page_header(
		"nixpkgs",
		format!("Search your locked nixpkgs and install into {target}."),
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
			&app.search_input,
			IconName::Search,
		));

	let body = if app.results.is_empty() {
		let text = if app.searching {
			"Searching..."
		} else if app.search_input.read(cx).value().trim().is_empty() {
			"Type to search packages."
		} else {
			"No packages match."
		};
		empty(text, cx).into_any_element()
	} else {
		uniform_list(
			"search-results",
			app.results.len(),
			cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
				range
					.filter_map(|index| {
						let hit = this.results.get(index)?;
						let installed = installed_scopes(&this.session.engine, &hit.attr);
						Some(
							hover_row(cx)
								.id(("package-row", index))
								.when(this.keyboard.selected == index, |row| {
									row.bg(cx.theme().list_active)
								})
								.on_click(cx.listener(move |this, _, window, cx| {
									this.select_row(index, window, cx)
								}))
								.child(
									v_flex()
										.flex_1()
										.min_w_0()
										.child(
											h_flex()
												.gap_2()
												.child(
													div()
														.font_weight(FontWeight::MEDIUM)
														.child(hit.attr.clone()),
												)
												.child(muted(hit.version.clone(), cx))
												.children(installed.iter().map(|scope| {
													Tag::success()
														.xsmall()
														.child(format!("in {scope}"))
												})),
										)
										.child(muted(hit.description.clone(), cx).truncate()),
								)
								.child(
									Button::new(("install", index))
										.small()
										.outline()
										.icon(IconName::Plus)
										.label("Install")
										.on_click(cx.listener(move |this, _, window, cx| {
											this.install(index, window, cx);
										})),
								),
						)
					})
					.collect::<Vec<_>>()
			}),
		)
		.track_scroll(&app.keyboard.package_scroll)
		.size_full()
		.into_any_element()
	};

	v_flex()
		.size_full()
		.child(header)
		.child(search)
		.child(
			div()
				.flex_1()
				.min_h_0()
				.border_t_1()
				.border_color(cx.theme().border)
				.child(body),
		)
		.into_any_element()
}
