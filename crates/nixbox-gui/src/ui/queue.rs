//! The queue of pending ops and the output of the running rebuild.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, FontWeight, IntoElement, ParentElement as _, Styled as _, div,
    prelude::FluentBuilder as _, uniform_list,
};

use super::{empty, muted, page_header, row};
use crate::app::NixboxApp;

pub fn render_queue(app: &mut NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
    let building = app.session.is_building();
    let apply = (!building && !app.session.queue.is_empty()).then(|| {
        Button::new("apply-queue")
            .primary()
            .icon(IconName::Play)
            .label("Apply now")
            .on_click(cx.listener(|this, _, _, cx| this.apply_queue(cx)))
            .into_any_element()
    });
    let header = page_header(
        "Queue",
        "Changes for one target are written and rebuilt together; the other target waits its turn.",
        apply,
        cx,
    );

    let running = app.session.build_label().map(|label| {
        row(cx)
            .child(Spinner::new().small())
            .child(
                div()
                    .flex_1()
                    .font_weight(FontWeight::MEDIUM)
                    .child(label.to_string()),
            )
            .child(muted("running", cx))
    });
    let body = if running.is_none() && app.session.queue.is_empty() {
        empty("Nothing queued.", cx).into_any_element()
    } else {
        v_flex()
            .children(running)
            .children(app.session.queue.iter().enumerate().map(|(index, op)| {
                row(cx).child(div().flex_1().child(op.label())).child(
                    Button::new(("dequeue", index))
                        .small()
                        .ghost()
                        .icon(IconName::Close)
                        .tooltip("Drop from the queue")
                        .on_click(cx.listener(move |this, _, _, cx| this.dequeue(index, cx))),
                )
            }))
            .into_any_element()
    };

    v_flex()
        .size_full()
        .child(header)
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

pub fn render_build(app: &mut NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
    let label = app.session.build_label().map(str::to_string);
    let cancel = label.is_some().then(|| {
        Button::new("cancel-build")
            .danger()
            .outline()
            .label("Cancel")
            .on_click(cx.listener(|this, _, _, cx| this.cancel_build(cx)))
            .into_any_element()
    });
    let header = page_header(
        "Build",
        label.map_or_else(
            || "Output of the most recent rebuild.".to_string(),
            |label| format!("Running: {label}"),
        ),
        cancel,
        cx,
    );

    let body = if app.session.log.is_empty() {
        empty("No rebuild has run yet.", cx).into_any_element()
    } else {
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
        .size_full()
        .into_any_element()
    };

    v_flex()
        .size_full()
        .child(header)
        .child(
            h_flex()
                .flex_1()
                .min_h_0()
                .py_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .font_family(cx.theme().mono_font_family.clone())
                .text_sm()
                .when(app.session.is_building(), |log| log.bg(cx.theme().muted))
                .child(body),
        )
        .into_any_element()
}
