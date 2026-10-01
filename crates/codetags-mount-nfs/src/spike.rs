//! P0b S2: a read-only, one-file NFSv3 server. It proves an unprivileged
//! mount, list, read, and unmount on macOS (and in CI) before the view core
//! exists, and measures the NFS client's caching. P5.2 replaces it with the
//! `ViewFs` adapter.
//!
//! The server binds 127.0.0.1 on an ephemeral port and answers only mount
//! requests for its random export path (PLAN.md §2.8). It runs on its own
//! thread with a single-threaded tokio runtime, so callers stay synchronous.

use std::fmt::Write as _;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, PoisonError, RwLock, mpsc};
use std::thread::JoinHandle;
use std::time::SystemTime;

use nfs3_server::nfs3_types::nfs3::{
    fattr3, filename3, ftype3, nfspath3, nfsstat3, nfstime3, specdata3,
};
use nfs3_server::tcp::{NFSTcp, NFSTcpListener};
use nfs3_server::vfs::{
    DirEntryPlus, FileHandleU64, NextResult, NfsReadFileSystem, ReadDirPlusIterator,
};
use tokio::sync::oneshot;

/// The one file the spike serves at first.
pub const HELLO_NAME: &str = "hello.txt";
/// Its contents.
pub const HELLO_CONTENT: &str = "hello from codetags\n";

/// File ids. 0 is reserved by NFS; the root is 1 and files count up from 2.
const ROOT_ID: u64 = 1;
const FIRST_FILE_ID: u64 = 2;

/// Bytes of randomness in the export path: 128 bits, as 32 hex digits.
const TOKEN_BYTES: usize = 16;

/// A running loopback server. Dropping it stops the server too, but
/// [`Server::stop`] reports errors.
///
/// Stop the server only after the mount is gone: the NFS client blocks on a
/// server that has stopped answering.
#[derive(Debug)]
pub struct Server {
    address: SocketAddr,
    export_path: String,
    tree: Arc<RwLock<Tree>>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl Server {
    /// Starts the one-file server on 127.0.0.1, on a port the OS picks. Its
    /// files are owned by `uid` and `gid`, so they match the mount point's
    /// owner.
    pub fn start(uid: u32, gid: u32) -> io::Result<Self> {
        let export_path = format!("/{}", random_token()?);
        let tree = Arc::new(RwLock::new(Tree::new()));
        let filesystem = HelloFs {
            tree: Arc::clone(&tree),
            uid,
            gid,
        };
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = oneshot::channel();
        let token = export_path.clone();
        let thread = std::thread::Builder::new()
            .name("codetags-nfs".to_string())
            .spawn(move || serve(filesystem, &token, &ready_tx, stop_rx))?;
        let address = match ready_rx.recv() {
            Ok(Ok(address)) => address,
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                return Err(match thread.join() {
                    Ok(Err(error)) => error,
                    _ => io::Error::other("the NFS server thread exited before it was ready"),
                });
            }
        };
        Ok(Self {
            address,
            export_path,
            tree,
            stop: Some(stop_tx),
            thread: Some(thread),
        })
    }

    /// The address the server listens on: always 127.0.0.1.
    pub fn ip(&self) -> IpAddr {
        self.address.ip()
    }

    /// The TCP port, which serves both the MOUNT and the NFS protocols.
    pub fn port(&self) -> u16 {
        self.address.port()
    }

    /// The only path the server lets a client mount: `/` and 32 random hex
    /// digits.
    pub fn export_path(&self) -> &str {
        &self.export_path
    }

    /// Adds `name` to the root holding `content`, or replaces its content.
    ///
    /// The staleness hook (S2's measurement): it changes the tree the way a
    /// new generation would. With `bump_dir_mtime`, the root's mtime becomes
    /// the current time, which is how an NFS client learns that a directory
    /// changed; without it, the root's attributes stay as they were.
    pub fn add_file(&self, name: &str, content: &[u8], bump_dir_mtime: bool) {
        let mut tree = self.tree.write().unwrap_or_else(PoisonError::into_inner);
        tree.add(name, content);
        if bump_dir_mtime {
            tree.bump_root_mtime();
        }
    }

    /// Stops the server and waits for its thread.
    pub fn stop(mut self) -> io::Result<()> {
        self.shut_down()
    }

