//! SCIP ingest into a generation (brief §4.4 rules, PLAN.md §2.2, §2.4).
//!
//! [`ingest`] writes one provider run's SCIP index into the generation a
//! [`GenerationWriter`](codetags_model::GenerationWriter) is writing: `file`,
//! `symbol`, `call_site` and `call_target` rows, a `run` row and its
//! per-file edge counts in `run_file`. Several runs may share a generation.
//!
//! # Policies
//!
//! - **Symbols.** Every symbol the project defines (a definition occurrence,
//!   or `SymbolInformation` in a document), plus every symbol a call site
//!   targets. Each has its canonical name (PLAN.md §2.7, D15) and its dotted
//!   module ancestors. Two symbols with one canonical name are both kept,
//!   and reported in [`IngestReport::collisions`]; nothing is merged.
//! - **Locals.** `local N` symbols, and parameters, are the discarded local
//!   scope (brief §4.4): their occurrences are dropped and counted. A call
//!   through a closure or a function pointer references a local, so it is
//!   lost with it (V65).
//! - **Attribution.** A reference is attributed to the innermost definition
//!   whose enclosing range contains it, using **definition** occurrences'
//!   enclosing ranges only: rust-analyzer gives a reference the *target's*
//!   range (V65). Modules are not callers, so a reference inside nothing but
//!   a module (an import, a re-export) is not a call site; it is counted as
//!   outside a definition.
//! - **Call sites** are references to callables: functions, methods and
//!   macros (by the provider's kind, or by descriptor for symbols defined
//!   elsewhere). Types, fields and modules are not. A call site's `ref_kind`
//!   is `macro` for a macro, `call` when an argument list follows the name in
//!   the source, and `value` otherwise: a function used as a value (a
//!   callback, `map(double)`, `let f: fn() = g`), which SCIP's roles alone
//!   cannot tell from a call (V65).
//! - **Operators.** rust-analyzer writes `+`, `-` and `%` as references to
//!   `core` impls, three one-character occurrences per operator (V65). A
//!   reference whose source text holds no letter, digit or `_` is not a name,
//!   so it is dropped and counted as an operator reference. Calls inside a
//!   macro's body have no occurrences at all and are lost (V65).
//! - **External symbols.** References to symbols of other packages (the
//!   standard library, dependencies) are kept: calls into a dependency are
//!   part of a module's architecture. Their `symbol` rows have no file and
//!   `external = true`. A symbol of the project's own package that has no
//!   definition (rust-analyzer's derived impls, V65) is not external.
//! - **Targets.** Each call site has exactly one `call_target`, its declared
//!   target, with `method = declared`. Implementations of trait methods are
//!   not guessed: rust-analyzer writes no `is_implementation` relationships
//!   (V64), and expanding them waits on a decision.
//! - **Dispatch** comes from what SCIP gives, and never claims more (see
//!   [`Dispatch`]): `static` for a free function, an impl block's method or a
//!   macro; `virtual` for a trait's method; never `dynamic`; and `unknown`
//!   for every scheme but rust-analyzer's.
//! - **Provenance.** Every call site's `source` is `scip@<tool>-<version>`,
//!   e.g. `scip@rust-analyzer-1.96.1`; the run's full version string is in
//!   `run.provider_version`.
//!
//! Not done yet: `signature` and `receiver_text`. The brief's control
//! context and literal names per call site, which SCIP does not carry, are
//! task P1.3b.

mod analyze;
mod rules;
mod source;
mod write;

use std::fmt;
use std::path::Path;
use std::time::SystemTime;

use codetags_model::Connection;
use codetags_names::canonical::Collision;

pub use self::analyze::RefKind;
pub use self::rules::Dispatch;
use crate::provider::scip::ScipIndex;

/// What to ingest.
#[derive(Debug)]
pub struct Input<'a> {
    /// The project root the index was made from; sources are read from it.
    pub root: &'a Path,
    /// The provider's index.
    pub index: &'a ScipIndex,
    /// The language of every document, e.g. `rust`.
    pub language: &'a str,
    /// The provider run that produced the index.
    pub run: RunInfo,
}

