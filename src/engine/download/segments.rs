//! Per-segment fetch workers: each downloads one byte range (with retry) into a
//! memory slice or a file region, streaming per-chunk progress into a shared
//! channel. The caller thread aggregates those deltas with a blocking `recv()`
//! loop, so a slow segment or a retry backoff simply stalls the receiver rather
//! than busy-waiting.

use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use crate::Result;
use crate::engine::url::Url;

/// Default read chunk size (64 KiB) so progress callbacks are not too frequent.
pub(super) const READ_CHUNK: usize = 64 * 1024;

/// How many times a single segment request is retried before giving up.
const SEGMENT_RETRIES: u32 = 3;

/// Base backoff (200 ms) doubling per retry, capped at two seconds.
const BACKOFF_INITIAL_MS: u64 = 200;
const BACKOFF_CAP_MS: u64 = 2_000;

/// The parallel download destination: a memory slice or a file region.
pub(super) enum Sink<'a> {
    Memory(&'a mut [u8]),
    File {
        file: &'a mut std::fs::File,
        start: u64,
    },
}

/// A byte range of a download, owned so it can move into a scoped thread.
pub(super) struct Segment {
    url: Url,
    start: u64,
    end: u64,
}

impl Segment {
    /// A segment carrying the `[start, end]` byte range of `url`.
    pub(super) fn new(url: Url, start: u64, end: u64) -> Self {
        Segment { url, start, end }
    }

    /// The inclusive start offset of the range.
    pub(super) fn start(&self) -> u64 {
        self.start
    }

    /// The number of bytes this segment promises to deliver.
    fn len(&self) -> u64 {
        self.end - self.start + 1
    }

    /// `bytes=start-end` request header value.
    fn range(&self) -> String {
        format!("bytes={}-{}", self.start, self.end)
    }

    /// Open the `[start, end]` range, retrying transient failures with backoff.
    /// Never retries once the shared cancel flag or the process-wide Ctrl+C flag
    /// is set, so a sibling failure or cancel aborts every segment fast.
    fn open(&self, cancel: &AtomicBool) -> Result<ureq::Response> {
        for attempt in 0..=SEGMENT_RETRIES {
            let response = ureq::get(self.url.as_str())
                .set("Range", &self.range())
                .call()
                .map_err(|e| crate::Error::Network(e.to_string()));
            match response {
                Ok(response) => return Ok(response),
                Err(error) => {
                    if is_cancelled(cancel) || attempt == SEGMENT_RETRIES {
                        return Err(error);
                    }
                    std::thread::sleep(backoff(attempt + 1));
                }
            }
        }
        unreachable!("open returns from both match arms")
    }
}

/// Fetch the segment into `sink`, streaming progress deltas and honouring the
/// shared cancel flag between chunks.
pub(super) fn fetch_into(
    segment: &Segment,
    mut sink: Sink,
    tx: &mpsc::Sender<u64>,
    cancel: &AtomicBool,
) -> Result<()> {
    let response = segment.open(cancel)?;
    let mut reader = response.into_reader();
    let mut buffer = [0u8; READ_CHUNK];
    let mut done = 0u64;
    loop {
        if is_cancelled(cancel) {
            return Err(crate::Error::Cancelled);
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|e| crate::Error::Network(e.to_string()))?;
        if read == 0 {
            break;
        }
        let take = take_length(read, segment, done);
        write_chunk(&mut sink, &buffer[..take], done)?;
        done += take as u64;
        let _ = tx.send(take as u64);
    }
    if done != segment.len() {
        return Err(crate::Error::Network(format!(
            "segment {}-{} truncated: got {} bytes",
            segment.start, segment.end, done
        )));
    }
    Ok(())
}

/// Bytes to write for chunk `read` at offset `done`: never overshoot the range.
fn take_length(read: usize, segment: &Segment, done: u64) -> usize {
    read.min((segment.len() - done) as usize)
}

/// Write `chunk` at offset `done` into a memory slice or a file region.
fn write_chunk(sink: &mut Sink, chunk: &[u8], done: u64) -> Result<()> {
    match sink {
        Sink::Memory(out) => {
            let at = done as usize;
            out[at..(at + chunk.len())].copy_from_slice(chunk);
        }
        Sink::File { file, start } => {
            file.seek(SeekFrom::Start(*start + done))
                .map_err(io_error)?;
            file.write_all(chunk).map_err(io_error)?;
        }
    }
    Ok(())
}

/// Whether the shared or process-wide cancel flag is set.
fn is_cancelled(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::SeqCst) || crate::engine::cancel_flag().load(Ordering::SeqCst)
}

/// Exponential backoff for the `attempt`-th retry, capped at two seconds.
fn backoff(attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(4);
    let millis = (BACKOFF_INITIAL_MS << shift).min(BACKOFF_CAP_MS);
    Duration::from_millis(millis)
}

/// Map an I/O error onto the engine error type.
pub(super) fn io_error(error: std::io::Error) -> crate::Error {
    crate::Error::Network(error.to_string())
}

/// Split `buf` into one mutable slice per range, in range order.
pub(super) fn split_regions<'a>(buf: &'a mut [u8], ranges: &[(u64, u64)]) -> Vec<&'a mut [u8]> {
    let mut regions = Vec::new();
    let mut remaining = buf;
    for (start, end) in ranges {
        let len = (end - start + 1) as usize;
        let (head, tail) = remaining.split_at_mut(len);
        regions.push(head);
        remaining = tail;
    }
    regions
}

/// Block until every worker has dropped its sender, reporting cumulative bytes.
pub(super) fn aggregate(
    rx: &mpsc::Receiver<u64>,
    total: u64,
    on_progress: &mut dyn FnMut(u64, Option<u64>),
) {
    let mut received = 0u64;
    while let Ok(delta) = rx.recv() {
        received += delta;
        on_progress(received, Some(total));
    }
}

/// Store the first download error and flag cancellation; later errors are dropped.
pub(super) fn record(
    result: Result<()>,
    cancel: &Arc<AtomicBool>,
    error: &Arc<Mutex<Option<crate::Error>>>,
) {
    if let Err(err) = result {
        cancel.store(true, Ordering::SeqCst);
        let mut slot = error.lock().unwrap_or_else(|poison| poison.into_inner());
        if slot.is_none() {
            *slot = Some(err);
        }
    }
}

/// Take the recorded error out of the shared slot; `None` if nothing failed.
pub(super) fn take_error(error: &Mutex<Option<crate::Error>>) -> Result<()> {
    let mut slot = error.lock().unwrap_or_else(|poison| poison.into_inner());
    match slot.take() {
        Some(err) => Err(err),
        None => Ok(()),
    }
}
