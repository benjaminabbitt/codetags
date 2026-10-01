//! The schema of one index generation (brief §4.4, PLAN.md §2.2).
//!
//! # Generations are rebuilt, never migrated
//!
//! A completed generation is immutable. When the schema changes,
//! [`SCHEMA_VERSION`] changes with it, and the next indexing run writes a new
//! generation under the new schema. Nothing ever upgrades an old file in
//! place: a reader that finds any other version refuses the generation with
//! [`StoreError::UnknownSchemaVersion`], and the fix is to reindex.
//!
//! # Tables
//!
//! - `schema_info`: one row holding the generation's schema version.
//! - `run`: one row per provider run, with its per-file edge counts in
//!   `run_file`. This is the generation's metadata (PLAN.md §2.2).
//! - `file`: every indexed source file.
//! - `symbol`, `call_site`, `call_target`: the brief's call graph (§4.4).

use duckdb::{Connection, OptionalExt};

use crate::StoreError;

/// The schema version this build writes, and the only one it reads.
///
/// Bump it on any change to [`SCHEMA_SQL`]; old generations are then rebuilt,
/// not migrated.
///
/// - 1: P1.1, the brief's sketch plus `run`, `run_file` and `file`.
/// - 2: P1.3, SCIP ingest: canonical names, module ancestors, the `external`
///   flag and package on `symbol`; the run, column, reference kind and
///   constraints on `call_site`.
pub const SCHEMA_VERSION: u32 = 2;

/// The DDL of one generation, without the `schema_info` row.
pub const SCHEMA_SQL: &str = "
CREATE TABLE schema_info (
  schema_version INTEGER NOT NULL
);

-- One row per provider run that produced this generation.
CREATE TABLE run (
  run_id           INTEGER PRIMARY KEY,
  provider         TEXT NOT NULL,         -- e.g. rust-analyzer-scip, scip-go
  provider_version TEXT NOT NULL,
  args             TEXT[] NOT NULL,
  started_at       TIMESTAMPTZ NOT NULL,
  finished_at      TIMESTAMPTZ,
  status           TEXT NOT NULL CHECK (status IN ('succeeded', 'failed')),
  -- The git tree SHA when available, otherwise a content hash of the tree.
  source_tree      TEXT NOT NULL
);

-- Per-file edge counts of each run, for the edge-count regression (P1.8).
CREATE TABLE run_file (
  run_id     INTEGER NOT NULL REFERENCES run,
  path       TEXT NOT NULL,
  edge_count BIGINT NOT NULL,
  PRIMARY KEY (run_id, path)
);

-- Every indexed source file. `path` is project-relative, with `/` separators.
CREATE TABLE file (
  path         TEXT PRIMARY KEY,
  language     TEXT,
  content_hash TEXT NOT NULL,
  size         BIGINT NOT NULL
);

-- Every project definition, and every symbol a call site targets.
CREATE TABLE symbol (
  id         TEXT PRIMARY KEY,  -- the exact SCIP symbol
  -- The canonical dotted name (PLAN.md §2.7, D15). Not unique: two symbols
  -- with one name are a reported collision, never merged.
  name       TEXT NOT NULL,
  lang       TEXT,
  kind       TEXT NOT NULL,     -- e.g. function, method, trait_method, struct, module, macro
  file       TEXT,              -- NULL when the project holds no definition of it
  start_line INT,               -- 1-based, of the definition's enclosing range
  end_line   INT,
  signature  TEXT,
  module     TEXT,              -- innermost dotted module; NULL at a package root
  modules    TEXT[] NOT NULL,   -- every ancestor module, outermost first
  package    TEXT,              -- the SCIP package name
  external   BOOLEAN NOT NULL   -- belongs to a package the index defines nothing in
);

CREATE TABLE call_site (
  site_id         BIGINT PRIMARY KEY,
  run_id          INTEGER NOT NULL REFERENCES run,
  caller          TEXT NOT NULL,  -- symbol id of the innermost enclosing definition
  file            TEXT NOT NULL,
  line            INT NOT NULL,   -- 1-based
  col             INT NOT NULL,   -- 1-based, in the provider's position encoding
  ordinal         INT NOT NULL,   -- 1-based position among the caller's sites
  -- call: followed by an argument list; value: a callable used as a value
  -- (a callback, a function pointer); macro: a macro invocation.
  ref_kind        TEXT NOT NULL CHECK (ref_kind IN ('call', 'value', 'macro')),
  receiver_text   TEXT,
  declared_target TEXT NOT NULL,
  dispatch        TEXT NOT NULL
                  CHECK (dispatch IN ('static', 'virtual', 'dynamic', 'unknown')),
  source          TEXT NOT NULL   -- scip@<indexer>-<version> | lsp@<ver> | treesitter | callgraph
);

CREATE TABLE call_target (
  site_id BIGINT NOT NULL REFERENCES call_site,
  target  TEXT NOT NULL,
  method  TEXT NOT NULL CHECK (method IN
            ('declared', 'cha', 'rta', 'vta', 'jelly', 'di-binding', 'name-match'))
);
";

/// Creates the schema in an empty database and records [`SCHEMA_VERSION`].
pub fn create(db: &Connection) -> Result<(), StoreError> {
    db.execute_batch(SCHEMA_SQL)?;
    db.execute(
        "INSERT INTO schema_info (schema_version) VALUES (?)",
        [SCHEMA_VERSION],
    )?;
    Ok(())
}

