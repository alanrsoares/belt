//! Minimal output helpers for `grabsite`, on top of `local-common` terminal
//! utilities.
//!
//! Output is terse, one line per lifecycle step:
//! ```text
//! › Xvfb :12 up (1380x898)
//! › gecko profile ready (persistent)
//! › mullvad-browser loading https://lite.duckduckgo.com/…
//! › settle 8s elapsed
//! ✓ stable after 26.2s (2 identical captures)
//! wrote 48,213 B /tmp/shot.png in 27.1s
//! ```
//! With `-q` only the final `wrote …` result line and any warnings/errors
//! are printed; intermediate `›` steps are suppressed.

use local_common::cli::CommonFlags;
use local_common::term::{color_enabled_for, Colour};
use std::io::Write;

/// Colour handle resolved against the process `CommonFlags` and the TTY.
#[derive(Debug, Clone, Copy)]
pub struct Color(Colour);

impl Color {
    pub fn from_flags(flags: &CommonFlags) -> Self {
        let std = std::io::stdout();
        Color(Colour::new(color_enabled_for(&std, flags.no_color)))
    }

    pub fn cyan(&self, t: impl AsRef<str>) -> String {
        self.0.cyan(t)
    }

    pub fn yellow(&self, t: impl AsRef<str>) -> String {
        self.0.yellow(t)
    }
}

/// Progress reporter. Tracks wall-clock elapsed for the final timing line and
/// honours `quiet` for intermediate steps.
pub struct Out {
    pub quiet: bool,
    started: std::time::Instant,
}

impl Out {
    pub fn new(quiet: bool) -> Self {
        Self {
            quiet,
            started: std::time::Instant::now(),
        }
    }

    /// An intermediate lifecycle step. Suppressed when `quiet`.
    pub fn step(&mut self, msg: String) {
        if self.quiet {
            return;
        }
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "› {msg}");
    }

    /// A warning — always printed, even when `quiet`.
    pub fn warn(&mut self, msg: String) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{msg}");
    }

    /// A final success/detail line — always printed, even when `quiet`.
    pub fn result(&mut self, msg: String) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{msg}");
    }

    /// Human-formatted wall-clock time since the run started (`27.1s`).
    pub fn elapsed(&self) -> String {
        format!("{:.1}s", self.started.elapsed().as_secs_f64())
    }
}
