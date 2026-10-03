//! Rendering. The window has top tabs, one page, and a status line; pages
//! are flat lists separated by rules rather than nested panels.

mod flakes;
mod installed;
mod logo;
mod options;
mod packages;
mod queue;
mod repository;
mod settings;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
	AnyElement, App, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
	Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
	prelude::FluentBuilder as _, px,
};

use nixbox_core::vcs::Backend;

use crate::app::{NixboxApp, Page};

impl Render for NixboxApp {
	fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		let page = match self.page {
			Page::Packages => packages::render(self, cx),
			Page::Flakes => flakes::render(self, cx),
			Page::Installed => installed::render(self, cx),
			Page::Options => options::render(self, cx),
			Page::Queue => queue::render_queue(self, cx),
			Page::Build => queue::render_build(self, cx),
			Page::VersionControl => repository::render(self, cx),
			Page::Settings => settings::render(self, cx),
		};
		v_flex()
			.id("nixbox")
			.key_context("Nixbox")
			.on_action(cx.listener(NixboxApp::focus_search))
			.size_full()
			.child(self.navigation(cx))
			.child(div().flex_1().min_h_0().min_w_0().child(page))
			.child(self.status_bar(cx))
			.children(NixboxApp::overlays(window, cx))
	}
}

impl NixboxApp {
	fn navigation(&self, cx: &mut Context<Self>) -> impl IntoElement {
		let queued = self.session.queue.len();
		let building = self.session.is_building();
		let labels = self.session.engine.config.tab_labels;
		let backend = self.repository.backend;
		let pages = [
			("nixpkgs", Page::Packages),
			("Flakes", Page::Flakes),
			("Installed", Page::Installed),
			("Queue", Page::Queue),
			("Build", Page::Build),
			("Version control", Page::VersionControl),
			("Settings", Page::Settings),
		];
		let active_page = if self.page == Page::Options {
			Page::Installed
		} else {
			self.page
		};
		let selected = pages
			.iter()
			.position(|(_, page)| *page == active_page)
			.unwrap_or_default();
		let target = self.session.engine.config.target.label();
		let destinations = pages.each_ref().map(|(_, page)| *page);

		// Equal flexible sides keep the tabs centered in the window. The rule is
		// drawn inside the row, like the tab bar's own, so the two coincide.
		h_flex()
			.relative()
			.w_full()
			.flex_shrink_0()
			.px_6()
			.gap_4()
			.items_center()
			.child(
				div()
					.absolute()
					.left_0()
					.bottom_0()
					.size_full()
					.border_b_1()
					.border_color(cx.theme().border),
			)
			.child(
				div()
					.flex_1()
					.min_w_0()
					.truncate()
					.font_weight(FontWeight::SEMIBOLD)
					.text_color(cx.theme().link)
					.child("NixBox"),
			)
			.child(
				TabBar::new("page-tabs")
					.underline()
					.min_w_0()
					.selected_index(selected)
					.on_click(cx.listener(move |this, index: &usize, window, cx| {
						if let Some(page) = destinations.get(*index) {
							this.set_page(*page, window, cx);
						}
					}))
					.children(pages.map(|(label, page)| {
						Tab::new()
							.aria_label(label)
							.child(
								h_flex()
									.gap_1p5()
									.items_center()
									.when(labels.shows_icons(), |tab| {
										tab.child(page_icon(page, backend))
									})
									.when(labels.shows_names(), |tab| tab.child(label)),
							)
							.when(!labels.shows_names(), |tab| {
								tab.tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
							})
							.when(page == Page::Queue && queued > 0, |tab| {
								tab.suffix(Tag::secondary().small().child(queued.to_string()))
							})
							.when(page == Page::Build && building, |tab| {
								tab.suffix(Spinner::new().small())
							})
					})),
			)
			.child(
				h_flex().flex_1().min_w_0().justify_end().child(
					div()
						.truncate()
						.text_xs()
						.text_color(cx.theme().muted_foreground)
						.child(format!("Installing into {target}")),
				),
			)
	}

	fn status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
		let theme = cx.theme();
		h_flex()
			.h_8()
			.px_4()
			.gap_3()
			.border_t_1()
			.border_color(theme.border)
			.text_sm()
			.text_color(theme.muted_foreground)
			.when(self.searching || self.catalog_loading, |bar| {
				bar.child(Spinner::new().small())
			})
			.child(div().flex_1().truncate().child(self.status.clone()))
			.when_some(self.session.last_error.clone(), |bar, error| {
				bar.child(
					div()
						.max_w_1_2()
						.truncate()
						.text_color(cx.theme().danger)
						.child(format!("Last rebuild failed: {error}")),
				)
				.child(
					Button::new("dismiss-error")
						.ghost()
						.xsmall()
						.label("Dismiss")
						.on_click(cx.listener(|this, _, _, cx| this.dismiss_error(cx))),
				)
			})
	}
}

/// Title, one-line explanation, and optional trailing controls.
fn page_header(
	title: impl Into<SharedString>,
	subtitle: impl Into<SharedString>,
	trailing: Option<AnyElement>,
	cx: &App,
) -> gpui_kit::Div {
	h_flex()
		.px_6()
		.pt_5()
		.pb_3()
		.gap_4()
		.child(
			v_flex()
				.flex_1()
				.gap_1()
				.child(
					div()
						.text_lg()
						.font_weight(FontWeight::SEMIBOLD)
						.text_color(cx.theme().link)
						.child(title.into()),
				)
				.child(
					div()
						.text_sm()
						.text_color(cx.theme().muted_foreground)
						.child(subtitle.into()),
				),
		)
		.children(trailing)
}

/// One list row with a bottom rule.
fn row(cx: &App) -> gpui_kit::Div {
	h_flex()
		.w_full()
		.px_6()
		.py_2()
		.gap_3()
		.border_b_1()
		.border_color(cx.theme().border)
}

/// A line of secondary text.
fn muted(text: impl Into<SharedString>, cx: &App) -> gpui_kit::Div {
	div()
		.text_sm()
		.text_color(cx.theme().muted_foreground)
		.child(text.into())
}

/// Centered text for an empty page or section.
fn empty(text: impl Into<SharedString>, cx: &App) -> gpui_kit::Div {
	v_flex()
		.size_full()
		.items_center()
		.justify_center()
		.child(muted(text, cx))
}

/// Section label inside a page.
fn section(title: impl Into<SharedString>, count: usize, cx: &App) -> gpui_kit::Div {
	h_flex()
		.px_6()
		.pt_4()
		.pb_1()
		.gap_2()
		.text_xs()
		.font_weight(FontWeight::SEMIBOLD)
		.text_color(cx.theme().muted_foreground)
		.child(title.into().to_uppercase())
		.child(count.to_string())
}

/// The navigation icon for `page`; version control shows the repository's logo.
fn page_icon(page: Page, backend: Option<Backend>) -> AnyElement {
	let icon = match page {
		Page::Packages => IconName::Search,
		Page::Flakes => IconName::Github,
		Page::Installed | Page::Options => IconName::HardDrive,
		Page::Queue => IconName::GalleryVerticalEnd,
		Page::Build => IconName::SquareTerminal,
		Page::Settings => IconName::Settings,
		Page::VersionControl => return logo::vcs_logo(backend, px(16.)),
	};
	Icon::new(icon).size_4().into_any_element()
}
