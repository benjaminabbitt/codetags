//! The codetags LSP shim (M3). Stage 0 (PLAN.md D18) built the session
//! recorder ([`record`]) and its analyzer ([`analyze`]). Stages 1 and 2
//! (D14-D22) add the shim between editors and upstream lspmux ([`serve`],
//! with its rules in [`policy`] and [`root`]), the on-demand daemon
//! ([`daemon`]), wrappers ([`wrapper`]), and lspmux's configuration: what
//! `codetags lsp setup` writes ([`setup`], [`lspmux`]) and what `codetags
//! doctor` checks ([`doctor`]).
//!
//! This crate must never depend on DuckDB: editors start the binary outside
//! cargo, where a DuckDB-linked binary does not find its library.

pub mod analyze;
pub mod daemon;
pub mod doctor;
pub mod framing;
pub mod invocation;
pub mod logpath;
pub mod lspmux;
pub mod message;
pub mod policy;
pub mod ready;
pub mod record;
pub mod root;
pub mod serve;
pub mod setup;
pub mod tools;
pub mod uri;
pub mod wiring;
pub mod wrapper;
