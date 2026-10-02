//! Steps for `features/index/`: SCIP providers over the fixtures (PLAN.md §9).
//!
//! Symbols are named by their descriptors: the SCIP symbol without its
//! scheme, manager, package and version, e.g. `charge/double().`. Lines are
//! 1-based.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use codetags_ingest::ingest;
use codetags_ingest::provider::go::CallEdge;
use codetags_ingest::provider::python::UnresolvedReference;
use codetags_ingest::provider::rust::RustAnalyzer;
use codetags_ingest::provider::scip::{Document, Occurrence, ScipIndex, SymbolKind};
use codetags_ingest::provider::ts::jelly::CallGraph;
use codetags_model::{GenerationStore, ToSql};
use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use regex::Regex;

use crate::CodetagsWorld;

/// The outcome of a provider run: the decoded index, or the error's message.
pub(crate) type RunOutcome = Arc<Result<ScipIndex, String>>;

/// The outcome of a Jelly run: the call graph, or the error's message.
pub(crate) type GraphOutcome = Arc<Result<CallGraph, String>>;

/// State the index steps share within one scenario.
#[derive(Debug, Default)]
pub struct IndexState {
    /// The last SCIP provider run.
    pub(crate) run: Option<RunOutcome>,
    /// The call graph of the last provider run, for providers that write one
    /// (Go, `steps_index_go`).
    pub(crate) call_graph: Option<Arc<Vec<CallEdge>>>,
    /// The root the last Go provider run indexed.
    pub(crate) root: Option<PathBuf>,
    /// The report of the last ingest into a generation (P1.3).
    pub(crate) ingest: Option<codetags_ingest::ingest::IngestReport>,
    /// The last Jelly run (TypeScript, `steps_index_ts`).
    pub(crate) graph: Option<GraphOutcome>,
    /// Whether the call-graph run is the latest run, the one the run steps
    /// check.
    pub(crate) graph_last: bool,
    /// The project a `Given` step built in the scratch directory.
    pub(crate) project: Option<PathBuf>,
    /// The unresolved references of the last Python provider run
    /// (`steps_index_python`).
    pub(crate) unresolved: Option<Arc<Vec<UnresolvedReference>>>,
}

impl IndexState {
    /// Records a SCIP provider run as the latest run.
    pub(crate) fn set_run(&mut self, run: RunOutcome) {
        self.run = Some(run);
        self.graph_last = false;
    }

    /// Records a call-graph run as the latest run.
    pub(crate) fn set_graph(&mut self, graph: GraphOutcome) {
        self.graph = Some(graph);
        self.graph_last = true;
    }

    /// The latest run's error message, or `None` if it succeeded.
    fn last_error(&self) -> Option<&str> {
        let error = if self.graph_last {
            let graph = self.graph.as_ref().expect("an earlier step ran Jelly");
            graph.as_ref().as_ref().err()
        } else {
            let run = self.run.as_ref().expect("an earlier step ran a provider");
            run.as_ref().as_ref().err()
        };
        error.map(String::as_str)
    }
}

/// Fixture runs, shared by the scenarios of one process: the index of a
/// fixture is deterministic, and each run takes several seconds. Keyed by
/// tool and fixture.
fn fixture_runs() -> &'static Mutex<HashMap<String, RunOutcome>> {
    static RUNS: OnceLock<Mutex<HashMap<String, RunOutcome>>> = OnceLock::new();
    RUNS.get_or_init(Default::default)
}

