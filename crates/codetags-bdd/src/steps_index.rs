//! Steps for `features/index/`: SCIP providers over the fixtures (PLAN.md §9).
//!
//! Symbols are named by their descriptors: the SCIP symbol without its
//! scheme, manager, package and version, e.g. `charge/double().`. Lines are
//! 1-based.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use codetags_ingest::provider::go::CallEdge;
use codetags_ingest::provider::rust::RustAnalyzer;
use codetags_ingest::provider::scip::{Document, Occurrence, ScipIndex, SymbolKind};
use cucumber::{then, when};
use regex::Regex;

use crate::CodetagsWorld;

/// The outcome of a provider run: the decoded index, or the error's message.
pub(crate) type RunOutcome = Arc<Result<ScipIndex, String>>;

/// State the index steps share within one scenario.
#[derive(Debug, Default)]
pub struct IndexState {
    /// The last provider run.
    pub(crate) run: Option<RunOutcome>,
    /// The call graph of the last provider run, for providers that write one
    /// (Go, `steps_index_go`).
    pub(crate) call_graph: Option<Arc<Vec<CallEdge>>>,
    /// The root the last Go provider run indexed.
    pub(crate) root: Option<PathBuf>,
}

/// Fixture runs, shared by the scenarios of one process: the index of a
/// fixture is deterministic, and each run takes several seconds.
fn fixture_runs() -> &'static Mutex<HashMap<String, RunOutcome>> {
    static RUNS: OnceLock<Mutex<HashMap<String, RunOutcome>>> = OnceLock::new();
    RUNS.get_or_init(Default::default)
}

/// `tests/fixtures/<name>`, without `..` components (the provider gets an
/// absolute, normalized root on every OS).
pub(crate) fn fixture(name: &str) -> PathBuf {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/codetags-bdd sits two levels below the repo root");
    repo.join("tests").join("fixtures").join(name)
}

/// Runs rust-analyzer over `root`, with its output and cargo's target
/// directory in `scratch`, so nothing is written into the project.
fn run_rust(root: &Path, scratch: &Path) -> RunOutcome {
    let provider = RustAnalyzer::new().env("CARGO_TARGET_DIR", scratch.join("target"));
    Arc::new(
        provider
            .index(root, &scratch.join("scip"))
            .map(|run| run.index)
            .map_err(|error| error.to_string()),
    )
}

pub(crate) fn index(world: &CodetagsWorld) -> &ScipIndex {
    let run = world
        .index
        .run
        .as_ref()
        .expect("an earlier step ran a provider");
    match run.as_ref() {
        Ok(index) => index,
        Err(error) => panic!("the provider run failed: {error}"),
    }
}

pub(crate) fn document<'a>(index: &'a ScipIndex, path: &str) -> &'a Document {
    index
        .documents
        .iter()
        .find(|document| document.relative_path == path)
        .unwrap_or_else(|| panic!("no document {path:?}"))
}

/// The descriptors of a global SCIP symbol; `None` for a local.
pub(crate) fn descriptors(symbol: &str) -> Option<&str> {
    if symbol.starts_with("local ") {
        return None;
    }
    symbol.splitn(5, ' ').nth(4)
}

pub(crate) fn definition<'a>(index: &'a ScipIndex, name: &str) -> &'a Occurrence {
    index
        .documents
        .iter()
        .flat_map(|document| &document.occurrences)
        .find(|occurrence| {
            occurrence.is_definition() && descriptors(&occurrence.symbol) == Some(name)
        })
        .unwrap_or_else(|| panic!("no definition of {name:?}"))
}

#[when(expr = "the Rust provider indexes the fixture {string}")]
fn rust_indexes_fixture(world: &mut CodetagsWorld, name: String) {
    let cached = fixture_runs()
        .lock()
        .expect("fixture cache lock")
        .get(&name)
        .cloned();
    let run = match cached {
        Some(run) => run,
        None => {
            let run = run_rust(&fixture(&name), world.scratch());
            fixture_runs()
                .lock()
                .expect("fixture cache lock")
                .insert(name, run.clone());
            run
        }
    };
    world.index.run = Some(run);
}

#[when(expr = "the Rust provider indexes a directory whose Cargo.toml is {string}")]
fn rust_indexes_manifest(world: &mut CodetagsWorld, manifest: String) {
    let scratch = world.scratch().to_path_buf();
    let root = scratch.join("project");
    std::fs::create_dir_all(&root).expect("create project dir");
    std::fs::write(root.join("Cargo.toml"), manifest).expect("write Cargo.toml");
    world.index.run = Some(run_rust(&root, &scratch));
}

