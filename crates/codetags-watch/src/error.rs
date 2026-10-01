//! Watcher errors, with the fix spelled out where the OS imposes a limit.

use std::fmt;
use std::io;
use std::path::PathBuf;

/// Linux's errno for "too many open files", which `inotify_init1` returns
/// when the user's inotify instance limit (or the process's descriptor
/// limit) is reached.
const EMFILE: i32 = 24;

/// A suggested value for raised inotify limits: the value most
/// distributions' IDE guides use.
const SUGGESTED_WATCHES: u64 = 524_288;

/// Why the watcher could not start, or stopped watching part of the project.
#[derive(Debug)]
pub enum WatchError {
    /// The project root is missing or is not a readable directory.
    Root {
        /// The root as given.
        path: PathBuf,
        /// What reading it failed with.
        source: io::Error,
    },
    /// The OS limit on watches is reached, so part of the project is not
    /// watched. On Linux this is `fs.inotify.max_user_watches`.
    WatchLimit {
        /// The directory whose watch failed, if notify said.
        path: Option<PathBuf>,
        /// The current limit, if it could be read.
        limit: Option<u64>,
    },
    /// Linux only: no inotify instance could be created, because the user's
    /// `fs.inotify.max_user_instances` or the process's descriptor limit is
    /// reached.
    InstanceLimit {
        /// The current `fs.inotify.max_user_instances`, if it could be read.
        limit: Option<u64>,
    },
    /// Any other error from notify.
    Notify(notify::Error),
    /// The watcher's thread has stopped; no more batches will come.
    Stopped,
}

impl WatchError {
    /// Maps a notify error, naming the OS limit when one was hit.
    pub fn from_notify(error: notify::Error) -> Self {
        match &error.kind {
            notify::ErrorKind::MaxFilesWatch => WatchError::WatchLimit {
                path: error.paths.first().cloned(),
                limit: read_limit("max_user_watches"),
            },
            notify::ErrorKind::Io(io)
                if cfg!(target_os = "linux") && io.raw_os_error() == Some(EMFILE) =>
            {
                WatchError::InstanceLimit {
                    limit: read_limit("max_user_instances"),
                }
            }
            _ => WatchError::Notify(error),
        }
    }
}

/// Reads `/proc/sys/fs/inotify/<name>`; `None` elsewhere or on failure.
fn read_limit(name: &str) -> Option<u64> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    std::fs::read_to_string(format!("/proc/sys/fs/inotify/{name}"))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// ` (currently N)`, or nothing when the limit is unknown.
fn currently(limit: Option<u64>) -> String {
    limit
        .map(|n| format!(" (currently {n})"))
        .unwrap_or_default()
}

impl fmt::Display for WatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WatchError::Root { path, source } => {
                write!(f, "cannot watch {}: {source}", path.display())
            }
            WatchError::WatchLimit { path, limit } => {
                let at = path
                    .as_ref()
                    .map(|p| format!(" while watching {}", p.display()))
                    .unwrap_or_default();
                if cfg!(target_os = "linux") {
                    write!(
                        f,
                        "the inotify watch limit is reached{at}: fs.inotify.max_user_watches{} \
                         allows too few watches for one per project directory. Raise it with \
                         `sudo sysctl fs.inotify.max_user_watches={SUGGESTED_WATCHES}`, and keep \
                         it across reboots with `echo fs.inotify.max_user_watches={SUGGESTED_WATCHES} \
                         | sudo tee /etc/sysctl.d/90-codetags.conf`",
                        currently(*limit)
                    )
                } else {
                    write!(f, "the operating system's file watch limit is reached{at}")
                }
            }
            WatchError::InstanceLimit { limit } => write!(
                f,
                "cannot create an inotify instance: fs.inotify.max_user_instances{} or this \
                 process's open-file limit is reached. Raise the first with \
                 `sudo sysctl fs.inotify.max_user_instances=1024`, or the second with \
                 `ulimit -n`",
                currently(*limit)
            ),
            WatchError::Notify(error) => write!(f, "file watcher error: {error}"),
            WatchError::Stopped => f.write_str("the file watcher has stopped"),
        }
    }
}

impl std::error::Error for WatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WatchError::Root { source, .. } => Some(source),
            WatchError::Notify(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_watch_limit_names_the_directory() {
        let error = WatchError::from_notify(
            notify::Error::new(notify::ErrorKind::MaxFilesWatch).add_path("/p/src".into()),
        );
        let message = error.to_string();
        assert!(matches!(error, WatchError::WatchLimit { .. }), "{error:?}");
        assert!(message.contains("/p/src"), "{message}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn on_linux_a_watch_limit_names_the_sysctl_and_how_to_raise_it() {
        let message = WatchError::WatchLimit {
            path: None,
            limit: Some(8192),
        }
        .to_string();
        assert!(
            message.contains("fs.inotify.max_user_watches (currently 8192)"),
            "{message}"
        );
        assert!(
            message.contains("sudo sysctl fs.inotify.max_user_watches=524288"),
            "{message}"
        );
        assert!(message.contains("/etc/sysctl.d/"), "{message}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn on_linux_emfile_is_the_instance_limit() {
        let error =
            WatchError::from_notify(notify::Error::io(io::Error::from_raw_os_error(EMFILE)));
        assert!(
            matches!(error, WatchError::InstanceLimit { .. }),
            "{error:?}"
        );
        assert!(error.to_string().contains("fs.inotify.max_user_instances"));
    }

    #[test]
    fn other_errors_pass_through() {
        let error = WatchError::from_notify(notify::Error::generic("boom"));
        assert!(matches!(error, WatchError::Notify(_)), "{error:?}");
        assert!(error.to_string().contains("boom"));
    }
}
