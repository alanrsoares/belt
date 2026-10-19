//! JWT token structure, timestamp humanization, and claim inspection.

use std::time::{SystemTime, UNIX_EPOCH};

use local_common::Colour;

use crate::base64url;
use crate::json;

/// A parsed JSON Web Token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JwtToken {
    pub raw_header: String,
    pub raw_payload: String,
    pub signature: String,
    pub header_json: String,
    pub payload_json: String,
}

impl JwtToken {
    /// Parse and decode a raw JWT string (`header.payload.signature`).
    pub fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        let raw = if raw.len() > 7 && raw[..7].eq_ignore_ascii_case("bearer ") {
            raw[7..].trim()
        } else {
            raw
        };
        let parts: Vec<&str> = raw.split('.').collect();
        if parts.len() != 3 {
            return Err(format!(
                "invalid JWT format: expected 3 dot-separated parts, found {}",
                parts.len()
            ));
        }

        let header_json = base64url::decode_to_string(parts[0])
            .map_err(|e| format!("failed to decode header: {e}"))?;
        let payload_json = base64url::decode_to_string(parts[1])
            .map_err(|e| format!("failed to decode payload: {e}"))?;

        Ok(Self {
            raw_header: parts[0].to_string(),
            raw_payload: parts[1].to_string(),
            signature: parts[2].to_string(),
            header_json,
            payload_json,
        })
    }

    /// Extract a named claim from the payload.
    pub fn get_claim(&self, key: &str) -> Option<String> {
        json::extract_field(&self.payload_json, key)
    }

    /// Extract a named field from the header.
    pub fn get_header_field(&self, key: &str) -> Option<String> {
        json::extract_field(&self.header_json, key)
    }

    /// Check expiration status and return human-readable summary.
    pub fn expiration_status(&self, now_secs: u64) -> Option<TokenStatus> {
        let parse_epoch = |s: String| {
            s.parse::<u64>()
                .ok()
                .or_else(|| s.parse::<f64>().ok().map(|f| f as u64))
        };

        let exp = self.get_claim("exp").and_then(parse_epoch);
        let nbf = self.get_claim("nbf").and_then(parse_epoch);

        if let Some(exp_secs) = exp {
            if now_secs > exp_secs {
                return Some(TokenStatus::Expired {
                    diff_secs: now_secs - exp_secs,
                    exp_epoch: exp_secs,
                });
            }
        }

        if let Some(nbf_secs) = nbf {
            if now_secs < nbf_secs {
                return Some(TokenStatus::NotYetValid {
                    diff_secs: nbf_secs - now_secs,
                    nbf_epoch: nbf_secs,
                });
            }
        }

        exp.map(|exp_secs| TokenStatus::Valid {
            diff_secs: exp_secs - now_secs,
            exp_epoch: exp_secs,
        })
    }

    /// Format full inspection view.
    pub fn format_inspection(&self, c: &Colour, now_secs: u64) -> String {
        let mut out = String::new();

        // 1. Status banner
        if let Some(status) = self.expiration_status(now_secs) {
            match status {
                TokenStatus::Valid {
                    diff_secs,
                    exp_epoch,
                } => {
                    out.push_str(&format!(
                        "{} {} (expires in {}, epoch {})\n\n",
                        c.green("●"),
                        c.green(c.bold("VALID")),
                        format_duration(diff_secs),
                        exp_epoch
                    ));
                }
                TokenStatus::Expired {
                    diff_secs,
                    exp_epoch,
                } => {
                    out.push_str(&format!(
                        "{} {} (expired {} ago, epoch {})\n\n",
                        c.red("●"),
                        c.red(c.bold("EXPIRED")),
                        format_duration(diff_secs),
                        exp_epoch
                    ));
                }
                TokenStatus::NotYetValid {
                    diff_secs,
                    nbf_epoch,
                } => {
                    out.push_str(&format!(
                        "{} {} (not active for another {}, epoch {})\n\n",
                        c.yellow("●"),
                        c.yellow(c.bold("NOT YET ACTIVE")),
                        format_duration(diff_secs),
                        nbf_epoch
                    ));
                }
            }
        }

        // 2. Header
        out.push_str(&format!("{}\n", c.bold(c.cyan("HEADER"))));
        out.push_str(&json::prettify(&self.header_json));
        out.push_str("\n\n");

        // 3. Payload
        out.push_str(&format!("{}\n", c.bold(c.green("PAYLOAD"))));
        out.push_str(&json::prettify(&self.payload_json));
        out.push_str("\n\n");

        // 4. Signature
        out.push_str(&format!("{}\n", c.bold("SIGNATURE")));
        out.push_str(&self.signature);
        out.push('\n');

        out
    }
}

