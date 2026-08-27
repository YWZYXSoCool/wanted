//! Execution and rollback.
//!
//! Folds ops: a failure in the staging phase discards the whole staging area; the
//! commit phase (environment write-back) is not atomic, so a failure replays the
//! compensations LIFO via `UndoLog`. No failure leaves a half-finished result.

use crate::Result;
use crate::engine::Ctx;
use crate::engine::ops::Compensation;
use crate::engine::plan::Plan;
use crate::engine::staging::Staging;
use crate::error::Error;
use crate::report::Progress;

/// Accumulates the compensations of executed ops, replayed in reverse on failure.
#[derive(Default)]
pub struct UndoLog {
    entries: Vec<Compensation>,
}

impl UndoLog {
    /// Record one compensation.
    pub fn push(&mut self, compensation: Compensation) {
        self.entries.push(compensation);
    }

    /// Replay all compensations LIFO, collecting the outcome of each step.
    pub fn rollback(&self, ctx: &Ctx) -> RollbackReport {
        let mut report = RollbackReport::default();
        for entry in self.entries.iter().rev() {
            if let Err(e) = entry.undo(ctx.fs, ctx.env) {
                report.failures.push(e.to_string());
            }
        }
        report
    }
}

/// A list of rollback success/failure outcomes.
#[derive(Default)]
pub struct RollbackReport {
    /// Descriptions of compensations that could not be replayed.
    pub failures: Vec<String>,
}

impl RollbackReport {
    /// The first failure; `None` when nothing failed.
    pub fn first_failure(&self) -> Option<&str> {
        self.failures.first().map(String::as_str)
    }
}

/// Execute a plan; any failure rolls back and returns the original error
/// (with rollback-failure details attached).
pub fn execute(plan: &Plan, ctx: &Ctx) -> Result<()> {
    let staging = Staging::from_dir(plan.staging_dir.clone());
    staging.ensure_clean(ctx.fs)?;

    let mut log = UndoLog::default();
    let outcome = run_phases(plan, ctx, &mut log);
    match outcome {
        Ok(()) => {
            staging.abort(ctx.fs)?;
            Ok(())
        }
        Err(original) => {
            let report = log.rollback(ctx);
            let _ = staging.abort(ctx.fs);
            Err(merge_errors(original, report))
        }
    }
}

/// Execute `plans` in order until one succeeds, returning its index. Each plan
/// runs transactionally, so a failing attempt rolls back completely (including
/// environment writes) before the next starts from a clean state. When every
/// attempt fails, the panics-free error aggregates them.
pub fn execute_chain(plans: &[Plan], ctx: &Ctx) -> Result<usize> {
    let mut failures = Vec::new();
    for (index, plan) in plans.iter().enumerate() {
        match execute(plan, ctx) {
            Ok(()) => return Ok(index),
            Err(error) => failures.push(format!("{} attempt {}: {error}", plan.name, index + 1)),
        }
    }
    Err(Error::Other(format!(
        "all install attempts failed: {}",
        failures.join("; ")
    )))
}

fn run_phases(plan: &Plan, ctx: &Ctx, log: &mut UndoLog) -> Result<()> {
    for op in &plan.staged_ops {
        ctx.reporter.report(Progress::Phase(op.label()));
        log.push(op.apply(ctx)?);
    }

    if plan.app_dir != plan.dest_dir {
        let parent = plan.dest_dir.parent().ok_or_else(|| {
            crate::Error::Other(format!("dest has no parent: {}", plan.dest_dir.display()))
        })?;
        ctx.fs.create_dir_all(parent)?;
        ctx.fs.rename(&plan.app_dir, &plan.dest_dir)?;
        log.push(Compensation::RemoveDir(plan.dest_dir.clone()));
    }

    for op in &plan.commit_ops {
        ctx.reporter.report(Progress::Phase(op.label()));
        log.push(op.apply(ctx)?);
    }

    Ok(())
}

fn merge_errors(original: Error, report: RollbackReport) -> Error {
    match report.first_failure() {
        None => original,
        Some(rebound) => Error::Rollback(format!(
            "{original}; additionally rollback failed: {rebound}"
        )),
    }
}
