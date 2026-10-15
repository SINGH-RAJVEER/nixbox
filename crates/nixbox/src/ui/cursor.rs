//! Vim cursors for the filter inputs. The input draws its own line caret,
//! which is the insert mode cursor. Normal mode covers it with a block over
//! the character under the cursor, redrawn in the background color, and
//! visual mode outlines the character the selection moves.

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, IconName};
use gpui_kit::{
	AnyElement, App, BorderStyle, Bounds, Entity, EntityInputHandler as _, Focusable as _, Hsla,
	IntoElement, ParentElement as _, Pixels, Styled as _, TextAlign, Window, canvas, div, fill,
	outline, point, size,
};
use nixbox_config::InputMode;

use crate::app::NixboxApp;
use crate::keyboard::Mode;

#[derive(Clone, Copy)]
enum Shape {
	Block,
	Outline,
}

/// A page's search field, with the cursor shape of the current Vim mode.
pub fn filter_input(app: &NixboxApp, state: &Entity<InputState>, icon: IconName) -> AnyElement {
	let input = Input::new(state)
		.readonly(app.filter_readonly())
		.prefix(icon)
		.cleanable(true);
	let shape = match (app.session.engine.config.input_mode, app.keyboard.mode) {
		(InputMode::Vim, Mode::Normal) => Shape::Block,
		(InputMode::Vim, Mode::Visual) => Shape::Outline,
		_ => return input.into_any_element(),
	};
	let state = state.clone();
	let target = app.vim_cursor();
	// The input sizes its text with `text_sm`; the overlay inherits the same.
	div()
		.relative()
		.text_sm()
		.child(input)
		.child(
			canvas(
				|_, _, _| {},
				move |_, _, window, cx| paint(&state, shape, target, window, cx),
			)
			.absolute()
			.size_full(),
		)
		.into_any_element()
}

fn paint(
	state: &Entity<InputState>,
	shape: Shape,
	target: usize,
	window: &mut Window,
	cx: &mut App,
) {
	if !window.is_window_active() || !state.read(cx).focus_handle(cx).is_focused(window) {
		return;
	}
	let Some((caret, _)) = state.read(cx).cursor_layout() else {
		return;
	};
	let value = state.read(cx).value();
	let caret_offset = state.read(cx).cursor().min(value.len());
	// Normal mode sits on the character after the caret; visual mode on the
	// character the selection last moved to.
	let start = match shape {
		Shape::Block => caret_offset,
		Shape::Outline => byte_offset(&value, target),
	};
	let end = value[start..]
		.chars()
		.next()
		.map_or(start, |ch| start + ch.len_utf8());

	let span = |from: usize, to: usize, window: &mut Window, cx: &mut App| {
		width(state, &value, from..to, window, cx)
	};
	let x = if start >= caret_offset {
		caret.origin.x + span(caret_offset, start, window, cx)
	} else {
		caret.origin.x - span(start, caret_offset, window, cx)
	};
	let style = window.text_style();
	let font_size = style.font_size.to_pixels(window.rem_size());
	let glyph = |text: &str, color: Hsla, window: &mut Window| {
		let run = gpui_kit::TextRun {
			color,
			..style.to_run(text.len())
		};
		window
			.text_system()
			.shape_line(text.to_string().into(), font_size, &[run], None)
	};
	let char_width = match span(start, end, window, cx) {
		width if end > start => width,
		// Past the last character: as wide as a digit, like a terminal.
		_ => glyph("0", Hsla::default(), window).width,
	};
	let bounds = Bounds::new(
		point(x, caret.origin.y),
		size(char_width, caret.size.height),
	);
	let theme = cx.theme();
	let (caret_color, background, field) =
		(theme.caret, theme.background, theme.input_background());
	match shape {
		Shape::Block => {
			window.paint_quad(fill(bounds, caret_color));
			if end > start {
				let line = glyph(&value[start..end], background, window);
				let _ = line.paint(
					bounds.origin,
					bounds.size.height,
					TextAlign::Left,
					None,
					window,
					cx,
				);
			}
		}
		Shape::Outline => {
			// The input keeps its line caret at the selection's end; paint the
			// field over it so only the box marks the cursor.
			window.paint_quad(fill(caret, background));
			window.paint_quad(fill(caret, field));
			window.paint_quad(outline(bounds, caret_color, BorderStyle::Solid));
		}
	}
}

/// The width of the text between two byte offsets.
fn width(
	state: &Entity<InputState>,
	value: &str,
	range: std::ops::Range<usize>,
	window: &mut Window,
	cx: &mut App,
) -> Pixels {
	let range = utf16(value, range.start)..utf16(value, range.end);
	state
		.update(cx, |state, cx| {
			state.bounds_for_range(range, Bounds::default(), window, cx)
		})
		.map(|bounds| bounds.size.width)
		.unwrap_or_default()
}

fn byte_offset(value: &str, index: usize) -> usize {
	value
		.char_indices()
		.nth(index)
		.map_or(value.len(), |(offset, _)| offset)
}

fn utf16(value: &str, offset: usize) -> usize {
	value[..offset].encode_utf16().count()
}

#[cfg(test)]
mod tests {
	use gpui_kit::component::Root;
	use gpui_kit::test::TestWindowExt as _;
	use gpui_kit::{AppContext as _, TestAppContext, px, size};
	use nixbox_config::Config;
	use nixbox_core::{Engine, Session};
	use nixbox_nix::manifest::Manifest;

	use super::*;

	#[gpui_kit::test]
	async fn every_vim_mode_paints_its_cursor(cx: &mut TestAppContext) {
		cx.update(gpui_kit::init);
		let runtime = tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.expect("runtime");
		let config = Config {
			input_mode: InputMode::Vim,
			..Config::default()
		};
		let (session, build_rx) = Session::new(Engine::from_parts(
			config,
			Manifest::default(),
			Manifest::default(),
			Vec::new(),
		));
		let session = session
			.without_persistence()
			.with_runtime(runtime.handle().clone());
		let mut app = None;
		let handle = cx.open_window(size(px(1000.), px(700.)), |window, cx| {
			let view = cx
				.new(|cx| NixboxApp::new(session, build_rx, runtime.handle().clone(), window, cx));
			app = Some(view.clone());
			Root::new(view, window, cx)
		});
		let app = app.expect("view");
		// An inactive window hides every cursor.
		cx.update_window(handle.into(), |_, window, _| window.activate_window())
			.expect("window");
		cx.run_until_parked();
		cx.update_window(handle.into(), |_, window, cx| {
			app.update(cx, |app, cx| {
				app.enter_insert(window, cx);
				app.search_input
					.update(cx, |input, cx| input.set_value("ripgrep", window, cx));
			});
			window.render_frame(cx);
			for mode in [Mode::Normal, Mode::Visual, Mode::Insert] {
				app.update(cx, |app, _| app.keyboard.mode = mode);
				window.render_frame(cx);
			}
			app.update(cx, |app, cx| {
				app.search_input
					.update(cx, |input, cx| input.set_value("", window, cx));
				app.keyboard.mode = Mode::Normal;
			});
			window.render_frame(cx);
		})
		.expect("window");
	}
}
