//! Canonical symbol names, the path profile, and collision suffixes (PLAN.md §2.7).
//!
//! Everything here is pure: no I/O, no host lookups except
//! [`Platform::host`], and every platform-dependent rule takes the target
//! platform explicitly so all three can be tested on any host.

pub mod canonical;
pub mod scip;
