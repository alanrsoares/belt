//! Subprocess wrapper around GitHub's `gh` CLI.

use std::io::{self, ErrorKind, IsTerminal, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use local_common::json::{parse, JsonValue};
use local_common::{Colour, SPINNER};

use crate::model::FileChange;

pub const PR_FIELDS: &str = "number,title,url,state,isDraft,author,baseRefName,headRefName,\
createdAt,updatedAt,mergedAt,closedAt,additions,deletions,changedFiles,mergeable,\
mergeStateStatus,reviewDecision,latestReviews,reviewRequests,labels,commits,\
statusCheckRollup,files,body,comments";

/// Build the argv for `gh pr view`.
pub fn build_view_args(target: Option<&str>, repo: Option<&str>, web: bool) -> Vec<String> {
    let mut args = vec!["pr".to_string(), "view".to_string()];
    if let Some(t) = target {
        args.push(t.to_string());
    }
    if let Some(r) = repo {
        args.push("--repo".to_string());
        args.push(r.to_string());
    }
    if web {
        args.push("--web".to_string());
    } else {
        args.push("--json".to_string());
        args.push(PR_FIELDS.to_string());
    }
    args
}

/// Fetch raw JSON for a PR via `gh pr view`, showing a TTY spinner on stderr.
pub fn fetch_pr_json(
    target: Option<&str>,
    repo: Option<&str>,
    c: &Colour,
) -> Result<String, String> {
    let args = build_view_args(target, repo, false);
    let show_spinner = io::stderr().is_terminal();
    let done = Arc::new(AtomicBool::new(false));

    let spinner_handle = if show_spinner {
        let done_flag = Arc::clone(&done);
        let label = c.dim("Fetching PR…");
        Some(thread::spawn(move || {
            let mut err = io::stderr();
            let mut idx = 0usize;
            while !done_flag.load(Ordering::Relaxed) {
                let frame = SPINNER[idx % SPINNER.len()];
                let _ = write!(err, "\r\x1b[2K{frame} {label}");
                let _ = err.flush();
                idx = idx.wrapping_add(1);
                thread::sleep(Duration::from_millis(80));
            }
            let _ = write!(err, "\r\x1b[2K");
            let _ = err.flush();
        }))
    } else {
        None
    };

    let output = Command::new("gh").args(&args).stdin(Stdio::null()).output();

    done.store(true, Ordering::Relaxed);
    if let Some(handle) = spinner_handle {
        let _ = handle.join();
    }

    let output = output.map_err(|e| {
        if e.kind() == ErrorKind::NotFound {
            "`gh` CLI not found on PATH — install from https://cli.github.com/".to_string()
        } else {
            format!("failed to run `gh`: {e}")
        }
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err(format!("`gh pr view` exited with {}", output.status));
        }
        return Err(stderr);
    }

    String::from_utf8(output.stdout).map_err(|e| format!("`gh` emitted non-UTF-8 output: {e}"))
}

/// Build the argv for `gh api --paginate --slurp` to fetch every changed file
/// for a PR URL (`https://<host>/<owner>/<repo>/pull/<number>`).
pub fn build_files_api_args(url: &str) -> Option<Vec<String>> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.len() < 5 || parts[3] != "pull" {
        return None;
    }
    let host = parts[0];
    let owner = parts[1];
    let repo = parts[2];
    let number_str = parts[4].split(['?', '#']).next()?;
    let number: u64 = number_str.parse().ok()?;

    let mut args = vec![
        "api".to_string(),
        "--paginate".to_string(),
        "--slurp".to_string(),
    ];
    if !host.eq_ignore_ascii_case("github.com") {
        args.push("--hostname".to_string());
        args.push(host.to_string());
    }
    args.push(format!("repos/{owner}/{repo}/pulls/{number}/files?per_page=100"));
    Some(args)
}

