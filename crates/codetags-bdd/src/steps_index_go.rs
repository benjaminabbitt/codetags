//! Steps for `features/index/go.feature` (PLAN.md §9 P1.5): the Go provider
//! (`scip-go` and `gocallgraph`), plus index steps any provider can use.
//!
//! Lines and columns are 1-based. Symbols are named by their descriptors, as
//! in [`crate::steps_index`].

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use codetags_ingest::provider::go::{CallEdge, GoProvider};
use codetags_ingest::provider::scip::{Occurrence, Range};
use codetags_names::canonical::canonical_name;
use cucumber::gherkin::Step;
use cucumber::{then, when};

use crate::CodetagsWorld;
use crate::steps_index::{RunOutcome, definition, descriptors, document, fixture, index};

/// A Go run: the index (or the error), and the call graph when it passed.
type GoOutcome = (RunOutcome, Option<Arc<Vec<CallEdge>>>);

/// Fixture runs by (fixture, pattern), shared by one process's scenarios.
fn go_fixture_runs() -> &'static Mutex<HashMap<(String, String), GoOutcome>> {
    static RUNS: OnceLock<Mutex<HashMap<(String, String), GoOutcome>>> = OnceLock::new();
    RUNS.get_or_init(Default::default)
}

/// Runs the Go provider over `patterns` of `root`, writing into `scratch`.
fn run_go(root: &Path, scratch: &Path, patterns: &[&str]) -> GoOutcome {
    match GoProvider::new().index_packages(root, &scratch.join("go"), patterns) {
        Ok(run) => (Arc::new(Ok(run.scip.index)), Some(Arc::new(run.edges))),
        Err(error) => (Arc::new(Err(error.to_string())), None),
    }
}

fn record(world: &mut CodetagsWorld, root: PathBuf, (run, call_graph): GoOutcome) {
    world.index.set_run(run);
    world.index.call_graph = call_graph;
    world.index.root = Some(root);
}

fn go_indexes_fixture_packages(world: &mut CodetagsWorld, name: String, pattern: Option<String>) {
    let root = fixture(&name);
    let key = (name, pattern.clone().unwrap_or_default());
    let cached = go_fixture_runs()
        .lock()
        .expect("fixture cache lock")
        .get(&key)
        .cloned();
    let outcome = match cached {
        Some(outcome) => outcome,
        None => {
            let patterns: Vec<&str> = pattern.iter().map(String::as_str).collect();
            let outcome = run_go(&root, world.scratch(), &patterns);
            go_fixture_runs()
                .lock()
                .expect("fixture cache lock")
                .insert(key, outcome.clone());
            outcome
        }
    };
    record(world, root, outcome);
}

#[when(expr = "the Go provider indexes the fixture {string}")]
fn go_indexes_fixture(world: &mut CodetagsWorld, name: String) {
    go_indexes_fixture_packages(world, name, None);
}

#[when(expr = "the Go provider indexes the packages {string} of the fixture {string}")]
fn go_indexes_packages(world: &mut CodetagsWorld, pattern: String, name: String) {
    go_indexes_fixture_packages(world, name, Some(pattern));
}

#[when(expr = "the Go provider indexes a module whose main.go is:")]
fn go_indexes_main(world: &mut CodetagsWorld, step: &Step) {
    let source = step
        .docstring
        .as_ref()
        .expect("a docstring holding main.go");
    let scratch = world.scratch().to_path_buf();
    let root = scratch.join("module");
    std::fs::create_dir_all(&root).expect("create module dir");
    std::fs::write(
        root.join("go.mod"),
        "module example.com/broken\n\ngo 1.24\n",
    )
    .expect("write go.mod");
    std::fs::write(root.join("main.go"), source).expect("write main.go");
    let outcome = run_go(&root, &scratch, &[]);
    record(world, root, outcome);
}

#[when(expr = "the Go provider indexes a directory that does not exist")]
fn go_indexes_nothing(world: &mut CodetagsWorld) {
    let scratch = world.scratch().to_path_buf();
    let root = scratch.join("no-such-module");
    let outcome = run_go(&root, &scratch, &[]);
    record(world, root, outcome);
}

/// Global references in a document (not definitions, not locals).
fn references(occurrences: &[Occurrence]) -> impl Iterator<Item = &Occurrence> {
    occurrences
        .iter()
        .filter(|occurrence| !occurrence.is_definition() && !occurrence.is_local())
}

/// The definitions in `occurrences` that have an enclosing range.
fn enclosing_definitions(occurrences: &[Occurrence]) -> impl Iterator<Item = (&Occurrence, Range)> {
    occurrences.iter().filter_map(|occurrence| {
        occurrence
            .enclosing_range
            .filter(|_| occurrence.is_definition())
            .map(|range| (occurrence, range))
    })
}

#[then(expr = "every document has a reference within a definition's enclosing range")]
fn every_document_has_attributed_reference(world: &mut CodetagsWorld) {
    let index = index(world);
    let unattributed: Vec<&str> = index
        .documents
        .iter()
        .filter(|document| {
            !references(&document.occurrences).any(|reference| {
                enclosing_definitions(&document.occurrences)
                    .any(|(_, range)| range.contains(&reference.range))
            })
        })
        .map(|document| document.relative_path.as_str())
        .collect();
    assert!(!index.documents.is_empty(), "the index has no documents");
    assert!(
        unattributed.is_empty(),
        "no reference lies within a definition in {unattributed:?}"
    );
}

