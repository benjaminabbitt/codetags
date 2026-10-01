//! Canonical symbol names, item ids, and collision suffixes (PLAN.md §2.7,
//! D15).
//!
//! Everything here is pure: no I/O and no host lookups.

pub mod canonical;
pub mod collision;
pub mod item_id;
mod percent;
pub mod scip;

pub use percent::PercentError;