/// Checks that `db` records exactly [`SCHEMA_VERSION`].
///
/// A generation with no `schema_info` row, or with another version, is
/// refused with [`StoreError::UnknownSchemaVersion`].
pub fn check(db: &Connection) -> Result<(), StoreError> {
    let has_table: bool = db.query_row(
        "SELECT count(*) > 0 FROM duckdb_tables() WHERE table_name = 'schema_info'",
        [],
        |row| row.get(0),
    )?;
    let found: Option<i64> = if has_table {
        db.query_row("SELECT schema_version FROM schema_info", [], |row| {
            row.get(0)
        })
        .optional()?
    } else {
        None
    };
    if found == Some(i64::from(SCHEMA_VERSION)) {
        Ok(())
    } else {
        Err(StoreError::UnknownSchemaVersion {
            found,
            supported: SCHEMA_VERSION,
        })
    }
}

#[cfg(test)]
mod tests {
    use duckdb::Connection;

    use super::{SCHEMA_VERSION, check, create};
    use crate::StoreError;

    fn fresh() -> Connection {
        let db = Connection::open_in_memory().expect("open");
        create(&db).expect("create schema");
        db
    }

    #[test]
    fn a_fresh_schema_records_the_current_version() {
        let db = fresh();
        let version: i64 = db
            .query_row("SELECT schema_version FROM schema_info", [], |r| r.get(0))
            .expect("read version");
        assert_eq!(version, i64::from(SCHEMA_VERSION));
        check(&db).expect("current version is accepted");
    }

    #[test]
    fn every_table_exists() {
        let db = fresh();
        for table in [
            "schema_info",
            "run",
            "run_file",
            "file",
            "symbol",
            "call_site",
            "call_target",
        ] {
            db.execute_batch(&format!("SELECT * FROM {table}"))
                .unwrap_or_else(|e| panic!("table {table}: {e}"));
        }
    }

    #[test]
    fn a_run_records_its_metadata_and_edge_counts() {
        let db = fresh();
        db.execute_batch(
            "INSERT INTO run VALUES (1, 'rust-analyzer-scip', '2025-12-01', ['scip', '.'],
                 '2026-09-30 10:00:00+00', '2026-09-30 10:01:00+00', 'succeeded', 'abc123');
             INSERT INTO run_file VALUES (1, 'src/lib.rs', 42);",
        )
        .expect("insert run");
        let edges: i64 = db
            .query_row(
                "SELECT edge_count FROM run_file WHERE run_id = 1",
                [],
                |r| r.get(0),
            )
            .expect("read edge count");
        assert_eq!(edges, 42);
        assert!(
            db.execute_batch("INSERT INTO run VALUES (2, 'p', 'v', [], now(), NULL, 'bogus', 't')")
                .is_err(),
            "status is constrained"
        );
    }

    #[test]
    fn call_sites_are_constrained() {
        let db = fresh();
        db.execute_batch(
            "INSERT INTO run VALUES (1, 'p', 'v', [], now(), now(), 'succeeded', 't');
             INSERT INTO symbol (id, name, kind, modules, external)
                 VALUES ('s f().', 'f', 'function', ['m'], false);
             INSERT INTO call_site VALUES
                 (1, 1, 's f().', 'a.rs', 1, 1, 1, 'call', NULL, 's f().', 'static', 'scip@x');
             INSERT INTO call_target VALUES (1, 's f().', 'declared');",
        )
        .expect("a valid call site");
        for (column, bad) in [("ref_kind", "jump"), ("dispatch", "maybe")] {
            let (ref_kind, dispatch) = if column == "ref_kind" {
                (bad, "static")
            } else {
                ("call", bad)
            };
            let insert = format!(
                "INSERT INTO call_site VALUES
                     (2, 1, 's f().', 'a.rs', 1, 1, 1, '{ref_kind}', NULL, 's f().', '{dispatch}', 'x')"
            );
            assert!(db.execute_batch(&insert).is_err(), "{column} = {bad}");
        }
        assert!(
            db.execute_batch("INSERT INTO call_target VALUES (1, 's f().', 'guess')")
                .is_err(),
            "method is constrained"
        );
        assert!(
            db.execute_batch(
                "INSERT INTO symbol (id, name, kind, modules, external)
                     VALUES ('s g().', NULL, 'function', [], false)"
            )
            .is_err(),
            "every symbol has a canonical name"
        );
    }

    #[test]
    fn another_version_is_refused() {
        let db = fresh();
        db.execute_batch("UPDATE schema_info SET schema_version = 999")
            .expect("update");
        match check(&db) {
            Err(StoreError::UnknownSchemaVersion {
                found: Some(999),
                supported: SCHEMA_VERSION,
            }) => {}
            other => panic!("expected an unknown-version error, got {other:?}"),
        }
    }

    #[test]
    fn a_database_without_a_version_is_refused() {
        let db = Connection::open_in_memory().expect("open");
        match check(&db) {
            Err(StoreError::UnknownSchemaVersion { found: None, .. }) => {}
            other => panic!("expected an unknown-version error, got {other:?}"),
        }
        db.execute_batch("CREATE TABLE schema_info (schema_version INTEGER NOT NULL)")
            .expect("create empty schema_info");
        assert!(matches!(
            check(&db),
            Err(StoreError::UnknownSchemaVersion { found: None, .. })
        ));
    }

    #[test]
    fn the_refusal_names_both_versions() {
        let message = StoreError::UnknownSchemaVersion {
            found: Some(999),
            supported: 1,
        }
        .to_string();
        assert!(message.contains("schema version 999"), "{message}");
        assert!(message.contains("reads only schema version 1"), "{message}");
    }
}
