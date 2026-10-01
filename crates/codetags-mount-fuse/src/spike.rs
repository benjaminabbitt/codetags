//! P0b S1: a read-only, one-file FUSE filesystem. It proves an unprivileged
//! mount, list, read, and unmount on Linux (and in CI) before the view core
//! exists. P5.1 replaces it with the `ViewFs` adapter.

use std::ffi::OsStr;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use fuser::{
    BackgroundSession, Config, Errno, FileAttr, FileHandle, FileType, Filesystem, Generation,
    INodeNo, LockOwner, MountOption, OpenFlags, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry,
    Request,
};

/// The one file the spike serves.
pub const HELLO_NAME: &str = "hello.txt";
/// Its contents.
pub const HELLO_CONTENT: &str = "hello from codetags\n";

const HELLO_INO: INodeNo = INodeNo(2);
/// Kernel cache lifetime. Views will use near-zero TTLs (brief §4.5); the
/// spike's content never changes.
const TTL: Duration = Duration::from_secs(1);

/// A mounted filesystem. Dropping it unmounts too, but [`Mounted::unmount`]
/// reports errors.
#[derive(Debug)]
pub struct Mounted {
    session: BackgroundSession,
}

impl Mounted {
    /// Unmounts and waits for the filesystem thread to finish.
    pub fn unmount(self) -> io::Result<()> {
        self.session.umount_and_join()
    }
}

/// Mounts the one-file filesystem read-only on `mountpoint`, an existing
/// empty directory the caller owns.
///
/// No `auto_unmount`: `fuser` only allows it with `allow_other`, which needs
/// `user_allow_other` in `/etc/fuse.conf`, an admin change. After a crash the
/// mount is cleared with `fusermount3 -u`, unprivileged.
pub fn mount_hello(mountpoint: &Path) -> io::Result<Mounted> {
    let owner = std::fs::metadata(mountpoint)?;
    let filesystem = Hello {
        uid: owner.uid(),
        gid: owner.gid(),
    };
    let mut config = Config::default();
    config.mount_options = vec![
        MountOption::RO,
        MountOption::FSName("codetags".to_string()),
        MountOption::Subtype("codetags".to_string()),
    ];
    let session = fuser::spawn_mount(filesystem, mountpoint, &config)?;
    Ok(Mounted { session })
}

struct Hello {
    uid: u32,
    gid: u32,
}

impl Hello {
    fn attr(&self, ino: INodeNo) -> Option<FileAttr> {
        let (kind, perm, size, nlink) = if ino == INodeNo::ROOT {
            (FileType::Directory, 0o555, 0, 2)
        } else if ino == HELLO_INO {
            (FileType::RegularFile, 0o444, HELLO_CONTENT.len() as u64, 1)
        } else {
            return None;
        };
        Some(FileAttr {
            ino,
            size,
            blocks: size.div_ceil(512),
            atime: UNIX_EPOCH,
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
            crtime: UNIX_EPOCH,
            kind,
            perm,
            nlink,
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            flags: 0,
            blksize: 512,
        })
    }
}

impl Filesystem for Hello {
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        match self.attr(HELLO_INO) {
            Some(attr) if parent == INodeNo::ROOT && name == HELLO_NAME => {
                reply.entry(&TTL, &attr, Generation(0));
            }
            _ => reply.error(Errno::ENOENT),
        }
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        match self.attr(ino) {
            Some(attr) => reply.attr(&TTL, &attr),
            None => reply.error(Errno::ENOENT),
        }
    }

    fn read(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        if ino != HELLO_INO {
            reply.error(Errno::ENOENT);
            return;
        }
        let bytes = HELLO_CONTENT.as_bytes();
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(bytes.len());
        let end = start.saturating_add(size as usize).min(bytes.len());
        reply.data(&bytes[start..end]);
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        if ino != INodeNo::ROOT {
            reply.error(Errno::ENOTDIR);
            return;
        }
        let entries = [
            (INodeNo::ROOT, FileType::Directory, "."),
            (INodeNo::ROOT, FileType::Directory, ".."),
            (HELLO_INO, FileType::RegularFile, HELLO_NAME),
        ];
        let skip = usize::try_from(offset).unwrap_or(usize::MAX);
        for (index, (ino, kind, name)) in entries.into_iter().enumerate().skip(skip) {
            // The offset handed back is the index of the *next* entry.
            if reply.add(ino, index as u64 + 1, kind, name) {
                break;
            }
        }
        reply.ok();
    }
}
