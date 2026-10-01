//! Steps for `features/index/typescript.feature`: scip-typescript and Jelly
//! (PLAN.md §9 P1.6). Shared helpers and the run steps live in
//! [`crate::steps_index`].
//!
//! A location is `<path>:<line>`, 1-based; a function's is the line it
//! starts on.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use codetags_ingest::provider::ts::ScipTypescript;
use codetags_ingest::provider::ts::jelly::{CallGraph, CalleeKind, Jelly, Location};
use cucumber::{given, then, when};

use crate::CodetagsWorld;
use crate::steps_index::{
    GraphOutcome, RunOutcome, cached_fixture_run, definition, descriptors, document, fixture, index,
};

/// Runs scip-typescript over `root`, its output in `scratch`.
fn run_scip_typescript(provider: &ScipTypescript, root: &Path, scratch: &Path) -> RunOutcome {
    Arc::new(
        provider
            .index(root, &scratch.join("scip"))
            .map(|run| run.index)
            .map_err(|error| error.to_string()),
    )
}

/// Runs Jelly over `root`, its output in `scratch`.
fn run_jelly(root: &Path, scratch: &Path) -> GraphOutcome {
    Arc::new(
        Jelly::new()
            .analyze(root, &scratch.join("jelly"))
            .map(|run| run.graph)
            .map_err(|error| error.to_string()),
    )
}

/// Jelly's fixture runs, shared by the scenarios of one process.
fn jelly_fixture_runs() -> &'static Mutex<HashMap<String, GraphOutcome>> {
    static RUNS: OnceLock<Mutex<HashMap<String, GraphOutcome>>> = OnceLock::new();
    RUNS.get_or_init(Default::default)
}

fn project(world: &CodetagsWorld) -> &Path {
    world
        .index
        .project
        .as_deref()
        .expect("an earlier step built a project")
}

fn graph(world: &CodetagsWorld) -> &CallGraph {
    let graph = world
        .index
        .graph
        .as_ref()
        .expect("an earlier step ran Jelly");
    match graph.as_ref() {
        Ok(graph) => graph,
        Err(error) => panic!("the Jelly run failed: {error}"),
    }
}

/// A fresh project directory in the scratch directory, holding `path` with
/// `contents`, and the fixture "ts"'s configuration when `configured`.
fn build_project(world: &mut CodetagsWorld, path: &str, contents: &[u8], configured: bool) {
    let root = world.scratch().join("project");
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().expect("a file path has a parent"))
        .expect("create project dirs");
    std::fs::write(&file, contents).expect("write project file");
    if configured {
        for name in ["tsconfig.json", "package.json"] {
            std::fs::copy(fixture("ts").join(name), root.join(name)).expect("copy config");
        }
    }
    world.index.project = Some(root);
}

#[given(expr = "a TypeScript project whose {string} is {string}")]
fn ts_project(world: &mut CodetagsWorld, path: String, contents: String) {
    build_project(world, &path, contents.as_bytes(), true);
}

#[given(expr = "a TypeScript project whose {string} declares a function in {int} MB")]
fn ts_project_of_size(world: &mut CodetagsWorld, path: String, megabytes: usize) {
    let mut contents = String::from("export function big(): number {\n  return 1;\n}\n");
    let padding = "// padding to grow the file past scip-typescript's size limit\n";
    while contents.len() < megabytes * 1_000_000 {
        contents.push_str(padding);
    }
    build_project(world, &path, contents.as_bytes(), true);
}

#[given(expr = "a directory without a tsconfig.json whose {string} is {string}")]
fn bare_project(world: &mut CodetagsWorld, path: String, contents: String) {
    build_project(world, &path, contents.as_bytes(), false);
}

#[when(expr = "the TypeScript provider indexes the fixture {string}")]
fn ts_indexes_fixture(world: &mut CodetagsWorld, name: String) {
    let scratch = world.scratch().to_path_buf();
    let run = cached_fixture_run("scip-typescript", &name, || {
        run_scip_typescript(&ScipTypescript::new(), &fixture(&name), &scratch)
    });
    world.index.set_run(run);
}

#[when(expr = "the TypeScript provider indexes that project")]
fn ts_indexes_project(world: &mut CodetagsWorld) {
    let scratch = world.scratch().to_path_buf();
    let run = run_scip_typescript(&ScipTypescript::new(), project(world), &scratch);
    world.index.set_run(run);
}

#[when(expr = "the TypeScript provider indexes that project with a file-size limit of {string}")]
fn ts_indexes_project_with_limit(world: &mut CodetagsWorld, limit: String) {
    let scratch = world.scratch().to_path_buf();
    let provider = ScipTypescript::new().max_file_byte_size(limit);
    let run = run_scip_typescript(&provider, project(world), &scratch);
    world.index.set_run(run);
}

