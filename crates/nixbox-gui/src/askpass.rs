//! The password prompt sudo shows through `SUDO_ASKPASS`.
//!
//! sudo reads the password from this program's stdout and treats a non-zero
//! exit as the user declining, so the rebuild stops without retrying.

use std::io::Write as _;
use std::process::ExitCode;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Root, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, QuitMode,
    Render, SharedString, Styled as _, Subscription, Window, WindowBounds, WindowOptions, px, size,
};

/// Asks for the password in a small window and hands it to sudo.
pub fn run(prompt: &str) -> ExitCode {
    let prompt: SharedString = if prompt.trim().is_empty() {
        "Password".into()
    } else {
        prompt.trim().to_string().into()
    };
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        // Closing the window has to end the process, or sudo waits forever.
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            let bounds = Bounds::centered(None, size(px(420.), px(190.)), cx);
            cx.spawn(async move |cx| {
                let opened = cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        app_id: Some("nixbox".into()),
                        is_resizable: false,
                        ..WindowOptions::default()
                    },
                    |window, cx| {
                        let view = cx.new(|cx| Askpass::new(prompt, window, cx));
                        cx.new(|cx| Root::new(view, window, cx))
                    },
                );
                if opened.is_err() {
                    finish(None);
                }
            })
            .detach();
        });
    // The window closed without an answer.
    ExitCode::FAILURE
}

/// Prints the password for sudo and exits, or exits non-zero on cancel.
/// Exiting here rather than returning keeps gpui from tearing down first.
fn finish(password: Option<&str>) -> ! {
    match password {
        Some(password) => {
            let mut stdout = std::io::stdout();
            let _ = writeln!(stdout, "{password}");
            let _ = stdout.flush();
            std::process::exit(0)
        }
        None => std::process::exit(1),
    }
}

struct Askpass {
    prompt: SharedString,
    input: Entity<InputState>,
    _subscription: Subscription,
}

impl Askpass {
    fn new(prompt: SharedString, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).masked(true));
        input.update(cx, |input, cx| input.focus(window, cx));
        let subscription = cx.subscribe_in(&input, window, |this, _, event, _, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.submit(cx);
            }
        });
        window.set_window_title("NixBox needs your password");
        Self {
            prompt,
            input,
            _subscription: subscription,
        }
    }

    fn submit(&self, cx: &mut Context<Self>) {
        finish(Some(&self.input.read(cx).value()));
    }
}

impl Render for Askpass {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .p_5()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        gpui_kit::div()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child("A system rebuild needs root"),
                    )
                    .child(
                        gpui_kit::div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.prompt.clone()),
                    ),
            )
            .child(Input::new(&self.input).mask_toggle())
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel")
                            .label("Cancel")
                            .on_click(|_, _, _| finish(None)),
                    )
                    .child(
                        Button::new("ok")
                            .primary()
                            .label("Authenticate")
                            .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                    ),
            )
    }
}
