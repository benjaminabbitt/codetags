//! The generation store over an index directory (PLAN.md §2.2).
//!
//! Layout of the index directory, normally `.codetags/index/`:
//!
//! ```text
//! write.lock          the writer's OS file lock; never deleted
//! gen-<N>.duckdb      generation N's database (plus DuckDB side files)
//! gen-<N>.complete    written last; a generation without it is incomplete
//! ```
//!
//! Generation numbers start at 1 and only grow: a writer takes one more than
//! the highest number any file in the directory carries, complete or not, so
//! a number is never reused. File names carry the number in plain decimal and
//! listings sort by the parsed number (see `names`).

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use crate::names::{self, COMPLETE, DATABASE};
use crate::reader::{Generation, GenerationReader};
use crate::writer::{GenerationWriter, WriteLock};
use crate::{StoreError, schema};

/// How many complete generations GC keeps: the newest, and the previous one
/// for the edge-count regression diff (PLAN.md §2.2, P1.8).
const KEEP: usize = 2;

/// How often [`GenerationStore::open_newest`] re-lists the directory when the
/// newest generation vanishes between listing and opening it.
const OPEN_ATTEMPTS: usize = 3;

/// The generations in one index directory, normally `.codetags/index/`.
///
/// Concurrent writers are not supported: [`begin`](Self::begin) and
/// [`gc`](Self::gc) take the index's `write.lock` and fail with
/// [`StoreError::WriterBusy`] while another handle, in any process, holds it.
/// Readers take no lock, and any number of processes may read at once.
#[derive(Debug, Clone)]
pub struct GenerationStore {
    index: PathBuf,
}

/// What [`GenerationStore::gc`] did.
#[derive(Debug, Default)]
pub struct GcReport {
    /// Complete generations kept, ascending.
    pub kept: Vec<u64>,
    /// Generations whose files were all removed, ascending.
    pub removed: Vec<u64>,
    /// Files that could not be removed, such as a database another process
    /// still has open on Windows. The next GC retries them.
    pub deferred: Vec<(PathBuf, io::Error)>,
}

/// The files of one generation found in the index directory.
#[derive(Debug, Default)]
struct Entry {
    has_database: bool,
    has_marker: bool,
    files: Vec<PathBuf>,
}

impl Entry {
    fn is_complete(&self) -> bool {
        self.has_database && self.has_marker
    }
}

impl GenerationStore {
    /// A store over the index directory `index`, which need not exist yet.
    pub fn new(index: impl Into<PathBuf>) -> Self {
        Self {
            index: index.into(),
        }
    }

    /// The index directory.
    pub fn index(&self) -> &Path {
        &self.index
    }

    /// Begins writing the next generation, N+1 where N is the highest number
    /// in the directory, into a new `gen-<N+1>.duckdb` with the current
    /// schema. No other process has that file open: readers open only
    /// complete generations, and the returned writer holds `write.lock`.
    ///
    /// Fails with [`StoreError::WriterBusy`] if another writer holds the lock.
    pub fn begin(&self) -> Result<GenerationWriter, StoreError> {
        let lock = WriteLock::acquire(&self.index)?;
        let number = self.scan()?.keys().next_back().map_or(1, |n| n + 1);
        GenerationWriter::create(&self.index, number, lock)
    }

    /// The complete generations, ascending. An index directory that does not
    /// exist holds none.
    pub fn complete_generations(&self) -> Result<Vec<u64>, StoreError> {
        Ok(self
            .scan()?
            .into_iter()
            .filter(|(_, entry)| entry.is_complete())
            .map(|(number, _)| number)
            .collect())
    }

    /// Opens complete generation `number` read-only, checking its schema
    /// version.
    ///
    /// Fails with [`StoreError::NotComplete`] if the generation has no
    /// completion marker, and with [`StoreError::UnknownSchemaVersion`] if it
    /// was written under another schema.
    pub fn open(&self, number: u64) -> Result<Generation, StoreError> {
        let marker = self.index.join(names::file_name(number, COMPLETE));
        if !marker.is_file() {
            return Err(StoreError::NotComplete {
                index: self.index.clone(),
                number,
            });
        }
        let db = crate::open_read_only(&self.index.join(names::file_name(number, DATABASE)))?;
        schema::check(&db)?;
        Ok(Generation::new(number, db))
    }

