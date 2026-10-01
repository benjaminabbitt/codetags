//! The helper's server: the socket, its clients, and the fanotify thread.
//!
//! Threads:
//!
//! - **fanotify:** reads events, resolves each to a path once per subscribed
//!   root on its filesystem, and queues it, without blocking, for every
//!   client subscribed to a root holding it. A full queue marks the client
//!   as lagging, and it is sent an overflow.
//! - **per client, requests:** the handshake, then subscriptions; each root
//!   is checked by the client's checker before it is accepted.
//! - **per client, events:** asks the checker whether the client may see
//!   each queued event, and sends it if so.
//! - **watchdog:** exits when the socket is removed or replaced, since no
//!   client can reach the helper then.
//!
//! Lock order: a client's writer, then the registry. The fanotify thread
//! takes only the registry; the checker lock is never held with another.

use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind};
use std::os::fd::AsFd;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use codetags_watch::privhelper::proto::{HelperEvent, Reply, Request, VERSION, read_frame};
use nix::libc;
use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};

use crate::checker::{Checker, Identity};
use crate::fanotify::{self, Group};
use crate::handle;
use crate::parse::{self, INFO_DFID, INFO_DFID_NAME, INFO_FID, RawEvent};

/// The backend name sent in the handshake.
const BACKEND: &str = "fanotify";
/// How long a client may take over its handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a write to a client may block before the client is dropped.
/// The watcher reads its socket on a thread of its own, so only a stuck
/// client takes this long.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// Events queued per client before it counts as lagging.
const QUEUE: usize = 16 * 1024;
/// How often the watchdog checks the socket.
const WATCHDOG: Duration = Duration::from_millis(250);
/// The size of one fanotify read.
const READ_BUFFER: usize = 64 * 1024;

/// Writes one line to the log (stderr; the journal under systemd).
pub fn log(message: impl std::fmt::Display) {
    eprintln!("codetags-privhelper: {message}");
}

/// Locks `mutex`, recovering it if a thread panicked while holding it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What the fanotify thread hands a client's event thread.
enum Delivery {
    Event { root: PathBuf, event: HelperEvent },
    Overflow,
}

/// One client's subscription to a root.
#[derive(Clone)]
struct Subscriber {
    client: u64,
    queue: SyncSender<Delivery>,
    lagged: Arc<AtomicBool>,
}

impl Subscriber {
    fn deliver(&self, delivery: Delivery) {
        if let Err(TrySendError::Full(_)) = self.queue.try_send(delivery) {
            self.lagged.store(true, Ordering::Relaxed);
        }
    }
}

/// A subscribed root, shared by every client watching it.
struct Root {
    /// Canonicalized by a subscriber's checker.
    path: PathBuf,
    /// The root, opened: marks the filesystem and decodes its handles.
    mount: File,
    /// Its filesystem's ID, as `statvfs` reports it.
    fsid: u64,
    subscribers: Vec<Subscriber>,
}

/// Every subscribed root.
#[derive(Default)]
struct Registry {
    roots: Vec<Root>,
}

/// Whether a record's fsid (`val[0] | val[1] << 32`) names the filesystem
/// whose `statvfs` fsid is `statvfs`. 64-bit glibc packs `f_fsid` the same
/// way (V86); musl keeps only `val[0]`.
fn same_fs(record: u64, statvfs: u64) -> bool {
    record == statvfs || (statvfs >> 32 == 0 && record & 0xffff_ffff == statvfs)
}

