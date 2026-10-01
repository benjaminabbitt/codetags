//! Canonical symbol names, the Windows private-use mapping, item ids, and
//! collision suffixes (PLAN.md §2.3, §2.7, D15).
//!
//! Everything here is pure: no I/O and no host lookups, so every rule is
//! tested on every host.

pub mod canonical;
pub mod collision;
pub mod item_id;
pub mod scip;
pub mod windows;
