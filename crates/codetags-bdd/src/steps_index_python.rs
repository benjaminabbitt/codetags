//! Steps for `features/index/python.feature` (PLAN.md §9 P1.7): the Python
//! provider (`scip-python` plus unresolved name sites).
//!
//! Lines and columns are 1-based. Symbols are named by their descriptors, as
//! in [`crate::steps_index`].

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use codetags_ingest::provider::python::{PythonProvider, UnresolvedReference};
use cucumber::gherkin::Step;
use cucumber::{then, when};

use crate::CodetagsWorld;
use crate::steps_index::{RunOutcome, descriptors, fixture, index};

/// A Python run: the index (or the error), and the unresolved references
/// when it passed.
type PythonOutcome = (RunOutcome, Option<Arc<Vec<UnresolvedReference>>>);

/// Fixture runs, shared by one process's scenarios.
fn python_fixture_runs() -> &'static Mutex<HashMap<String, PythonOutcome>> {
    static RUNS: OnceLock<Mutex<HashMap<String, PythonOutcome>>> = OnceLock::new();
    RUNS.get_or_init(Default::default)
}

/// Runs the Python provider over `root`, writing into `scratch`.
fn run_python(root: &Path, scratch: &Path) -> PythonOutcome {
    match PythonProvider::new().index(root, &scratch.join("python")) {
        Ok(run) => (Arc::new(Ok(run.scip.index)), Some(Arc::new(run.unresolved))),
        Err(error) => (Arc::new(Err(error.to_string())), None),
    }
}

fn record(world: &mut CodetagsWorld, (run, unresolved): PythonOutcome) {
    world.index.set_run(run);
    world.index.unresolved = unresolved;
}

#[when(expr = "the Python provider indexes the fixture {string}")]
fn python_indexes_fixture(world: &mut CodetagsWorld, name: String) {
    let cached = python_fixture_runs()
        .lock()
        .expect("fixture cache lock")
        .get(&name)
        .cloned();
    let outcome = match cached {
        Some(outcome) => outcome,
        None => {
            let outcome = run_python(&fixture(&name), world.scratch());
            python_fixture_runs()
                .lock()
                .expect("fixture cache lock")
                .insert(name, outcome.clone());
            outcome
        }
    };
    record(world, outcome);
}

#[when(expr = "the Python provider indexes a project whose main.py is:")]
fn python_indexes_main(world: &mut CodetagsWorld, step: &Step) {
    let source = step
        .docstring
        .as_ref()
        .expect("a docstring holding main.py");
    let scratch = world.scratch().to_path_buf();
    let root = scratch.join("project");
    std::fs::create_dir_all(&root).expect("create project dir");
    std::fs::write(
        root.join("pyproject.toml"),
        "[project]\nname = \"broken\"\nversion = \"0.1.0\"\n",
    )
    .expect("write pyproject.toml");
    std::fs::write(root.join("main.py"), source).expect("write main.py");
    let outcome = run_python(&root, &scratch);
    record(world, outcome);
}

#[when(expr = "the Python provider indexes a directory that does not exist")]
fn python_indexes_nothing(world: &mut CodetagsWorld) {
    let scratch = world.scratch().to_path_buf();
    let outcome = run_python(&scratch.join("no-such-project"), &scratch);
    record(world, outcome);
}

#[then(expr = "every definition of a method descriptor has an enclosing range")]
fn method_descriptors_have_enclosing_ranges(world: &mut CodetagsWorld) {
    let mut checked = 0;
    for document in &index(world).documents {
        for occurrence in &document.occurrences {
            if occurrence.is_definition()
                && !occurrence.is_local()
                && occurrence.symbol.ends_with("().")
            {
                assert!(
                    occurrence.enclosing_range.is_some(),
                    "{} in {} has no enclosing range",
                    occurrence.symbol,
                    document.relative_path
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "the index defines no methods");
}

/// Whether some document or the external symbols hold information for the
/// symbol named `name`; panics when the index never mentions the symbol.
fn has_symbol_information(world: &CodetagsWorld, name: &str) -> bool {
    let index = index(world);
    let occurs = index
        .documents
        .iter()
        .flat_map(|document| &document.occurrences)
        .any(|occurrence| descriptors(&occurrence.symbol) == Some(name));
    assert!(occurs, "{name:?} never occurs in the index");
    index
        .documents
        .iter()
        .flat_map(|document| &document.symbols)
        .chain(&index.external_symbols)
        .any(|info| descriptors(&info.symbol) == Some(name))
}

#[then(expr = "the symbol {string} has symbol information")]
fn symbol_has_information(world: &mut CodetagsWorld, name: String) {
    assert!(
        has_symbol_information(world, &name),
        "{name:?} has no symbol information"
    );
}

#[then(expr = "the symbol {string} has no symbol information")]
fn symbol_has_no_information(world: &mut CodetagsWorld, name: String) {
    assert!(
        !has_symbol_information(world, &name),
        "{name:?} has symbol information"
    );
}

#[then(expr = "the unresolved references are:")]
fn unresolved_references_are(world: &mut CodetagsWorld, step: &Step) {
    let table = step.table.as_ref().expect("a table of references");
    let mut rows = table.rows.iter();
    let header = rows.next().expect("a header row");
    assert_eq!(
        header,
        &["file", "line", "column", "name", "called", "reason"],
        "table columns"
    );
    let expected: BTreeSet<Vec<String>> = rows.cloned().collect();
    // The index step reports a failed run with its error.
    index(world);
    let actual: BTreeSet<Vec<String>> = world
        .index
        .unresolved
        .as_deref()
        .expect("the provider run listed unresolved references")
        .iter()
        .map(|reference| {
            let site = &reference.site;
            vec![
                site.file.clone(),
                (site.range.start_line + 1).to_string(),
                (site.range.start_character + 1).to_string(),
                site.name.clone(),
                if site.called { "yes" } else { "no" }.to_string(),
                reference.reason.as_str().to_string(),
            ]
        })
        .collect();
    assert_eq!(actual, expected, "unresolved references");
}