impl Registry {
    /// Subscribes `subscriber` to `root`, marking its filesystem if no root
    /// on it is marked yet.
    fn add(&mut self, group: &Group, root: PathBuf, subscriber: Subscriber) -> Result<(), String> {
        if let Some(existing) = self.roots.iter_mut().find(|r| r.path == root) {
            if !existing
                .subscribers
                .iter()
                .any(|s| s.client == subscriber.client)
            {
                existing.subscribers.push(subscriber);
            }
            return Ok(());
        }
        let mount = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&root)
            .map_err(|error| format!("{}: {error}", root.display()))?;
        let fsid = nix::sys::statvfs::fstatvfs(&mount)
            .map_err(|errno| format!("statvfs {}: {errno}", root.display()))?
            .filesystem_id() as u64;
        if !self.roots.iter().any(|r| r.fsid == fsid) {
            group.mark_filesystem(&mount, true).map_err(|errno| {
                format!(
                    "fanotify cannot mark the filesystem holding {}: {}",
                    root.display(),
                    errno.desc()
                )
            })?;
            log(format!("marked the filesystem holding {}", root.display()));
        }
        self.roots.push(Root {
            path: root,
            mount,
            fsid,
            subscribers: vec![subscriber],
        });
        Ok(())
    }

    /// Drops every subscription of `client`, and the marks no root needs.
    fn remove_client(&mut self, group: &Group, client: u64) {
        for root in &mut self.roots {
            root.subscribers.retain(|s| s.client != client);
        }
        let (gone, kept): (Vec<Root>, Vec<Root>) = std::mem::take(&mut self.roots)
            .into_iter()
            .partition(|root| root.subscribers.is_empty());
        for root in gone {
            if !kept.iter().any(|r| r.fsid == root.fsid) {
                let _ = group.mark_filesystem(&root.mount, false);
                log(format!(
                    "unmarked the filesystem holding {}",
                    root.path.display()
                ));
            }
        }
        self.roots = kept;
    }

    /// Queues `event` for every client subscribed to a root that holds it.
    fn dispatch(&self, event: &RawEvent) {
        if fanotify::overflowed(event.mask) {
            for subscriber in self.roots.iter().flat_map(|r| &r.subscribers) {
                subscriber.deliver(Delivery::Overflow);
            }
            return;
        }
        let Some(kind) = fanotify::kind(event.mask) else {
            return;
        };
        let pid = u32::try_from(event.pid).ok().filter(|&pid| pid != 0);
        for root in &self.roots {
            let Some(path) = resolve(root, event) else {
                continue;
            };
            if !path.starts_with(&root.path) {
                continue;
            }
            for subscriber in &root.subscribers {
                subscriber.deliver(Delivery::Event {
                    root: root.path.clone(),
                    event: HelperEvent {
                        path: path.clone(),
                        kind,
                        pid,
                    },
                });
            }
        }
    }
}

/// The path an event names, as seen through `root`'s mount: the directory
/// and entry name if reported, else the directory, else the object itself.
fn resolve(root: &Root, event: &RawEvent) -> Option<PathBuf> {
    [INFO_DFID_NAME, INFO_DFID, INFO_FID]
        .into_iter()
        .flat_map(|info_type| event.fids.iter().filter(move |f| f.info_type == info_type))
        .filter(|fid| same_fs(fid.fsid, root.fsid))
        .find_map(|fid| {
            let fd = handle::open(root.mount.as_fd(), fid.handle_type, &fid.handle).ok()?;
            let base = handle::path_of(&fd)?;
            match &fid.name {
                Some(name) if name.as_os_str() == "." => Some(base),
                Some(name) => {
                    let mut parts = Path::new(name).components();
                    match (parts.next(), parts.next()) {
                        (Some(Component::Normal(_)), None) => Some(base.join(name)),
                        _ => None,
                    }
                }
                None => Some(base),
            }
        })
}

/// What every thread shares.
struct Shared {
    group: Group,
    registry: Mutex<Registry>,
    /// Where checkers are started from.
    exe: PathBuf,
    next_client: AtomicU64,
}

/// The helper's settings.
#[derive(Debug, Clone)]
pub struct Options {
    /// The socket to listen on.
    pub socket: PathBuf,
}

/// Runs the helper until its socket goes away or the process is killed.
pub fn serve(options: &Options) -> Result<(), String> {
    let group = Group::init().map_err(|error| error.to_string())?;
    if !nix::unistd::Uid::effective().is_root() {
        // Unprivileged fanotify (Linux 5.13+) gets this far, but cannot mark
        // a filesystem, so every subscription will be refused, and clients
        // fall back to notify.
        log("not running as root: every subscription will be refused");
    }
    log(if group.names {
        "fanotify group with FAN_REPORT_FID and FAN_REPORT_DFID_NAME"
    } else {
        "fanotify group with FAN_REPORT_FID only (Linux before 5.9): events name directories"
    });
    let (listener, inode) = bind(&options.socket)?;
    log(format!("listening on {}", options.socket.display()));
    let shared = Arc::new(Shared {
        group,
        registry: Mutex::new(Registry::default()),
        exe: PathBuf::from("/proc/self/exe"),
        next_client: AtomicU64::new(1),
    });

    let socket = options.socket.clone();
    std::thread::Builder::new()
        .name("watchdog".into())
        .spawn(move || watchdog(&socket, inode))
        .map_err(|error| error.to_string())?;
    let reader = Arc::clone(&shared);
    std::thread::Builder::new()
        .name("fanotify".into())
        .spawn(move || read_events(&reader))
        .map_err(|error| error.to_string())?;

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let shared = Arc::clone(&shared);
                let spawned = std::thread::Builder::new()
                    .name("client".into())
                    .spawn(move || client(&shared, stream));
                if let Err(error) = spawned {
                    log(format!("cannot start a client thread: {error}"));
                }
            }
            Err(error) => log(format!("accept: {error}")),
        }
    }
    Ok(())
}

