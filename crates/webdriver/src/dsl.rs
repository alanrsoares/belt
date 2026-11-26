//! DSL grammar and step representation for browser automation.

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Goto(String),
    Viewport {
        width: u32,
        height: u32,
    },
    WaitFor {
        selector: String,
        timeout_ms: u64,
    },
    Wait {
        duration_ms: u64,
    },
    Click {
        selector: String,
    },
    Type {
        selector: String,
        text: String,
        clear: bool,
    },
    Eval {
        expr: String,
    },
    Screenshot {
        path: String,
        full_page: bool,
        selector: Option<String>,
    },
    Pdf {
        path: String,
    },
    Html {
        path: Option<String>,
    },
    Reload {
        timeout_ms: u64,
    },
    WaitForUrl {
        substring: String,
        timeout_ms: u64,
    },
    WaitForHydration {
        quiet_ms: u64,
        timeout_ms: u64,
    },
    Press {
        chord: String,
    },
    Hover {
        selector: String,
    },
    Upload {
        selector: String,
        path: String,
    },
    Url,
    Title,
    A11y,
    Dialog {
        accept: bool,
        text: Option<String>,
    },
    Find {
        query: String,
    },
    Text {
        selector: Option<String>,
    },
    Snapshot {
        all: bool,
    },
    Frame {
        target: Option<String>,
    },
    Console {
        mode: ConsoleMode,
    },
    /// Start a screencast to `path`, or stop it when `None`.
    Record {
        path: Option<String>,
    },
    /// Play the video from here at `factor` times real speed.
    RecordSpeed {
        factor: f64,
    },
    /// Run `then` only if `selector` is visible now (`visible`), or only if it
    /// is not. Checked once, without waiting.
    If {
        selector: String,
        visible: bool,
        then: Box<Step>,
    },
    /// Overlay caption; `duration_ms == 0` keeps it until the next caption.
    Caption {
        text: String,
        duration_ms: u64,
    },
    Close,
}

/// What `console` should print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleMode {
    /// Errors and warnings only, stacks truncated, repeats collapsed.
    Errors,
    /// Every buffered entry, untruncated and uncollapsed.
    Full,
    /// Drop the buffer.
    Clear,
}

/// Parse a duration string like "60s", "500ms", "2m", "10".
pub fn parse_duration_ms(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration string".to_string());
    }

    if let Some(ms_str) = s.strip_suffix("ms") {
        ms_str
            .parse::<u64>()
            .map_err(|e| format!("invalid millisecond duration '{s}': {e}"))
    } else if let Some(s_str) = s.strip_suffix('s') {
        let secs = s_str
            .parse::<f64>()
            .map_err(|e| format!("invalid second duration '{s}': {e}"))?;
        Ok((secs * 1000.0).round() as u64)
    } else if let Some(m_str) = s.strip_suffix('m') {
        let mins = m_str
            .parse::<f64>()
            .map_err(|e| format!("invalid minute duration '{s}': {e}"))?;
        Ok((mins * 60_000.0).round() as u64)
    } else {
        // Default bare numbers to seconds
        let secs = s
            .parse::<f64>()
            .map_err(|e| format!("invalid duration '{s}': {e}"))?;
        Ok((secs * 1000.0).round() as u64)
    }
}

/// Parse a multi-line script string into DSL steps.
pub fn parse_script(script: &str, default_timeout_ms: u64) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();

    for line in script.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }

        let tokens = tokenize_line(trimmed)?;
        if tokens.is_empty() {
            continue;
        }

        let parsed = parse_tokens(&tokens, default_timeout_ms)?;
        steps.extend(parsed);
    }

    Ok(steps)
}

/// Split a line into tokens, respecting single and double quotes.
pub fn tokenize_line(line: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current: Option<String> = None;
    let mut in_quote = None;
    let mut escape = false;

    for ch in line.chars() {
        if escape {
            current.get_or_insert_with(String::new).push(ch);
            escape = false;
            continue;
        }

        if ch == '\\' && in_quote != Some('\'') {
            escape = true;
            current.get_or_insert_with(String::new);
            continue;
        }

        if let Some(q) = in_quote {
            if ch == q {
                in_quote = None;
            } else {
                current.get_or_insert_with(String::new).push(ch);
            }
        } else if (ch == '"' || ch == '\'') && current.is_none() {
            // Quotes only group when they open a token, so JS like
            // `eval document.getElementById("out")` keeps its inner quotes.
            in_quote = Some(ch);
            current = Some(String::new());
        } else if ch.is_whitespace() {
            if let Some(tok) = current.take() {
                tokens.push(tok);
            }
        } else {
            current.get_or_insert_with(String::new).push(ch);
        }
    }

    if escape {
        current.get_or_insert_with(String::new).push('\\');
    }

    if in_quote.is_some() {
        return Err("unclosed quote in line".to_string());
    }

    if let Some(tok) = current {
        tokens.push(tok);
    }

    Ok(tokens)
}

