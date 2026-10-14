pub(crate) fn main() -> std::process::ExitCode {
	let program = env!("CARGO_BIN_NAME");
	let mut args: Vec<_> = std::env::args_os().collect();
	if program != "nixbox-cli" && args.len() == 1 {
		args.push("--help".into());
	}
	nixbox_cmd::run_from(program, args)
}