#[when(expr = "the Rust provider indexes a directory that does not exist")]
fn rust_indexes_nothing(world: &mut CodetagsWorld) {
    let scratch = world.scratch().to_path_buf();
    world.index.run = Some(run_rust(&scratch.join("no-such-project"), &scratch));
}

#[then(expr = "the provider run succeeds")]
fn run_succeeds(world: &mut CodetagsWorld) {
    index(world);
}

#[then(expr = "the provider run fails with stderr matching {string}")]
fn run_fails(world: &mut CodetagsWorld, pattern: String) {
    let run = world
        .index
        .run
        .as_ref()
        .expect("an earlier step ran a provider");
    let error = match run.as_ref() {
        Ok(_) => panic!("the provider run succeeded"),
        Err(error) => error,
    };
    let (_, stderr) = error
        .split_once("--- provider stderr ---")
        .unwrap_or_else(|| panic!("the error does not carry the provider's stderr: {error}"));
    let regex = Regex::new(&pattern).expect("valid regex");
    assert!(
        regex.is_match(stderr),
        "stderr does not match {pattern:?}: {error}"
    );
}

#[then(expr = "the index documents are exactly {string}")]
fn documents_are(world: &mut CodetagsWorld, expected: String) {
    let mut paths: Vec<&str> = index(world)
        .documents
        .iter()
        .map(|document| document.relative_path.as_str())
        .collect();
    paths.sort_unstable();
    let mut expected: Vec<&str> = expected.split_whitespace().collect();
    expected.sort_unstable();
    assert_eq!(paths, expected);
}

#[then(expr = "every definition of a function or method has an enclosing range")]
fn functions_have_enclosing_ranges(world: &mut CodetagsWorld) {
    let index = index(world);
    let mut checked = 0;
    for document in &index.documents {
        for info in &document.symbols {
            let callable = matches!(
                info.kind,
                Some(SymbolKind::Function | SymbolKind::Method | SymbolKind::TraitMethod)
            );
            if !callable {
                continue;
            }
            let definition = document
                .occurrences
                .iter()
                .find(|o| o.is_definition() && o.symbol == info.symbol)
                .unwrap_or_else(|| panic!("{} has no definition occurrence", info.symbol));
            assert!(
                definition.enclosing_range.is_some(),
                "{} has no enclosing range",
                info.symbol
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "the index defines no functions or methods");
}

#[then(expr = "the definition of {string} encloses lines {int} to {int}")]
fn definition_encloses(world: &mut CodetagsWorld, name: String, first: u32, last: u32) {
    let enclosing = definition(index(world), &name)
        .enclosing_range
        .unwrap_or_else(|| panic!("{name:?} has no enclosing range"));
    assert_eq!(
        (enclosing.start_line + 1, enclosing.end_line + 1),
        (first, last),
        "enclosing range of {name:?}"
    );
}

#[then(expr = "{string} occurs in {string} on line {int}")]
fn occurs_on_line(world: &mut CodetagsWorld, name: String, path: String, line: u32) {
    let document = document(index(world), &path);
    let found = document.occurrences.iter().any(|occurrence| {
        !occurrence.is_definition()
            && descriptors(&occurrence.symbol) == Some(name.as_str())
            && occurrence.range.start_line + 1 == line
    });
    assert!(found, "no reference to {name:?} in {path} on line {line}");
}

#[then(expr = "no symbol in the index has a relationship")]
fn no_relationships(world: &mut CodetagsWorld) {
    for document in &index(world).documents {
        for info in &document.symbols {
            assert!(
                info.relationships.is_empty(),
                "{} has relationships {:?}",
                info.symbol,
                info.relationships
            );
        }
    }
}

#[then(expr = "the document {string} has occurrences of local symbols")]
fn has_locals(world: &mut CodetagsWorld, path: String) {
    let document = document(index(world), &path);
    assert!(
        document.occurrences.iter().any(Occurrence::is_local),
        "{path} has no local occurrences"
    );
}

#[then(expr = "filtering local symbols keeps every other occurrence in {string}")]
fn filtering_keeps_others(world: &mut CodetagsWorld, path: String) {
    let original = document(index(world), &path);
    let mut filtered = original.clone();
    filtered.drop_locals();
    let expected: Vec<&Occurrence> = original
        .occurrences
        .iter()
        .filter(|o| !o.is_local())
        .collect();
    assert!(!expected.is_empty(), "{path} has only local occurrences");
    assert_eq!(filtered.occurrences.iter().collect::<Vec<_>>(), expected);
    assert!(
        filtered
            .symbols
            .iter()
            .all(|info| !info.symbol.starts_with("local "))
    );
}
