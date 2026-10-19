//! Fast, zero-dependency browser automation library, persistent sessions, and CLI engine.

pub mod base64;
pub mod browser;
pub mod cdp;
pub mod cli;
pub mod dsl;
pub mod json;
pub mod locator;

use browser::BrowserInstance;
use cdp::{CdpSession, ConsoleEntry};
use cli::{Action, CliOptions};
use dsl::{ConsoleMode, Step};
use local_common::format_size;
use local_common::term::{color_enabled_for, file_uri, format_duration, Colour};
use std::fs;
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::Instant;

/// A PNG smaller than this almost always means the frame was captured before
/// first paint — worth flagging rather than reporting as a clean pass.
const NEAR_BLANK_PNG_BYTES: usize = 5_000;

/// Cap on `snapshot` output, so a huge page cannot flood the transcript.
const SNAPSHOT_NODE_LIMIT: usize = 200;

/// Longest console line kept in `--errors` mode.
const CONSOLE_LINE_LIMIT: usize = 200;

pub fn run(args: &[String], out: &mut impl Write) -> Result<i32, String> {
    let c = colour();

    match cli::parse_args(args)? {
        Action::Help => {
            cli::print_help(out, &c);
            Ok(0)
        }
        Action::Version => {
            cli::print_version(out);
            Ok(0)
        }
        Action::ListSessions => {
            let sessions = browser::list_sessions()?;
            if sessions.is_empty() {
                let _ = writeln!(out, "No persistent sessions found.");
            } else {
                let _ = writeln!(out, "{}", c.bold("Saved Persistent Sessions:"));
                for (name, size) in sessions {
                    let _ = writeln!(
                        out,
                        "  • {} {} {}",
                        c.cyan(&name),
                        c.gray("—"),
                        format_size(size)
                    );
                }
            }
            Ok(0)
        }
        Action::ClearSession(name) => {
            let path = browser::clear_session(&name)?;
            let _ = writeln!(
                out,
                "{} Cleared session '{}' ({})",
                c.green("✓"),
                c.bold(&name),
                c.link(path.display().to_string(), file_uri(&path))
            );
            Ok(0)
        }
        Action::Run(opts) => execute(&opts, out),
    }
}

/// Colour follows the usual rules: off when stdout is piped or `NO_COLOR` is
/// set. An agent reading this output pays tokens for every escape sequence.
fn colour() -> Colour {
    Colour::new(color_enabled_for(&std::io::stdout(), false))
}

/// What a step produced. `detail` is the payload an agent actually wants; the
/// status word exists so failures are greppable without parsing prose.
struct Outcome {
    status: Status,
    detail: Option<String>,
}

enum Status {
    Ok,
    Warn,
}

impl Outcome {
    fn ok() -> Self {
        Self {
            status: Status::Ok,
            detail: None,
        }
    }

    fn with(detail: impl Into<String>) -> Self {
        Self {
            status: Status::Ok,
            detail: Some(detail.into()),
        }
    }

    fn warn(detail: impl Into<String>) -> Self {
        Self {
            status: Status::Warn,
            detail: Some(detail.into()),
        }
    }
}

