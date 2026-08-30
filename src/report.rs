//! Progress reporting during install.
//!
//! The engine only emits [`Progress`] events; how they are rendered is up to the
//! [`Reporter`] implementation (terminal spinner / silent / log). The engine and
//! the display are decoupled, so tests can inject the silent implementation.

pub mod terminal;

/// One progress event during an install.
#[derive(Clone, Debug)]
pub enum Progress {
    /// Start of a named phase (extract, write env).
    Phase(&'static str),
    /// A download is starting: pull `url` into the local staging file.
    DownloadSource { url: String },
    /// The install is attempting candidate method N of `total` (plan-level fallback).
    TryingMethod { index: usize, total: usize },
    /// A candidate method failed; the chain moves to the next one.
    MethodFailed { error: String },
    /// An external install command (e.g. a package manager) is being attempted.
    RunningCommand { tool: String },
    /// An external command failed; the command list moves to the next candidate.
    CommandFailed { tool: String, error: String },
    /// Byte progress; `total` is `None` when unknown.
    Bytes { done: u64, total: Option<u64> },
}

/// Pure logical progress bar state, decoupled from terminal rendering, so the
/// "progress actually advances" behavior can be unit-tested.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProgressState {
    /// Confirmed total bytes; `None` means unknown (indeterminate progress).
    pub total: Option<u64>,
    /// Bytes downloaded so far; monotonically non-decreasing.
    pub position: u64,
}

impl ProgressState {
    /// Advance state by applying an event: `Bytes` adopts the total and moves
    /// the position monotonically; any other event resets to start fresh.
    pub fn update(self, event: &Progress) -> ProgressState {
        match event {
            Progress::Bytes { done, total } => ProgressState {
                total: total.or(self.total),
                position: self.position.max(*done),
            },
            _ => ProgressState::default(),
        }
    }
}

/// The consumer side of progress events.
pub trait Reporter: Send {
    /// Report one progress event.
    fn report(&self, event: Progress);
}

/// A silent implementation that drops every event, for tests and headless use.
pub struct SilentReporter;

impl Reporter for SilentReporter {
    fn report(&self, _event: Progress) {}
}

#[cfg(test)]
mod tests;
