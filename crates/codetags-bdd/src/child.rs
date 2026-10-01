//! Child-process mode: steps that need "another process" re-run the current
//! test executable with [`MODE_ENV`] set, and [`maybe_run_child`] turns that
//! process into a short-lived worker instead of a cucumber run.

use std::path::PathBuf;
use std::process::{Command, exit};

use crate::CommandOutcome;

/// Selects the child's job; unset in normal runs.
const MODE_ENV: &str = "CODETAGS_BDD_CHILD";
const DB_ENV: &str = "CODETAGS_BDD_DB";
const TABLE_ENV: &str = "CODETAGS_BDD_TABLE";
const VALUE_ENV: &str = "CODETAGS_BDD_VALUE";

/// Exit status a child uses when the operation it attempted failed.
pub(crate) const FAILED: i32 = 3;

/// A job a child process can do.
pub(crate) enum Job<'a> {
    /// Open the database read-only and print the table's `v` column, sorted.
    List { table: &'a str },
    /// Open the database read-only and try to insert `value`.
    Insert { table: &'a str, value: &'a str },
}

/// Runs `job` against `database` in a fresh process.
pub(crate) fn spawn(database: &std::path::Path, job: Job<'_>) -> CommandOutcome {
    let exe = std::env::current_exe().expect("locate the test executable");
    let mut command = Command::new(exe);
    command.env(DB_ENV, database);
    match job {
        Job::List { table } => command.env(MODE_ENV, "duckdb-list").env(TABLE_ENV, table),
        Job::Insert { table, value } => command
            .env(MODE_ENV, "duckdb-insert")
            .env(TABLE_ENV, table)
            .env(VALUE_ENV, value),
    };
    command.output().expect("spawn child process").into()
}

/// Runs the child job `mode` in a fresh process, with the extra environment
/// `env`. Other step modules define their modes in their own `child` function.
pub(crate) fn spawn_mode(mode: &str, env: &[(&str, &std::ffi::OsStr)]) -> CommandOutcome {
    let exe = std::env::current_exe().expect("locate the test executable");
    let mut command = Command::new(exe);
    command.env(MODE_ENV, mode).envs(env.iter().copied());
    command.output().expect("spawn child process").into()
}

/// If this process was started as a child, does its job and exits; otherwise
/// returns immediately. Call it first thing in the runner's `main`.
pub fn maybe_run_child() {
    let Ok(mode) = std::env::var(MODE_ENV) else {
        return;
    };
    let result = match mode.as_str() {
        "duckdb-list" | "duckdb-insert" => duckdb_job(&mode),
        store if store.starts_with(crate::steps_store::CHILD_PREFIX) => {
            crate::steps_store::child(store)
        }
        other => Err(format!("unknown child mode {other:?}")),
    };
    match result {
        Ok(stdout) => {
            println!("{stdout}");
            exit(0)
        }
        Err(error) => {
            eprintln!("{error}");
            exit(FAILED)
        }
    }
}

/// The DuckDB model-layer jobs (`steps_model`).
fn duckdb_job(mode: &str) -> Result<String, String> {
    let database = PathBuf::from(std::env::var(DB_ENV).expect("child needs a database path"));
    let table = std::env::var(TABLE_ENV).expect("child needs a table");
    let table = crate::steps_model::identifier(&table);
    if mode == "duckdb-list" {
        list(&database, table)
    } else {
        let value = std::env::var(VALUE_ENV).expect("child needs a value");
        insert(&database, table, &value)
    }
}

fn list(database: &std::path::Path, table: &str) -> Result<String, String> {
    let db = codetags_model::open_read_only(database).map_err(|e| e.to_string())?;
    let mut statement = db
        .prepare(&format!("SELECT v FROM {table} ORDER BY v"))
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let values: Result<Vec<String>, _> = rows.collect();
    Ok(values.map_err(|e| e.to_string())?.join(" "))
}

fn insert(database: &std::path::Path, table: &str, value: &str) -> Result<String, String> {
    let db = codetags_model::open_read_only(database).map_err(|e| e.to_string())?;
    db.execute(&format!("INSERT INTO {table} VALUES (?)"), [value])
        .map_err(|e| e.to_string())?;
    Ok("inserted".to_string())
}
