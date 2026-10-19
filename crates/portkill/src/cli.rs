//! CLI argument parsing for `portkill`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// List listening ports in tabular format.
    List,
    /// Terminate processes bound to the requested ports or process names.
    Kill,
    /// Print help.
    Help,
    /// Print version.
    Version,
}

use local_common::{split_flag, ArgCursor, CommonFlags};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub action: Action,
    pub ports: Vec<u16>,
    pub process_names: Vec<String>,
    pub force: bool,
    pub dry_run: bool,
    pub flags: CommonFlags,
}

impl Default for Parsed {
    fn default() -> Self {
        Self {
            action: Action::List,
            ports: Vec::new(),
            process_names: Vec::new(),
            force: false,
            dry_run: false,
            flags: CommonFlags::default(),
        }
    }
}

pub const VERSION: &str = "portkill 0.1.0";

pub const HELP: &str = r#"portkill [PORTS...] [OPTIONS] — inspect and kill processes on listening ports.

USAGE
    portkill                    # list all active listening ports
    portkill 3000 8080          # kill processes listening on ports 3000 and 8080
    portkill -f node            # kill processes by command name
    portkill -l [PORTS...]      # list ports (filtered if ports specified)

OPTIONS
    -k, --kill           explicitly kill matched processes
    -l, --list           list matched listening sockets instead of killing
    -f, --find <NAME>    match processes whose command contains NAME
    -9, --force          use SIGKILL instead of SIGTERM
        --dry-run        show what would be killed without sending signals
        --color          force colour output
        --no-color       disable colour output
    -h, --help           print this help
    -V, --version        print version
"#;

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Parsed, String> {
    let mut cursor = ArgCursor::new(args.into_iter());
    let mut p = Parsed::default();
    let mut explicit_list = false;
    let mut explicit_kill = false;

    while let Some(a) = cursor.next() {
        let (flag, inline) = split_flag(&a);
        match flag {
            "-h" | "--help" => p.action = Action::Help,
            "-V" | "--version" => p.action = Action::Version,
            "-l" | "--list" => explicit_list = true,
            "-k" | "--kill" => explicit_kill = true,
            "-9" | "--force" => p.force = true,
            "--dry-run" => p.dry_run = true,
            f if p.flags.check_arg(f) => {}
            "-f" | "--find" | "--name" => {
                let name = cursor.require_value("--find", inline)?;
                p.process_names.push(name);
            }
            s if s.starts_with('-') => return Err(format!("unknown flag: {s}")),
            s => {
                if let Ok(port) = s.parse::<u16>() {
                    if port > 0 {
                        p.ports.push(port);
                    } else {
                        return Err(format!("invalid port number: '{s}'"));
                    }
                } else {
                    return Err(format!("invalid port number: '{s}'"));
                }
            }
        }
    }

    if p.action == Action::Help || p.action == Action::Version {
        return Ok(p);
    }

    if explicit_list {
        p.action = Action::List;
    } else if explicit_kill || !p.ports.is_empty() || !p.process_names.is_empty() {
        p.action = Action::Kill;
    } else {
        p.action = Action::List;
    }

    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_default_is_list() {
        let p = parse(Vec::<String>::new()).unwrap();
        assert_eq!(p.action, Action::List);
        assert!(p.ports.is_empty());
    }

    #[test]
    fn parse_ports_triggers_kill() {
        let p = parse(vec![
            "3000".to_string(),
            "8080".to_string(),
            "-9".to_string(),
        ])
        .unwrap();
        assert_eq!(p.action, Action::Kill);
        assert_eq!(p.ports, vec![3000, 8080]);
        assert!(p.force);
    }

    #[test]
    fn parse_explicit_list_with_ports() {
        let p = parse(vec!["-l".to_string(), "3000".to_string()]).unwrap();
        assert_eq!(p.action, Action::List);
        assert_eq!(p.ports, vec![3000]);
    }

    #[test]
    fn parse_find_process_name() {
        let p = parse(vec!["-f".to_string(), "node".to_string()]).unwrap();
        assert_eq!(p.action, Action::Kill);
        assert_eq!(p.process_names, vec!["node"]);
    }

    #[test]
    fn parse_rejects_port_zero() {
        let err = parse(vec!["0".to_string()]).unwrap_err();
        assert!(err.contains("invalid port number: '0'"));
    }
}
