//! File watching and coalescing (brief §4.3, PLAN.md C2, P4.1, P4.2).
//!
//! [`Watcher::start`] watches a project root with notify, the unprivileged
//! baseline on every OS (inotify, FSEvents, ReadDirectoryChangesW), and
//! sends [`Batch`]es of project-relative changes that the [`Coalescer`] has
//! merged and timed:
//!
//! ```no_run
//! # fn main() -> Result<(), codetags_watch::WatchError> {
//! use codetags_watch::{WatchConfig, Watcher};
//!
//! let watcher = Watcher::start("/path/to/project", WatchConfig::default())?;
//! loop {
//!     let batch = watcher.recv()?;
//!     for change in &batch.changes {
//!         println!("{} {}", change.kind, change.path.display());
//!     }
//! }
//! # }
//! ```

pub mod change;
pub mod coalesce;
mod error;
pub mod exclude;
mod snapshot;
mod watcher;

pub use change::{Batch, Change, ChangeKind, RescanCause, merge};
pub use coalesce::{CoalesceConfig, Coalescer};
pub use error::WatchError;
pub use watcher::{WatchConfig, Watcher};
