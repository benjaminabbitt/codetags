//! Turning a SCIP index into the rows of one run (pure: no database).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use codetags_names::canonical::{CanonicalNames, Collision, is_rust_impl_block};
use codetags_names::scip::{GlobalSymbol, Symbol, parse};

use super::rules::{self, Dispatch};
use super::source::Source;
use super::{IngestError, Skipped};
use crate::provider::scip::{Occurrence, Range, ScipIndex, SymbolKind};

/// How a call site uses its target (the `ref_kind` column).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    /// Followed by an argument list.
    Call,
    /// Used as a value: a callback, a function pointer.
    Value,
    /// A macro invocation.
    Macro,
}

impl RefKind {
    /// The column value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Value => "value",
            Self::Macro => "macro",
        }
    }
}

/// A `file` row.
#[derive(Debug)]
pub(crate) struct FileRow {
    pub path: String,
    pub contents: Vec<u8>,
}

/// A `symbol` row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SymbolRow {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub file: Option<String>,
    /// 1-based first and last lines.
    pub lines: Option<(u32, u32)>,
    pub modules: Vec<String>,
    pub package: String,
    pub external: bool,
}

/// A `call_site` row, with its declared `call_target`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SiteRow {
    pub caller: String,
    pub file: String,
    /// 1-based.
    pub line: u32,
    /// 1-based.
    pub col: u32,
    /// 1-based position among the caller's sites.
    pub ordinal: u32,
    pub ref_kind: RefKind,
    pub target: String,
    pub dispatch: Dispatch,
}

/// The rows of one run.
#[derive(Debug, Default)]
pub(crate) struct Analysis {
    pub files: Vec<FileRow>,
    pub symbols: Vec<SymbolRow>,
    pub sites: Vec<SiteRow>,
    pub skipped: Skipped,
    pub collisions: Vec<Collision>,
}

/// What the index says about a symbol defined in it.
#[derive(Debug, Default)]
struct Definition {
    file: String,
    lines: Option<(u32, u32)>,
    kind: Option<SymbolKind>,
}

/// A definition that references may be attributed to.
#[derive(Debug)]
struct Caller<'a> {
    symbol: &'a str,
    enclosing: Range,
}

/// Parsed global symbols, by SCIP string. `None` for a symbol with no
/// canonical name (a local or a parameter).
#[derive(Default)]
struct Parsed(HashMap<String, Option<GlobalSymbol>>);

impl Parsed {
    fn get(&mut self, symbol: &str) -> Result<Option<&GlobalSymbol>, IngestError> {
        if !self.0.contains_key(symbol) {
            let parsed = match parse(symbol).map_err(|source| IngestError::Symbol {
                symbol: symbol.to_string(),
                message: source.to_string(),
            })? {
                Symbol::Global(global) if !has_parameter(&global) => Some(global),
                _ => None,
            };
            self.0.insert(symbol.to_string(), parsed);
        }
        Ok(self.0.get(symbol).and_then(Option::as_ref))
    }
}

/// A parameter descriptor anywhere makes a symbol a parameter, scoped like a
/// local (`codetags_names::canonical`).
fn has_parameter(symbol: &GlobalSymbol) -> bool {
    symbol
        .descriptors
        .iter()
        .any(|d| d.suffix == codetags_names::scip::Suffix::Parameter)
}

fn package_key(symbol: &GlobalSymbol) -> (String, String) {
    (symbol.package.manager.clone(), symbol.package.name.clone())
}

/// 1-based first and last lines of a range.
fn lines(range: &Range) -> (u32, u32) {
    (range.start_line + 1, range.end_line + 1)
}

/// The innermost caller whose enclosing range contains `range`: the latest
/// start, then the earliest end, then the smallest symbol.
fn innermost<'a>(callers: &[Caller<'a>], range: &Range) -> Option<&'a str> {
    callers
        .iter()
        .filter(|caller| caller.enclosing.contains(range))
        .max_by(|a, b| {
            let start = |c: &Caller<'_>| (c.enclosing.start_line, c.enclosing.start_character);
            let end = |c: &Caller<'_>| (c.enclosing.end_line, c.enclosing.end_character);
            start(a)
                .cmp(&start(b))
                .then(end(b).cmp(&end(a)))
                .then(b.symbol.cmp(a.symbol))
        })
        .map(|caller| caller.symbol)
}