/// The cached run of `tool` over the fixture `name`, made by `run` on a
/// cache miss.
pub(crate) fn cached_fixture_run(
    tool: &str,
    name: &str,
    run: impl FnOnce() -> RunOutcome,
) -> RunOutcome {
    let key = format!("{tool}:{name}");
    let cached = fixture_runs()
        .lock()
        .expect("fixture cache lock")
        .get(&key)
        .cloned();
    cached.unwrap_or_else(|| {
        let outcome = run();
        fixture_runs()
            .lock()
            .expect("fixture cache lock")
            .insert(key, outcome.clone());
        outcome
    })
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

/// The Rust provider's run over fixture `name`, shared by every scenario of
/// this process.
fn fixture_run(world: &mut CodetagsWorld, name: &str) -> RunOutcome {
    let scratch = world.scratch().to_path_buf();
    cached_fixture_run("rust", name, || run_rust(&fixture(name), &scratch))
}

#[when(expr = "the Rust provider indexes the fixture {string}")]
fn rust_indexes_fixture(world: &mut CodetagsWorld, name: String) {
    let run = fixture_run(world, &name);
    world.index.set_run(run);
}

#[when(expr = "the Rust provider indexes a directory whose Cargo.toml is {string}")]
fn rust_indexes_manifest(world: &mut CodetagsWorld, manifest: String) {
    let scratch = world.scratch().to_path_buf();
    let root = scratch.join("project");
    std::fs::create_dir_all(&root).expect("create project dir");
    std::fs::write(root.join("Cargo.toml"), manifest).expect("write Cargo.toml");
    world.index.set_run(run_rust(&root, &scratch));
}

#[when(expr = "the Rust provider indexes a directory that does not exist")]
fn rust_indexes_nothing(world: &mut CodetagsWorld) {
    let scratch = world.scratch().to_path_buf();
    world
        .index
        .set_run(run_rust(&scratch.join("no-such-project"), &scratch));
}

#[then(expr = "the provider run succeeds")]
fn run_succeeds(world: &mut CodetagsWorld) {
    if let Some(error) = world.index.last_error() {
        panic!("the provider run failed: {error}");
    }
}

#[then(expr = "the provider run fails with stderr matching {string}")]
fn run_fails(world: &mut CodetagsWorld, pattern: String) {
    let error = world
        .index
        .last_error()
        .expect("the provider run succeeded");
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

// --- P1.3: ingest into a generation ------------------------------------------

/// The scenario's index directory: the same one the generation-store steps
/// use, so their steps apply to ingested generations too.
fn index_store(world: &mut CodetagsWorld) -> GenerationStore {
    GenerationStore::new(world.scratch().join(".codetags").join("index"))
}

/// A read-only connection to the newest complete generation.
fn newest(world: &mut CodetagsWorld) -> codetags_model::Connection {
    index_store(world)
        .open_newest()
        .expect("open the newest complete generation")
        .connect()
        .expect("connect")
}

/// Rows of `sql` as strings, one `Vec` per row; NULL reads as "".
fn rows(db: &codetags_model::Connection, sql: &str, params: &[&dyn ToSql]) -> Vec<Vec<String>> {
    let mut statement = db.prepare(sql).expect("prepare");
    let mut out = Vec::new();
    let mut result = statement.query(params).expect("query");
    while let Some(row) = result.next().expect("row") {
        // DuckDB knows the column count only once the statement has run.
        let columns = row.as_ref().column_count();
        out.push(
            (0..columns)
                .map(|i| {
                    row.get::<_, Option<String>>(i)
                        .expect("text")
                        .unwrap_or_default()
                })
                .collect(),
        );
    }
    out
}

/// A table's rows as maps from header to cell.
pub(crate) fn table_rows(step: &Step) -> Vec<HashMap<String, String>> {
    let table = step.table.as_ref().expect("the step has a table");
    let header = &table.rows[0];
    table.rows[1..]
        .iter()
        .map(|row| header.iter().cloned().zip(row.iter().cloned()).collect())
        .collect()
}

fn yes_no(value: &str) -> &'static str {
    match value {
        "true" => "yes",
        "false" => "no",
        other => panic!("not a boolean: {other:?}"),
    }
}

fn ingest_fixture(world: &mut CodetagsWorld, name: &str) {
    let run = fixture_run(world, name);
    let index = match run.as_ref() {
        Ok(index) => index,
        Err(error) => panic!("the provider run failed: {error}"),
    };
    let root = fixture(name);
    let now = SystemTime::now();
    let writer = index_store(world).begin().expect("begin a generation");
    let report = ingest::ingest(
        writer.connection(),
        &ingest::Input {
            root: &root,
            index,
            language: "rust",
            run: ingest::RunInfo {
                provider: codetags_ingest::provider::rust::PROVIDER.to_string(),
                provider_version: index.tool.clone(),
                args: vec!["scip".to_string(), root.display().to_string()],
                started_at: now,
                finished_at: now,
            },
        },
    )
    .unwrap_or_else(|error| panic!("ingest failed: {error}"));
    writer.complete().expect("complete the generation");
    world.index.ingest = Some(report);
}

#[given(expr = "the fixture {string} has been ingested into a new generation")]
fn fixture_has_been_ingested(world: &mut CodetagsWorld, name: String) {
    ingest_fixture(world, &name);
}

#[when(expr = "the fixture {string} is ingested into a new generation")]
fn fixture_is_ingested(world: &mut CodetagsWorld, name: String) {
    ingest_fixture(world, &name);
}

#[then(expr = "the generation's symbols include:")]
fn symbols_include(world: &mut CodetagsWorld, step: &Step) {
    let db = newest(world);
    let symbols = rows(
        &db,
        "SELECT name, id, kind, file, start_line::TEXT || '-' || end_line::TEXT,
                module, external::TEXT
         FROM symbol",
        &[],
    );
    for expected in table_rows(step) {
        let found = symbols
            .iter()
            .find(|row| row[0] == expected["name"])
            .unwrap_or_else(|| panic!("no symbol named {:?}", expected["name"]));
        let actual: HashMap<&str, String> = HashMap::from([
            ("name", found[0].clone()),
            (
                "descriptors",
                descriptors(&found[1]).unwrap_or_default().to_string(),
            ),
            ("kind", found[2].clone()),
            ("file", found[3].clone()),
            ("lines", found[4].clone()),
            ("module", found[5].clone()),
            ("external", yes_no(&found[6]).to_string()),
        ]);
        for (column, value) in &expected {
            assert_eq!(
                &actual[column.as_str()],
                value,
                "{column} of symbol {:?}",
                expected["name"]
            );
        }
        assert_eq!(
            symbols
                .iter()
                .filter(|row| row[0] == expected["name"])
                .count(),
            1,
            "{:?} names more than one symbol",
            expected["name"]
        );
    }
}

#[then(expr = "the symbols defined in {string} are exactly:")]
fn symbols_defined_in(world: &mut CodetagsWorld, path: String, step: &Step) {
    let db = newest(world);
    let mut actual: Vec<String> = rows(&db, "SELECT name FROM symbol WHERE file = ?", &[&path])
        .into_iter()
        .map(|row| row[0].clone())
        .collect();
    actual.sort();
    let mut expected: Vec<String> = table_rows(step)
        .into_iter()
        .map(|row| row["name"].clone())
        .collect();
    expected.sort();
    assert_eq!(actual, expected, "symbols defined in {path}");
}

#[then(expr = "the ingest reports no canonical-name collisions")]
fn no_collisions(world: &mut CodetagsWorld) {
    let report = world
        .index
        .ingest
        .as_ref()
        .expect("an earlier step ingested");
    assert!(
        report.collisions.is_empty(),
        "collisions: {:?}",
        report.collisions
    );
}

/// The call sites in `path` on `line`, as sorted rows of
/// caller, target, kind, dispatch, external.
fn call_sites_on(world: &mut CodetagsWorld, path: &str, line: i64) -> Vec<Vec<String>> {
    let db = newest(world);
    let mut sites = rows(
        &db,
        "SELECT caller.name, target.name, s.ref_kind, s.dispatch, target.external::TEXT
         FROM call_site s
         JOIN symbol caller ON caller.id = s.caller
         JOIN symbol target ON target.id = s.declared_target
         WHERE s.file = ? AND s.line = ?",
        &[&path, &line],
    );
    for site in &mut sites {
        site[4] = yes_no(&site[4]).to_string();
    }
    sites.sort();
    sites
}

#[then(expr = "the call sites in {string} on line {int} are:")]
fn call_sites_are(world: &mut CodetagsWorld, path: String, line: i64, step: &Step) {
    let actual = call_sites_on(world, &path, line);
    let mut expected: Vec<Vec<String>> = table_rows(step)
        .into_iter()
        .map(|row| {
            ["caller", "target", "kind", "dispatch", "external"]
                .iter()
                .map(|column| row[*column].clone())
                .collect()
        })
        .collect();
    expected.sort();
    assert_eq!(actual, expected, "call sites in {path} on line {line}");
}

#[then(expr = "there are no call sites in {string} on line {int}")]
fn no_call_sites(world: &mut CodetagsWorld, path: String, line: i64) {
    let actual = call_sites_on(world, &path, line);
    assert!(
        actual.is_empty(),
        "call sites in {path} on line {line}: {actual:?}"
    );
}

#[then(expr = "no symbol or call site in the generation comes from a local symbol")]
fn nothing_local(world: &mut CodetagsWorld) {
    let db = newest(world);
    let locals = rows(
        &db,
        "SELECT id FROM symbol WHERE id LIKE 'local %'
         UNION ALL SELECT caller FROM call_site WHERE caller LIKE 'local %'
         UNION ALL SELECT declared_target FROM call_site WHERE declared_target LIKE 'local %'
         UNION ALL SELECT target FROM call_target WHERE target LIKE 'local %'",
        &[],
    );
    assert!(
        locals.is_empty(),
        "local symbols in the generation: {locals:?}"
    );
}

#[then(
    expr = "every call site has exactly one call target, its declared target, with the method {string}"
)]
fn one_declared_target(world: &mut CodetagsWorld, method: String) {
    let db = newest(world);
    let sites = rows(&db, "SELECT count(*)::TEXT FROM call_site", &[]);
    assert_ne!(sites[0][0], "0", "the generation has no call sites");
    let bad = rows(
        &db,
        "SELECT s.site_id::TEXT, count(t.site_id)::TEXT,
                string_agg(t.method, ','), bool_and(t.target = s.declared_target)::TEXT
         FROM call_site s LEFT JOIN call_target t USING (site_id)
         GROUP BY s.site_id
         HAVING count(t.site_id) <> 1
             OR NOT bool_and(t.target = s.declared_target)
             OR any_value(t.method) <> ?",
        &[&method],
    );
    assert!(bad.is_empty(), "call sites with other targets: {bad:?}");
}

