//! Domain model for a GitHub pull request parsed from `gh pr view --json`.

use std::time::{SystemTime, UNIX_EPOCH};

use local_common::json::{parse, JsonValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mergeable {
    Ready,
    Conflicting,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeState {
    Clean,
    Dirty,
    Blocked,
    Behind,
    Unstable,
    HasHooks,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    pub author: String,
    pub state: ReviewState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CheckBucket {
    Fail,
    Pending,
    Cancel,
    Skip,
    Pass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub workflow: Option<String>,
    pub bucket: CheckBucket,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub oid: String,
    pub headline: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: PrState,
    pub author: String,
    pub base_ref: String,
    pub head_ref: String,
    pub created_at: String,
    pub updated_at: String,
    pub merged_at: Option<String>,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub mergeable: Mergeable,
    pub merge_state: MergeState,
    pub review_decision: ReviewDecision,
    pub reviews: Vec<Review>,
    pub review_requests: Vec<String>,
    pub labels: Vec<Label>,
    pub commits: Vec<Commit>,
    pub checks: Vec<Check>,
    pub files: Vec<FileChange>,
    pub body: String,
    pub comment_count: usize,
}

impl PullRequest {
    pub fn from_json_str(raw: &str) -> Result<Self, String> {
        let root = parse(raw).map_err(|e| format!("failed to parse `gh` JSON: {e}"))?;
        Self::from_json(&root)
    }

    pub fn from_json(v: &JsonValue) -> Result<Self, String> {
        let number = v
            .get("number")
            .and_then(JsonValue::as_i64)
            .ok_or_else(|| "missing `number` in `gh` output".to_string())?
            .max(0) as u64;

        let title = str_field(v, "title").unwrap_or_default();
        let url = str_field(v, "url").unwrap_or_default();
        let is_draft = v
            .get("isDraft")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        let raw_state = str_field(v, "state").unwrap_or_else(|| "OPEN".to_string());
        let state = match raw_state.as_str() {
            "MERGED" => PrState::Merged,
            "CLOSED" => PrState::Closed,
            _ if is_draft => PrState::Draft,
            _ => PrState::Open,
        };

        let author = v
            .get("author")
            .and_then(|a| str_field(a, "login").or_else(|| str_field(a, "name")))
            .unwrap_or_else(|| "ghost".to_string());

        let base_ref = str_field(v, "baseRefName").unwrap_or_default();
        let head_ref = str_field(v, "headRefName").unwrap_or_default();
        let created_at = str_field(v, "createdAt").unwrap_or_default();
        let updated_at = str_field(v, "updatedAt").unwrap_or_default();
        let merged_at = str_field(v, "mergedAt").filter(|s| !s.is_empty());

        let additions = u64_field(v, "additions");
        let deletions = u64_field(v, "deletions");
        let changed_files = u64_field(v, "changedFiles");

        let mergeable = match str_field(v, "mergeable").as_deref() {
            Some("MERGEABLE") => Mergeable::Ready,
            Some("CONFLICTING") => Mergeable::Conflicting,
            _ => Mergeable::Unknown,
        };

        let merge_state = match str_field(v, "mergeStateStatus").as_deref() {
            Some("CLEAN") => MergeState::Clean,
            Some("DIRTY") => MergeState::Dirty,
            Some("BLOCKED") => MergeState::Blocked,
            Some("BEHIND") => MergeState::Behind,
            Some("UNSTABLE") => MergeState::Unstable,
            Some("HAS_HOOKS") => MergeState::HasHooks,
            _ => MergeState::Unknown,
        };

        let review_decision = match str_field(v, "reviewDecision").as_deref() {
            Some("APPROVED") => ReviewDecision::Approved,
            Some("CHANGES_REQUESTED") => ReviewDecision::ChangesRequested,
            Some("REVIEW_REQUIRED") => ReviewDecision::ReviewRequired,
            _ => ReviewDecision::None,
        };

        let reviews = parse_reviews(v.get("latestReviews"));
        let review_requests = parse_review_requests(v.get("reviewRequests"));
        let labels = parse_labels(v.get("labels"));
        let commits = parse_commits(v.get("commits"));
        let checks = parse_checks(v.get("statusCheckRollup"));
        let files = parse_files(v.get("files"));
        let body = str_field(v, "body").unwrap_or_default();
        let comment_count = v
            .get("comments")
            .and_then(JsonValue::as_array)
            .map_or(0, |a| a.len());

        Ok(Self {
            number,
            title,
            url,
            state,
            author,
            base_ref,
            head_ref,
            created_at,
            updated_at,
            merged_at,
            additions,
            deletions,
            changed_files,
            mergeable,
            merge_state,
            review_decision,
            reviews,
            review_requests,
            labels,
            commits,
            checks,
            files,
            body,
            comment_count,
        })
    }
}

fn str_field(v: &JsonValue, key: &str) -> Option<String> {
    v.get(key)
        .and_then(JsonValue::as_str)
        .map(ToString::to_string)
}

fn u64_field(v: &JsonValue, key: &str) -> u64 {
    v.get(key)
        .and_then(JsonValue::as_i64)
        .map_or(0, |n| n.max(0) as u64)
}

fn parse_reviews(v: Option<&JsonValue>) -> Vec<Review> {
    let Some(arr) = v.and_then(JsonValue::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| {
            let author = item
                .get("author")
                .and_then(|a| str_field(a, "login"))
                .unwrap_or_else(|| "ghost".to_string());
            let state = match str_field(item, "state").as_deref() {
                Some("APPROVED") => ReviewState::Approved,
                Some("CHANGES_REQUESTED") => ReviewState::ChangesRequested,
                Some("COMMENTED") => ReviewState::Commented,
                Some("DISMISSED") => ReviewState::Dismissed,
                Some("PENDING") => ReviewState::Pending,
                _ => return None,
            };
            Some(Review { author, state })
        })
        .collect()
}

fn parse_review_requests(v: Option<&JsonValue>) -> Vec<String> {
    let Some(arr) = v.and_then(JsonValue::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| {
            str_field(item, "login")
                .or_else(|| str_field(item, "slug"))
                .or_else(|| str_field(item, "name"))
        })
        .collect()
}

fn parse_labels(v: Option<&JsonValue>) -> Vec<Label> {
    let Some(arr) = v.and_then(JsonValue::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| {
            let name = str_field(item, "name")?;
            let color = str_field(item, "color").unwrap_or_default();
            Some(Label { name, color })
        })
        .collect()
}

fn parse_commits(v: Option<&JsonValue>) -> Vec<Commit> {
    let Some(arr) = v.and_then(JsonValue::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| {
            let oid = str_field(item, "oid")?;
            let headline = str_field(item, "messageHeadline").unwrap_or_default();
            Some(Commit { oid, headline })
        })
        .collect()
}

fn parse_checks(v: Option<&JsonValue>) -> Vec<Check> {
    let Some(arr) = v.and_then(JsonValue::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| {
            // CheckRun has `name` + `status` + `conclusion`; StatusContext has `context` + `state`.
            if let Some(name) = str_field(item, "name") {
                let status = str_field(item, "status").unwrap_or_default();
                let conclusion = str_field(item, "conclusion").unwrap_or_default();
                let bucket = classify_check_run(&status, &conclusion);
                let workflow = str_field(item, "workflowName").filter(|s| !s.is_empty());
                let url = str_field(item, "detailsUrl").filter(|s| !s.is_empty());
                Some(Check {
                    name,
                    workflow,
                    bucket,
                    url,
                })
            } else if let Some(context) = str_field(item, "context") {
                let state = str_field(item, "state").unwrap_or_default();
                let bucket = match state.as_str() {
                    "SUCCESS" => CheckBucket::Pass,
                    "FAILURE" | "ERROR" => CheckBucket::Fail,
                    _ => CheckBucket::Pending,
                };
                let url = str_field(item, "targetUrl").filter(|s| !s.is_empty());
                Some(Check {
                    name: context,
                    workflow: None,
                    bucket,
                    url,
                })
            } else {
                None
            }
        })
        .collect()
}

fn classify_check_run(status: &str, conclusion: &str) -> CheckBucket {
    if status != "COMPLETED" && !status.is_empty() && conclusion.is_empty() {
        return CheckBucket::Pending;
    }
    match conclusion {
        "SUCCESS" | "NEUTRAL" => CheckBucket::Pass,
        "SKIPPED" => CheckBucket::Skip,
        "CANCELLED" => CheckBucket::Cancel,
        "FAILURE" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE" | "STALE" => {
            CheckBucket::Fail
        }
        "" => CheckBucket::Pending,
        _ => CheckBucket::Fail,
    }
}

fn parse_files(v: Option<&JsonValue>) -> Vec<FileChange> {
    let Some(arr) = v.and_then(JsonValue::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| {
            let path = str_field(item, "path")?;
            let additions = u64_field(item, "additions");
            let deletions = u64_field(item, "deletions");
            Some(FileChange {
                path,
                additions,
                deletions,
            })
        })
        .collect()
}

pub fn now_epoch_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Parse an ISO-8601 UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`) into Unix epoch seconds.
pub fn parse_iso_utc(s: &str) -> Option<i64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut dp = date.split('-');
    let year: i64 = dp.next()?.parse().ok()?;
    let month: i64 = dp.next()?.parse().ok()?;
    let day: i64 = dp.next()?.parse().ok()?;
    if dp.next().is_some() {
        return None;
    }

    let time = time.split('.').next().unwrap_or(time);
    let mut tp = time.split(':');
    let hour: i64 = tp.next()?.parse().ok()?;
    let min: i64 = tp.next()?.parse().ok()?;
    let sec: i64 = tp.next()?.parse().ok()?;
    if tp.next().is_some() {
        return None;
    }

    let days = days_from_civil(year, month, day)?;
    Some(days * 86_400 + hour * 3_600 + min * 60 + sec)
}

/// Howard Hinnant's civil-to-days algorithm (days since 1970-01-01).
fn days_from_civil(y: i64, m: i64, d: i64) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// Format an ISO-8601 UTC timestamp relative to `now_secs` (`just now`, `14m ago`, `3h ago`, `5d ago`).
pub fn relative_time(iso: &str, now_secs: i64) -> String {
    let Some(ts) = parse_iso_utc(iso) else {
        return iso.to_string();
    };
    let delta = (now_secs - ts).max(0);
    if delta < 60 {
        "just now".to_string()
    } else if delta < 3_600 {
        format!("{}m ago", delta / 60)
    } else if delta < 86_400 {
        format!("{}h ago", delta / 3_600)
    } else if delta < 86_400 * 30 {
        format!("{}d ago", delta / 86_400)
    } else if delta < 86_400 * 365 {
        format!("{}mo ago", delta / (86_400 * 30))
    } else {
        format!("{}y ago", delta / (86_400 * 365))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_JSON: &str = r#"{
        "number": 142,
        "title": "feat(pr): add terminal PR overview",
        "url": "https://github.com/alanrsoares/belt/pull/142",
        "state": "OPEN",
        "isDraft": false,
        "author": { "login": "alanrsoares" },
        "baseRefName": "main",
        "headRefName": "feat/pr-tool",
        "createdAt": "2026-11-18T01:00:00Z",
        "updatedAt": "2026-11-18T03:00:00Z",
        "mergedAt": null,
        "additions": 320,
        "deletions": 45,
        "changedFiles": 6,
        "mergeable": "MERGEABLE",
        "mergeStateStatus": "CLEAN",
        "reviewDecision": "APPROVED",
        "latestReviews": [
            { "author": { "login": "alice" }, "state": "APPROVED" },
            { "author": { "login": "bob" }, "state": "COMMENTED" }
        ],
        "reviewRequests": [
            { "login": "carol" }
        ],
        "labels": [
            { "name": "enhancement", "color": "a2eeef" }
        ],
        "commits": [
            { "oid": "abcdef123456", "messageHeadline": "feat(pr): scaffold crate" }
        ],
        "statusCheckRollup": [
            { "name": "test (ubuntu)", "status": "COMPLETED", "conclusion": "SUCCESS", "workflowName": "CI" },
            { "name": "clippy", "status": "COMPLETED", "conclusion": "FAILURE", "workflowName": "CI" },
            { "context": "deploy/preview", "state": "PENDING" }
        ],
        "files": [
            { "path": "crates/pr/src/lib.rs", "additions": 120, "deletions": 0 }
        ],
        "body": "Adds `pr` tool.\n\n- Infers PR from cwd",
        "comments": [{ "id": "1" }, { "id": "2" }]
    }"#;

    #[test]
    fn parses_sample_pr_json() {
        let pr = PullRequest::from_json_str(SAMPLE_JSON).unwrap();
        assert_eq!(pr.number, 142);
        assert_eq!(pr.state, PrState::Open);
        assert_eq!(pr.author, "alanrsoares");
        assert_eq!(pr.mergeable, Mergeable::Ready);
        assert_eq!(pr.review_decision, ReviewDecision::Approved);
        assert_eq!(pr.reviews.len(), 2);
        assert_eq!(pr.review_requests, vec!["carol"]);
        assert_eq!(pr.checks.len(), 3);
        assert_eq!(pr.checks[0].bucket, CheckBucket::Pass);
        assert_eq!(pr.checks[1].bucket, CheckBucket::Fail);
        assert_eq!(pr.checks[2].bucket, CheckBucket::Pending);
        assert_eq!(pr.comment_count, 2);
    }

    #[test]
    fn formats_relative_time() {
        let base = parse_iso_utc("2026-11-18T12:00:00Z").unwrap();
        assert_eq!(relative_time("2026-11-18T11:59:30Z", base), "just now");
        assert_eq!(relative_time("2026-11-18T11:45:00Z", base), "15m ago");
        assert_eq!(relative_time("2026-11-18T09:00:00Z", base), "3h ago");
        assert_eq!(relative_time("2026-11-15T12:00:00Z", base), "3d ago");
    }
}
