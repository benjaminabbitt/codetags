//! P0b S3: a one-file WinFsp filesystem. It proves an unprivileged mount,
//! list, read, and unmount on Windows (and in CI) before the view core
//! exists. P5.3 replaces it with the `ViewFs` adapter.
//!
//! For S4 it also records every file name WinFsp passes to it for a lookup,
//! an open, or a create, so a test can see which names reach a filesystem.
//! In [`Mode::Probe`] it also serves files named with the private-use
//! mapping that Cygwin, MSYS2 and WSL use ([`crate::private_use`]).

use std::ffi::c_void;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{
    NTSTATUS, STATUS_ACCESS_DENIED, STATUS_END_OF_FILE, STATUS_OBJECT_NAME_NOT_FOUND,
};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_READONLY};
use winfsp::filesystem::{
    DirBuffer, DirInfo, DirMarker, FileInfo, FileSecurity, FileSystemContext, OpenFileInfo,
    VolumeInfo, WideNameInfo,
};
use winfsp::host::{FileSystemHost, FineGuard, VolumeParams};
use winfsp::{FspError, U16CStr};
use winfsp_sys::{FILE_ACCESS_RIGHTS, FILE_FLAGS_AND_ATTRIBUTES};

use crate::WinFsp;
use crate::private_use::{PROBE_NAMES, private_use_spelling, probe_content};

/// The one file the spike serves.
pub const HELLO_NAME: &str = "hello.txt";
/// Its contents.
pub const HELLO_CONTENT: &str = "hello from codetags\n";

/// How long WinFsp may cache file and directory info, in milliseconds. Views
/// will use short timeouts plus notify (PLAN.md P5.3); the spike's content
/// never changes.
const INFO_TIMEOUT_MS: u32 = 1000;

/// What the mount serves and accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A read-only volume holding [`HELLO_NAME`]: WinFsp refuses every write
    /// before it reaches us.
    ReadOnly,
    /// S4's probe: a writable volume whose `create` records the name and
    /// refuses with `STATUS_ACCESS_DENIED`, so creates reach the filesystem.
    /// It serves [`HELLO_NAME`] and, for each name in
    /// [`PROBE_NAMES`](crate::private_use::PROBE_NAMES), a file named with
    /// its private-use spelling, holding its
    /// [`probe_content`](crate::private_use::probe_content).
    Probe,
}