pub fn execute(opts: &CliOptions, out: &mut impl Write) -> Result<i32, String> {
    let c = colour();
    let overall_start = Instant::now();

    let target_dir = if let Some(ref dir) = opts.user_data_dir {
        Some(dir.clone())
    } else if let Some(ref name) = opts.session_name {
        Some(browser::resolve_session_path(name)?)
    } else {
        None
    };

    if opts.verbose {
        let profile = match (&opts.session_name, &opts.user_data_dir) {
            (Some(name), _) => format!("session:{name}"),
            (None, Some(_)) => "custom-profile".to_string(),
            _ => "ephemeral".to_string(),
        };
        let _ = writeln!(out, "{}", c.gray(format!("launching [{profile}]")));
    }

    let (mut instance, mut session) = BrowserInstance::launch(
        opts.custom_browser.as_deref(),
        opts.headless,
        target_dir,
        &opts.browser_args,
    )?;
    session.set_default_timeout(opts.default_timeout_ms);

    let mut failures = 0;
    let mut step_num = 0;

    match execute_steps(
        &mut session,
        &opts.steps,
        opts,
        &mut step_num,
        &mut failures,
        out,
        &c,
    ) {
        Ok(Flow::Continue) => {}
        Ok(Flow::Stop) => {
            instance.shutdown(&mut session);
            return Ok(finish(failures, opts, overall_start, out, &c));
        }
        Err(e) => {
            instance.shutdown(&mut session);
            return Err(e);
        }
    }

    // Streaming mode keeps the browser alive and executes one command per line,
    // so an interactive drive pays the ~1s launch cost once instead of per call.
    if opts.stream_stdin {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let line = line.map_err(|e| format!("failed to read stdin: {e}"))?;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
                continue;
            }

            let parsed = match dsl::parse_script(trimmed, opts.default_timeout_ms) {
                Ok(steps) => steps,
                Err(e) => {
                    failures += 1;
                    let _ = writeln!(out, "{} {e}", c.red("err"));
                    let _ = out.flush();
                    if !opts.keep_going {
                        break;
                    }
                    continue;
                }
            };

            let mut stop = false;
            for step in &parsed {
                step_num += 1;
                match run_step(&mut session, step, step_num, opts, out, &c) {
                    Ok(Flow::Continue) => {}
                    Ok(Flow::Stop) => {
                        stop = true;
                        break;
                    }
                    Err(e) => {
                        failures += 1;
                        let _ = writeln!(out, "{} {e}", c.red("err"));
                        if !opts.keep_going {
                            stop = true;
                            break;
                        }
                    }
                }
            }
            let _ = out.flush();
            if stop {
                break;
            }
        }
    }

    instance.shutdown(&mut session);
    Ok(finish(failures, opts, overall_start, out, &c))
}

fn execute_steps(
    session: &mut CdpSession,
    steps: &[Step],
    opts: &CliOptions,
    step_num: &mut usize,
    failures: &mut usize,
    out: &mut impl Write,
    c: &Colour,
) -> Result<Flow, String> {
    for step in steps {
        *step_num += 1;
        match run_step(session, step, *step_num, opts, out, c) {
            Ok(Flow::Continue) => {}
            Ok(Flow::Stop) => return Ok(Flow::Stop),
            Err(e) => {
                if opts.keep_going {
                    *failures += 1;
                    let _ = writeln!(out, "{} {e}", c.red("err"));
                } else {
                    return Err(e);
                }
            }
        }
    }
    Ok(Flow::Continue)
}

enum Flow {
    Continue,
    Stop,
}

fn finish(
    failures: usize,
    opts: &CliOptions,
    started: Instant,
    out: &mut impl Write,
    c: &Colour,
) -> i32 {
    if opts.verbose {
        let _ = writeln!(
            out,
            "{}",
            c.gray(format!("done in {}", format_duration(started.elapsed())))
        );
    }
    i32::from(failures > 0)
}

