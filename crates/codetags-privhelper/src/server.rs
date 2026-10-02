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
//!
//! Every resource a client can make the helper spend is capped
//! ([`crate::limits`]). The accept thread refuses a connection over a cap
//! itself, without starting a thread for it.

use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use codetags_watch::privhelper::proto::{HelperEvent, Reply, Request, VERSION, read_frame};
use nix::libc;
use nix::sys::socket::{UnixCredentials, getsockopt, sockopt::PeerCredentials};

use crate::checker::{Binding, Checker, DirId, Identity};
use crate::fanotify::{self, Group};
use crate::handle;
use crate::limits::{Admission, Gate, Limits, Subscriptions, Ticket};
use crate::parse::{self, INFO_DFID, INFO_DFID_NAME, INFO_FID, RawEvent};
use crate::socket;

/// The backend name sent in the handshake.
const BACKEND: &str = "fanotify";
/// How long a client may take over its handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a write to a client may block before the client is dropped.
/// The watcher reads its socket on a thread of its own, so only a stuck
/// client takes this long.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the accept thread may block writing a refusal.
const REFUSAL_TIMEOUT: Duration = Duration::from_millis(100);
/// How often the watchdog checks the socket.
const WATCHDOG: Duration = Duration::from_millis(250);
/// The size of one fanotify read.
const READ_BUFFER: usize = 64 * 1024;

/// Writes one line to the log (stderr; the journal under systemd).
pub fn log(message: impl std::fmt::Display) {
    eprintln!("codetags-privhelper: {message}");
}

/// Locks `mutex`, recovering it if a thread panicked while holding it.
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What the fanotify thread hands a client's event thread.
enum Delivery {
    /// An event under `root`. The client may see it only if its path still
    /// leads to what `binding` names.
    Event {
        root: PathBuf,
        event: HelperEvent,
        binding: Binding,
    },
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
    /// Queues `delivery` without blocking. A full queue drops it and marks
    /// the client as lagging, so it is sent an overflow: each client's
    /// memory stays bounded by its queue.
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
            let Some((path, binding)) = resolve(root, event) else {
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
                    binding,
                });
            }
        }
    }
}

/// The path an event names, as seen through `root`'s mount: the directory
/// and entry name if reported, else the directory, else the object itself.
///
/// With it comes what the path must still lead to when the client's
/// checker sees it (threat model F1). It is taken from the descriptor the
/// handle opened, never from the path: for an entry, the directory holding
/// it; for an object reported as itself, the object. An event on the root
/// itself is bound to the root's open descriptor.
fn resolve(root: &Root, event: &RawEvent) -> Option<(PathBuf, Binding)> {
    [INFO_DFID_NAME, INFO_DFID, INFO_FID]
        .into_iter()
        .flat_map(|info_type| event.fids.iter().filter(move |f| f.info_type == info_type))
        .filter(|fid| same_fs(fid.fsid, root.fsid))
        .find_map(|fid| {
            let fd = handle::open(root.mount.as_fd(), fid.handle_type, &fid.handle).ok()?;
            let base = handle::path_of(&fd)?;
            let (path, entry) = match &fid.name {
                Some(name) if name.as_os_str() == "." => (base, false),
                Some(name) => {
                    let mut parts = Path::new(name).components();
                    match (parts.next(), parts.next()) {
                        (Some(Component::Normal(_)), None) => (base.join(name), true),
                        _ => return None,
                    }
                }
                None => (base, false),
            };
            let binding = if path == root.path {
                Binding::Dir(DirId::of(&root.mount).ok()?)
            } else if entry {
                Binding::Dir(DirId::of(&fd).ok()?)
            } else {
                Binding::Entry(DirId::of(&fd).ok()?)
            };
            Some((path, binding))
        })
}

/// What every thread shares.
struct Shared {
    group: Group,
    registry: Mutex<Registry>,
    /// Where checkers are started from.
    exe: PathBuf,
    next_client: AtomicU64,
    /// The caps.
    limits: Limits,
    /// Places for running checkers.
    checkers: Arc<Gate>,
}

