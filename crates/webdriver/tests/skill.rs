//! Keeps `skills/webdriver/SKILL.md` in step with `webdriver --help`: every
//! verb and long flag must appear in both, so neither can drift.

use local_common::term::Colour;
use std::collections::BTreeSet;

const SKILL: &str = include_str!("../../../skills/webdriver/SKILL.md");

fn help() -> String {
    let mut buf = Vec::new();
    webdriver::cli::print_help(&mut buf, &Colour::new(false));
    String::from_utf8(buf).expect("help is UTF-8")
}

/// Lines between `start` (exclusive) and the next line matching `is_end`.
fn section<'a>(text: &'a str, start: &str, is_end: fn(&str) -> bool) -> Vec<&'a str> {
    text.lines()
        .skip_while(|l| l.trim() != start)
        .skip(1)
        .take_while(|l| !is_end(l))
        .collect()
}

fn long_flags(spec: &str) -> impl Iterator<Item = String> + '_ {
    spec.split_whitespace()
        .filter(|w| w.starts_with("--"))
        .map(|w| w.trim_end_matches(',').to_string())
}

fn help_verbs(help: &str) -> BTreeSet<String> {
    let mut verbs = BTreeSet::new();
    for line in section(help, "VERBS & ACTIONS:", |l| l.trim().is_empty()) {
        let words: Vec<_> = line.split_whitespace().collect();
        verbs.insert(words[0].to_string());
        // `url / title` documents two verbs on one line.
        if words.get(1) == Some(&"/") {
            verbs.insert(words[2].to_string());
        }
    }
    verbs
}

fn help_flags(help: &str) -> BTreeSet<String> {
    section(help, "OPTIONS:", |l| l.trim().is_empty())
        .into_iter()
        // Wrapped description lines are indented deeper and may start with a flag.
        .filter(|l| l.len() - l.trim_start().len() <= 8)
        .map(str::trim)
        .filter(|l| l.starts_with('-'))
        .flat_map(|l| long_flags(l.split("  ").next().unwrap_or_default()).collect::<Vec<_>>())
        .collect()
}

/// First code span of each `- ` bullet under a `## heading`.
fn skill_items(heading: &str) -> Vec<&'static str> {
    section(SKILL, heading, |l| l.starts_with("## "))
        .into_iter()
        .filter_map(|l| l.strip_prefix("- `"))
        .filter_map(|l| l.split('`').next())
        .collect()
}

#[test]
fn skill_verbs_match_help() {
    let skill: BTreeSet<_> = skill_items("## Verbs")
        .iter()
        .map(|s| s.split_whitespace().next().unwrap_or_default().to_string())
        .collect();
    assert_eq!(skill, help_verbs(&help()), "SKILL.md verbs vs --help verbs");
}

#[test]
fn skill_flags_match_help() {
    let skill: BTreeSet<_> = skill_items("## Options")
        .into_iter()
        .flat_map(long_flags)
        .collect();
    assert_eq!(
        skill,
        help_flags(&help()),
        "SKILL.md options vs --help options"
    );
}
