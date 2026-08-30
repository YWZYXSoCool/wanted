//! Terminal rendering of install [`Progress`] events.
//!
//! [`TerminalReporter`] lives here (not in the CLI front end) so the report module
//! owns both the event vocabulary and its reference rendering, and `main.rs` stays
//! under the per-file line budget.

use crate::report::{Progress, Reporter};

/// Render engine progress events as a multi-line terminal display.
///
/// Real downloads often lack `Content-Length`, so the total is unknown; forcing a
/// deterministic progress bar would render an empty "0 B/0 B" bar at full width.
/// Hence an indeterminate spinner line plus a live byte count, shown alongside a
/// persistent status line (tool name, then the active phase) — two concurrently
/// visible progress texts, decoupled from the single aggregated byte total the
/// engine reports.
pub struct TerminalReporter {
    panel: indicatif::MultiProgress,
    status: indicatif::ProgressBar,
    spinner: indicatif::ProgressBar,
}

impl TerminalReporter {
    /// A terminal reporter whose status line names the tool being installed.
    pub fn new(tool: &str) -> Self {
        let panel = indicatif::MultiProgress::new();
        let status = panel.add(indicatif::ProgressBar::new(0).with_style(Self::status_style()));
        status.set_message(format!("installing {tool}"));
        let spinner =
            panel.add(indicatif::ProgressBar::new_spinner().with_style(Self::spinner_style()));
        Self {
            panel,
            status,
            spinner,
        }
    }

    /// A plain status line with no bar or spinner.
    fn status_style() -> indicatif::ProgressStyle {
        indicatif::ProgressStyle::with_template("{msg}").expect("static status template is valid")
    }

    /// The live-byte spinner line.
    fn spinner_style() -> indicatif::ProgressStyle {
        indicatif::ProgressStyle::with_template("{spinner:.cyan} {bytes}")
            .expect("static spinner template is valid")
            .tick_chars("|/-\\")
    }

    /// Clear every progress line so the final message prints cleanly.
    pub fn finish(&self) {
        let _ = self.panel.clear();
    }
}

impl Reporter for TerminalReporter {
    fn report(&self, event: Progress) {
        match event {
            Progress::Phase(label) => self.start_phase(label.to_string()),
            Progress::DownloadSource { url } => self.start_phase(format!("download from {url}")),
            Progress::TryingMethod { index, total } => {
                self.start_phase(format!("trying install method {index}/{total}"));
            }
            Progress::MethodFailed { error } => {
                self.status
                    .set_message(format!("method failed: {error}; trying next"));
            }
            Progress::RunningCommand { tool } => {
                self.start_phase(format!("running {tool}"));
            }
            Progress::CommandFailed { tool, error } => {
                self.status
                    .set_message(format!("{tool} failed: {error}; trying next"));
            }
            Progress::Bytes { done, .. } => {
                self.spinner.set_position(done);
                self.spinner.tick();
            }
        }
    }
}

impl TerminalReporter {
    /// Move to a new phase: show `message` as the status line and reset the spinner.
    fn start_phase(&self, message: String) {
        self.status.set_message(message);
        self.spinner.set_position(0);
    }
}
