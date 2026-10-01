//! The Go provider (brief §4.4 Go row): `scip-go` for symbols and
//! references, and `tools/gocallgraph` for call edges keyed by call site.
//!
//! Both programs are started by name from `PATH`; `just setup-providers`
//! installs the versions `providers.toml` pins. scip-go writes
//! `index.scip`, checked as every SCIP index is ([`super`]). gocallgraph
//! writes JSON lines, one [`CallEdge`] each, which this runner parses.
//!
//! Failures are loud and carry the failing program's stderr: either program
//! failing to start or exiting non-zero, a missing index or edge file, an
//! edge line that does not parse, and an empty call graph for code whose
//! index references functions. scip-go exits 0 on code that does not
//! type-check (V79), so gocallgraph, which does not, is what catches it.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde::Deserialize;

use super::scip::{ScipIndex, SymbolKind};
use super::{ProviderError, ScipRun, check_output};

/// The file name the runner asks scip-go to write in the output directory.
pub const INDEX_FILE: &str = "index.scip";

/// The file name the runner asks gocallgraph to write in the output
/// directory.
pub const CALL_GRAPH_FILE: &str = "callgraph.jsonl";

/// How a call names its callee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallKind {
    /// The call names its callee (including a function literal or a method
    /// value bound where it is called).
    Static,
    /// An interface method or a function value: the callee is one of the
    /// site's edges.
    Dynamic,
}

/// The analysis that produced an edge (brief §4.4: VTA, CHA for libraries
/// without `main`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallAlgorithm {
    /// Variable type analysis, for programs with a `main` package.
    Vta,
    /// Class hierarchy analysis.
    Cha,
}

impl CallKind {
    /// The name gocallgraph writes.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Dynamic => "dynamic",
        }
    }
}

impl CallAlgorithm {
    /// The name gocallgraph writes.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vta => "vta",
            Self::Cha => "cha",
        }
    }
}

/// A position in a source file: a path relative to the module root with `/`
/// separators, a 1-based line, and a 1-based byte column.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourcePosition {
    /// The file, relative to the module root.
    pub file: String,
    /// 1-based line.
    pub line: u32,
    /// 1-based byte column.
    pub column: u32,
}

/// One call-graph edge. A polymorphic call has one edge per callee at the
/// same site.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CallEdge {
    /// The calling function as go/ssa names it, e.g.
    /// `example.com/shop/internal/pay.Settle`; a function literal is
    /// `<enclosing>$<n>`, and a generic function's instances are the generic
    /// function.
    pub caller: String,
    /// The called function, named the same way, e.g.
    /// `(example.com/shop/internal/pay.Card).Charge`.
    pub callee: String,
    /// The call site: where the called name starts (the selector of `x.f()`,
    /// the identifier of `f()`), which is where scip-go puts its reference.
    pub site: SourcePosition,
    /// Whether the call names its callee.
    pub kind: CallKind,
    /// The analysis that found the edge.
    pub algorithm: CallAlgorithm,
    /// Where the callee's name is declared, when it is in the module.
    pub callee_definition: Option<SourcePosition>,
}

/// One JSON line as gocallgraph writes it.
#[derive(Deserialize)]
struct RawEdge {
    caller: String,
    callee: String,
    file: String,
    line: u32,
    column: u32,
    kind: CallKind,
    algorithm: CallAlgorithm,
    callee_file: Option<String>,
    callee_line: Option<u32>,
    callee_column: Option<u32>,
}

impl From<RawEdge> for CallEdge {
    fn from(raw: RawEdge) -> Self {
        let callee_definition = match (raw.callee_file, raw.callee_line, raw.callee_column) {
            (Some(file), Some(line), Some(column)) => Some(SourcePosition { file, line, column }),
            _ => None,
        };
        Self {
            caller: raw.caller,
            callee: raw.callee,
            site: SourcePosition {
                file: raw.file,
                line: raw.line,
                column: raw.column,
            },
            kind: raw.kind,
            algorithm: raw.algorithm,
            callee_definition,
        }
    }
}

