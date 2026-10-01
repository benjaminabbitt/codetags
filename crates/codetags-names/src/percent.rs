//! Percent-encoding shared by the path profile and item ids: `%XX` per
//! UTF-8 byte, upper-case hex on output, either case on input.

use std::fmt;

/// Why a percent-encoded string did not decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PercentError {
    /// A `%` at this byte offset is not followed by two hex digits.
    BadEscape(usize),
    /// The decoded bytes are not UTF-8.
    NotUtf8,
}

impl fmt::Display for PercentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PercentError::BadEscape(at) => write!(
                f,
                "the % at byte {at} is not followed by two hex digits (write a literal % as %25)"
            ),
            PercentError::NotUtf8 => f.write_str("the decoded bytes are not UTF-8"),
        }
    }
}

impl std::error::Error for PercentError {}

/// Appends `c` to `out` as `%XX` for each of its UTF-8 bytes.
pub(crate) fn push_encoded(out: &mut String, c: char) {
    let mut buf = [0u8; 4];
    for byte in c.encode_utf8(&mut buf).bytes() {
        out.push('%');
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0xF));
    }
}

fn hex_digit(nibble: u8) -> char {
    char::from(b"0123456789ABCDEF"[usize::from(nibble & 0xF)])
}

/// Decodes every `%XX` in `s`. Other characters pass through unchanged.
pub(crate) fn decode(s: &str) -> Result<String, PercentError> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()))
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .ok_or(PercentError::BadEscape(i))?;
            out.push(hex);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| PercentError::NotUtf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_utf8_bytes_upper_case() {
        let mut out = String::new();
        for c in [':', 'é', '\u{1}'] {
            push_encoded(&mut out, c);
        }
        assert_eq!(out, "%3A%C3%A9%01");
    }

    #[test]
    fn decodes_either_case() {
        assert_eq!(decode("a%3a%3Ab%C3%a9"), Ok("a::bé".into()));
    }

    #[test]
    fn rejects_bad_escapes() {
        assert_eq!(decode("50%"), Err(PercentError::BadEscape(2)));
        assert_eq!(decode("%4"), Err(PercentError::BadEscape(0)));
        assert_eq!(decode("%zz"), Err(PercentError::BadEscape(0)));
        assert_eq!(decode("%+1"), Err(PercentError::BadEscape(0)));
        assert_eq!(decode("%FF"), Err(PercentError::NotUtf8));
    }
}
