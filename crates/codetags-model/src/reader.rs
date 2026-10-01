//! Reading complete generations, and switching between them.

use std::sync::{Arc, Mutex, PoisonError, RwLock};

use duckdb::Connection;

use crate::{GenerationStore, StoreError};

/// One complete generation, open read-only.
///
/// Its database stays open until the last clone of the `Arc` holding it, and
/// every connection from [`connect`](Self::connect), is dropped. So a handle
/// taken while generation N was current reads entirely from N, whatever
/// generations are completed after it.
#[derive(Debug)]
pub struct Generation {
    number: u64,
    db: Mutex<Connection>,
}

impl Generation {
    /// Wraps a read-only connection whose schema version has been checked.
    pub(crate) fn new(number: u64, db: Connection) -> Self {
        Self {
            number,
            db: Mutex::new(db),
        }
    }

    /// This generation's number.
    pub fn number(&self) -> u64 {
        self.number
    }

    /// A new read-only connection to this generation's database, for use on
    /// one thread. Connections share the open database, so this is cheap.
    pub fn connect(&self) -> Result<Connection, StoreError> {
        let db = self.db.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(db.try_clone()?)
    }
}

/// Serves the newest complete generation, and switches to a newer one on
/// [`refresh`](Self::refresh) by swapping an `Arc` (PLAN.md §2.2).
///
/// Handles already taken with [`current`](Self::current) keep serving the
/// generation they were taken from until they are dropped.
#[derive(Debug)]
pub struct GenerationReader {
    store: GenerationStore,
    current: RwLock<Arc<Generation>>,
}

impl GenerationReader {
    /// A reader that starts at `current`.
    pub(crate) fn new(store: GenerationStore, current: Generation) -> Self {
        Self {
            store,
            current: RwLock::new(Arc::new(current)),
        }
    }

    /// The generation this reader currently serves.
    pub fn current(&self) -> Arc<Generation> {
        Arc::clone(&self.current.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// Switches to the newest complete generation if it is newer than the
    /// current one. Returns whether it switched.
    pub fn refresh(&self) -> Result<bool, StoreError> {
        let current = self.current().number();
        let newest = self.store.complete_generations()?.last().copied();
        match newest {
            Some(newest) if newest > current => {
                let generation = Arc::new(self.store.open(newest)?);
                let mut slot = self.current.write().unwrap_or_else(PoisonError::into_inner);
                if slot.number() < generation.number() {
                    *slot = generation;
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}