/// Analyses `index`, reading each document's source with `read`.
pub(crate) fn analyze(
    index: &ScipIndex,
    mut read: impl FnMut(&str) -> Result<Vec<u8>, IngestError>,
) -> Result<Analysis, IngestError> {
    let mut parsed = Parsed::default();
    let mut analysis = Analysis::default();

    // Definitions: SymbolInformation says which document defines a symbol;
    // the definition occurrence gives its lines.
    let mut defined: BTreeMap<String, Definition> = BTreeMap::new();
    for document in &index.documents {
        for info in &document.symbols {
            if parsed.get(&info.symbol)?.is_none() {
                continue;
            }
            let entry = defined.entry(info.symbol.clone()).or_default();
            if entry.file.is_empty() {
                entry.file = document.relative_path.clone();
            }
            entry.kind = entry.kind.or(info.kind);
        }
        for occurrence in &document.occurrences {
            if !occurrence.is_definition() || parsed.get(&occurrence.symbol)?.is_none() {
                continue;
            }
            let entry = defined.entry(occurrence.symbol.clone()).or_default();
            if entry.lines.is_none() {
                entry.file = document.relative_path.clone();
                let extent = occurrence.enclosing_range.unwrap_or(occurrence.range);
                entry.lines = Some(lines(&extent));
            }
        }
    }
    let mut project_packages = BTreeSet::new();
    for symbol in defined.keys() {
        if let Some(global) = parsed.get(symbol)? {
            project_packages.insert(package_key(global));
        }
    }
    let kind_of = |symbol: &str| defined.get(symbol).and_then(|d| d.kind);

    let mut targets: BTreeSet<String> = BTreeSet::new();
    for document in &index.documents {
        let path = document.relative_path.as_str();
        let contents = read(path)?;
        let source = Source::new(&contents);
        analysis.files.push(FileRow {
            path: path.to_string(),
            contents,
        });

        let mut callers = Vec::new();
        for occurrence in &document.occurrences {
            let Some(enclosing) = occurrence.enclosing_range else {
                continue;
            };
            if !occurrence.is_definition() {
                continue;
            }
            if let Some(global) = parsed.get(&occurrence.symbol)?
                && !rules::is_module(global, kind_of(&occurrence.symbol))
                && !is_rust_impl_block(global)
            {
                callers.push(Caller {
                    symbol: occurrence.symbol.as_str(),
                    enclosing,
                });
            }
        }

        for occurrence in &document.occurrences {
            let Some(target) = parsed.get(&occurrence.symbol)? else {
                analysis.skipped.local += 1;
                continue;
            };
            if occurrence.is_definition() {
                continue;
            }
            if source.is_name(&occurrence.range) == Some(false) {
                analysis.skipped.operator += 1;
                continue;
            }
            let info = kind_of(&occurrence.symbol);
            if !rules::is_callable(target, info) {
                analysis.skipped.non_callable += 1;
                continue;
            }
            let Some(caller) = innermost(&callers, &occurrence.range) else {
                analysis.skipped.outside_definition += 1;
                continue;
            };
            analysis
                .sites
                .push(site(path, caller, occurrence, target, info, &source));
            targets.insert(occurrence.symbol.clone());
        }
    }
    number_sites(&mut analysis.sites);

    let mut names = CanonicalNames::default();
    let ids: BTreeSet<&String> = defined.keys().chain(&targets).collect();
    for id in ids {
        let Some(global) = parsed.get(id)?.cloned() else {
            continue;
        };
        let space = rules::namespace(&global, kind_of(id));
        let symbol = Symbol::Global(global);
        let named = names
            .insert_as(&symbol, space)
            .map_err(|error| IngestError::Symbol {
                symbol: id.clone(),
                message: error.to_string(),
            })?;
        // No canonical name: an impl block, a container rather than a symbol.
        let (true, Symbol::Global(global)) = (named, symbol) else {
            continue;
        };
        let definition = defined.get(id);
        analysis.symbols.push(SymbolRow {
            id: id.clone(),
            // Assigned below, once every symbol is known (the field policy).
            name: String::new(),
            kind: rules::kind(&global, definition.and_then(|d| d.kind)),
            file: definition.map(|d| d.file.clone()),
            lines: definition.and_then(|d| d.lines),
            modules: rules::modules(&global).map_err(|error| IngestError::Symbol {
                symbol: id.clone(),
                message: error.to_string(),
            })?,
            package: global.package.name.clone(),
            external: !project_packages.contains(&package_key(&global)),
        });
    }
    let mut assigned = names.assigned();
    for row in &mut analysis.symbols {
        row.name = assigned.remove(&row.id).unwrap_or_default();
    }
    analysis.collisions = names.collisions();
    Ok(analysis)
}

/// A call site of `target` at `occurrence`, attributed to `caller`.
fn site(
    path: &str,
    caller: &str,
    occurrence: &Occurrence,
    target: &GlobalSymbol,
    info: Option<SymbolKind>,
    source: &Source,
) -> SiteRow {
    let ref_kind = if rules::is_macro(target, info) {
        RefKind::Macro
    } else if source.is_called(&occurrence.range) {
        RefKind::Call
    } else {
        RefKind::Value
    };
    SiteRow {
        caller: caller.to_string(),
        file: path.to_string(),
        line: occurrence.range.start_line + 1,
        col: occurrence.range.start_character + 1,
        ordinal: 0,
        ref_kind,
        target: occurrence.symbol.clone(),
        dispatch: rules::dispatch(target),
    }
}

