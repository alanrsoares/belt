//! CLI argument parsing for `fanout`.

use std::thread;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const HELP: &str = r#"fanout — concurrent quality gate & task matrix runner (inspired by Turborepo)

USAGE:
    fanout [TARGETS...] [FLAGS] [OPTIONS] [-- <ARGS>...]

TARGETS:
    check              Run workspace quality gates (default)
    check:full         Run the full workspace quality gate set
    <script>...        Run one or more scripts across workspace (e.g. lint typecheck test)

FLAGS:
    --bail             Abort remaining tasks on first failure
    --compact          CI report, but silent until a task fails
                       (the report is the default off a TTY or with CI=true)
    --dry-run          Print the resolved task list and exit without running it
    --topological      Enforce topological package dependency order (^build)
    --color            Force ANSI color output
    --no-color         Disable ANSI color output
    -h, --help         Print this help message
    -V, --version      Print version information

OPTIONS:
    -s, --since <ref>  Run tasks only for packages changed since <ref> (e.g. main, HEAD~1)
    --filter <pattern> Scope execution to packages matching glob or Turborepo filter ([ref], ...pkg)
    --timeout <ms>     Timeout per task in milliseconds (default: 900000 = 15m)
    --tail <n>         Lines of output to retain on failure (default: 40, 0 = unlimited)
    -j, --jobs <n>     Concurrency ceiling (default: hardware threads, min 2)
    -- <args>...       Forward trailing arguments directly to each task command
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Help,
    Version,
    Run,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub action: Action,
    pub targets: Vec<String>,
    pub filter: Option<String>,
    pub since: Option<String>,
    pub topological: bool,
    pub passthrough_args: Vec<String>,
    pub bail: bool,
    pub timeout_ms: u64,
    pub compact: bool,
    pub dry_run: bool,
    pub tail_lines: usize,
    pub jobs: usize,
    pub color: bool,
    pub no_color: bool,
}

impl Default for Options {
    fn default() -> Self {
        let default_jobs = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .max(2);

        Self {
            action: Action::Run,
            targets: vec!["check".to_string()],
            filter: None,
            since: None,
            topological: false,
            passthrough_args: Vec::new(),
            bail: false,
            timeout_ms: 15 * 60 * 1000,
            compact: false,
            dry_run: false,
            tail_lines: 40,
            jobs: default_jobs,
            color: false,
            no_color: false,
        }
    }
}

use local_common::{split_flag, ArgCursor};

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Options, String> {
    let mut opts = Options::default();
    let mut cursor = ArgCursor::new(args.into_iter());
    let mut positional_set = false;

    while let Some(arg) = cursor.next() {
        if arg == "--" {
            // Trailing passthrough args: collect all remaining tokens
            opts.passthrough_args.extend(cursor);
            break;
        }

        let (flag, inline) = split_flag(&arg);

        match flag {
            "-h" | "--help" => {
                opts.action = Action::Help;
                return Ok(opts);
            }
            "-V" | "--version" => {
                opts.action = Action::Version;
                return Ok(opts);
            }
            "--bail" => opts.bail = true,
            "--compact" => opts.compact = true,
            "--dry-run" | "--list" => opts.dry_run = true,
            "--topological" | "--deps" => opts.topological = true,
            "--color" => opts.color = true,
            "--no-color" => opts.no_color = true,
            "-s" | "--since" => {
                let val = cursor.require_value("--since", inline)?;
                opts.since = Some(val);
            }
            "--filter" => {
                let val = cursor.require_value("--filter", inline)?;
                apply_filter_option(&val, &mut opts);
            }
            "--timeout" => {
                let val = cursor.require_value("--timeout", inline)?;
                let ms: u64 = val.parse().map_err(|_| {
                    format!("invalid --timeout value '{val}', expected integer milliseconds")
                })?;
                if ms > 0 {
                    opts.timeout_ms = ms;
                }
            }
            "--tail" => {
                let val = cursor.require_value("--tail", inline)?;
                let lines: usize = val
                    .parse()
                    .map_err(|_| format!("invalid --tail value '{val}', expected integer"))?;
                opts.tail_lines = lines;
            }
            "-j" | "--jobs" => {
                let val = cursor.require_value("--jobs", inline)?;
                let jobs: usize = val
                    .parse()
                    .map_err(|_| format!("invalid --jobs value '{val}', expected integer >= 1"))?;
                if jobs >= 1 {
                    opts.jobs = jobs;
                }
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option '{other}'"));
            }
            pos => {
                if !positional_set {
                    opts.targets.clear();
                    positional_set = true;
                }
                opts.targets.push(pos.to_string());
            }
        }
    }

    Ok(opts)
}

fn apply_filter_option(filter_val: &str, opts: &mut Options) {
    let trimmed = filter_val.trim();
    if let Some(inner) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        // Turborepo style git ref filter: --filter=[main]
        opts.since = Some(inner.to_string());
    } else {
        opts.filter = Some(trimmed.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_defaults() {
        let opts = parse(vec![]).unwrap();
        assert_eq!(opts.targets, vec!["check"]);
        assert!(!opts.bail);
        assert!(!opts.compact);
        assert_eq!(opts.tail_lines, 40);
        assert_eq!(opts.timeout_ms, 900_000);
        assert!(opts.jobs >= 2);
    }

    #[test]
    fn parses_custom_targets_and_flags() {
        let args = vec![
            "lint".into(),
            "typecheck".into(),
            "test".into(),
            "--bail".into(),
            "--compact".into(),
            "--topological".into(),
            "--tail".into(),
            "100".into(),
            "--jobs".into(),
            "8".into(),
            "--filter".into(),
            "@renkonos/*".into(),
            "-s".into(),
            "main".into(),
            "--timeout".into(),
            "60000".into(),
            "--".into(),
            "--coverage".into(),
            "-u".into(),
        ];
        let opts = parse(args).unwrap();
        assert_eq!(opts.targets, vec!["lint", "typecheck", "test"]);
        assert!(opts.bail);
        assert!(opts.compact);
        assert!(opts.topological);
        assert_eq!(opts.tail_lines, 100);
        assert_eq!(opts.jobs, 8);
        assert_eq!(opts.filter.as_deref(), Some("@renkonos/*"));
        assert_eq!(opts.since.as_deref(), Some("main"));
        assert_eq!(opts.timeout_ms, 60_000);
        assert_eq!(opts.passthrough_args, vec!["--coverage", "-u"]);
    }

    #[test]
    fn parses_dry_run() {
        assert!(parse(vec!["--dry-run".to_string()]).unwrap().dry_run);
        assert!(parse(vec!["--list".to_string()]).unwrap().dry_run);
        assert!(!parse(vec![]).unwrap().dry_run);
    }

    #[test]
    fn parses_turborepo_git_filter() {
        let args = vec!["--filter=[HEAD~1]".into()];
        let opts = parse(args).unwrap();
        assert_eq!(opts.since.as_deref(), Some("HEAD~1"));
    }

    #[test]
    fn parses_equals_syntax() {
        let args = vec![
            "--filter=*ui*".into(),
            "--since=origin/main".into(),
            "--jobs=4".into(),
            "--tail=20".into(),
            "--timeout=30000".into(),
        ];
        let opts = parse(args).unwrap();
        assert_eq!(opts.filter.as_deref(), Some("*ui*"));
        assert_eq!(opts.since.as_deref(), Some("origin/main"));
        assert_eq!(opts.jobs, 4);
        assert_eq!(opts.tail_lines, 20);
        assert_eq!(opts.timeout_ms, 30_000);
    }
}
