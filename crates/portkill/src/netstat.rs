//! Probing and parsing system sockets via Windows `netstat` and `tasklist`.

use std::collections::{HashMap, HashSet};
#[cfg(windows)]
use std::process::Command;

use crate::socket::{extract_port, ProcessSocket};

/// Query active listening TCP sockets on Windows via `netstat -ano -p tcp` and `tasklist`.
#[cfg(windows)]
pub fn query_sockets() -> std::io::Result<Vec<ProcessSocket>> {
    let output = Command::new("netstat")
        .args(["-ano", "-p", "tcp"])
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    let tasks = query_process_names().unwrap_or_default();
    Ok(parse_netstat_output(&stdout, &tasks))
}

#[cfg(windows)]
fn query_process_names() -> std::io::Result<HashMap<u32, String>> {
    let output = Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_tasklist_output(&stdout))
}

/// Terminate process by PID via Windows `taskkill`.
///
/// If `force` is true, passes `/F` to force termination.
#[cfg(windows)]
pub fn kill_process(pid: u32, force: bool) -> std::io::Result<()> {
    let mut cmd = Command::new("taskkill");
    if force {
        cmd.arg("/F");
    }
    let status = cmd.args(["/PID", &pid.to_string()]).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "failed to kill process {pid}"
        )))
    }
}

/// Parse stdout from `netstat -ano -p tcp`.
///
/// Format:
/// `  Proto  Local Address          Foreign Address        State           PID`
/// `  TCP    0.0.0.0:3000           0.0.0.0:0              LISTENING       14280`
pub fn parse_netstat_output(raw: &str, task_map: &HashMap<u32, String>) -> Vec<ProcessSocket> {
    let mut results = Vec::new();
    let mut seen = HashSet::new();

    for line in raw.lines() {
        let line = line.trim();
        if !line.starts_with("TCP") {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        // Columns: Proto, Local Address, Foreign Address, State, PID
        if parts.len() < 5 {
            continue;
        }

        let proto = parts[0].to_string();
        let local_addr = parts[1];
        let state = parts[3].to_string();
        if state != "LISTENING" {
            continue;
        }

        let pid: u32 = match parts[4].parse() {
            Ok(p) => p,
            Err(_) => continue,
        };

        let port = match extract_port(local_addr) {
            Some(p) => p,
            None => continue,
        };

        let command = task_map
            .get(&pid)
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());

        let entry = ProcessSocket {
            pid,
            command,
            user: "-".to_string(),
            proto,
            port,
            address: local_addr.to_string(),
            state,
        };

        let key = (entry.pid, entry.port, entry.proto.clone());
        if seen.insert(key) {
            results.push(entry);
        }
    }

    results.sort_by_key(|s| s.port);
    results
}

/// Parse CSV output from `tasklist /FO CSV /NH`.
///
/// Format:
/// `"node.exe","14280","Console","1","35,420 K"`
#[cfg_attr(not(windows), allow(dead_code))]
pub fn parse_tasklist_output(raw: &str) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split("\",\"").collect();
        if parts.len() >= 2 {
            let image_name = parts[0].trim_matches('"');
            let pid_str = parts[1].trim_matches('"');
            if let Ok(pid) = pid_str.parse::<u32>() {
                let clean_name = image_name.strip_suffix(".exe").unwrap_or(image_name);
                map.insert(pid, clean_name.to_string());
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tasklist_csv() {
        let sample = r#"
"System Idle Process","0","Services","0","8 K"
"System","4","Services","0","3,828 K"
"node.exe","14280","Console","1","35,420 K"
"bun.exe","9124","Console","1","24,100 K"
"#;
        let tasks = parse_tasklist_output(sample);
        assert_eq!(tasks.get(&14280).map(|s| s.as_str()), Some("node"));
        assert_eq!(tasks.get(&9124).map(|s| s.as_str()), Some("bun"));
        assert_eq!(tasks.get(&4).map(|s| s.as_str()), Some("System"));
    }

    #[test]
    fn parse_standard_netstat_output() {
        let netstat_sample = r#"
Active Connections

  Proto  Local Address          Foreign Address        State           PID
  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1044
  TCP    0.0.0.0:3000           0.0.0.0:0              LISTENING       14280
  TCP    127.0.0.1:5173         0.0.0.0:0              LISTENING       9124
  TCP    127.0.0.1:54321        127.0.0.1:54322        ESTABLISHED     5000
  TCP    [::]:135               [::]:0                 LISTENING       1044
  TCP    [::]:3000              [::]:0                 LISTENING       14280
"#;
        let mut tasks = HashMap::new();
        tasks.insert(14280, "node".to_string());
        tasks.insert(9124, "bun".to_string());

        let sockets = parse_netstat_output(netstat_sample, &tasks);
        assert_eq!(sockets.len(), 3); // 135, 3000 (deduped ipv4/ipv6), 5173

        assert_eq!(sockets[0].port, 135);
        assert_eq!(sockets[0].command, "unknown");

        assert_eq!(sockets[1].port, 3000);
        assert_eq!(sockets[1].pid, 14280);
        assert_eq!(sockets[1].command, "node");

        assert_eq!(sockets[2].port, 5173);
        assert_eq!(sockets[2].pid, 9124);
        assert_eq!(sockets[2].command, "bun");
    }
}
