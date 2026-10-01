//! Steps for the generation store (codetags-model, PLAN.md §2.2).
//!
//! The files a generation "records" are rows of its `file` table. Steps that
//! say "another process" or "a writer process" run in a child process
//! ([`child`]), so the store's cross-process behaviour is what is tested.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use codetags_model::Connection;
use codetags_model::{Generation, GenerationStore, GenerationWriter};
use cucumber::{given, then, when};

use crate::CodetagsWorld;
use crate::child::{self, FAILED};

/// Child modes this module handles all start with this prefix.
pub(crate) const CHILD_PREFIX: &str = "store-";
const INDEX_ENV: &str = "CODETAGS_BDD_INDEX";
const FILES_ENV: &str = "CODETAGS_BDD_FILES";

impl CodetagsWorld {
    /// The scenario's index directory, inside the scratch directory.
    fn store(&mut self) -> GenerationStore {
        let index = self.scratch().join(".codetags").join("index");
        GenerationStore::new(index)
    }
}

/// Inserts `files` (whitespace-separated paths) into the `file` table.
fn record(db: &Connection, files: &str) -> Result<(), String> {
    for path in files.split_whitespace() {
        db.execute(
            "INSERT INTO file (path, language, content_hash, size) VALUES (?, NULL, 'bdd', 0)",
            [path],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The paths in `generation`'s `file` table, sorted and space-joined.
fn files_of(generation: &Generation) -> Result<String, String> {
    let db = generation.connect().map_err(|e| e.to_string())?;
    let mut statement = db
        .prepare("SELECT path FROM file ORDER BY path")
        .map_err(|e| e.to_string())?;
    let paths: Result<Vec<String>, _> = statement
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect();
    Ok(paths.map_err(|e| e.to_string())?.join(" "))
}

/// Begins a generation in `store` and records `files` in it.
fn begin_recording(store: &GenerationStore, files: &str) -> Result<GenerationWriter, String> {
    let writer = store.begin().map_err(|e| e.to_string())?;
    record(writer.connection(), files)?;
    Ok(writer)
}

/// Runs the child job `mode` against `index`.
fn spawn(mode: &str, index: &Path, files: &str) -> crate::CommandOutcome {
    child::spawn_mode(
        mode,
        &[
            (INDEX_ENV, index.as_os_str()),
            (FILES_ENV, OsStr::new(files)),
        ],
    )
}

/// A child process's job: `store-write`, `store-crash`, or `store-list`.
pub(crate) fn child(mode: &str) -> Result<String, String> {
    let index = PathBuf::from(std::env::var(INDEX_ENV).expect("child needs an index"));
    let files = std::env::var(FILES_ENV).unwrap_or_default();
    let store = GenerationStore::new(index);
    match mode {
        "store-write" => {
            let writer = begin_recording(&store, &files)?;
            let number = writer.complete().map_err(|e| e.to_string())?;
            Ok(number.to_string())
        }
        "store-crash" => {
            let writer = begin_recording(&store, &files)?;
            // Die mid-write: no destructors, no close, no completion marker.
            println!("{}", writer.number());
            std::process::exit(0)
        }
        "store-list" => {
            let generation = store.open_newest().map_err(|e| e.to_string())?;
            files_of(&generation)
        }
        other => Err(format!("unknown child mode {other:?}")),
    }
}

/// The generation number in an index file name `gen-<N>.<suffix>`.
fn generation_of(name: &str) -> Option<u64> {
    let rest = name.strip_prefix("gen-")?;
    rest.split_once('.')?.0.parse().ok()
}

#[given(expr = "an index with one complete generation per file in {string}")]
fn an_index_with_generations(world: &mut CodetagsWorld, files: String) {
    let store = world.store();
    for file in files.split_whitespace() {
        let writer = begin_recording(&store, file).expect("write generation");
        writer.complete().expect("complete generation");
    }
}

#[given(expr = "an index whose newest complete generation has schema version {int}")]
fn an_index_with_schema_version(world: &mut CodetagsWorld, version: i64) {
    let store = world.store();
    let writer = store.begin().expect("begin generation");
    writer
        .connection()
        .execute("UPDATE schema_info SET schema_version = ?", [version])
        .expect("set schema version");
    writer.complete().expect("complete generation");
}

#[given(expr = "a reader has opened the newest complete generation")]
fn a_reader_has_opened(world: &mut CodetagsWorld) {
    let reader = world.store().reader().expect("open reader");
    world.reader = Some(reader);
}

#[given(expr = "a writer has begun a generation")]
fn a_writer_has_begun(world: &mut CodetagsWorld) {
    let writer = world.store().begin().expect("begin generation");
    world.writer = Some(writer);
}

#[given(
    expr = "a writer process exits before completing a generation recording the files {string}"
)]
fn a_writer_process_exits(world: &mut CodetagsWorld, files: String) {
    let index = world.store().index().to_path_buf();
    let outcome = spawn("store-crash", &index, &files);
    assert_eq!(outcome.status, Some(0), "child failed: {}", outcome.stderr);
}

#[when(expr = "another process writes a generation recording the files {string}")]
fn another_process_writes(world: &mut CodetagsWorld, files: String) {
    let index = world.store().index().to_path_buf();
    world.last = Some(spawn("store-write", &index, &files));
}

#[when(expr = "another process opens the newest complete generation and lists its files")]
fn another_process_lists_files(world: &mut CodetagsWorld) {
    let index = world.store().index().to_path_buf();
    world.last = Some(spawn("store-list", &index, ""));
}

#[when(expr = "the reader switches to the newest complete generation")]
fn the_reader_switches(world: &mut CodetagsWorld) {
    let reader = world.reader.as_ref().expect("a reader was opened");
    world.held = Some(reader.current());
    assert!(reader.refresh().expect("refresh"), "no newer generation");
}

#[when(expr = "the index is garbage-collected")]
fn the_index_is_collected(world: &mut CodetagsWorld) {
    let report = world.store().gc().expect("gc");
    assert!(
        report.deferred.is_empty(),
        "deferred: {:?}",
        report.deferred
    );
}

#[then(expr = "that process completes generation {int}")]
fn that_process_completes(world: &mut CodetagsWorld, number: u64) {
    let last = world.last();
    assert_eq!(last.status, Some(0), "child failed: {}", last.stderr);
    assert_eq!(last.stdout.trim_end(), number.to_string());
}

#[then(expr = "that process fails with an error matching {string}")]
fn that_process_fails(world: &mut CodetagsWorld, pattern: String) {
    let last = world.last();
    assert_eq!(
        last.status,
        Some(FAILED),
        "expected failure; stdout: {} stderr: {}",
        last.stdout,
        last.stderr
    );
    let regex = regex::Regex::new(&pattern).expect("valid regex");
    assert!(
        regex.is_match(&last.stderr),
        "stderr {:?} does not match {pattern:?}",
        last.stderr
    );
}

#[then(expr = "the reader reads the files {string}")]
fn the_reader_reads(world: &mut CodetagsWorld, expected: String) {
    let reader = world.reader.as_ref().expect("a reader was opened");
    assert_eq!(files_of(&reader.current()).expect("read files"), expected);
}

#[then(expr = "the generation the reader held before the switch reads the files {string}")]
fn the_held_generation_reads(world: &mut CodetagsWorld, expected: String) {
    let held: &Arc<Generation> = world.held.as_ref().expect("the reader switched");
    assert_eq!(files_of(held).expect("read files"), expected);
}

#[then(expr = "the index holds only the generations {string}")]
fn the_index_holds_only(world: &mut CodetagsWorld, expected: String) {
    let store = world.store();
    let expected: Vec<u64> = expected
        .split_whitespace()
        .map(|n| n.parse().expect("generation number"))
        .collect();
    assert_eq!(store.complete_generations().expect("list"), expected);
    for entry in std::fs::read_dir(store.index()).expect("read index") {
        let name = entry.expect("entry").file_name();
        let name = name.to_string_lossy();
        if let Some(number) = generation_of(&name) {
            assert!(expected.contains(&number), "{name} was not collected");
        }
    }
}
