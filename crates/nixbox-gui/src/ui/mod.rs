//! Rendering. The window is a sidebar, one page, and a status line; pages
//! are flat lists separated by rules rather than nested panels.

mod flakes;
mod installed;
mod options;
mod packages;
mod queue;
mod settings;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::sidebar::{Sidebar, SidebarHeader, SidebarMenu, SidebarMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Window, div, prelude::FluentBuilder as _,
};

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
            Page::Settings => settings::render(self, cx),
        };
        h_flex()
            .id("nixbox")
            .key_context("Nixbox")
            .on_action(cx.listener(NixboxApp::focus_search))
            .size_full()
            .child(self.sidebar(cx))
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .child(div().flex_1().min_h_0().child(page))
                    .child(self.status_bar(cx)),
            )
            .children(NixboxApp::overlays(window, cx))
    }
}

impl NixboxApp {
    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let queued = self.session.queue.len();
        let building = self.session.is_building();
        let item = |label: &'static str, icon: IconName, page: Page| {
            SidebarMenuItem::new(label)
                .icon(icon)
                .active(
                    self.page == page || (page == Page::Installed && self.page == Page::Options),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.set_page(page, window, cx)))
        };
        let queue = item("Queue", IconName::GalleryVerticalEnd, Page::Queue)
            .when(queued > 0, |item| {
                item.suffix(move |_, _| Tag::secondary().small().child(queued.to_string()))
            });
        let build = item("Build", IconName::SquareTerminal, Page::Build)
            .when(building, |item| item.suffix(|_, _| Spinner::new().small()));
        let target = self.session.engine.config.target.label();

        Sidebar::new("sidebar")
            .collapsible(false)
            .header(
                SidebarHeader::new().child(
                    v_flex()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("NixBox"))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("Installing into {target}")),
                        ),
                ),
            )
            .child(
                SidebarMenu::new()
                    .child(item("nixpkgs", IconName::Search, Page::Packages))
                    .child(item("Flakes", IconName::Github, Page::Flakes))
                    .child(item("Installed", IconName::HardDrive, Page::Installed))
                    .child(queue)
                    .child(build)
                    .child(item("Settings", IconName::Settings, Page::Settings)),
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