/// Token validity status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenStatus {
    Valid { diff_secs: u64, exp_epoch: u64 },
    Expired { diff_secs: u64, exp_epoch: u64 },
    NotYetValid { diff_secs: u64, nbf_epoch: u64 },
}

/// Format seconds into a friendly human-readable duration (e.g. `2h 15m 30s`).
pub fn format_duration(seconds: u64) -> String {
    if seconds == 0 {
        return "0s".to_string();
    }

    let days = seconds / 86400;
    let hours = (seconds % 86400) / 3600;
    let mins = (seconds % 3600) / 60;
    let secs = seconds % 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if mins > 0 {
        parts.push(format!("{mins}m"));
    }
    if secs > 0 && days == 0 {
        parts.push(format!("{secs}s"));
    }

    parts.join(" ")
}

/// Return current time in unix epoch seconds.
pub fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_jwt() {
        // {"alg":"HS256","typ":"JWT"} . {"sub":"1234567890","name":"John Doe","iat":1516239022} . signature
        let token_str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let token = JwtToken::parse(token_str).unwrap();

        assert_eq!(token.get_header_field("alg"), Some("HS256".to_string()));
        assert_eq!(token.get_claim("sub"), Some("1234567890".to_string()));
        assert_eq!(token.get_claim("name"), Some("John Doe".to_string()));
    }

    #[test]
    fn parse_strips_bearer_prefix_case_insensitively() {
        let raw = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.sig";
        let t1 = JwtToken::parse(&format!("Bearer {raw}")).unwrap();
        let t2 = JwtToken::parse(&format!("  bearer   {raw}  ")).unwrap();
        assert_eq!(t1.get_claim("sub"), Some("1234567890".to_string()));
        assert_eq!(t2.get_claim("sub"), Some("1234567890".to_string()));
    }

    #[test]
    fn format_duration_samples() {
        assert_eq!(format_duration(45), "45s");
        assert_eq!(format_duration(125), "2m 5s");
        assert_eq!(format_duration(3665), "1h 1m 5s");
        assert_eq!(format_duration(90000), "1d 1h");
    }

    #[test]
    fn expiration_status_checks() {
        let token_str =
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJhbGFuIiwiZXhwIjoxMDAwfQ.sig";
        let token = JwtToken::parse(token_str).unwrap();

        // When now is 900 (before exp 1000) -> Valid
        let valid_status = token.expiration_status(900).unwrap();
        assert_eq!(
            valid_status,
            TokenStatus::Valid {
                diff_secs: 100,
                exp_epoch: 1000
            }
        );

        // When now is 1100 (after exp 1000) -> Expired
        let expired_status = token.expiration_status(1100).unwrap();
        assert_eq!(
            expired_status,
            TokenStatus::Expired {
                diff_secs: 100,
                exp_epoch: 1000
            }
        );
    }

    #[test]
    fn expiration_status_handles_nbf_only_and_exp_priority() {
        let header = base64url::encode(br#"{"alg":"HS256"}"#);
        let nbf_only_payload = base64url::encode(br#"{"sub":"alan","nbf":2000}"#);
        let token_nbf = JwtToken::parse(&format!("{header}.{nbf_only_payload}.sig")).unwrap();

        assert_eq!(
            token_nbf.expiration_status(1500),
            Some(TokenStatus::NotYetValid {
                diff_secs: 500,
                nbf_epoch: 2000
            })
        );
        assert_eq!(token_nbf.expiration_status(2500), None);

        // When both exp and nbf exist and token is already expired, exp check takes precedence
        let both_payload = base64url::encode(br#"{"exp":1000,"nbf":2000}"#);
        let token_both = JwtToken::parse(&format!("{header}.{both_payload}.sig")).unwrap();
        assert_eq!(
            token_both.expiration_status(1500),
            Some(TokenStatus::Expired {
                diff_secs: 500,
                exp_epoch: 1000
            })
        );
    }
}