/// A mounted filesystem. Dropping it unmounts too, but [`Mounted::unmount`]
/// reports errors.
#[derive(Debug)]
pub struct Mounted {
    mountpoint: PathBuf,
    names: Arc<Mutex<Vec<String>>>,
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Mounted {
    /// Where the filesystem is mounted.
    pub fn mountpoint(&self) -> &Path {
        &self.mountpoint
    }

    /// Returns the names WinFsp has passed to the filesystem since the last
    /// call, in order, as the volume-relative paths it received (`\name`),
    /// and clears the record.
    pub fn take_names(&self) -> Vec<String> {
        std::mem::take(&mut *self.names.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Unmounts and waits for the filesystem thread to finish.
    pub fn unmount(mut self) -> io::Result<()> {
        self.shut_down()
    }

    fn shut_down(&mut self) -> io::Result<()> {
        if let Some(stop) = self.stop.take() {
            // The thread may already have exited; joining reports that.
            let _ = stop.send(());
        }
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .map_err(|_| io::Error::other("the WinFsp host thread panicked")),
            None => Ok(()),
        }
    }
}

impl Drop for Mounted {
    fn drop(&mut self) {
        let _ = self.shut_down();
    }
}

/// Mounts the spike filesystem on `mountpoint`, which must not exist yet:
/// WinFsp creates the directory and removes it on unmount. Its parent must
/// exist.
pub fn mount_hello(winfsp: &WinFsp, mountpoint: &Path, mode: Mode) -> io::Result<Mounted> {
    let _ = winfsp; // proof that the DLL is loaded
    let names = Arc::new(Mutex::new(Vec::new()));
    let filesystem = Hello {
        names: Arc::clone(&names),
        entries: entries(mode),
    };
    let (ready_tx, ready_rx) = mpsc::channel();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let target = mountpoint.to_path_buf();
    // FileSystemHost is not Send, so one thread creates, runs, and drops it.
    let thread = std::thread::Builder::new()
        .name("winfsp-host".to_string())
        .spawn(move || {
            let mut host = match start_host(filesystem, &target, mode) {
                Ok(host) => host,
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(()));
            // Runs until told to stop, or until the Mounted is gone.
            let _ = stop_rx.recv();
            host.stop();
            host.unmount();
        })?;
    let started = ready_rx
        .recv()
        .map_err(|_| io::Error::other("the WinFsp host thread exited before mounting"))?;
    let mut mounted = Mounted {
        mountpoint: mountpoint.to_path_buf(),
        names,
        stop: Some(stop_tx),
        thread: Some(thread),
    };
    match started {
        Ok(()) => Ok(mounted),
        Err(error) => {
            let _ = mounted.shut_down();
            Err(error)
        }
    }
}

fn entries(mode: Mode) -> Vec<Entry> {
    let mut entries = vec![Entry::new(HELLO_NAME, HELLO_CONTENT.to_string())];
    if mode == Mode::Probe {
        entries.extend(
            PROBE_NAMES
                .iter()
                .map(|name| Entry::new(&private_use_spelling(name), probe_content(name))),
        );
    }
    entries
}

fn start_host(
    filesystem: Hello,
    mountpoint: &Path,
    mode: Mode,
) -> io::Result<FileSystemHost<Hello, FineGuard>> {
    let mut params = VolumeParams::new();
    params
        .filesystem_name("codetags")
        .sector_size(512)
        .sectors_per_allocation_unit(1)
        .max_component_length(255)
        .case_sensitive_search(true)
        .case_preserved_names(true)
        .unicode_on_disk(true)
        .read_only_volume(mode == Mode::ReadOnly)
        .file_info_timeout(INFO_TIMEOUT_MS);
    let mut host: FileSystemHost<Hello, FineGuard> = FileSystemHost::new(params, filesystem)
        .map_err(|error| io::Error::other(format!("FspFileSystemCreate: {error}")))?;
    host.mount(mountpoint.as_os_str()).map_err(|error| {
        io::Error::other(format!(
            "FspFileSystemSetMountPoint({}): {error}",
            mountpoint.display()
        ))
    })?;
    host.start()
        .map_err(|error| io::Error::other(format!("FspFileSystemStartDispatcher: {error}")))?;
    Ok(host)
}

/// A file in the root directory.
struct Entry {
    /// Its name, as UTF-16 code units.
    name: Vec<u16>,
    content: String,
}

impl Entry {
    fn new(name: &str, content: String) -> Self {
        Self {
            name: name.encode_utf16().collect(),
            content,
        }
    }
}

/// The filesystem: a root directory holding the entries.
struct Hello {
    names: Arc<Mutex<Vec<String>>>,
    entries: Vec<Entry>,
}

/// What an open handle refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Node {
    Root,
    /// An index into `Hello::entries`.
    File(usize),
}

/// An open handle.
struct Handle {
    node: Node,
    listing: DirBuffer,
}

fn status(status: NTSTATUS) -> FspError {
    FspError::NTSTATUS(status.0)
}

impl Hello {
    fn record(&self, name: &U16CStr) {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_string_lossy());
    }

    fn find(&self, path: &U16CStr) -> winfsp::Result<Node> {
        let backslash = u16::from(b'\\');
        match path.as_slice() {
            [first] if *first == backslash => Ok(Node::Root),
            [first, name @ ..] if *first == backslash => self
                .entries
                .iter()
                .position(|entry| entry.name == name)
                .map(Node::File)
                .ok_or_else(|| status(STATUS_OBJECT_NAME_NOT_FOUND)),
            _ => Err(status(STATUS_OBJECT_NAME_NOT_FOUND)),
        }
    }

