//! Command-line argument parsing, persistent sessions, and help definitions.

use crate::dsl::{self, Step};
use local_common::term::Colour;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct CliOptions {
    pub custom_browser: Option<String>,
    /// Extra flags handed to the browser binary verbatim.
    pub browser_args: Vec<String>,
    pub default_timeout_ms: u64,
    pub headless: bool,
    pub quiet: bool,
    pub verbose: bool,
    pub session_name: Option<String>,
    pub user_data_dir: Option<PathBuf>,
    pub steps: Vec<Step>,
    /// Keep running after a failed step instead of aborting the batch.
    pub keep_going: bool,
    /// Read and execute one command per line from stdin, streaming results back
    /// as each finishes, so the browser stays open across an interactive drive.
    pub stream_stdin: bool,
}

pub enum Action {
    Run(CliOptions),
    ListSessions,
    ClearSession(String),
    Help,
    Version,
}

use local_common::{split_flag, ArgCursor};

pub fn parse_args(args: &[String]) -> Result<Action, String> {
    parse(args.to_vec())
}

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Action, String> {
    let mut cursor = ArgCursor::new(args.into_iter());

    let mut custom_browser = None;
    let mut browser_args: Vec<String> = Vec::new();
    let mut default_timeout_ms = 30_000;
    let mut headless = true;
    let mut quiet = false;
    let mut verbose = false;
    let mut session_name = None;
    let mut user_data_dir = None;
    let mut script_file = None;
    let mut read_stdin = false;
    let mut keep_going = false;

    let mut step_tokens = Vec::new();
    let mut has_args = false;

    while let Some(arg) = cursor.next() {
        has_args = true;
        let (flag, inline) = split_flag(&arg);

        match flag {
            "-h" | "--help" | "help" => return Ok(Action::Help),
            "-V" | "--version" | "version" => return Ok(Action::Version),
            "--list-sessions" | "sessions" => return Ok(Action::ListSessions),
            "--clear-session" => {
                let name = cursor
                    .require_value("--clear-session", inline)
                    .map_err(|_| "--clear-session requires a session name".to_string())?;
                return Ok(Action::ClearSession(name));
            }
            "-b" | "--browser" => {
                let path = cursor
                    .require_value("--browser", inline)
                    .map_err(|_| "--browser requires a path".to_string())?;
                custom_browser = Some(path);
            }
            "--browser-arg" => {
                let flag_val = cursor
                    .require_value("--browser-arg", inline)
                    .map_err(|_| "--browser-arg requires a flag (e.g. --no-sandbox)".to_string())?;
                browser_args.push(flag_val);
            }
            "-s" | "--session" => {
                let name = cursor
                    .require_value("--session", inline)
                    .map_err(|_| "--session requires a session name (e.g. 'my-app')".to_string())?;
                session_name = Some(name);
            }
            "--user-data-dir" | "--profile" => {
                let path_str = cursor
                    .require_value("--user-data-dir", inline)
                    .map_err(|_| "--user-data-dir requires a directory path".to_string())?;
                user_data_dir = Some(PathBuf::from(path_str));
            }
            "-t" | "--timeout" => {
                let dur_str = cursor
                    .require_value("--timeout", inline)
                    .map_err(|_| "--timeout requires a duration (e.g. 60s, 500ms)".to_string())?;
                default_timeout_ms = dsl::parse_duration_ms(&dur_str)?;
            }
            "--headed" | "--no-headless" => {
                headless = false;
            }
            "--headless" => {
                headless = true;
            }
            "-q" | "--quiet" => {
                quiet = true;
            }
            "--keep-going" | "--no-fail-fast" => {
                keep_going = true;
            }
            "-v" | "--verbose" => {
                verbose = true;
            }
            "--file" | "-f" => {
                let path = cursor
                    .require_value("--file", inline)
                    .map_err(|_| "--file requires a path".to_string())?;
                script_file = Some(path);
            }
            "-" | "--repl" | "--stdin" => {
                read_stdin = true;
            }
            _ => {
                step_tokens.push(arg);
            }
        }
    }

    if !has_args {
        return Ok(Action::Help);
    }

    let mut steps = Vec::new();

    if let Some(file_path) = script_file {
        let content = fs::read_to_string(&file_path)
            .map_err(|e| format!("failed to read script file '{file_path}': {e}"))?;
        let parsed = dsl::parse_script(&content, default_timeout_ms)?;
        steps.extend(parsed);
    }

    if !step_tokens.is_empty() {
        let parsed = dsl::parse_tokens(&step_tokens, default_timeout_ms)?;
        steps.extend(parsed);
    }

    if steps.is_empty() && !read_stdin {
        return Ok(Action::Help);
    }

    Ok(Action::Run(CliOptions {
        custom_browser,
        browser_args,
        default_timeout_ms,
        headless,
        quiet,
        verbose,
        session_name,
        user_data_dir,
        steps,
        keep_going,
        stream_stdin: read_stdin,
    }))
}

