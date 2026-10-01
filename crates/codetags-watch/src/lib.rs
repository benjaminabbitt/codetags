//! File watching and coalescing (brief §4.3, PLAN.md C2, P4.1, P4.2).
//!
//! The [`Coalescer`] merges watcher events per path and decides when to
//! flush them as a [`Batch`].

pub mod change;
pub mod coalesce;

pub use change::{Batch, Change, ChangeKind, RescanCause, merge};
pub use coalesce::{CoalesceConfig, Coalescer};
