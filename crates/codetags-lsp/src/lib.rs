//! The codetags LSP shim (M3). Stage 0 (PLAN.md D18) builds only the
//! session recorder ([`record`]) and its analyzer ([`analyze`]); the lspmux
//! shim (P3.4) grows from the same framing ([`framing`]).
//!
//! This crate must never depend on DuckDB: editors start the binary outside
//! cargo, where a DuckDB-linked binary does not find its library.

pub mod analyze;
pub mod framing;
pub mod invocation;
pub mod logpath;
pub mod message;
pub mod record;