#[then(expr = "every call site's source matches {string}")]
fn sources_match(world: &mut CodetagsWorld, pattern: String) {
    let regex = Regex::new(&pattern).expect("valid regex");
    let db = newest(world);
    let sources = rows(&db, "SELECT DISTINCT source FROM call_site", &[]);
    assert!(!sources.is_empty(), "the generation has no call sites");
    for source in sources {
        assert!(regex.is_match(&source[0]), "source {:?}", source[0]);
    }
}

#[then(expr = "the generation holds one succeeded run of {string}")]
fn one_run(world: &mut CodetagsWorld, provider: String) {
    let db = newest(world);
    let runs = rows(&db, "SELECT provider, status FROM run", &[]);
    assert_eq!(runs, vec![vec![provider, "succeeded".to_string()]]);
}

#[then(expr = "the run's edge counts are:")]
fn edge_counts(world: &mut CodetagsWorld, step: &Step) {
    let db = newest(world);
    let actual = rows(
        &db,
        "SELECT path, edge_count::TEXT FROM run_file ORDER BY path",
        &[],
    );
    let expected: Vec<Vec<String>> = table_rows(step)
        .into_iter()
        .map(|row| vec![row["file"].clone(), row["edges"].clone()])
        .collect();
    assert_eq!(actual, expected);
}

