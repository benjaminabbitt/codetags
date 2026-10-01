//! Item ids for the tags file (PLAN.md §2.3, §2.4, D15).
//!
//! An id is `file:<project-relative POSIX path>` or `sym:<canonical name>`.
//! It is written bare unless it holds whitespace or `"`; then the whole id
//! is written as a tagma quoted token (tagma SPEC §2): wrapped in `"`, with
//! `"` written `\"` and `\` written `\\`.
//!
//! ```text
//! file:src/billing/charge.rs
//! "file:docs/My Notes.md"
//! ```
//!
//! tagma's `add_line` keeps a quoted id's quotes and does not decode them
//! (V21), so codetags splits a line with
//! `tagma_core::token::split_unquoted_whitespace` and reads the id with
//! [`ItemId`]'s `FromStr`. Every id it writes is exactly one such field.

use std::fmt;
use std::str::FromStr;

/// The id of a taggable item.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ItemId {
    /// A file, by its project-relative POSIX path.
    File(String),
    /// A symbol, by its canonical name.
    Symbol(String),
}

/// The prefix of a file id.
pub const FILE_PREFIX: &str = "file:";
/// The prefix of a symbol id.
pub const SYMBOL_PREFIX: &str = "sym:";

/// Why a string is not an item id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemIdError {
    /// It does not start with `file:` or `sym:`.
    UnknownKind,
    /// The value after the prefix is empty.
    Empty,
    /// A bare id holds whitespace or `"`, which only a quoted id may.
    NeedsQuoting(char),
    /// A quoted id is malformed: unterminated, a bad `\` escape, or
    /// text after the closing quote.
    BadQuoting(String),
}

impl fmt::Display for ItemIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ItemIdError::UnknownKind => write!(
                f,
                "an item id starts with {FILE_PREFIX:?} or {SYMBOL_PREFIX:?}"
            ),
            ItemIdError::Empty => f.write_str("an item id has a value after its prefix"),
            ItemIdError::NeedsQuoting(c) => write!(
                f,
                "an item id holding {c:?} must be quoted: \"file:...\", with \\\" and \\\\ escapes"
            ),
            ItemIdError::BadQuoting(why) => write!(f, "malformed quoted item id: {why}"),
        }
    }
}

impl std::error::Error for ItemIdError {}

/// Returns `true` if an id holding `c` must be quoted.
fn needs_quoting(c: char) -> bool {
    c == '"' || c.is_whitespace()
}

impl ItemId {
    fn parts(&self) -> (&'static str, &str) {
        match self {
            ItemId::File(path) => (FILE_PREFIX, path),
            ItemId::Symbol(name) => (SYMBOL_PREFIX, name),
        }
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (prefix, value) = self.parts();
        if !value.chars().any(needs_quoting) {
            return write!(f, "{prefix}{value}");
        }
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        write!(f, "\"{prefix}{escaped}\"")
    }
}

/// Decodes a whole tagma quoted token: `"`, content with `\"` and `\\`
/// escapes, `"`, and nothing after.
fn unquote(s: &str) -> Result<String, ItemIdError> {
    let bad = |why: &str| Err(ItemIdError::BadQuoting(why.to_string()));
    let Some(body) = s.strip_prefix('"') else {
        return bad("no opening quote");
    };
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(e @ ('"' | '\\')) => out.push(e),
                Some(_) => return bad("a backslash escapes only '\"' or '\\'"),
                None => return bad("a dangling backslash"),
            },
            '"' => {
                if chars.next().is_some() {
                    return bad("text after the closing quote");
                }
                return Ok(out);
            }
            _ => out.push(c),
        }
    }
    bad("no closing quote")
}

impl FromStr for ItemId {
    type Err = ItemIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let text = if s.starts_with('"') {
            unquote(s)?
        } else if let Some(c) = s.chars().find(|c| needs_quoting(*c)) {
            return Err(ItemIdError::NeedsQuoting(c));
        } else {
            s.to_string()
        };
        let id = if let Some(path) = text.strip_prefix(FILE_PREFIX) {
            ItemId::File(path.to_string())
        } else if let Some(name) = text.strip_prefix(SYMBOL_PREFIX) {
            ItemId::Symbol(name.to_string())
        } else {
            return Err(ItemIdError::UnknownKind);
        };
        if id.parts().1.is_empty() {
            return Err(ItemIdError::Empty);
        }
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn spellings() {
        for (id, text) in [
            (ItemId::File("src/a.rs".into()), "file:src/a.rs"),
            (ItemId::File("My Notes.md".into()), "\"file:My Notes.md\""),
            (ItemId::File("100%.txt".into()), "file:100%.txt"),
            (ItemId::File("a\\b".into()), "file:a\\b"),
            (ItemId::File("say \"hi\"".into()), "\"file:say \\\"hi\\\"\""),
            (ItemId::File("a\\ b".into()), "\"file:a\\\\ b\""),
            (
                ItemId::Symbol("a.Charge<T>.c+1".into()),
                "sym:a.Charge<T>.c+1",
            ),
        ] {
            assert_eq!(id.to_string(), text);
            assert_eq!(text.parse(), Ok(id));
        }
    }

    #[test]
    fn rejects() {
        assert_eq!("dir:x".parse::<ItemId>(), Err(ItemIdError::UnknownKind));
        assert_eq!("file:".parse::<ItemId>(), Err(ItemIdError::Empty));
        assert_eq!("\"sym:\"".parse::<ItemId>(), Err(ItemIdError::Empty));
        assert_eq!(
            "file:a b".parse::<ItemId>(),
            Err(ItemIdError::NeedsQuoting(' '))
        );
        assert_eq!(
            "file:a\"b".parse::<ItemId>(),
            Err(ItemIdError::NeedsQuoting('"'))
        );
        for bad in ["\"file:a", "\"file:a\"b", "\"file:\\n\"", "\"file:a\\"] {
            assert!(
                matches!(bad.parse::<ItemId>(), Err(ItemIdError::BadQuoting(_))),
                "{bad:?}"
            );
        }
    }

    proptest! {
        #[test]
        fn round_trips_as_one_tagma_field(value in "(?s).{1,30}", file in any::<bool>()) {
            let id = if file { ItemId::File(value) } else { ItemId::Symbol(value) };
            let text = id.to_string();
            let fields = tagma_core::token::split_unquoted_whitespace(&text);
            prop_assert_eq!(fields, Ok(vec![text.as_str()]));
            prop_assert_eq!(text.parse::<ItemId>(), Ok(id));
        }
    }
}
