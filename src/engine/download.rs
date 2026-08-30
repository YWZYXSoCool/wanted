//! The real network downloader: fetches large files with segmented parallel
//! HTTP `Range` requests, writing them either to memory or straight to disk.
//!
//! The per-segment machietry lives in the [`segments`] submodule; this module
//! decides how many segments to spawn, whether a download is big enough to
//! parallelise, and where the reassembled bytes land.

mod segments;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use crate::Result;
use crate::engine::url::Url;
use crate::engine::{Downloader, Fs};
use segments::{
    READ_CHUNK, Segment, Sink, aggregate, fetch_into, io_error, record, split_regions, take_error,
};

/// Default number of concurrent segment workers for a parallel download.
pub const DEFAULT_PARALLEL_WORKERS: usize = 4;

/// Files smaller than this are not segmented and fetched as a single stream.
pub(crate) const PARALLEL_MIN_BYTES: u64 = 8 << 20;

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
            if crate::engine::cancel_flag().load(Ordering::SeqCst) {
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
    pub(crate) fn segment_ranges(total: u64, workers: usize) -> Vec<(u64, u64)> {
        let chunk = total.div_ceil(workers as u64);
        (0..workers)
            .map(|i| {
                let start = i as u64 * chunk;
                (start, (start + chunk - 1).min(total - 1))
            })
            .filter(|(start, end)| start <= end)
            .collect()
    }

    /// Download segments in parallel into an in-memory buffer; the calling
    /// thread aggregates received bytes as progress.
    fn parallel_stream(
        url: &Url,
        total: u64,
        workers: usize,
        on_progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<Vec<u8>> {
        let cancel = Arc::new(AtomicBool::new(false));
        let error: Arc<Mutex<Option<crate::Error>>> = Arc::new(Mutex::new(None));
        let ranges = Self::segment_ranges(total, workers);
        let mut buf = vec![0u8; total as usize];

        let out = split_regions(&mut buf, &ranges);
        thread::scope(|scope| {
            let (tx, rx) = mpsc::channel::<u64>();
            for ((start, end), out) in ranges.into_iter().zip(out) {
                let segment = Segment::new(url.clone(), start, end);
                let tx = tx.clone();
                let cancel = cancel.clone();
                let error = error.clone();
                scope.spawn(move || {
                    record(
                        fetch_into(&segment, Sink::Memory(out), &tx, &cancel),
                        &cancel,
                        &error,
                    )
                });
            }
            drop(tx);
            aggregate(&rx, total, on_progress);
        });

        take_error(&error)?;
        Ok(buf)
    }

    /// Download segments in parallel straight to a pre-sized temporary file,
    /// then rename it into place (a partial file never survives an error).
    fn parallel_to_file(
        url: &Url,
        total: u64,
        workers: usize,
        to: &Path,
        on_progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<()> {
        let temporary = part_path(to);
        create_part(&temporary, total)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let error: Arc<Mutex<Option<crate::Error>>> = Arc::new(Mutex::new(None));

        thread::scope(|scope| {
            let (tx, rx) = mpsc::channel::<u64>();
            for (start, end) in Self::segment_ranges(total, workers) {
                let segment = Segment::new(url.clone(), start, end);
                let tx = tx.clone();
                let cancel = cancel.clone();
                let error = error.clone();
                let path = temporary.clone();
                scope.spawn(move || {
                    let mut file = open_part(&path);
                    record(
                        fetch_into(
                            &segment,
                            Sink::File {
                                file: &mut file,
                                start: segment.start(),
                            },
                            &tx,
                            &cancel,
                        ),
                        &cancel,
                        &error,
                    )
                });
            }
            drop(tx);
            aggregate(&rx, total, on_progress);
        });

        if let Err(error) = take_error(&error) {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
        std::fs::rename(&temporary, to).map_err(|e| failure_cleanup(&temporary, e))
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

    fn fetch_to(
        &self,
        fs: &dyn Fs,
        url: &Url,
        to: &Path,
        on_progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<()> {
        match Self::probe_total(url)? {
            Some(total) if total >= PARALLEL_MIN_BYTES => {
                Self::parallel_to_file(url, total, self.workers, to, on_progress)
            }
            _ => {
                let bytes = self.fetch(url, on_progress)?;
                fs.write(to, &bytes)
            }
        }
    }
}

impl RealDownloader {
    /// Parse the total byte count from the trailing segment of `Content-Range`
    /// (e.g. `bytes 0-0/135468`); returns `None` when invalid.
    pub(crate) fn content_range_total(header: Option<&str>) -> Option<u64> {
        header?.rsplit('/').next()?.trim().parse().ok()
    }
}

/// The sibling path of `to` used as the staging file during a streaming download.
fn part_path(to: &Path) -> PathBuf {
    let mut path = to.to_path_buf();
    let mut name = to
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push_str(".part");
    path.set_file_name(name);
    path
}

/// Create the staging file, create its parent directory, and pre-size it.
fn create_part(path: &Path, total: u64) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io_error)?;
    }
    let file = std::fs::File::create(path).map_err(io_error)?;
    file.set_len(total).map_err(io_error)
}

/// Open a write handle to the staging file for a worker segment.
fn open_part(path: &Path) -> std::fs::File {
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap_or_else(|_| unreachable!("staging file was created before workers spawn"))
}

/// Delete the staging file when the final rename fails, then surface the error.
fn failure_cleanup(temporary: &Path, error: std::io::Error) -> crate::Error {
    let _ = std::fs::remove_file(temporary);
    crate::Error::Network(error.to_string())
}