/// A Go provider run that passed every check.
#[derive(Debug)]
pub struct GoRun {
    /// scip-go's index.
    pub scip: ScipRun,
    /// The edge file gocallgraph wrote, inside the output directory.
    pub call_graph_path: PathBuf,
    /// The parsed edges, in gocallgraph's order (by site, then caller and
    /// callee).
    pub edges: Vec<CallEdge>,
    /// gocallgraph's stderr.
    pub call_graph_stderr: String,
}

/// Why a Go provider run failed. Every variant from a program that ran
/// carries its stderr.
#[derive(Debug)]
pub enum GoError {
    /// scip-go failed, or its index failed the checks.
    Scip(ProviderError),
    /// gocallgraph failed to start, exited non-zero, or wrote no edge file.
    CallGraph(ProviderError),
    /// A line of the edge file is not a call edge.
    BadEdge {
        /// The 1-based line number.
        line: usize,
        /// Why it did not parse.
        message: String,
        /// gocallgraph's stderr.
        stderr: String,
    },
    /// gocallgraph wrote no edges, but the index references functions.
    NoEdges {
        /// A document that references a function.
        example: String,
        /// gocallgraph's stderr.
        stderr: String,
    },
}

impl GoError {
    /// The failing program's stderr, when it ran.
    pub fn stderr(&self) -> Option<&str> {
        match self {
            Self::Scip(error) | Self::CallGraph(error) => error.stderr(),
            Self::BadEdge { stderr, .. } | Self::NoEdges { stderr, .. } => Some(stderr),
        }
    }
}

impl fmt::Display for GoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let stderr = match self {
            Self::Scip(error) => return write!(f, "scip-go: {error}"),
            Self::CallGraph(error) => return write!(f, "gocallgraph: {error}"),
            Self::BadEdge {
                line,
                message,
                stderr,
            } => {
                write!(f, "gocallgraph: line {line} is not a call edge: {message}")?;
                stderr
            }
            Self::NoEdges { example, stderr } => {
                write!(
                    f,
                    "gocallgraph wrote no call edges, but {example} references functions"
                )?;
                stderr
            }
        };
        write!(f, "\n--- provider stderr ---\n{}", stderr.trim_end())
    }
}

impl std::error::Error for GoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Scip(error) | Self::CallGraph(error) => Some(error),
            _ => None,
        }
    }
}

/// Runs `scip-go` and `gocallgraph`.
#[derive(Debug, Clone)]
pub struct GoProvider {
    scip_go: OsString,
    gocallgraph: OsString,
    envs: Vec<(OsString, OsString)>,
}

impl Default for GoProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl GoProvider {
    /// The `scip-go` and `gocallgraph` found on `PATH`.
    pub fn new() -> Self {
        Self::with_programs("scip-go", "gocallgraph")
    }

    /// Specific executables.
    pub fn with_programs(scip_go: impl Into<OsString>, gocallgraph: impl Into<OsString>) -> Self {
        Self {
            scip_go: scip_go.into(),
            gocallgraph: gocallgraph.into(),
            envs: Vec::new(),
        }
    }