    /// Opens the newest complete generation read-only. Incomplete
    /// generations, such as one whose writer crashed, are never opened.
    ///
    /// Fails with [`StoreError::NoGeneration`] if there is none.
    pub fn open_newest(&self) -> Result<Generation, StoreError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let Some(&newest) = self.complete_generations()?.last() else {
                return Err(StoreError::NoGeneration {
                    index: self.index.clone(),
                });
            };
            match self.open(newest) {
                // A concurrent GC removed it after we listed it; list again.
                Err(StoreError::NotComplete { .. }) if attempt < OPEN_ATTEMPTS => {}
                result => return result,
            }
        }
    }

    /// A reader serving the newest complete generation, which can switch to a
    /// newer one later ([`GenerationReader::refresh`]).
    pub fn reader(&self) -> Result<GenerationReader, StoreError> {
        Ok(GenerationReader::new(self.clone(), self.open_newest()?))
    }

    /// Keeps the newest two complete generations and deletes every other
    /// generation's files, including incomplete generations left by a crashed
    /// writer.
    ///
    /// GC takes `write.lock`, so it never deletes a generation still being
    /// written; it fails with [`StoreError::WriterBusy`] while a writer runs.
    /// A file that cannot be deleted (on Windows, a database another process
    /// still has open) is reported in [`GcReport::deferred`] and retried at
    /// the next GC. Each generation loses its completion marker first, so no
    /// reader opens it once its deletion has started.
    pub fn gc(&self) -> Result<GcReport, StoreError> {
        self.gc_with(&mut remove)
    }

    /// [`gc`](Self::gc), deleting each path with `remove`.
    fn gc_with(
        &self,
        remove: &mut dyn FnMut(&Path) -> io::Result<()>,
    ) -> Result<GcReport, StoreError> {
        let _lock = WriteLock::acquire(&self.index)?;
        let entries = self.scan()?;
        let complete: Vec<u64> = entries
            .iter()
            .filter(|(_, entry)| entry.is_complete())
            .map(|(&number, _)| number)
            .collect();
        let kept = complete[complete.len().saturating_sub(KEEP)..].to_vec();
        let mut report = GcReport {
            kept,
            ..GcReport::default()
        };
        for (number, entry) in entries {
            if report.kept.contains(&number) {
                continue;
            }
            let marker = self.index.join(names::file_name(number, COMPLETE));
            let (markers, rest): (Vec<_>, Vec<_>) =
                entry.files.into_iter().partition(|path| *path == marker);
            let mut removed_all = true;
            for path in markers.into_iter().chain(rest) {
                if let Err(error) = remove(&path) {
                    let marker_failed = path == marker;
                    report.deferred.push((path, error));
                    removed_all = false;
                    if marker_failed {
                        // Still complete; leave its database for the next GC.
                        break;
                    }
                }
            }
            if removed_all {
                report.removed.push(number);
            }
        }
        Ok(report)
    }

    /// Every generation that has at least one file in the index directory.
    fn scan(&self) -> Result<BTreeMap<u64, Entry>, StoreError> {
        let mut entries: BTreeMap<u64, Entry> = BTreeMap::new();
        let listing = match std::fs::read_dir(&self.index) {
            Ok(listing) => listing,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(entries),
            Err(error) => return Err(StoreError::io(&self.index)(error)),
        };
        for item in listing {
            let item = item.map_err(StoreError::io(&self.index))?;
            let name = item.file_name();
            let Some((number, suffix)) = name.to_str().and_then(names::parse) else {
                continue;
            };
            let entry = entries.entry(number).or_default();
            entry.has_database |= suffix == DATABASE;
            entry.has_marker |= suffix == COMPLETE;
            entry.files.push(item.path());
        }
        Ok(entries)
    }
}

