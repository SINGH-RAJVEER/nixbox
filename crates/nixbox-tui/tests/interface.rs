use std::process::Command;

#[test]
fn tui_does_not_include_cli_commands_or_interface_switches() {
	for args in [
		&["search", "firefox"][..],
		&["install", "hello"],
		&["--gui"],
		&["--tui"],
	] {
		let output = Command::new(env!("CARGO_BIN_EXE_nixbox-tui"))
			.args(args)
			.output()
			.expect("TUI arguments");
		assert_eq!(output.status.code(), Some(2));
	}
}
