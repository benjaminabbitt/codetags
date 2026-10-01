//! The codetags LSP shim (M3). Stage 0 (PLAN.md D18) built the session
//! recorder ([`record`]) and its analyzer ([`analyze`]). Stage 1 (D19, D20)
//! adds lspmux's configuration: what `codetags lsp setup` writes ([`setup`],
//! [`lspmux`]) and what `codetags doctor` checks ([`doctor`]). The lspmux
//! shim (P3.4) grows from the same framing ([`framing`]).
//!
//! This crate must never depend on DuckDB: editors start the binary outside
//! cargo, where a DuckDB-linked binary does not find its library.

pub mod analyze;
pub mod doctor;
pub mod framing;
pub mod invocation;
pub mod logpath;
pub mod lspmux;
pub mod message;
pub mod record;
pub mod setup;
pub mod tools;
