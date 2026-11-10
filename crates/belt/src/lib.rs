//! `belt` — runs any belt tool by name, and serves them all as a stdio MCP
//! server (JSON-RPC 2.0, one message per line).

use std::collections::HashMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use local_common::json::{parse, JsonValue};
use local_common::process::{own_process_group, terminate};
use local_common::strip_ansi;

/// Newest first. `initialize` echoes the client's version when listed here.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// A belt tool exposed over MCP.
pub struct Tool {
    pub name: &'static str,
    pub summary: &'static str,
    /// Can delete files, kill processes or write a project tree.
    pub destructive: bool,
}

pub const TOOLS: &[Tool] = &[
    Tool {
        name: "scaffold",
        summary: "Offline project scaffolder (TS/Bun/Biome, Rust, Go, Python/uv). Writes files into the working directory.",
        destructive: true,
    },
    Tool {
        name: "portkill",
        summary: "Inspect listening ports and kill the processes that hold them.",
        destructive: true,
    },
    Tool {
        name: "jwt",
        summary: "Decode a JWT and inspect or extract its claims.",
        destructive: false,
    },
    Tool {
        name: "devclean",
        summary: "Scan for build artifacts across ecosystems and reclaim disk space. `--clean` deletes them.",
        destructive: true,
    },
    Tool {
        name: "fanout",
        summary: "Run quality gates and task matrices concurrently, with DAG ordering and Git change detection.",
        destructive: false,
    },
    Tool {
        name: "webdriver",
        summary: "Browser automation: screenshots, PDF export, DOM dumps, clicks and persistent sessions (`--session`).",
        destructive: false,
    },
    Tool {
        name: "grabsite",
        summary: "Screenshot a page in a GUI browser for builds with CDP disabled.",
        destructive: false,
    },
];

pub fn find_tool(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

/// The tool's binary next to this executable, else the bare name for `PATH`.
pub fn resolve_bin(name: &str) -> PathBuf {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(&file)))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(file))
}

fn obj(pairs: Vec<(&str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        pairs
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect::<HashMap<_, _>>(),
    )
}

fn s(v: &str) -> JsonValue {
    JsonValue::String(v.to_string())
}

fn tool_json(tool: &Tool) -> JsonValue {
    obj(vec![
        ("name", s(tool.name)),
        (
            "description",
            s(&format!(
                "{} Pass CLI arguments in `args`; call with [\"--help\"] for usage.",
                tool.summary
            )),
        ),
        (
            "inputSchema",
            obj(vec![
                ("type", s("object")),
                (
                    "properties",
                    obj(vec![(
                        "args",
                        obj(vec![
                            ("type", s("array")),
                            ("items", obj(vec![("type", s("string"))])),
                            ("description", s("Command-line arguments, one per item.")),
                        ]),
                    )]),
                ),
                ("required", JsonValue::Array(vec![s("args")])),
            ]),
        ),
        (
            "annotations",
            obj(vec![
                ("readOnlyHint", JsonValue::Bool(!tool.destructive)),
                ("destructiveHint", JsonValue::Bool(tool.destructive)),
            ]),
        ),
    ])
}

/// Per-call limit, matching `fanout`'s default task timeout.
pub const TOOL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Max bytes kept per stream; the rest is read and discarded.
pub const OUTPUT_LIMIT: u64 = 1024 * 1024;

/// Read a pipe to the end on its own thread so a full pipe never blocks the
/// child. Keeps the first [`OUTPUT_LIMIT`] bytes and counts the rest.
fn drain(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<(Vec<u8>, u64)> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = Vec::new();
        let mut dropped = 0;
        if let Some(mut pipe) = pipe {
            let _ = (&mut pipe).take(OUTPUT_LIMIT).read_to_end(&mut buf);
            dropped = io::copy(&mut pipe, &mut io::sink()).unwrap_or(0);
        }
        let _ = tx.send((buf, dropped));
    });
    rx
}

