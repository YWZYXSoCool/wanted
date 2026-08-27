//! Operations and their compensations (the undo actions).
//!
//! Each [`Op::apply`] returns its [`Compensation`] — an "action with a way out"
//! pair. Compensations are composable first-class values; rollback replays them
//! in reverse LIFO order.

use std::path::PathBuf;

use crate::Result;
use crate::engine::fs::Fs;
use crate::engine::{Ctx, unpack};
use crate::env::{self, EnvDelta};
use crate::report::Progress;

/// One executable operation (lazy data, not an action).
#[derive(Clone, Debug)]
pub enum Op {
    /// Download the URL to `to`.
    Download { url: String, to: PathBuf },
    /// Extract the archive at `from` to `to`.
    Unpack { from: PathBuf, to: PathBuf },
    /// Apply the deltas to the environment backend.
    WriteEnv { deltas: Vec<EnvDelta> },
}

impl Op {
    /// Execute the operation and return its compensation.
    pub fn apply(&self, ctx: &Ctx) -> Result<Compensation> {
        match self {
            Op::Download { url, to } => {
                let bytes = ctx.downloader.fetch(url, &mut |done, total| {
                    ctx.reporter.report(Progress::Bytes { done, total });
                })?;
                ctx.fs.write(to, &bytes)?;
                Ok(Compensation::RemoveFile(to.clone()))
            }
            Op::Unpack { from, to } => {
                let bytes = ctx.fs.read(from)?;
                unpack::extract(&bytes, to, ctx.fs)?;
                Ok(Compensation::RemoveDir(to.clone()))
            }
            Op::WriteEnv { deltas } => {
                let snapshots = env::apply_deltas(deltas, ctx.env)?;
                let restores: Vec<Compensation> = snapshots
                    .into_iter()
                    .map(|(name, old)| Compensation::RestoreEnv(RestoreEnv { name, old }))
                    .collect();
                Ok(Compensation::Composite(restores))
            }
        }
    }
}

/// A snapshot-style compensation for one environment variable.
#[derive(Clone, Debug)]
pub struct RestoreEnv {
    /// Variable name.
    pub name: String,
    /// Value before apply; `None` means it did not exist (rollback deletes it).
    pub old: Option<String>,
}

/// A compensation: the inverse of an action, replayed LIFO to roll back.
#[derive(Clone, Debug)]
pub enum Compensation {
    /// Delete one file.
    RemoveFile(PathBuf),
    /// Recursively delete one directory.
    RemoveDir(PathBuf),
    /// Restore a variable's old value.
    RestoreEnv(RestoreEnv),
    /// A group of child compensations (composite).
    Composite(Vec<Compensation>),
    /// No compensation needed (used as a placeholder).
    None,
}

impl Compensation {
    /// Replay this compensation. Missing paths are tolerated so a compensation
    /// never causes a secondary failure.
    pub fn undo(&self, fs: &dyn Fs, env: &dyn crate::env::EnvStore) -> Result<()> {
        match self {
            Compensation::RemoveFile(path) => {
                if fs.exists(path)? {
                    fs.remove_file(path)?;
                }
                Ok(())
            }
            Compensation::RemoveDir(path) => {
                if fs.exists(path)? {
                    fs.remove_dir_all(path)?;
                }
                Ok(())
            }
            Compensation::RestoreEnv(restore) => match &restore.old {
                Some(value) => env.write(&restore.name, value),
                None => env.remove(&restore.name),
            },
            Compensation::Composite(parts) => {
                for part in parts.iter().rev() {
                    part.undo(fs, env)?;
                }
                Ok(())
            }
            Compensation::None => Ok(()),
        }
    }
}
