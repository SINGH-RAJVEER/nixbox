//! Reviewed configuration commits, without loading an engine.

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{IsTerminal, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use nixbox_config::Config;
use nixbox_core::vcs::{Backend, Review, Vcs};

use crate::cli::GlobalArgs;

#[derive(Args, Debug, Default)]
pub struct CommitArgs {
	/// Edit the proposed message using EDITOR, a single executable path.
	#[arg(long)]
	pub edit: bool,
	/// Show the review without changing anything. Unavailable for JJ.
	#[arg(long)]
	pub dry_run: bool,
	/// Explicitly approve the displayed review without a prompt.
	#[arg(long, short = 'y')]
	pub yes: bool,
}

pub fn run(args: &CommitArgs, global: &GlobalArgs) -> Result<ExitCode> {
	ensure!(
		!global.json,
		"commit uses text reviews; --json is not supported"
	);
	let mut config = Config::load_or_default()?;
	global.apply(&mut config);
	execute(args, &Vcs::new(&config)?)
}

fn execute(args: &CommitArgs, vcs: &Vcs) -> Result<ExitCode> {
	// Core JJ reviews snapshot the working copy. Refuse rather than mutate
	// repository metadata during a dry run.
	if args.dry_run && vcs.repository()?.backend == Backend::Jj {
		bail!("JJ commit dry-run is unavailable: the core review API snapshots the working copy");
	}
	let edited;
	let message = if args.edit && !args.dry_run {
		let initial = vcs.suggested_message()?;
		edited = edit_message(&initial)?;
		Some(edited.as_str())
	} else {
		if args.edit {
			eprintln!("Dry run: editor skipped; showing the unedited message.");
		}
		None
	};
	let review = vcs.review(message)?;
	show_review(&mut std::io::stdout().lock(), &review)?;
	if approve(args, "Commit exactly this review?")? {
		let outcome = vcs.commit(&review)?;
		println!("Committed {}.", outcome.revision);
		if let Some(warning) = outcome.warning {
			eprintln!("Warning: {warning}");
		}
	}
	Ok(ExitCode::SUCCESS)
}

fn show_review(output: &mut impl Write, review: &Review) -> Result<()> {
	writeln!(
		output,
		"Repository: {:?} {}",
		review.repository().backend,
		review.repository().root.display()
	)?;
	for (label, text) in [
		("Status", review.status()),
		("Diff", review.diff()),
		("Message", review.message()),
	] {
		writeln!(output, "--- {label} ---")?;
		output.write_all(text.as_bytes())?;
		// Separators are outside the exact review text, including its final newline.
		writeln!(output, "\n--- End {label} ---")?;
	}
	output.flush()?;
	Ok(())
}

fn approve(args: &CommitArgs, question: &str) -> Result<bool> {
	std::io::stdout().flush()?;
	if args.dry_run {
		eprintln!("Dry run: nothing changed.");
		return Ok(false);
	}
	if args.yes {
		return Ok(true);
	}
	ensure!(
		std::io::stdin().is_terminal(),
		"refusing commit without a terminal to ask at; pass --yes"
	);
	eprint!("{question} [y/N] ");
	std::io::stderr().flush()?;
	let mut answer = String::new();
	std::io::stdin().read_line(&mut answer)?;
	let approved = matches!(answer.trim(), "y" | "Y" | "yes" | "Yes");
	if !approved {
		eprintln!("Cancelled.");
	}
	Ok(approved)
}

struct MessageFile(PathBuf);

impl Drop for MessageFile {
	fn drop(&mut self) {
		let _ = fs::remove_file(&self.0);
	}
}

fn edit_message(initial: &str) -> Result<String> {
	let editor = std::env::var_os("EDITOR")
		.context("--edit requires EDITOR to name one executable, without arguments")?;
	edit_with(&editor, initial)
}

fn edit_with(editor: &OsStr, initial: &str) -> Result<String> {
	ensure!(!editor.is_empty(), "EDITOR cannot be empty");
	static NEXT: AtomicU64 = AtomicU64::new(0);
	let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
	let path = std::env::temp_dir().join(format!(
		"nixbox-message-{}-{stamp}-{}.txt",
		std::process::id(),
		NEXT.fetch_add(1, Ordering::Relaxed)
	));
	let mut file = OpenOptions::new()
		.write(true)
		.create_new(true)
		.mode(0o600)
		.open(&path)
		.context("creating temporary commit message")?;
	let temporary = MessageFile(path);
	file.write_all(initial.as_bytes())?;
	file.sync_all()?;
	drop(file);
	let status = Command::new(editor).arg(&temporary.0).status().context(
		"running EDITOR directly; use one executable path without arguments or shell syntax",
	)?;
	ensure!(status.success(), "editor failed; no commit made");
	let message =
		fs::read_to_string(&temporary.0).context("reading edited UTF-8 commit message")?;
	ensure!(!message.trim().is_empty(), "commit message cannot be blank");
	Ok(message)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::cli::Cli;
	use clap::Parser;

	struct TempRoot(PathBuf);

	impl Drop for TempRoot {
		fn drop(&mut self) {
			let _ = fs::remove_dir_all(&self.0);
		}
	}

	fn temporary_root() -> TempRoot {
		let stamp = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_nanos();
		let path =
			std::env::temp_dir().join(format!("nixbox-cmd-commit-{}-{stamp}", std::process::id()));
		fs::create_dir(&path).unwrap();
		TempRoot(path)
	}

	#[test]
	fn commit_is_top_level_and_accepts_edit_and_review_flags() {
		for binary in ["nixbox", "nixbox-cli"] {
			let cli = Cli::try_parse_from([binary, "commit"]).unwrap();
			assert!(matches!(
				cli.command,
				Some(crate::cli::Command::Commit(CommitArgs {
					edit: false,
					dry_run: false,
					yes: false,
				}))
			));
			for approval in ["--yes", "-y"] {
				let cli = Cli::try_parse_from([binary, "commit", "--edit", "--dry-run", approval])
					.unwrap();
				assert!(matches!(
					cli.command,
					Some(crate::cli::Command::Commit(CommitArgs {
						edit: true,
						dry_run: true,
						yes: true,
					}))
				));
			}
			for option in ["-m", "--message"] {
				assert!(Cli::try_parse_from([binary, "commit", option, "custom"]).is_err());
			}
			assert!(Cli::try_parse_from([binary, "vcs"]).is_err());
		}
	}

	#[test]
	fn dry_run_overrides_yes_and_noninteractive_commits_require_approval() {
		assert!(
			!approve(
				&CommitArgs {
					dry_run: true,
					yes: true,
					..CommitArgs::default()
				},
				"Test?"
			)
			.unwrap()
		);
		assert!(
			approve(
				&CommitArgs {
					dry_run: false,
					yes: true,
					..CommitArgs::default()
				},
				"Test?"
			)
			.unwrap()
		);
		if !std::io::stdin().is_terminal() {
			assert!(
				approve(&CommitArgs::default(), "Test?")
					.unwrap_err()
					.to_string()
					.contains("--yes")
			);
		}
	}

	#[test]
	fn git_dry_runs_preserve_files_index_and_head_and_render_exact_review() {
		if Command::new("git").arg("--version").output().is_err() {
			eprintln!("SKIP Git integration test: git is not installed");
			return;
		}
		let root = temporary_root();
		let vcs = Vcs::for_root(&root.0).unwrap();
		vcs.init(Backend::Git).unwrap();
		fs::write(root.0.join("home.nix"), "manual configuration\n").unwrap();
		let message = "subject\n\nExact body, with trailing spaces  \n";
		let review = vcs.review(Some(message)).unwrap();
		let mut rendered = Vec::new();
		show_review(&mut rendered, &review).unwrap();
		let rendered = String::from_utf8(rendered).unwrap();
		for (label, text) in [
			("Status", review.status()),
			("Diff", review.diff()),
			("Message", message),
		] {
			assert!(rendered.contains(&format!("--- {label} ---\n{text}\n--- End {label} ---")));
		}
		execute(
			&CommitArgs {
				edit: true,
				dry_run: true,
				yes: true,
			},
			&vcs,
		)
		.unwrap();
		assert_eq!(vcs.review(Some(message)).unwrap(), review);
		assert!(!root.0.join(".git/index").exists());
		assert!(
			!Command::new("git")
				.current_dir(&root.0)
				.args(["rev-parse", "--verify", "HEAD"])
				.output()
				.unwrap()
				.status
				.success()
		);
		assert_eq!(
			fs::read_to_string(root.0.join("home.nix")).unwrap(),
			"manual configuration\n"
		);
	}

	#[test]
	fn jj_commit_dry_run_refuses_without_snapshotting() {
		if Command::new("jj").arg("--version").output().is_err() {
			return;
		}
		let root = temporary_root();
		let vcs = Vcs::for_root(&root.0).unwrap();
		vcs.init(Backend::Jj).unwrap();
		let operation = || {
			let output = Command::new("jj")
				.current_dir(&root.0)
				.args([
					"--ignore-working-copy",
					"op",
					"log",
					"--no-graph",
					"--limit",
					"1",
					"-T",
					"id",
				])
				.output()
				.unwrap();
			assert!(
				output.status.success(),
				"{}",
				String::from_utf8_lossy(&output.stderr)
			);
			output.stdout
		};
		let before = operation();
		fs::write(root.0.join("manual.nix"), "unsnapshotted change\n").unwrap();
		let error = execute(
			&CommitArgs {
				edit: true,
				dry_run: true,
				yes: true,
			},
			&vcs,
		)
		.unwrap_err();
		assert!(error.to_string().contains("snapshots"));
		assert_eq!(operation(), before);
	}

	#[test]
	fn editor_failure_blank_message_and_shell_syntax_are_refused() {
		let message = "exact\n\nbody  \n";
		assert_eq!(edit_with(OsStr::new("true"), message).unwrap(), message);
		assert!(
			edit_with(OsStr::new("false"), message)
				.unwrap_err()
				.to_string()
				.contains("editor failed")
		);
		assert!(edit_with(OsStr::new("true"), " \n").is_err());
		assert!(edit_with(OsStr::new("true; false"), message).is_err());
	}
}