/// Deletes `path`, a file or a directory (DuckDB's `.tmp` spill directory).
/// A path that is already gone counts as deleted.
fn remove(path: &Path) -> io::Result<()> {
    let result = match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(error) => Err(error),
    };
    match result {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::Path;

    use super::GenerationStore;
    use crate::StoreError;

    fn store() -> (tempfile::TempDir, GenerationStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = GenerationStore::new(dir.path().join("index"));
        (dir, store)
    }

    /// Writes and completes a generation recording `path` in its `file` table.
    fn write(store: &GenerationStore, path: &str) -> u64 {
        let writer = store.begin().expect("begin");
        writer
            .connection()
            .execute(
                "INSERT INTO file (path, content_hash, size) VALUES (?, 'h', 0)",
                [path],
            )
            .expect("insert");
        writer.complete().expect("complete")
    }

    fn files(generation: &crate::Generation) -> Vec<String> {
        let db = generation.connect().expect("connect");
        let mut statement = db
            .prepare("SELECT path FROM file ORDER BY path")
            .expect("prepare");
        statement
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows")
    }

    fn names(store: &GenerationStore) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(store.index())
            .expect("read index")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn an_empty_index_has_no_generation() {
        let (_dir, store) = store();
        assert_eq!(
            store.complete_generations().expect("list"),
            Vec::<u64>::new()
        );
        assert!(matches!(
            store.open_newest(),
            Err(StoreError::NoGeneration { .. })
        ));
    }

    #[test]
    fn generations_number_from_one_and_complete_with_a_marker() {
        let (_dir, store) = store();
        assert_eq!(write(&store, "a.rs"), 1);
        assert_eq!(write(&store, "b.rs"), 2);
        assert_eq!(store.complete_generations().expect("list"), [1, 2]);
        assert_eq!(
            names(&store),
            [
                "gen-1.complete",
                "gen-1.duckdb",
                "gen-2.complete",
                "gen-2.duckdb",
                "write.lock"
            ]
        );
        let newest = store.open_newest().expect("open");
        assert_eq!(newest.number(), 2);
        assert_eq!(files(&newest), ["b.rs"]);
    }

    #[test]
    fn listing_sorts_numerically() {
        let (_dir, store) = store();
        for _ in 0..10 {
            write(&store, "a.rs");
        }
        assert_eq!(
            store.complete_generations().expect("list"),
            (1..=10).collect::<Vec<_>>()
        );
        assert_eq!(store.open_newest().expect("open").number(), 10);
    }

    #[test]
    fn a_generation_being_written_is_not_listed_or_opened() {
        let (_dir, store) = store();
        write(&store, "a.rs");
        let writer = store.begin().expect("begin");
        assert_eq!(writer.number(), 2);
        assert_eq!(store.complete_generations().expect("list"), [1]);
        assert_eq!(store.open_newest().expect("open").number(), 1);
        assert!(matches!(
            store.open(2),
            Err(StoreError::NotComplete { number: 2, .. })
        ));
    }

    #[test]
    fn a_second_writer_is_refused_until_the_first_finishes() {
        let (_dir, store) = store();
        let first = store.begin().expect("begin");
        match store.begin() {
            Err(StoreError::WriterBusy { lock }) => assert!(lock.ends_with("write.lock")),
            other => panic!("expected WriterBusy, got {other:?}"),
        }
        assert!(matches!(store.gc(), Err(StoreError::WriterBusy { .. })));
        first.complete().expect("complete");
        assert_eq!(store.begin().expect("begin again").number(), 2);
    }

    #[test]
    fn an_abandoned_generation_number_is_not_reused() {
        let (_dir, store) = store();
        write(&store, "a.rs");
        drop(store.begin().expect("begin"));
        assert_eq!(store.complete_generations().expect("list"), [1]);
        assert_eq!(write(&store, "c.rs"), 3);
    }

    #[test]
    fn an_unknown_schema_version_is_refused() {
        let (_dir, store) = store();
        let writer = store.begin().expect("begin");
        writer
            .connection()
            .execute_batch("UPDATE schema_info SET schema_version = 999")
            .expect("update");
        writer.complete().expect("complete");
        assert!(matches!(
            store.open_newest(),
            Err(StoreError::UnknownSchemaVersion {
                found: Some(999),
                ..
            })
        ));
    }

    #[test]
    fn a_reader_switches_while_old_handles_keep_their_generation() {
        let (_dir, store) = store();
        write(&store, "a.rs");
        let reader = store.reader().expect("reader");
        let old = reader.current();
        assert!(!reader.refresh().expect("refresh"), "nothing newer yet");
        write(&store, "b.rs");
        assert_eq!(files(&reader.current()), ["a.rs"]);
        assert!(reader.refresh().expect("refresh"));
        assert_eq!(reader.current().number(), 2);
        assert_eq!(files(&reader.current()), ["b.rs"]);
        assert_eq!(files(&old), ["a.rs"]);
    }

    #[test]
    fn gc_keeps_the_newest_two_and_removes_the_rest() {
        let (_dir, store) = store();
        for path in ["a.rs", "b.rs", "c.rs", "d.rs"] {
            write(&store, path);
        }
        drop(store.begin().expect("begin an incomplete generation 5"));
        let report = store.gc().expect("gc");
        assert_eq!(report.kept, [3, 4]);
        assert_eq!(report.removed, [1, 2, 5]);
        assert!(report.deferred.is_empty());
        assert_eq!(
            names(&store),
            [
                "gen-3.complete",
                "gen-3.duckdb",
                "gen-4.complete",
                "gen-4.duckdb",
                "write.lock"
            ]
        );
    }

    #[test]
    fn gc_with_one_generation_keeps_it() {
        let (_dir, store) = store();
        write(&store, "a.rs");
        assert_eq!(store.gc().expect("gc").kept, [1]);
        assert!(store.gc().expect("repeat gc").removed.is_empty());
    }

    #[test]
    fn gc_retries_a_failed_delete_at_the_next_gc() {
        let (_dir, store) = store();
        for path in ["a.rs", "b.rs", "c.rs"] {
            write(&store, path);
        }
        let mut refuse = |path: &Path| -> io::Result<()> {
            if path.ends_with("gen-1.duckdb") {
                Err(io::Error::other("open elsewhere"))
            } else {
                super::remove(path)
            }
        };
        let report = store.gc_with(&mut refuse).expect("gc");
        assert!(report.removed.is_empty());
        assert_eq!(report.deferred.len(), 1);
        assert!(report.deferred[0].0.ends_with("gen-1.duckdb"));
        // Its marker went first, so it is no longer complete or openable.
        assert_eq!(store.complete_generations().expect("list"), [2, 3]);

        let report = store.gc().expect("second gc");
        assert_eq!(report.removed, [1]);
        assert!(!store.index().join("gen-1.duckdb").exists());
    }

    #[test]
    fn a_failed_marker_delete_leaves_the_generation_whole() {
        let (_dir, store) = store();
        for path in ["a.rs", "b.rs", "c.rs"] {
            write(&store, path);
        }
        let mut refuse = |path: &Path| -> io::Result<()> {
            if path.ends_with("gen-1.complete") {
                Err(io::Error::other("denied"))
            } else {
                super::remove(path)
            }
        };
        let report = store.gc_with(&mut refuse).expect("gc");
        assert_eq!(report.deferred.len(), 1);
        assert_eq!(files(&store.open(1).expect("still complete")), ["a.rs"]);
    }

    #[test]
    fn stray_files_are_ignored() {
        let (_dir, store) = store();
        write(&store, "a.rs");
        std::fs::write(store.index().join("gen-01.duckdb"), b"").expect("write");
        std::fs::write(store.index().join("notes.txt"), b"").expect("write");
        assert_eq!(store.complete_generations().expect("list"), [1]);
        assert_eq!(write(&store, "b.rs"), 2);
        store.gc().expect("gc");
        assert!(store.index().join("notes.txt").exists());
    }
}
