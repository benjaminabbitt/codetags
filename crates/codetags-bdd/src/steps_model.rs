//! Steps for the DuckDB model layer (codetags-model).

use cucumber::{given, then, when};

use crate::CodetagsWorld;
use crate::child::{self, FAILED, Job};

/// Returns `name` if it is a safe SQL identifier (`[a-z_]+`); panics otherwise.
/// Table names come from feature files, so this keeps them out of SQL syntax.
pub(crate) fn identifier(name: &str) -> &str {
    assert!(
        !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
        "table name {name:?} must match [a-z_]+"
    );
    name
}

#[given(expr = "a DuckDB database with a table {string} holding the rows {string}")]
fn a_duckdb_database(world: &mut CodetagsWorld, table: String, rows: String) {
    let table = identifier(&table);
    let path = world.scratch().join("gen.duckdb");
    let db = codetags_model::open_read_write(&path).expect("create database");
    db.execute_batch(&format!("CREATE TABLE {table} (v TEXT)"))
        .expect("create table");
    for row in rows.split_whitespace() {
        db.execute(&format!("INSERT INTO {table} VALUES (?)"), [row])
            .expect("insert row");
    }
    drop(db); // release the write lock before another process opens the file
    world.database = Some(path);
}

#[when(expr = "another process opens it read-only and lists table {string}")]
fn another_process_lists(world: &mut CodetagsWorld, table: String) {
    let database = world
        .database
        .clone()
        .expect("a Given step created a database");
    world.last = Some(child::spawn(&database, Job::List { table: &table }));
}

#[when(expr = "another process opens it read-only and inserts {string} into table {string}")]
fn another_process_inserts(world: &mut CodetagsWorld, value: String, table: String) {
    let database = world
        .database
        .clone()
        .expect("a Given step created a database");
    world.last = Some(child::spawn(
        &database,
        Job::Insert {
            table: &table,
            value: &value,
        },
    ));
}

#[then(expr = "that process reads exactly {string}")]
fn that_process_reads(world: &mut CodetagsWorld, expected: String) {
    let last = world.last();
    assert_eq!(last.status, Some(0), "child failed: {}", last.stderr);
    assert_eq!(last.stdout.trim_end(), expected);
}

#[then(expr = "that process's write is refused")]
fn that_process_write_is_refused(world: &mut CodetagsWorld) {
    let last = world.last();
    assert_eq!(
        last.status,
        Some(FAILED),
        "expected the insert to fail; stdout: {} stderr: {}",
        last.stdout,
        last.stderr
    );
}
