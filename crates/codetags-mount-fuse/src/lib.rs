//! Linux live-mount backend over FUSE (PLAN.md §2.6).
//!
//! Mounts go through the setuid `fusermount3` helper, so an unprivileged user
//! can mount (C3). [`helper`] finds the helper and checks it; [`spike`] is the
//! P0b S1 one-file filesystem that P5.1 replaces with the `ViewFs` adapter.

#[cfg(unix)]
pub mod helper;
#[cfg(target_os = "linux")]
pub mod spike;
