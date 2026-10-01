//! The private-use spelling of names that Win32 forbids (S4).
//!
//! Cygwin, MSYS2 (Git for Windows' bash) and WSL store a character Win32
//! forbids in a file name as U+F000 plus its ASCII code, so `:` becomes
//! U+F03A; they do the same for a trailing dot or space. WinFsp's kernel
//! driver passes such names through, while it rejects the ASCII characters
//! themselves (docs/verification.md). The spike's probe mode serves files
//! named this way, so the S4 probe can see what each tool sends and shows.
//!
//! This is the spike's test fixture, not the product mapping: that is
//! `codetags_names::windows` (P1.2, D15). This one also maps trailing dots
//! and spaces, so the probe can ask whether any tool sends them that way.
//!
//! Platform-neutral, so it is unit-tested everywhere.

/// Names whose private-use spelling the probe serves: one per character
/// Win32 forbids in a name (other than the separators), plus a trailing dot
/// and a trailing space.
pub const PROBE_NAMES: [&str; 9] = ["a\"b", "a*b", "a:b", "a<b", "a>b", "a?b", "a|b", "a.", "a "];

/// The private-use spelling of `name`: each character Win32 forbids becomes
/// U+F000 plus its ASCII code, as do trailing dots and spaces. Other
/// characters are unchanged.
pub fn private_use_spelling(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let trailing_from = chars
        .iter()
        .rposition(|&c| c != '.' && c != ' ')
        .map_or(0, |last| last + 1);
    chars
        .iter()
        .enumerate()
        .map(|(index, &c)| {
            let forbidden = matches!(c, '"' | '*' | ':' | '<' | '>' | '?' | '|');
            if forbidden || index >= trailing_from {
                char::from_u32(0xF000 | c as u32).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

/// The content of the probe file for `name`.
pub fn probe_content(name: &str) -> String {
    format!("private-use {name}\n")
}

/// `name` with every character outside printable ASCII written as
/// `<U+XXXX>`, so a report shows the exact code points.
pub fn code_points(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c.to_string()
            } else {
                format!("<U+{:04X}>", c as u32)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_characters_move_to_the_private_use_area() {
        assert_eq!(private_use_spelling("a:b"), "a\u{F03A}b");
        assert_eq!(private_use_spelling("a\"b|c"), "a\u{F022}b\u{F07C}c");
        assert_eq!(private_use_spelling("prov=scip"), "prov=scip");
    }

    #[test]
    fn only_trailing_dots_and_spaces_move() {
        assert_eq!(private_use_spelling("a. b."), "a. b\u{F02E}");
        assert_eq!(private_use_spelling("a ."), "a\u{F020}\u{F02E}");
        assert_eq!(private_use_spelling(" a"), " a");
    }

    #[test]
    fn code_points_spell_out_what_is_not_printable_ascii() {
        assert_eq!(code_points("a\u{F03A}b"), "a<U+F03A>b");
        assert_eq!(code_points("a b"), "a b");
        assert_eq!(code_points("a\tb"), "a<U+0009>b");
    }
}
