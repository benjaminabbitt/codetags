//! What a batch reports: changes to project paths, and why a rescan happened.

use std::fmt;
use std::path::PathBuf;

/// The net effect of the events on one path within a batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChangeKind {
    /// The path did not exist at the last batch and exists now.
    Created,
    /// The path existed at the last batch and still does, with new content.
    Changed,
    /// The path existed at the last batch and is gone now.
    Deleted,
}

impl ChangeKind {
    /// The lowercase name used in logs and features: `created`, `changed` or
    /// `deleted`.
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeKind::Created => "created",
            ChangeKind::Changed => "changed",
            ChangeKind::Deleted => "deleted",
        }
    }
}

impl fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ChangeKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "created" => Ok(ChangeKind::Created),
            "changed" => Ok(ChangeKind::Changed),
            "deleted" => Ok(ChangeKind::Deleted),
            other => Err(format!(
                "unknown change kind {other:?}: use created, changed or deleted"
            )),
        }
    }
}

/// Merges two events on the same path into their net effect (brief §4.3).
///
/// `None` means the events cancel out and the path is dropped from the batch.
///
/// - created, then deleted: dropped;
/// - created, then anything else: created;
/// - deleted, then created: changed;
/// - otherwise: the later event.
pub fn merge(prev: ChangeKind, next: ChangeKind) -> Option<ChangeKind> {
    use ChangeKind::{Changed, Created, Deleted};
    match (prev, next) {
        (Created, Deleted) => None,
        (Created, _) => Some(Created),
        (Deleted, Created) => Some(Changed),
        (_, next) => Some(next),
    }
}

/// One path in a batch.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Change {
    /// The path relative to the watched root, with the OS's separators.
    pub path: PathBuf,
    /// What happened to it.
    pub kind: ChangeKind,
}

/// Why a batch's changes came from a rescan rather than from events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RescanCause {
    /// The OS dropped events: inotify's queue overflowed, FSEvents asked for
    /// a rescan, or ReadDirectoryChangesW's buffer overflowed. notify reports
    /// all three as `Flag::Rescan`.
    Overflow,
    /// notify reported an error other than a watch limit, so events may have
    /// been lost. Holds notify's message.
    WatchError(String),
    /// [`crate::Watcher::rescan`] asked for one.
    Requested,
    /// The privileged helper lost events: its fanotify queue overflowed, or
    /// it could not keep up with this client.
    HelperOverflow,
    /// The privileged helper's connection ended, so the watcher switched to
    /// notify. Holds the reason.
    HelperLost(String),
}

impl fmt::Display for RescanCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RescanCause::Overflow => write!(
                f,
                "{} overflowed and events were lost; the project was rescanned and the \
                 difference from the last batch reported",
                overflow_source()
            ),
            RescanCause::WatchError(message) => write!(
                f,
                "the file watcher reported an error ({message}); the project was rescanned \
                 and the difference from the last batch reported"
            ),
            RescanCause::Requested => f.write_str("a rescan was requested"),
            RescanCause::HelperOverflow => f.write_str(
                "the privileged helper lost events (its fanotify queue overflowed, or this \
                 client fell behind); the project was rescanned and the difference from the \
                 last batch reported",
            ),
            RescanCause::HelperLost(reason) => write!(
                f,
                "{reason}; the watcher switched to notify, rescanned the project and reported \
                 the difference from the last batch"
            ),
        }
    }
}

/// What overflows on this OS, for [`RescanCause::Overflow`]'s message.
fn overflow_source() -> &'static str {
    if cfg!(target_os = "linux") {
        "the inotify event queue (fs.inotify.max_queued_events)"
    } else if cfg!(target_os = "macos") {
        "the FSEvents stream (MustScanSubDirs)"
    } else if cfg!(windows) {
        "the ReadDirectoryChangesW buffer"
    } else {
        "the file watcher's event queue"
    }
}

/// Changes flushed together by the coalescer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Batch {
    /// The changes, sorted by path. Empty only in a rescan batch whose
    /// rescan found nothing new.
    pub changes: Vec<Change>,
    /// Set when a rescan replaced the batch's events, with the reason.
    pub rescan: Option<RescanCause>,
}

#[cfg(test)]
mod tests {
    use super::ChangeKind::{Changed, Created, Deleted};
    use super::*;

    #[test]
    fn create_then_delete_is_dropped() {
        assert_eq!(merge(Created, Deleted), None);
    }

    #[test]
    fn create_then_anything_else_stays_a_create() {
        assert_eq!(merge(Created, Changed), Some(Created));
        assert_eq!(merge(Created, Created), Some(Created));
    }

    #[test]
    fn delete_then_create_is_a_change() {
        assert_eq!(merge(Deleted, Created), Some(Changed));
    }

    #[test]
    fn otherwise_the_latest_wins() {
        assert_eq!(merge(Deleted, Changed), Some(Changed));
        assert_eq!(merge(Deleted, Deleted), Some(Deleted));
        assert_eq!(merge(Changed, Deleted), Some(Deleted));
        assert_eq!(merge(Changed, Created), Some(Created));
        assert_eq!(merge(Changed, Changed), Some(Changed));
    }

    #[test]
    fn kinds_round_trip_through_their_names() {
        for kind in [Created, Changed, Deleted] {
            assert_eq!(kind.as_str().parse::<ChangeKind>(), Ok(kind));
        }
        assert!("moved".parse::<ChangeKind>().is_err());
    }
}
