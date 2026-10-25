//! Shared execution path for every command that changes the user's config.
//!
//! Writing happens before the rebuild, deliberately: if the rebuild is
//! interrupted the files already reflect the intent, so recovering is just
//! `nixbox apply`.

use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Args;
use nixbox_config::Target;
use nixbox_core::rebuild::LogFormat;
use nixbox_core::telemetry::{self, Telemetry};
use nixbox_core::{
	Activation, Engine, Escalation, InProgress, Op, PersistedState, Reporter, rebuild,
};
use nixbox_nix::build::BuildEvent;
use tokio::sync::{mpsc, oneshot};

use crate::cli::EXIT_FAILURE;
use crate::commands::target_name;

/// Exit status when the configuration was written but the rebuild failed.
/// Distinct from a plain failure so scripts can tell "nixbox could not do it"
/// from "nix said no".
pub const EXIT_REBUILD_FAILED: u8 = 3;

#[derive(Args, Debug, Clone, Default)]
pub struct ApplyOpts {
	/// Print what would change and exit without touching anything.
	#[arg(long)]
	pub dry_run: bool,

	/// Write manifests but skip profile updates and configuration activation.
	#[arg(long, visible_alias = "no-activate")]
	pub no_rebuild: bool,

	/// Do not ask before changing the configuration.
	#[arg(long, short = 'y')]
	pub yes: bool,
}

/// What a command decided to do, ready to be confirmed and executed.
pub struct Plan {
	pub scope: Target,
	pub ops: Vec<Op>,
	/// One line per change, shown before the prompt.
	pub summary: Vec<String>,
}

/// Sends engine notes straight to stderr, so a long rebuild shows its work.
struct StderrReporter;

impl Reporter for StderrReporter {
	fn info(&mut self, msg: String) {
		eprintln!("  {msg}");
	}

	fn warn(&mut self, msg: String) {
		eprintln!("  Warning: {msg}");
	}
}

pub async fn execute(
	engine: &mut Engine,
	plan: Plan,
	opts: &ApplyOpts,
	telemetry: &Telemetry,
) -> Result<ExitCode> {
	execute_with(engine, plan, opts, None, telemetry).await
}

/// [`execute`], with `activation` overriding the one the ops call for.
pub async fn execute_with(
	engine: &mut Engine,
	plan: Plan,
	opts: &ApplyOpts,
	activation: Option<Activation>,
	telemetry: &Telemetry,
) -> Result<ExitCode> {
	let activation = activation.unwrap_or_else(|| engine.activation_for(&plan.ops));
	let action = match activation {
		Activation::Packages => format!("update the {} package profile", plan.scope.label()),
		Activation::Configuration => format!("rebuild {}", target_name(plan.scope)),
	};
	for line in &plan.summary {
		eprintln!("  {line}");
	}

	if opts.dry_run {
		eprintln!("Dry run: nothing written. Drop --dry-run to {action}.");
		return Ok(ExitCode::SUCCESS);
	}

	if !confirm_action(opts, plan.scope, &action)? {
		eprintln!("Cancelled.");
		return Ok(ExitCode::SUCCESS);
	}

	let mut reporter = StderrReporter;
	if let Some(saved) = PersistedState::load() {
		saved.ensure_context(&engine.config)?;
	}
	for op in &plan.ops {
		engine.apply(op, &mut reporter)?;
	}
	if plan.ops.is_empty() {
		// `nixbox apply` rewrites the managed file even with nothing queued,
		// so a hand-edited or deleted file is restored before the rebuild.
		let managed = engine.write_manifest(plan.scope)?;
		reporter.info(format!("Wrote {}.", managed.path().display()));
	}

	if opts.no_rebuild {
		eprintln!(
			"Wrote the configuration. Run `{}{} --target {} {}` to finish.",
			crate::program(),
			selection_flags(&engine.config),
			plan.scope.label(),
			retry_command(activation)
		);
		return Ok(ExitCode::SUCCESS);
	}

	let label = match plan.summary.as_slice() {
		[only] => only.clone(),
		rest => format!("{} change(s)", rest.len()),
	};
	run_activation(engine, plan.scope, &label, activation, telemetry).await
}

