//! `grabsite` — screenshot a website through its GUI browser.
//!
//! Hardened, Tor-lineage builds (Mullvad Browser, Tor Browser, and Firefox
//! variants that ship them) intentionally expose **no** CDP endpoint and
//! **no** working `--headless --screenshot` surface: the headless mode is
//! disabled by design to preserve the browser's fingerprinting guarantees.
//!
//! The one reliable capture path for those builds is the same one a human
//! uses — open the window, let it paint, grab the screen. `grabsite`
//! automates exactly that: it spins up a private `Xvfb` (or reuses a display
//! with `--display`), launches the browser in GUI mode with a persistent
//! profile, waits until the pixels settle, and writes the root window to a
//! PNG via `import`.
//!
//! The `webdriver` tool in this workspace covers the CDP-enabled case
//! (Chrome, stock Firefox, …); the two are complements, not rivals.
//!
//! # Lifecycle
//! 1. Start an X display (`Xvfb :N` on a free number, or reuse `:N`).
//! 2. Prepare the persistent profile (`~/.local/share/belt/grabsite/profile`).
//! 3. Launch the browser there (gecko: `-profile` + `--private-window`;
//!    chromium: `--user-data-dir` + `--incognito`).
//! 4. Wait out the load `--settle`, then capture pairs of frames until two
//!    agree, or the `--timeout` budget is exhausted.
//! 5. Stop the browser, kill the display (if owned), write the result line.

pub mod capture;
pub mod cli;
pub mod display;
pub mod term;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use local_common::paths::tool_data_dir;
use local_common::process::own_process_group;

use crate::cli::Action;
use crate::term::{Color, Out};

/// A PNG this small almost always means the window captured before first
/// paint. Flag it rather than hand back a blank.
const NEAR_BLANK_PNG: u64 = 4_096;

/// Resolved engine family for browser launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Engine {
    Gecko,
    Chromium,
}

impl Engine {
    fn as_str(self) -> &'static str {
        match self {
            Engine::Gecko => "gecko",
            Engine::Chromium => "chromium",
        }
    }
}

/// Find the browser binary from a user-supplied path or name, falling back
/// to a list of known hardened/gecko builds if unspecified.
fn resolve_browser(spec: Option<&str>) -> Result<(String, PathBuf), String> {
    if let Some(explicit) = spec {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Ok((explicit.to_string(), path.canonicalize().unwrap_or(path)));
        }
        if let Some(bin) = which::find(explicit) {
            let p = PathBuf::from(&bin);
            return Ok((bin, p.canonicalize().unwrap_or(p)));
        }
        return Err(format!(
            "browser '{explicit}' not found. Check the path or ensure it is in $PATH."
        ));
    }

    // Default lookup when no browser was explicitly requested.
    if let Some(bin) = which::find("mullvad-browser") {
        let p = PathBuf::from(&bin);
        return Ok((bin, p.canonicalize().unwrap_or(p)));
    }

    let candidates = [
        "/usr/bin/mullvad-browser",
        "/usr/local/bin/mullvad-browser",
        "/snap/bin/mullvad-browser",
        "/Applications/Mullvad Browser.app/Contents/MacOS/mullvadbrowser",
        "/usr/bin/firefox",
        "/usr/local/bin/firefox",
        "/home/linuxbrew/.linuxbrew/bin/firefox",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.is_file() {
            return Ok((
                p.to_string_lossy().into_owned(),
                p.canonicalize().unwrap_or(p),
            ));
        }
    }

    if let Some(bin) = which::find("firefox") {
        let p = PathBuf::from(&bin);
        return Ok((bin, p.canonicalize().unwrap_or(p)));
    }

    Err(
        "browser 'mullvad-browser' not found. Check the path or install it. (tried mullvad-browser, firefox, and $PATH)"
            .to_string(),
    )
}

pub mod which {
    /// Search $PATH.
    pub fn find(name: &str) -> Option<String> {
        let names = std::env::var_os("PATH")?;
        for d in std::env::split_paths(&names) {
            let c = d.join(name);
            if c.is_file() {
                return Some(c.to_string_lossy().into_owned());
            }
        }
        None
    }
}

/// Detect the engine from the resolved browser path or its symlink target.
fn detect_engine(spec: &str, _path: &Path, forced: crate::cli::Family) -> Engine {
    if forced != crate::cli::Family::Auto {
        return match forced {
            crate::cli::Family::Chromium => Engine::Chromium,
            crate::cli::Family::Mullvad => Engine::Gecko,
            crate::cli::Family::Auto => unreachable!(),
        };
    }
    let s = spec.to_ascii_lowercase();
    if s.contains("chrome") || s.contains("chromium") || s.contains("brave") || s.contains("edg") {
        Engine::Chromium
    } else {
        // mullvad, tor, firefox, libxul, etc.
        Engine::Gecko
    }
}

/// Persistent profile home for this tool.
fn profile_dir() -> Option<PathBuf> {
    tool_data_dir("grabsite").map(|d| d.join("profile"))
}

fn ts_tag() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{now}")
}

