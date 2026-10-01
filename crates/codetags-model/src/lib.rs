//! DuckDB schema, migrations, and the immutable generation store (PLAN.md §2.2).
//!
//! DuckDB allows one read-write process or many read-only processes on a file,
//! never both (V7). The generation store is built on that rule: a generation is
//! written by exactly one process, then only ever opened read-only.

use std::path::Path;

use duckdb::{AccessMode, Config, Connection};

pub use duckdb::Error;

/// Opens the database at `path` for writing, creating it if absent.
///
/// Only one process may hold a database open for writing, and while it does,
/// no other process can open it at all.
pub fn open_read_write(path: &Path) -> Result<Connection, Error> {
    Connection::open(path)
}

/// Opens the existing database at `path` read-only.
///
/// Any number of processes may do this at once, provided none holds the file
/// open for writing. Writes through the connection fail.
pub fn open_read_only(path: &Path) -> Result<Connection, Error> {
    let config = Config::default().access_mode(AccessMode::ReadOnly)?;
    Connection::open_with_flags(path, config)
}

#[cfg(test)]
mod tests {
    use super::{open_read_only, open_read_write};

    #[test]
    fn a_closed_database_reopens_read_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("gen.duckdb");
        {
            let db = open_read_write(&path).expect("open read-write");
            db.execute_batch("CREATE TABLE t (v TEXT); INSERT INTO t VALUES ('x');")
                .expect("write");
        }
        let db = open_read_only(&path).expect("open read-only");
        let v: String = db
            .query_row("SELECT v FROM t", [], |row| row.get(0))
            .expect("read");
        assert_eq!(v, "x");
        assert!(db.execute_batch("INSERT INTO t VALUES ('y')").is_err());
    }

    #[test]
    fn read_only_open_of_a_missing_file_fails() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(open_read_only(&dir.path().join("absent.duckdb")).is_err());
    }
}
