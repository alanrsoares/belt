//! Plain, CI-parseable report used off a TTY, under `CI`, and by `--compact`.
//!
//! Every line fanout writes here starts at column 0 so GitHub Actions workflow
//! commands (`::group::`, `::error::`) are recognised. A failed task prints:
//!
//! ```text
//! failed test:full exit=1 duration=533.69s
//! ::group::test:full
//! <unprefixed tail>
//! ::endgroup::
//! ::error title=test%3Afull::failed in 533.69s (exit 1)
//! ```

use std::time::Duration;

use crate::runner::TaskResult;
use crate::ui::{strip_ansi, Outcome};

/// `CI=true` / `CI=1` selects the report even on a TTY.
pub fn ci_env_set() -> bool {
    std::env::var("CI").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

/// Seconds with two decimals, e.g. `533.69s`.
pub fn secs(d: Duration) -> String {
    format!("{:.2}s", d.as_secs_f64())
}

/// Escape a workflow-command message.
pub fn escape_data(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// Escape a workflow-command property value (`title=`, …).
pub fn escape_property(s: &str) -> String {
    escape_data(s).replace(':', "%3A").replace(',', "%2C")
}

fn workflow_command(line: &str) -> Option<String> {
    let plain = strip_ansi(line);
    let trimmed = plain.trim_start();
    trimmed.starts_with("::").then(|| trimmed.to_string())
}

/// Whether a child output line belongs in the retained tail. A child's own
/// `::group::` / `::endgroup::` would close the task's group early, so they go.
pub fn keep_in_tail(line: &str) -> bool {
    !workflow_command(line)
        .is_some_and(|cmd| cmd.starts_with("::group::") || cmd.starts_with("::endgroup::"))
}

/// A tail line as printed: child annotations are moved to column 0 so they
/// still register; everything else is passed through untouched.
fn render_tail_line(line: &str) -> String {
    match workflow_command(line) {
        Some(cmd)
            if cmd.starts_with("::error")
                || cmd.starts_with("::warning")
                || cmd.starts_with("::notice") =>
        {
            cmd
        }
        _ => line.to_string(),
    }
}

/// `running <name>`, printed as a task starts.
pub fn started(name: &str) -> String {
    format!("running {name}\n")
}

/// The block for a settled task. Passed and cancelled tasks get one line; a
/// failure or timeout gets its tail in a group and a column-0 annotation.
pub fn settled(
    name: &str,
    outcome: Outcome,
    exit_code: i32,
    duration: Duration,
    tail: &[String],
    omitted: usize,
) -> String {
    let (head, annotation) = match outcome {
        Outcome::Failed(_) => (
            format!("failed {name} exit={exit_code} duration={}", secs(duration)),
            format!("failed in {} (exit {exit_code})", secs(duration)),
        ),
        Outcome::Timeout => (
            format!("timeout {name} after={}", secs(duration)),
            format!("timed out after {}", secs(duration)),
        ),
        Outcome::Passed => return format!("passed {name} duration={}\n", secs(duration)),
        Outcome::Cancelled => return format!("cancelled {name}\n"),
        Outcome::Pending | Outcome::Running => return String::new(),
    };

    let mut out = format!("{head}\n");
    if !tail.is_empty() {
        out.push_str(&format!("::group::{}\n", escape_data(name)));
        if omitted > 0 {
            let plural = if omitted == 1 { "" } else { "s" };
            out.push_str(&format!("… {omitted} earlier line{plural} omitted\n"));
        }
        for line in tail {
            out.push_str(&render_tail_line(line));
            out.push('\n');
        }
        out.push_str("::endgroup::\n");
    }
    out.push_str(&format!(
        "::error title={}::{}\n",
        escape_property(name),
        escape_data(&annotation)
    ));
    out
}

/// One summary line, plus a column-0 annotation when anything went wrong.
pub fn summary(results: &[TaskResult], elapsed: Duration) -> String {
    let count = |pred: fn(&Outcome) -> bool| results.iter().filter(|r| pred(&r.outcome)).count();
    let passed = count(|o| *o == Outcome::Passed);
    let failed = count(Outcome::is_bad);
    let cancelled = count(|o| *o == Outcome::Cancelled);

    let plural = if results.len() == 1 { "" } else { "s" };
    let mut line = format!("{} task{plural}: {passed} passed", results.len());
    if failed > 0 {
        line.push_str(&format!(", {failed} failed"));
    }
    if cancelled > 0 {
        line.push_str(&format!(", {cancelled} cancelled"));
    }
    line.push_str(&format!(" ({})", secs(elapsed)));

    if failed == 0 {
        return format!("{line}\n");
    }

    let names: Vec<&str> = results
        .iter()
        .filter(|r| r.outcome.is_bad())
        .map(|r| r.name.as_str())
        .collect();
    line.push_str(&format!(" — {}", names.join(", ")));
    format!("{line}\n::error::{}\n", escape_data(&line))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(name: &str, outcome: Outcome, secs: f64) -> TaskResult {
        TaskResult {
            name: name.to_string(),
            outcome,
            duration: Duration::from_secs_f64(secs),
            color_idx: 0,
        }
    }

    #[test]
    fn escapes_property_values() {
        assert_eq!(escape_property("test:full"), "test%3Afull");
        assert_eq!(escape_property("a,b%\r\n"), "a%2Cb%25%0D%0A");
        assert_eq!(escape_data("a:b,c%\n"), "a:b,c%25%0A");
    }

    #[test]
    fn failed_block_groups_the_tail_and_annotates() {
        let tail = vec!["boom".to_string(), "  ::error file=a.ts::bad".to_string()];
        let block = settled(
            "test:full",
            Outcome::Failed(1),
            1,
            Duration::from_millis(533_690),
            &tail,
            12,
        );
        assert_eq!(
            block,
            "failed test:full exit=1 duration=533.69s\n\
             ::group::test:full\n\
             … 12 earlier lines omitted\n\
             boom\n\
             ::error file=a.ts::bad\n\
             ::endgroup::\n\
             ::error title=test%3Afull::failed in 533.69s (exit 1)\n"
        );
    }

    #[test]
    fn timeout_without_output_is_two_lines() {
        let block = settled(
            "seed:check",
            Outcome::Timeout,
            124,
            Duration::from_secs(900),
            &[],
            0,
        );
        assert_eq!(
            block,
            "timeout seed:check after=900.00s\n\
             ::error title=seed%3Acheck::timed out after 900.00s\n"
        );
    }

    #[test]
    fn passed_and_cancelled_are_one_line() {
        let d = Duration::from_millis(1500);
        assert_eq!(
            settled("lint", Outcome::Passed, 0, d, &[], 0),
            "passed lint duration=1.50s\n"
        );
        assert_eq!(
            settled("lint", Outcome::Cancelled, 0, d, &[], 0),
            "cancelled lint\n"
        );
    }

    #[test]
    fn child_groups_are_dropped_from_the_tail() {
        assert!(!keep_in_tail("::group::inner"));
        assert!(!keep_in_tail("\x1b[90m  ::endgroup::"));
        assert!(keep_in_tail("::error::kept"));
        assert!(keep_in_tail("plain"));
    }

    #[test]
    fn summary_annotates_only_on_failure() {
        let green = vec![
            result("lint", Outcome::Passed, 1.0),
            result("test", Outcome::Passed, 2.0),
        ];
        assert_eq!(
            summary(&green, Duration::from_millis(2250)),
            "2 tasks: 2 passed (2.25s)\n"
        );

        let red = vec![
            result("lint", Outcome::Passed, 1.0),
            result("test:full", Outcome::Failed(1), 2.0),
        ];
        assert_eq!(
            summary(&red, Duration::from_millis(660_250)),
            "2 tasks: 1 passed, 1 failed (660.25s) — test:full\n\
             ::error::2 tasks: 1 passed, 1 failed (660.25s) — test:full\n"
        );
    }
}