/// Sorts sites by position and numbers each caller's sites from 1.
fn number_sites(sites: &mut [SiteRow]) {
    sites.sort_by(|a, b| {
        (&a.file, a.line, a.col, &a.target).cmp(&(&b.file, b.line, b.col, &b.target))
    });
    let mut next: HashMap<String, u32> = HashMap::new();
    for site in sites {
        let ordinal = next.entry(site.caller.clone()).or_insert(0);
        *ordinal += 1;
        site.ordinal = *ordinal;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::provider::scip::{Document, SymbolInfo};

    const P: &str = "rust-analyzer cargo demo 0.1.0 ";
    const CORE: &str = "rust-analyzer cargo core https://example.org/core ";
    const DEFINITION: i32 = 1;

    fn range(line: u32, start: u32, end_line: u32, end: u32) -> Range {
        Range {
            start_line: line,
            start_character: start,
            end_line,
            end_character: end,
        }
    }

    fn definition(symbol: &str, at: Range, enclosing: Range) -> Occurrence {
        Occurrence {
            symbol: symbol.to_string(),
            range: at,
            enclosing_range: Some(enclosing),
            roles: DEFINITION,
        }
    }

    fn reference(symbol: &str, at: Range) -> Occurrence {
        Occurrence {
            symbol: symbol.to_string(),
            range: at,
            enclosing_range: None,
            roles: 0,
        }
    }

    fn info(symbol: &str, kind: SymbolKind) -> SymbolInfo {
        SymbolInfo {
            symbol: symbol.to_string(),
            kind: Some(kind),
            relationships: Vec::new(),
        }
    }

    /// The source of `src/a.rs` in the tests, 0-based lines:
    const SOURCE: &str = "\
mod m {
    use crate::f;
    pub fn outer() {
        fn inner() { f(); }
        f(1 + 2);
        let g = f;
        helper::<u8>(x);
    }
}
";

    fn index() -> ScipIndex {
        let module = format!("{P}m/");
        let outer = format!("{P}m/outer().");
        let inner = format!("{P}m/outer().inner().");
        let f = format!("{P}f().");
        let helper = format!("{P}helper().");
        let add = format!("{CORE}ops/impl#[i32][Add]add().");
        ScipIndex {
            tool: "rust-analyzer 1.96.1 (x)".into(),
            project_root: "file:///p".into(),
            external_symbols: vec![],
            documents: vec![Document {
                relative_path: "src/a.rs".into(),
                occurrences: vec![
                    definition(&module, range(0, 4, 0, 5), range(0, 0, 8, 1)),
                    // An import: inside only the module.
                    reference(&f, range(1, 15, 1, 16)),
                    definition(&outer, range(2, 11, 2, 16), range(2, 4, 7, 5)),
                    definition(&inner, range(3, 11, 3, 16), range(3, 8, 3, 27)),
                    reference(&f, range(3, 21, 3, 22)),
                    reference(&f, range(4, 8, 4, 9)),
                    // ` + ` as three one-character occurrences (V65).
                    reference(&add, range(4, 11, 4, 12)),
                    reference(&add, range(4, 12, 4, 13)),
                    reference(&add, range(4, 13, 4, 14)),
                    reference("local 0", range(5, 12, 5, 13)),
                    reference(&f, range(5, 16, 5, 17)),
                    reference(&helper, range(6, 8, 6, 14)),
                    reference("local 1", range(6, 21, 6, 22)),
                ],
                symbols: vec![
                    info(&module, SymbolKind::Module),
                    info(&outer, SymbolKind::Function),
                    info(&inner, SymbolKind::Function),
                ],
            }],
        }
    }

    fn run() -> Analysis {
        analyze(&index(), |path| {
            assert_eq!(path, "src/a.rs");
            Ok(SOURCE.as_bytes().to_vec())
        })
        .unwrap()
    }

    fn short(sites: &[SiteRow]) -> Vec<(String, u32, &'static str, String, u32)> {
        sites
            .iter()
            .map(|s| {
                (
                    s.caller.trim_start_matches(P).to_string(),
                    s.line,
                    s.ref_kind.as_str(),
                    s.target.trim_start_matches(P).to_string(),
                    s.ordinal,
                )
            })
            .collect()
    }

    #[test]
    fn references_go_to_the_innermost_enclosing_definition() {
        let analysis = run();
        assert_eq!(
            short(&analysis.sites),
            [
                ("m/outer().inner().".into(), 4, "call", "f().".into(), 1),
                ("m/outer().".into(), 5, "call", "f().".into(), 1),
                ("m/outer().".into(), 6, "value", "f().".into(), 2),
                ("m/outer().".into(), 7, "call", "helper().".into(), 3),
            ]
        );
    }

    #[test]
    fn skipped_references_are_counted_by_reason() {
        let analysis = run();
        assert_eq!(
            analysis.skipped,
            Skipped {
                local: 2,
                operator: 3,
                non_callable: 0,
                outside_definition: 1,
            }
        );
    }

    #[test]
    fn symbols_cover_definitions_and_targets() {
        let analysis = run();
        let by_name: BTreeMap<&str, &SymbolRow> = analysis
            .symbols
            .iter()
            .map(|s| (s.name.as_str(), s))
            .collect();
        assert_eq!(
            by_name.keys().copied().collect::<Vec<_>>(),
            [
                "demo.f",
                "demo.helper",
                "demo.m",
                "demo.m.outer",
                "demo.m.outer.inner"
            ]
        );
        let outer = by_name["demo.m.outer"];
        assert_eq!(outer.kind, "function");
        assert_eq!(outer.file.as_deref(), Some("src/a.rs"));
        assert_eq!(outer.lines, Some((3, 8)));
        assert_eq!(outer.modules, ["demo.m"]);
        assert!(!outer.external);
        // Defined nowhere, but in the project's package.
        let f = by_name["demo.f"];
        assert_eq!(
            (f.file.as_deref(), f.lines, f.external),
            (None, None, false)
        );
        assert_eq!(f.kind, "function");
        // The operator's target is not a call site, so not a symbol.
        assert!(analysis.symbols.iter().all(|s| !s.id.starts_with(CORE)));
    }

    #[test]
    fn external_targets_are_marked_external() {
        let mut index = index();
        let vec_push = format!("{CORE}vec/impl#[Vec]push().");
        index.documents[0]
            .occurrences
            .push(reference(&vec_push, range(6, 8, 6, 14)));
        let analysis = analyze(&index, |_| Ok(SOURCE.as_bytes().to_vec())).unwrap();
        let push = analysis
            .symbols
            .iter()
            .find(|s| s.id == vec_push)
            .expect("push is a symbol");
        assert!(push.external);
        assert_eq!(push.package, "core");
        assert_eq!(push.name, "core.vec.Vec.push");
        assert_eq!(push.modules, ["core.vec"]);
    }

    #[test]
    fn colliding_names_are_reported_and_both_kept() {
        let mut index = index();
        let other = "rust-analyzer cargo demo 0.2.0 m/outer().";
        index.documents[0]
            .occurrences
            .push(reference(other, range(4, 8, 4, 9)));
        let analysis = analyze(&index, |_| Ok(SOURCE.as_bytes().to_vec())).unwrap();
        assert_eq!(analysis.collisions.len(), 1);
        assert_eq!(analysis.collisions[0].name, "demo.m.outer");
        assert_eq!(
            analysis
                .symbols
                .iter()
                .filter(|s| s.name == "demo.m.outer")
                .count(),
            2
        );
    }

    #[test]
    fn impl_blocks_are_not_symbols_and_getters_keep_the_plain_name() {
        let mut index = index();
        let block = format!("{P}m/impl#[S]");
        let field = format!("{P}m/S#f.");
        let getter = format!("{P}m/impl#[S]f().");
        let document = &mut index.documents[0];
        // The block encloses a reference that lies in no method.
        document
            .occurrences
            .push(definition(&block, range(6, 0, 6, 1), range(6, 0, 7, 0)));
        document
            .occurrences
            .push(definition(&field, range(6, 2, 6, 3), range(6, 2, 6, 3)));
        document
            .occurrences
            .push(definition(&getter, range(6, 4, 6, 5), range(6, 4, 6, 6)));
        document.symbols.push(info(&block, SymbolKind::Struct));
        let analysis = analyze(&index, |_| Ok(SOURCE.as_bytes().to_vec())).unwrap();
        assert!(analysis.symbols.iter().all(|s| s.id != block));
        let name = |id: &str| {
            analysis
                .symbols
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.name.clone())
        };
        assert_eq!(name(&field).as_deref(), Some("demo.m.S.f+field"));
        assert_eq!(name(&getter).as_deref(), Some("demo.m.S.f"));
        assert!(analysis.collisions.is_empty(), "{:?}", analysis.collisions);
        // The block is no caller: `helper` on line 7 is still `outer`'s.
        assert!(analysis.sites.iter().all(|s| s.caller != block));
    }

    #[test]
    fn an_unparsable_symbol_is_an_error() {
        let mut index = index();
        index.documents[0]
            .occurrences
            .push(reference("rust-analyzer cargo", range(0, 0, 0, 1)));
        let error = analyze(&index, |_| Ok(SOURCE.as_bytes().to_vec())).unwrap_err();
        assert!(matches!(error, IngestError::Symbol { .. }), "{error}");
    }
}