/// Run one step and print exactly one line for it (plus any data payload).
///
/// Errors propagate to the caller, which decides between fail-fast and
/// `--keep-going`; in batch mode a step failure aborts, matching the fact that
/// every later step would be acting on state that never happened.
fn run_step(
    session: &mut CdpSession,
    step: &Step,
    step_num: usize,
    opts: &CliOptions,
    out: &mut impl Write,
    c: &Colour,
) -> Result<Flow, String> {
    let started = Instant::now();

    let outcome: Outcome = match step {
        Step::Close => return Ok(Flow::Stop),

        Step::Goto(url) => {
            session.navigate(url, opts.default_timeout_ms)?;
            let landed = session.current_url()?;
            // Chrome parks failed navigations on an error page rather than
            // reporting a failure, which otherwise reads as a clean `ok`.
            if landed.starts_with("chrome-error://") {
                return Err(format!("navigation to '{url}' failed (no response)"));
            }
            Outcome::with(landed)
        }
        Step::Viewport { width, height } => {
            session.set_viewport(*width, *height)?;
            Outcome::with(format!("{width}x{height}"))
        }
        Step::WaitFor {
            selector,
            timeout_ms,
        } => {
            session.wait_for(selector, *timeout_ms)?;
            Outcome::ok()
        }
        Step::WaitForUrl {
            substring,
            timeout_ms,
        } => Outcome::with(session.wait_for_url(substring, *timeout_ms)?),
        Step::WaitForHydration {
            quiet_ms,
            timeout_ms,
        } => {
            session.wait_for_quiet(*quiet_ms, *timeout_ms)?;
            Outcome::ok()
        }
        Step::Wait { duration_ms } => {
            std::thread::sleep(std::time::Duration::from_millis(*duration_ms));
            Outcome::ok()
        }
        Step::Reload { timeout_ms } => Outcome::with(session.reload(*timeout_ms)?),
        Step::Click { selector } => {
            session.click(selector, opts.default_timeout_ms)?;
            Outcome::ok()
        }
        Step::Hover { selector } => {
            session.hover(selector, opts.default_timeout_ms)?;
            Outcome::ok()
        }
        Step::Press { chord } => {
            session.press(chord)?;
            Outcome::ok()
        }
        Step::Type {
            selector,
            text,
            clear,
        } => {
            session.type_text(selector, text, *clear, opts.default_timeout_ms)?;
            Outcome::ok()
        }
        Step::Eval { expr } => Outcome::with(session.evaluate(expr)?.to_json_string()),
        Step::Snapshot { all } => {
            let nodes = session.snapshot(*all, SNAPSHOT_NODE_LIMIT)?;
            for node in &nodes {
                let _ = writeln!(out, "{} {} \"{}\"", node.reference, node.role, node.name);
            }
            Outcome::with(format!(
                "{} node{}",
                nodes.len(),
                if nodes.len() == 1 { "" } else { "s" }
            ))
        }
        Step::Frame { target } => Outcome::with(session.target_frame(target.as_deref())?),
        Step::Url => Outcome::with(session.current_url()?),
        Step::Title => Outcome::with(session.title()?),
        Step::Console { mode } => {
            session.drain_events()?;
            let rendered = render_console(session.console_entries(), *mode);
            if let ConsoleMode::Clear = mode {
                session.clear_console();
            }
            for line in &rendered.lines {
                let _ = writeln!(out, "{line}");
            }
            Outcome::with(rendered.summary)
        }
        Step::Screenshot {
            path,
            full_page,
            selector,
        } => {
            let bytes = session.capture_screenshot(*full_page, selector.as_deref())?;
            fs::write(path, &bytes)
                .map_err(|e| format!("failed to save screenshot to '{path}': {e}"))?;
            let where_ = linked(c, path);
            let size = format_size(bytes.len() as u64);
            if bytes.len() < NEAR_BLANK_PNG_BYTES {
                Outcome::warn(format!("{where_} (near-blank, {size})"))
            } else {
                Outcome::with(format!("{where_} ({size})"))
            }
        }
        Step::Pdf { path } => {
            let bytes = session.print_to_pdf()?;
            fs::write(path, &bytes).map_err(|e| format!("failed to save PDF to '{path}': {e}"))?;
            Outcome::with(format!(
                "{} ({})",
                linked(c, path),
                format_size(bytes.len() as u64)
            ))
        }
        Step::Html { path } => match path {
            Some(p) => {
                let html = session.get_html()?;
                fs::write(p, html.as_bytes())
                    .map_err(|e| format!("failed to write HTML to '{p}': {e}"))?;
                Outcome::with(linked(c, p))
            }
            None => {
                let _ = writeln!(out, "{}", session.get_html()?);
                Outcome::ok()
            }
        },
    };

    // In quiet mode only payload-bearing steps speak; bare acknowledgements are
    // pure noise once a caller trusts the non-zero exit code.
    let silent = opts.quiet && matches!(outcome.status, Status::Ok) && outcome.detail.is_none();
    if !silent {
        let word = match outcome.status {
            Status::Ok => c.green("ok"),
            Status::Warn => c.yellow("warn"),
        };
        let prefix = if opts.verbose {
            format!("[{step_num}] {} ", verb_of(step))
        } else {
            String::new()
        };
        let suffix = if opts.verbose {
            c.gray(format!(" ({})", format_duration(started.elapsed())))
        } else {
            String::new()
        };
        match outcome.detail {
            Some(d) => {
                let _ = writeln!(out, "{prefix}{word} {d}{suffix}");
            }
            None => {
                let _ = writeln!(out, "{prefix}{word}{suffix}");
            }
        }
    }

    Ok(Flow::Continue)
}

