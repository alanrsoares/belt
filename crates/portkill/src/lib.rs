//! `portkill` — fast, zero-dependency port inspector and process killer.

mod cli;
mod lsof;
mod netstat;
mod socket;

use std::collections::HashSet;
use std::io::Write;

use local_common::Colour;

use cli::{Action, Parsed};
pub use lsof::parse_lsof_output;
pub use netstat::parse_netstat_output;
pub use socket::ProcessSocket;

#[cfg(not(windows))]
pub use lsof::{kill_process, query_sockets};

#[cfg(windows)]
pub use netstat::{kill_process, query_sockets};

/// Main runner invoked by `main.rs`.
pub fn run<I: IntoIterator<Item = String>>(args: I) -> i32 {
    let p = match cli::parse(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("portkill: {e}");
            eprintln!("run `portkill --help` for usage.");
            return 1;
        }
    };

    let mut out = std::io::stdout();
    let c = p.flags.resolve_colour(&out);

    match p.action {
        Action::Help => {
            print!("{}", cli::HELP);
            0
        }
        Action::Version => {
            print!("{}", cli::VERSION);
            0
        }
        Action::List => run_list(&p, &c, &mut out),
        Action::Kill => run_kill(&p, &c, &mut out),
    }
}

fn run_list(p: &Parsed, c: &Colour, out: &mut impl Write) -> i32 {
    let sockets = match query_sockets() {
        Ok(s) => s,
        Err(e) => {
            let _ = writeln!(
                out,
                "{} failed to probe listening sockets: {e}",
                c.red("error:")
            );
            return 1;
        }
    };

    let matched = filter_sockets(&sockets, &p.ports, &p.process_names);
    if matched.is_empty() {
        if p.ports.is_empty() && p.process_names.is_empty() {
            let _ = writeln!(out, "No listening TCP sockets found.");
        } else {
            let _ = writeln!(out, "No matching listening sockets found.");
        }
        return 0;
    }

    print_socket_table(&matched, c, out);
    0
}

fn run_kill(p: &Parsed, c: &Colour, out: &mut impl Write) -> i32 {
    let sockets = match query_sockets() {
        Ok(s) => s,
        Err(e) => {
            let _ = writeln!(out, "{} failed to query sockets: {e}", c.red("error:"));
            return 1;
        }
    };

    let matched = filter_sockets(&sockets, &p.ports, &p.process_names);
    if matched.is_empty() {
        let _ = writeln!(out, "No matching processes found to kill.");
        return 0;
    }

    let mut killed_pids = HashSet::new();
    #[cfg(not(windows))]
    let signal_name = if p.force {
        "SIGKILL (-9)"
    } else {
        "SIGTERM (-15)"
    };
    #[cfg(windows)]
    let signal_name = if p.force { "taskkill /F" } else { "taskkill" };

    let mut had_failures = false;
    for sock in matched {
        if !killed_pids.insert(sock.pid) {
            continue;
        }

        if p.dry_run {
            let _ = writeln!(
                out,
                "{} would kill {} (PID {}) listening on port {} with {}",
                c.cyan("dry-run:"),
                c.bold(&sock.command),
                sock.pid,
                c.cyan(sock.port.to_string()),
                signal_name
            );
        } else {
            match kill_process(sock.pid, p.force) {
                Ok(_) => {
                    let _ = writeln!(
                        out,
                        "{} killed {} (PID {}) on port {} via {}",
                        c.green("✓"),
                        c.bold(&sock.command),
                        sock.pid,
                        c.cyan(sock.port.to_string()),
                        signal_name
                    );
                }
                Err(e) => {
                    had_failures = true;
                    let _ = writeln!(
                        out,
                        "{} failed to kill {} (PID {}): {e}",
                        c.red("✗"),
                        sock.command,
                        sock.pid
                    );
                }
            }
        }
    }

    kill_exit_code(had_failures)
}

fn kill_exit_code(had_failures: bool) -> i32 {
    if had_failures {
        1
    } else {
        0
    }
}

/// Filter socket list by requested ports and/or process names.
pub fn filter_sockets<'a>(
    sockets: &'a [ProcessSocket],
    ports: &[u16],
    process_names: &[String],
) -> Vec<&'a ProcessSocket> {
    sockets
        .iter()
        .filter(|s| {
            let port_match = ports.is_empty() || ports.contains(&s.port);
            let name_match = process_names.is_empty()
                || process_names
                    .iter()
                    .any(|n| s.command.to_lowercase().contains(&n.to_lowercase()));
            port_match && name_match
        })
        .collect()
}

/// Render a formatted table of active sockets.
pub fn print_socket_table(sockets: &[&ProcessSocket], c: &Colour, out: &mut impl Write) {
    let _ = writeln!(
        out,
        "{:<8} {:<8} {:<16} {:<8} {:<12} {}",
        c.bold("PORT"),
        c.bold("PID"),
        c.bold("COMMAND"),
        c.bold("PROTO"),
        c.bold("USER"),
        c.bold("ADDRESS")
    );

    for s in sockets {
        let _ = writeln!(
            out,
            "{:<8} {:<8} {:<16} {:<8} {:<12} {}",
            c.cyan(s.port.to_string()),
            s.pid,
            c.bold(&s.command),
            s.proto,
            s.user,
            s.address
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_sockets_by_port_and_name() {
        let sample = vec![
            ProcessSocket {
                pid: 100,
                command: "node".into(),
                user: "alan".into(),
                proto: "TCP".into(),
                port: 3000,
                address: "*:3000".into(),
                state: "LISTEN".into(),
            },
            ProcessSocket {
                pid: 200,
                command: "python3".into(),
                user: "alan".into(),
                proto: "TCP".into(),
                port: 8000,
                address: "127.0.0.1:8000".into(),
                state: "LISTEN".into(),
            },
        ];

        let matched_3000 = filter_sockets(&sample, &[3000], &[]);
        assert_eq!(matched_3000.len(), 1);
        assert_eq!(matched_3000[0].command, "node");

        let matched_py = filter_sockets(&sample, &[], &["python".to_string()]);
        assert_eq!(matched_py.len(), 1);
        assert_eq!(matched_py[0].port, 8000);
    }

    #[test]
    fn kill_exit_code_reflects_signal_failures() {
        assert_eq!(kill_exit_code(false), 0);
        assert_eq!(kill_exit_code(true), 1);
    }
}
