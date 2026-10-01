//! Errors from the schema and the generation store.

use std::fmt;
use std::io;
use std::path::PathBuf;

/// Why a schema or generation-store operation failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum StoreError {
    /// DuckDB reported an error.
    Duckdb(duckdb::Error),
    /// A filesystem operation on `path` failed.
    Io {
        /// The file or directory involved.
        path: PathBuf,
        /// The underlying error.
        source: io::Error,
    },
    /// A generation's schema version is not the one this build reads.
    ///
    /// Generations are rebuilt, never migrated, so the fix is to reindex.
    UnknownSchemaVersion {
        /// The version the generation records; `None` if it records none.
        found: Option<i64>,
        /// The only version this build reads ([`crate::SCHEMA_VERSION`]).
        supported: u32,
    },
    /// Another writer holds the index's `write.lock`. Concurrent writers are
    /// not supported.
    WriterBusy {
        /// The lock file.
        lock: PathBuf,
    },
    /// The index holds no complete generation.
    NoGeneration {
        /// The index directory.
        index: PathBuf,
    },
    /// The generation has no completion marker: it is still being written,
    /// its writer died, or GC has begun to remove it.
    NotComplete {
        /// The index directory.
        index: PathBuf,
        /// The generation asked for.
        number: u64,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duckdb(error) => write!(f, "duckdb: {error}"),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::UnknownSchemaVersion {
                found: Some(found),
                supported,
            } => write!(
                f,
                "unknown schema version {found}: this build reads only schema version \
                 {supported}; generations are rebuilt, not migrated, so reindex"
            ),
            Self::UnknownSchemaVersion {
                found: None,
                supported,
            } => write!(
                f,
                "the generation records no schema version: this build reads only schema \
                 version {supported}; generations are rebuilt, not migrated, so reindex"
            ),
            Self::WriterBusy { lock } => write!(
                f,
                "another writer holds {}; concurrent writers are not supported",
                lock.display()
            ),
            Self::NoGeneration { index } => {
                write!(f, "{} holds no complete generation", index.display())
            }
            Self::NotComplete { index, number } => write!(
                f,
                "generation {number} in {} is not complete",
                index.display()
            ),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Duckdb(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<duckdb::Error> for StoreError {
    fn from(error: duckdb::Error) -> Self {
        Self::Duckdb(error)
    }
}

impl StoreError {
    /// Wraps an I/O error with the path it concerns.
    pub(crate) fn io(path: impl Into<PathBuf>) -> impl FnOnce(io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io { path, source }
    }
}
