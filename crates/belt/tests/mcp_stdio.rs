use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn mcp_stdio_round_trip() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_belt"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn belt mcp");
    let input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "\n\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        "\n",
    );
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let lines: Vec<&str> = std::str::from_utf8(&output.stdout)
        .unwrap()
        .lines()
        .collect();
    assert_eq!(
        lines.len(),
        2,
        "one reply per request, none for notifications"
    );
    assert!(lines[0].contains(r#""protocolVersion":"2025-06-18""#));
    assert!(lines[1].contains(r#""name":"webdriver""#));
}
