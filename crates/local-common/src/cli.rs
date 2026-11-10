//! Standardized zero-dependency CLI parsing helpers.
//!
//! Provides reusable primitives for common flags (`--help`, `--version`,
//! `--color`, `--no-color`), argument splitting (`--flag=value` vs `--flag value`),
//! and color initialization.

use std::io::IsTerminal;

use crate::term::{color_enabled_for, Colour};

/// Common output flags shared across CLIs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommonFlags {
    pub color: bool,
    pub no_color: bool,
}

impl CommonFlags {
    /// Resolve terminal-aware [`Colour`] according to `--color`, `--no-color`,
    /// `NO_COLOR` env var, and terminal stream status.
    pub fn resolve_colour(&self, stream: &impl IsTerminal) -> Colour {
        let enabled = if self.color {
            true
        } else if self.no_color {
            false
        } else {
            color_enabled_for(stream, false)
        };
        Colour::new(enabled)
    }

    /// Try to handle common color flags (`--color`, `--no-color`).
    /// Returns `true` if the argument was a common color flag.
    pub fn check_arg(&mut self, arg: &str) -> bool {
        match arg {
            "--color" => {
                self.color = true;
                self.no_color = false;
                true
            }
            "--no-color" => {
                self.no_color = true;
                self.color = false;
                true
            }
            _ => false,
        }
    }
}

/// Helper for consuming flag values whether given as `--flag value` or `--flag=value`.
pub struct ArgCursor<I: Iterator<Item = String>> {
    iter: I,
}

impl<I: Iterator<Item = String>> ArgCursor<I> {
    pub fn new(iter: I) -> Self {
        Self { iter }
    }

    /// Pull the value for `flag` from either an inline `--flag=val` match or
    /// by advancing to the next argument token.
    ///
    /// If inline is `Some(val)`, returns `Ok(val.to_string())`.
    /// Otherwise tries `self.next()`, erroring with a descriptive message if absent.
    pub fn require_value(&mut self, flag: &str, inline: Option<&str>) -> Result<String, String> {
        if let Some(val) = inline {
            return Ok(val.to_string());
        }
        self.iter
            .next()
            .ok_or_else(|| format!("{flag} requires a value"))
    }
}

impl<I: Iterator<Item = String>> Iterator for ArgCursor<I> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}

/// Split an argument token into `(base_flag, inline_value)`.
///
/// For example:
/// - `"--foo=bar"` -> `("--foo", Some("bar"))`
/// - `"-f=bar"` -> `("-f", Some("bar"))`
/// - `"--foo"` -> `("--foo", None)`
/// - `"-f"` -> `("-f", None)`
pub fn split_flag(arg: &str) -> (&str, Option<&str>) {
    if arg.starts_with('-') {
        if let Some((flag, val)) = arg.split_once('=') {
            if !flag.is_empty() && flag != "-" && flag != "--" {
                return (flag, Some(val));
            }
        }
    }
    (arg, None)
}

/// The closest candidate to a mistyped `input`, for "did you mean" hints.
/// Only close matches count: at most a third of the input's length in edits.
pub fn suggest<'a>(input: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let limit = (input.chars().count() / 3).max(1);
    candidates
        .into_iter()
        .map(|c| (edit_distance(input, c), c))
        .filter(|&(d, _)| d <= limit)
        .min_by_key(|&(d, _)| d)
        .map(|(_, c)| c)
}

/// Edit distance over chars, counting a swap of two neighbours as one edit
/// (optimal string alignment), so `tset` is one edit from `test`.
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    d[0] = (0..=b.len()).collect();
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j - 1] + cost)
                .min(d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggest_picks_close_matches_only() {
        let names = ["test", "typecheck", "lint"];
        assert_eq!(suggest("tset", names), Some("test"));
        assert_eq!(suggest("lnt", names), Some("lint"));
        assert_eq!(suggest("deploy", names), None);
        assert_eq!(suggest("--jbos", ["--jobs", "--bail"]), Some("--jobs"));
    }

    #[test]
    fn split_flag_splits_equals_syntax() {
        assert_eq!(split_flag("--since=main"), ("--since", Some("main")));
        assert_eq!(split_flag("--since"), ("--since", None));
        assert_eq!(split_flag("-s"), ("-s", None));
        assert_eq!(split_flag("plain"), ("plain", None));
    }

    #[test]
    fn cursor_require_value_inline_and_next() {
        let args = vec!["arg1".to_string(), "val2".to_string()];
        let mut cursor = ArgCursor::new(args.into_iter());

        // Inline value returns directly without consuming iterator
        let val1 = cursor.require_value("--test", Some("inline")).unwrap();
        assert_eq!(val1, "inline");

        // Next consumes from iterator
        let val2 = cursor.require_value("--test2", None).unwrap();
        assert_eq!(val2, "arg1");

        let val3 = cursor.require_value("--test3", None).unwrap();
        assert_eq!(val3, "val2");

        // Empty returns error
        assert!(cursor.require_value("--test4", None).is_err());
    }

    #[test]
    fn common_flags_check() {
        let mut flags = CommonFlags::default();
        assert!(!flags.color && !flags.no_color);

        assert!(flags.check_arg("--color"));
        assert!(flags.color && !flags.no_color);

        assert!(flags.check_arg("--no-color"));
        assert!(flags.no_color && !flags.color);

        assert!(flags.check_arg("--color"));
        assert!(flags.color && !flags.no_color);

        assert!(!flags.check_arg("--other"));
    }
}
