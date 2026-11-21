//! Subprocess wrapper around GitHub's `gh` CLI.

use std::io::{self, ErrorKind, IsTerminal, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use local_common::json::{parse, JsonValue};
use local_common::{Colour, SPINNER};

use crate::model::{FileChange, ThreadCounts};

pub const PR_FIELDS: &str = "number,title,url,state,isDraft,author,baseRefName,headRefName,\
createdAt,updatedAt,mergedAt,closedAt,additions,deletions,changedFiles,mergeable,\
mergeStateStatus,reviewDecision,latestReviews,reviews,reviewRequests,labels,commits,\
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

fn parse_pr_url(url: &str) -> Option<(&str, &str, &str, u64)> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let mut parts = rest.split('/');
    let host = parts.next()?;
    let owner = parts.next()?;
    let repo = parts.next()?;
    if parts.next()? != "pull" {
        return None;
    }
    let number_str = parts.next()?.split(['?', '#']).next()?;
    let number: u64 = number_str.parse().ok()?;
    Some((host, owner, repo, number))
}

/// Build the argv for `gh api --paginate --slurp` to fetch every changed file
/// for a PR URL (`https://<host>/<owner>/<repo>/pull/<number>`).
pub fn build_files_api_args(url: &str) -> Option<Vec<String>> {
    let (host, owner, repo, number) = parse_pr_url(url)?;

    // Always pin the host: without `--hostname`, `gh api` falls back to its
    // default host (`GH_HOST`), which may serve an unrelated PR with this path.
    Some(vec![
        "api".to_string(),
        "--paginate".to_string(),
        "--slurp".to_string(),
        "--hostname".to_string(),
        host.to_string(),
        format!("repos/{owner}/{repo}/pulls/{number}/files?per_page=100"),
    ])
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

const THREADS_QUERY: &str =
    "query($owner: String!, $repo: String!, $number: Int!, $cursor: String) { \
  repository(owner: $owner, name: $repo) { \
    pullRequest(number: $number) { \
      reviewThreads(first: 100, after: $cursor) { \
        pageInfo { hasNextPage endCursor } \
        nodes { isResolved } \
      } \
    } \
  } \
}";

/// Upper bound on review-thread pages (100 threads each) fetched per PR.
const MAX_THREAD_PAGES: usize = 50;

/// Resolution counts for one page of review threads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadPage {
    pub resolved: usize,
    pub unresolved: usize,
    /// Cursor for the next page, `None` on the last page.
    pub next_cursor: Option<String>,
}

/// Build `gh api graphql` argv to fetch one page of review threads.
pub fn build_threads_graphql_args(url: &str, cursor: Option<&str>) -> Option<Vec<String>> {
    let (host, owner, repo, number) = parse_pr_url(url)?;
    // Always pin the host so a `GH_HOST` default cannot answer for another PR.
    let mut args = vec![
        "api".to_string(),
        "graphql".to_string(),
        "--hostname".to_string(),
        host.to_string(),
        "-f".to_string(),
        format!("query={THREADS_QUERY}"),
        "-f".to_string(),
        format!("owner={owner}"),
        "-f".to_string(),
        format!("repo={repo}"),
        "-F".to_string(),
        format!("number={number}"),
    ];
    if let Some(cursor) = cursor {
        args.push("-f".to_string());
        args.push(format!("cursor={cursor}"));
    }
    Some(args)
}