/// Asks before changing anything. A non-interactive stdin has to pass
/// `--yes`, so a script can never silently trigger `sudo nixos-rebuild`.
fn confirm_action(opts: &ApplyOpts, scope: Target, action: &str) -> Result<bool> {
	if opts.yes {
		return Ok(true);
	}
	if !std::io::stdin().is_terminal() {
		bail!(
			"refusing to change your {} configuration without a terminal to ask at; pass --yes",
			target_name(scope)
		);
	}

	let prompt = if opts.no_rebuild {
		"Write these changes?".into()
	} else {
		format!("Write these changes and {action}?")
	};
	eprint!("{prompt} [y/N] ");
	std::io::stderr().flush()?;

	let mut answer = String::new();
	std::io::stdin().read_line(&mut answer)?;
	Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "Yes"))
}

/// Runs the rebuild, recording it as in progress first.
///
/// The configuration is already written by the time this is called, so a
/// process that dies here leaves files that are correct and a system that has
/// not been switched. Recording it is what lets `nixbox resume` — or the TUI —
/// pick that up instead of leaving it to be noticed by accident.
pub(crate) async fn run_activation(
	engine: &Engine,
	scope: Target,
	label: &str,
	activation: Activation,
	telemetry: &Telemetry,
) -> Result<ExitCode> {
	let attempt = telemetry.begin_activation(scope, activation);
	if let Some(saved) = PersistedState::load() {
		saved.ensure_context(&engine.config)?;
	}
	mark_in_progress(&engine.config, scope, label, activation);
	let (build_tx, mut build_rx) = mpsc::channel::<BuildEvent>(64);
	let (cancel_tx, cancel_rx) = oneshot::channel();
	let task = tokio::spawn(rebuild::run_activation_for(
		engine.config.clone(),
		scope,
		activation,
		Escalation::Terminal,
		LogFormat::Plain,
		build_tx,
		cancel_rx,
	));

	let mut cancel = Some(cancel_tx);
	let mut outcome = Outcome::Succeeded;
	let mut terminal_outcome = None;
	loop {
		tokio::select! {
			event = build_rx.recv() => {
				match event {
					Some(BuildEvent::Line(line)) => eprintln!("{line}"),
					Some(BuildEvent::Nix(log)) => {
						for line in log.plain_lines() {
							eprintln!("{line}");
						}
					}
					Some(BuildEvent::Finished(Ok(()))) => {
						terminal_outcome = Some(telemetry::Outcome::Succeeded);
						break;
					}
					Some(BuildEvent::Finished(Err(error))) => {
						terminal_outcome = Some(telemetry::Outcome::Failed);
						outcome = Outcome::Failed(error);
						break;
					}
					Some(BuildEvent::Cancelled) => {
						terminal_outcome = Some(telemetry::Outcome::Cancelled);
						outcome = Outcome::Cancelled;
						break;
					}
					None => {
						outcome = Outcome::Failed("activation ended without a verdict".into());
						break;
					}
				}
			}
			_ = tokio::signal::ctrl_c(), if cancel.is_some() => {
				eprintln!("Cancelling; waiting for nix to stop.");
				if let Some(tx) = cancel.take() {
					let _ = tx.send(());
				}
			}
		}
	}
	let _ = task.await;
	if let (Some(attempt), Some(outcome)) = (attempt, terminal_outcome) {
		attempt.finish(outcome);
	}
	let retry = format!(
		"{} --target {} {}",
		selection_flags(&engine.config),
		scope.label(),
		retry_command(activation)
	);

	match &outcome {
		Outcome::Succeeded => {
			finish_in_progress(None);
			eprintln!("Done.");
		}
		Outcome::Cancelled => {
			finish_in_progress(None);
			eprintln!(
				"Cancelled. Your manifests are already written — run `{} {retry}` to finish.",
				crate::program()
			);
		}
		Outcome::Failed(error) => {
			finish_in_progress(Some(error.clone()));
			eprintln!("Activation failed: {error}");
			eprintln!(
				"Your manifests are written; fix the error and run `{} {retry}`.",
				crate::program()
			);
		}
	}
	Ok(ExitCode::from(exit_code_for(&outcome)))
}