/// A provider run, recorded in the generation's `run` table.
#[derive(Debug, Clone)]
pub struct RunInfo {
    /// The provider, e.g. `rust-analyzer scip`.
    pub provider: String,
    /// The provider's full version.
    pub provider_version: String,
    /// The provider's arguments.
    pub args: Vec<String>,
    /// When the provider started.
    pub started_at: SystemTime,
    /// When it finished.
    pub finished_at: SystemTime,
}

/// What an ingest wrote and skipped.
#[derive(Debug, Clone, Default)]
pub struct IngestReport {
    /// The run's id in the generation.
    pub run_id: i64,
    /// The provenance written on every call site, e.g.
    /// `scip@rust-analyzer-1.96.1`.
    pub source: String,
    /// `file` rows written.
    pub files: usize,
    /// `symbol` rows written.
    pub symbols: usize,
    /// `call_site` rows written, each with one declared `call_target`.
    pub call_sites: usize,
    /// Occurrences that are not symbols or call sites, by reason.
    pub skipped: Skipped,
    /// Canonical names shared by more than one symbol. Every symbol keeps its
    /// own row; nothing is merged.
    pub collisions: Vec<Collision>,
}

/// Occurrences that did not become symbols or call sites, by reason.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Skipped {
    /// Occurrences of `local N` symbols and parameters, definitions
    /// included.
    pub local: usize,
    /// References whose source text is not a name: operators (V65).
    pub operator: usize,
    /// References to symbols that are not callable (types, fields, modules).
    pub non_callable: usize,
    /// References to callables outside every non-module definition: imports
    /// and re-exports.
    pub outside_definition: usize,
}

/// Why an ingest failed.
#[derive(Debug)]
pub enum IngestError {
    /// DuckDB reported an error.
    Duckdb(codetags_model::Error),
    /// A source file could not be read.
    Source {
        /// The file, relative to the root.
        path: String,
        /// The underlying error.
        source: std::io::Error,
    },
    /// A SCIP symbol does not parse, or has no canonical spelling.
    Symbol {
        /// The symbol.
        symbol: String,
        /// Why.
        message: String,
    },
}

impl fmt::Display for IngestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duckdb(error) => write!(f, "duckdb: {error}"),
            Self::Source { path, source } => write!(f, "reading {path}: {source}"),
            Self::Symbol { symbol, message } => write!(f, "symbol {symbol:?}: {message}"),
        }
    }
}

impl std::error::Error for IngestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Duckdb(error) => Some(error),
            Self::Source { source, .. } => Some(source),
            Self::Symbol { .. } => None,
        }
    }
}

impl From<codetags_model::Error> for IngestError {
    fn from(error: codetags_model::Error) -> Self {
        Self::Duckdb(error)
    }
}

/// The provenance of a call site from the indexer `tool` (`<name> <version>
/// ...`, as SCIP metadata gives it): `scip@<name>-<version>`.
pub fn source_tag(tool: &str) -> String {
    let mut words = tool.split_whitespace();
    match (words.next(), words.next()) {
        (Some(name), Some(version)) => format!("scip@{name}-{version}"),
        (Some(name), None) => format!("scip@{name}"),
        _ => "scip@unknown".to_string(),
    }
}

/// Ingests `input` into the generation `db` is writing, as one run, in one
/// transaction.
pub fn ingest(db: &Connection, input: &Input<'_>) -> Result<IngestReport, IngestError> {
    let analysis = analyze::analyze(input.index, |path| {
        std::fs::read(input.root.join(path)).map_err(|source| IngestError::Source {
            path: path.to_string(),
            source,
        })
    })?;
    let source = source_tag(&input.index.tool);
    let run_id = write::write(db, &analysis, &input.run, input.language, &source)?;
    Ok(IngestReport {
        run_id,
        source,
        files: analysis.files.len(),
        symbols: analysis.symbols.len(),
        call_sites: analysis.sites.len(),
        skipped: analysis.skipped,
        collisions: analysis.collisions,
    })
}

#[cfg(test)]
mod tests;
