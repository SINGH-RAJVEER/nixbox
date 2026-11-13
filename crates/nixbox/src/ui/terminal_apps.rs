//! Instructions only: terminal applications are never installed by the GUI.

use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
	AnyElement, ClipboardItem, Context, InteractiveElement as _, IntoElement, ParentElement as _,
	StatefulInteractiveElement as _, Styled as _, div,
};

use super::settings::{section, setting};
use crate::app::NixboxApp;

pub(super) fn render(cx: &mut Context<NixboxApp>) -> AnyElement {
	let mut group = section(
		"Terminal applications",
		"Separate installations. Enable nix-command and flakes, then run a command below. Nothing is installed until you do.",
		false,
		cx,
	)
	.id("terminal-applications");
	for (id, name, command, launch) in [
		(
			"tui",
			"Terminal UI",
			"nix profile add github:SINGH-RAJVEER/nixbox#nixbox-tui",
			"Then run nixbox-tui.",
		),
		(
			"cli",
			"Command line",
			"nix profile add github:SINGH-RAJVEER/nixbox#nixbox-cli",
			"Then run nixbox --help or nb --help. Only this package provides nixbox and nb.",
		),
	] {
		let control = v_flex().gap_1().child(
			h_flex()
				.items_center()
				.gap_3()
				.child(
					div()
						.id(format!("install-command-{id}"))
						.flex_1()
						.min_w_0()
						.overflow_x_scroll()
						.text_sm()
						.font_family(cx.theme().mono_font_family.clone())
						.child(command),
				)
				.child(
					Button::new(format!("copy-terminal-{id}"))
						.small()
						.outline()
						.label("Copy")
						.on_click(move |_, _, cx| {
							cx.write_to_clipboard(ClipboardItem::new_string(command.to_owned()))
						}),
				),
		);
		group = group.child(setting(name, launch, control, cx));
	}
	group.into_any_element()
}