/// Binds the socket, replacing only a stale socket, never another file,
/// and opens it to every user: who may see what is decided per client.
fn bind(path: &Path) -> Result<(UnixListener, u64), String> {
    let shown = path.display();
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o755)
            .create(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => {
            if UnixStream::connect(path).is_ok() {
                return Err(format!("another helper is listening on {shown}"));
            }
            fs::remove_file(path).map_err(|error| format!("remove stale {shown}: {error}"))?;
        }
        Ok(_) => {
            return Err(format!(
                "{shown} exists and is not a socket; not replacing it"
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(format!("{shown}: {error}")),
    }
    let listener = UnixListener::bind(path).map_err(|error| format!("bind {shown}: {error}"))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o666))
        .map_err(|error| format!("chmod {shown}: {error}"))?;
    let inode = fs::symlink_metadata(path)
        .map_err(|error| format!("{shown}: {error}"))?
        .ino();
    Ok((listener, inode))
}

/// Exits once the socket is no longer the one bound.
fn watchdog(socket: &Path, inode: u64) {
    loop {
        std::thread::sleep(WATCHDOG);
        match fs::symlink_metadata(socket) {
            Ok(meta) if meta.ino() == inode => {}
            _ => {
                log(format!(
                    "{} was removed or replaced; exiting",
                    socket.display()
                ));
                std::process::exit(0);
            }
        }
    }
}

/// The fanotify thread.
fn read_events(shared: &Shared) {
    let mut buf = vec![0u8; READ_BUFFER];
    loop {
        let len = match shared.group.read(&mut buf) {
            Ok(len) => len,
            Err(errno) => {
                log(format!("reading fanotify events failed ({errno}); exiting"));
                std::process::exit(1);
            }
        };
        match parse::parse(&buf[..len]) {
            Ok(events) => {
                let registry = lock(&shared.registry);
                for event in &events {
                    registry.dispatch(event);
                }
            }
            Err(error) => log(error),
        }
    }
}

/// Serves one client connection, then drops its subscriptions.
fn client(shared: &Shared, stream: UnixStream) {
    let credentials = match getsockopt(&stream, PeerCredentials) {
        Ok(credentials) => credentials,
        Err(errno) => {
            log(format!(
                "SO_PEERCRED failed ({errno}); dropping the connection"
            ));
            return;
        }
    };
    let (pid, uid, gid) = (credentials.pid(), credentials.uid(), credentials.gid());
    let id = shared.next_client.fetch_add(1, Ordering::Relaxed);
    log(format!("client {id}: pid {pid}, uid {uid} connected"));
    match serve_client(shared, stream, id, Identity::of(uid, gid)) {
        Ok(()) => log(format!("client {id}: disconnected")),
        Err(error) => log(format!("client {id}: {error}")),
    }
}

fn serve_client(shared: &Shared, stream: UnixStream, id: u64, who: Identity) -> io::Result<()> {
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let mut reader = stream.try_clone()?;
    let writer = Arc::new(Mutex::new(stream));
    let reply = |reply: Reply| reply.write_to(&mut *lock(&writer));

    match read_frame(&mut reader)? {
        None => return Ok(()),
        Some((tag, body)) => match Request::decode(tag, &body) {
            Ok(Request::Hello { version }) if version == VERSION => {}
            Ok(Request::Hello { version }) => {
                return reply(Reply::Rejected {
                    reason: format!(
                        "protocol {version} is not supported; this helper speaks {VERSION}"
                    ),
                });
            }
            _ => {
                return reply(Reply::Rejected {
                    reason: "expected a handshake".into(),
                });
            }
        },
    }
    let checker = match Checker::spawn(&shared.exe, &who) {
        Ok(checker) => Arc::new(Mutex::new(checker)),
        Err(error) => {
            reply(Reply::Rejected {
                reason: format!("the helper cannot check access as uid {}: {error}", who.uid),
            })?;
            return Err(error);
        }
    };
    reply(Reply::Welcome {
        version: VERSION,
        backend: BACKEND.into(),
    })?;
    reader.set_read_timeout(None)?;

    let (queue, deliveries) = sync_channel(QUEUE);
    let lagged = Arc::new(AtomicBool::new(false));
    let sender = {
        let (writer, checker, lagged) = (
            Arc::clone(&writer),
            Arc::clone(&checker),
            Arc::clone(&lagged),
        );
        std::thread::Builder::new()
            .name("client-events".into())
            .spawn(move || send_events(&deliveries, &writer, &checker, &lagged))?
    };
    let subscriber = Subscriber {
        client: id,
        queue,
        lagged,
    };
    let result = read_requests(shared, &mut reader, &writer, &checker, subscriber);
    // The registry's copies of the queue go with the subscriptions, which
    // ends the event thread.
    lock(&shared.registry).remove_client(&shared.group, id);
    let _ = lock(&writer).shutdown(std::net::Shutdown::Both);
    let _ = sender.join();
    result
}

