//! Terminal rendering for `pr`.

use std::io::Write;

use local_common::{draw_meter, visible_width, Colour, Glyphs};

use crate::model::{
    relative_time, Check, CheckBucket, FileChange, MergeState, Mergeable, PrState, PullRequest,
    ReviewDecision, ReviewState,
};

#[derive(Debug, Clone, Copy)]
pub struct RenderOpts {
    pub width: usize,
    pub compact: bool,
    pub all_files: bool,
    pub full_body: bool,
    pub all_checks: bool,
    pub now_secs: i64,
    pub glyphs: Glyphs,
}

const MAX_DEFAULT_COMMITS: usize = 5;
const MAX_DEFAULT_FILES: usize = 10;
const MAX_PREVIEW_BODY_LINES: usize = 10;

pub fn render_pr(
    pr: &PullRequest,
    c: &Colour,
    opts: RenderOpts,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let width = opts.width.clamp(52, 104);
    let g = &opts.glyphs;
    render_header(pr, c, g, width, opts.now_secs, out)?;
    render_status_strip(pr, c, g, out)?;
    render_reviews(pr, c, g, width, out)?;
    render_checks(pr, c, g, width, opts.all_checks, out)?;

    if !opts.compact {
        render_commits(pr, c, g, width, out)?;
        render_files(pr, c, g, width, opts.all_files, out)?;
        render_body(pr, c, width, opts.full_body, out)?;
    }

    Ok(())
}

fn render_header(
    pr: &PullRequest,
    c: &Colour,
    g: &Glyphs,
    width: usize,
    now_secs: i64,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let inner = width.saturating_sub(4);
    let badge = state_badge(pr.state, c, g);
    let num = c.link(format!("#{}", pr.number), &pr.url);

    let top_rule = "─".repeat(width.saturating_sub(2));
    writeln!(out, "{}", c.gray(format!("╭{top_rule}╮")))?;

    // Title line(s): badge + #num + wrapped title.
    let prefix = format!("{badge}  {} ", c.cyan(&num));
    let prefix_w = visible_width(&prefix);
    let title_lines = wrap_text(&pr.title, inner.saturating_sub(prefix_w).max(16));
    for (i, line) in title_lines.iter().enumerate() {
        let content = if i == 0 {
            format!("{prefix}{}", c.bold(line))
        } else {
            format!("{}{}", " ".repeat(prefix_w), c.bold(line))
        };
        write_box_line(&content, inner, c, out)?;
    }

    // Metadata line: @author · branch head -> base · age · comments.
    let age = relative_time(pr.merged_at.as_deref().unwrap_or(&pr.updated_at), now_secs);
    let mut meta = format!(
        "{} {} {}{} → {} {} {}",
        c.bold(format!("@{}", pr.author)),
        c.gray("·"),
        c.cyan(g.branch()),
        c.cyan(&pr.head_ref),
        c.dim(&pr.base_ref),
        c.gray("·"),
        c.dim(age),
    );
    if pr.comment_count > 0 {
        meta.push_str(&format!(
            " {} {}",
            c.gray("·"),
            c.dim(format!("{} comments", pr.comment_count))
        ));
    }
    for line in wrap_plain_or_styled(&meta, inner) {
        write_box_line(&line, inner, c, out)?;
    }

    // Labels (if any).
    if !pr.labels.is_empty() {
        let labels = pr
            .labels
            .iter()
            .map(|l| c.yellow(format!("[{}]", l.name)))
            .collect::<Vec<_>>()
            .join(" ");
        write_box_line(&labels, inner, c, out)?;
    }

    // URL line.
    if !pr.url.is_empty() {
        let url_line = c.dim(c.link(&pr.url, &pr.url));
        write_box_line(&url_line, inner, c, out)?;
    }

    writeln!(out, "{}", c.gray(format!("╰{top_rule}╯")))?;
    Ok(())
}

fn write_box_line(
    content: &str,
    inner: usize,
    c: &Colour,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let w = visible_width(content);
    let pad = inner.saturating_sub(w);
    writeln!(
        out,
        "{} {content}{} {}",
        c.gray("│"),
        " ".repeat(pad),
        c.gray("│")
    )
}