fn verb_of(step: &Step) -> &'static str {
    match step {
        Step::Goto(_) => "goto",
        Step::Viewport { .. } => "viewport",
        Step::WaitFor { .. } => "wait-for",
        Step::WaitForUrl { .. } => "wait-for-url",
        Step::WaitForHydration { .. } => "wait-for-hydration",
        Step::Wait { .. } => "wait",
        Step::Reload { .. } => "reload",
        Step::Click { .. } => "click",
        Step::Hover { .. } => "hover",
        Step::Press { .. } => "press",
        Step::Type { clear: true, .. } => "fill",
        Step::Type { .. } => "type",
        Step::Eval { .. } => "eval",
        Step::Snapshot { .. } => "snapshot",
        Step::Frame { .. } => "frame",
        Step::Url => "url",
        Step::Title => "title",
        Step::Console { .. } => "console",
        Step::Screenshot { .. } => "screenshot",
        Step::Pdf { .. } => "pdf",
        Step::Html { .. } => "html",
        Step::Close => "close",
    }
}

/// Absolute path, rendered as an OSC 8 hyperlink for humans and as plain text
/// whenever colour is off — which is exactly when something is parsing it.
fn linked(c: &Colour, path: &str) -> String {
    let absolute = fs::canonicalize(path)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| {
            std::env::current_dir()
                .map(|d| d.join(Path::new(path)).display().to_string())
                .unwrap_or_else(|_| path.to_string())
        });

    c.link(&absolute, file_uri(Path::new(&absolute)))
}

struct RenderedConsole {
    lines: Vec<String>,
    summary: String,
}

/// Collapse repeats and truncate stacks so a noisy page costs a few lines
/// rather than a few hundred.
fn render_console(entries: &[ConsoleEntry], mode: ConsoleMode) -> RenderedConsole {
    if let ConsoleMode::Clear = mode {
        return RenderedConsole {
            lines: Vec::new(),
            summary: "console cleared".to_string(),
        };
    }

    let full = matches!(mode, ConsoleMode::Full);
    let selected: Vec<&ConsoleEntry> = entries.iter().filter(|e| full || e.is_error()).collect();

    if selected.is_empty() {
        return RenderedConsole {
            lines: Vec::new(),
            summary: if full {
                "no console output".to_string()
            } else {
                "no errors".to_string()
            },
        };
    }

    let mut lines: Vec<String> = Vec::new();
    let mut counts: Vec<usize> = Vec::new();

    for entry in &selected {
        let text = if full {
            entry.text.clone()
        } else {
            truncate(entry.text.replace('\n', " ").trim(), CONSOLE_LINE_LIMIT)
        };
        let line = format!("{}: {}", entry.level, text);
        match lines.iter().position(|l| *l == line) {
            Some(i) if !full => counts[i] += 1,
            _ => {
                lines.push(line);
                counts.push(1);
            }
        }
    }

    let rendered = lines
        .into_iter()
        .zip(counts)
        .map(|(line, n)| {
            if n > 1 {
                format!("{line} (×{n})")
            } else {
                line
            }
        })
        .collect();

    RenderedConsole {
        lines: rendered,
        summary: format!(
            "{} {}",
            selected.len(),
            if full { "entries" } else { "errors" }
        ),
    }
}