/// The subcommand that repeats `activation` from the files on disk.
fn retry_command(activation: Activation) -> &'static str {
	match activation {
		Activation::Packages => "apply",
		Activation::Configuration => "apply --configuration",
	}
}

fn selection_flags(config: &nixbox_config::Config) -> String {
	if config.native_modules_dir.is_none() {
		return String::new();
	}
	if let Some(output) = &config.home_configuration {
		return format!(" --home-configuration '{}'", output.replace('\'', "'\\''"));
	}
	let mut flags = String::new();
	for (flag, value) in [
		("--host", &config.nixos_host),
		("--home-user", &config.home_user),
	] {
		if let Some(value) = value {
			flags.push_str(&format!(" {flag} '{}'", value.replace('\'', "'\\''")));
		}
	}
	flags
}

/// Records the rebuild about to start, leaving any queue the TUI saved alone.
///
/// Failing to write the file is not worth stopping a rebuild over; it only
/// costs the ability to resume one that gets killed.
fn mark_in_progress(
	config: &nixbox_config::Config,
	scope: Target,
	label: &str,
	activation: Activation,
) {
	let mut state = PersistedState::load().unwrap_or_default();
	state.context = Some(config.identity());
	state.in_progress = Some(InProgress {
		scope,
		label: label.to_string(),
		activation,
	});
	state.last_error = None;
	let _ = state.save();
}

/// Clears the record once the rebuild has reached a verdict, keeping the error
/// so the next run can explain what happened.
fn finish_in_progress(error: Option<String>) {
	let mut state = PersistedState::load().unwrap_or_default();
	state.in_progress = None;
	state.last_error = error;
	let _ = state.save();
}

/// How a rebuild ended.
#[derive(Debug)]
enum Outcome {
	Succeeded,
	Cancelled,
	Failed(String),
}

/// A failed rebuild is worth its own exit code: it means nixbox did its part
/// and nix refused, which a script may well want to retry rather than treat
/// as a usage error.
fn exit_code_for(outcome: &Outcome) -> u8 {
	match outcome {
		Outcome::Succeeded => 0,
		Outcome::Cancelled => EXIT_FAILURE,
		Outcome::Failed(_) => EXIT_REBUILD_FAILED,
	}
}

#[cfg(test)]
mod tests {
	use super::{ApplyOpts, EXIT_REBUILD_FAILED, Outcome, confirm_action, exit_code_for};
	use nixbox_config::Target;

	#[test]
	fn each_ending_maps_to_its_own_exit_code() {
		assert_eq!(exit_code_for(&Outcome::Succeeded), 0);
		assert_eq!(exit_code_for(&Outcome::Cancelled), 1);
		assert_eq!(
			exit_code_for(&Outcome::Failed("boom".into())),
			EXIT_REBUILD_FAILED
		);
	}

	#[test]
	fn yes_skips_the_prompt_entirely() {
		let opts = ApplyOpts {
			yes: true,
			..ApplyOpts::default()
		};

		assert!(
			confirm_action(&opts, Target::NixosSystem, "update the profile")
				.expect("no prompt needed")
		);
	}

	#[test]
	fn a_non_interactive_run_without_yes_is_refused() {
		// The test harness runs without a terminal on stdin, which is exactly
		// the case this guard exists for.
		let error = confirm_action(
			&ApplyOpts::default(),
			Target::NixosSystem,
			"update the profile",
		)
		.expect_err("should refuse without a terminal");

		assert!(error.to_string().contains("--yes"), "{error}");
	}
}
