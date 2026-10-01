//! A minimal SCIP reader over the pinned `scip` bindings (V63).
//!
//! It decodes `index.scip` into plain types that the provider checks, the
//! BDD steps, and ingest (P1.3) share, so nothing else touches protobuf.
//! Ranges are decoded from the typed fields when present and from the
//! deprecated packed `range`/`enclosing_range` arrays otherwise: stale
//! bindings that knew only one encoding once read a provider's output as zero
//! references (brief §7).

use std::fmt;
use std::path::{Path, PathBuf};

use protobuf::Message;
use scip::types::occurrence::{Typed_enclosing_range, Typed_range};
use scip::types::{MultiLineRange, SingleLineRange, SymbolRole};

pub use scip::types::symbol_information::Kind as SymbolKind;

/// A decoded SCIP index.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScipIndex {
    /// The provider's name and version, from `metadata.tool_info`.
    pub tool: String,
    /// `metadata.project_root`, a URI.
    pub project_root: String,
    /// One per indexed source file.
    pub documents: Vec<Document>,
}

/// One source file in the index.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document {
    /// Path relative to the project root, with `/` separators.
    pub relative_path: String,
    /// Every occurrence in the file, in the provider's order.
    pub occurrences: Vec<Occurrence>,
    /// Symbols defined in this file.
    pub symbols: Vec<SymbolInfo>,
}

impl ScipIndex {
    /// Removes every local symbol's occurrences and information (brief §4.4:
    /// `local N` symbols are exactly the discarded local scope).
    pub fn drop_locals(&mut self) {
        self.documents.iter_mut().for_each(Document::drop_locals);
    }
}

impl Document {
    /// Removes this document's local symbols' occurrences and information.
    pub fn drop_locals(&mut self) {
        self.occurrences.retain(|occurrence| !occurrence.is_local());
        self.symbols.retain(|info| !is_local_symbol(&info.symbol));
    }
}

/// A position range: 0-based lines and characters, end exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Range {
    /// First line.
    pub start_line: u32,
    /// Character offset on the first line.
    pub start_character: u32,
    /// Last line.
    pub end_line: u32,
    /// Character offset on the last line (exclusive).
    pub end_character: u32,
}

impl Range {
    /// Whether `other` lies within this range.
    pub fn contains(&self, other: &Range) -> bool {
        (self.start_line, self.start_character) <= (other.start_line, other.start_character)
            && (other.end_line, other.end_character) <= (self.end_line, self.end_character)
    }
}

/// One occurrence of a symbol in a document.
#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    /// The SCIP symbol string (`local N` for a local).
    pub symbol: String,
    /// Where the symbol's name appears.
    pub range: Range,
    /// For a definition, the whole definition's extent (brief §4.4: the
    /// innermost enclosing range attributes references).
    pub enclosing_range: Option<Range>,
    /// The `SymbolRole` bit set.
    pub roles: i32,
}

impl Occurrence {
    /// Whether this occurrence defines its symbol.
    pub fn is_definition(&self) -> bool {
        self.roles & (SymbolRole::Definition as i32) != 0
    }

    /// Whether the symbol is a document-local one (`local N`), which ingest
    /// filters out (brief §4.4).
    pub fn is_local(&self) -> bool {
        is_local_symbol(&self.symbol)
    }
}

/// Whether `symbol` is a document-local SCIP symbol (`local <id>`).
pub fn is_local_symbol(symbol: &str) -> bool {
    symbol.starts_with("local ")
}

/// Metadata about a symbol defined in a document.
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolInfo {
    /// The SCIP symbol string.
    pub symbol: String,
    /// The provider's kind, or `None` when unspecified or unknown to the
    /// pinned bindings.
    pub kind: Option<SymbolKind>,
    /// Relationships to other symbols.
    pub relationships: Vec<Relationship>,
}

/// A relationship from a symbol to another.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Relationship {
    /// The other symbol.
    pub symbol: String,
    /// The symbol implements `symbol` (trait impls, brief §4.4).
    pub is_implementation: bool,
    /// Find-references on the symbol should include `symbol`.
    pub is_reference: bool,
    /// `symbol` is the symbol's type definition.
    pub is_type_definition: bool,
    /// The symbol's definition is `symbol`'s.
    pub is_definition: bool,
}

/// Why an index could not be read.
#[derive(Debug)]
pub enum ReadError {
    /// The file could not be read.
    Io {
        /// The index file.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The bytes are not a SCIP index.
    Decode(String),
    /// A range array has a length other than 3 or 4, or a negative field.
    BadRange {
        /// The document holding the occurrence.
        document: String,
        /// The occurrence's symbol.
        symbol: String,
        /// The raw values.
        values: Vec<i32>,
    },
    /// A typed range encoding newer than the pinned bindings.
    UnknownRangeEncoding {
        /// The document holding the occurrence.
        document: String,
        /// The occurrence's symbol.
        symbol: String,
    },
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "reading {}: {source}", path.display()),
            Self::Decode(message) => write!(f, "not a SCIP index: {message}"),
            Self::BadRange {
                document,
                symbol,
                values,
            } => write!(
                f,
                "{document}: occurrence of {symbol:?} has a malformed range {values:?}"
            ),
            Self::UnknownRangeEncoding { document, symbol } => write!(
                f,
                "{document}: occurrence of {symbol:?} uses a range encoding the pinned SCIP bindings do not know"
            ),
        }
    }
}

