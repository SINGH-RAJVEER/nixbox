//! Desktop front-end for nixbox.
//!
//! nixbox-gui opens the independently installed desktop window.
//! The binary is also sudo's askpass
//! helper: a system rebuild runs `sudo -A` with `SUDO_ASKPASS` pointing back
//! here, and sudo starts this program with the password prompt as its only
//! argument.

use std::os::unix::process::CommandExt as _;
use std::process::{Command, ExitCode};

use anyhow::{Context as _, Result};
use clap::Parser as _;
use gpui_kit::component::Root;
use gpui_kit::{
	App, AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};
use nixbox_config::{Config, Target};
use nixbox_core::rebuild::{ASKPASS_ENV, ASKPASS_LIBRARY_PATH_ENV};
use nixbox_core::{Engine, Escalation, Session};
use tracing_subscriber::EnvFilter;

use crate::app::{self, NixboxApp};
use crate::askpass;

#[derive(clap::Parser)]
#[command(
	version,
	about = "NixBox desktop GUI",
	after_help = "Terminal applications are separate installs. See Settings > Terminal applications for flake installation steps."
)]
struct Args {
	#[arg(long, short = 't', value_parser = ["home-manager", "hm", "home", "nixos", "system", "nixos-system"])]
	target: Option<String>,
	#[arg(long, short = 'c')]
	channel: Option<String>,
}

pub(crate) fn main() -> ExitCode {
	// Before the command line is parsed: sudo passes the prompt text as the
	// only argument, which is no subcommand.
	if std::env::var_os(ASKPASS_ENV).is_some() {
		tracing_subscriber::fmt()
			.with_env_filter(EnvFilter::from_default_env())
			.with_writer(std::io::stderr)
			.init();
		restore_library_path();
		let prompt = std::env::args().nth(1).unwrap_or_default();
		return askpass::run(&prompt);
	}

	let args = Args::parse();
	tracing_subscriber::fmt()
		.with_env_filter(
			EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("error")),
		)
		.with_writer(std::io::stderr)
		.init();
	match run(args) {
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
	eprintln!("nixbox: restarting with the library path failed: {error}");
}

fn run(args: Args) -> Result<()> {
	// gpui drives the window on its own executor; everything in nixbox-nix
	// is tokio, so it gets a runtime of its own for the life of the app.
	let runtime = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.thread_name("nixbox-worker")
		.build()
		.context("starting the async runtime")?;
	let handle = runtime.handle().clone();

	let mut config = Config::load_or_default()?;
	if let Some(target) = args.target {
		config.target = if matches!(target.as_str(), "home-manager" | "hm" | "home") {
			Target::HomeManager
		} else {
			Target::NixosSystem
		};
	}
	if let Some(channel) = args.channel {
		config.channel = channel;
	}
	let engine = Engine::with_config(config)?;
	let helper = std::env::current_exe().context("locating the nixbox executable")?;
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
						app_id: Some("nixbox".into()),
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
					eprintln!("nixbox: opening the window failed: {error:#}");
					cx.update(|cx| cx.quit());
				}
			})
			.detach();
		});

	drop(runtime);
	Ok(())
}
