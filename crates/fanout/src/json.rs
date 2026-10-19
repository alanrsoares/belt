//! Lightweight, zero-dependency JSON parser re-exported from `local_common`.

pub use local_common::json::{parse, JsonValue};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_literals() {
        assert_eq!(parse("null").unwrap(), JsonValue::Null);
        assert_eq!(parse("true").unwrap(), JsonValue::Bool(true));
        assert_eq!(parse("false").unwrap(), JsonValue::Bool(false));
        assert_eq!(parse("42").unwrap(), JsonValue::Number(42.0));
        assert_eq!(parse("-3.5").unwrap(), JsonValue::Number(-3.5));
        assert_eq!(
            parse("\"hello world\"").unwrap(),
            JsonValue::String("hello world".into())
        );
    }

    #[test]
    fn parses_escaped_strings() {
        let json = r#""line1\nline2\t\"quoted\"\\slash""#;
        assert_eq!(
            parse(json).unwrap(),
            JsonValue::String("line1\nline2\t\"quoted\"\\slash".into())
        );
    }

    #[test]
    fn parses_arrays_and_objects() {
        let json = r#"
        {
            "name": "renkonos",
            "private": true,
            "workspaces": {
                "packages": [
                    "apps/*",
                    "packages/*"
                ]
            },
            "scripts": {
                "lint": "biome check",
                "typecheck": "tsc"
            }
        }
        "#;
        let v = parse(json).unwrap();
        assert_eq!(v.get("name").and_then(|n| n.as_str()), Some("renkonos"));
        assert_eq!(v.get("private"), Some(&JsonValue::Bool(true)));

        let ws = v.get("workspaces").unwrap();
        let pkgs = ws.get("packages").and_then(|p| p.as_array()).unwrap();
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].as_str(), Some("apps/*"));
        assert_eq!(pkgs[1].as_str(), Some("packages/*"));

        let scripts = v.get("scripts").unwrap();
        assert_eq!(
            scripts.get("lint").and_then(|s| s.as_str()),
            Some("biome check")
        );
    }
}
