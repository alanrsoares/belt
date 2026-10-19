//! Shared zero-dependency JSON parser, serializer, formatter, and field extractor.

use std::collections::HashMap;

/// A JSON value representation.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(HashMap<String, JsonValue>),
}

impl JsonValue {
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            Self::Object(map) => map.get(key),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Number(n) => Some(*n as i64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            Self::Array(arr) => Some(arr.as_slice()),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&HashMap<String, JsonValue>> {
        match self {
            Self::Object(map) => Some(map),
            _ => None,
        }
    }

    pub fn to_json_string(&self) -> String {
        match self {
            Self::Null => "null".to_string(),
            Self::Bool(b) => b.to_string(),
            Self::Number(n) => {
                if !n.is_finite() {
                    "null".to_string()
                } else if n.fract() == 0.0 {
                    format!("{:.0}", n)
                } else {
                    format!("{}", n)
                }
            }
            Self::String(s) => format!("\"{}\"", escape_str(s)),
            Self::Array(items) => {
                let inner: Vec<String> = items.iter().map(|v| v.to_json_string()).collect();
                format!("[{}]", inner.join(","))
            }
            Self::Object(map) => {
                let mut pairs: Vec<(&String, &JsonValue)> = map.iter().collect();
                pairs.sort_by(|a, b| a.0.cmp(b.0));
                let inner: Vec<String> = pairs
                    .into_iter()
                    .map(|(k, v)| format!("\"{}\":{}", escape_str(k), v.to_json_string()))
                    .collect();
                format!("{{{}}}", inner.join(","))
            }
        }
    }
}

/// Escape a string for inclusion inside JSON quotes.
pub fn escape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0C' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Prettify a JSON string with 2-space indentation, keeping empty `{}` and `[]` compact.
pub fn prettify(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len() * 2);
    let mut indent: usize = 0;
    let mut in_string = false;
    let mut escape = false;
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        i += 1;

        if escape {
            out.push(ch);
            escape = false;
            continue;
        }

        if ch == '\\' && in_string {
            out.push(ch);
            escape = true;
            continue;
        }

        if ch == '"' {
            in_string = !in_string;
            out.push(ch);
            continue;
        }

        if in_string {
            out.push(ch);
            continue;
        }

        match ch {
            '{' | '[' => {
                let close = if ch == '{' { '}' } else { ']' };
                let mut peek = i;
                while peek < chars.len() && chars[peek].is_whitespace() {
                    peek += 1;
                }
                if peek < chars.len() && chars[peek] == close {
                    out.push(ch);
                    out.push(close);
                    i = peek + 1;
                } else {
                    out.push(ch);
                    indent += 2;
                    out.push('\n');
                    out.push_str(&" ".repeat(indent));
                }
            }
            '}' | ']' => {
                indent = indent.saturating_sub(2);
                out.push('\n');
                out.push_str(&" ".repeat(indent));
                out.push(ch);
            }
            ',' => {
                out.push(ch);
                out.push('\n');
                out.push_str(&" ".repeat(indent));
            }
            ':' => {
                out.push(':');
                out.push(' ');
            }
            ' ' | '\t' | '\r' | '\n' => {}
            _ => out.push(ch),
        }
    }

    out
}

/// Extract a top-level key's value from a JSON object.
/// String values are returned unquoted; non-string values are returned as compact JSON.
pub fn extract_field(json: &str, key: &str) -> Option<String> {
    let val = parse(json).ok()?;
    let field = val.get(key)?;
    match field {
        JsonValue::String(s) => Some(s.clone()),
        other => Some(other.to_json_string()),
    }
}

/// Parse a JSON string into a [`JsonValue`].
pub fn parse(input: &str) -> Result<JsonValue, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut idx = 0;
    skip_whitespace(&chars, &mut idx);
    let val = parse_value(&chars, &mut idx)?;
    skip_whitespace(&chars, &mut idx);
    if idx < chars.len() {
        return Err(format!(
            "trailing data after JSON root at character index {idx}"
        ));
    }
    Ok(val)
}

fn skip_whitespace(chars: &[char], idx: &mut usize) {
    while *idx < chars.len() && chars[*idx].is_whitespace() {
        *idx += 1;
    }
}

