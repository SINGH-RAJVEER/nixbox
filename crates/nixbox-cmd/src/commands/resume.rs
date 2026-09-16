//! `nixbox resume` — finish work a previous run left behind.
//!
//! Both front-ends write the same `state.json`, so this picks up an
//! interrupted rebuild or a queue left by the TUI, and the TUI picks up
//! whatever the CLI leaves.

use std::collections::BTreeMap;
use std::process::ExitCode;

use anyhow::Result;
use clap::Args;
use nixbox_config::Target;
use nixbox_core::{Op, PersistedState};

use crate::apply::{ApplyOpts, Plan, execute, run_rebuild};
use crate::cli::GlobalArgs;
use crate::commands::target_name;

#[derive(Args, Debug)]
pub struct ResumeArgs {
    /// Throw the saved work away instead of finishing it.
    #[arg(long, conflicts_with_all = ["dry_run", "no_rebuild"])]
    pub discard: bool,

    #[command(flatten)]
    pub apply: ApplyOpts,
}

pub async fn run(args: &ResumeArgs, global: &GlobalArgs) -> Result<ExitCode> {
    let Some(saved) = PersistedState::load().filter(|state| !state.is_empty()) else {
        eprintln!("Nothing to resume.");
        return Ok(ExitCode::SUCCESS);
    };

    if args.discard {
        PersistedState::clear()?;
        eprintln!("Discarded the saved work.");
        return Ok(ExitCode::SUCCESS);
    }

    if let Some(error) = &saved.last_error {
        eprintln!("The previous run reported: {error}");
    }

    // An interrupted rebuild had its files written before it started, so the
    // configuration is already correct and only the rebuild has to run again.
    // The queue is still worth reporting afterwards, so a skipped rebuild
    // falls through rather than returning.
    if let Some(in_progress) = &saved.in_progress {
        eprintln!(
            "Interrupted rebuild of {}: {}",
            target_name(in_progress.scope),
            in_progress.label
        );
        if args.apply.dry_run || args.apply.no_rebuild {
            eprintln!("  the configuration is already written; skipping the rebuild.");
        } else {
            let engine = global.engine()?;
            let code = run_rebuild(&engine, in_progress.scope, &in_progress.label).await?;
            clear_in_progress()?;
            if code != ExitCode::SUCCESS {
                return Ok(code);
            }
        }
    }

    if saved.pending_queue.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }

    // Ops of different scopes cannot share a rebuild, so they are applied one
    // scope at a time, in the order the queue first mentions each.
    let mut batches: BTreeMap<usize, (Target, Vec<Op>)> = BTreeMap::new();
    let mut order: Vec<Target> = Vec::new();
    for op in saved.pending_queue {
        let scope = op.scope();
        let index = order
            .iter()
            .position(|seen| *seen == scope)
            .unwrap_or_else(|| {
                order.push(scope);
                order.len().saturating_sub(1)
            });
        batches
            .entry(index)
            .or_insert_with(|| (scope, Vec::new()))
            .1
            .push(op);
    }

    let mut last = ExitCode::SUCCESS;
    for (scope, ops) in batches.into_values() {
        let summary: Vec<String> = ops.iter().map(Op::label).collect();
        let mut engine = global.engine()?;
        last = execute(
            &mut engine,
            Plan {
                scope,
                ops,
                summary,
            },
            &args.apply,
        )
        .await?;
        if last != ExitCode::SUCCESS {
            return Ok(last);
        }
        if !args.apply.dry_run {
            drop_applied_scope(scope)?;
        }
    }
    Ok(last)
}

/// Clears the interrupted rebuild without touching a queue the TUI may have
/// saved alongside it.
fn clear_in_progress() -> Result<()> {
    let mut state = PersistedState::load().unwrap_or_default();
    state.in_progress = None;
    state.last_error = None;
    state.save()
}

/// Drops the ops that were just applied, leaving any other scope queued.
fn drop_applied_scope(scope: Target) -> Result<()> {
    let mut state = PersistedState::load().unwrap_or_default();
    state.pending_queue.retain(|op| op.scope() != scope);
    state.last_error = None;
    state.save()
}
