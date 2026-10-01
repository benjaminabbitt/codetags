//! Writing one generation, under the index's write lock.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use duckdb::Connection;

use crate::names::{self, COMPLETE, DATABASE};
use crate::{SCHEMA_VERSION, StoreError};

/// The index's exclusive writer lock, `write.lock`.
///
/// The lock is an OS file lock (`File::try_lock`: `flock` on Unix,
/// `LockFileEx` on Windows, V32), so the OS releases it when its holder
/// exits, even after a crash. The file itself is never deleted: deleting a
/// lock file races with a process that has just opened it.
#[derive(Debug)]
pub(crate) struct WriteLock {
    _file: File,
}

impl WriteLock {
    /// Takes the lock in `index`, creating the directory and the file if
    /// needed. Fails with [`StoreError::WriterBusy`] if another handle, in
    /// this or any other process, holds it.
    pub(crate) fn acquire(index: &Path) -> Result<Self, StoreError> {
        std::fs::create_dir_all(index).map_err(StoreError::io(index))?;
        let path = index.join(names::WRITE_LOCK);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(StoreError::io(&path))?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => Err(StoreError::WriterBusy { lock: path }),
            Err(std::fs::TryLockError::Error(source)) => Err(StoreError::Io { path, source }),
        }
    }
}

/// A generation being written. Obtain one from
/// [`GenerationStore::begin`](crate::GenerationStore::begin).
///
/// The writer holds the index's write lock for its whole life, and is the only
/// process that ever opens its database read-write. Fill the database through
/// [`connection`](Self::connection), then call [`complete`](Self::complete).
/// A writer dropped without completing, or a process that dies mid-write,
/// leaves an incomplete generation: readers never open it, and the next GC
/// deletes it.
#[derive(Debug)]
pub struct GenerationWriter {
    number: u64,
    index: PathBuf,
    db: Connection,
    _lock: WriteLock,
}

impl GenerationWriter {
    /// Creates generation `number`'s database in `index` with the current
    /// schema. The caller holds `lock` and has checked that the number is
    /// unused.
    pub(crate) fn create(index: &Path, number: u64, lock: WriteLock) -> Result<Self, StoreError> {
        let path = index.join(names::file_name(number, DATABASE));
        let db = crate::open_read_write(&path)?;
        crate::schema::create(&db)?;
        Ok(Self {
            number,
            index: index.to_path_buf(),
            db,
            _lock: lock,
        })
    }

    /// This generation's number.
    pub fn number(&self) -> u64 {
        self.number
    }

    /// The read-write connection to this generation's database.
    pub fn connection(&self) -> &Connection {
        &self.db
    }

    /// Closes the database, makes it durable, then writes the completion
    /// marker `gen-<N>.complete` last. From then on the generation is
    /// immutable and readers may open it. Returns the generation's number.
    pub fn complete(self) -> Result<u64, StoreError> {
        let Self {
            number,
            index,
            db,
            _lock,
        } = self;
        db.execute_batch("CHECKPOINT")?;
        db.close().map_err(|(_, error)| error)?;

        let database = index.join(names::file_name(number, DATABASE));
        sync_file(&database)?;
        sync_dir(&index)?;

        let marker = index.join(names::file_name(number, COMPLETE));
        let mut file = File::create(&marker).map_err(StoreError::io(&marker))?;
        writeln!(file, "schema_version {SCHEMA_VERSION}").map_err(StoreError::io(&marker))?;
        file.sync_all().map_err(StoreError::io(&marker))?;
        sync_dir(&index)?;
        Ok(number)
    }
}

/// Flushes `path`'s contents to disk. Windows needs write access to flush.
fn sync_file(path: &Path) -> Result<(), StoreError> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(StoreError::io(path))
}

/// Makes `dir`'s entries durable. Only Unix can open a directory to sync it;
/// on Windows a rename or create is durable once the file is flushed.
fn sync_dir(dir: &Path) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        File::open(dir)
            .and_then(|file| file.sync_all())
            .map_err(StoreError::io(dir))?;
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}
