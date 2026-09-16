//! `nixbox completions` — shell completion scripts.
//!
//! The script is generated for the name the binary was installed as, so a
//! `nixbox-cli` install completes `nixbox-cli` rather than a command the
//! user does not have.

use clap::CommandFactory;
use clap_complete::{Shell, generate};

use crate::cli::Cli;
use crate::program;

pub fn run(shell: Shell) {
    let name = program();
    let mut command = Cli::command().name(name).bin_name(name);
    generate(shell, &mut command, name, &mut std::io::stdout());
}