impl std::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Reads and decodes the SCIP index at `path`.
pub fn read_index(path: &Path) -> Result<ScipIndex, ReadError> {
    let bytes = std::fs::read(path).map_err(|source| ReadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    decode_index(&bytes)
}

/// Decodes a SCIP index from its protobuf bytes.
pub fn decode_index(bytes: &[u8]) -> Result<ScipIndex, ReadError> {
    let raw = scip::types::Index::parse_from_bytes(bytes)
        .map_err(|error| ReadError::Decode(error.to_string()))?;
    let tool = raw
        .metadata
        .tool_info
        .as_ref()
        .map(|info| format!("{} {}", info.name, info.version))
        .unwrap_or_default();
    let documents = raw
        .documents
        .iter()
        .map(decode_document)
        .collect::<Result<_, _>>()?;
    Ok(ScipIndex {
        tool,
        project_root: raw.metadata.project_root.clone(),
        documents,
    })
}

fn decode_document(raw: &scip::types::Document) -> Result<Document, ReadError> {
    let occurrences = raw
        .occurrences
        .iter()
        .map(|occurrence| decode_occurrence(&raw.relative_path, occurrence))
        .collect::<Result<_, _>>()?;
    let symbols = raw.symbols.iter().map(decode_symbol).collect();
    Ok(Document {
        relative_path: raw.relative_path.clone(),
        occurrences,
        symbols,
    })
}

fn decode_occurrence(
    document: &str,
    raw: &scip::types::Occurrence,
) -> Result<Occurrence, ReadError> {
    let bad = |values: &[i32]| ReadError::BadRange {
        document: document.to_string(),
        symbol: raw.symbol.clone(),
        values: values.to_vec(),
    };
    let unknown = || ReadError::UnknownRangeEncoding {
        document: document.to_string(),
        symbol: raw.symbol.clone(),
    };
    // The typed form takes precedence over the packed one (scip.proto). A
    // typed variant newer than the pinned bindings is an error, never a
    // silently empty range.
    let range = match &raw.typed_range {
        Some(Typed_range::SingleLineRange(single)) => single_line(single),
        Some(Typed_range::MultiLineRange(multi)) => multi_line(multi),
        Some(_) => return Err(unknown()),
        None => packed(&raw.range),
    }
    .ok_or_else(|| bad(&raw.range))?;
    let enclosing_range = match &raw.typed_enclosing_range {
        Some(Typed_enclosing_range::SingleLineEnclosingRange(single)) => {
            Some(single_line(single).ok_or_else(|| bad(&raw.enclosing_range))?)
        }
        Some(Typed_enclosing_range::MultiLineEnclosingRange(multi)) => {
            Some(multi_line(multi).ok_or_else(|| bad(&raw.enclosing_range))?)
        }
        Some(_) => return Err(unknown()),
        None if raw.enclosing_range.is_empty() => None,
        None => Some(packed(&raw.enclosing_range).ok_or_else(|| bad(&raw.enclosing_range))?),
    };
    Ok(Occurrence {
        symbol: raw.symbol.clone(),
        range,
        enclosing_range,
        roles: raw.symbol_roles,
    })
}

fn decode_symbol(raw: &scip::types::SymbolInformation) -> SymbolInfo {
    let kind = raw
        .kind
        .enum_value()
        .ok()
        .filter(|kind| *kind != SymbolKind::UnspecifiedKind);
    SymbolInfo {
        symbol: raw.symbol.clone(),
        kind,
        relationships: raw
            .relationships
            .iter()
            .map(|relationship| Relationship {
                symbol: relationship.symbol.clone(),
                is_implementation: relationship.is_implementation,
                is_reference: relationship.is_reference,
                is_type_definition: relationship.is_type_definition,
                is_definition: relationship.is_definition,
            })
            .collect(),
    }
}

fn single_line(range: &SingleLineRange) -> Option<Range> {
    let line = u32::try_from(range.line).ok()?;
    Some(Range {
        start_line: line,
        start_character: u32::try_from(range.start_character).ok()?,
        end_line: line,
        end_character: u32::try_from(range.end_character).ok()?,
    })
}

fn multi_line(range: &MultiLineRange) -> Option<Range> {
    Some(Range {
        start_line: u32::try_from(range.start_line).ok()?,
        start_character: u32::try_from(range.start_character).ok()?,
        end_line: u32::try_from(range.end_line).ok()?,
        end_character: u32::try_from(range.end_character).ok()?,
    })
}

/// The deprecated packed encoding: `[line, start, end]` or
/// `[start line, start, end line, end]`.
fn packed(values: &[i32]) -> Option<Range> {
    let field = |i: usize| values.get(i).and_then(|v| u32::try_from(*v).ok());
    match values.len() {
        3 => Some(Range {
            start_line: field(0)?,
            start_character: field(1)?,
            end_line: field(0)?,
            end_character: field(2)?,
        }),
        4 => Some(Range {
            start_line: field(0)?,
            start_character: field(1)?,
            end_line: field(2)?,
            end_character: field(3)?,
        }),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    /// Builds a raw occurrence with packed ranges.
    pub(crate) fn raw_occurrence(
        symbol: &str,
        range: &[i32],
        enclosing: &[i32],
        roles: i32,
    ) -> scip::types::Occurrence {
        let mut occurrence = scip::types::Occurrence::new();
        occurrence.symbol = symbol.to_string();
        occurrence.range = range.to_vec();
        occurrence.enclosing_range = enclosing.to_vec();
        occurrence.symbol_roles = roles;
        occurrence
    }

    /// Encodes one document holding `occurrences` as index bytes.
    pub(crate) fn index_bytes(path: &str, occurrences: Vec<scip::types::Occurrence>) -> Vec<u8> {
        let mut document = scip::types::Document::new();
        document.relative_path = path.to_string();
        document.occurrences = occurrences;
        let mut index = scip::types::Index::new();
        index.documents.push(document);
        index.write_to_bytes().unwrap()
    }

    fn only_occurrence(bytes: &[u8]) -> Occurrence {
        let index = decode_index(bytes).unwrap();
        index.documents[0].occurrences[0].clone()
    }

    #[test]
    fn packed_ranges_decode_in_both_lengths() {
        let occurrence = only_occurrence(&index_bytes(
            "src/a.rs",
            vec![raw_occurrence("s", &[4, 7, 10], &[3, 0, 9, 1], 1)],
        ));
        assert_eq!(
            occurrence.range,
            Range {
                start_line: 4,
                start_character: 7,
                end_line: 4,
                end_character: 10
            }
        );
        assert_eq!(
            occurrence.enclosing_range,
            Some(Range {
                start_line: 3,
                start_character: 0,
                end_line: 9,
                end_character: 1
            })
        );
        assert!(occurrence.is_definition());
    }

    #[test]
    fn typed_ranges_take_precedence_over_packed_ones() {
        let mut raw = raw_occurrence("s", &[0, 0, 1], &[0, 0, 0, 1], 0);
        let mut single = SingleLineRange::new();
        single.line = 2;
        single.start_character = 3;
        single.end_character = 8;
        raw.set_single_line_range(single);
        let mut multi = MultiLineRange::new();
        multi.start_line = 1;
        multi.end_line = 5;
        multi.end_character = 2;
        raw.set_multi_line_enclosing_range(multi);
        let occurrence = only_occurrence(&index_bytes("src/a.rs", vec![raw]));
        assert_eq!(
            (occurrence.range.start_line, occurrence.range.end_character),
            (2, 8)
        );
        let enclosing = occurrence.enclosing_range.unwrap();
        assert_eq!((enclosing.start_line, enclosing.end_line), (1, 5));
        assert!(!occurrence.is_definition());
    }

    #[test]
    fn no_enclosing_range_is_none() {
        let occurrence = only_occurrence(&index_bytes(
            "src/a.rs",
            vec![raw_occurrence("s", &[0, 0, 1], &[], 8)],
        ));
        assert_eq!(occurrence.enclosing_range, None);
    }

    #[test]
    fn malformed_ranges_are_errors_not_zeroes() {
        for bad in [&[1, 2][..], &[0, -1, 3], &[]] {
            let bytes = index_bytes("src/a.rs", vec![raw_occurrence("s", bad, &[], 0)]);
            let error = decode_index(&bytes).unwrap_err();
            assert!(
                matches!(error, ReadError::BadRange { .. }),
                "{bad:?}: {error}"
            );
        }
    }

    #[test]
    fn garbage_is_a_decode_error() {
        assert!(matches!(
            decode_index(b"\xff\xff\xff not protobuf"),
            Err(ReadError::Decode(_))
        ));
    }

    #[test]
    fn local_symbols_are_recognised() {
        assert!(is_local_symbol("local 12"));
        assert!(!is_local_symbol("rust-analyzer cargo billing 0.1.0 local/"));
    }

    #[test]
    fn drop_locals_keeps_only_global_symbols() {
        let bytes = index_bytes(
            "src/a.rs",
            vec![
                raw_occurrence("local 0", &[0, 0, 1], &[], 1),
                raw_occurrence("cargo x 1 f().", &[1, 0, 1], &[], 0),
            ],
        );
        let mut index = decode_index(&bytes).unwrap();
        index.drop_locals();
        let symbols: Vec<_> = index.documents[0]
            .occurrences
            .iter()
            .map(|o| o.symbol.as_str())
            .collect();
        assert_eq!(symbols, ["cargo x 1 f()."]);
    }

    #[test]
    fn contains_is_inclusive_of_equal_bounds() {
        let outer = Range {
            start_line: 1,
            start_character: 0,
            end_line: 5,
            end_character: 1,
        };
        let inner = Range {
            start_line: 1,
            start_character: 0,
            end_line: 2,
            end_character: 9,
        };
        assert!(outer.contains(&inner));
        assert!(outer.contains(&outer));
        assert!(!inner.contains(&outer));
    }
}
