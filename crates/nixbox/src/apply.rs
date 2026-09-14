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
use nixbox_core::{Engine, HOME_FALLBACK_NOTE, Op, Reporter, rebuild::resolve as resolve_rebuild};
use nixbox_nix::build::{BuildEvent, rebuild};
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

    /// Write the configuration changes but skip the rebuild.
    #[arg(long)]
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

pub async fn execute(engine: &mut Engine, plan: Plan, opts: &ApplyOpts) -> Result<ExitCode> {
    for line in &plan.summary {
        eprintln!("  {line}");
    }

    if opts.dry_run {
        eprintln!(
            "Dry run: nothing written. Drop --dry-run to apply and rebuild {}.",
            target_name(plan.scope)
        );
        return Ok(ExitCode::SUCCESS);
    }

    if !confirm(opts, plan.scope)? {
        eprintln!("Cancelled.");
        return Ok(ExitCode::SUCCESS);
    }

    let mut reporter = StderrReporter;
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
        eprintln!("Wrote the configuration. Run `nixbox apply` to rebuild.");
        return Ok(ExitCode::SUCCESS);
    }

    run_rebuild(engine, plan.scope).await
}

/// Asks before changing anything. A non-interactive stdin has to pass
/// `--yes`, so a script can never silently trigger `sudo nixos-rebuild`.
fn confirm(opts: &ApplyOpts, scope: Target) -> Result<bool> {
    if opts.yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        bail!(
            "refusing to change your {} configuration without a terminal to ask at; pass --yes",
            target_name(scope)
        );
    }

    let action = if opts.no_rebuild {
        "Write these changes?"
    } else {
        "Write these changes and rebuild?"
    };
    eprint!("{action} [y/N] ");
    std::io::stderr().flush()?;

    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "Yes"))
}

async fn run_rebuild(engine: &Engine, scope: Target) -> Result<ExitCode> {
    let config_dir = engine.config.home_manager_dir();
    let command = resolve_rebuild(&config_dir, scope).await;
    if command.via_nixos_fallback {
        eprintln!("{HOME_FALLBACK_NOTE}");
    }
    eprintln!("$ {}", command.display());

    let (build_tx, mut build_rx) = mpsc::channel::<BuildEvent>(64);
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let program = command.program.clone();
    let args = command.args.clone();
    let task = tokio::spawn(async move {
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        if let Err(e) = rebuild(&program, &borrowed, build_tx.clone(), cancel_rx).await {
            let _ = build_tx
                .send(BuildEvent::Finished(Err(e.to_string())))
                .await;
        }
    });

    let mut cancel = Some(cancel_tx);
    let mut outcome = Outcome::Succeeded;
    loop {
        tokio::select! {
            event = build_rx.recv() => {
                match event {
                    Some(BuildEvent::Line(line)) => eprintln!("{line}"),
                    Some(BuildEvent::Finished(Ok(()))) => break,
                    Some(BuildEvent::Finished(Err(error))) => {
                        outcome = Outcome::Failed(error);
                        break;
                    }
                    Some(BuildEvent::Cancelled) => {
                        outcome = Outcome::Cancelled;
                        break;
                    }
                    // The sender went away without a verdict; nothing said it
                    // failed, so take the silence as success.
                    None => break,
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

    match &outcome {
        Outcome::Succeeded => eprintln!("Done."),
        Outcome::Cancelled => eprintln!(
            "Cancelled. Your configuration is already written — run `nixbox apply` to finish."
        ),
        Outcome::Failed(error) => {
            eprintln!("Rebuild failed: {error}");
            eprintln!("Your configuration is written; fix the error and run `nixbox apply`.");
        }
    }
    Ok(ExitCode::from(exit_code_for(&outcome)))
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
    use super::{ApplyOpts, EXIT_REBUILD_FAILED, Outcome, confirm, exit_code_for};
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

        assert!(confirm(&opts, Target::NixosSystem).expect("no prompt needed"));
    }

    #[test]
    fn a_non_interactive_run_without_yes_is_refused() {
        // The test harness runs without a terminal on stdin, which is exactly
        // the case this guard exists for.
        let error = confirm(&ApplyOpts::default(), Target::NixosSystem)
            .expect_err("should refuse without a terminal");

        assert!(error.to_string().contains("--yes"), "{error}");
    }
}