pub fn print_help(out: &mut impl Write, c: &Colour) {
    let _ = writeln!(
        out,
        "{} — fast, zero-dependency browser automation & persistent session CLI

{}
    webdriver [OPTIONS] <URL> [VERBS...]
    webdriver --session <NAME> <URL> [VERBS...]
    webdriver [OPTIONS] -f script.wd
    webdriver [OPTIONS] --repl        # one command per stdin line, browser stays open

{}
    One line per step: 'ok', 'ok <payload>', 'warn <detail>' or 'err <message>'.
    Colour is off whenever stdout is not a TTY. Exit code is 1 if any step failed.

{}
    # Hop 1: Authenticate and store cookies/localStorage in 'my-app' session profile
    webdriver --session my-app http://localhost:3000/login fill '#user' 'admin' fill '#pass' 'sec' click '#submit' wait-for '.dashboard'

    # Hop 2: Subsequent request reuses authenticated session profile seamlessly
    webdriver --session my-app http://localhost:3000/admin/settings screenshot settings.png

    # Manage saved sessions
    webdriver --list-sessions
    webdriver --clear-session my-app

{} (anywhere a selector is accepted):
    {}Some label          Innermost element containing the text
    {}button:Save         ARIA role + accessible name
    {}<css> or bare CSS    CSS selector
    {}e7                    Handle from the last snapshot (names come from Chrome)

    Actions retry until the element is present and visible, up to --timeout, so a
    wait-for before every click is unnecessary. Clicks and typing go through real
    browser input events.

{}
    {} <url>                     Navigate to URL (default for initial URL)
    {}                         Reload current page
    {} <w> <h>               Set window size (e.g. 1280 800 or 1280x800)
    {} <loc> [timeout]       Wait until element is visible AND painted
    {} <substr> [t]      Wait until URL contains substring (redirects)
    {} [quiet]     Wait until DOM stops mutating (default 500ms)
    {} <duration>                Fixed sleep (e.g. 500ms, 2s, 1m)
    {} <locator>                Click element
    {} <locator>                Real mouse move, so CSS :hover applies
    {} <key>                    Key chord, e.g. Enter, Escape, Meta+O
    {} <loc> <text>              Append text (fires React onChange)
    {} <loc> <text>              Clear then type
    {} <js_expr>                 Evaluate JS, prints JSON result
    {} / {}                   Print current URL / page title
    {} [--errors|--full|--clear]  Buffered console errors, repeats collapsed
    {} [--all]                 Accessibility tree as 'eN role \"name\"' lines
    {} <substr> | --top           Aim later commands at an iframe (URL or name)
    {} [path] [flags]      Capture PNG (flags: --full-page, --selector <loc>)
    {} [path]                     Print page to PDF
    {} [path]                    Dump current HTML to file or stdout
    {}                          End the run (useful in --repl)

{}
    -s, --session <name>        Persistent session profile name (preserves cookies & storage)
        --user-data-dir <dir>   Custom browser profile data directory
        --list-sessions         List stored persistent session profiles
        --clear-session <name>  Delete stored persistent session profile
    -b, --browser <path>        Custom browser binary (Chromium, Chrome, Brave)
        --browser-arg <flag>    Extra browser flag, repeatable (e.g. --browser-arg --no-sandbox).
                                Running as root already implies --no-sandbox and
                                --disable-dev-shm-usage, for CI containers
    -t, --timeout <duration>    Default step timeout (default: 30s)
    -f, --file <path>           Read a script file instead of CLI verbs
        --repl, -               Stream commands from stdin, one per line
        --keep-going            Continue after a failed step (default: stop)
        --headed                Show browser UI window (disable headless mode)
    -q, --quiet                 Print only steps that carry a payload
    -v, --verbose               Step numbers and timings
    -h, --help                  Print help and exit
    -V, --version               Print version and exit",
        c.cyan("webdriver"),
        c.bold("USAGE:"),
        c.bold("OUTPUT:"),
        c.bold("PERSISTENT AUTH & SESSIONS:"),
        c.bold("LOCATORS"),
        c.yellow("text="),
        c.yellow("role="),
        c.yellow("sel="),
        c.yellow("ref="),
        c.bold("VERBS & ACTIONS:"),
        c.yellow("goto"),
        c.yellow("reload"),
        c.yellow("viewport"),
        c.yellow("wait-for"),
        c.yellow("wait-for-url"),
        c.yellow("wait-for-hydration"),
        c.yellow("wait"),
        c.yellow("click"),
        c.yellow("hover"),
        c.yellow("press"),
        c.yellow("type"),
        c.yellow("fill"),
        c.yellow("eval"),
        c.yellow("url"),
        c.yellow("title"),
        c.yellow("console"),
        c.yellow("snapshot"),
        c.yellow("frame"),
        c.yellow("screenshot"),
        c.yellow("pdf"),
        c.yellow("html"),
        c.yellow("close"),
        c.bold("OPTIONS:"),
    );
}