/// Parse one page of the review-thread query. Only threads that carry an
/// explicit `isResolved` boolean are counted, so a malformed node fails the
/// page instead of silently counting as unresolved.
pub fn parse_review_threads_page(raw: &str) -> Result<ThreadPage, String> {
    let root = parse(raw).map_err(|e| format!("failed to parse `gh api graphql` JSON: {e}"))?;
    let threads = root
        .get("data")
        .and_then(|d| d.get("repository"))
        .and_then(|r| r.get("pullRequest"))
        .and_then(|p| p.get("reviewThreads"))
        .ok_or_else(|| "missing `reviewThreads` in `gh api graphql` response".to_string())?;

    let nodes = threads
        .get("nodes")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "missing `nodes` in `reviewThreads` response".to_string())?;

    let (mut resolved, mut unresolved) = (0, 0);
    for node in nodes {
        match node.get("isResolved").and_then(JsonValue::as_bool) {
            Some(true) => resolved += 1,
            Some(false) => unresolved += 1,
            None => return Err("review thread without `isResolved` flag".to_string()),
        }
    }

    let page_info = threads.get("pageInfo");
    let has_next = page_info
        .and_then(|p| p.get("hasNextPage"))
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let next_cursor = if has_next {
        let cursor = page_info
            .and_then(|p| p.get("endCursor"))
            .and_then(JsonValue::as_str)
            .ok_or_else(|| "`hasNextPage` without `endCursor`".to_string())?;
        Some(cursor.to_string())
    } else {
        None
    };

    Ok(ThreadPage {
        resolved,
        unresolved,
        next_cursor,
    })
}

fn fetch_threads_page(url: &str, cursor: Option<&str>) -> Result<ThreadPage, String> {
    let args = build_threads_graphql_args(url, cursor)
        .ok_or_else(|| format!("cannot derive repository/PR number from URL: {url}"))?;
    let output = Command::new("gh")
        .args(&args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("failed to run `gh api graphql`: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("`gh api graphql` exited with {}", output.status)
        } else {
            stderr
        });
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|e| format!("`gh api graphql` emitted non-UTF-8 output: {e}"))?;
    parse_review_threads_page(&stdout)
}

/// Fetch review thread resolution counts for a PR via `gh api graphql`,
/// following the cursor through every page. Any failed page fails the whole
/// fetch, so callers never see a partial count.
pub fn fetch_review_threads(url: &str) -> Result<ThreadCounts, String> {
    let (mut resolved, mut unresolved) = (0, 0);
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_THREAD_PAGES {
        let page = fetch_threads_page(url, cursor.as_deref())?;
        resolved += page.resolved;
        unresolved += page.unresolved;
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => {
                return Ok(ThreadCounts {
                    total: resolved + unresolved,
                    resolved,
                    unresolved,
                })
            }
        }
    }
    Err(format!(
        "more than {MAX_THREAD_PAGES} pages of review threads"
    ))
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
                "--hostname",
                "github.com",
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
    fn threads_query_pins_host_and_cursor() {
        let first = build_threads_graphql_args("https://github.com/o/r/pull/7", None).unwrap();
        assert_eq!(&first[..4], ["api", "graphql", "--hostname", "github.com"]);
        assert!(first.iter().all(|a| !a.starts_with("cursor=")));

        let next =
            build_threads_graphql_args("https://ghe.example.com/o/r/pull/7", Some("abc")).unwrap();
        assert_eq!(&next[2..4], ["--hostname", "ghe.example.com"]);
        assert_eq!(&next[next.len() - 2..], ["-f", "cursor=abc"]);
    }

    #[test]
    fn parses_thread_page_with_next_cursor() {
        let raw = r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{
            "pageInfo":{"hasNextPage":true,"endCursor":"C1"},
            "nodes":[{"isResolved":true},{"isResolved":true},{"isResolved":false}]}}}}}"#;
        let page = parse_review_threads_page(raw).unwrap();
        assert_eq!(page.resolved, 2);
        assert_eq!(page.unresolved, 1);
        assert_eq!(page.next_cursor.as_deref(), Some("C1"));
    }

    #[test]
    fn thread_page_without_resolution_flag_is_an_error() {
        let raw = r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{
            "pageInfo":{"hasNextPage":false,"endCursor":null},
            "nodes":[{"isResolved":true},{}]}}}}}"#;
        assert!(parse_review_threads_page(raw).is_err());
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