fn state_badge(state: PrState, c: &Colour, g: &Glyphs) -> String {
    match state {
        PrState::Open => c.green(format!("{} OPEN", g.pr_open())),
        PrState::Draft => c.gray(format!("{} DRAFT", g.pr_draft())),
        PrState::Merged => c.cyan(format!("{} MERGED", g.pr_merged())),
        PrState::Closed => c.red(format!("{} CLOSED", g.pr_closed())),
    }
}

fn render_status_strip(
    pr: &PullRequest,
    c: &Colour,
    g: &Glyphs,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let diff_bar = diff_blocks(pr.additions, pr.deletions, 5, c);
    let diffstat = format!(
        "{} {} {} {} {}",
        c.green(format!("+{}", pr.additions)),
        c.red(format!("-{}", pr.deletions)),
        diff_bar,
        c.gray("·"),
        c.dim(format!(
            "{} {}",
            pr.changed_files,
            if pr.changed_files == 1 {
                "file"
            } else {
                "files"
            }
        ))
    );

    let merge = match (pr.state, pr.mergeable, pr.merge_state) {
        (PrState::Merged, _, _) => c.cyan(format!("{} merged", g.pr_merged())),
        (PrState::Closed, _, _) => c.gray("— closed"),
        (_, Mergeable::Conflicting, _) | (_, _, MergeState::Dirty) => {
            c.red(format!("{} conflicts", g.err()))
        }
        (_, _, MergeState::Behind) => c.yellow(format!("{} behind", g.warn())),
        (_, _, MergeState::Blocked) => c.yellow(format!("{} blocked", g.pending())),
        (_, _, MergeState::Unstable) => c.yellow(format!("{} unstable", g.warn())),
        (_, Mergeable::Ready, _) => c.green(format!("{} mergeable", g.ok())),
        _ => c.gray("… merge unknown"),
    };

    let review = match pr.review_decision {
        ReviewDecision::Approved => c.green(format!("{} approved", g.ok())),
        ReviewDecision::ChangesRequested => c.red(format!("{} changes requested", g.err())),
        ReviewDecision::ReviewRequired => c.yellow(format!("{} review required", g.pending())),
        ReviewDecision::None => c.dim(format!("{} no review gate", g.pending())),
    };

    writeln!(
        out,
        "  {diffstat}   {}   {merge}   {}   {review}",
        c.gray("│"),
        c.gray("│")
    )?;
    Ok(())
}

fn diff_blocks(add: u64, del: u64, slots: usize, c: &Colour) -> String {
    let total = add + del;
    if total == 0 {
        return c.gray("▱".repeat(slots));
    }
    let green_slots = ((add as f64 / total as f64) * slots as f64).round() as usize;
    let green_slots = green_slots.min(slots);
    let red_slots = slots.saturating_sub(green_slots);
    format!(
        "{}{}",
        c.green("▰".repeat(green_slots)),
        c.red("▰".repeat(red_slots))
    )
}

fn render_section_header(
    title: &str,
    width: usize,
    c: &Colour,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let prefix = format!("── {title} ");
    let fill = width.saturating_sub(visible_width(&prefix));
    writeln!(out)?;
    writeln!(out, "{}{}", c.bold(&prefix), c.gray("─".repeat(fill)))?;
    Ok(())
}

fn render_reviews(
    pr: &PullRequest,
    c: &Colour,
    g: &Glyphs,
    width: usize,
    out: &mut impl Write,
) -> std::io::Result<()> {
    if pr.reviews.is_empty() && pr.review_requests.is_empty() {
        return Ok(());
    }
    render_section_header("Reviews", width, c, out)?;

    let mut items = Vec::new();
    for r in &pr.reviews {
        let icon = match r.state {
            ReviewState::Approved => c.green(g.ok()),
            ReviewState::ChangesRequested => c.red(g.err()),
            ReviewState::Commented => c.cyan(g.comment()),
            ReviewState::Dismissed => c.gray("−"),
            ReviewState::Pending => c.yellow(g.pending()),
        };
        items.push(format!("{icon} @{}", r.author));
    }
    for req in &pr.review_requests {
        items.push(format!("{} @{req}", c.yellow(g.pending())));
    }

    writeln!(out, "  {}", items.join("   "))?;
    Ok(())
}

