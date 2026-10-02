//! The syntax around a SCIP call site, from tree-sitter (PLAN.md D32: the
//! spike for P1.3b). **Not wired into ingest:** nothing here changes what a
//! generation holds yet.
//!
//! SCIP names every call site's target but carries no syntax around it.
//! P1.3b records each site's control context (the enclosing conditional,
//! loop, `try` and `await`) and the string-literal arguments that name
//! things (topics, tables, routes, event types; brief §4.4). Here
//! tree-sitter supplies only that syntax, joined to a site SCIP already
//! found by its position; names still come from SCIP.
//!
//! - **The join.** A SCIP position is a 0-based line and a column in the
//!   provider's encoding: UTF-8 bytes for rust-analyzer and scip-go,
//!   UTF-16 code units for scip-python and scip-typescript (V92, V162).
//!   [`byte_offset`] converts it to a byte offset in the source, which
//!   tree-sitter indexes by; the smallest named node there is the name SCIP
//!   pointed at.
//! - **The walk.** From that name up to the enclosing definition: the first
//!   call node whose callee holds the name is the site's call (none when the
//!   name is a function used as a value, e.g. a callback argument); every
//!   control node passed on the way, entered through a child that counts
//!   (not an `if`'s condition, not a `for`'s iterable), adds to the context.
//!   Closures and lambdas are passed through, since SCIP attributes their
//!   calls to the enclosing definition.
//! - **Literals.** The call's direct arguments that are constant string
//!   literals, without their quotes; escapes are kept as written. A string
//!   with interpolation (an f-string, a template with `${}`) is not a
//!   constant, and is skipped.
//!
//! One walker serves all four languages; what differs is the node-kind table
//! in [`table`].

mod table;

use tree_sitter::{Node, Parser, Tree};

use self::table::{Counted, table};

/// A language the walker knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    /// tree-sitter-rust.
    Rust,
    /// tree-sitter-go.
    Go,
    /// tree-sitter-typescript's TypeScript grammar, also used for
    /// JavaScript files.
    TypeScript,
    /// tree-sitter-python.
    Python,
}

impl Language {
    /// The grammar.
    pub fn grammar(self) -> tree_sitter::Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
        }
    }
}

/// How a provider counts columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// UTF-8 bytes: rust-analyzer (V65), scip-go.
    Utf8,
    /// UTF-16 code units: scip-python (V92), scip-typescript (V162).
    Utf16,
}

/// A kind of control context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Control {
    /// Runs only on some paths: `if`, `match`, `switch`, a ternary.
    Conditional,
    /// Runs repeatedly: a loop body, a comprehension's element.
    Loop,
    /// Inside a `try` block's body.
    Try,
    /// Awaited.
    Await,
    /// Started as a goroutine (`go`), Go only.
    Goroutine,
    /// Deferred (`defer`), Go only.
    Defer,
}

/// The syntax around one call site.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SiteSyntax {
    /// The kind of the call node whose callee holds the site's name; `None`
    /// when the name is not called there (a function used as a value).
    pub call: Option<&'static str>,
    /// The control context, innermost first.
    pub control: Vec<Control>,
    /// The call's constant string-literal arguments, in order, unquoted.
    pub literals: Vec<String>,
}

/// Why parsing failed.
#[derive(Debug)]
pub enum SyntaxError {
    /// The grammar's ABI is outside the range the tree-sitter core reads.
    Abi(tree_sitter::LanguageError),
    /// tree-sitter returned no tree.
    NoTree,
}

impl std::fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Abi(error) => write!(f, "grammar: {error}"),
            Self::NoTree => write!(f, "tree-sitter returned no tree"),
        }
    }
}

impl std::error::Error for SyntaxError {}

/// Parses `source` as `language`.
pub fn parse(language: Language, source: &[u8]) -> Result<Tree, SyntaxError> {
    let mut parser = Parser::new();
    parser
        .set_language(&language.grammar())
        .map_err(SyntaxError::Abi)?;
    parser.parse(source, None).ok_or(SyntaxError::NoTree)
}

/// The byte offset of 0-based `line`, `column` in `source`, with the column
/// counted in `encoding`; `None` past the end of the line or the source,
/// or inside a character.
pub fn byte_offset(source: &[u8], line: u32, column: u32, encoding: Encoding) -> Option<usize> {
    let mut start = 0;
    for _ in 0..line {
        start += source.get(start..)?.iter().position(|&b| b == b'\n')? + 1;
    }
    let rest = source.get(start..)?;
    let text = &rest[..rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len())];
    let column = usize::try_from(column).ok()?;
    match encoding {
        Encoding::Utf8 => (column <= text.len()).then_some(start + column),
        Encoding::Utf16 => {
            let text = std::str::from_utf8(text).ok()?;
            let mut units = 0;
            for (at, c) in text.char_indices() {
                if units == column {
                    return Some(start + at);
                }
                units += c.len_utf16();
                if units > column {
                    return None;
                }
            }
            (units == column).then_some(start + text.len())
        }
    }
}

