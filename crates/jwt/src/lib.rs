//! `jwt` — fast, zero-dependency JWT decoder and claim inspector.

mod base64url;
mod cli;
mod json;
mod token;

use std::io::{self, Read, Write};

use local_common::{is_terminal, Colour};

use cli::{Action, Parsed};
pub use token::{now_epoch_secs, JwtToken, TokenStatus};

/// Main runner invoked by `main.rs`.
pub fn run<I: IntoIterator<Item = String>>(args: I) -> i32 {
    let p = match cli::parse(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("jwt: {e}");
            eprintln!("run `jwt --help` for usage.");
            return 1;
        }
    };

    let mut out = io::stdout();
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
        Action::Inspect => run_inspect(&p, &c, &mut out),
    }
}

fn run_inspect(p: &Parsed, c: &Colour, out: &mut impl Write) -> i32 {
    let raw_token = match resolve_token(p.token.clone()) {
        Ok(t) => t,
        Err(e) => {
            let _ = writeln!(out, "{} {}", c.red("error:"), e);
            return 1;
        }
    };

    let token = match JwtToken::parse(&raw_token) {
        Ok(t) => t,
        Err(e) => {
            let _ = writeln!(out, "{} {}", c.red("error:"), e);
            return 1;
        }
    };

    // 1. Specific claims requested
    if !p.claims.is_empty() {
        for claim_name in &p.claims {
            let val = token
                .get_claim(claim_name)
                .or_else(|| token.get_header_field(claim_name))
                .unwrap_or_else(|| "<missing>".to_string());
            if p.claims.len() == 1 {
                let _ = writeln!(out, "{val}");
            } else {
                let _ = writeln!(out, "{}: {val}", c.bold(claim_name));
            }
        }
        return 0;
    }

    // 2. Header only
    if p.header_only {
        if p.raw_json {
            let _ = writeln!(out, "{}", token.header_json);
        } else {
            let _ = writeln!(out, "{}", json::prettify(&token.header_json));
        }
        return 0;
    }

    // 3. Payload only
    if p.payload_only {
        if p.raw_json {
            let _ = writeln!(out, "{}", token.payload_json);
        } else {
            let _ = writeln!(out, "{}", json::prettify(&token.payload_json));
        }
        return 0;
    }

    // 4. Signature only
    if p.signature_only {
        let _ = writeln!(out, "{}", token.signature);
        return 0;
    }

    // 5. Raw JSON full output
    if p.raw_json {
        let _ = writeln!(
            out,
            "{{\"header\": {}, \"payload\": {}, \"signature\": \"{}\"}}",
            token.header_json.trim(),
            token.payload_json.trim(),
            json::escape_str(&token.signature)
        );
        return 0;
    }

    // 6. Full inspection
    let now = now_epoch_secs();
    let inspection = token.format_inspection(c, now);
    let _ = write!(out, "{inspection}");
    0
}

/// Resolve token string from CLI argument or piped standard input.
fn resolve_token(token_opt: Option<String>) -> Result<String, String> {
    if let Some(t) = token_opt {
        let trimmed = t.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }

    let stdin = io::stdin();
    // Only attempt reading stdin if it is piped or redirected (not an interactive TTY).
    if !is_terminal(&stdin) {
        let mut stdin_buf = String::new();
        if stdin.lock().read_to_string(&mut stdin_buf).is_ok() {
            let trimmed = stdin_buf.trim().to_string();
            if !trimmed.is_empty() {
                return Ok(trimmed);
            }
        }
    }

    Err("no JWT provided via argument or stdin (try `jwt <TOKEN>` or `pbpaste | jwt`)".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_flag_without_header_or_payload_outputs_combined_json() {
        let token_str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIn0.sig123";
        let p = cli::parse(vec!["--json".to_string(), token_str.to_string()]).unwrap();
        let mut out = Vec::new();
        let code = run_inspect(&p, &Colour::new(false), &mut out);
        assert_eq!(code, 0);

        let text = String::from_utf8(out).unwrap();
        let parsed = json::parse(text.trim()).expect("output should be valid JSON");
        assert_eq!(
            parsed
                .get("header")
                .and_then(|h| h.get("alg"))
                .and_then(|v| v.as_str()),
            Some("HS256")
        );
        assert_eq!(
            parsed
                .get("payload")
                .and_then(|p| p.get("sub"))
                .and_then(|v| v.as_str()),
            Some("1234567890")
        );
        assert_eq!(
            parsed.get("signature").and_then(|v| v.as_str()),
            Some("sig123")
        );
    }
}
