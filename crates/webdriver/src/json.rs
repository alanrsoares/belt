//! Zero-dependency JSON parser and serializer for CDP messages.

pub use local_common::json::{escape_str, parse, JsonValue};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_surrogate_pairs() {
        let val = parse(r#"{"emoji":"\uD83D\uDCBE ok","bmp":"\u00e9"}"#).expect("parse failed");
        assert_eq!(
            val.get("emoji").and_then(|v| v.as_str()),
            Some("\u{1F4BE} ok")
        );
        assert_eq!(val.get("bmp").and_then(|v| v.as_str()), Some("\u{e9}"));
    }

    #[test]
    fn parse_complex_json() {
        let raw = r#"{
            "id": 1,
            "method": "Page.navigate",
            "params": {
                "url": "https://example.com",
                "timeout": 30000,
                "flags": ["fast", "silent"]
            },
            "enabled": true,
            "session": null
        }"#;

        let val = parse(raw).expect("parse failed");
        assert_eq!(val.get("id").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(
            val.get("method").and_then(|v| v.as_str()),
            Some("Page.navigate")
        );

        let params = val.get("params").unwrap();
        assert_eq!(
            params.get("url").and_then(|v| v.as_str()),
            Some("https://example.com")
        );
        assert_eq!(val.get("enabled").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(val.get("session"), Some(&JsonValue::Null));
    }
}