fn parse_value(chars: &[char], idx: &mut usize) -> Result<JsonValue, String> {
    skip_whitespace(chars, idx);
    if *idx >= chars.len() {
        return Err("unexpected end of json input".to_string());
    }

    match chars[*idx] {
        'n' => parse_null(chars, idx),
        't' | 'f' => parse_bool(chars, idx),
        '"' => parse_string(chars, idx).map(JsonValue::String),
        '[' => parse_array(chars, idx).map(JsonValue::Array),
        '{' => parse_object(chars, idx).map(JsonValue::Object),
        '-' | '0'..='9' => parse_number(chars, idx).map(JsonValue::Number),
        c => Err(format!("unexpected character '{c}' at index {idx}")),
    }
}

fn parse_null(chars: &[char], idx: &mut usize) -> Result<JsonValue, String> {
    if chars.get(*idx..*idx + 4) == Some(&['n', 'u', 'l', 'l'])
        && !chars
            .get(*idx + 4)
            .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        *idx += 4;
        Ok(JsonValue::Null)
    } else {
        Err(format!("invalid null literal at index {idx}"))
    }
}

fn parse_bool(chars: &[char], idx: &mut usize) -> Result<JsonValue, String> {
    if chars.get(*idx..*idx + 4) == Some(&['t', 'r', 'u', 'e'])
        && !chars
            .get(*idx + 4)
            .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        *idx += 4;
        Ok(JsonValue::Bool(true))
    } else if chars.get(*idx..*idx + 5) == Some(&['f', 'a', 'l', 's', 'e'])
        && !chars
            .get(*idx + 5)
            .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        *idx += 5;
        Ok(JsonValue::Bool(false))
    } else {
        Err(format!("invalid boolean literal at index {idx}"))
    }
}

