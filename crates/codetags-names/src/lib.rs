//! Canonical symbol names, the path profile, and collision suffixes (PLAN.md §2.7).
//!
//! Everything here is pure: no I/O, no host lookups except
//! [`platform::Platform::host`], and every platform-dependent rule takes
//! the target platform explicitly so all three can be tested on any host.

pub mod canonical;
pub mod collision;
pub mod item_id;
pub mod path_profile;
mod percent;
pub mod platform;
pub mod scip;

pub use percent::PercentError;
