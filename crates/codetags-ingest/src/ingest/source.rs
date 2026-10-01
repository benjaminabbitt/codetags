//! Lexical facts about an occurrence, read from the source text.
//!
//! SCIP gives an occurrence's range but not what kind of expression it is.
//! Two facts come from the text instead:
//!
//! - whether the occurrence is a **name** at all: rust-analyzer writes
//!   operator uses (`+`, `-`, `%`) as references to `core` impls, three
//!   one-character occurrences over ` + ` (V65), none of which spells a
//!   name;
//! - whether a name is **called**: the next token after it, skipping
//!   whitespace and a turbofish (`::<…>`), is `(`.
//!
//! Positions are byte offsets within a line, which is what rust-analyzer
//! writes (V65). An offset that is out of range or not on a character
//! boundary yields "unknown", never a panic.

use crate::provider::scip::Range;

/// One source file, split into lines.
#[derive(Debug)]
pub(crate) struct Source {
    text: String,
    /// Byte offset of each line's start in `text`.
    line_starts: Vec<usize>,
}

impl Source {
    /// Wraps a file's contents. Invalid UTF-8 is replaced, which keeps
    /// offsets intact only on valid lines; facts about others are unknown.
    pub(crate) fn new(bytes: &[u8]) -> Self {
        let text = String::from_utf8_lossy(bytes).into_owned();
        let line_starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(at, _)| at + 1))
            .collect();
        Self { text, line_starts }
    }

    /// The byte offset of `(line, character)`, if it is in the file.
    fn offset(&self, line: u32, character: u32) -> Option<usize> {
        let start = *self.line_starts.get(usize::try_from(line).ok()?)?;
        let offset = start.checked_add(usize::try_from(character).ok()?)?;
        (offset <= self.text.len() && self.text.is_char_boundary(offset)).then_some(offset)
    }

    /// The text `range` covers, if it is in the file.
    pub(crate) fn text(&self, range: &Range) -> Option<&str> {
        let start = self.offset(range.start_line, range.start_character)?;
        let end = self.offset(range.end_line, range.end_character)?;
        self.text.get(start..end)
    }

    /// Whether `range` spells a name: `Some(false)` for an operator or other
    /// punctuation, `None` when the range is not in the file.
    pub(crate) fn is_name(&self, range: &Range) -> Option<bool> {
        let text = self.text(range)?;
        Some(text.chars().any(|c| c.is_alphanumeric() || c == '_'))
    }

    /// Whether the name at `range` is followed by an argument list:
    /// `name(`, `name (` or `name::<T>(`.
    pub(crate) fn is_called(&self, range: &Range) -> bool {
        let Some(end) = self.offset(range.end_line, range.end_character) else {
            return false;
        };
        let mut rest = self.text[end..].trim_start();
        if let Some(generic) = rest.strip_prefix("::") {
            let generic = generic.trim_start();
            let Some(after) = skip_angle_brackets(generic) else {
                return false;
            };
            rest = after.trim_start();
        }
        rest.starts_with('(')
    }
}

/// Skips one balanced `<…>` group at the start of `text`.
fn skip_angle_brackets(text: &str) -> Option<&str> {
    if !text.starts_with('<') {
        return None;
    }
    let mut depth = 0usize;
    for (at, c) in text.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[at + 1..]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn range(line: u32, start: u32, end: u32) -> Range {
        Range {
            start_line: line,
            start_character: start,
            end_line: line,
            end_character: end,
        }
    }

    #[test]
    fn operator_occurrences_are_not_names() {
        let source = Source::new(b"fn f() {\n    amount + self.0\n}\n");
        // rust-analyzer's three occurrences over ` + ` (V65).
        for column in 10..13 {
            assert_eq!(
                source.is_name(&range(1, column, column + 1)),
                Some(false),
                "{column}"
            );
        }
        assert_eq!(source.is_name(&range(1, 4, 10)), Some(true));
        assert_eq!(source.text(&range(1, 4, 10)), Some("amount"));
    }

    #[test]
    fn ranges_outside_the_file_are_unknown() {
        let source = Source::new("é = 1\n".as_bytes());
        assert_eq!(source.is_name(&range(5, 0, 1)), None);
        assert_eq!(source.is_name(&range(0, 0, 40)), None);
        // Inside the two-byte `é`.
        assert_eq!(source.is_name(&range(0, 1, 2)), None);
        assert!(!source.is_called(&range(9, 0, 1)));
    }

    #[test]
    fn a_name_followed_by_arguments_is_called() {
        let source = Source::new(
            b"a(1); b (2); c::<Vec<u8>>(); d::< T >(x); e.map(f); let g = h;\nk\n  (1)",
        );
        let called = |start: u32, end: u32| source.is_called(&range(0, start, end));
        assert!(called(0, 1), "a(");
        assert!(called(6, 7), "b (");
        assert!(called(13, 14), "c::<Vec<u8>>(");
        assert!(called(29, 30), "d::< T >(");
        assert!(called(44, 47), "map(");
        assert!(!called(48, 49), "f)");
        assert!(!called(60, 61), "h;");
        assert!(
            source.is_called(&range(1, 0, 1)),
            "k, then ( on the next line"
        );
    }

    #[test]
    fn an_unclosed_turbofish_is_not_a_call() {
        let source = Source::new(b"c::<Vec<u8>(");
        assert!(!source.is_called(&range(0, 0, 1)));
    }
}
