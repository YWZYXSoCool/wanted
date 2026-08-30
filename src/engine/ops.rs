//! Operations and their compensations (the undo actions).
//!
//! Each [`Op::apply`] returns its [`Compensation`] — an "action with a way out"
//! pair. Compensations are composable first-class values; rollback replays them
//! in reverse LIFO order.

use std::path::{Path, PathBuf};

use crate::Result;
use crate::Version;
use crate::engine::fs::Fs;
use crate::engine::{Ctx, Url, unpack};
use crate::env::{self, EnvDelta, EnvVar};
use crate::plugin::raw::RawCommand;
use crate::report::Progress;

/// One executable operation (lazy data, not an action).
#[derive(Clone, Debug)]
pub enum Op {
    /// Download the URL to `to`.
    Download { url: Url, to: PathBuf },
    /// Extract the archive at `from` to `to`.
    Unpack { from: PathBuf, to: PathBuf },
    /// Run the executable at `exe` as a silent installer writing into `base`.
    RunInstaller {
        /// The downloaded installer executable.
        exe: PathBuf,
        /// Silent-install arguments (placeholders already expanded).
        args: Vec<String>,
        /// The target install directory in the run directory.
        base: PathBuf,
    },
    /// Apply the deltas to the environment backend.
    WriteEnv { deltas: Vec<EnvDelta> },
    /// Run external install commands in fallback order, writing into `base`.
    RunCommand {
        /// Fully-expanded command lines, tried in order until one succeeds.
        commands: Vec<CommandInvocation>,
        /// The target install directory this run promises to fill.
        base: PathBuf,
    },
}

impl Op {
    /// Execute the operation and return its compensation.
    pub fn apply(&self, ctx: &Ctx) -> Result<Compensation> {
        match self {
            Op::Download { url, to } => {
                ctx.reporter.report(Progress::DownloadSource {
                    url: url.as_str().to_string(),
                });
                ctx.downloader
                    .fetch_to(ctx.fs, url, to, &mut |done, total| {
                        ctx.reporter.report(Progress::Bytes { done, total });
                    })?;
                Ok(Compensation::RemoveFile(to.clone()))
            }
            Op::Unpack { from, to } => {
                let bytes = ctx.fs.read(from)?;
                unpack::extract(&bytes, to, ctx.fs)?;
                Ok(Compensation::RemoveDir(to.clone()))
            }
            Op::RunInstaller { exe, args, base } => {
                ctx.fs.create_dir_all(base)?;
                ctx.runner.run(exe, args)?;
                Ok(Compensation::RemoveDir(base.clone()))
            }
            Op::WriteEnv { deltas } => {
                let snapshots = env::apply_deltas(deltas, ctx.env)?;
                let restores: Vec<Compensation> = snapshots
                    .into_iter()
                    .map(|(name, old)| Compensation::RestoreEnv(RestoreEnv { name, old }))
                    .collect();
                Ok(Compensation::Composite(restores))
            }
            Op::RunCommand { commands, base } => {
                ctx.fs.create_dir_all(base)?;
                CommandInvocation::try_fallback(commands, ctx)
                    .map(|()| Compensation::RemoveDir(base.clone()))
            }
        }
    }

    /// The static phase label for this op; `None` for ops that self-report a richer
    /// start event (download reports [`Progress::DownloadSource`] itself).
    pub fn label(&self) -> Option<&'static str> {
        match self {
            Op::Download { .. } => None,
            Op::Unpack { .. } => Some("Extracting"),
            Op::RunInstaller { .. } => Some("Installing"),
            Op::RunCommand { .. } => Some("Installing"),
            Op::WriteEnv { .. } => Some("Configuring env"),
        }
    }
}

/// A program name to run via the operating system's PATH (e.g. `cargo`, `npm`).
///
/// Resolution through PATH (and `.exe`/PATHEXT on Windows) is left to
/// [`std::process::Command`]; the newtype makes the program a distinct value
/// instead of a bare positional string, so the run boundary stays explicit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandName(String);

impl CommandName {
    /// The program name as passed to `Command::new`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for CommandName {
    fn from(value: String) -> Self {
        CommandName(value)
    }
}

impl std::fmt::Display for CommandName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One fully-expanded external command line (plan-time data, not an action).
#[derive(Clone, Debug)]
pub struct CommandInvocation {
    /// The program to invoke, resolved via PATH.
    pub tool: CommandName,
    /// Arguments, placeholders already expanded.
    pub args: Vec<String>,
    /// Extra environment variables, layered over the inherited environment.
    pub env: Vec<(String, String)>,
}

impl CommandInvocation {
    /// Build a fully-expanded invocation from a raw command, substituting
    /// `{base}` (the install directory), `{version}`, and `{user}`.
    pub fn from_raw(raw: &RawCommand, base: &Path, version: &Version) -> CommandInvocation {
        let expand =
            |template: &str| crate::engine::expand::expand_template(template, base, version);
        CommandInvocation {
            tool: raw.tool.clone().into(),
            args: raw.args.iter().map(|arg| expand(arg)).collect(),
            env: raw
                .env
                .iter()
                .map(|(name, value)| (name.clone(), expand(value)))
                .collect(),
        }
    }

    /// Run this invocation on the context's process runner.
    pub fn run(&self, ctx: &Ctx) -> Result<()> {
        ctx.runner
            .run_with_env(Path::new(self.tool.as_str()), &self.args, &self.env)
    }

    /// Run `commands` in order until one succeeds. A missing tool and a failing
    /// command are indistinguishable here (both surface as an `Err`), and both
    /// fall back to the next candidate; when every command fails the errors are
    /// aggregated into a single [`crate::Error::Process`].
    pub fn try_fallback(commands: &[CommandInvocation], ctx: &Ctx) -> Result<()> {
        let mut failures = Vec::new();
        for invocation in commands {
            let tool = invocation.tool.to_string();
            ctx.reporter
                .report(Progress::RunningCommand { tool: tool.clone() });
            match invocation.run(ctx) {
                Ok(()) => return Ok(()),
                Err(error) => {
                    ctx.reporter.report(Progress::CommandFailed {
                        tool: tool.clone(),
                        error: error.to_string(),
                    });
                    failures.push(format!("{tool}: {error}"));
                }
            }
        }
        Err(crate::Error::Process(format!(
            "all install commands failed: {}",
            failures.join("; ")
        )))
    }
}

/// A snapshot-style compensation for one environment variable.
#[derive(Clone, Debug)]
pub struct RestoreEnv {
    /// Variable name.
    pub name: EnvVar,
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