pub fn print_version(out: &mut impl Write) {
    let _ = writeln!(out, "webdriver {}", env!("CARGO_PKG_VERSION"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cli_full_flow() {
        let args = vec![
            "-s".to_string(),
            "auth-test".to_string(),
            "-t".to_string(),
            "45s".to_string(),
            "-q".to_string(),
            "https://example.com".to_string(),
            "wait-for".to_string(),
            "h1".to_string(),
            "screenshot".to_string(),
            "out.png".to_string(),
        ];

        let action = parse_args(&args).expect("parse failed");
        match action {
            Action::Run(opts) => {
                assert_eq!(opts.session_name, Some("auth-test".to_string()));
                assert_eq!(opts.default_timeout_ms, 45_000);
                assert!(opts.quiet);
                assert_eq!(opts.steps.len(), 3);
            }
            _ => panic!("expected Run action"),
        }
    }

    #[test]
    fn parse_repeated_browser_args() {
        let args = vec![
            "--browser-arg".to_string(),
            "--no-sandbox".to_string(),
            "--browser-arg".to_string(),
            "--lang=en-GB".to_string(),
            "https://example.com".to_string(),
        ];

        match parse_args(&args).expect("parse failed") {
            Action::Run(opts) => {
                assert_eq!(opts.browser_args, vec!["--no-sandbox", "--lang=en-GB"]);
            }
            _ => panic!("expected Run action"),
        }

        assert!(parse_args(&["--browser-arg".to_string()]).is_err());
    }

    #[test]
    fn parse_help_and_version() {
        assert!(matches!(
            parse_args(&["--help".to_string()]),
            Ok(Action::Help)
        ));
        assert!(matches!(
            parse_args(&["-V".to_string()]),
            Ok(Action::Version)
        ));
        assert!(matches!(parse_args(&[]), Ok(Action::Help)));
    }

    #[test]
    fn parse_session_actions() {
        assert!(matches!(
            parse_args(&["--list-sessions".to_string()]),
            Ok(Action::ListSessions)
        ));
        assert!(matches!(
            parse_args(&["--clear-session".to_string(), "app".to_string()]),
            Ok(Action::ClearSession(ref s)) if s == "app"
        ));
    }

    #[test]
    fn parse_headed_and_custom_browser() {
        let args = vec![
            "--headed".to_string(),
            "-b".to_string(),
            "/path/to/chrome".to_string(),
            "https://test.com".to_string(),
        ];
        let action = parse_args(&args).expect("parse failed");
        match action {
            Action::Run(opts) => {
                assert!(!opts.headless);
                assert_eq!(opts.custom_browser, Some("/path/to/chrome".to_string()));
                assert_eq!(opts.steps.len(), 1);
            }
            _ => panic!("expected Run action"),
        }
    }
}
