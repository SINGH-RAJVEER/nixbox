//! The queue of pending ops and the output of the running rebuild.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, v_flex};
use gpui_kit::{
	AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
	StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
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
			.id("queue-rows")
			.size_full()
			.overflow_y_scroll()
			.track_scroll(&app.keyboard.queue_scroll)
			.children(running)
			.children(app.session.queue.iter().enumerate().map(|(index, op)| {
				row(cx)
					.when(app.keyboard.selected == index, |row| {
						row.bg(cx.theme().list_active)
					})
					.child(div().flex_1().child(op.label()))
					.child(
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
