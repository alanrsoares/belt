//! CLI argument parsing for `pr`.

use local_common::{split_flag, ArgCursor, CommonFlags};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    View,
    Web,
    Help,
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub action: Action,
    pub target: Option<String>,
    pub repo: Option<String>,
    pub compact: bool,
    pub hud: bool,
    pub all_files: bool,
    pub full_body: bool,
    pub all_checks: bool,
    pub raw_json: bool,
    pub flags: CommonFlags,
}

impl Default for Parsed {
    fn default() -> Self {
        Self {
            action: Action::View,
            target: None,
            repo: None,
            compact: false,
            hud: false,
            all_files: false,
            full_body: false,
            all_checks: false,
            raw_json: false,
            flags: CommonFlags::default(),
        }
    }
}

pub const VERSION: &str = "belt-pr 0.1.0";

pub const HELP: &str = r#"belt-pr [TARGET] [OPTIONS] — terminal-friendly GitHub pull request overview.

Infers the pull request from the current git branch when TARGET is omitted.
Also available via `belt pr`.

USAGE
    belt-pr                              # PR for current git branch
    belt-pr 142                          # PR #142 in current repo
    belt-pr #142                         # same, leading '#' stripped
    belt-pr feat/auth                    # PR for branch feat/auth
    belt-pr https://github.com/o/r/pull/1
    belt-pr 142 -R owner/repo            # PR in another repository
    belt-pr -c                           # compact one-screen summary
    belt-pr --checks --files             # expand all checks and changed files

OPTIONS
    -R, --repo <OWNER/REPO>   select repository ([HOST/]OWNER/REPO)
    -c, --compact             compact view (omit commits, files, body)
        --hud                 merge-readiness HUD card (checks, comments, approvals)
    -f, --files               show all changed files (default: top 10)
    -b, --body                show full PR description (default: 10-line preview)
        --checks              show all status checks
    -w, --web                 open the pull request in the browser
    -j, --json                print raw JSON from `gh pr view`
        --color               force colour output
        --no-color            disable colour output
    -h, --help                print this help
    -V, --version             print version
"#;

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Parsed, String> {
    let mut cursor = ArgCursor::new(args.into_iter());
    let mut p = Parsed::default();

    while let Some(a) = cursor.next() {
        let (flag, inline) = split_flag(&a);
        match flag {
            "-h" | "--help" => p.action = Action::Help,
            "-V" | "--version" => p.action = Action::Version,
            "-w" | "--web" => p.action = Action::Web,
            "-c" | "--compact" => p.compact = true,
            "--hud" => p.hud = true,
            "-f" | "--files" => p.all_files = true,
            "-b" | "--body" => p.full_body = true,
            "--checks" => p.all_checks = true,
            "-j" | "--json" => p.raw_json = true,
            f if p.flags.check_arg(f) => {}
            "-R" | "--repo" => {
                p.repo = Some(cursor.require_value("--repo", inline)?);
            }
            s if s.starts_with('-') => return Err(format!("unknown flag: {s}")),
            s => {
                if p.target.is_some() {
                    return Err(format!("unexpected argument: {s}"));
                }
                let normalized = match s.strip_prefix('#') {
                    Some("") => return Err("invalid PR target: `#`".to_string()),
                    Some(rest) if rest.chars().all(|c| c.is_ascii_digit()) => rest,
                    _ => s,
                };
                p.target = Some(normalized.to_string());
            }
        }
    }

    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn defaults_to_cwd_view() {
        let p = parse(args(&[])).unwrap();
        assert_eq!(p.action, Action::View);
        assert_eq!(p.target, None);
        assert_eq!(p.repo, None);
    }

    #[test]
    fn strips_hash_from_pr_number() {
        let p = parse(args(&["#142"])).unwrap();
        assert_eq!(p.target.as_deref(), Some("142"));
    }

    #[test]
    fn preserves_hash_in_branch_name() {
        let p = parse(args(&["#topic"])).unwrap();
        assert_eq!(p.target.as_deref(), Some("#topic"));
    }

    #[test]
    fn parses_repo_and_flags() {
        let p = parse(args(&[
            "99",
            "-R",
            "acme/app",
            "--compact",
            "--checks",
            "--no-color",
        ]))
        .unwrap();
        assert_eq!(p.target.as_deref(), Some("99"));
        assert_eq!(p.repo.as_deref(), Some("acme/app"));
        assert!(p.compact);
        assert!(p.all_checks);
        assert!(p.flags.no_color);
    }

    #[test]
    fn rejects_unknown_flag_and_extra_positional() {
        assert!(parse(args(&["--bogus"])).is_err());
        assert!(parse(args(&["1", "2"])).is_err());
        assert!(parse(args(&["#"])).is_err());
    }
}