#[then(expr = "the call targets of the call to {string} in {string} on line {int} are:")]
fn call_targets_are(
    world: &mut CodetagsWorld,
    declared: String,
    path: String,
    line: i64,
    step: &Step,
) {
    let db = newest(world);
    let sites = rows(
        &db,
        "SELECT count(*)::TEXT FROM call_site s JOIN symbol d ON d.id = s.declared_target
         WHERE s.file = ? AND s.line = ? AND d.name = ?",
        &[&path, &line, &declared],
    );
    if sites[0][0] == "0" {
        let all = rows(
            &db,
            "SELECT s.line::TEXT, d.name FROM call_site s JOIN symbol d ON d.id = s.declared_target
             WHERE s.file = ? ORDER BY s.line",
            &[&path],
        );
        panic!("no call to {declared} in {path} on line {line}; its call sites: {all:?}");
    }
    let mut actual = rows(
        &db,
        "SELECT t.name, ct.method,
                (count(*) OVER (PARTITION BY ct.site_id, ct.method))::TEXT
         FROM call_site s
         JOIN symbol d ON d.id = s.declared_target
         JOIN call_target ct ON ct.site_id = s.site_id
         JOIN symbol t ON t.id = ct.target
         WHERE s.file = ? AND s.line = ? AND d.name = ?",
        &[&path, &line, &declared],
    );
    actual.sort();
    let mut expected: Vec<Vec<String>> = table_rows(step)
        .into_iter()
        .map(|row| {
            ["target", "method", "candidates"]
                .iter()
                .map(|column| row[*column].clone())
                .collect()
        })
        .collect();
    expected.sort();
    assert_eq!(
        actual, expected,
        "call targets of the call to {declared} in {path} on line {line}"
    );
}

