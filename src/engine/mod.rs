//! Execution engine.
//!
//! Layered as plan (pure) -> staging -> commit, with every step carrying its own
//! compensation. All I/O goes through the [`Fs`] and downloader seams so rollback
//! can be replayed against in-memory backends.

pub mod execute;
pub mod fs;
pub mod ops;
pub mod plan;
pub mod staging;
pub mod unpack;
pub mod url;

mod download;
mod expand;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::Result;
use crate::env::EnvStore;
use crate::report::Reporter;

pub use download::{DEFAULT_PARALLEL_WORKERS, RealDownloader};
pub use fs::{Fs, MemFs, RealFs};
pub use url::Url;

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "dl_tests.rs"]
mod dl_tests;

#[cfg(test)]
#[path = "installer_tests.rs"]
mod installer_tests;

#[cfg(test)]
#[path = "command_tests.rs"]
mod command_tests;

/// Downloader abstraction: pulls a URL into bytes and reports progress while
/// streaming. The real implementation hits the network; tests inject fakes.
pub trait Downloader: Send {
    /// Fetch all bytes of `url`, reporting `(done, total)` per chunk read
    /// (`total` is `None` when unknown).
    fn fetch(&self, url: &Url, on_progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Vec<u8>>;

    /// Fetch `url` streaming straight into the file at `to`, reporting progress.
    /// Downloaders may stage to a temporary file; the default buffers the whole
    /// body in memory then writes it through `fs`, so test doubles only need to
    /// implement [`Self::fetch`].
    fn fetch_to(
        &self,
        fs: &dyn Fs,
        url: &Url,
        to: &Path,
        on_progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<()> {
        let bytes = self.fetch(url, on_progress)?;
        fs.write(to, &bytes)
    }
}

/// Process-runner abstraction: runs a child process to completion. The real
/// implementation spawns an OS process; tests inject fakes.
pub trait ProcessRunner: Send {
    /// Run `program` with `args`, waiting for it to finish.
    fn run(&self, program: &std::path::Path, args: &[String]) -> Result<()>;

    /// Run `program` with `args` and extra environment variables layered over the
    /// inherited environment. Defaults to delegating to [`Self::run`] (ignoring
    /// `env`), so test doubles only need to override `run`.
    fn run_with_env(
        &self,
        program: &std::path::Path,
        args: &[String],
        _env: &[(String, String)],
    ) -> Result<()> {
        self.run(program, args)
    }
}

/// The real process runner: a blocking child process with inherited stdio.
pub struct RealProcess;

impl ProcessRunner for RealProcess {
    fn run(&self, program: &std::path::Path, args: &[String]) -> Result<()> {
        self.run_with_env(program, args, &[])
    }

    fn run_with_env(
        &self,
        program: &std::path::Path,
        args: &[String],
        env: &[(String, String)],
    ) -> Result<()> {
        let status = std::process::Command::new(program)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .args(args)
            .status()
            .map_err(|e| crate::Error::Process(e.to_string()))?;
        if status.success() {
            Ok(())
        } else {
            Err(crate::Error::Process(format!("exited with {status}")))
        }
    }
}

/// Process-wide cancellation flag, set by the signal handler on Ctrl+C so
/// in-flight downloads abort and the normal rollback path runs instead of the
/// OS killing the process mid-install.
static CANCELLED: AtomicBool = AtomicBool::new(false);

/// Accessor for the process-wide cancellation flag.
pub fn cancel_flag() -> &'static AtomicBool {
    &CANCELLED
}

/// Execution context: root directory plus swappable backends for test doubles.
pub struct Ctx<'a> {
    /// The `.wanted` root directory.
    pub root: PathBuf,
    /// Filesystem backend.
    pub fs: &'a dyn Fs,
    /// Download backend.
    pub downloader: &'a dyn Downloader,
    /// Process-backend (runs silent installers).
    pub runner: &'a dyn ProcessRunner,
    /// Environment variable persistence backend.
    pub env: &'a dyn EnvStore,
    /// Progress reporting backend.
    pub reporter: &'a dyn Reporter,
}
