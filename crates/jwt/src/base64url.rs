//! Pure, dependency-free Base64 and Base64URL decoder (RFC 4648).

#[allow(unused_imports)]
pub use local_common::base64::{decode, decode_to_string, encode};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_standard_base64() {
        assert_eq!(decode_to_string("aGVsbG8=").unwrap(), "hello");
        assert_eq!(decode_to_string("aGVsbG8").unwrap(), "hello");
        assert_eq!(decode_to_string("d29ybGQ=").unwrap(), "world");
    }

    #[test]
    fn decode_base64url_with_special_chars() {
        // {"alg":"HS256","typ":"JWT"} -> eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9
        let header = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
        let decoded = decode_to_string(header).unwrap();
        assert_eq!(decoded, r#"{"alg":"HS256","typ":"JWT"}"#);

        // Test URL safe chars '-' and '_'
        assert!(decode("-_--").is_ok());
    }

    #[test]
    fn decode_invalid_characters() {
        assert!(decode("not!valid!").is_err());
    }
}