/// Run `bin` with no shell and no stdin; fold stdout, stderr and the exit
/// status into one text block. Kills the process tree after `timeout`.
pub fn run_tool(bin: &Path, args: &[String], timeout: Duration) -> (String, bool) {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    own_process_group(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => return (format!("failed to run {}: {err}", bin.display()), true),
    };
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if start.elapsed() >= timeout => {
                terminate(&mut child);
                break Err(format!("[timeout after {}s]", timeout.as_secs()));
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(err) => {
                terminate(&mut child);
                break Err(format!("[wait failed: {err}]"));
            }
        }
    };

    // shortcut: a grandchild outside the process group can hold a pipe open; cap the wait.
    let collect = |rx: mpsc::Receiver<(Vec<u8>, u64)>| {
        let (bytes, dropped) = rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
        let mut text = strip_ansi(&String::from_utf8_lossy(&bytes));
        if dropped > 0 {
            text.push_str(&format!("\n[truncated {dropped} bytes]"));
        }
        text
    };
    let mut text = collect(stdout);
    let stderr = collect(stderr);
    if !stderr.trim().is_empty() {
        text.push_str("\n[stderr]\n");
        text.push_str(&stderr);
    }
    let failed = match status {
        Ok(status) if status.success() => false,
        Ok(status) => {
            let code = status
                .code()
                .map_or("signal".to_string(), |c| c.to_string());
            text.push_str(&format!("\n[exit {code}]"));
            true
        }
        Err(note) => {
            text.push('\n');
            text.push_str(&note);
            true
        }
    };
    (text, failed)
}

fn call(
    params: Option<&JsonValue>,
    resolve: &dyn Fn(&str) -> PathBuf,
) -> Result<JsonValue, String> {
    let name = params
        .and_then(|p| p.get("name"))
        .and_then(JsonValue::as_str)
        .ok_or("missing tool name")?;
    let tool = find_tool(name).ok_or_else(|| format!("unknown tool: {name}"))?;
    let args = match params
        .and_then(|p| p.get("arguments"))
        .and_then(|a| a.get("args"))
    {
        None => Vec::new(),
        Some(JsonValue::Array(items)) => items
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .ok_or("`args` must be an array of strings")?,
        Some(_) => return Err("`args` must be an array of strings".to_string()),
    };
    let (text, failed) = run_tool(&resolve(tool.name), &args, TOOL_TIMEOUT);
    Ok(obj(vec![
        (
            "content",
            JsonValue::Array(vec![obj(vec![("type", s("text")), ("text", s(&text))])]),
        ),
        ("isError", JsonValue::Bool(failed)),
    ]))
}

fn initialize(params: Option<&JsonValue>) -> JsonValue {
    let requested = params
        .and_then(|p| p.get("protocolVersion"))
        .and_then(JsonValue::as_str);
    let version = requested
        .filter(|v| PROTOCOL_VERSIONS.contains(v))
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    obj(vec![
        ("protocolVersion", s(version)),
        ("capabilities", obj(vec![("tools", obj(vec![]))])),
        (
            "serverInfo",
            obj(vec![
                ("name", s("belt")),
                ("version", s(env!("CARGO_PKG_VERSION"))),
            ]),
        ),
    ])
}

fn error(id: JsonValue, code: i64, message: &str) -> String {
    obj(vec![
        ("jsonrpc", s("2.0")),
        ("id", id),
        (
            "error",
            obj(vec![
                ("code", JsonValue::Number(code as f64)),
                ("message", s(message)),
            ]),
        ),
    ])
    .to_json_string()
}