/// Build the browser argv for the engine and url.
fn engine_args(engine: Engine, profile: &Path, url: &str) -> Vec<String> {
    match engine {
        Engine::Gecko => vec![
            "-profile".into(),
            profile.to_string_lossy().into_owned(),
            "--private-window".into(),
            "-no-remote".into(),
            url.into(),
        ],
        Engine::Chromium => vec![
            "--user-data-dir".into(),
            profile.to_string_lossy().into_owned(),
            "--no-first-run".into(),
            "--no-default-browser-check".into(),
            "--disable-sync".into(),
            "--incognito".into(),
            url.into(),
        ],
    }
}

/// The top-level `run` the binary calls. Returns the process exit code.
pub fn run(args: Vec<String>, out: &mut impl Write) -> Result<i32, String> {
    let p = cli::parse(args)?;

    match p.action {
        Action::Help => {
            let _ = out.write_all(cli::HELP.as_bytes());
            return Ok(0);
        }
        Action::Version => {
            let _ = writeln!(out, "{}", cli::VERSION);
            return Ok(0);
        }
        Action::WipeProfile => {
            let profile = profile_dir().ok_or("no $HOME / cannot resolve profile dir")?;
            if profile.exists() {
                fs::remove_dir_all(&profile)
                    .map_err(|e| format!("cannot wipe profile {}: {e}", profile.display()))?;
            }
            let _ = writeln!(out, "profile wiped: {}", profile.display());
            return Ok(0);
        }
        Action::Capture => {}
    }

    let c = Color::from_flags(&p.flags);
    let mut o = Out::new(p.quiet);

    // Resolve the browser.
    let (name, path) = resolve_browser(p.browser.as_deref())?;
    let engine = detect_engine(&name, &path, p.family);

    // Output path.
    let out_path = p
        .out
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("grabsite-{}.png", ts_tag())));
    // Make sure the output directory exists.
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            if let Err(e) = fs::create_dir_all(parent) {
                return Err(format!(
                    "cannot create output dir {}: {e}",
                    parent.display()
                ));
            }
        }
    }

    // Scratch companion path for the settle loop (two-frame compare).
    let stem = out_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "capture".to_string());
    let prev_path = out_path.with_file_name(format!("{}.{}.prev.png", stem, std::process::id()));

    let mut display = match display::start(p.display, p.screen, Duration::from_secs(15)) {
        Ok(d) => d,
        Err(e) => {
            o.warn(format!("{} {e}", c.yellow("⚠")));
            return Ok(2);
        }
    };
    o.step(format!(
        "{} up ({}) {}",
        c.cyan(format!("Xvfb :{}", display.number())),
        format_args!("{}x{}", p.screen.0, p.screen.1),
        if display.is_shared() {
            c.yellow("(reused)")
        } else {
            String::new()
        }
    ));

    // Prepare the profile.
    let profile = profile_dir().ok_or("no $HOME / cannot resolve profile dir")?;
    if p.wipe_profile {
        let _ = fs::remove_dir_all(&profile);
        o.step(format!("{} profile wiped", c.cyan("profile")));
    }
    if let Err(e) = fs::create_dir_all(&profile) {
        return Err(format!("cannot create profile {}: {e}", profile.display()));
    }
    o.step(format!("{} ready", c.cyan("profile")));

    // Launch the browser in GUI mode on the display.
    let launched = Instant::now();
    let mut cmd = Command::new(&name);
    cmd.args(engine_args(
        engine,
        &profile,
        p.url.as_deref().unwrap_or("about:blank"),
    ))
    .env("DISPLAY", format!(":{}", display.number()))
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    own_process_group(&mut cmd);

    let mut browser: Option<Child> = match cmd.spawn() {
        Ok(child) => Some(child),
        Err(e) => {
            o.warn(format!("{} failed to launch {name}: {e}", c.yellow("⚠")));
            display.close();
            return Ok(3);
        }
    };
    o.step(format!(
        "{} ({}) loading {}",
        c.cyan("browser"),
        engine.as_str(),
        p.url.as_deref().unwrap_or("about:blank")
    ));

    let settle_result = capture::wait_stable(
        &mut o,
        &c,
        display.number(),
        &out_path,
        &prev_path,
        &capture::Budget {
            settle: Duration::from_secs(p.settle_s),
            launch: launched,
            deadline: Instant::now() + Duration::from_secs(p.timeout_s),
        },
    );

    // Stop the browser (always, before the display).
    if let Some(ref mut b) = browser {
        local_common::process::terminate(b);
    }

    // Kill our display (shared ones are left running — the user owns them).
    display.close();

    // Clean scratch.
    let _ = fs::remove_file(&prev_path);

    if let Err(e) = settle_result {
        o.warn(format!("{} {e}", c.yellow("⚠")));
        return Ok(4);
    }

    // Inspect the result.
    let bytes = fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
    if bytes < NEAR_BLANK_PNG {
        o.warn(format!(
            "{} capture is only {bytes} B — the page may not have painted (near-blank)",
            c.yellow("⚠")
        ));
    }

    let msg = format!(
        "wrote {} B {} in {}",
        with_commas(bytes),
        display_path(&out_path),
        o.elapsed()
    );
    let _ = writeln!(out, "{msg}");
    let _ = out.flush();

    Ok(0)
}

fn with_commas(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::new();
    for (i, b) in bytes.iter().enumerate() {
        let rem = (bytes.len() - i) % 3;
        if rem == 0 && i != 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

fn display_path(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