#[then(
    expr = "the reference to {string} on line {int} of {string} lies within the definition of {string}"
)]
fn reference_within(
    world: &mut CodetagsWorld,
    name: String,
    line: u32,
    path: String,
    expected: String,
) {
    let document = document(index(world), &path);
    let reference = references(&document.occurrences)
        .find(|occurrence| {
            descriptors(&occurrence.symbol) == Some(name.as_str())
                && occurrence.range.start_line + 1 == line
        })
        .unwrap_or_else(|| panic!("no reference to {name:?} in {path} on line {line}"));
    // The innermost: the containing definition that every other containing
    // definition contains.
    let innermost = enclosing_definitions(&document.occurrences)
        .filter(|(_, range)| range.contains(&reference.range))
        .fold(
            None,
            |best: Option<(&Occurrence, Range)>, candidate| match best {
                Some((_, range)) if !range.contains(&candidate.1) => best,
                _ => Some(candidate),
            },
        )
        .unwrap_or_else(|| panic!("no definition in {path} encloses line {line}"));
    assert_eq!(
        descriptors(&innermost.0.symbol),
        Some(expected.as_str()),
        "innermost definition enclosing {name:?} on line {line}"
    );
}

#[then(expr = "the symbol {string} implements {string}")]
fn symbol_implements(world: &mut CodetagsWorld, name: String, target: String) {
    let info = index(world)
        .documents
        .iter()
        .flat_map(|document| &document.symbols)
        .find(|info| descriptors(&info.symbol) == Some(name.as_str()))
        .unwrap_or_else(|| panic!("no symbol information for {name:?}"));
    assert!(
        info.relationships.iter().any(|relationship| {
            relationship.is_implementation
                && descriptors(&relationship.symbol) == Some(target.as_str())
        }),
        "{name:?} does not implement {target:?}: {:?}",
        info.relationships
    );
}

#[then(expr = "the definition of {string} has the canonical name {string}")]
fn definition_canonical_name(world: &mut CodetagsWorld, name: String, expected: String) {
    let symbol = &definition(index(world), &name).symbol;
    let parsed = codetags_names::scip::parse(symbol).unwrap_or_else(|e| panic!("{symbol:?}: {e}"));
    let canonical = canonical_name(&parsed).unwrap_or_else(|e| panic!("{symbol:?}: {e}"));
    assert_eq!(canonical.as_deref(), Some(expected.as_str()), "{symbol:?}");
}

fn call_graph(world: &CodetagsWorld) -> &[CallEdge] {
    // The index step reports a failed run with its error.
    index(world);
    world
        .index
        .call_graph
        .as_deref()
        .expect("the provider run wrote a call graph")
}

#[then(expr = "the call-graph edges on line {int} of {string} are:")]
fn edges_on_line(world: &mut CodetagsWorld, line: u32, path: String, step: &Step) {
    let table = step.table.as_ref().expect("a table of edges");
    let mut rows = table.rows.iter();
    let header = rows.next().expect("a header row");
    assert_eq!(
        header,
        &["caller", "callee", "column", "kind", "algorithm"],
        "table columns"
    );
    let expected: BTreeSet<Vec<String>> = rows.cloned().collect();
    let actual: BTreeSet<Vec<String>> = call_graph(world)
        .iter()
        .filter(|edge| edge.site.file == path && edge.site.line == line)
        .map(|edge| {
            vec![
                edge.caller.clone(),
                edge.callee.clone(),
                edge.site.column.to_string(),
                edge.kind.as_str().to_string(),
                edge.algorithm.as_str().to_string(),
            ]
        })
        .collect();
    assert_eq!(actual, expected, "edges on line {line} of {path}");
}

#[then(
    expr = "every call site in the call graph, except a function literal's, starts an occurrence in the index"
)]
fn call_sites_start_occurrences(world: &mut CodetagsWorld) {
    let edges = call_graph(world);
    let index = index(world);
    let root = world
        .index
        .root
        .as_deref()
        .expect("a Go run recorded its root");
    let mut sources: HashMap<&str, Vec<String>> = HashMap::new();
    let mut missing = BTreeSet::new();
    for edge in edges {
        let site = &edge.site;
        let lines = sources.entry(site.file.as_str()).or_insert_with(|| {
            std::fs::read_to_string(root.join(&site.file))
                .unwrap_or_else(|e| panic!("{}: {e}", site.file))
                .lines()
                .map(str::to_string)
                .collect()
        });
        let text = &lines[site.line as usize - 1][site.column as usize - 1..];
        if text.starts_with("func") {
            continue;
        }
        let found = document(index, &site.file).occurrences.iter().any(|o| {
            o.range.start_line + 1 == site.line && o.range.start_character + 1 == site.column
        });
        if !found {
            missing.insert(format!("{}:{}:{}", site.file, site.line, site.column));
        }
    }
    assert!(!edges.is_empty(), "the call graph has no edges");
    assert!(
        missing.is_empty(),
        "call sites with no occurrence: {missing:?}"
    );
}
