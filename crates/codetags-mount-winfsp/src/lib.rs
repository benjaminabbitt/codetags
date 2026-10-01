//! Windows live-mount backend over WinFsp, optional at runtime (PLAN.md §2.6, D3, D9).
//!
//! WinFsp is installed by an admin once; after that an unprivileged user can
//! mount. When it is absent, codetags falls back to static views and the CLI.
//! The DLL is delay-loaded, so binaries start either way; [`load`] decides at
//! runtime. [`availability`] holds the platform-neutral parts (the reasons,
//! the attribution notice); [`spike`] is the P0b S3 one-file filesystem that
//! P5.3 replaces with the `ViewFs` adapter.
//!
//! Licence: the `winfsp` and `winfsp-sys` crates this backend uses are
//! GPL-3.0, so Windows binaries that include it are distributed under GPL-3.0
//! (D9). WinFsp itself is GPLv3 with a FLOSS exception that requires showing
//! [`availability::ATTRIBUTION`] and [`availability::REPO_URL`] (V14).

pub mod availability;
#[cfg(windows)]
mod load;
pub mod private_use;
#[cfg(windows)]
pub mod spike;

#[cfg(windows)]
pub use load::{WinFsp, load};
