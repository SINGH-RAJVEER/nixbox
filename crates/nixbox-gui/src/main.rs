#[cfg(feature = "native")]
mod app;
#[cfg(feature = "native")]
mod askpass;
#[cfg(feature = "native")]
mod model;
#[cfg(feature = "native")]
mod native;
#[cfg(feature = "native")]
mod options;
#[cfg(feature = "native")]
mod theme;
#[cfg(feature = "native")]
mod ui;

#[cfg(not(feature = "native"))]
mod launcher;

#[rustfmt::skip]
fn main() -> std::process::ExitCode {
	#[cfg(feature = "native")]
	return native::main();

	#[cfg(not(feature = "native"))]
	return launcher::main();
}
