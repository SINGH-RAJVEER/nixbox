//! Instructions only: terminal applications are never installed by the GUI.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
	AnyElement, ClipboardItem, Context, FontWeight, InteractiveElement as _, IntoElement,
	ParentElement as _, StatefulInteractiveElement as _, Styled as _, div,
};

use super::muted;
use crate::app::NixboxApp;

pub(super) fn render(cx: &mut Context<NixboxApp>) -> AnyElement {
	let mut section = v_flex()
		.id("terminal-applications")
		.flex_shrink_0()
		.px_6()
		.py_4()
		.gap_3()
		.child(div().font_weight(FontWeight::MEDIUM).child("Terminal applications"))
		.child(muted("The GUI, terminal UI, and CLI are separate installations. Choose either terminal application, or install both.", cx))
		.child(muted("1. Enable nix-command and flakes in your Nix configuration.", cx))
		.child(muted("2. Copy the command for the application you want and run it in a terminal.", cx));
	for (id, name, command, launch) in [
		(
			"tui",
			"Terminal UI",
			"nix profile add github:SINGH-RAJVEER/nixbox#nixbox-tui",
			"nixbox-tui",
		),
		(
			"cli",
			"Command line",
			"nix profile add github:SINGH-RAJVEER/nixbox#nixbox-cli",
			"nixbox --help or nb --help",
		),
	] {
		section = section.child(
			v_flex()
				.gap_2()
				.child(div().text_sm().font_weight(FontWeight::MEDIUM).child(name))
				.child(
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
								.ghost()
								.label("Copy command")
								.on_click(move |_, _, cx| {
									cx.write_to_clipboard(ClipboardItem::new_string(
										command.to_owned(),
									))
								}),
						),
				)
				.child(muted(format!("3. After installation, run: {launch}"), cx)),
		);
	}
	section.child(muted("Installing the GUI or TUI does not install nixbox or nb. Those commands belong only to the CLI package. Nothing is downloaded until you run an installation command.", cx)).into_any_element()
}
