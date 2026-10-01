#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, UNIX_EPOCH};

use codetags_model::Connection;

use super::*;
use crate::provider::scip::{Document, Occurrence, Range, SymbolInfo, SymbolKind};

const P: &str = "rust-analyzer cargo demo 0.1.0 ";

fn range(line: u32, start: u32, end_line: u32, end: u32) -> Range {
    Range {
        start_line: line,
        start_character: start,
        end_line,
        end_character: end,
    }
}

/// `src/a/b.rs`:
///
/// ```text
/// pub fn f() {
///     g();
/// }
/// ```
fn index() -> ScipIndex {
    let f = format!("{P}a/b/f().");
    let g = format!("{P}a/b/g().");
    ScipIndex {
        tool: "rust-analyzer 1.96.1 (31fca3a 2026-06-26)".into(),
        project_root: "file:///p".into(),
        external_symbols: vec![],
        documents: vec![Document {
            relative_path: "src/a/b.rs".into(),
            occurrences: vec![
                Occurrence {
                    symbol: f.clone(),
                    range: range(0, 7, 0, 8),
                    enclosing_range: Some(range(0, 0, 2, 1)),
                    roles: 1,
                },
                Occurrence {
                    symbol: g,
                    range: range(1, 4, 1, 5),
                    enclosing_range: None,
                    roles: 0,
                },
            ],
            symbols: vec![SymbolInfo {
                symbol: f,
                kind: Some(SymbolKind::Function),
                relationships: Vec::new(),
            }],
        }],
    }
}

const SOURCE: &str = "pub fn f() {\n    g();\n}\n";

fn run_info() -> RunInfo {
    RunInfo {
        provider: "rust-analyzer scip".into(),
        provider_version: "rust-analyzer 1.96.1 (31fca3a 2026-06-26)".into(),
        args: vec!["scip".into(), "/p".into()],
        started_at: UNIX_EPOCH + Duration::from_secs(1_790_000_000),
        finished_at: UNIX_EPOCH + Duration::from_secs(1_790_000_017),
    }
}

/// A fresh generation schema, and a project root holding `SOURCE`.
fn setup() -> (Connection, tempfile::TempDir) {
    let db = Connection::open_in_memory().unwrap();
    codetags_model::schema::create(&db).unwrap();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("src/a")).unwrap();
    std::fs::write(root.path().join("src/a/b.rs"), SOURCE).unwrap();
    (db, root)
}

fn ingest_once(db: &Connection, root: &Path) -> IngestReport {
    let index = index();
    ingest(
        db,
        &Input {
            root,
            index: &index,
            language: "rust",
            run: run_info(),
        },
    )
    .unwrap()
}

fn text(db: &Connection, sql: &str) -> String {
    db.query_row(sql, [], |row| row.get(0)).unwrap()
}

#[test]
fn a_run_writes_every_table() {
    let (db, root) = setup();
    let report = ingest_once(&db, root.path());
    assert_eq!(
        (
            report.run_id,
            report.files,
            report.symbols,
            report.call_sites
        ),
        (1, 1, 2, 1)
    );
    assert_eq!(report.source, "scip@rust-analyzer-1.96.1");
    assert_eq!(
        text(
            &db,
            "SELECT path || ' ' || language || ' ' || size::TEXT || ' ' || content_hash FROM file"
        ),
        format!(
            "src/a/b.rs rust {} sha256:{}",
            SOURCE.len(),
            // sha256 of SOURCE, from `sha256sum`.
            "36e9b1a81436fc5b1f52c2683a063f2db9670878704b14d13bae9d53794072a3"
        )
    );
}

#[test]
fn symbols_carry_names_modules_and_lines() {
    let (db, root) = setup();
    ingest_once(&db, root.path());
    assert_eq!(
        text(
            &db,
            "SELECT name || ' ' || kind || ' ' || file || ' ' || start_line::TEXT || '-'
                    || end_line::TEXT || ' ' || module || ' ' || modules::TEXT || ' '
                    || package || ' ' || external::TEXT
             FROM symbol WHERE name = 'demo.a.b.f'"
        ),
        "demo.a.b.f function src/a/b.rs 1-3 demo.a.b [demo.a, demo.a.b] demo false"
    );
    assert_eq!(
        text(
            &db,
            "SELECT name || ' ' || kind || ' ' || coalesce(file, 'NULL') || ' ' || external::TEXT
             FROM symbol WHERE name = 'demo.a.b.g'"
        ),
        "demo.a.b.g function NULL false"
    );
}

#[test]
fn call_sites_have_a_declared_target_and_provenance() {
    let (db, root) = setup();
    ingest_once(&db, root.path());
    assert_eq!(
        text(
            &db,
            "SELECT s.site_id::TEXT || ' ' || s.run_id::TEXT || ' ' || s.file || ':'
                    || s.line::TEXT || ':' || s.col::TEXT || ' #' || s.ordinal::TEXT || ' '
                    || s.ref_kind || ' ' || s.dispatch || ' ' || s.source || ' '
                    || t.method || ' ' || (t.target = s.declared_target)::TEXT
             FROM call_site s JOIN call_target t USING (site_id)"
        ),
        "1 1 src/a/b.rs:2:5 #1 call static scip@rust-analyzer-1.96.1 declared true"
    );
}

#[test]
fn the_run_records_metadata_and_edge_counts() {
    let (db, root) = setup();
    ingest_once(&db, root.path());
    assert_eq!(
        text(
            &db,
            "SELECT provider || ' | ' || provider_version || ' | ' || args::TEXT || ' | '
                    || epoch(finished_at - started_at)::TEXT || ' | ' || status
             FROM run"
        ),
        "rust-analyzer scip | rust-analyzer 1.96.1 (31fca3a 2026-06-26) | [scip, /p] | 17.0 | succeeded"
    );
    assert!(text(&db, "SELECT source_tree FROM run").starts_with("sha256:"));
    assert_eq!(
        text(
            &db,
            "SELECT path || ' ' || edge_count::TEXT FROM run_file WHERE run_id = 1"
        ),
        "src/a/b.rs 1"
    );
}

#[test]
fn a_second_run_in_one_generation_gets_new_ids() {
    let (db, root) = setup();
    ingest_once(&db, root.path());
    let second = ingest_once(&db, root.path());
    assert_eq!(second.run_id, 2);
    assert_eq!(
        text(
            &db,
            "SELECT string_agg(site_id::TEXT, ' ' ORDER BY site_id) FROM call_site"
        ),
        "1 2"
    );
}

#[test]
fn a_missing_source_file_is_an_error() {
    let (db, root) = setup();
    std::fs::remove_file(root.path().join("src/a/b.rs")).unwrap();
    let index = index();
    let error = ingest(
        &db,
        &Input {
            root: root.path(),
            index: &index,
            language: "rust",
            run: run_info(),
        },
    )
    .unwrap_err();
    assert!(
        matches!(&error, IngestError::Source { path, .. } if path == "src/a/b.rs"),
        "{error}"
    );
}

#[test]
fn source_tags_name_the_indexer_and_its_version() {
    assert_eq!(
        source_tag("rust-analyzer 1.96.1 (31fca3a 2026-06-26)"),
        "scip@rust-analyzer-1.96.1"
    );
    assert_eq!(source_tag("scip-go"), "scip@scip-go");
    assert_eq!(source_tag(""), "scip@unknown");
}
