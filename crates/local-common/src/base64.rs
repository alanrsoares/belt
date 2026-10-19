//! Pure, dependency-free Base64 and Base64URL encoder and decoder (RFC 4648).

/// Decode a Base64 or Base64URL string (with or without `=` padding) into bytes.
pub fn decode(input: &str) -> Result<Vec<u8>, &'static str> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let mut buf = Vec::with_capacity(input.len());
    let mut seen_padding = false;
    for ch in input.chars() {
        if ch == '\r' || ch == '\n' {
            continue;
        }
        if ch == '=' {
            seen_padding = true;
            continue;
        }
        if seen_padding {
            return Err("invalid character after base64 padding");
        }
        match ch {
            'A'..='Z' => buf.push(ch as u8 - b'A'),
            'a'..='z' => buf.push(ch as u8 - b'a' + 26),
            '0'..='9' => buf.push(ch as u8 - b'0' + 52),
            '+' | '-' => buf.push(62),
            '/' | '_' => buf.push(63),
            _ => return Err("invalid character in base64 string"),
        }
    }

    let mut out = Vec::with_capacity((buf.len() * 3) / 4);
    let (chunks, remainder) = buf.as_chunks::<4>();

    for chunk in chunks {
        let b0 = chunk[0];
        let b1 = chunk[1];
        let b2 = chunk[2];
        let b3 = chunk[3];

        out.push((b0 << 2) | (b1 >> 4));
        out.push((b1 << 4) | (b2 >> 2));
        out.push((b2 << 6) | b3);
    }

    match remainder.len() {
        0 => {}
        1 => return Err("truncated base64 sequence"),
        2 => {
            let b0 = remainder[0];
            let b1 = remainder[1];
            out.push((b0 << 2) | (b1 >> 4));
        }
        3 => {
            let b0 = remainder[0];
            let b1 = remainder[1];
            let b2 = remainder[2];
            out.push((b0 << 2) | (b1 >> 4));
            out.push((b1 << 4) | (b2 >> 2));
        }
        _ => unreachable!(),
    }

    Ok(out)
}

/// Decode Base64/Base64URL into a UTF-8 String.
pub fn decode_to_string(input: &str) -> Result<String, String> {
    let bytes = decode(input).map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|e| format!("invalid UTF-8 sequence: {e}"))
}

/// Encode bytes into a Base64 string with standard `=` padding.
pub fn encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    if data.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    let (chunks, remainder) = data.as_chunks::<3>();

    for chunk in chunks {
        let b0 = chunk[0];
        let b1 = chunk[1];
        let b2 = chunk[2];

        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        out.push(TABLE[(b2 & 0x3f) as usize] as char);
    }

    match remainder.len() {
        0 => {}
        1 => {
            let b0 = remainder[0];
            out.push(TABLE[(b0 >> 2) as usize] as char);
            out.push(TABLE[((b0 & 0x03) << 4) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let b0 = remainder[0];
            let b1 = remainder[1];
            out.push(TABLE[(b0 >> 2) as usize] as char);
            out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
            out.push(TABLE[((b1 & 0x0f) << 2) as usize] as char);
            out.push('=');
        }
        _ => unreachable!(),
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let cases = [
            "",
            "f",
            "fo",
            "foo",
            "foob",
            "fooba",
            "foobar",
            "Hello, World! 🚀",
        ];
        for c in cases {
            let enc = encode(c.as_bytes());
            let dec = decode(&enc).expect("decode failed");
            assert_eq!(String::from_utf8(dec).unwrap(), c);
        }
    }

    #[test]
    fn decode_base64url_and_reject_mid_padding_or_spaces() {
        let header = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
        assert_eq!(
            decode_to_string(header).unwrap(),
            r#"{"alg":"HS256","typ":"JWT"}"#
        );
        assert!(decode("-_--").is_ok());
        assert!(decode("a=GVsbG8=").is_err());
        assert!(decode("Bearer eyJ").is_err());
        assert!(decode("a").is_err());
    }
}
