use std::process::Command;

#[test]
fn aliases_show_help_without_opening_an_interface() {
	for (name, binary) in [
		("nixbox", env!("CARGO_BIN_EXE_nixbox")),
		("nb", env!("CARGO_BIN_EXE_nb")),
	] {
		let bare = Command::new(binary).output().expect("bare CLI");
		let help = Command::new(binary)
			.arg("--help")
			.output()
			.expect("CLI help");
		assert!(bare.status.success());
		assert_eq!(bare.stdout, help.stdout);
		let text = String::from_utf8(help.stdout).expect("help text");
		assert!(text.contains(&format!("Usage: {name}")));
		for flag in ["--gui", "--tui"] {
			assert!(!text.contains(flag));
			let output = Command::new(binary)
				.arg(flag)
				.output()
				.expect("invalid flag");
			assert_eq!(output.status.code(), Some(2));
		}
	}
}