/// Handles subscriptions until the client hangs up.
fn read_requests(
    shared: &Shared,
    reader: &mut UnixStream,
    writer: &Mutex<UnixStream>,
    checker: &Mutex<Checker>,
    subscriber: Subscriber,
) -> io::Result<()> {
    loop {
        let Some((tag, body)) = read_frame(reader)? else {
            return Ok(());
        };
        let root = match Request::decode(tag, &body) {
            Ok(Request::Subscribe { root }) => root,
            Ok(other) => {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    format!("unexpected request {other:?}"),
                ));
            }
            Err(error) => return Err(error.into()),
        };
        let decision = lock(checker).check_root(&root)?;
        // Hold the writer while registering, so no event for the root can
        // be written before its acceptance.
        let mut out = lock(writer);
        let reply = match decision {
            Err(reason) => Reply::Refused { root, reason },
            Ok(canonical) => {
                match lock(&shared.registry).add(
                    &shared.group,
                    canonical.clone(),
                    subscriber.clone(),
                ) {
                    Ok(()) => Reply::Accepted { root: canonical },
                    Err(reason) => Reply::Refused { root, reason },
                }
            }
        };
        match &reply {
            Reply::Accepted { root } => log(format!(
                "client {}: watching {}",
                subscriber.client,
                root.display()
            )),
            Reply::Refused { root, reason } => log(format!(
                "client {}: refused {}: {reason}",
                subscriber.client,
                root.display()
            )),
            _ => {}
        }
        reply.write_to(&mut *out)?;
    }
}

/// A client's event thread: sends each queued event the client may see.
fn send_events(
    deliveries: &Receiver<Delivery>,
    writer: &Mutex<UnixStream>,
    checker: &Mutex<Checker>,
    lagged: &AtomicBool,
) {
    let stop = || {
        let _ = lock(writer).shutdown(std::net::Shutdown::Both);
    };
    for delivery in deliveries {
        let reply = match delivery {
            Delivery::Overflow => Some(Reply::Overflow),
            Delivery::Event { root, event } => match lock(checker).may_see(&root, &event.path) {
                Ok(true) => Some(Reply::Event(event)),
                Ok(false) => None,
                Err(error) => {
                    log(format!("the checker failed ({error}); dropping the client"));
                    return stop();
                }
            },
        };
        let overflow = lagged.swap(false, Ordering::Relaxed);
        let mut out = lock(writer);
        for reply in reply.into_iter().chain(overflow.then_some(Reply::Overflow)) {
            if reply.write_to(&mut *out).is_err() {
                drop(out);
                return stop();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filesystem_ids_match_glibc_and_musl_packing() {
        assert!(same_fs(0x1234_5678_9abc_def0, 0x1234_5678_9abc_def0));
        assert!(same_fs(0x1234_5678_9abc_def0, 0x9abc_def0));
        assert!(!same_fs(0x1234_5678_9abc_def0, 0x1111_5678_9abc_def0));
        assert!(!same_fs(0x1234_5678_9abc_def0, 0x9abc_def1));
    }

    #[test]
    fn bind_replaces_a_stale_socket_but_never_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run/codetags/helper.sock");
        let (listener, _) = bind(&path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o666);
        assert!(bind(&path).unwrap_err().contains("another helper"));
        drop(listener);
        bind(&path).unwrap();
        let file = dir.path().join("file");
        fs::write(&file, "keep").unwrap();
        assert!(bind(&file).unwrap_err().contains("not a socket"));
        assert_eq!(fs::read_to_string(&file).unwrap(), "keep");
    }
}