    fn info(&self, node: Node) -> FileInfo {
        let mut info = FileInfo::default();
        match node {
            Node::Root => info.file_attributes = FILE_ATTRIBUTE_DIRECTORY.0,
            Node::File(index) => {
                let size = self
                    .entries
                    .get(index)
                    .map_or(0, |entry| entry.content.len());
                info.file_attributes = FILE_ATTRIBUTE_READONLY.0;
                info.file_size = size as u64;
                info.allocation_size = info.file_size.div_ceil(512) * 512;
                info.index_number = index as u64 + 2;
            }
        }
        info
    }
}

impl FileSystemContext for Hello {
    type FileContext = Handle;

    fn get_security_by_name(
        &self,
        file_name: &U16CStr,
        _security_descriptor: Option<&mut [c_void]>,
        _reparse_point_resolver: impl FnOnce(&U16CStr) -> Option<FileSecurity>,
    ) -> winfsp::Result<FileSecurity> {
        self.record(file_name);
        let node = self.find(file_name)?;
        // No security descriptor: WinFsp then grants the access asked for
        // (winfsp src/dll/security.c, FspAccessCheckEx), and the volume's
        // read-only flag still refuses writes.
        Ok(FileSecurity {
            reparse: false,
            sz_security_descriptor: 0,
            attributes: self.info(node).file_attributes,
        })
    }

    fn open(
        &self,
        file_name: &U16CStr,
        _create_options: u32,
        _granted_access: FILE_ACCESS_RIGHTS,
        file_info: &mut OpenFileInfo,
    ) -> winfsp::Result<Handle> {
        self.record(file_name);
        let node = self.find(file_name)?;
        *file_info.as_mut() = self.info(node);
        Ok(Handle {
            node,
            listing: DirBuffer::new(),
        })
    }

    fn close(&self, _context: Handle) {}

    fn create(
        &self,
        file_name: &U16CStr,
        _create_options: u32,
        _granted_access: FILE_ACCESS_RIGHTS,
        _file_attributes: FILE_FLAGS_AND_ATTRIBUTES,
        _security_descriptor: Option<&[c_void]>,
        _allocation_size: u64,
        _extra_buffer: Option<&[u8]>,
        _extra_buffer_is_reparse_point: bool,
        _file_info: &mut OpenFileInfo,
    ) -> winfsp::Result<Handle> {
        self.record(file_name);
        Err(status(STATUS_ACCESS_DENIED))
    }

    fn get_file_info(&self, context: &Handle, file_info: &mut FileInfo) -> winfsp::Result<()> {
        *file_info = self.info(context.node);
        Ok(())
    }

    fn read(&self, context: &Handle, buffer: &mut [u8], offset: u64) -> winfsp::Result<u32> {
        let Node::File(index) = context.node else {
            return Err(status(STATUS_END_OF_FILE));
        };
        let bytes = self
            .entries
            .get(index)
            .map_or(&[][..], |entry| entry.content.as_bytes());
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        if start >= bytes.len() {
            return Err(status(STATUS_END_OF_FILE));
        }
        let count = buffer.len().min(bytes.len() - start);
        buffer[..count].copy_from_slice(&bytes[start..start + count]);
        Ok(count as u32)
    }

    fn read_directory(
        &self,
        context: &Handle,
        _pattern: Option<&U16CStr>,
        marker: DirMarker,
        buffer: &mut [u8],
    ) -> winfsp::Result<u32> {
        if context.node != Node::Root {
            return Err(status(STATUS_OBJECT_NAME_NOT_FOUND));
        }
        // The root directory lists no "." or "..", as on NTFS.
        if let Ok(lock) = context.listing.acquire(marker.is_none(), None) {
            for (index, entry) in self.entries.iter().enumerate() {
                let mut info: DirInfo = DirInfo::new();
                info.set_name_raw(entry.name.as_slice())?;
                *info.file_info_mut() = self.info(Node::File(index));
                lock.write(&mut info)?;
            }
        }
        Ok(context.listing.read(marker, buffer))
    }

    fn get_volume_info(&self, out_volume_info: &mut VolumeInfo) -> winfsp::Result<()> {
        out_volume_info.total_size = 1 << 20;
        out_volume_info.free_size = 0;
        out_volume_info.set_volume_label("codetags");
        Ok(())
    }
}
