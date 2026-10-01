//! The client of the optional privileged helper (PLAN.md P4.4, C2, §2.8).
//!
//! `codetags-privhelper` is an accelerator an admin may run as root. On
//! Linux it watches whole filesystems with fanotify (`FAN_MARK_FILESYSTEM`),
//! so the watcher needs no per-directory inotify watches and learns which
//! process wrote each file. Nothing requires it: the [`crate::Watcher`]
//! tries it first and falls back to notify whenever the socket is absent,
//! refuses the connection, fails the handshake, or refuses the project; and
//! it switches to notify, with a rescan, if the helper goes away later.
//! Either way it logs which mode is active ([`WatchMode`]).
//!
//! The helper serves [`proto`] on a Unix socket, [`DEFAULT_SOCKET`] unless
//! [`SOCKET_ENV`] names another. It identifies each client with
//! `SO_PEERCRED` and delivers an event only if that client's user could list
//! the directory holding the path; the check runs as that user, never as
//! root. The `codetags-privhelper` crate documents its security model.

#[cfg(unix)]
mod client;
#[cfg(unix)]
pub mod proto;

#[cfg(unix)]
pub use client::{Client, HelperError, Notice, Shutdown};

use std::fmt;
use std::path::PathBuf;

/// Where the helper listens unless [`SOCKET_ENV`] says otherwise.
///
/// `/run` is root-owned and cleared at boot, so only root can create
/// `/run/codetags/`, and a stale socket never outlives a reboot.
pub const DEFAULT_SOCKET: &str = "/run/codetags/privhelper.sock";

/// Overrides [`DEFAULT_SOCKET`] for [`HelperSocket::Default`], and is the
/// helper's own default too, so an admin who moves the socket sets it once.
pub const SOCKET_ENV: &str = "CODETAGS_PRIVHELPER_SOCKET";

/// Which helper socket the watcher tries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum HelperSocket {
    /// On Linux, [`SOCKET_ENV`] if set, else [`DEFAULT_SOCKET`]. Elsewhere
    /// none: there is no helper for macOS (C2) and the Windows one is still
    /// to come.
    #[default]
    Default,
    /// This socket, on any Unix.
    At(PathBuf),
    /// Never use the helper.
    Off,
}

impl HelperSocket {
    /// The socket to try, or why none is tried.
    pub fn resolve(&self) -> Result<PathBuf, &'static str> {
        match self {
            HelperSocket::Off => Err("the privileged helper is turned off"),
            HelperSocket::At(path) if cfg!(unix) => Ok(path.clone()),
            HelperSocket::Default if cfg!(target_os = "linux") => Ok(std::env::var_os(SOCKET_ENV)
                .filter(|value| !value.is_empty())
                .map_or_else(|| PathBuf::from(DEFAULT_SOCKET), PathBuf::from)),
            _ => Err("there is no privileged helper on this OS"),
        }
    }
}

/// Where a watcher's events come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchMode {
    /// notify: inotify, FSEvents or ReadDirectoryChangesW.
    Notify {
        /// Why the helper is not used.
        why_not_helper: String,
    },
    /// The privileged helper.
    Helper {
        /// The helper's event source, e.g. `fanotify`.
        backend: String,
        /// Its socket.
        socket: PathBuf,
    },
}

impl WatchMode {
    /// `notify`, or the helper's backend (`fanotify`).
    pub fn source(&self) -> &str {
        match self {
            WatchMode::Notify { .. } => "notify",
            WatchMode::Helper { backend, .. } => backend,
        }
    }
}

/// notify's backend on this OS, for logs.
fn notify_backend() -> &'static str {
    if cfg!(target_os = "linux") {
        "inotify"
    } else if cfg!(target_os = "macos") {
        "FSEvents"
    } else if cfg!(windows) {
        "ReadDirectoryChangesW"
    } else {
        "the OS's file watcher"
    }
}

impl fmt::Display for WatchMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WatchMode::Notify { why_not_helper } => write!(
                f,
                "notify ({}); not the privileged helper: {why_not_helper}",
                notify_backend()
            ),
            WatchMode::Helper { backend, socket } => write!(
                f,
                "the privileged helper ({backend}) at {}",
                socket.display()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_and_explicit_sockets_resolve() {
        assert!(HelperSocket::Off.resolve().is_err());
        let at = HelperSocket::At("/x/h.sock".into()).resolve();
        if cfg!(unix) {
            assert_eq!(at.unwrap(), PathBuf::from("/x/h.sock"));
        } else {
            assert!(at.is_err());
        }
    }

    #[test]
    fn modes_name_their_source() {
        let notify = WatchMode::Notify {
            why_not_helper: "no privileged helper at /run/x".into(),
        };
        assert_eq!(notify.source(), "notify");
        assert!(
            notify
                .to_string()
                .contains("no privileged helper at /run/x")
        );
        let helper = WatchMode::Helper {
            backend: "fanotify".into(),
            socket: DEFAULT_SOCKET.into(),
        };
        assert_eq!(helper.source(), "fanotify");
        assert!(helper.to_string().contains(DEFAULT_SOCKET));
    }
}
