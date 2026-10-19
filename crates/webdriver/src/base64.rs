//! Pure, dependency-free Base64 encoder and decoder (RFC 4648).

pub use local_common::base64::{decode, encode};

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
}