/// The helper's settings.
#[derive(Debug, Clone)]
pub struct Options {
    /// The socket to listen on.
    pub socket: PathBuf,
    /// The resource limits.
    pub limits: Limits,
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
    // The process still has one thread here, as socket::bind needs.
    let (listener, inode) = socket::bind(&options.socket)?;
    log(format!("listening on {}", options.socket.display()));
    let limits = options.limits;
    log(format!(
        "limits: {} connections ({} per uid), {} checkers, {} subscriptions and {} queued \
         events per connection",
        limits.connections,
        limits.connections_per_uid,
        limits.checkers,
        limits.subscriptions,
        limits.queue
    ));
    let admission = Admission::new(limits.connections, limits.connections_per_uid);
    let shared = Arc::new(Shared {
        group,
        registry: Mutex::new(Registry::default()),
        exe: PathBuf::from("/proc/self/exe"),
        next_client: AtomicU64::new(1),
        limits,
        checkers: Gate::new(limits.checkers),
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
        let stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                log(format!("accept: {error}"));
                continue;
            }
        };
        let credentials = match getsockopt(&stream, PeerCredentials) {
            Ok(credentials) => credentials,
            Err(errno) => {
                log(format!(
                    "SO_PEERCRED failed ({errno}); dropping the connection"
                ));
                continue;
            }
        };
        let ticket = match admission.admit(credentials.uid()) {
            Ok(ticket) => ticket,
            Err(reason) => {
                refuse(stream, credentials.pid(), &reason);
                continue;
            }
        };
        let shared = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name("client".into())
            .spawn(move || client(&shared, stream, credentials, ticket));
        if let Err(error) = spawned {
            log(format!("cannot start a client thread: {error}"));
        }
    }
    Ok(())
}

