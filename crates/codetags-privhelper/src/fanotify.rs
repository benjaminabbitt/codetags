//! The fanotify group: initialised with file-handle reporting, marked on
//! whole filesystems, read raw (brief §4.3, V9, V86).

use std::fs::File;
use std::os::fd::AsFd;

use codetags_watch::privhelper::proto::EventKind;
use nix::errno::Errno;
use nix::libc;
use nix::sys::fanotify::{EventFFlags, Fanotify, InitFlags, MarkFlags, MaskFlags};

/// What the helper asks to hear about: every change to a directory entry or
/// a file's contents or metadata, on directories too.
fn mask() -> MaskFlags {
    MaskFlags::FAN_CREATE
        | MaskFlags::FAN_DELETE
        | MaskFlags::FAN_MOVED_FROM
        | MaskFlags::FAN_MOVED_TO
        | MaskFlags::FAN_MODIFY
        | MaskFlags::FAN_CLOSE_WRITE
        | MaskFlags::FAN_ATTRIB
        | MaskFlags::FAN_DELETE_SELF
        | MaskFlags::FAN_MOVE_SELF
        | MaskFlags::FAN_ONDIR
}

/// A fanotify group that reports file handles.
#[derive(Debug)]
pub struct Group {
    fan: Fanotify,
    /// Whether events name the entry (`FAN_REPORT_DFID_NAME`, Linux 5.9+),
    /// or carry only the object's handle (`FAN_REPORT_FID`, Linux 5.1+).
    pub names: bool,
}

/// Why the group could not be created.
#[derive(Debug)]
pub enum InitError {
    /// `EPERM`: the helper lacks `CAP_SYS_ADMIN`.
    NotPrivileged,
    /// `EINVAL` even without names: the kernel predates `FAN_REPORT_FID`.
    KernelTooOld,
    /// Anything else.
    Other(Errno),
}

impl std::fmt::Display for InitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InitError::NotPrivileged => f.write_str(
                "fanotify_init failed with EPERM: filesystem-wide marks need CAP_SYS_ADMIN, so \
                 run codetags-privhelper as root (it is optional: codetags works without it)",
            ),
            InitError::KernelTooOld => f.write_str(
                "fanotify_init rejected FAN_REPORT_FID: the helper needs Linux 5.1 or later",
            ),
            InitError::Other(errno) => write!(f, "fanotify_init failed: {errno}"),
        }
    }
}

impl Group {
    /// Creates the group: `FAN_REPORT_FID` with `FAN_REPORT_DFID_NAME` where
    /// the kernel has it (5.9+), else `FAN_REPORT_FID` alone (5.1+). nix's
    /// `InitFlags` names neither flag (V88), so they come from libc.
    pub fn init() -> Result<Self, InitError> {
        let base = InitFlags::FAN_CLASS_NOTIF | InitFlags::FAN_CLOEXEC;
        let fid = InitFlags::from_bits_retain(libc::FAN_REPORT_FID);
        let names = InitFlags::from_bits_retain(libc::FAN_REPORT_DFID_NAME);
        let event_flags = EventFFlags::O_RDONLY;
        match Fanotify::init(base | fid | names, event_flags) {
            Ok(fan) => return Ok(Self { fan, names: true }),
            Err(Errno::EINVAL) => {}
            Err(Errno::EPERM) => return Err(InitError::NotPrivileged),
            Err(errno) => return Err(InitError::Other(errno)),
        }
        match Fanotify::init(base | fid, event_flags) {
            Ok(fan) => Ok(Self { fan, names: false }),
            Err(Errno::EINVAL) => Err(InitError::KernelTooOld),
            Err(Errno::EPERM) => Err(InitError::NotPrivileged),
            Err(errno) => Err(InitError::Other(errno)),
        }
    }

    /// Adds (or with `add` false, removes) a `FAN_MARK_FILESYSTEM` mark on
    /// the filesystem holding the open directory `dir`. Marking the open
    /// directory, not a path, means a path swapped since it was opened
    /// cannot redirect the mark.
    pub fn mark_filesystem(&self, dir: &File, add: bool) -> nix::Result<()> {
        let action = if add {
            MarkFlags::FAN_MARK_ADD
        } else {
            MarkFlags::FAN_MARK_REMOVE
        };
        let flags = action | MarkFlags::FAN_MARK_FILESYSTEM;
        self.fan
            .mark(flags, mask(), dir.as_fd(), None::<&std::path::Path>)
    }