fn render_checks(
    pr: &PullRequest,
    c: &Colour,
    g: &Glyphs,
    width: usize,
    all_checks: bool,
    out: &mut impl Write,
) -> std::io::Result<()> {
    if pr.checks.is_empty() {
        return Ok(());
    }

    let total = pr.checks.len();
    let passed = pr
        .checks
        .iter()
        .filter(|ch| ch.bucket == CheckBucket::Pass)
        .count();
    let failed = pr
        .checks
        .iter()
        .filter(|ch| ch.bucket == CheckBucket::Fail)
        .count();
    let pending = pr
        .checks
        .iter()
        .filter(|ch| ch.bucket == CheckBucket::Pending)
        .count();
    let skipped = pr
        .checks
        .iter()
        .filter(|ch| matches!(ch.bucket, CheckBucket::Skip | CheckBucket::Cancel))
        .count();

    render_section_header(&format!("Checks ({passed}/{total})"), width, c, out)?;

    let ratio = passed as f64 / total as f64;
    let bar_color = if failed > 0 {
        "\x1b[31m"
    } else if pending > 0 {
        "\x1b[33m"
    } else {
        "\x1b[32m"
    };
    let bar = if c.bold("x") == "x" {
        draw_meter(ratio, 18, "", "", "")
    } else {
        draw_meter(ratio, 18, bar_color, "\x1b[0m", "\x1b[90m")
    };

    let mut summary_parts = vec![c.green(format!("{passed} passed"))];
    if failed > 0 {
        summary_parts.push(c.red(format!("{failed} failed")));
    }
    if pending > 0 {
        summary_parts.push(c.yellow(format!("{pending} pending")));
    }
    if skipped > 0 {
        summary_parts.push(c.gray(format!("{skipped} skipped")));
    }
    writeln!(out, "  {bar}  {}", summary_parts.join(&c.gray(" · ")))?;

    let mut sorted: Vec<&Check> = pr.checks.iter().collect();
    sorted.sort_by_key(|ch| ch.bucket);

    for ch in sorted {
        let should_show =
            all_checks || matches!(ch.bucket, CheckBucket::Fail | CheckBucket::Pending);
        if !should_show {
            continue;
        }
        let icon = match ch.bucket {
            CheckBucket::Pass => c.green(g.ok()),
            CheckBucket::Fail => c.red(g.err()),
            CheckBucket::Pending => c.yellow(g.pending()),
            CheckBucket::Skip | CheckBucket::Cancel => c.gray(g.cancel()),
        };
        let name = match &ch.url {
            Some(u) => c.link(&ch.name, u),
            None => ch.name.clone(),
        };
        let wf = ch
            .workflow
            .as_deref()
            .map(|w| format!(" {}", c.dim(format!("({w})"))))
            .unwrap_or_default();
        writeln!(out, "  {icon} {name}{wf}")?;
    }

    Ok(())
}

fn render_commits(
    pr: &PullRequest,
    c: &Colour,
    g: &Glyphs,
    width: usize,
    out: &mut impl Write,
) -> std::io::Result<()> {
    if pr.commits.is_empty() {
        return Ok(());
    }
    render_section_header(&format!("Commits ({})", pr.commits.len()), width, c, out)?;

    let shown = pr.commits.len().min(MAX_DEFAULT_COMMITS);
    let max_msg = width.saturating_sub(16);
    for cm in pr.commits.iter().take(shown) {
        let short_oid: String = cm.oid.chars().take(7).collect();
        let msg = truncate_ellipsis(&cm.headline, max_msg);
        writeln!(
            out,
            "  {} {}  {msg}",
            c.dim(g.commit()),
            c.yellow(short_oid)
        )?;
    }
    if pr.commits.len() > shown {
        writeln!(
            out,
            "  {}",
            c.dim(format!("… and {} more commits", pr.commits.len() - shown))
        )?;
    }
    Ok(())
}