    fn shut_down(&mut self) -> io::Result<()> {
        if let Some(stop) = self.stop.take() {
            // An error means the server already exited; join reports why.
            let _ = stop.send(());
        }
        match self.thread.take().map(JoinHandle::join) {
            None => Ok(()),
            Some(Ok(result)) => result,
            Some(Err(_)) => Err(io::Error::other("the NFS server thread panicked")),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.shut_down();
    }
}

/// The server thread: binds, reports the address, then serves until told to
/// stop. Dropping the runtime at the end cancels every connection.
fn serve(
    filesystem: HelloFs,
    export_path: &str,
    ready: &mpsc::Sender<io::Result<SocketAddr>>,
    stop: oneshot::Receiver<()>,
) -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let mut listener = match NFSTcpListener::bind_ro("127.0.0.1:0", filesystem).await {
            Ok(listener) => listener,
            Err(error) => {
                let _ = ready.send(Err(error));
                return Ok(());
            }
        };
        listener.with_export_name(export_path);
        let address = SocketAddr::new(listener.get_listen_ip(), listener.get_listen_port());
        if ready.send(Ok(address)).is_err() {
            return Ok(());
        }
        tokio::select! {
            result = listener.handle_forever() => result,
            _ = stop => Ok(()),
        }
    })
}

/// 128 random bits from the OS, as lowercase hex.
fn random_token() -> io::Result<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    let mut token = String::with_capacity(TOKEN_BYTES * 2);
    for byte in bytes {
        let _ = write!(token, "{byte:02x}");
    }
    Ok(token)
}

fn now() -> nfstime3 {
    nfstime3::try_from(SystemTime::now()).unwrap_or_default()
}

/// The served tree: one root directory holding files.
#[derive(Debug)]
struct Tree {
    root_mtime: nfstime3,
    /// In id order, which is also readdir order.
    files: Vec<File>,
}

#[derive(Debug)]
struct File {
    id: u64,
    name: String,
    content: Vec<u8>,
    mtime: nfstime3,
}

impl Tree {
    fn new() -> Self {
        let created = now();
        Self {
            root_mtime: created,
            files: vec![File {
                id: FIRST_FILE_ID,
                name: HELLO_NAME.to_string(),
                content: HELLO_CONTENT.as_bytes().to_vec(),
                mtime: created,
            }],
        }
    }

    fn add(&mut self, name: &str, content: &[u8]) {
        let mtime = now();
        if let Some(file) = self.files.iter_mut().find(|file| file.name == name) {
            file.content = content.to_vec();
            file.mtime = mtime;
            return;
        }
        let id = self.files.last().map_or(FIRST_FILE_ID, |file| file.id + 1);
        self.files.push(File {
            id,
            name: name.to_string(),
            content: content.to_vec(),
            mtime,
        });
    }

    /// Moves the root's mtime to now, and strictly forward even if the clock
    /// has not ticked, so a client always sees a change.
    fn bump_root_mtime(&mut self) {
        let old = self.root_mtime;
        let mut new = now();
        if (new.seconds, new.nseconds) <= (old.seconds, old.nseconds) {
            new = if old.nseconds < 999_999_999 {
                nfstime3 {
                    seconds: old.seconds,
                    nseconds: old.nseconds + 1,
                }
            } else {
                nfstime3 {
                    seconds: old.seconds.saturating_add(1),
                    nseconds: 0,
                }
            };
        }
        self.root_mtime = new;
    }

    fn file(&self, id: u64) -> Option<&File> {
        self.files.iter().find(|file| file.id == id)
    }
}

/// The read-only filesystem the server exports. `nfs3_server`'s
/// `bind_ro` refuses every write with `NFS3ERR_ROFS`.
struct HelloFs {
    tree: Arc<RwLock<Tree>>,
    uid: u32,
    gid: u32,
}

impl HelloFs {
    fn attr(&self, tree: &Tree, id: u64) -> Option<fattr3> {
        let (type_, mode, nlink, size, mtime) = if id == ROOT_ID {
            (ftype3::NF3DIR, 0o555, 2, 0, tree.root_mtime)
        } else {
            let file = tree.file(id)?;
            (
                ftype3::NF3REG,
                0o444,
                1,
                file.content.len() as u64,
                file.mtime,
            )
        };
        Some(fattr3 {
            type_,
            mode,
            nlink,
            uid: self.uid,
            gid: self.gid,
            size,
            used: size,
            rdev: specdata3::default(),
            fsid: 0,
            fileid: id,
            atime: mtime,
            mtime,
            ctime: mtime,
        })
    }