    /// Adds an inode mark on the directory `dir`, for its entries' events.
    /// Unprivileged users may do this (Linux 5.13+), so tests can check the
    /// parser against the kernel without root.
    #[cfg(test)]
    pub fn mark_directory(&self, dir: &File) -> nix::Result<()> {
        self.fan.mark(
            MarkFlags::FAN_MARK_ADD,
            mask() | MaskFlags::FAN_EVENT_ON_CHILD,
            dir.as_fd(),
            None::<&std::path::Path>,
        )
    }

    /// Reads one batch of raw events into `buf`, blocking until there is
    /// one; returns the length read.
    pub fn read(&self, buf: &mut [u8]) -> nix::Result<usize> {
        loop {
            match nix::unistd::read(self.fan.as_fd(), buf) {
                Err(Errno::EINTR) => {}
                other => return other,
            }
        }
    }
}

/// Whether the event's mask reports lost events.
pub fn overflowed(mask: u64) -> bool {
    mask & libc::FAN_Q_OVERFLOW != 0
}

/// The protocol kind for an event's mask, or `None` for an event the helper
/// did not ask for. fanotify merges events on the same object, so a mask
/// can hold several bits; the most telling one wins. The watcher re-reads
/// the path whatever the kind, so this is advice.
pub fn kind(mask: u64) -> Option<EventKind> {
    let has = |bit: u64| mask & bit != 0;
    Some(if has(libc::FAN_CREATE) {
        EventKind::Created
    } else if has(libc::FAN_MOVED_TO) {
        EventKind::MovedTo
    } else if has(libc::FAN_DELETE) || has(libc::FAN_DELETE_SELF) {
        EventKind::Deleted
    } else if has(libc::FAN_MOVED_FROM) || has(libc::FAN_MOVE_SELF) {
        EventKind::MovedFrom
    } else if has(libc::FAN_MODIFY) || has(libc::FAN_CLOSE_WRITE) {
        EventKind::Modified
    } else if has(libc::FAN_ATTRIB) {
        EventKind::Attrib
    } else {
        return None;
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{self, INFO_DFID_NAME};

    #[test]
    fn kinds_prefer_the_most_telling_bit() {
        assert_eq!(
            kind(libc::FAN_CREATE | libc::FAN_MODIFY),
            Some(EventKind::Created)
        );
        assert_eq!(kind(libc::FAN_CLOSE_WRITE), Some(EventKind::Modified));
        assert_eq!(kind(libc::FAN_DELETE_SELF), Some(EventKind::Deleted));
        assert_eq!(kind(libc::FAN_MOVE_SELF), Some(EventKind::MovedFrom));
        assert_eq!(kind(libc::FAN_MOVED_TO), Some(EventKind::MovedTo));
        assert_eq!(kind(libc::FAN_ATTRIB), Some(EventKind::Attrib));
        assert_eq!(kind(libc::FAN_ACCESS), None);
        assert!(overflowed(libc::FAN_Q_OVERFLOW));
    }

    /// The parser against real kernel output, unprivileged: an inode mark
    /// on a scratch directory, with the flags the helper uses (V86).
    #[test]
    fn the_kernel_reports_what_the_parser_expects() {
        let dir = tempfile::tempdir().unwrap();
        let group = Group::init().unwrap();
        assert!(group.names, "this kernel lacks FAN_REPORT_DFID_NAME");
        let opened = File::open(dir.path()).unwrap();
        group.mark_directory(&opened).unwrap();
        std::fs::write(dir.path().join("x.rs"), "x").unwrap();

        let mut buf = vec![0u8; 64 * 1024];
        let len = group.read(&mut buf).unwrap();
        let events = parse::parse(&buf[..len]).unwrap();
        let created = events
            .iter()
            .find(|event| kind(event.mask) == Some(EventKind::Created))
            .expect("a create event");
        assert_eq!(created.pid, std::process::id() as i32);
        let named = created
            .fids
            .iter()
            .find(|fid| fid.info_type == INFO_DFID_NAME)
            .expect("a DFID_NAME record");
        assert_eq!(named.name.as_deref(), Some("x.rs".as_ref()));
        let fsid = nix::sys::statvfs::fstatvfs(&opened)
            .unwrap()
            .filesystem_id();
        assert_eq!(named.fsid, fsid as u64, "fsid packing differs from statvfs");
    }
}
