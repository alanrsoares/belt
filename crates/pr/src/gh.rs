//! Subprocess wrapper around GitHub's `gh` CLI.

use std::io::ErrorKind;
use std::process::Command;

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

/// Fetch raw JSON for a PR via `gh pr view`.
pub fn fetch_pr_json(target: Option<&str>, repo: Option<&str>) -> Result<String, String> {
    let args = build_view_args(target, repo, false);
    let output = Command::new("gh").args(&args).output().map_err(|e| {
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

/// Open a PR in the user's default browser via `gh pr view --web`.
pub fn open_pr_web(target: Option<&str>, repo: Option<&str>) -> Result<(), String> {
    let args = build_view_args(target, repo, true);
    let output = Command::new("gh").args(&args).output().map_err(|e| {
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
}