fn render_files(
    pr: &PullRequest,
    c: &Colour,
    g: &Glyphs,
    width: usize,
    all_files: bool,
    out: &mut impl Write,
) -> std::io::Result<()> {
    if pr.files.is_empty() {
        return Ok(());
    }
    render_section_header(&format!("Files ({})", pr.files.len()), width, c, out)?;

    let mut files: Vec<&FileChange> = pr.files.iter().collect();
    files.sort_by_key(|a| std::cmp::Reverse(a.additions + a.deletions));

    let limit = if all_files {
        files.len()
    } else {
        files.len().min(MAX_DEFAULT_FILES)
    };
    let max_delta = files
        .iter()
        .take(limit)
        .map(|f| f.additions + f.deletions)
        .max()
        .unwrap_or(1)
        .max(1);

    let path_col = width
        .saturating_sub(28 + g.file().chars().count())
        .clamp(20, 56);
    for f in files.iter().take(limit) {
        let path = truncate_left(&f.path, path_col);
        let pad = path_col.saturating_sub(path.chars().count());
        let bar = file_bar(f.additions, f.deletions, max_delta, 10, c);
        let counts = format!(
            "{} {}",
            c.green(format!("+{:>3}", f.additions)),
            c.red(format!("-{:>3}", f.deletions))
        );
        writeln!(
            out,
            "  {}{path}{}  {counts}  {bar}",
            c.dim(g.file()),
            " ".repeat(pad)
        )?;
    }
    if files.len() > limit {
        writeln!(
            out,
            "  {}",
            c.dim(format!(
                "… and {} more files (pass --files to show all)",
                files.len() - limit
            ))
        )?;
    }
    Ok(())
}

fn file_bar(add: u64, del: u64, max_total: u64, max_len: usize, c: &Colour) -> String {
    let total = add + del;
    if total == 0 {
        return String::new();
    }
    let len =
        (((total as f64 / max_total as f64) * max_len as f64).ceil() as usize).clamp(1, max_len);
    let plus = ((add as f64 / total as f64) * len as f64).round() as usize;
    let plus = plus.min(len);
    let minus = len.saturating_sub(plus);
    format!("{}{}", c.green("+".repeat(plus)), c.red("-".repeat(minus)))
}

fn render_body(
    pr: &PullRequest,
    c: &Colour,
    width: usize,
    full_body: bool,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let cleaned = strip_html_comments(&pr.body);
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    render_section_header("Description", width, c, out)?;

    let mut wrapped = Vec::new();
    for raw_line in trimmed.lines() {
        let line = raw_line.trim_end();
        if line.is_empty() {
            wrapped.push(String::new());
        } else {
            wrapped.extend(wrap_text(line, width.saturating_sub(2)));
        }
    }

    let limit = if full_body {
        wrapped.len()
    } else {
        wrapped.len().min(MAX_PREVIEW_BODY_LINES)
    };
    for line in wrapped.iter().take(limit) {
        if line.starts_with('#') {
            writeln!(out, "  {}", c.bold(line))?;
        } else {
            writeln!(out, "  {line}")?;
        }
    }
    if wrapped.len() > limit {
        writeln!(
            out,
            "  {}",
            c.dim(format!(
                "… ({} more lines; pass --body to expand)",
                wrapped.len() - limit
            ))
        )?;
    }
    Ok(())
}

fn strip_html_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        if let Some(end) = rest[start + 4..].find("-->") {
            rest = &rest[start + 4 + end + 3..];
        } else {
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    out
}

fn wrap_plain_or_styled(s: &str, max_w: usize) -> Vec<String> {
    if visible_width(s) <= max_w {
        return vec![s.to_string()];
    }
    vec![s.to_string()]
}

