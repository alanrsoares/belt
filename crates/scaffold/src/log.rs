//! Records every scaffold run as a one-line TSV entry so the user can see what
//! they generated and when, e.g.:
//!
//!    1735699200   ts   web   /Users/alana/dev/web
//!
//! The log lives under the tool's config dir (`~/.config/belt/scaffold`)
//! and is *best-effort*: a logging failure must never fail a scaffold.

use std::fs;
use std::io::Write;
use std::path::Path;

use crate::templates::Lang;

fn sanitize_log_field(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\t' | '\n' | '\r' => ' ',
            other => other,
        })
        .collect()
}

/// Build a one-line, tab-separated record.
///
/// Pure (takes its own timestamp) so it is trivially unit-testable.
pub fn record_line(secs: &str, lang: Lang, name: &str, target: &Path) -> String {
    let clean_name = sanitize_log_field(name);
    let clean_target = sanitize_log_field(&target.display().to_string());
    format!("{secs}\t{lang}\t{clean_name}\t{clean_target}\n")
}

/// Append `line` to `recent.log` under `dir`, creating the directory tree if
/// needed. Returns the underlying io result; callers ignore errors so a missing
/// or unreadable config dir cannot break a scaffold.
pub fn append(dir: &Path, line: &str) -> std::io::Result<()> {
    local_common::ensure_dir(dir)?;
    let log_path = dir.join("recent.log");
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    f.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_line_is_tab_separated_and_terminates_with_newline() {
        let target = Path::new("/tmp/proj");
        let line = record_line("123", Lang::Ts, "web", target);
        assert!(line.ends_with('\n'));
        assert_eq!(line, "123\tts\tweb\t/tmp/proj\n");
    }

    #[test]
    fn record_line_strips_tabs_and_newlines() {
        let target = Path::new("/tmp/proj\twith\nnewlines");
        let line = record_line("123", Lang::Rust, "bad\tname\r\nhere", target);
        assert_eq!(line, "123\trust\tbad name  here\t/tmp/proj with newlines\n");
    }
}
