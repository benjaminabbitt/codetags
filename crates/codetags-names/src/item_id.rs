//! Item ids for the tags file (PLAN.md §2.3, §2.4).
//!
//! An id is `file:<project-relative POSIX path>` or `sym:<canonical name>`.
//! The value is percent-encoded so the id never needs quoting: tagma's
//! `add_line` would keep a quoted id's quotes (V21), and codetags splits
//! lines with `tagma_core::token::split_unquoted_whitespace`.
//!
//! Only what would break a line is encoded: `%` itself, `"`, every
//! Unicode whitespace character, and every control character. `/` stays
//! raw, so a file id reads like its path (`file:docs/My%20Notes.md`).
//! This differs from the path profile on purpose: an id is a field in a
//! text file, not a file name.
//!
//! Decoding accepts any valid `%XX`, upper or lower case; [`ItemId`]'s
//! `Display` always writes the one canonical spelling, so the tags file
//! normalizes ids when it rewrites them.

use std::fmt;
use std::str::FromStr;

use crate::percent::{self, PercentError};

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
    /// It holds a character that must be encoded (whitespace, `"`, a
    /// control character).
    Unencoded(char),
    /// The value's percent-encoding is malformed.
    Percent(PercentError),
}

impl fmt::Display for ItemIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ItemIdError::UnknownKind => write!(
                f,
                "an item id starts with {FILE_PREFIX:?} or {SYMBOL_PREFIX:?}"
            ),
            ItemIdError::Empty => f.write_str("an item id has a value after its prefix"),
            ItemIdError::Unencoded(c) => {
                write!(f, "{c:?} must be percent-encoded in an item id")
            }
            ItemIdError::Percent(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ItemIdError {}

fn must_encode(c: char) -> bool {
    c == '%' || c == '"' || c.is_whitespace() || c.is_control()
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
        let mut out = String::with_capacity(prefix.len() + value.len());
        out.push_str(prefix);
        for c in value.chars() {
            if must_encode(c) {
                percent::push_encoded(&mut out, c);
            } else {
                out.push(c);
            }
        }
        f.write_str(&out)
    }
}

impl FromStr for ItemId {
    type Err = ItemIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(c) = s.chars().find(|c| *c != '%' && must_encode(*c)) {
            return Err(ItemIdError::Unencoded(c));
        }
        let (make, value): (fn(String) -> ItemId, &str) =
            if let Some(path) = s.strip_prefix(FILE_PREFIX) {
                (ItemId::File, path)
            } else if let Some(name) = s.strip_prefix(SYMBOL_PREFIX) {
                (ItemId::Symbol, name)
            } else {
                return Err(ItemIdError::UnknownKind);
            };
        if value.is_empty() {
            return Err(ItemIdError::Empty);
        }
        percent::decode(value)
            .map(make)
            .map_err(ItemIdError::Percent)
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
            (ItemId::File("My Notes.md".into()), "file:My%20Notes.md"),
            (ItemId::File("a\"b%\u{a0}".into()), "file:a%22b%25%C2%A0"),
            (ItemId::Symbol("a.B.c+1".into()), "sym:a.B.c+1"),
        ] {
            assert_eq!(id.to_string(), text);
            assert_eq!(text.parse(), Ok(id));
        }
    }

    #[test]
    fn rejects() {
        assert_eq!("dir:x".parse::<ItemId>(), Err(ItemIdError::UnknownKind));
        assert_eq!("file:".parse::<ItemId>(), Err(ItemIdError::Empty));
        assert_eq!(
            "file:a b".parse::<ItemId>(),
            Err(ItemIdError::Unencoded(' '))
        );
        assert_eq!(
            "sym:\"a\"".parse::<ItemId>(),
            Err(ItemIdError::Unencoded('"'))
        );
        assert!(matches!(
            "file:50%".parse::<ItemId>(),
            Err(ItemIdError::Percent(_))
        ));
    }

    proptest! {
        #[test]
        fn round_trips_and_is_one_unquoted_field(value in "(?s).{1,30}", file in any::<bool>()) {
            let id = if file { ItemId::File(value) } else { ItemId::Symbol(value) };
            let text = id.to_string();
            prop_assert!(!text.contains('"'));
            prop_assert!(!text.chars().any(char::is_whitespace));
            prop_assert_eq!(text.parse::<ItemId>(), Ok(id));
        }
    }
}