/// Parse a slice of string tokens into DSL steps.
pub fn parse_tokens(tokens: &[String], default_timeout_ms: u64) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    let mut idx = 0;

    while idx < tokens.len() {
        let tok = &tokens[idx];

        // First positional or bare URL
        if idx == 0 && is_url(tok) {
            steps.push(Step::Goto(tok.clone()));
            idx += 1;
            continue;
        }

        match tok.as_str() {
            "goto" => {
                idx += 1;
                if idx >= tokens.len() {
                    return Err("'goto' requires a URL argument".to_string());
                }
                steps.push(Step::Goto(tokens[idx].clone()));
                idx += 1;
            }
            "viewport" => {
                idx += 1;
                if idx >= tokens.len() {
                    return Err("'viewport' requires width and height arguments".to_string());
                }
                let w_str = &tokens[idx];
                idx += 1;
                let (w, h) = if w_str.contains('x') || w_str.contains('*') {
                    let parts: Vec<&str> = w_str.split(['x', '*']).collect();
                    if parts.len() != 2 {
                        return Err(format!("invalid viewport dimensions '{w_str}'"));
                    }
                    let w = parts[0]
                        .parse::<u32>()
                        .map_err(|e| format!("invalid width: {e}"))?;
                    let h = parts[1]
                        .parse::<u32>()
                        .map_err(|e| format!("invalid height: {e}"))?;
                    (w, h)
                } else {
                    if idx >= tokens.len() {
                        return Err("'viewport' requires height after width".to_string());
                    }
                    let w = w_str
                        .parse::<u32>()
                        .map_err(|e| format!("invalid width: {e}"))?;
                    let h = tokens[idx]
                        .parse::<u32>()
                        .map_err(|e| format!("invalid height: {e}"))?;
                    idx += 1;
                    (w, h)
                };
                steps.push(Step::Viewport {
                    width: w,
                    height: h,
                });
            }
            "wait-for" => {
                idx += 1;
                if idx >= tokens.len() {
                    return Err("'wait-for' requires a locator argument".to_string());
                }
                let selector = tokens[idx].clone();
                idx += 1;

                let mut timeout_ms = default_timeout_ms;
                if idx < tokens.len() {
                    if tokens[idx] == "--timeout" {
                        idx += 1;
                        if idx >= tokens.len() {
                            return Err("'--timeout' requires a duration argument".to_string());
                        }
                        timeout_ms = parse_duration_ms(&tokens[idx])?;
                        idx += 1;
                    } else if let Ok(d) = parse_duration_ms(&tokens[idx]) {
                        timeout_ms = d;
                        idx += 1;
                    }
                }
                steps.push(Step::WaitFor {
                    selector,
                    timeout_ms,
                });
            }
            "wait" => {
                idx += 1;
                if idx >= tokens.len() {
                    return Err("'wait' requires a duration argument (e.g. 500ms, 2s)".to_string());
                }
                let duration_ms = parse_duration_ms(&tokens[idx])?;
                steps.push(Step::Wait { duration_ms });
                idx += 1;
            }
            "if" | "unless" => {
                idx += 1;
                let selector = tokens
                    .get(idx)
                    .cloned()
                    .ok_or_else(|| format!("'{tok}' requires <locator> <step>"))?;
                let mut rest = parse_tokens(&tokens[idx + 1..], default_timeout_ms)?.into_iter();
                let then = rest
                    .next()
                    .ok_or_else(|| format!("'{tok}' requires a step after the locator"))?;
                steps.push(Step::If {
                    selector,
                    visible: tok == "if",
                    then: Box::new(then),
                });
                steps.extend(rest);
                idx = tokens.len();
            }
            "record" if tokens.get(idx + 1).is_some_and(|t| t == "speed") => {
                idx += 2;
                let factor = tokens
                    .get(idx)
                    .and_then(|t| t.parse::<f64>().ok())
                    .filter(|f| f.is_finite() && *f > 0.0)
                    .ok_or("'record speed' requires a factor above 0 (e.g. 4, 0.5)")?;
                idx += 1;
                steps.push(Step::RecordSpeed { factor });
            }
            "record" => {
                idx += 1;
                let path = match tokens.get(idx) {
                    Some(t) if t == "stop" => None,
                    Some(t) => Some(t.clone()),
                    None => return Err("'record' requires <path> or 'stop'".to_string()),
                };
                idx += 1;
                steps.push(Step::Record { path });
            }
            "caption" => {
                idx += 1;
                // Always takes the next argument as text, so `caption close`
                // shows "close". Bare `caption` or `caption ""` clears it.
                let text = tokens.get(idx).cloned().unwrap_or_default();
                idx += 1;
                let mut duration_ms = 0;
                if let Some(d) = tokens.get(idx).and_then(|t| parse_duration_ms(t).ok()) {
                    duration_ms = d;
                    idx += 1;
                }
                steps.push(Step::Caption { text, duration_ms });
            }
            "click" => {
                idx += 1;
                let selector = tokens
                    .get(idx)
                    .cloned()
                    .ok_or_else(|| "'click' requires a locator argument".to_string())?;
                idx += 1;
                steps.push(Step::Click { selector });
            }
            "type" | "fill" => {
                let clear = tok == "fill";
                idx += 1;
                if idx + 1 >= tokens.len() {
                    return Err(format!("'{tok}' requires <locator> and <text> arguments"));
                }
                let selector = tokens[idx].clone();
                let text = tokens[idx + 1].clone();
                steps.push(Step::Type {
                    selector,
                    text,
                    clear,
                });
                idx += 2;
            }
            "eval" => {
                idx += 1;
                if idx >= tokens.len() {
                    return Err("'eval' requires a JavaScript expression argument".to_string());
                }
                steps.push(Step::Eval {
                    expr: tokens[idx].clone(),
                });
                idx += 1;
            }
            "screenshot" => {
                idx += 1;
                let mut path = "screenshot.png".to_string();
                let mut full_page = false;
                let mut selector = None;

                while idx < tokens.len() {
                    if tokens[idx] == "--full-page" {
                        full_page = true;
                        idx += 1;
                    } else if tokens[idx] == "--selector" {
                        idx += 1;
                        if idx >= tokens.len() {
                            return Err("'--selector' requires a locator argument".to_string());
                        }
                        selector = Some(tokens[idx].clone());
                        idx += 1;
                    } else if !tokens[idx].starts_with('-')
                        && (tokens[idx].ends_with(".png")
                            || tokens[idx].ends_with(".jpg")
                            || tokens[idx].ends_with(".jpeg")
                            || !is_verb(&tokens[idx]))
                    {
                        path = tokens[idx].clone();
                        idx += 1;
                    } else {
                        break;
                    }
                }
                steps.push(Step::Screenshot {
                    path,
                    full_page,
                    selector,
                });
            }
            "pdf" => {
                idx += 1;
                let path = if idx < tokens.len()
                    && !tokens[idx].starts_with('-')
                    && !is_verb(&tokens[idx])
                {
                    let p = tokens[idx].clone();
                    idx += 1;
                    p
                } else {
                    "page.pdf".to_string()
                };
                steps.push(Step::Pdf { path });
            }
            "reload" => {
                idx += 1;
                steps.push(Step::Reload {
                    timeout_ms: default_timeout_ms,
                });
            }
            "wait-for-url" => {
                idx += 1;
                if idx >= tokens.len() {
                    return Err("'wait-for-url' requires a URL substring".to_string());
                }
                let substring = tokens[idx].clone();
                idx += 1;
                let mut timeout_ms = default_timeout_ms;
                if idx < tokens.len() && !is_verb(&tokens[idx]) {
                    if let Ok(d) = parse_duration_ms(&tokens[idx]) {
                        timeout_ms = d;
                        idx += 1;
                    }
                }
                steps.push(Step::WaitForUrl {
                    substring,
                    timeout_ms,
                });
            }
            "wait-for-hydration" | "settle" => {
                idx += 1;
                let mut quiet_ms = 500;
                if idx < tokens.len() && !is_verb(&tokens[idx]) {
                    if let Ok(d) = parse_duration_ms(&tokens[idx]) {
                        quiet_ms = d;
                        idx += 1;
                    }
                }
                steps.push(Step::WaitForHydration {
                    quiet_ms,
                    timeout_ms: default_timeout_ms,
                });
            }
            "press" => {
                idx += 1;
                if idx >= tokens.len() {
                    return Err("'press' requires a key (e.g. Enter, Meta+O)".to_string());
                }
                steps.push(Step::Press {
                    chord: tokens[idx].clone(),
                });
                idx += 1;
            }
            "upload" => {
                idx += 1;
                let (Some(selector), Some(path)) = (tokens.get(idx), tokens.get(idx + 1)) else {
                    return Err("'upload' requires <locator> and <path> arguments".to_string());
                };
                steps.push(Step::Upload {
                    selector: selector.clone(),
                    path: path.clone(),
                });
                idx += 2;
            }
            "hover" => {
                idx += 1;
                let selector = tokens
                    .get(idx)
                    .cloned()
                    .ok_or_else(|| "'hover' requires a locator argument".to_string())?;
                idx += 1;
                steps.push(Step::Hover { selector });
            }
            "snapshot" => {
                idx += 1;
                let mut all = false;
                if idx < tokens.len() && tokens[idx] == "--all" {
                    all = true;
                    idx += 1;
                }
                steps.push(Step::Snapshot { all });
            }
            "frame" => {
                idx += 1;
                let target = if idx < tokens.len() && !is_verb(&tokens[idx]) {
                    let t = tokens[idx].clone();
                    idx += 1;
                    if t == "--top" || t == "top" {
                        None
                    } else {
                        Some(t)
                    }
                } else {
                    None
                };
                steps.push(Step::Frame { target });
            }
            "url" => {
                idx += 1;
                steps.push(Step::Url);
            }
            "title" => {
                idx += 1;
                steps.push(Step::Title);
            }
            "a11y" => {
                idx += 1;
                steps.push(Step::A11y);
            }
            "dialog" => {
                idx += 1;
                let accept = match tokens.get(idx).map(String::as_str) {
                    Some("accept") => true,
                    Some("dismiss") => false,
                    _ => return Err("dialog requires accept or dismiss".into()),
                };
                idx += 1;
                let text = if accept
                    && idx < tokens.len()
                    && !is_verb(&tokens[idx])
                    && !is_url(&tokens[idx])
                {
                    idx += 1;
                    Some(tokens[idx - 1].clone())
                } else {
                    None
                };
                steps.push(Step::Dialog { accept, text });
            }
            "find" => {
                idx += 1;
                let query = tokens
                    .get(idx)
                    .cloned()
                    .ok_or("find requires a query (role or name text)")?;
                idx += 1;
                steps.push(Step::Find { query });
            }
            "text" => {
                idx += 1;
                let selector =
                    if idx < tokens.len() && !is_verb(&tokens[idx]) && !is_url(&tokens[idx]) {
                        idx += 1;
                        Some(tokens[idx - 1].clone())
                    } else {
                        None
                    };
                steps.push(Step::Text { selector });
            }
            "console" => {
                idx += 1;
                let mut mode = ConsoleMode::Errors;
                if idx < tokens.len() {
                    match tokens[idx].as_str() {
                        "--errors" => {
                            idx += 1;
                        }
                        "--full" => {
                            mode = ConsoleMode::Full;
                            idx += 1;
                        }
                        "--clear" => {
                            mode = ConsoleMode::Clear;
                            idx += 1;
                        }
                        _ => {}
                    }
                }
                steps.push(Step::Console { mode });
            }
            "close" => {
                idx += 1;
                steps.push(Step::Close);
            }
            "html" => {
                idx += 1;
                let path = if idx < tokens.len()
                    && !tokens[idx].starts_with('-')
                    && !is_verb(&tokens[idx])
                {
                    let p = tokens[idx].clone();
                    idx += 1;
                    Some(p)
                } else {
                    None
                };
                steps.push(Step::Html { path });
            }
            other if is_url(other) => {
                steps.push(Step::Goto(other.to_string()));
                idx += 1;
            }
            unknown => {
                return Err(format!("unknown command or verb '{unknown}'"));
            }
        }
    }

    Ok(steps)
}

