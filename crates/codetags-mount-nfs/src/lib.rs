//! macOS live-mount backend: in-process NFSv3 loopback server (PLAN.md §2.6).
//!
//! macOS has no unprivileged FUSE, but its built-in NFS client mounts an
//! NFSv3 server onto a directory the user owns without root (C3, V13). The
//! server runs in the codetags process on `nfs3_server` (V42), bound to
//! 127.0.0.1 only, with an unguessable export path (§2.8).
//!
//! - [`spike`]: P0b S2's one-file server. It is plain TCP, so it builds and
//!   is tested on every OS; P5.2 replaces it with the `ViewFs` adapter.
//! - [`options`]: the `mount_nfs` options the backend mounts with.
//! - `mount` (macOS only): mounting and unmounting with `mount_nfs`/`umount`.

#[cfg(target_os = "macos")]
pub mod mount;
pub mod options;
pub mod spike;
