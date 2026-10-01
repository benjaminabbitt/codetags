//! The Windows private-use mapping (PLAN.md §2.7, D15).
//!
//! Win32 forbids `" * : < > ? |` in a file name. Cygwin, MSYS2 (Git Bash)
//! and WSL store each of them as the private-use code point U+F000 plus its
//! ASCII code, so `:` is stored as U+F03A (V39). The WinFsp backend decodes
//! names it receives with [`from_windows_name`] and encodes names it returns
//! with [`to_windows_name`], and the Windows static-tree writer encodes the
//! same way. Agents in Git Bash or WSL then type and see the real
//! characters.
//!
//! The mapping is pure and works on any host. It covers exactly the seven
//! characters above. Not yet handled, pending spike S4's report on what
//! really reaches a WinFsp file system:
//!
//! - TODO(S4): reserved device names (`CON`, `NUL`, `COM1`, …, with or
//!   without an extension).
//! - TODO(S4): a trailing `.` or space, which Win32 strips.
//! - TODO(S4): control characters 1–31, which Cygwin also maps to U+F001
//!   through U+F01F. Canonical names never hold them, but a quoted tag
//!   value could.
//!
//! **Ambiguity.** A name that already holds one of the seven private-use
//! code points decodes to the ASCII character, as it does under Cygwin.
//! Round-tripping is exact for every name without them.

/// The characters Win32 forbids that the mapping moves to private use.
pub const MAPPED: [char; 7] = ['"', '*', ':', '<', '>', '?', '|'];

/// The start of the private-use block the mapping uses.
const BASE: u32 = 0xF000;

/// The private-use code point that stands for `c`, if `c` is mapped.
fn to_private(c: char) -> Option<char> {
    if MAPPED.contains(&c) {
        char::from_u32(BASE + u32::from(c))
    } else {
        None
    }
}

/// The mapped character that the private-use code point `c` stands for.
fn from_private(c: char) -> Option<char> {
    let ascii = u32::from(c).checked_sub(BASE)?;
    char::from_u32(ascii).filter(|a| MAPPED.contains(a))
}

/// Encodes a name for a Windows file system: each of [`MAPPED`] becomes
/// U+F000 plus its ASCII code. Everything else is unchanged.
pub fn to_windows_name(name: &str) -> String {
    name.chars().map(|c| to_private(c).unwrap_or(c)).collect()
}

/// Decodes a name read from a Windows file system: each private-use code
/// point that stands for one of [`MAPPED`] becomes that character.
pub fn from_windows_name(name: &str) -> String {
    name.chars().map(|c| from_private(c).unwrap_or(c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn maps_each_reserved_character() {
        assert_eq!(
            to_windows_name("\"*:<>?|"),
            "\u{F022}\u{F02A}\u{F03A}\u{F03C}\u{F03E}\u{F03F}\u{F07C}"
        );
        assert_eq!(to_windows_name("a\\b/c~d"), "a\\b/c~d");
        assert_eq!(from_windows_name("prov\u{F03A}source"), "prov:source");
        // A private-use code point that stands for no mapped character stays.
        assert_eq!(from_windows_name("\u{F041}\u{F000}"), "\u{F041}\u{F000}");
    }

    /// Characters that may appear in a name, heavy on the mapped ones.
    fn name() -> impl Strategy<Value = String> {
        prop_oneof!["[\"*:<>?|a-z=~ .\\\\/]{0,24}", "\\PC{0,24}"]
            .prop_filter("holds no mapped private-use code point", |s| {
                !s.chars().any(|c| from_private(c).is_some())
            })
    }

    proptest! {
        #[test]
        fn round_trips(name in name()) {
            let windows = to_windows_name(&name);
            prop_assert!(!windows.contains(MAPPED), "{windows:?}");
            prop_assert_eq!(from_windows_name(&windows), name);
        }

        #[test]
        fn decoding_then_encoding_is_the_identity_on_windows_names(name in "\\PC{0,24}") {
            // Any name Windows can hold (none of the mapped characters)
            // survives decode then encode.
            let stored = to_windows_name(&name);
            prop_assert_eq!(to_windows_name(&from_windows_name(&stored)), stored);
        }
    }
}