fn parse_string(chars: &[char], idx: &mut usize) -> Result<String, String> {
    if *idx >= chars.len() || chars[*idx] != '"' {
        return Err(format!("expected '\"' at index {idx}"));
    }
    *idx += 1;

    let mut out = String::new();
    while *idx < chars.len() {
        let ch = chars[*idx];
        *idx += 1;
        match ch {
            '"' => return Ok(out),
            '\\' => {
                if *idx >= chars.len() {
                    return Err("unterminated escape sequence in string".to_string());
                }
                let esc = chars[*idx];
                *idx += 1;
                match esc {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'b' => out.push('\x08'),
                    'f' => out.push('\x0C'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'u' => {
                        if *idx + 4 > chars.len() {
                            return Err("truncated unicode escape".to_string());
                        }
                        let hex_str: String = chars[*idx..*idx + 4].iter().collect();
                        *idx += 4;
                        let mut code = u32::from_str_radix(&hex_str, 16).map_err(|e| {
                            format!("invalid hex in unicode escape '\\u{hex_str}': {e}")
                        })?;

                        // Non-BMP characters arrive as a UTF-16 surrogate pair
                        // (\uD800..\uDBFF followed by \uDC00..\uDFFF). Peek ahead
                        // without consuming unrelated escapes if unpaired.
                        if (0xD800..0xDC00).contains(&code)
                            && chars.get(*idx..*idx + 2) == Some(&['\\', 'u'])
                            && *idx + 6 <= chars.len()
                        {
                            let low_hex: String = chars[*idx + 2..*idx + 6].iter().collect();
                            if let Ok(low) = u32::from_str_radix(&low_hex, 16) {
                                if (0xDC00..0xE000).contains(&low) {
                                    *idx += 6;
                                    code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                }
                            }
                        }

                        out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                    }
                    other => {
                        out.push(other);
                    }
                }
            }
            other => out.push(other),
        }
    }
    Err("unterminated string literal".to_string())
}

fn parse_number(chars: &[char], idx: &mut usize) -> Result<f64, String> {
    let start = *idx;
    if chars[*idx] == '-' {
        *idx += 1;
    }
    while *idx < chars.len() && chars[*idx].is_ascii_digit() {
        *idx += 1;
    }
    if *idx < chars.len() && chars[*idx] == '.' {
        *idx += 1;
        while *idx < chars.len() && chars[*idx].is_ascii_digit() {
            *idx += 1;
        }
    }
    if *idx < chars.len() && (chars[*idx] == 'e' || chars[*idx] == 'E') {
        *idx += 1;
        if *idx < chars.len() && (chars[*idx] == '+' || chars[*idx] == '-') {
            *idx += 1;
        }
        while *idx < chars.len() && chars[*idx].is_ascii_digit() {
            *idx += 1;
        }
    }

    let num_str: String = chars[start..*idx].iter().collect();
    num_str
        .parse::<f64>()
        .map_err(|e| format!("invalid number '{num_str}': {e}"))
}

fn parse_array(chars: &[char], idx: &mut usize) -> Result<Vec<JsonValue>, String> {
    if *idx >= chars.len() || chars[*idx] != '[' {
        return Err(format!("expected '[' at index {idx}"));
    }
    *idx += 1;

    let mut items = Vec::new();
    skip_whitespace(chars, idx);
    if *idx < chars.len() && chars[*idx] == ']' {
        *idx += 1;
        return Ok(items);
    }

    loop {
        skip_whitespace(chars, idx);
        let val = parse_value(chars, idx)?;
        items.push(val);
        skip_whitespace(chars, idx);

        if *idx >= chars.len() {
            return Err("unterminated array".to_string());
        }

        match chars[*idx] {
            ',' => {
                *idx += 1;
                skip_whitespace(chars, idx);
                if *idx < chars.len() && chars[*idx] == ']' {
                    *idx += 1;
                    return Ok(items);
                }
            }
            ']' => {
                *idx += 1;
                return Ok(items);
            }
            c => {
                return Err(format!(
                    "expected ',' or ']' in array, found '{c}' at index {idx}"
                ))
            }
        }
    }
}

fn parse_object(chars: &[char], idx: &mut usize) -> Result<HashMap<String, JsonValue>, String> {
    if *idx >= chars.len() || chars[*idx] != '{' {
        return Err(format!("expected '{{' at index {idx}"));
    }
    *idx += 1;

    let mut map = HashMap::new();
    skip_whitespace(chars, idx);
    if *idx < chars.len() && chars[*idx] == '}' {
        *idx += 1;
        return Ok(map);
    }

    loop {
        skip_whitespace(chars, idx);
        if *idx >= chars.len() {
            return Err("unterminated object".to_string());
        }

        let key = parse_string(chars, idx)?;
        skip_whitespace(chars, idx);

        if *idx >= chars.len() || chars[*idx] != ':' {
            return Err(format!("expected ':' after key in object at index {idx}"));
        }
        *idx += 1;

        let val = parse_value(chars, idx)?;
        map.insert(key, val);
        skip_whitespace(chars, idx);

        if *idx >= chars.len() {
            return Err("unterminated object".to_string());
        }

        match chars[*idx] {
            ',' => {
                *idx += 1;
                skip_whitespace(chars, idx);
                if *idx < chars.len() && chars[*idx] == '}' {
                    *idx += 1;
                    return Ok(map);
                }
            }
            '}' => {
                *idx += 1;
                return Ok(map);
            }
            c => {
                return Err(format!(
                    "expected ',' or '}}' in object, found '{c}' at index {idx}"
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_surrogate_pairs_and_unpaired_surrogates() {
        let val = parse(r#"{"emoji":"\uD83D\uDCBE ok","bmp":"\u00e9","unpaired":"\uD800\"quote"}"#)
            .expect("parse failed");
        assert_eq!(
            val.get("emoji").and_then(|v| v.as_str()),
            Some("\u{1F4BE} ok")
        );
        assert_eq!(val.get("bmp").and_then(|v| v.as_str()), Some("\u{e9}"));
        assert_eq!(
            val.get("unpaired").and_then(|v| v.as_str()),
            Some("\u{FFFD}\"quote")
        );
    }

    #[test]
    fn extract_field_handles_nested_and_complex_values() {
        let json = r#"{"other":"sub","nested":{"sub":"wrong"},"sub":"real","roles":["admin","user"],"quote":"a \"b\" c"}"#;
        assert_eq!(extract_field(json, "sub"), Some("real".to_string()));
        assert_eq!(
            extract_field(json, "roles"),
            Some(r#"["admin","user"]"#.to_string())
        );
        assert_eq!(extract_field(json, "quote"), Some("a \"b\" c".to_string()));
        assert_eq!(extract_field(json, "missing"), None);
    }

    #[test]
    fn prettify_keeps_empty_containers_compact() {
        assert_eq!(
            prettify(r#"{"a":[],"b":{}}"#),
            "{\n  \"a\": [],\n  \"b\": {}\n}"
        );
    }
}
