//! Probing and parsing system sockets via `lsof`.
//!
//! Designed to run purely in userspace on macOS using `lsof -nP -iTCP -sTCP:LISTEN`.
//! The parser is completely pure and isolated from I/O so it is easily unit-tested.

use std::collections::HashSet;
use std::process::Command;

pub use crate::socket::ProcessSocket;

/// Query active listening TCP sockets from `lsof`.
pub fn query_sockets() -> std::io::Result<Vec<ProcessSocket>> {
    let output = Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN"])
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_lsof_output(&stdout))
}

/// Parse stdout from `lsof -nP -iTCP -sTCP:LISTEN`.
///
/// Output format:
/// `COMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME`
/// `node 12345 alan 23u IPv6 0x... 0t0 TCP *:3000 (LISTEN)`
pub fn parse_lsof_output(raw: &str) -> Vec<ProcessSocket> {
    let mut results = Vec::new();
    let mut seen = HashSet::new();

    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("COMMAND") {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        // Expected columns: COMMAND, PID, USER, FD, TYPE, DEVICE, SIZE/OFF, NODE, NAME...
        if parts.len() < 9 {
            continue;
        }

        let command = parts[0].to_string();
        let pid: u32 = match parts[1].parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let user = parts[2].to_string();
        let proto = parts[7].to_string();

        // The NAME column starts at index 8 and may contain " (LISTEN)"
        let name_parts = &parts[8..];
        let full_name = name_parts.join(" ");
        let clean_name = full_name.split('(').next().unwrap_or("").trim();

        let port = match extract_port(clean_name) {
            Some(p) => p,
            None => continue,
        };

        let state = if full_name.contains("LISTEN") {
            "LISTEN".to_string()
        } else {
            "-".to_string()
        };

        let entry = ProcessSocket {
            pid,
            command,
            user,
            proto,
            port,
            address: clean_name.to_string(),
            state,
        };

        // Deduplicate multiple identical bindings (e.g. IPv4 and IPv6 on the same port)
        let key = (entry.pid, entry.port, entry.proto.clone());
        if seen.insert(key) {
            results.push(entry);
        }
    }

    results.sort_by_key(|s| s.port);
    results
}

/// Extract port number from address string like `*:3000`, `127.0.0.1:8080`, `[::1]:5173`.
fn extract_port(address: &str) -> Option<u16> {
    let last_colon = address.rfind(':')?;
    let port_str = &address[last_colon + 1..];
    port_str.parse::<u16>().ok()
}

/// Terminate process by PID.
///
/// If `force` is true, sends `SIGKILL` (`kill -9`), otherwise `SIGTERM` (`kill -15`).
pub fn kill_process(pid: u32, force: bool) -> std::io::Result<()> {
    let signal = if force { "-9" } else { "-TERM" };
    let status = Command::new("kill")
        .args([signal, &pid.to_string()])
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "failed to kill process {pid}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_standard_lsof_output() {
        let sample = r#"
COMMAND     PID        USER   FD   TYPE             DEVICE SIZE/OFF NODE NAME
rapportd    636 alanrsoares   13u  IPv4 0x151c09881696d1de      0t0  TCP *:64463 (LISTEN)
rapportd    636 alanrsoares   15u  IPv6 0x610e8a17847fdfed      0t0  TCP *:64463 (LISTEN)
ollama     1692 alanrsoares    3u  IPv4 0x71d7f73bec3a7e54      0t0  TCP 127.0.0.1:11434 (LISTEN)
bun        2535 alanrsoares    8u  IPv6 0x7a0ed7c6a89e05aa      0t0  TCP *:50000 (LISTEN)
"#;

        let sockets = parse_lsof_output(sample);
        assert_eq!(sockets.len(), 3);

        assert_eq!(sockets[0].command, "ollama");
        assert_eq!(sockets[0].pid, 1692);
        assert_eq!(sockets[0].port, 11434);
        assert_eq!(sockets[0].address, "127.0.0.1:11434");

        assert_eq!(sockets[1].command, "bun");
        assert_eq!(sockets[1].port, 50000);

        assert_eq!(sockets[2].command, "rapportd");
        assert_eq!(sockets[2].port, 64463);
    }

    #[test]
    fn extract_port_handles_various_formats() {
        assert_eq!(extract_port("*:3000"), Some(3000));
        assert_eq!(extract_port("127.0.0.1:8080"), Some(8080));
        assert_eq!(extract_port("0.0.0.0:5173"), Some(5173));
        assert_eq!(extract_port("[::1]:9000"), Some(9000));
        assert_eq!(extract_port("invalid"), None);
    }
}