/// Turns a connection away on the accept thread, without a thread of its
/// own: one `Rejected` frame, written without waiting for the handshake.
/// The client reads it as the reply to its handshake, and falls back to
/// notify.
fn refuse(mut stream: UnixStream, pid: i32, reason: &str) {
    log(format!("refused a connection from pid {pid}: {reason}"));
    let _ = stream.set_write_timeout(Some(REFUSAL_TIMEOUT));
    let _ = Reply::Rejected {
        reason: reason.to_string(),
    }
    .write_to(&mut stream);
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

/// Serves one client connection, then drops its subscriptions. `_ticket`
/// holds the connection's place among the open ones until then.
fn client(shared: &Shared, stream: UnixStream, credentials: UnixCredentials, _ticket: Ticket) {
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
    // Held until the checker has exited: `serve_client` joins the event
    // thread, the checker's other owner, before it returns.
    let Some(_checker_place) = shared.checkers.enter() else {
        return reply(Reply::Rejected {
            reason: format!(
                "the helper is running its maximum of {} checkers; try again later",
                shared.checkers.limit()
            ),
        });
    };
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

    let (queue, deliveries) = sync_channel(shared.limits.queue);
    let lagged = Arc::new(AtomicBool::new(false));
    let sender = {
        let (writer, checker, lagged) = (
            Arc::clone(&writer),
            Arc::clone(&checker),
            Arc::clone(&lagged),
        );
        std::thread::Builder::new()
            .name("client-events".into())
            .spawn(move || {
                let may_see = |root: &Path, path: &Path, binding: Binding| {
                    lock(&checker).may_see(root, path, binding)
                };
                if let Err(error) = send_events(&deliveries, &writer, may_see, &lagged) {
                    log(format!("{error}; dropping the client"));
                    let _ = lock(&writer).shutdown(std::net::Shutdown::Both);
                }
            })?
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
    let mut subscriptions = Subscriptions::new(shared.limits.subscriptions);
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
        let decision = decision.and_then(|canonical| {
            subscriptions.check(&canonical)?;
            Ok(canonical)
        });
        let reply = match decision {
            Err(reason) => Reply::Refused { root, reason },
            Ok(canonical) => {
                match lock(&shared.registry).add(
                    &shared.group,
                    canonical.clone(),
                    subscriber.clone(),
                ) {
                    Ok(()) => {
                        subscriptions.insert(canonical.clone());
                        Reply::Accepted { root: canonical }
                    }
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

/// A client's event thread: sends each queued event the client may see, as
/// `may_see` (the client's checker) decides, and an overflow once the queue
/// has dropped any. Returns when the queue's senders are gone, or with an
/// error once the checker or the connection fails.
fn send_events<W: Write>(
    deliveries: &Receiver<Delivery>,
    writer: &Mutex<W>,
    mut may_see: impl FnMut(&Path, &Path, Binding) -> io::Result<bool>,
    lagged: &AtomicBool,
) -> Result<(), String> {
    for delivery in deliveries {
        let reply = match delivery {
            Delivery::Overflow => Some(Reply::Overflow),
            Delivery::Event {
                root,
                event,
                binding,
            } => match may_see(&root, &event.path, binding) {
                Ok(true) => Some(Reply::Event(event)),
                Ok(false) => None,
                Err(error) => return Err(format!("the checker failed ({error})")),
            },
        };
        let overflow = lagged.swap(false, Ordering::Relaxed);
        let mut out = lock(writer);
        for reply in reply.into_iter().chain(overflow.then_some(Reply::Overflow)) {
            reply
                .write_to(&mut *out)
                .map_err(|error| format!("writing to the client failed ({error})"))?;
        }
    }
    Ok(())
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

    const BOUND: Binding = Binding::Dir(DirId { dev: 1, ino: 2 });

    fn event(name: &str) -> Delivery {
        Delivery::Event {
            root: PathBuf::from("/r"),
            event: HelperEvent {
                path: Path::new("/r").join(name),
                kind: codetags_watch::privhelper::proto::EventKind::Modified,
                pid: None,
            },
            binding: BOUND,
        }
    }

    /// The replies in the bytes `send_events` wrote.
    fn replies(mut bytes: &[u8]) -> Vec<Reply> {
        let mut out = Vec::new();
        while let Some(reply) = Reply::read_from(&mut bytes).unwrap() {
            out.push(reply);
        }
        out
    }

    /// A client that falls behind costs at most its queue: what does not
    /// fit is dropped, and the client is sent an overflow, so it rescans.
    #[test]
    fn a_full_queue_drops_events_and_sends_an_overflow() {
        let (queue, deliveries) = sync_channel(2);
        let lagged = Arc::new(AtomicBool::new(false));
        let subscriber = Subscriber {
            client: 1,
            queue,
            lagged: Arc::clone(&lagged),
        };
        for name in ["a", "b", "c", "d", "e"] {
            subscriber.deliver(event(name));
        }
        assert!(lagged.load(Ordering::Relaxed));
        drop(subscriber);
        let writer = Mutex::new(Vec::new());
        let mut asked = Vec::new();
        let may_see = |_: &Path, path: &Path, binding| {
            asked.push((path.to_path_buf(), binding));
            Ok(true)
        };
        send_events(&deliveries, &writer, may_see, &lagged).unwrap();
        let sent = replies(&lock(&writer));
        let events = sent.iter().filter(|r| matches!(r, Reply::Event(_))).count();
        let overflows = sent.iter().filter(|r| matches!(r, Reply::Overflow)).count();
        assert_eq!((events, overflows), (2, 1), "{sent:?}");
        // The checker was asked about each, with the binding it came with.
        assert_eq!(
            asked,
            [
                (PathBuf::from("/r/a"), BOUND),
                (PathBuf::from("/r/b"), BOUND)
            ]
        );
    }

    #[test]
    fn events_the_checker_refuses_are_not_sent() {
        let (queue, deliveries) = sync_channel(4);
        queue.send(event("seen")).unwrap();
        queue.send(event("hidden")).unwrap();
        drop(queue);
        let writer = Mutex::new(Vec::new());
        let lagged = AtomicBool::new(false);
        let may_see = |_: &Path, path: &Path, _| Ok(path.ends_with("seen"));
        send_events(&deliveries, &writer, may_see, &lagged).unwrap();
        assert_eq!(replies(&lock(&writer)).len(), 1);

        let (queue, deliveries) = sync_channel(1);
        queue.send(event("x")).unwrap();
        let failing = |_: &Path, _: &Path, _| Err(io::Error::other("gone"));
        let error = send_events(&deliveries, &writer, failing, &lagged).unwrap_err();
        assert!(error.contains("checker failed"), "{error}");
    }
}