/// Parse the JSON array (or slurped array of page arrays) returned by
/// `gh api --paginate --slurp repos/.../pulls/.../files`.
pub fn parse_paginated_files_json(raw: &str) -> Result<Vec<FileChange>, String> {
    let root = parse(raw).map_err(|e| format!("failed to parse `gh api` files JSON: {e}"))?;
    let items = root
        .as_array()
        .ok_or_else(|| "expected JSON array from `gh api` files endpoint".to_string())?;

    let mut files = Vec::new();
    for entry in items {
        if let Some(page) = entry.as_array() {
            for item in page {
                push_file_change(item, &mut files);
            }
        } else {
            push_file_change(entry, &mut files);
        }
    }
    Ok(files)
}

fn push_file_change(item: &JsonValue, out: &mut Vec<FileChange>) {
    let path = item
        .get("filename")
        .or_else(|| item.get("path"))
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    if path.is_empty() {
        return;
    }
    let additions = item
        .get("additions")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0)
        .max(0) as u64;
    let deletions = item
        .get("deletions")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0)
        .max(0) as u64;
    out.push(FileChange {
        path: path.to_string(),
        additions,
        deletions,
    });
}

/// Fetch all changed files for a PR via `gh api --paginate --slurp`.
pub fn fetch_all_pr_files(url: &str) -> Result<Vec<FileChange>, String> {
    let args = build_files_api_args(url)
        .ok_or_else(|| format!("cannot derive repository/PR number from URL: {url}"))?;
    let output = Command::new("gh")
        .args(&args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("failed to run `gh api`: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("`gh api` exited with {}", output.status)
        } else {
            stderr
        });
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|e| format!("`gh api` emitted non-UTF-8 output: {e}"))?;
    parse_paginated_files_json(&stdout)
}

/// Open a PR in the user's default browser via `gh pr view --web`.
pub fn open_pr_web(target: Option<&str>, repo: Option<&str>) -> Result<(), String> {
    let args = build_view_args(target, repo, true);
    let output = Command::new("gh")
        .args(&args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            if e.kind() == ErrorKind::NotFound {
                "`gh` CLI not found on PATH — install from https://cli.github.com/".to_string()
            } else {
                format!("failed to run `gh`: {e}")
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("`gh pr view --web` exited with {}", output.status)
        } else {
            stderr
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_default_cwd_args() {
        let args = build_view_args(None, None, false);
        assert_eq!(args[0], "pr");
        assert_eq!(args[1], "view");
        assert_eq!(args[2], "--json");
        assert_eq!(args[3], PR_FIELDS);
    }

    #[test]
    fn builds_targeted_repo_web_args() {
        let args = build_view_args(Some("42"), Some("acme/app"), true);
        assert_eq!(args, ["pr", "view", "42", "--repo", "acme/app", "--web"]);
    }

    #[test]
    fn builds_paginated_files_api_args_for_github_and_ghe() {
        let gh_args = build_files_api_args("https://github.com/alanrsoares/belt/pull/10").unwrap();
        assert_eq!(
            gh_args,
            [
                "api",
                "--paginate",
                "--slurp",
                "repos/alanrsoares/belt/pulls/10/files?per_page=100"
            ]
        );

        let ghe_args = build_files_api_args("https://ghe.example.com/org/repo/pull/99").unwrap();
        assert_eq!(
            ghe_args,
            [
                "api",
                "--paginate",
                "--slurp",
                "--hostname",
                "ghe.example.com",
                "repos/org/repo/pulls/99/files?per_page=100"
            ]
        );
    }

    #[test]
    fn parses_slurped_paginated_files_over_100() {
        let page1: Vec<String> = (1..=100)
            .map(|i| format!(r#"{{"filename":"src/f{i}.rs","additions":{i},"deletions":1}}"#))
            .collect();
        let page2 = r#"[{"filename":"src/f101.rs","additions":7,"deletions":2}]"#;
        let json = format!("[[{}],{}]", page1.join(","), page2);
        let files = parse_paginated_files_json(&json).unwrap();
        assert_eq!(files.len(), 101);
        assert_eq!(files[0].path, "src/f1.rs");
        assert_eq!(files[100].path, "src/f101.rs");
        assert_eq!(files[100].additions, 7);
        assert_eq!(files[100].deletions, 2);
    }
}
