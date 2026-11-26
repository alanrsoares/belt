//! CLI argument parsing for `jwt`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Inspect or extract claims from a token.
    Inspect,
    /// Print help.
    Help,
    /// Print version.
    Version,
}

use local_common::{split_flag, ArgCursor, CommonFlags};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub action: Action,
    pub token: Option<String>,
    pub header_only: bool,
    pub payload_only: bool,
    pub signature_only: bool,
    pub claims: Vec<String>,
    pub raw_json: bool,
    pub flags: CommonFlags,
}

impl Default for Parsed {
    fn default() -> Self {
        Self {
            action: Action::Inspect,
            token: None,
            header_only: false,
            payload_only: false,
            signature_only: false,
            claims: Vec::new(),
            raw_json: false,
            flags: CommonFlags::default(),
        }
    }
}

pub const VERSION: &str = concat!("jwt ", env!("CARGO_PKG_VERSION"));

pub const HELP: &str = r#"jwt [TOKEN] [OPTIONS] — inspect and decode JSON Web Tokens.

USAGE
    jwt eyJhbGci...               # decode and display token info
    pbpaste | jwt                 # read from clipboard via stdin (macOS)
    xclip -o -sel clip | jwt      # ...or wl-paste | jwt (Linux)
    jwt -p <TOKEN>                # output payload JSON only
    jwt -c exp,sub,roles <TOKEN>  # extract specific claim values

OPTIONS
    -p, --payload         print payload JSON only
    -H, --header          print header JSON only
    -s, --signature       print signature only
    -c, --claims <KEYS>   comma-separated list of claim names to extract
    -j, --json            raw unformatted JSON (ideal for piping into jq/jless)
        --color           force colour output
        --no-color        disable colour output
    -h, --help            print this help
    -V, --version         print version
"#;

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Parsed, String> {
    let mut cursor = ArgCursor::new(args.into_iter());
    let mut p = Parsed::default();

    while let Some(a) = cursor.next() {
        let (flag, inline) = split_flag(&a);
        match flag {
            "-h" | "--help" => p.action = Action::Help,
            "-V" | "--version" => p.action = Action::Version,
            "-p" | "--payload" => p.payload_only = true,
            "-H" | "--header" => p.header_only = true,
            "-s" | "--signature" => p.signature_only = true,
            "-j" | "--json" => p.raw_json = true,
            f if p.flags.check_arg(f) => {}
            "-c" | "--claim" | "--claims" => {
                let val = cursor.require_value("--claims", inline)?;
                for claim in val.split(',') {
                    let trimmed = claim.trim();
                    if !trimmed.is_empty() {
                        p.claims.push(trimmed.to_string());
                    }
                }
            }
            s if s.starts_with('-') => return Err(format!("unknown flag: {s}")),
            s => {
                if p.token.is_some() {
                    return Err(format!("unexpected argument: {s}"));
                }
                p.token = Some(s.to_string());
            }
        }
    }

    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_token_and_flags() {
        let p = parse(vec![
            "-p".to_string(),
            "-c".to_string(),
            "exp,sub".to_string(),
            "my.token.sig".to_string(),
        ])
        .unwrap();

        assert_eq!(p.action, Action::Inspect);
        assert_eq!(p.token.as_deref(), Some("my.token.sig"));
        assert!(p.payload_only);
        assert_eq!(p.claims, vec!["exp", "sub"]);
    }
}
