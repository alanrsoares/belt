//! CLI argument parsing for `grabsite`.

use local_common::{split_flag, ArgCursor, CommonFlags};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Action {
    /// Capture a URL (the default action).
    #[default]
    Capture,
    /// Wipe the persistent profile without capturing.
    WipeProfile,
    /// Print help.
    Help,
    /// Print version.
    Version,
}

/// How the target browser opens its window/args.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Family {
    /// Detect from the resolved browser path.
    #[default]
    Auto,
    /// Gecko family: Mullvad Browser, Tor Browser, Firefox (`-profile`,
    /// `--private-window`).
    Mullvad,
    /// Chromium family: Chrome, Chromium, Brave (`--user-data-dir`,
    /// `--incognito`).
    Chromium,
}

impl Family {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "auto" => Ok(Self::Auto),
            "mullvad" | "gecko" | "firefox" => Ok(Self::Mullvad),
            "chromium" | "chrome" => Ok(Self::Chromium),
            other => Err(format!(
                "unknown family '{other}' (expected auto, mullvad, or chromium)"
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Mullvad => "mullvad",
            Self::Chromium => "chromium",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub action: Action,
    pub url: Option<String>,
    pub out: Option<String>,
    pub browser: Option<String>,
    pub family: Family,
    pub display: Option<u16>,
    pub screen: (u16, u16),
    pub timeout_s: u64,
    pub settle_s: u64,
    pub wipe_profile: bool,
    pub quiet: bool,
    pub flags: CommonFlags,
}

impl Default for Parsed {
    fn default() -> Self {
        Self {
            action: Action::default(),
            url: None,
            out: None,
            browser: None,
            family: Family::default(),
            display: None,
            screen: (1380, 898),
            timeout_s: 90,
            settle_s: 8,
            wipe_profile: false,
            quiet: false,
            flags: CommonFlags::default(),
        }
    }
}

/// Parse `screen` as `WxH` (or `WxHxD`).
fn parse_screen(s: &str) -> Result<(u16, u16), String> {
    let (dim, extra) = s
        .split_once('x')
        .ok_or_else(|| format!("invalid --screen '{s}' (expected WxH)"))?;
    let w: u16 = dim
        .parse()
        .map_err(|_| format!("invalid --screen width in '{s}'"))?;
    let h_part = extra.split_once('x').map(|(h, _)| h).unwrap_or(extra);
    let h: u16 = h_part
        .parse()
        .map_err(|_| format!("invalid --screen height in '{s}'"))?;
    Ok((w, h))
}

/// Parse a preset display like `:60` or `:0` into its number.
fn parse_display(s: &str) -> Result<u16, String> {
    let num = s
        .strip_prefix(':')
        .ok_or_else(|| format!("invalid --display '{s}' (expected :N)"))?;
    let n: u16 = num
        .parse()
        .map_err(|_| format!("invalid --display number in '{s}'"))?;
    if (0..=99).contains(&n) {
        Ok(n)
    } else {
        Err(format!("display number out of range in '{s}' (0..99)"))
    }
}

pub const VERSION: &str = "grabsite 0.1.0";

pub const HELP: &str = r#"grabsite [OPTIONS] [URL] — screenshot a website through its GUI browser.

Drives the browser window directly, which is the only path that works for
hardened builds (Mullvad Browser, Tor Browser) where the CDP and headless
screenshot surfaces are intentionally disabled. Spins up a private X display
(or reuses one with --display), loads the URL with a persistent profile,
waits for two identical captures, and writes the root window to PNG.

USAGE
    grabsite https://example.com
    grabsite --browser chrome https://example.com
    grabsite -o shot.png --timeout 120 https://example.com
    grabsite --display :0 https://example.com    # reuse an existing display
    grabsite --wipe-profile                     # wipe persistent profile and exit

OPTIONS
    -b, --browser <PATH|NAME>   browser binary (default: mullvad-browser)
    -o, --out <FILE>            output PNG (default: grabsite-<ts>.png)
        --family <NAME>         force engine family: auto | mullvad | chromium
        --display <:N>          reuse an existing X display instead of spawning Xvfb
        --screen <WxH>          virtual screen size (default: 1380x898)
        --timeout <SECS>        total budget for load + settle (default: 90)
        --settle <SECS>         delay before the first capture (default: 8)
        --wipe-profile          delete the persistent profile before launching (or wipe and exit)
    -q, --quiet                 print only the result line
        --color                 force colour output
        --no-color              disable colour output
    -h, --help                  print this help
    -V, --version               print version
"#;

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Parsed, String> {
    let mut cursor = ArgCursor::new(args.into_iter());
    let mut p = Parsed::default();

    while let Some(a) = cursor.next() {
        let (flag, inline) = split_flag(&a);
        match flag {
            "-h" | "--help" => p.action = Action::Help,
            "-V" | "--version" => p.action = Action::Version,
            "-q" | "--quiet" => p.quiet = true,
            "--wipe-profile" => p.wipe_profile = true,
            f if p.flags.check_arg(f) => {}
            "-b" | "--browser" => {
                let v = cursor.require_value("--browser", inline)?;
                p.browser = Some(v);
            }
            "-o" | "--out" => {
                let v = cursor.require_value("--out", inline)?;
                p.out = Some(v);
            }
            "--family" => {
                let v = cursor.require_value("--family", inline)?;
                p.family = Family::parse(&v)?;
            }
            "--display" => {
                let v = cursor.require_value("--display", inline)?;
                p.display = Some(parse_display(&v)?);
            }
            "--screen" => {
                let v = cursor.require_value("--screen", inline)?;
                p.screen = parse_screen(&v)?;
            }
            "--timeout" => {
                let v = cursor.require_value("--timeout", inline)?;
                p.timeout_s = v.parse().map_err(|_| format!("invalid --timeout '{v}'"))?;
            }
            "--settle" => {
                let v = cursor.require_value("--settle", inline)?;
                p.settle_s = v.parse().map_err(|_| format!("invalid --settle '{v}'"))?;
            }
            s if s.starts_with('-') => return Err(format!("unknown flag: {s}")),
            s => {
                if p.url.is_some() {
                    return Err(format!("unexpected argument: {s}"));
                }
                if !s.starts_with("http://")
                    && !s.starts_with("https://")
                    && !s.starts_with("file://")
                {
                    return Err(format!(
                        "URL must start with http://, https://, or file:// (got '{s}')"
                    ));
                }
                p.url = Some(s.to_string());
            }
        }
    }

    if p.action == Action::Capture && p.url.is_none() {
        if p.wipe_profile {
            p.action = Action::WipeProfile;
        } else {
            return Err(
                "missing URL\n  try: grabsite https://example.com\n  or: grabsite --help"
                    .to_string(),
            );
        }
    }

    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_url_and_out() {
        let p = parse(strs(&["https://example.com", "-o", "shot.png"])).unwrap();
        assert_eq!(p.url.as_deref(), Some("https://example.com"));
        assert_eq!(p.out.as_deref(), Some("shot.png"));
        assert_eq!(p.action, Action::Capture);
    }

    #[test]
    fn parse_inline_flags() {
        let p = parse(strs(&[
            "--timeout=30",
            "--screen=800x600",
            "https://x.test",
        ]))
        .unwrap();
        assert_eq!(p.timeout_s, 30);
        assert_eq!(p.screen, (800, 600));
    }

    #[test]
    fn family_and_browser() {
        let p = parse(strs(&[
            "-b",
            "chrome",
            "--family",
            "chromium",
            "https://x.test",
        ]))
        .unwrap();
        assert_eq!(p.browser.as_deref(), Some("chrome"));
        assert_eq!(p.family, Family::Chromium);
    }

    #[test]
    fn display_parsing() {
        let p = parse(strs(&["--display", ":60", "https://x.test"])).unwrap();
        assert_eq!(p.display, Some(60));
        let p0 = parse(strs(&["--display", ":0", "https://x.test"])).unwrap();
        assert_eq!(p0.display, Some(0));
        assert!(parse(strs(&["--display", "60", "https://x.test"])).is_err());
        assert!(parse(strs(&["--display", ":100", "https://x.test"])).is_err());
        assert!(parse(strs(&["--display", ":999", "https://x.test"])).is_err());
    }

    #[test]
    fn screen_rejects_garbage() {
        assert!(parse(strs(&["--screen", "abc", "https://x.test"])).is_err());
        assert!(parse(strs(&["--screen", "800", "https://x.test"])).is_err());
    }

    #[test]
    fn url_scheme_is_required() {
        assert!(parse(strs(&["example.com"])).is_err());
        assert!(parse(Vec::<String>::new()).is_err());
    }

    #[test]
    fn wipe_profile_standalone() {
        let p = parse(strs(&["--wipe-profile"])).unwrap();
        assert_eq!(p.action, Action::WipeProfile);
        assert!(p.wipe_profile);

        let p_with_url = parse(strs(&["--wipe-profile", "https://x.test"])).unwrap();
        assert_eq!(p_with_url.action, Action::Capture);
        assert!(p_with_url.wipe_profile);
    }

    #[test]
    fn unknown_flag_errors() {
        assert!(parse(strs(&["--nope", "https://x.test"])).is_err());
    }

    #[test]
    fn second_positional_errors() {
        assert!(parse(strs(&["https://a.test", "https://b.test"])).is_err());
    }
}
