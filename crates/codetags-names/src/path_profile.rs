//! The path profile (PLAN.md §2.7, C6): one `q/` path component spells
//! exactly one tagma postfix element.
//!
//! **Decoding** percent-decodes the component (`%XX` per UTF-8 byte). On
//! Linux and macOS a component may also hold raw tagma syntax
//! (`q/prov:source=scip`); on Windows a component that Windows could not
//! have stored as a file name is refused, so a name never decodes on one
//! OS and not on another by accident of the host.
//!
//! **Encoding** writes the one portable spelling: it encodes exactly what
//! some supported OS forbids, so the result is a legal file name on every
//! platform and the same spelling works everywhere.
//!
//! - `%` is always `%25`, and `/` is always `%2F`.
//! - Windows's reserved characters `< > : " \ | ? *` and U+0000 to U+001F
//!   are encoded. (PLAN.md §2.7 lists the tagma characters among them;
//!   `?` is added here because Windows forbids it too, V34.)
//! - A trailing `.` or space is encoded, which also covers `.` and `..`.
//! - A Windows device name (`con`, `NUL.txt`, `com1`, …) has its first
//!   character encoded: `%63on`.
//!
//! Everything else, including non-ASCII text, is written raw. The rules
//! for device names and trailing characters are the documented Windows
//! rules; spike S4 checks them against WinFsp.

use std::fmt;

use crate::percent::{self, PercentError};
use crate::platform::{
    MAX_NAME_BYTES, Platform, is_legal_filename, is_windows_device_name, is_windows_reserved_char,
};

/// Why an element could not be encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    /// The element is empty; a path component cannot be.
    Empty,
    /// The encoded component would exceed [`MAX_NAME_BYTES`].
    TooLong(usize),
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EncodeError::Empty => f.write_str("an empty query element has no path component"),
            EncodeError::TooLong(len) => write!(
                f,
                "the encoded component is {len} bytes; a file name holds at most {MAX_NAME_BYTES}"
            ),
        }
    }
}

impl std::error::Error for EncodeError {}

/// Why a path component did not decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The component is not a legal file name on the platform, so it can
    /// only have been written raw on another OS.
    NotAFileName(Platform),
    /// The component's percent-encoding is malformed.
    Percent(PercentError),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::NotAFileName(platform) => write!(
                f,
                "not a legal file name on {platform}; encode reserved characters as %XX"
            ),
            DecodeError::Percent(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Returns `true` for a character the portable encoding always escapes.
fn must_encode(c: char) -> bool {
    c == '%' || is_windows_reserved_char(c)
}

/// Encodes one tagma postfix element as one path component, portable to
/// every [`Platform`].
///
/// # Errors
///
/// [`EncodeError::Empty`] for an empty element, and
/// [`EncodeError::TooLong`] if the result would not fit in one file name.
pub fn encode_component(element: &str) -> Result<String, EncodeError> {
    if element.is_empty() {
        return Err(EncodeError::Empty);
    }
    let device = is_windows_device_name(element);
    let last = element.char_indices().next_back().map_or(0, |(i, _)| i);
    let mut out = String::with_capacity(element.len());
    for (i, c) in element.char_indices() {
        let encode = must_encode(c) || (i == 0 && device) || (i == last && matches!(c, '.' | ' '));
        if encode {
            percent::push_encoded(&mut out, c);
        } else {
            out.push(c);
        }
    }
    if out.len() > MAX_NAME_BYTES {
        return Err(EncodeError::TooLong(out.len()));
    }
    Ok(out)
}

/// Decodes one path component, as found on `platform`, into the tagma
/// postfix element it spells.
///
/// Parsing the element is the caller's job (the tagma bridge, P2.4).
///
/// # Errors
///
/// [`DecodeError::NotAFileName`] if `platform` could not hold the
/// component as a name, and [`DecodeError::Percent`] for a malformed
/// escape or a non-UTF-8 result.
pub fn decode_component(component: &str, platform: Platform) -> Result<String, DecodeError> {
    if !is_legal_filename(component, platform) {
        return Err(DecodeError::NotAFileName(platform));
    }
    percent::decode(component).map_err(DecodeError::Percent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn encodes_the_minimum() {
        for (element, component) in [
            ("kind=function", "kind=function"),
            ("prov:source=scip", "prov%3Asource=scip"),
            ("a/b%c", "a%2Fb%25c"),
            ("con", "%63on"),
            ("Nul.txt", "%4Eul.txt"),
            ("é", "é"),
            ("a.", "a%2E"),
            ("a ", "a%20"),
            (".", "%2E"),
            ("..", ".%2E"),
            ("a\tb", "a%09b"),
        ] {
            assert_eq!(encode_component(element).as_deref(), Ok(component));
        }
    }

    #[test]
    fn refuses_what_no_file_name_can_hold() {
        assert_eq!(encode_component(""), Err(EncodeError::Empty));
        assert_eq!(
            encode_component(&":".repeat(100)),
            Err(EncodeError::TooLong(300))
        );
    }

    #[test]
    fn raw_syntax_decodes_on_unix_only() {
        for p in [Platform::Linux, Platform::MacOs] {
            assert_eq!(decode_component("a:b", p).as_deref(), Ok("a:b"));
        }
        assert_eq!(
            decode_component("a:b", Platform::Windows),
            Err(DecodeError::NotAFileName(Platform::Windows))
        );
    }

    fn element() -> impl Strategy<Value = String> {
        prop_oneof![
            // tagma-ish elements, heavy on reserved characters
            "[a-z:=<>~!*()\"%/\\\\|?. -]{1,24}",
            // device names with and without extensions and odd case
            "(?i)(con|prn|aux|nul|com[1-9¹²³]|lpt[1-9¹²³]|conin\\$|conout\\$)( *)(\\.[a-z]{0,3})?",
            // anything printable or not
            "\\PC{1,24}",
            // any characters, controls and NUL included
            "(?s).{1,20}",
        ]
    }

    proptest! {
        #[test]
        fn encoding_is_legal_everywhere_and_round_trips(element in element()) {
            let component = encode_component(&element).expect("short elements encode");
            for platform in Platform::ALL {
                prop_assert!(is_legal_filename(&component, platform), "{component:?} on {platform}");
                prop_assert_eq!(decode_component(&component, platform), Ok(element.clone()));
            }
        }

        #[test]
        fn decoding_then_encoding_reaches_the_portable_spelling(element in element()) {
            // Any legal raw spelling decodes to an element whose portable
            // spelling decodes to the same element.
            if let Ok(decoded) = decode_component(&element, Platform::Linux)
                && let Ok(portable) = encode_component(&decoded)
            {
                prop_assert_eq!(decode_component(&portable, Platform::Windows), Ok(decoded));
            }
        }
    }
}
