//! Downloader (`RealDownloader`) tests: the pure segment-splitting function,
//! `Content-Range` parsing, an end-to-end parallel segmented download
//! reassembled from a local HTTP server, streaming straight to disk, and retry
//! recovery from a transient failure.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::engine::download::PARALLEL_MIN_BYTES;
use crate::engine::{Downloader, RealDownloader, RealFs, Url};

#[test]
fn segment_ranges_covers_whole_file_in_worker_ranges() {
    assert_eq!(
        RealDownloader::segment_ranges(100, 4),
        vec![(0, 24), (25, 49), (50, 74), (75, 99)]
    );
}

#[test]
fn segment_ranges_drops_empty_tail_for_small_files() {
    assert_eq!(
        RealDownloader::segment_ranges(10, 4),
        vec![(0, 2), (3, 5), (6, 8), (9, 9)]
    );
}

#[test]
fn content_range_total_parses_tail_segment() {
    assert_eq!(
        RealDownloader::content_range_total(Some("bytes 0-0/135468")),
        Some(135468)
    );
    assert_eq!(
        RealDownloader::content_range_total(Some("bytes 0-1/*")),
        None
    );
    assert_eq!(RealDownloader::content_range_total(None), None);
}

/// A minimal HTTP server: answers `Range` requests with a 206 slice, otherwise a
/// 200 full body, for downloader tests.
struct RangeServer {
    url: Url,
}

impl RangeServer {
    /// An always-successful server for `body`.
    fn serve(body: Vec<u8>) -> Self {
        Self::serve_with(body, Arc::new(AtomicUsize::new(0)))
    }

    /// A server that answers 500 to the first `fail_first` full-body requests
    /// (the `bytes=0-N` range covering the whole file), so a retrying download
    /// deterministically reconnects once.
    fn serve_flaky(body: Vec<u8>, fail_first: usize) -> Self {
        Self::serve_with(body, Arc::new(AtomicUsize::new(fail_first)))
    }

    fn serve_with(body: Vec<u8>, failures: Arc<AtomicUsize>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap();
        let url = Url::from(format!("http://{port}/go1.27.zip"));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => handle_range_request(stream, &body, &failures),
                    Err(_) => break,
                }
            }
        });
        RangeServer { url }
    }
}

/// Handle one request: parse the `Range` header and write back a 206 slice.
fn handle_range_request(mut stream: TcpStream, body: &[u8], failures: &AtomicUsize) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut header = Vec::new();
    let mut one = [0u8; 1];
    loop {
        if stream.read(&mut one).unwrap_or(0) == 0 || header.len() > 16 * 1024 {
            break;
        }
        header.push(one[0]);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&header);
    let range = text
        .lines()
        .find_map(|line| line.strip_prefix("Range: "))
        .and_then(parse_range);
    let total = body.len() as u64;
    let full_range = range == Some((0, total - 1));
    if full_range && failures.load(Ordering::SeqCst) > 0 {
        failures.fetch_sub(1, Ordering::SeqCst);
        let _ =
            stream.write_all(b"HTTP/1.1 500 Internal Server Error\r\nConnection: close\r\n\r\n");
        return;
    }
    let (start, end) = range.unwrap_or((0, total - 1));
    let slice = &body[start as usize..=end as usize];
    let status = if range.is_some() {
        format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{total}\r\n")
    } else {
        "HTTP/1.1 200 OK\r\n".to_string()
    };
    let _ = stream.write_all(
        format!(
            "{status}Content-Length: {}\r\nConnection: close\r\n\r\n",
            slice.len()
        )
        .as_bytes(),
    );
    let _ = stream.write_all(slice);
}

/// Parse a `bytes=start-end` range header.
fn parse_range(value: &str) -> Option<(u64, u64)> {
    let spec = value.strip_prefix("bytes=")?;
    let (start, end) = spec.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?))
}

/// Real downloader: after a parallel segmented download the bytes must be fully
/// reassembled, and progress must reach the total.
#[test]
fn real_downloader_parallel_reassembles_body() {
    let body_len = PARALLEL_MIN_BYTES as usize + 4097;
    let body: Vec<u8> = (0..body_len).map(|i| (i % 251) as u8).collect();
    let server = RangeServer::serve(body.clone());
    let mut last = (0u64, None);
    let bytes = RealDownloader::default()
        .fetch(&server.url, &mut |done, total| last = (done, total))
        .unwrap();
    assert_eq!(bytes, body);
    assert_eq!(last, (body_len as u64, Some(body_len as u64)));
}

/// The streaming path must write the reassembled body to the target file,
/// report progress to the total, and leave no staging file behind.
#[test]
fn fetch_to_streams_parallel_and_reassembles_file() {
    let body_len = PARALLEL_MIN_BYTES as usize + 4097;
    let body: Vec<u8> = (0..body_len).map(|i| (i % 251) as u8).collect();
    let server = RangeServer::serve(body.clone());
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("out.bin");
    let mut last = (0u64, None);
    RealDownloader::default()
        .fetch_to(&RealFs, &server.url, &target, &mut |done, total| {
            last = (done, total)
        })
        .unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), body);
    assert_eq!(last, (body_len as u64, Some(body_len as u64)));
    let part = format!("{}.part", target.display());
    assert!(
        !Path::new(&part).exists(),
        "staging file must be renamed into place"
    );
}

/// A single transient 500 on the data request must be recovered by the
/// per-segment retry; the `bytes=0-0` probe and the final body both succeed.
#[test]
fn download_retries_a_transient_failure() {
    let body_len = PARALLEL_MIN_BYTES as usize + 17;
    let body: Vec<u8> = (0..body_len).map(|i| (i % 251) as u8).collect();
    let server = RangeServer::serve_flaky(body.clone(), 1);
    let bytes = RealDownloader::with_workers(1)
        .fetch(&server.url, &mut |_, _| {})
        .unwrap();
    assert_eq!(bytes, body);
}