    /// Sets an environment variable for both programs, e.g. `GOFLAGS`.
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.envs
            .push((key.as_ref().to_os_string(), value.as_ref().to_os_string()));
        self
    }

    fn run(&self, program: &OsStr, args: &[&OsStr]) -> Result<Output, ProviderError> {
        Command::new(program)
            .args(args)
            .envs(self.envs.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .output()
            .map_err(|source| ProviderError::Spawn {
                program: program.to_string_lossy().into_owned(),
                source,
            })
    }

    /// `scip-go --version`, e.g. `0.2.7`.
    pub fn scip_go_version(&self) -> Result<String, ProviderError> {
        let output = self.run(&self.scip_go, &[OsStr::new("--version")])?;
        if !output.status.success() {
            return Err(ProviderError::Exit {
                status: output.status,
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Indexes every package of the Go module at `root` (`./...`); see
    /// [`Self::index_packages`].
    pub fn index(&self, root: &Path, out_dir: &Path) -> Result<GoRun, GoError> {
        self.index_packages(root, out_dir, &[])
    }

    /// Indexes the packages `patterns` (relative to `root`; `./...` when
    /// empty) of the Go module at `root` into `out_dir`, creating it if
    /// needed and replacing earlier output there, and checks the result
    /// (see the module docs for what fails).
    pub fn index_packages(
        &self,
        root: &Path,
        out_dir: &Path,
        patterns: &[&str],
    ) -> Result<GoRun, GoError> {
        let index_path = out_dir.join(INDEX_FILE);
        let call_graph_path = out_dir.join(CALL_GRAPH_FILE);
        prepare(out_dir, &[&index_path, &call_graph_path]).map_err(GoError::Scip)?;

        let mut args: Vec<&OsStr> = vec![
            OsStr::new("index"),
            OsStr::new("--module-root"),
            root.as_os_str(),
            OsStr::new("--output"),
            index_path.as_os_str(),
        ];
        args.extend(patterns.iter().map(OsStr::new));
        let output = self.run(&self.scip_go, &args).map_err(GoError::Scip)?;
        let scip = check_exit(output)
            .and_then(|stderr| check_output(root, &index_path, stderr, declares_function))
            .map_err(GoError::Scip)?;

        let mut args: Vec<&OsStr> = vec![
            OsStr::new("-C"),
            root.as_os_str(),
            OsStr::new("-o"),
            call_graph_path.as_os_str(),
        ];
        args.extend(patterns.iter().map(OsStr::new));
        let output = self
            .run(&self.gocallgraph, &args)
            .map_err(GoError::CallGraph)?;
        let stderr = check_exit(output).map_err(GoError::CallGraph)?;
        let Ok(text) = std::fs::read_to_string(&call_graph_path) else {
            return Err(GoError::CallGraph(ProviderError::MissingOutput {
                path: call_graph_path,
                stderr,
            }));
        };
        let edges = parse_edges(&text, &stderr)?;
        check_edges(&edges, &scip.index, &stderr)?;
        Ok(GoRun {
            scip,
            call_graph_path,
            edges,
            call_graph_stderr: stderr,
        })
    }
}

/// Creates `out_dir` and removes earlier output files in it: a stale file
/// must never pass for this run's output.
fn prepare(out_dir: &Path, files: &[&Path]) -> Result<(), ProviderError> {
    let out_error = |source| ProviderError::OutputDir {
        path: out_dir.to_path_buf(),
        source,
    };
    std::fs::create_dir_all(out_dir).map_err(out_error)?;
    for file in files {
        match std::fs::remove_file(file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(out_error(error)),
        }
    }
    Ok(())
}

/// The program's stderr when it exited 0, else the exit error.
fn check_exit(output: Output) -> Result<String, ProviderError> {
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if output.status.success() {
        Ok(stderr)
    } else {
        Err(ProviderError::Exit {
            status: output.status,
            stderr,
        })
    }
}

/// Parses gocallgraph's JSON lines. Blank lines are skipped; any other line
/// that is not an edge is an error.
pub fn parse_edges(text: &str, stderr: &str) -> Result<Vec<CallEdge>, GoError> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(at, line)| {
            serde_json::from_str::<RawEdge>(line)
                .map(CallEdge::from)
                .map_err(|error| GoError::BadEdge {
                    line: at + 1,
                    message: error.to_string(),
                    stderr: stderr.to_string(),
                })
        })
        .collect()
}

/// Fails when `edges` is empty but `index` references a function (brief
/// §4.4: a provider that silently loses its edges must fail).
pub fn check_edges(edges: &[CallEdge], index: &ScipIndex, stderr: &str) -> Result<(), GoError> {
    if !edges.is_empty() {
        return Ok(());
    }
    match references_function(index) {
        Some(example) => Err(GoError::NoEdges {
            example,
            stderr: stderr.to_string(),
        }),
        None => Ok(()),
    }
}

/// The first document that references a function or method, if any. A
/// function is a symbol whose last descriptor is a method (`f().`), or one
/// the index gives a function, method or interface-method kind (scip-go
/// writes an interface method as a term, `Method#Charge.`). A function used
/// as a value counts too.
fn references_function(index: &ScipIndex) -> Option<String> {
    let callable: std::collections::HashSet<&str> = index
        .documents
        .iter()
        .flat_map(|document| &document.symbols)
        .filter(|info| {
            matches!(
                info.kind,
                Some(SymbolKind::Function | SymbolKind::Method | SymbolKind::MethodSpecification)
            )
        })
        .map(|info| info.symbol.as_str())
        .collect();
    index
        .documents
        .iter()
        .find(|document| {
            document.occurrences.iter().any(|occurrence| {
                !occurrence.is_definition()
                    && !occurrence.is_local()
                    && (occurrence.symbol.ends_with("().")
                        || callable.contains(occurrence.symbol.as_str()))
            })
        })
        .map(|document| document.relative_path.clone())
}

/// Whether Go `source` declares a function or method: the keyword `func`
/// followed by whitespace and a name or a receiver. Lexical, so a
/// commented-out declaration counts too; it only decides whether an index
/// with no occurrences at all is suspicious.
pub fn declares_function(source: &str) -> bool {
    let is_ident = |c: char| c == '_' || c.is_alphanumeric();
    source.match_indices("func").any(|(at, _)| {
        let before = source[..at].chars().next_back();
        let after = &source[at + 4..];
        let rest = after.trim_start();
        before.is_none_or(|c| !is_ident(c))
            && after.len() > rest.len()
            && rest
                .chars()
                .next()
                .is_some_and(|c| c == '(' || c == '_' || c.is_alphabetic())
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::super::scip::{Document, Occurrence, Range, SymbolInfo};
    use super::*;

    const SETTLE_CARD: &str = r#"{"caller":"example.com/shop/internal/pay.Settle","callee":"(example.com/shop/internal/pay.Card).Charge","file":"internal/pay/pay.go","line":29,"column":11,"kind":"dynamic","algorithm":"vta","callee_file":"internal/pay/pay.go","callee_line":15,"callee_column":15}"#;
    const PRINTLN: &str = r#"{"caller":"example.com/shop/cmd/shop.main","callee":"fmt.Println","file":"cmd/shop/main.go","line":13,"column":7,"kind":"static","algorithm":"cha"}"#;

    #[test]
    fn edges_parse_with_and_without_a_callee_definition() {
        let edges = parse_edges(&format!("{SETTLE_CARD}\n\n{PRINTLN}\n"), "").unwrap();
        assert_eq!(edges.len(), 2);
        assert_eq!(
            edges[0],
            CallEdge {
                caller: "example.com/shop/internal/pay.Settle".into(),
                callee: "(example.com/shop/internal/pay.Card).Charge".into(),
                site: SourcePosition {
                    file: "internal/pay/pay.go".into(),
                    line: 29,
                    column: 11
                },
                kind: CallKind::Dynamic,
                algorithm: CallAlgorithm::Vta,
                callee_definition: Some(SourcePosition {
                    file: "internal/pay/pay.go".into(),
                    line: 15,
                    column: 15
                }),
            }
        );
        assert_eq!(edges[1].kind, CallKind::Static);
        assert_eq!(edges[1].algorithm, CallAlgorithm::Cha);
        assert_eq!(edges[1].callee_definition, None);
    }

    #[test]
    fn a_line_that_is_not_an_edge_is_an_error_with_its_number() {
        let text = format!("{PRINTLN}\n{{\"caller\":\"x\"}}\n");
        let error = parse_edges(&text, "the stderr").unwrap_err();
        assert!(matches!(error, GoError::BadEdge { line: 2, .. }), "{error}");
        assert!(error.to_string().contains("the stderr"), "{error}");
    }

    #[test]
    fn an_unknown_kind_or_algorithm_is_an_error() {
        for (from, to) in [("\"static\"", "\"virtual\""), ("\"cha\"", "\"rta\"")] {
            let error = parse_edges(&PRINTLN.replace(from, to), "").unwrap_err();
            assert!(matches!(error, GoError::BadEdge { .. }), "{error}");
        }
    }

    fn occurrence(symbol: &str, roles: i32) -> Occurrence {
        Occurrence {
            symbol: symbol.into(),
            range: Range {
                start_line: 0,
                start_character: 0,
                end_line: 0,
                end_character: 1,
            },
            enclosing_range: None,
            roles,
        }
    }

    fn index_with(occurrences: Vec<Occurrence>, symbols: Vec<SymbolInfo>) -> ScipIndex {
        ScipIndex {
            documents: vec![Document {
                relative_path: "main.go".into(),
                occurrences,
                symbols,
            }],
            ..ScipIndex::default()
        }
    }

    const PKG: &str = "scip-go gomod example.com/shop . `example.com/shop/internal/pay`/";

    #[test]
    fn no_edges_where_the_index_references_a_function_is_an_error() {
        let index = index_with(vec![occurrence(&format!("{PKG}Settle()."), 8)], vec![]);
        let error = check_edges(&[], &index, "the stderr").unwrap_err();
        assert!(matches!(error, GoError::NoEdges { .. }), "{error}");
        assert!(error.to_string().contains("main.go"), "{error}");
        assert_eq!(error.stderr(), Some("the stderr"));
    }

    #[test]
    fn a_reference_to_an_interface_method_counts_as_a_function() {
        let method = format!("{PKG}Method#Charge.");
        let info = SymbolInfo {
            symbol: method.clone(),
            kind: Some(SymbolKind::MethodSpecification),
            relationships: vec![],
        };
        let index = index_with(vec![occurrence(&method, 8)], vec![info]);
        assert!(check_edges(&[], &index, "").is_err());
    }

    #[test]
    fn no_edges_without_function_references_passes() {
        let index = index_with(
            vec![
                occurrence(&format!("{PKG}main()."), 1),
                occurrence(&format!("{PKG}Card#Fee."), 8),
                occurrence("local 0", 8),
            ],
            vec![],
        );
        assert!(check_edges(&[], &index, "").is_ok());
    }

    #[test]
    fn edges_pass_whatever_the_index_holds() {
        let index = index_with(vec![occurrence(&format!("{PKG}Settle()."), 8)], vec![]);
        let edges = parse_edges(PRINTLN, "").unwrap();
        assert!(check_edges(&edges, &index, "").is_ok());
    }

    #[test]
    fn function_declarations_are_found() {
        assert!(declares_function("func main() {}"));
        assert!(declares_function("func (c Card) Charge(amount int) int"));
        assert!(declares_function("func\tSum[T Number](xs []T) T"));
    }

    #[test]
    fn non_declarations_are_not() {
        assert!(!declares_function("var f func(int) int"));
        assert!(!declares_function("x := funcs[0]"));
        assert!(!declares_function("// nothing"));
        assert!(!declares_function("func"));
    }

    #[test]
    fn missing_programs_fail_to_spawn_and_name_the_stage() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-program");
        let provider = GoProvider::with_programs(&missing, &missing);
        let error = provider
            .index(dir.path(), &dir.path().join("out"))
            .unwrap_err();
        assert!(
            matches!(error, GoError::Scip(ProviderError::Spawn { .. })),
            "{error}"
        );
        assert!(error.to_string().starts_with("scip-go: "), "{error}");
        assert!(matches!(
            provider.scip_go_version().unwrap_err(),
            ProviderError::Spawn { .. }
        ));
    }

    /// The quoted value of `key = "..."` in `text`, after the line `section`.
    fn toml_string(text: &str, section: &str, key: &str) -> String {
        let mut in_section = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                in_section = line == section;
            } else if in_section
                && let Some((k, v)) = line.split_once('=')
                && k.trim() == key
            {
                return v.trim().trim_matches('"').to_string();
            }
        }
        panic!("no {key} under {section:?}")
    }

    #[test]
    fn gocallgraph_go_mod_requires_the_go_that_providers_toml_pins() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let providers = std::fs::read_to_string(repo.join("providers.toml")).unwrap();
        let go_mod = std::fs::read_to_string(repo.join("tools/gocallgraph/go.mod")).unwrap();
        let go = toml_string(&providers, "[go]", "version");
        let required = go_mod
            .lines()
            .find_map(|line| line.strip_prefix("go "))
            .unwrap()
            .trim();
        let minor = |v: &str| v.split('.').take(2).collect::<Vec<_>>().join(".");
        assert_eq!(minor(&go), minor(required), "providers.toml [go] version");
        let tools = toml_string(&providers, "[gocallgraph]", "x-tools");
        assert!(
            go_mod.contains(&format!("golang.org/x/tools {tools}")),
            "tools/gocallgraph/go.mod pins x/tools {tools}"
        );
    }
}