fn truncate(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_string();
    }
    let head: String = s.chars().take(limit).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::WebSocketClient;
    use std::io::Read;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn help_and_version_write_to_out_buffer() {
        let mut out = Vec::new();
        let code = run(&["--version".to_string()], &mut out).unwrap();
        assert_eq!(code, 0);
        assert!(String::from_utf8(out).unwrap().starts_with("webdriver "));

        let mut help_out = Vec::new();
        let code = run(&["--help".to_string()], &mut help_out).unwrap();
        assert_eq!(code, 0);
        assert!(String::from_utf8(help_out).unwrap().contains("USAGE:"));

        let mut plain_help = Vec::new();
        cli::print_help(&mut plain_help, &Colour::new(false));
        let plain_str = String::from_utf8(plain_help).unwrap();
        assert!(plain_str.contains("USAGE:"));
        assert!(!plain_str.contains('\x1b'));
    }

    #[test]
    fn keep_going_continues_past_failing_batch_step() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut req = [0u8; 512];
            let _ = sock.read(&mut req);
            sock.write_all(
                b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n",
            )
            .unwrap();

            // Read first CDP request (id=1, failing eval)
            let mut hdr = [0u8; 2];
            sock.read_exact(&mut hdr).unwrap();
            let len = (hdr[1] & 0x7f) as usize;
            let mut body = vec![0u8; 4 + len];
            sock.read_exact(&mut body).unwrap();

            let err_reply = br#"{"id":1,"error":{"message":"synthetic failure"}}"#;
            let mut frame = vec![0x81, err_reply.len() as u8];
            frame.extend_from_slice(err_reply);
            sock.write_all(&frame).unwrap();

            // Read second CDP request (id=2, succeeding eval)
            sock.read_exact(&mut hdr).unwrap();
            let len2 = (hdr[1] & 0x7f) as usize;
            let mut body2 = vec![0u8; 4 + len2];
            sock.read_exact(&mut body2).unwrap();

            let ok_reply = br#"{"id":2,"result":{"result":{"value":42}}}"#;
            let mut frame2 = vec![0x81, ok_reply.len() as u8];
            frame2.extend_from_slice(ok_reply);
            sock.write_all(&frame2).unwrap();
        });

        let ws = WebSocketClient::connect("127.0.0.1", port, "/devtools/page/1").unwrap();
        let mut session = CdpSession::new(ws);
        let steps = vec![
            Step::Eval {
                expr: "bad()".to_string(),
            },
            Step::Eval {
                expr: "40 + 2".to_string(),
            },
        ];
        let opts = CliOptions {
            custom_browser: None,
            browser_args: Vec::new(),
            default_timeout_ms: 5_000,
            headless: true,
            quiet: false,
            verbose: false,
            session_name: None,
            user_data_dir: None,
            steps: steps.clone(),
            keep_going: true,
            stream_stdin: false,
        };

        let mut step_num = 0;
        let mut failures = 0;
        let mut out = Vec::new();
        let c = Colour::new(false);

        let flow = execute_steps(
            &mut session,
            &steps,
            &opts,
            &mut step_num,
            &mut failures,
            &mut out,
            &c,
        )
        .unwrap();
        assert!(matches!(flow, Flow::Continue));
        assert_eq!(failures, 1);
        assert_eq!(step_num, 2);

        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("err CDP Runtime.evaluate error: synthetic failure"));
        assert!(output.contains("ok 42"));

        server.join().unwrap();
    }
}