#[then(expr = "the edge counts of the run of {string} are:")]
fn edge_counts_of_run(world: &mut CodetagsWorld, provider: String, step: &Step) {
    let db = newest(world);
    let actual = rows(
        &db,
        "SELECT f.path, f.edge_count::TEXT FROM run_file f JOIN run r USING (run_id)
         WHERE r.provider = ? AND r.status = 'succeeded' ORDER BY f.path",
        &[&provider],
    );
    let expected: Vec<Vec<String>> = table_rows(step)
        .into_iter()
        .map(|row| vec![row["file"].clone(), row["edges"].clone()])
        .collect();
    assert_eq!(actual, expected, "edge counts of the run of {provider}");
}

#[then(expr = "the ingest report counts:")]
fn report_counts(world: &mut CodetagsWorld, step: &Step) {
    let report = world
        .index
        .ingest
        .as_ref()
        .expect("an earlier step ingested");
    for row in table_rows(step) {
        let actual = match row["what"].as_str() {
            "files" => report.files,
            "symbols" => report.symbols,
            "call sites" => report.call_sites,
            "local occurrences" => report.skipped.local,
            "operator references" => report.skipped.operator,
            "non-callable references" => report.skipped.non_callable,
            "references outside a definition" => report.skipped.outside_definition,
            other => panic!("unknown count {other:?}"),
        };
        assert_eq!(actual.to_string(), row["count"], "{}", row["what"]);
    }
}

/// Copies the directory `from` into `to`, recursively.
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create directory");
    for entry in std::fs::read_dir(from).expect("read directory") {
        let entry = entry.expect("directory entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}

#[given(expr = "the scenario's directory holds a copy of the fixture {string}")]
fn copy_fixture(world: &mut CodetagsWorld, name: String) {
    let scratch = world.scratch().to_path_buf();
    copy_dir(&fixture(&name), &scratch);
}

#[when(expr = "codetags is run in the scenario's directory with {string}")]
fn codetags_in_scratch(world: &mut CodetagsWorld, args: String) {
    let binary = crate::CODETAGS_BIN
        .get()
        .expect("run() sets the binary path");
    let scratch = world.scratch().to_path_buf();
    let mut command = Command::new(binary);
    command
        .args(args.split_whitespace())
        .current_dir(&scratch)
        // The provider's cargo output stays out of the project copy.
        .env("CARGO_TARGET_DIR", scratch.join(".bdd-target"));
    for name in &world.env_removed {
        command.env_remove(name);
    }
    command.envs(world.env.iter().map(|(name, value)| (name, value)));
    let output = command.output().expect("spawn codetags");
    world.last = Some(output.into());
}

#[then(expr = "the file {string} in the scenario's directory holds the lines {string}")]
fn file_holds_lines(world: &mut CodetagsWorld, path: String, lines: String) {
    let file = world.scratch().join(&path);
    let text = std::fs::read_to_string(&file)
        .unwrap_or_else(|error| panic!("read {}: {error}", file.display()));
    let actual: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    let expected: Vec<&str> = lines.split_whitespace().collect();
    assert_eq!(actual, expected, "lines of {path}");
}