    fn read_tree(&self) -> std::sync::RwLockReadGuard<'_, Tree> {
        self.tree.read().unwrap_or_else(PoisonError::into_inner)
    }
}

impl NfsReadFileSystem for HelloFs {
    type Handle = FileHandleU64;

    fn root_dir(&self) -> FileHandleU64 {
        FileHandleU64::new(ROOT_ID)
    }

    async fn lookup(
        &self,
        dirid: &FileHandleU64,
        filename: &filename3<'_>,
    ) -> Result<FileHandleU64, nfsstat3> {
        if dirid.as_u64() != ROOT_ID {
            return Err(nfsstat3::NFS3ERR_NOTDIR);
        }
        let name = filename.as_ref();
        if name == b"." || name == b".." {
            return Ok(self.root_dir());
        }
        self.read_tree()
            .files
            .iter()
            .find(|file| file.name.as_bytes() == name)
            .map(|file| FileHandleU64::new(file.id))
            .ok_or(nfsstat3::NFS3ERR_NOENT)
    }

    async fn getattr(&self, id: &FileHandleU64) -> Result<fattr3, nfsstat3> {
        self.attr(&self.read_tree(), id.as_u64())
            .ok_or(nfsstat3::NFS3ERR_STALE)
    }

    async fn read(
        &self,
        id: &FileHandleU64,
        offset: u64,
        count: u32,
    ) -> Result<(Vec<u8>, bool), nfsstat3> {
        if id.as_u64() == ROOT_ID {
            return Err(nfsstat3::NFS3ERR_ISDIR);
        }
        let tree = self.read_tree();
        let file = tree.file(id.as_u64()).ok_or(nfsstat3::NFS3ERR_STALE)?;
        let bytes = &file.content;
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(bytes.len());
        let end = start
            .saturating_add(usize::try_from(count).unwrap_or(usize::MAX))
            .min(bytes.len());
        Ok((bytes[start..end].to_vec(), end == bytes.len()))
    }

    async fn readdirplus(
        &self,
        dirid: &FileHandleU64,
        cookie: u64,
    ) -> Result<impl ReadDirPlusIterator<FileHandleU64>, nfsstat3> {
        if dirid.as_u64() != ROOT_ID {
            return Err(nfsstat3::NFS3ERR_NOTDIR);
        }
        let tree = self.read_tree();
        // The cookie is the id of the last entry the client has seen.
        let entries = tree
            .files
            .iter()
            .filter(|file| file.id > cookie)
            .map(|file| DirEntryPlus {
                fileid: file.id,
                name: file.name.as_bytes().to_vec().into(),
                cookie: file.id,
                name_attributes: self.attr(&tree, file.id),
                name_handle: Some(FileHandleU64::new(file.id)),
            })
            .collect::<Vec<_>>();
        Ok(Entries {
            entries: entries.into_iter(),
        })
    }

    async fn readlink(&self, _id: &FileHandleU64) -> Result<nfspath3<'_>, nfsstat3> {
        Err(nfsstat3::NFS3ERR_INVAL)
    }
}

/// A directory listing, handed out front to back.
struct Entries {
    entries: std::vec::IntoIter<DirEntryPlus<FileHandleU64>>,
}

impl ReadDirPlusIterator<FileHandleU64> for Entries {
    async fn next(&mut self) -> NextResult<DirEntryPlus<FileHandleU64>> {
        self.entries.next().map_or(NextResult::Eof, NextResult::Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_root_mtime_always_moves_forward() {
        let mut tree = Tree::new();
        tree.root_mtime = nfstime3 {
            seconds: u32::MAX - 1,
            nseconds: 999_999_999,
        };
        tree.bump_root_mtime();
        assert_eq!(
            tree.root_mtime,
            nfstime3 {
                seconds: u32::MAX,
                nseconds: 0
            }
        );
    }

    #[test]
    fn adding_an_existing_name_replaces_its_content() {
        let mut tree = Tree::new();
        tree.add(HELLO_NAME, b"changed");
        assert_eq!(tree.files.len(), 1);
        assert_eq!(tree.files[0].content, b"changed");
    }

    #[test]
    fn random_tokens_are_32_hex_digits() {
        let token = random_token().unwrap_or_default();
        assert_eq!(token.len(), 32);
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}
