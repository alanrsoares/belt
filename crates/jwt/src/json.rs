//! Pure JSON formatting and claim extraction without external dependencies.

#[allow(unused_imports)]
pub use local_common::json::{escape_str, extract_field, parse, prettify, JsonValue};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prettify_json_string() {
        let raw = r#"{"sub":"1234","name":"Alan","admin":true}"#;
        let formatted = prettify(raw);
        assert!(formatted.contains("  \"sub\": \"1234\","));
        assert!(formatted.contains("  \"admin\": true"));
    }

    #[test]
    fn extract_field_values() {
        let json = r#"{"sub": "alan", "exp": 1725283900, "verified": true}"#;
        assert_eq!(extract_field(json, "sub"), Some("alan".to_string()));
        assert_eq!(extract_field(json, "exp"), Some("1725283900".to_string()));
        assert_eq!(extract_field(json, "verified"), Some("true".to_string()));
        assert_eq!(extract_field(json, "missing"), None);
    }

    #[test]
    fn extract_field_nested_escaped_and_array_claims() {
        let json = r#"{"nested":{"sub":"inner"},"sub":"top","escaped":"hello \"world\"","roles":["admin","editor"]}"#;
        assert_eq!(extract_field(json, "sub"), Some("top".to_string()));
        assert_eq!(
            extract_field(json, "escaped"),
            Some("hello \"world\"".to_string())
        );
        assert_eq!(
            extract_field(json, "roles"),
            Some(r#"["admin","editor"]"#.to_string())
        );
        assert_eq!(
            extract_field(json, "nested"),
            Some(r#"{"sub":"inner"}"#.to_string())
        );
    }
}