fn is_url(s: &str) -> bool {
    s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("file://")
        || s.starts_with("data:")
        || s.starts_with("about:")
}

fn is_verb(s: &str) -> bool {
    matches!(
        s,
        "goto"
            | "viewport"
            | "wait-for"
            | "wait"
            | "click"
            | "type"
            | "fill"
            | "eval"
            | "screenshot"
            | "pdf"
            | "html"
            | "reload"
            | "wait-for-url"
            | "wait-for-hydration"
            | "settle"
            | "press"
            | "hover"
            | "upload"
            | "url"
            | "title"
            | "a11y"
            | "dialog"
            | "text"
            | "find"
            | "console"
            | "snapshot"
            | "frame"
            | "record"
            | "caption"
            | "if"
            | "unless"
            | "close"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_user_example() {
        let args = vec![
            "http://localhost:3000".to_string(),
            "wait-for".to_string(),
            ".my-css-selector".to_string(),
            "--timeout".to_string(),
            "60s".to_string(),
            "screenshot".to_string(),
        ];

        let steps = parse_tokens(&args, 30_000).expect("parse failed");
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0], Step::Goto("http://localhost:3000".to_string()));
        assert_eq!(
            steps[1],
            Step::WaitFor {
                selector: ".my-css-selector".to_string(),
                timeout_ms: 60_000
            }
        );
        assert_eq!(
            steps[2],
            Step::Screenshot {
                path: "screenshot.png".to_string(),
                full_page: false,
                selector: None
            }
        );
    }

    #[test]
    fn parse_script_multiline() {
        let script = r#"
        goto https://example.com
        viewport 1280 800
        click #login-btn
        type #email "test@user.com"
        wait 500ms
        screenshot out.png --full-page
        "#;

        let steps = parse_script(script, 30_000).expect("script parse failed");
        assert_eq!(steps.len(), 6);
        assert_eq!(steps[0], Step::Goto("https://example.com".to_string()));
        assert_eq!(
            steps[1],
            Step::Viewport {
                width: 1280,
                height: 800
            }
        );
        assert_eq!(
            steps[2],
            Step::Click {
                selector: "#login-btn".to_string()
            }
        );
        assert_eq!(
            steps[3],
            Step::Type {
                selector: "#email".to_string(),
                text: "test@user.com".to_string(),
                clear: false
            }
        );
        assert_eq!(steps[4], Step::Wait { duration_ms: 500 });
        assert_eq!(
            steps[5],
            Step::Screenshot {
                path: "out.png".to_string(),
                full_page: true,
                selector: None,
            }
        );
    }

    #[test]
    fn inner_quotes_survive_in_unquoted_tokens() {
        let tokens = tokenize_line("eval document.getElementById(\"out\").textContent").unwrap();
        assert_eq!(tokens[1], "document.getElementById(\"out\").textContent");

        let quoted = tokenize_line("fill '#a' 'two words'").unwrap();
        assert_eq!(quoted, vec!["fill", "#a", "two words"]);
    }

    #[test]
    fn parse_new_verbs() {
        let steps = parse_script(
            "wait-for-url /dashboard 5s\nwait-for-hydration 300ms\npress Meta+O\nhover text=Save\nconsole --full\nurl\ntitle\nreload\nclose",
            30_000,
        )
        .expect("parse failed");

        assert_eq!(
            steps[0],
            Step::WaitForUrl {
                substring: "/dashboard".to_string(),
                timeout_ms: 5_000
            }
        );
        assert_eq!(
            steps[1],
            Step::WaitForHydration {
                quiet_ms: 300,
                timeout_ms: 30_000
            }
        );
        assert_eq!(
            steps[2],
            Step::Press {
                chord: "Meta+O".to_string()
            }
        );
        assert_eq!(
            steps[3],
            Step::Hover {
                selector: "text=Save".to_string()
            }
        );
        assert_eq!(
            steps[4],
            Step::Console {
                mode: ConsoleMode::Full
            }
        );
        assert_eq!(steps[5], Step::Url);
        assert_eq!(steps[6], Step::Title);
        assert_eq!(steps[7], Step::Reload { timeout_ms: 30_000 });
        assert_eq!(steps[8], Step::Close);
    }

    #[test]
    fn click_takes_single_locator_token() {
        let steps = parse_script("click \"text=Late Button\"", 30_000).expect("parse failed");
        assert_eq!(
            steps[0],
            Step::Click {
                selector: "text=Late Button".to_string()
            }
        );

        // Unquoted multi-word locators are rejected uniformly across all verbs.
        assert!(parse_script("click text=Late Button", 30_000).is_err());
        assert!(parse_script("hover text=Late Button", 30_000).is_err());

        let mixed = parse_tokens(
            &[
                "click".to_string(),
                "role=button:Save".to_string(),
                "screenshot".to_string(),
                "out.png".to_string(),
            ],
            30_000,
        )
        .expect("parse failed");
        assert_eq!(
            mixed[0],
            Step::Click {
                selector: "role=button:Save".to_string()
            }
        );
        assert_eq!(
            mixed[1],
            Step::Screenshot {
                path: "out.png".to_string(),
                full_page: false,
                selector: None
            }
        );

        assert!(parse_tokens(&["click".to_string()], 30_000).is_err());
    }

    #[test]
    fn parse_durations() {
        assert_eq!(parse_duration_ms("500ms").unwrap(), 500);
        assert_eq!(parse_duration_ms("2s").unwrap(), 2000);
        assert_eq!(parse_duration_ms("1.5s").unwrap(), 1500);
        assert_eq!(parse_duration_ms("2m").unwrap(), 120_000);
        assert_eq!(parse_duration_ms("10").unwrap(), 10_000);
        assert!(parse_duration_ms("").is_err());
        assert!(parse_duration_ms("abc").is_err());
    }

    #[test]
    fn parse_all_verbs() {
        let args = vec![
            "https://test.local".to_string(),
            "viewport".to_string(),
            "1920x1080".to_string(),
            "eval".to_string(),
            "1 + 1".to_string(),
            "fill".to_string(),
            "#user".to_string(),
            "admin".to_string(),
            "pdf".to_string(),
            "doc.pdf".to_string(),
            "html".to_string(),
            "dump.html".to_string(),
        ];

        let steps = parse_tokens(&args, 30_000).expect("parse failed");
        assert_eq!(steps.len(), 6);
        assert_eq!(steps[0], Step::Goto("https://test.local".to_string()));
        assert_eq!(
            steps[1],
            Step::Viewport {
                width: 1920,
                height: 1080
            }
        );
        assert_eq!(
            steps[2],
            Step::Eval {
                expr: "1 + 1".to_string()
            }
        );
        assert_eq!(
            steps[3],
            Step::Type {
                selector: "#user".to_string(),
                text: "admin".to_string(),
                clear: true
            }
        );
        assert_eq!(
            steps[4],
            Step::Pdf {
                path: "doc.pdf".to_string()
            }
        );
        assert_eq!(
            steps[5],
            Step::Html {
                path: Some("dump.html".to_string())
            }
        );
    }

    #[test]
    fn parse_errors() {
        assert!(parse_tokens(&["click".to_string()], 30_000).is_err());
        assert!(parse_tokens(&["wait-for".to_string()], 30_000).is_err());
        assert!(parse_tokens(&["viewport".to_string()], 30_000).is_err());
        assert!(parse_tokens(&["invalid_verb".to_string()], 30_000).is_err());
    }

    #[test]
    fn dangling_timeout_and_selector_flags_error() {
        assert!(parse_tokens(
            &[
                "wait-for".to_string(),
                "#el".to_string(),
                "--timeout".to_string()
            ],
            30_000
        )
        .is_err());
        assert!(parse_tokens(
            &["screenshot".to_string(), "--selector".to_string()],
            30_000
        )
        .is_err());
    }

    #[test]
    fn dialog_takes_accept_or_dismiss() {
        let steps = parse_script(
            "dialog dismiss\ndialog accept hello\ndialog accept\nurl",
            1000,
        )
        .unwrap();
        assert_eq!(
            steps[0],
            Step::Dialog {
                accept: false,
                text: None
            }
        );
        assert_eq!(
            steps[1],
            Step::Dialog {
                accept: true,
                text: Some("hello".into())
            }
        );
        assert_eq!(
            steps[2],
            Step::Dialog {
                accept: true,
                text: None
            }
        );
        assert!(parse_script("dialog", 1000).is_err());
        assert!(parse_script("dialog maybe", 1000).is_err());
    }

    #[test]
    fn a11y_is_a_verb() {
        let steps = parse_script("a11y\nurl", 1000).unwrap();
        assert_eq!(steps, vec![Step::A11y, Step::Url]);
    }

    #[test]
    fn text_takes_an_optional_selector() {
        let steps = parse_script("text\ntext article\nurl", 1000).unwrap();
        assert_eq!(steps[0], Step::Text { selector: None });
        assert_eq!(
            steps[1],
            Step::Text {
                selector: Some("article".into())
            }
        );
    }

    #[test]
    fn find_needs_a_query_and_allows_verb_names() {
        let steps = parse_script("find 'Link 250'\nfind close", 1000).unwrap();
        assert_eq!(
            steps,
            vec![
                Step::Find {
                    query: "Link 250".into()
                },
                Step::Find {
                    query: "close".into()
                },
            ]
        );
        assert!(parse_script("find", 1000).is_err());
    }

    #[test]
    fn tokenize_preserves_empty_quotes_and_single_quote_backslashes() {
        assert_eq!(
            tokenize_line(r#"fill #q """#).unwrap(),
            vec!["fill", "#q", ""]
        );
        assert_eq!(tokenize_line("fill #q ''").unwrap(), vec!["fill", "#q", ""]);
        assert_eq!(
            tokenize_line(r"eval 'C:\foo\bar'").unwrap(),
            vec!["eval", r"C:\foo\bar"]
        );
        assert_eq!(
            parse_script(r#"fill #q """#, 1000).unwrap(),
            vec![Step::Type {
                selector: "#q".into(),
                text: String::new(),
                clear: true,
            }]
        );
    }

    #[test]
    fn if_and_unless_guard_the_next_step() {
        let steps = parse_script(
            "if \"role=button:Review\" click role=button:Review\nunless .open hover .x\nurl",
            1000,
        )
        .unwrap();
        assert_eq!(
            steps,
            vec![
                Step::If {
                    selector: "role=button:Review".into(),
                    visible: true,
                    then: Box::new(Step::Click {
                        selector: "role=button:Review".into()
                    }),
                },
                Step::If {
                    selector: ".open".into(),
                    visible: false,
                    then: Box::new(Step::Hover {
                        selector: ".x".into()
                    }),
                },
                Step::Url,
            ]
        );
        // Only the first step is guarded; the rest of the line runs anyway.
        let line = parse_script("if .a click .b url", 1000).unwrap();
        assert!(matches!(line[0], Step::If { .. }));
        assert_eq!(line[1], Step::Url);
        assert!(parse_script("if .a", 1000).is_err());
        assert!(parse_script("unless", 1000).is_err());
    }

    #[test]
    fn record_speed_takes_a_positive_factor() {
        assert_eq!(
            parse_script("record speed 4", 1000).unwrap(),
            vec![Step::RecordSpeed { factor: 4.0 }]
        );
        assert!(parse_script("record speed 0", 1000).is_err());
        assert!(parse_script("record speed fast", 1000).is_err());
        assert!(parse_script("record speed", 1000).is_err());
    }

    #[test]
    fn tokenize_line_preserves_trailing_backslash() {
        assert_eq!(
            tokenize_line(r"eval foo\").unwrap(),
            vec!["eval".to_string(), r"foo\".to_string()]
        );
    }

    #[test]
    fn record_and_caption_parse() {
        let steps = parse_script(
            "record demo.mp4\ncaption \"Sign in\" 2s\ncaption hi\nrecord stop",
            1000,
        )
        .unwrap();
        assert_eq!(
            steps,
            vec![
                Step::Record {
                    path: Some("demo.mp4".into())
                },
                Step::Caption {
                    text: "Sign in".into(),
                    duration_ms: 2000
                },
                Step::Caption {
                    text: "hi".into(),
                    duration_ms: 0
                },
                Step::Record { path: None },
            ]
        );
        assert!(parse_script("record", 1000).is_err());
        assert_eq!(
            parse_script("record close", 1000).unwrap(),
            vec![Step::Record {
                path: Some("close".into())
            }]
        );
        assert_eq!(
            parse_script("upload \"#file\" ./a.docx", 1000).unwrap(),
            vec![Step::Upload {
                selector: "#file".into(),
                path: "./a.docx".into()
            }]
        );
        assert!(parse_script("upload #file", 1000).is_err());
        // A verb name is still caption text.
        assert_eq!(
            parse_script("caption close", 1000).unwrap(),
            vec![Step::Caption {
                text: "close".into(),
                duration_ms: 0
            }]
        );
        assert_eq!(
            parse_script("caption", 1000).unwrap(),
            vec![Step::Caption {
                text: String::new(),
                duration_ms: 0
            }]
        );
    }
}