/// Whether `inner` lies within `outer`.
fn within(inner: Node<'_>, outer: Node<'_>) -> bool {
    outer.start_byte() <= inner.start_byte() && inner.end_byte() <= outer.end_byte()
}

/// The child of `node` named `name`: by field, else the first child of
/// that kind.
fn child<'t>(node: Node<'t>, name: &str) -> Option<Node<'t>> {
    node.child_by_field_name(name).or_else(|| {
        let mut cursor = node.walk();
        node.children(&mut cursor).find(|c| c.kind() == name)
    })
}

/// Whether entering `node` through its child `from` counts for `counted`.
fn counts(node: Node<'_>, from: Node<'_>, counted: Counted) -> bool {
    let through = |fields: &[&str]| {
        fields
            .iter()
            .any(|field| node.child_by_field_name(field) == Some(from))
    };
    match counted {
        Counted::All => true,
        Counted::Except(fields) => !through(fields),
        Counted::Only(fields) => through(fields),
    }
}

/// `text` without a string literal's prefix letters, quotes and Rust raw
/// hashes.
fn unquote(text: &str) -> &str {
    let text = text.trim_start_matches(|c: char| c.is_ascii_alphabetic());
    let hashes = text.len() - text.trim_start_matches('#').len();
    let text = text
        .get(hashes..text.len().saturating_sub(hashes))
        .unwrap_or(text);
    for quote in ["\"\"\"", "'''", "\"", "'", "`"] {
        if let Some(inner) = text
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    text
}

/// The constant string literals among the direct arguments of `call`.
fn literals(language: Language, call: Node<'_>, arguments: &str, source: &[u8]) -> Vec<String> {
    let table = table(language);
    let Some(arguments) = child(call, arguments) else {
        return Vec::new();
    };
    let mut cursor = arguments.walk();
    arguments
        .named_children(&mut cursor)
        .filter(|node| table.strings.contains(&node.kind()))
        .filter(|node| {
            let mut cursor = node.walk();
            !node
                .named_children(&mut cursor)
                .any(|c| table.interpolations.contains(&c.kind()))
        })
        .filter_map(|node| node.utf8_text(source).ok())
        .map(|text| unquote(text).to_string())
        .collect()
}

/// The syntax around the call site whose name starts at byte `offset` of
/// `source`, parsed as `tree`; `None` when no named node starts there.
pub fn site_syntax(
    language: Language,
    tree: &Tree,
    source: &[u8],
    offset: usize,
) -> Option<SiteSyntax> {
    let table = table(language);
    let name = tree
        .root_node()
        .named_descendant_for_byte_range(offset, offset)?;
    if name.start_byte() != offset {
        return None;
    }
    let mut syntax = SiteSyntax::default();
    // Only the innermost call around the name decides: in a chain such as
    // `xs.map(f).collect()`, the outer call's callee spans `map(f)`, so `f`
    // would wrongly look like its callee.
    let mut call_decided = false;
    let mut from = name;
    let mut node = name.parent();
    while let Some(current) = node {
        let kind = current.kind();
        if table.definitions.contains(&kind) {
            break;
        }
        if !call_decided
            && let Some(&(call, callee, arguments)) =
                table.calls.iter().find(|(call, ..)| *call == kind)
        {
            call_decided = true;
            let callee = callee.and_then(|field| current.child_by_field_name(field));
            if callee.is_some_and(|callee| within(name, callee)) {
                syntax.call = Some(call);
                syntax.literals = literals(language, current, arguments, source);
            }
        }
        if let Some(&(_, control, counted)) =
            table.controls.iter().find(|(control, ..)| *control == kind)
            && counts(current, from, counted)
        {
            syntax.control.push(control);
        }
        from = current;
        node = current.parent();
    }
    Some(syntax)
}

/// The syntax around the site at 0-based `line`, `column` (in `encoding`)
/// of `source`: [`byte_offset`], then [`site_syntax`].
pub fn site_syntax_at(
    language: Language,
    tree: &Tree,
    source: &[u8],
    line: u32,
    column: u32,
    encoding: Encoding,
) -> Option<SiteSyntax> {
    let offset = byte_offset(source, line, column, encoding)?;
    site_syntax(language, tree, source, offset)
}

#[cfg(test)]
mod tests;