/// Handle one JSON-RPC line. `None` for notifications, which get no reply.
pub fn handle(line: &str, resolve: &dyn Fn(&str) -> PathBuf) -> Option<String> {
    let msg = match parse(line) {
        Ok(msg @ JsonValue::Object(_)) => msg,
        Ok(_) => return Some(error(JsonValue::Null, -32600, "invalid request")),
        Err(err) => {
            return Some(error(
                JsonValue::Null,
                -32700,
                &format!("parse error: {err}"),
            ))
        }
    };
    let id = msg.get("id")?.clone();
    let params = msg.get("params");
    let result = match msg.get("method").and_then(JsonValue::as_str) {
        Some("initialize") => initialize(params),
        Some("ping") => obj(vec![]),
        Some("tools/list") => obj(vec![(
            "tools",
            JsonValue::Array(TOOLS.iter().map(tool_json).collect()),
        )]),
        Some("tools/call") => match call(params, resolve) {
            Ok(result) => result,
            Err(err) => return Some(error(id, -32602, &err)),
        },
        Some(method) => return Some(error(id, -32601, &format!("method not found: {method}"))),
        None => return Some(error(id, -32600, "invalid request")),
    };
    Some(obj(vec![("jsonrpc", s("2.0")), ("id", id), ("result", result)]).to_json_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(line: &str) -> JsonValue {
        parse(&handle(line, &|n| PathBuf::from(n)).expect("reply")).expect("json")
    }

    #[test]
    fn initialize_echoes_supported_version() {
        let r = reply(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#,
        );
        let result = r.get("result").unwrap();
        assert_eq!(
            result.get("protocolVersion").unwrap().as_str(),
            Some("2024-11-05")
        );
        assert!(result.get("capabilities").unwrap().get("tools").is_some());
    }

    #[test]
    fn initialize_falls_back_to_newest_version() {
        let r = reply(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
        );
        assert_eq!(
            r.get("result")
                .unwrap()
                .get("protocolVersion")
                .unwrap()
                .as_str(),
            Some(PROTOCOL_VERSIONS[0])
        );
    }

    #[test]
    fn notifications_get_no_reply() {
        assert!(handle(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            &|n| PathBuf::from(n)
        )
        .is_none());
    }

    #[test]
    fn tools_list_has_every_tool() {
        let r = reply(r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#);
        assert_eq!(r.get("id").unwrap().as_str(), Some("a"));
        let tools = r
            .get("result")
            .unwrap()
            .get("tools")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(tools.len(), TOOLS.len());
        let devclean = tools
            .iter()
            .find(|t| t.get("name").unwrap().as_str() == Some("devclean"))
            .unwrap();
        assert_eq!(
            devclean
                .get("annotations")
                .unwrap()
                .get("destructiveHint")
                .unwrap()
                .as_bool(),
            Some(true)
        );
    }

    #[test]
    fn errors_use_jsonrpc_codes() {
        let code = |line: &str| {
            reply(line)
                .get("error")
                .unwrap()
                .get("code")
                .unwrap()
                .as_i64()
        };
        assert_eq!(code("{nope"), Some(-32700));
        assert_eq!(code("[1]"), Some(-32600));
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":2,"method":"nope"}"#),
            Some(-32601)
        );
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"rm"}}"#),
            Some(-32602)
        );
        assert_eq!(
            code(
                r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"jwt","arguments":{"args":[1]}}}"#
            ),
            Some(-32602)
        );
    }

    #[cfg(unix)]
    #[test]
    fn tools_call_runs_binary_without_shell() {
        let line = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"jwt","arguments":{"args":["a b","$HOME;"]}}}"#;
        let r = parse(&handle(line, &|_| PathBuf::from("echo")).unwrap()).unwrap();
        let result = r.get("result").unwrap();
        assert_eq!(result.get("isError").unwrap().as_bool(), Some(false));
        let text = result.get("content").unwrap().as_array().unwrap()[0]
            .get("text")
            .unwrap()
            .as_str()
            .unwrap();
        assert_eq!(text, "a b $HOME;\n");
    }

    #[cfg(unix)]
    #[test]
    fn tools_call_flags_failure() {
        let (text, failed) = run_tool(Path::new("false"), &[], TOOL_TIMEOUT);
        assert!(failed);
        assert!(text.contains("[exit 1]"));
        let (text, failed) = run_tool(Path::new("/nonexistent/belt-tool"), &[], TOOL_TIMEOUT);
        assert!(failed);
        assert!(text.starts_with("failed to run"));
    }

    #[cfg(unix)]
    #[test]
    fn tools_call_kills_on_timeout() {
        let start = Instant::now();
        let (text, failed) = run_tool(
            Path::new("sleep"),
            &["30".to_string()],
            Duration::from_millis(200),
        );
        assert!(failed);
        assert!(text.contains("[timeout after 0s]"), "{text}");
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn tools_call_truncates_large_output() {
        let script = format!("head -c {} /dev/zero | tr '\\0' a", OUTPUT_LIMIT + 10);
        let (text, failed) = run_tool(Path::new("sh"), &["-c".into(), script], TOOL_TIMEOUT);
        assert!(!failed);
        assert!(
            text.ends_with("\n[truncated 10 bytes]"),
            "{}",
            &text[text.len() - 40..]
        );
        let body = text.split_once("\n[truncated").unwrap().0;
        assert_eq!(body.len() as u64, OUTPUT_LIMIT);
        assert!(body.bytes().all(|b| b == b'a'));
    }
}
