//! nixpkgs search: a search box over a virtualized list of hits.

use gpui_kit::component::button::Button;
use gpui_kit::component::input::Input;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, FontWeight, IntoElement, ParentElement as _, Styled as _, div,
    uniform_list,
};

use super::{empty, muted, page_header, row};
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
    let search = div().px_6().pb_3().child(
        Input::new(&app.search_input)
            .prefix(IconName::Search)
            .cleanable(true),
    );

    let body = if app.results.is_empty() {
        let text = if app.searching {
            "Searching..."
        } else if app.search_input.read(cx).value().trim().is_empty() {
            "Type to search packages. Ctrl-F focuses the search from anywhere."
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
                            row(cx)
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
        .flex_1()
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
