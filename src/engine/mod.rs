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

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use crate::Result;
use crate::env::EnvStore;
use crate::report::Reporter;

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

/// Default read chunk size (64 KiB) so progress callbacks are not too frequent.
const READ_CHUNK: usize = 64 * 1024;

/// Default number of concurrent segment workers for a parallel download.
pub const DEFAULT_PARALLEL_WORKERS: usize = 4;

/// Files smaller than this are not segmented and fetched as a single stream.
const PARALLEL_MIN_BYTES: u64 = 8 << 20;

/// Process-wide cancellation flag, set by the signal handler on Ctrl+C so
/// in-flight downloads abort and the normal rollback path runs instead of the
/// OS killing the process mid-install.
static CANCELLED: AtomicBool = AtomicBool::new(false);

/// Accessor for the process-wide cancellation flag.
pub fn cancel_flag() -> &'static AtomicBool {
    &CANCELLED
}

/// A byte range of a file, owned so it can move into a scoped thread.
struct Segment {
    url: Url,
    start: u64,
    end: u64,
}

impl Segment {
    /// Fetch `[start, end]` into `out`, forwarding each chunk to the progress channel.
    fn run(&self, out: &mut [u8], tx: &mpsc::Sender<u64>, cancel: &AtomicBool) -> Result<()> {
        use std::io::Read;
        let range = format!("bytes={}-{}", self.start, self.end);
        let response = ureq::get(self.url.as_str())
            .set("Range", &range)
            .call()
            .map_err(|e| crate::Error::Network(e.to_string()))?;
        let mut reader = response.into_reader();
        let mut buffer = [0u8; READ_CHUNK];
        let mut filled = 0usize;
        while filled < out.len() {
            if cancel.load(Ordering::SeqCst) || cancel_flag().load(Ordering::SeqCst) {
                return Err(crate::Error::Cancelled);
            }
            let read = reader
                .read(&mut buffer)
                .map_err(|e| crate::Error::Network(e.to_string()))?;
            if read == 0 {
                break;
            }
            let take = read.min(out.len() - filled);
            out[filled..filled + take].copy_from_slice(&buffer[..take]);
            filled += take;
            let _ = tx.send(take as u64);
        }
        if filled != out.len() {
            return Err(crate::Error::Network(format!(
                "segment {}-{} truncated: got {} bytes",
                self.start, self.end, filled
            )));
        }
        Ok(())
    }
}

/// A `ureq`-based real downloader: fetches large files with segmented parallel
/// HTTP `Range` requests and falls back to a single stream when unsupported.
/// The number of concurrent segment workers is configurable.
pub struct RealDownloader {
    workers: usize,
}

impl Default for RealDownloader {
    fn default() -> Self {
        Self::with_workers(DEFAULT_PARALLEL_WORKERS)
    }
}

impl RealDownloader {
    /// A downloader splitting large files across `workers` parallel segments.
    pub fn with_workers(workers: usize) -> Self {
        RealDownloader {
            workers: workers.max(1),
        }
    }
}

impl Downloader for RealDownloader {
    fn fetch(&self, url: &Url, on_progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Vec<u8>> {
        match Self::probe_total(url)? {
            Some(total) if total >= PARALLEL_MIN_BYTES => {
                Self::parallel_stream(url, total, self.workers, on_progress)
            }
            _ => Self::single_stream(url, on_progress),
        }
    }
}

impl RealDownloader {
    /// Issue a `Range` probe: returns the total byte count when the server
    /// answers `206` (segmentation supported), otherwise `None`.
    fn probe_total(url: &Url) -> Result<Option<u64>> {
        let response = ureq::get(url.as_str())
            .set("Range", "bytes=0-0")
            .call()
            .map_err(|e| crate::Error::Network(e.to_string()))?;
        if response.status() == 206 {
            Ok(Self::content_range_total(response.header("Content-Range")))
        } else {
            Ok(None)
        }
    }