#[when(expr = "Jelly analyzes the fixture {string}")]
fn jelly_analyzes_fixture(world: &mut CodetagsWorld, name: String) {
    let cached = jelly_fixture_runs()
        .lock()
        .expect("fixture cache lock")
        .get(&name)
        .cloned();
    let run = cached.unwrap_or_else(|| {
        let run = run_jelly(&fixture(&name), world.scratch());
        jelly_fixture_runs()
            .lock()
            .expect("fixture cache lock")
            .insert(name, run.clone());
        run
    });
    world.index.set_graph(run);
}

#[when(expr = "Jelly analyzes that project")]
fn jelly_analyzes_project(world: &mut CodetagsWorld) {
    let scratch = world.scratch().to_path_buf();
    let run = run_jelly(project(world), &scratch);
    world.index.set_graph(run);
}

#[when(expr = "Jelly analyzes a directory that does not exist")]
fn jelly_analyzes_nothing(world: &mut CodetagsWorld) {
    let scratch = world.scratch().to_path_buf();
    world
        .index
        .set_graph(run_jelly(&scratch.join("no-such-project"), &scratch));
}

#[then(expr = "the definition of {string} has no enclosing range")]
fn definition_has_no_enclosing_range(world: &mut CodetagsWorld, name: String) {
    let enclosing = definition(index(world), &name).enclosing_range;
    assert!(
        enclosing.is_none(),
        "{name:?} has an enclosing range {enclosing:?}"
    );
}

#[then(expr = "the definitions of {string} enclose lines {string}")]
fn definitions_enclose(world: &mut CodetagsWorld, name: String, expected: String) {
    let found: Vec<String> = index(world)
        .documents
        .iter()
        .flat_map(|document| &document.occurrences)
        .filter(|o| o.is_definition() && descriptors(&o.symbol) == Some(name.as_str()))
        .map(|o| match o.enclosing_range {
            Some(r) => format!("{}-{}", r.start_line + 1, r.end_line + 1),
            None => "none".to_string(),
        })
        .collect();
    let expected: Vec<&str> = expected.split_whitespace().collect();
    assert_eq!(found, expected, "definitions of {name:?}");
}

#[then(expr = "{string} implements {string}")]
fn implements(world: &mut CodetagsWorld, name: String, target: String) {
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

#[then(expr = "no global symbol occurs in {string} on line {int}")]
fn no_global_on_line(world: &mut CodetagsWorld, path: String, line: u32) {
    let globals: Vec<&str> = document(index(world), &path)
        .occurrences
        .iter()
        .filter(|o| o.range.start_line + 1 == line && !o.is_local())
        .map(|o| o.symbol.as_str())
        .collect();
    assert!(
        globals.is_empty(),
        "{path} line {line} has global symbols {globals:?}"
    );
}

/// `<file>:<line>` for a location, its 1-based start line.
fn spelled(location: &Location) -> String {
    format!("{}:{}", location.file, location.range.start_line + 1)
}

#[then(expr = "the call graph files are exactly {string}")]
fn graph_files_are(world: &mut CodetagsWorld, expected: String) {
    let files: BTreeSet<&str> = graph(world).files.iter().map(String::as_str).collect();
    let expected: BTreeSet<&str> = expected.split_whitespace().collect();
    assert_eq!(files, expected);
}

#[then(expr = "the calls on line {int} of {string} reach exactly {string}")]
fn calls_reach(world: &mut CodetagsWorld, line: u32, path: String, expected: String) {
    let graph = graph(world);
    let at_line = |site: &Location| site.file == path && site.range.start_line + 1 == line;
    assert!(
        graph.sites.iter().any(at_line),
        "Jelly saw no call on line {line} of {path}"
    );
    let found: BTreeSet<String> = graph
        .function_edges()
        .filter(|edge| at_line(&edge.site))
        .map(|edge| spelled(&edge.callee))
        .collect();
    let expected: BTreeSet<String> = expected.split_whitespace().map(String::from).collect();
    assert_eq!(found, expected, "callees on line {line} of {path}");
}

#[then(expr = "the call graph has an edge from {string} to {string} at {string}")]
fn has_edge(world: &mut CodetagsWorld, caller: String, callee: String, site: String) {
    let graph = graph(world);
    let found = graph.function_edges().any(|edge| {
        spelled(&edge.caller) == caller
            && spelled(&edge.callee) == callee
            && spelled(&edge.site) == site
    });
    assert!(
        found,
        "no edge from {caller} to {callee} at {site}; edges at {site}: {:?}",
        graph
            .edges
            .iter()
            .filter(|edge| spelled(&edge.site) == site)
            .map(|edge| (spelled(&edge.caller), spelled(&edge.callee)))
            .collect::<Vec<_>>()
    );
}

#[then(expr = "line {int} of {string} loads the module {string}")]
fn loads_module(world: &mut CodetagsWorld, line: u32, path: String, module: String) {
    let found = graph(world).edges.iter().any(|edge| {
        edge.kind == CalleeKind::Module
            && edge.site.file == path
            && edge.site.range.start_line + 1 == line
            && edge.callee.file == module
    });
    assert!(found, "line {line} of {path} does not load {module}");
}
