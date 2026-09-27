//! Desktop front-end for nixbox.
//!
//! The same binary is also sudo's askpass helper: a system rebuild runs
//! `sudo -A` with `SUDO_ASKPASS` pointing back here, and sudo starts this
//! program with the password prompt as its only argument.

use std::os::unix::process::CommandExt as _;
use std::process::{Command, ExitCode};

use anyhow::{Context as _, Result};
use gpui_kit::component::Root;
use gpui_kit::{
    App, AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};
use nixbox_core::rebuild::{ASKPASS_ENV, ASKPASS_LIBRARY_PATH_ENV};
use nixbox_core::{Engine, Escalation, Session};
use tracing_subscriber::EnvFilter;

use crate::app::{self, NixboxApp};
use crate::askpass;

pub(crate) fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    if std::env::var_os(ASKPASS_ENV).is_some() {
        restore_library_path();
        let prompt = std::env::args().nth(1).unwrap_or_default();
        return askpass::run(&prompt);
    }

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("nixbox-gui: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Runs this program again with the `LD_LIBRARY_PATH` sudo stripped, so the
/// password window can load Vulkan and the windowing libraries. The loader
/// only reads the variable at startup, which is why setting it is not enough.
fn restore_library_path() {
    let Some(saved) = std::env::var_os(ASKPASS_LIBRARY_PATH_ENV) else {
        return;
    };
    if std::env::var_os("LD_LIBRARY_PATH").as_ref() == Some(&saved) {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let error = Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env("LD_LIBRARY_PATH", saved)
        .exec();
    eprintln!("nixbox-gui: restarting with the library path failed: {error}");
}

fn run() -> Result<()> {
    // gpui drives the window on its own executor; everything in nixbox-nix
    // is tokio, so it gets a runtime of its own for the life of the app.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("nixbox-worker")
        .build()
        .context("starting the async runtime")?;
    let handle = runtime.handle().clone();

    let engine = Engine::load()?;
    let helper = std::env::current_exe().context("locating the nixbox-gui executable")?;
    let (session, build_rx) = Session::new(engine);
    let session = session
        .with_runtime(handle.clone())
        .with_escalation(Escalation::Askpass(helper));

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            app::bind_keys(cx);
            let bounds = Bounds::centered(None, size(px(1180.), px(760.)), cx);
            cx.spawn(async move |cx| {
                let opened = cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        app_id: Some("nixbox-gui".into()),
                        titlebar: Some(TitlebarOptions {
                            title: Some("NixBox".into()),
                            ..TitlebarOptions::default()
                        }),
                        ..WindowOptions::default()
                    },
                    |window, cx| {
                        let view = cx.new(|cx| {
                            let mut app = NixboxApp::new(session, build_rx, handle, window, cx);
                            app.prepare_catalog(cx);
                            app
                        });
                        cx.new(|cx| Root::new(view, window, cx))
                    },
                );
                if let Err(error) = opened {
                    eprintln!("nixbox-gui: opening the window failed: {error:#}");
                    cx.update(|cx| cx.quit());
                }
            })
            .detach();
        });

    drop(runtime);
    Ok(())
}