    /// Single-connection streaming download (fallback when the server rejects
    /// `Range` or the file is small).
    fn single_stream(url: &Url, on_progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Vec<u8>> {
        use std::io::Read;
        let response = ureq::get(url.as_str())
            .call()
            .map_err(|e| crate::Error::Network(e.to_string()))?;
        let total = response
            .header("Content-Length")
            .and_then(|value| value.parse::<u64>().ok());
        let mut reader = response.into_reader();
        let mut bytes = Vec::new();
        let mut buffer = [0u8; READ_CHUNK];
        let mut done = 0u64;
        loop {
            if cancel_flag().load(Ordering::SeqCst) {
                return Err(crate::Error::Cancelled);
            }
            let read = reader
                .read(&mut buffer)
                .map_err(|e| crate::Error::Network(e.to_string()))?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
            done += read as u64;
            on_progress(done, total);
        }
        Ok(bytes)
    }

    /// Split `[0, total)` into at most `workers` closed ranges, dropping empty tails.
    fn segment_ranges(total: u64, workers: usize) -> Vec<(u64, u64)> {
        let chunk = total.div_ceil(workers as u64);
        (0..workers)
            .map(|i| {
                let start = i as u64 * chunk;
                (start, (start + chunk - 1).min(total - 1))
            })
            .filter(|(start, end)| start <= end)
            .collect()
    }

    /// Download segments in parallel: each thread fetches a disjoint byte range
    /// into its slice; the calling thread aggregates received bytes as progress.
    fn parallel_stream(
        url: &Url,
        total: u64,
        workers: usize,
        on_progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<Vec<u8>> {
        use std::time::Duration;

        let cancel = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None::<crate::Error>));
        let ranges = Self::segment_ranges(total, workers);
        let mut buf = vec![0u8; total as usize];

        thread::scope(|scope| {
            let (tx, rx) = mpsc::channel::<u64>();
            let slices = split_buf(&mut buf, &ranges);

            let mut handles = Vec::new();
            for ((start, end), slice) in ranges.iter().zip(slices) {
                let segment = Segment {
                    url: url.clone(),
                    start: *start,
                    end: *end,
                };
                let tx = tx.clone();
                let cancel = cancel.clone();
                let error = error.clone();
                handles.push(
                    scope.spawn(move || record(segment.run(slice, &tx, &cancel), &cancel, &error)),
                );
            }

            let mut received = 0u64;
            while !is_done(&handles, &cancel) {
                received = receive_all(&rx, received, total, on_progress);
                std::thread::sleep(Duration::from_millis(10));
            }
            receive_all(&rx, received, total, on_progress);
        });

        take_error(&error).map_or(Ok(buf), Err)
    }

    /// Parse the total byte count from the trailing segment of `Content-Range`
    /// (e.g. `bytes 0-0/135468`); returns `None` when invalid.
    fn content_range_total(header: Option<&str>) -> Option<u64> {
        header?.rsplit('/').next()?.trim().parse().ok()
    }
}

/// Split `buf` into one slice per range, in range order.
fn split_buf<'a>(buf: &'a mut [u8], ranges: &[(u64, u64)]) -> Vec<&'a mut [u8]> {
    let mut slices = Vec::new();
    let mut remaining = buf;
    for (start, end) in ranges {
        let len = (end - start + 1) as usize;
        let (head, tail) = remaining.split_at_mut(len);
        slices.push(head);
        remaining = tail;
    }
    slices
}

/// Store the first download error and flag cancellation; later errors are dropped.
fn record(result: Result<()>, cancel: &Arc<AtomicBool>, error: &Arc<Mutex<Option<crate::Error>>>) {
    if let Err(err) = result {
        cancel.store(true, Ordering::SeqCst);
        let mut slot = error.lock().unwrap_or_else(|poison| poison.into_inner());
        if slot.is_none() {
            *slot = Some(err);
        }
    }
}

/// Drain the progress channel into `received`, reporting cumulative bytes.
fn receive_all(
    rx: &mpsc::Receiver<u64>,
    mut received: u64,
    total: u64,
    on_progress: &mut dyn FnMut(u64, Option<u64>),
) -> u64 {
    for delta in rx.try_iter() {
        received += delta;
        on_progress(received, Some(total));
    }
    received
}

/// Whether every segment finished or a download was cancelled.
fn is_done(handles: &[thread::ScopedJoinHandle<'_, ()>], cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::SeqCst) || handles.iter().all(|handle| handle.is_finished())
}

/// Take the recorded error out of the shared slot; `None` if nothing failed.
fn take_error(error: &Mutex<Option<crate::Error>>) -> Option<crate::Error> {
    let mut slot = error.lock().unwrap_or_else(|poison| poison.into_inner());
    slot.take()
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