fn wrap_text(s: &str, max_w: usize) -> Vec<String> {
    if max_w == 0 || s.chars().count() <= max_w {
        return vec![s.to_string()];
    }
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    for word in s.split_whitespace() {
        let w = word.chars().count();
        if cur_w == 0 {
            cur.push_str(word);
            cur_w = w;
        } else if cur_w + 1 + w <= max_w {
            cur.push(' ');
            cur.push_str(word);
            cur_w += 1 + w;
        } else {
            lines.push(cur);
            cur = word.to_string();
            cur_w = w;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn truncate_ellipsis(s: &str, max_w: usize) -> String {
    let count = s.chars().count();
    if count <= max_w || max_w < 2 {
        return s.to_string();
    }
    let head: String = s.chars().take(max_w - 1).collect();
    format!("{head}…")
}

fn truncate_left(s: &str, max_w: usize) -> String {
    let count = s.chars().count();
    if count <= max_w || max_w < 2 {
        return s.to_string();
    }
    let skip = count - (max_w - 1);
    let tail: String = s.chars().skip(skip).collect();
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Commit, Label, Review};

    fn sample_pr() -> PullRequest {
        PullRequest {
            number: 42,
            title: "feat(pr): visually compelling overview".to_string(),
            url: "https://github.com/alanrsoares/belt/pull/42".to_string(),
            state: PrState::Open,
            author: "alanrsoares".to_string(),
            base_ref: "main".to_string(),
            head_ref: "feat/pr".to_string(),
            created_at: "2026-11-18T00:00:00Z".to_string(),
            updated_at: "2026-11-18T11:00:00Z".to_string(),
            merged_at: None,
            additions: 120,
            deletions: 30,
            changed_files: 2,
            mergeable: Mergeable::Ready,
            merge_state: MergeState::Clean,
            review_decision: ReviewDecision::Approved,
            reviews: vec![Review {
                author: "alice".to_string(),
                state: ReviewState::Approved,
            }],
            review_requests: vec!["bob".to_string()],
            labels: vec![Label {
                name: "cli".to_string(),
                color: "000000".to_string(),
            }],
            commits: vec![Commit {
                oid: "1234567890".to_string(),
                headline: "feat(pr): initial".to_string(),
            }],
            checks: vec![
                Check {
                    name: "test".to_string(),
                    workflow: Some("CI".to_string()),
                    bucket: CheckBucket::Pass,
                    url: None,
                },
                Check {
                    name: "lint".to_string(),
                    workflow: Some("CI".to_string()),
                    bucket: CheckBucket::Fail,
                    url: None,
                },
            ],
            files: vec![FileChange {
                path: "crates/pr/src/lib.rs".to_string(),
                additions: 120,
                deletions: 30,
            }],
            body: "<!-- hidden -->\nSummary line".to_string(),
            comment_count: 1,
        }
    }

    #[test]
    fn renders_overview_without_ansi_when_colour_disabled() {
        let pr = sample_pr();
        let mut buf = Vec::new();
        let opts = RenderOpts {
            width: 80,
            compact: false,
            all_files: false,
            full_body: false,
            all_checks: false,
            now_secs: crate::model::parse_iso_utc("2026-11-18T12:00:00Z").unwrap(),
            glyphs: Glyphs::new(false),
        };
        render_pr(&pr, &Colour::new(false), opts, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(!text.contains('\x1b'));
        assert!(text.contains("#42"));
        assert!(text.contains("● OPEN"));
        assert!(text.contains("1h ago"));
        assert!(text.contains("✓ @alice"));
        assert!(text.contains("◌ @bob"));
        assert!(text.contains("✕ lint (CI)"));
        assert!(text.contains("crates/pr/src/lib.rs"));
        assert!(text.contains("Summary line"));
        assert!(!text.contains("hidden"));
    }

    #[test]
    fn renders_nerdfont_glyphs_when_enabled() {
        let pr = sample_pr();
        let mut buf = Vec::new();
        let opts = RenderOpts {
            width: 80,
            compact: true,
            all_files: false,
            full_body: false,
            all_checks: false,
            now_secs: crate::model::parse_iso_utc("2026-11-18T12:00:00Z").unwrap(),
            glyphs: Glyphs::new(true),
        };
        render_pr(&pr, &Colour::new(false), opts, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("\u{f407} OPEN"));
        assert!(text.contains("\u{f418} feat/pr"));
        assert!(text.contains("\u{f00c} @alice"));
    }

    #[test]
    fn compact_mode_omits_commits_files_and_body() {
        let pr = sample_pr();
        let mut buf = Vec::new();
        let opts = RenderOpts {
            width: 80,
            compact: true,
            all_files: false,
            full_body: false,
            all_checks: false,
            now_secs: crate::model::parse_iso_utc("2026-11-18T12:00:00Z").unwrap(),
            glyphs: Glyphs::new(false),
        };
        render_pr(&pr, &Colour::new(false), opts, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(!text.contains("Commits ("));
        assert!(!text.contains("Files ("));
        assert!(!text.contains("Description"));
    }
}
