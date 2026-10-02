//! The watcher (PLAN.md P4.1): notify events in, coalesced batches out.
//!
//! notify's backends report the same edit differently: inotify sends
//! create, modify and close-write; FSEvents merges flags and arrives late;
//! ReadDirectoryChangesW sends renames as old/new name pairs. The [`Engine`]
//! therefore treats every event as a hint that a path may have changed, and
//! classifies it by looking at the path and at what it last knew (`current`).
//! The result is the same on every OS.
//!
//! Two snapshots are kept. `current` is the latest known state; `baseline`
//! is the state as of the last batch sent. A rescan diffs a fresh scan
//! against `baseline`, so its difference can replace the pending events
//! (brief §4.3).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use notify::event::{AccessKind, AccessMode, ModifyKind};
use notify::{Event, EventKind, RecursiveMode, Watcher as _};

use crate::change::{Batch, Change, ChangeKind, RescanCause};
use crate::coalesce::{CoalesceConfig, Coalescer};
use crate::error::WatchError;
use crate::exclude::{INDEX_LOCK, PathClass, classify};
#[cfg(unix)]
use crate::privhelper::{Client, Notice, Shutdown, proto::EventKind as HelperKind};
use crate::privhelper::{HelperSocket, WatchMode};
use crate::snapshot::{self, Entry, Snapshot};

/// How often the watcher re-checks `.git/index.lock` while it exists, in
/// case its removal event is missed.
const LOCK_POLL: Duration = Duration::from_millis(100);

/// How long the watcher thread sleeps when nothing is pending.
const IDLE: Duration = Duration::from_secs(3600);

/// Watcher settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchConfig {
    /// When batches are flushed.
    pub coalesce: CoalesceConfig,
    /// The privileged helper to try before notify (P4.4).
    pub helper: HelperSocket,
}

/// Adds the watches a newly found directory needs.
pub(crate) trait DirWatcher: Send {
    /// Called with each directory's absolute path before it is read.
    fn watch_dir(&mut self, dir: &Path) -> Result<(), WatchError>;
}

/// inotify: one non-recursive watch per directory, so excluded trees such
/// as `node_modules/` cost no watches (brief §4.3, "one watch per
/// directory"). notify's own recursive mode would watch them too.
struct PerDirectory(notify::RecommendedWatcher);

impl DirWatcher for PerDirectory {
    fn watch_dir(&mut self, dir: &Path) -> Result<(), WatchError> {
        match self.0.watch(dir, RecursiveMode::NonRecursive) {
            Ok(()) => Ok(()),
            Err(error) => match WatchError::from_notify(error) {
                limit @ (WatchError::WatchLimit { .. } | WatchError::InstanceLimit { .. }) => {
                    Err(limit)
                }
                // The directory vanished or cannot be read; an event or a
                // rescan accounts for it.
                _ => Ok(()),
            },
        }
    }
}

/// FSEvents and ReadDirectoryChangesW: one recursive watch on the root,
/// added at start. Excluded paths are filtered here. Per-directory watches
/// would be worse on both: notify restarts the FSEvents stream for every
/// path added, and each Windows watch holds a directory handle open.
struct Recursive(#[allow(dead_code)] notify::RecommendedWatcher);

impl DirWatcher for Recursive {
    fn watch_dir(&mut self, _dir: &Path) -> Result<(), WatchError> {
        Ok(())
    }
}

/// The privileged helper: its filesystem mark already covers every
/// directory, so nothing is added per directory.
#[cfg(unix)]
struct HelperCovers;

#[cfg(unix)]
impl DirWatcher for HelperCovers {
    fn watch_dir(&mut self, _dir: &Path) -> Result<(), WatchError> {
        Ok(())
    }
}

/// Starts notify over `root`, sending its events to `inbox`: per-directory
/// watches on Linux, one recursive watch elsewhere.
fn notify_dirs(root: &Path, inbox: Sender<Message>) -> Result<Box<dyn DirWatcher>, WatchError> {
    let mut notifier = notify::recommended_watcher(move |event| {
        let _ = inbox.send(Message::Event(event));
    })
    .map_err(WatchError::from_notify)?;
    if cfg!(any(target_os = "macos", windows)) {
        notifier
            .watch(root, RecursiveMode::Recursive)
            .map_err(WatchError::from_notify)?;
        Ok(Box::new(Recursive(notifier)))
    } else {
        Ok(Box::new(PerDirectory(notifier)))
    }
}

/// The watcher's state, driven by the watcher thread or by tests.
pub(crate) struct Engine {
    /// The canonical root; event paths are relative to it.
    root: PathBuf,
    /// Other spellings of the root that event paths may start with.
    aliases: Vec<PathBuf>,
    dirs: Box<dyn DirWatcher>,
    current: Snapshot,
    baseline: Snapshot,
    coalescer: Coalescer,
}

impl Engine {
    /// An engine over `root` (canonical), scanning it at once. Every
    /// directory is watched before it is read.
    pub fn new(
        root: PathBuf,
        aliases: Vec<PathBuf>,
        mut dirs: Box<dyn DirWatcher>,
        config: WatchConfig,
        now: Instant,
    ) -> Result<Self, WatchError> {
        let mut current = Snapshot::new();
        snapshot::scan(&root, Path::new(""), &mut current, &mut |rel| {
            dirs.watch_dir(&root.join(rel))
        })?;
        // git's internals are excluded from the scan, but its index lock is
        // watched, so .git itself needs a watch.
        let git = root.join(INDEX_LOCK[0]);
        if git.is_dir() {
            dirs.watch_dir(&git)?;
        }
        let mut engine = Self {
            root,
            aliases,
            dirs,
            baseline: current.clone(),
            current,
            coalescer: Coalescer::new(config.coalesce),
        };
        engine.check_lock(now);
        Ok(engine)
    }

    /// Handles one notify event or error at `now`.
    pub fn handle(&mut self, now: Instant, event: notify::Result<Event>) -> Result<(), WatchError> {
        let event = match event {
            Ok(event) => event,
            Err(error) => {
                return match WatchError::from_notify(error) {
                    limit @ (WatchError::WatchLimit { .. } | WatchError::InstanceLimit { .. }) => {
                        Err(limit)
                    }
                    WatchError::Notify(error)
                        if matches!(error.kind, notify::ErrorKind::PathNotFound) =>
                    {
                        Ok(())
                    }
                    other => self.rescan(now, RescanCause::WatchError(other.to_string())),
                };
            }
        };
        if event.need_rescan() {
            return self.rescan(now, RescanCause::Overflow);
        }
        let content = match event.kind {
            EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
            EventKind::Access(_) => return Ok(()),
            EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Any)
            | EventKind::Any
            | EventKind::Other => true,
            _ => false,
        };
        for path in &event.paths {
            self.examine(now, path, content)?;
        }
        Ok(())
    }

    /// Handles a hint that the absolute `path` may have changed, from an
    /// event source other than notify (the privileged helper). `content`
    /// says the source reported a write.
    #[cfg(unix)]
    pub fn hint(&mut self, now: Instant, path: &Path, content: bool) -> Result<(), WatchError> {
        self.examine(now, path, content)
    }

    /// Replaces how new directories are watched, after switching event
    /// source. Call [`Engine::rescan`] next, so every directory is watched.
    #[cfg(unix)]
    pub fn set_dirs(&mut self, dirs: Box<dyn DirWatcher>) {
        self.dirs = dirs;
    }

    /// Classifies the absolute `path`. `content` says the event reported a
    /// write, so an unchanged size and time still count as a change.
    fn examine(&mut self, now: Instant, path: &Path, content: bool) -> Result<(), WatchError> {
        let Some(rel) = self.relative(path) else {
            return Ok(());
        };
        match classify(&rel) {
            PathClass::IndexLock => {
                self.check_lock(now);
                return Ok(());
            }
            PathClass::Excluded => {
                if rel.as_path() == Path::new(INDEX_LOCK[0]) && path.is_dir() {
                    // .git appeared (git init or clone): watch its lock.
                    self.dirs.watch_dir(path)?;
                }
                return Ok(());
            }
            PathClass::Watched if rel.as_os_str().is_empty() => return Ok(()),
            PathClass::Watched => {}
        }
        let found = std::fs::symlink_metadata(path)
            .ok()
            .map(|meta| Entry::of(&meta));
        let known = self.current.get(&rel).copied();
        match (found, known) {
            (None, None) => {}
            (None, Some(_)) => self.remove_subtree(now, &rel),
            (Some(found), None) => {
                self.current.insert(rel.clone(), found);
                self.coalescer.record(now, rel.clone(), ChangeKind::Created);
                if found.is_dir {
                    self.sync_dir(now, &rel)?;
                }
            }
            (Some(found), Some(known)) if found.is_dir != known.is_dir => {
                self.remove_subtree(now, &rel);
                self.current.insert(rel.clone(), found);
                self.coalescer.record(now, rel.clone(), ChangeKind::Created);
                if found.is_dir {
                    self.sync_dir(now, &rel)?;
                }
            }
            (Some(found), Some(_)) if found.is_dir => {
                // A directory renamed into place over an existing one, or an
                // event about its entries: bring its subtree up to date.
                self.sync_dir(now, &rel)?;
            }
            (Some(found), Some(known)) => {
                if content || found != known {
                    self.current.insert(rel.clone(), found);
                    self.coalescer.record(now, rel, ChangeKind::Changed);
                }
            }
        }
        Ok(())
    }

    /// Scans the directory `rel` (watching each directory first, which closes
    /// the race with files written before its watch existed) and records
    /// the difference from `current` under it.
    fn sync_dir(&mut self, now: Instant, rel: &Path) -> Result<(), WatchError> {
        let mut found = Snapshot::new();
        let root = &self.root;
        let dirs = &mut self.dirs;
        let scanned = snapshot::scan(root, rel, &mut found, &mut |dir| {
            dirs.watch_dir(&root.join(dir))
        });
        let changes = snapshot::diff(snapshot::descendants(&self.current, rel), &found);
        for change in changes {
            match found.get(&change.path) {
                Some(entry) => self.current.insert(change.path.clone(), *entry),
                None => self.current.remove(&change.path),
            };
            self.coalescer.record(now, change.path, change.kind);
        }
        scanned
    }

    /// Records `rel` and everything under it as deleted.
    fn remove_subtree(&mut self, now: Instant, rel: &Path) {
        let gone: Vec<PathBuf> = snapshot::subtree(&self.current, rel)
            .map(|(path, _)| path.clone())
            .collect();
        for path in gone {
            self.current.remove(&path);
            self.coalescer.record(now, path, ChangeKind::Deleted);
        }
    }

    /// Rescans the whole project; its difference from the last batch
    /// replaces the pending events.
    pub fn rescan(&mut self, now: Instant, cause: RescanCause) -> Result<(), WatchError> {
        let mut found = Snapshot::new();
        let root = &self.root;
        let dirs = &mut self.dirs;
        let scanned = snapshot::scan(root, Path::new(""), &mut found, &mut |dir| {
            dirs.watch_dir(&root.join(dir))
        });
        let changes = snapshot::diff(&self.baseline, &found);
        self.current = found;
        self.coalescer.replace_all(now, changes, cause);
        self.check_lock(now);
        scanned
    }

    /// Re-reads whether `.git/index.lock` exists.
    pub fn check_lock(&mut self, now: Instant) {
        let lock = INDEX_LOCK
            .iter()
            .fold(self.root.clone(), |path, name| path.join(name));
        self.coalescer.set_index_lock(now, lock.exists());
    }

    /// Whether flushing is paused for the index lock.
    pub fn paused(&self, now: Instant) -> bool {
        self.coalescer.paused(now)
    }

    /// When the next batch may be due.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.coalescer.next_deadline()
    }

    /// Flushes a batch if one is due, and moves the baseline to match it.
    pub fn poll(&mut self, now: Instant) -> Option<Batch> {
        let batch = self.coalescer.poll(now)?;
        for Change { path, .. } in &batch.changes {
            match self.current.get(path) {
                Some(entry) => self.baseline.insert(path.clone(), *entry),
                None => self.baseline.remove(path),
            };
        }
        Some(batch)
    }

    /// `path` relative to the root, or `None` if it is outside it.
    fn relative(&self, path: &Path) -> Option<PathBuf> {
        std::iter::once(&self.root)
            .chain(&self.aliases)
            .find_map(|root| path.strip_prefix(root).ok())
            .map(Path::to_path_buf)
    }
}

/// Messages to the watcher thread.
enum Message {
    Event(notify::Result<Event>),
    /// From the privileged helper's connection.
    #[cfg(unix)]
    Helper(std::io::Result<Notice>),
    Rescan,
    Stop,
}

/// A connected, subscribed helper, before its events are forwarded.
#[cfg(unix)]
struct Subscribed {
    client: Client,
    /// The root as the helper canonicalized it.
    root: PathBuf,
}

/// Connects to the helper `config` names and subscribes to `root`, or says
/// why not.
#[cfg(unix)]
fn subscribe(config: &HelperSocket, root: &Path) -> Result<Subscribed, String> {
    let socket = config.resolve().map_err(str::to_string)?;
    let mut client = Client::connect(&socket).map_err(|error| error.to_string())?;
    let root = client.subscribe(root).map_err(|error| error.to_string())?;
    Ok(Subscribed { client, root })
}

#[cfg(not(unix))]
fn subscribe(config: &HelperSocket, _root: &Path) -> Result<std::convert::Infallible, String> {
    match config.resolve() {
        Ok(_) => Err("there is no privileged helper on this OS".into()),
        Err(why) => Err(why.into()),
    }
}

/// Logs which mode a watcher is in (brief §4.3: "log which mode is
/// active").
fn log_mode(root: &Path, mode: &WatchMode) {
    eprintln!("codetags-watch: watching {} with {mode}", root.display());
}

/// A running watcher over one project root.
///
/// It watches on a thread of its own, with the privileged helper if one
/// answers ([`WatchConfig::helper`]) and with notify (inotify, FSEvents or
/// ReadDirectoryChangesW) otherwise, and sends [`Batch`]es of changes
/// relative to the root. Dropping it stops the thread.
pub struct Watcher {
    root: PathBuf,
    mode: Arc<Mutex<WatchMode>>,
    control: Sender<Message>,
    batches: Receiver<Result<Batch, WatchError>>,
    thread: Option<JoinHandle<()>>,
    /// Ends the helper connection's forwarding thread.
    #[cfg(unix)]
    helper: Option<Shutdown>,
}

impl Watcher {
    /// Starts watching `root` recursively, excluding build output,
    /// dependencies, git's internals and codetags' own index
    /// ([`crate::exclude`]).
    ///
    /// It first tries the privileged helper `config.helper` names, and uses
    /// notify if the helper is absent, refuses, or fails; it logs which on
    /// stderr, and [`Watcher::mode`] says too. The helper is never required.
    ///
    /// The initial scan and the first watches happen before this returns, so
    /// a watch limit fails here; later limits come from
    /// [`Watcher::recv_timeout`]. Paths that exist now are not reported.
    pub fn start(root: impl AsRef<Path>, config: WatchConfig) -> Result<Self, WatchError> {
        let given = root.as_ref();
        let root_error = |source| WatchError::Root {
            path: given.to_path_buf(),
            source,
        };
        let canonical = std::fs::canonicalize(given).map_err(root_error)?;
        if !canonical.is_dir() {
            return Err(root_error(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "not a directory",
            )));
        }
        let mut aliases = Vec::new();
        if let Ok(absolute) = std::path::absolute(given)
            && absolute != canonical
        {
            aliases.push(absolute);
        }

        let (control, inbox) = mpsc::channel();
        // The helper's subscription is in place before the scan, so nothing
        // written after the scan is missed.
        #[allow(unused_mut)]
        let (dirs, mode, mut helper): (Box<dyn DirWatcher>, _, _) =
            match subscribe(&config.helper, &canonical) {
                Ok(subscribed) => {
                    #[cfg(unix)]
                    {
                        if subscribed.root != canonical && !aliases.contains(&subscribed.root) {
                            aliases.push(subscribed.root.clone());
                        }
                        let mode = WatchMode::Helper {
                            backend: subscribed.client.backend().to_string(),
                            socket: subscribed.client.socket().to_path_buf(),
                        };
                        (Box::new(HelperCovers), mode, Some(subscribed.client))
                    }
                    #[cfg(not(unix))]
                    match subscribed {}
                }
                Err(why_not_helper) => (
                    notify_dirs(&canonical, control.clone())?,
                    WatchMode::Notify { why_not_helper },
                    None,
                ),
            };
        log_mode(&canonical, &mode);
        let engine = Engine::new(canonical.clone(), aliases, dirs, config, Instant::now())?;

        #[cfg(unix)]
        let helper = match helper.take() {
            Some(client) => {
                let shutdown = client.shutdown_handle().map_err(notify::Error::io);
                let events = control.clone();
                let forwarded = client
                    .forward(move |notice| events.send(Message::Helper(notice)).is_ok())
                    .map_err(notify::Error::io);
                match (shutdown, forwarded) {
                    (Ok(shutdown), Ok(_thread)) => Some(shutdown),
                    (Err(error), _) | (_, Err(error)) => {
                        return Err(WatchError::Notify(error));
                    }
                }
            }
            None => None,
        };
        #[cfg(not(unix))]
        let _: Option<std::convert::Infallible> = helper;

        let mode = Arc::new(Mutex::new(mode));
        let (outbox, batches) = mpsc::channel();
        let thread = {
            let mode = Arc::clone(&mode);
            let control = control.clone();
            std::thread::Builder::new()
                .name("codetags-watch".into())
                .spawn(move || run(engine, &inbox, &outbox, &control, &mode))
                .map_err(|error| WatchError::Notify(notify::Error::io(error)))?
        };
        Ok(Self {
            root: canonical,
            mode,
            control,
            batches,
            thread: Some(thread),
            #[cfg(unix)]
            helper,
        })
    }

    /// The canonical root that batch paths are relative to.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where the events come from now: the helper, or notify. It changes
    /// from the helper to notify if the helper goes away.
    pub fn mode(&self) -> WatchMode {
        match self.mode.lock() {
            Ok(mode) => mode.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Asks for a rescan, as an overflow does: the project is scanned, and
    /// its difference from the last batch replaces the pending events.
    pub fn rescan(&self) {
        let _ = self.control.send(Message::Rescan);
    }

    /// Waits up to `timeout` for the next batch. `Ok(None)` means none came;
    /// an error is a watch limit or other trouble, after which the watcher
    /// keeps running over what it can still watch, or
    /// [`WatchError::Stopped`].
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Option<Batch>, WatchError> {
        match self.batches.recv_timeout(timeout) {
            Ok(Ok(batch)) => Ok(Some(batch)),
            Ok(Err(error)) => Err(error),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(WatchError::Stopped),
        }
    }

    /// Waits for the next batch.
    pub fn recv(&self) -> Result<Batch, WatchError> {
        match self.batches.recv() {
            Ok(result) => result,
            Err(_) => Err(WatchError::Stopped),
        }
    }
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher")
            .field("root", &self.root)
            .field("mode", &self.mode())
            .finish_non_exhaustive()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // Stop the thread first, so the helper's hang-up is not taken for a
        // lost helper.
        let _ = self.control.send(Message::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        #[cfg(unix)]
        if let Some(helper) = self.helper.take() {
            helper.shutdown();
        }
    }
}

/// The watcher thread: feeds events to the engine and sends due batches.
/// `control` is the thread's own inbox, for notify's events if it has to
/// switch from the helper to notify.
fn run(
    mut engine: Engine,
    inbox: &Receiver<Message>,
    outbox: &Sender<Result<Batch, WatchError>>,
    control: &Sender<Message>,
    mode: &Mutex<WatchMode>,
) {
    #[cfg(not(unix))]
    let _ = (control, mode);
    loop {
        let now = Instant::now();
        let mut wait = engine
            .next_deadline()
            .map_or(IDLE, |deadline| deadline.saturating_duration_since(now));
        let locked = engine.paused(now);
        if locked {
            wait = wait.min(LOCK_POLL);
        }
        let result = match inbox.recv_timeout(wait) {
            Ok(Message::Event(event)) => engine.handle(Instant::now(), event),
            #[cfg(unix)]
            Ok(Message::Helper(notice)) => helper_notice(&mut engine, notice, control, mode),
            Ok(Message::Rescan) => engine.rescan(Instant::now(), RescanCause::Requested),
            Ok(Message::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => Ok(()),
        };
        if let Err(error) = result
            && outbox.send(Err(error)).is_err()
        {
            return;
        }
        if locked {
            engine.check_lock(Instant::now());
        }
        while let Some(batch) = engine.poll(Instant::now()) {
            if outbox.send(Ok(batch)).is_err() {
                return;
            }
        }
    }
}

/// Handles what the helper sent. If the connection failed, switches to
/// notify and rescans, so nothing missed meanwhile is lost.
#[cfg(unix)]
fn helper_notice(
    engine: &mut Engine,
    notice: std::io::Result<Notice>,
    control: &Sender<Message>,
    mode: &Mutex<WatchMode>,
) -> Result<(), WatchError> {
    let now = Instant::now();
    match notice {
        Ok(Notice::Event(event)) => {
            engine.hint(now, &event.path, event.kind == HelperKind::Modified)
        }
        Ok(Notice::Overflow) => engine.rescan(now, RescanCause::HelperOverflow),
        Err(error) => {
            let reason = format!("the privileged helper's connection ended ({error})");
            engine.set_dirs(notify_dirs(&engine.root, control.clone())?);
            let notify = WatchMode::Notify {
                why_not_helper: reason.clone(),
            };
            log_mode(&engine.root, &notify);
            match mode.lock() {
                Ok(mut mode) => *mode = notify,
                Err(poisoned) => *poisoned.into_inner() = notify,
            }
            engine.rescan(now, RescanCause::HelperLost(reason))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, Flag, RemoveKind, RenameMode};
    use std::fs;
    use std::sync::{Arc, Mutex};

    /// Records the directories it was asked to watch.
    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<PathBuf>>>);

    impl DirWatcher for Recorder {
        fn watch_dir(&mut self, dir: &Path) -> Result<(), WatchError> {
            self.0.lock().unwrap().push(dir.to_path_buf());
            Ok(())
        }
    }

    /// Fails every watch after the first `allowed`.
    struct Limited(usize);

    impl DirWatcher for Limited {
        fn watch_dir(&mut self, dir: &Path) -> Result<(), WatchError> {
            if self.0 == 0 {
                return Err(WatchError::from_notify(
                    notify::Error::new(notify::ErrorKind::MaxFilesWatch).add_path(dir.into()),
                ));
            }
            self.0 -= 1;
            Ok(())
        }
    }

    struct Fixture {
        dir: tempfile::TempDir,
        root: PathBuf,
        start: Instant,
        watched: Recorder,
        engine: Engine,
    }

    fn write(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn fixture(files: &[&str]) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        for file in files {
            write(&root, file, "original");
        }
        let watched = Recorder::default();
        let start = Instant::now();
        let engine = Engine::new(
            root.clone(),
            Vec::new(),
            Box::new(watched.clone()),
            WatchConfig::default(),
            start,
        )
        .unwrap();
        Fixture {
            dir,
            root,
            start,
            watched,
            engine,
        }
    }

    impl Fixture {
        fn at(&self, ms: u64) -> Instant {
            self.start + Duration::from_millis(ms)
        }

        fn event(&mut self, ms: u64, kind: EventKind, rels: &[&str]) {
            let mut event = Event::new(kind);
            for rel in rels {
                event = event.add_path(self.root.join(rel));
            }
            self.engine.handle(self.at(ms), Ok(event)).unwrap();
        }

        /// Flushes everything pending, as `kind:path` strings.
        fn flush(&mut self) -> (Vec<String>, Option<RescanCause>) {
            let batch = self.engine.poll(self.at(1_000_000)).unwrap_or_default();
            let changes = batch
                .changes
                .iter()
                .map(|c| format!("{}:{}", c.kind, c.path.to_string_lossy().replace('\\', "/")))
                .collect();
            (changes, batch.rescan)
        }

        fn watched(&self) -> Vec<String> {
            let mut dirs: Vec<String> = self
                .watched
                .0
                .lock()
                .unwrap()
                .iter()
                .map(|dir| {
                    dir.strip_prefix(&self.root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/")
                })
                .collect();
            dirs.sort();
            dirs.dedup();
            dirs
        }
    }

    const CREATE: EventKind = EventKind::Create(CreateKind::Any);
    const MODIFY: EventKind = EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any));
    const REMOVE: EventKind = EventKind::Remove(RemoveKind::Any);

    #[test]
    fn existing_files_are_not_reported_and_excluded_trees_are_not_watched() {
        let mut f = fixture(&["src/lib.rs", "node_modules/x/i.js", ".git/HEAD"]);
        assert_eq!(f.flush().0, Vec::<String>::new());
        assert_eq!(f.watched(), ["", ".git", "src"]);
    }

    #[test]
    fn events_are_classified_by_looking_at_the_path() {
        let mut f = fixture(&["a.rs", "b.rs"]);
        write(&f.root, "a.rs", "edited!");
        f.event(0, MODIFY, &["a.rs"]);
        fs::remove_file(f.root.join("b.rs")).unwrap();
        f.event(1, REMOVE, &["b.rs"]);
        write(&f.root, "c.rs", "new");
        f.event(2, CREATE, &["c.rs"]);
        // A create event for a file that is already gone reports nothing.
        f.event(3, CREATE, &["ghost.rs"]);
        assert_eq!(
            f.flush().0,
            ["changed:a.rs", "deleted:b.rs", "created:c.rs"]
        );
    }

    #[test]
    fn a_sed_style_rename_over_the_target_is_one_change() {
        let mut f = fixture(&["src/lib.rs"]);
        write(&f.root, "src/sedX1", "new content");
        f.event(0, CREATE, &["src/sedX1"]);
        fs::rename(f.root.join("src/sedX1"), f.root.join("src/lib.rs")).unwrap();
        f.event(
            1,
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            &["src/sedX1", "src/lib.rs"],
        );
        assert_eq!(f.flush().0, ["changed:src/lib.rs"]);
    }

    #[test]
    fn a_new_directory_is_scanned_and_watched() {
        let mut f = fixture(&["src/lib.rs"]);
        write(&f.root, "src/new/deep/a.rs", "a");
        write(&f.root, "src/new/deep/b.rs", "b");
        // Only the top directory's creation was seen.
        f.event(0, CREATE, &["src/new"]);
        // A late event for a file the scan already found adds nothing.
        f.event(1, CREATE, &["src/new/deep/a.rs"]);
        assert_eq!(
            f.flush().0,
            [
                "created:src/new",
                "created:src/new/deep",
                "created:src/new/deep/a.rs",
                "created:src/new/deep/b.rs"
            ]
        );
        assert_eq!(f.watched(), ["", "src", "src/new", "src/new/deep"]);
    }

    #[test]
    fn a_deleted_directory_takes_its_contents_with_it() {
        let mut f = fixture(&["src/gone/a.rs", "src/gone/z/b.rs", "src/keep.rs"]);
        fs::remove_dir_all(f.root.join("src/gone")).unwrap();
        f.event(0, REMOVE, &["src/gone"]);
        assert_eq!(
            f.flush().0,
            [
                "deleted:src/gone",
                "deleted:src/gone/a.rs",
                "deleted:src/gone/z",
                "deleted:src/gone/z/b.rs"
            ]
        );
    }

    #[test]
    fn a_renamed_directory_is_deleted_and_created() {
        let mut f = fixture(&["old/a.rs"]);
        fs::rename(f.root.join("old"), f.root.join("new")).unwrap();
        f.event(
            0,
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            &["old", "new"],
        );
        assert_eq!(
            f.flush().0,
            [
                "created:new",
                "created:new/a.rs",
                "deleted:old",
                "deleted:old/a.rs"
            ]
        );
    }

    #[test]
    fn excluded_paths_and_access_events_are_ignored() {
        let mut f = fixture(&["src/lib.rs", ".git/HEAD"]);
        write(&f.root, "target/x", "x");
        f.event(0, CREATE, &["target", "target/x"]);
        write(&f.root, ".git/HEAD", "ref");
        f.event(1, MODIFY, &[".git/HEAD"]);
        f.event(
            2,
            EventKind::Access(AccessKind::Open(AccessMode::Read)),
            &["src/lib.rs"],
        );
        let outside = f.dir.path().parent().unwrap().join("elsewhere");
        f.engine
            .handle(f.at(3), Ok(Event::new(MODIFY).add_path(outside)))
            .unwrap();
        assert_eq!(f.flush().0, Vec::<String>::new());
    }

    #[test]
    fn the_index_lock_pauses_the_batch_until_it_goes() {
        let mut f = fixture(&["src/a.rs", ".git/HEAD"]);
        write(&f.root, ".git/index.lock", "");
        f.event(0, CREATE, &[".git/index.lock"]);
        write(&f.root, "src/a.rs", "checked out");
        f.event(1, MODIFY, &["src/a.rs"]);
        assert!(f.engine.poll(f.at(10_000)).is_none());
        fs::rename(f.root.join(".git/index.lock"), f.root.join(".git/index")).unwrap();
        f.event(
            10_001,
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            &[".git/index.lock", ".git/index"],
        );
        let batch = f.engine.poll(f.at(10_001)).unwrap();
        assert_eq!(batch.changes.len(), 1);
    }

    #[test]
    fn an_overflow_rescans_and_replaces_pending_events_with_the_difference() {
        let mut f = fixture(&["a.rs", "b.rs"]);
        // Events seen before the overflow: one that will turn out stale.
        write(&f.root, "a.rs", "edited!");
        f.event(0, MODIFY, &["a.rs"]);
        // Changes whose events were lost.
        fs::remove_file(f.root.join("b.rs")).unwrap();
        write(&f.root, "c/d.rs", "new");
        // The edit to a.rs is undone: it was "original" (8 bytes) and the
        // rescan cannot tell a rewrite with the same size and time apart,
        // so write different content of a new length.
        write(&f.root, "a.rs", "edited again");
        f.engine
            .handle(
                f.at(1),
                Ok(Event::new(EventKind::Other).set_flag(Flag::Rescan)),
            )
            .unwrap();
        let (changes, rescan) = f.flush();
        assert_eq!(
            changes,
            [
                "changed:a.rs",
                "deleted:b.rs",
                "created:c",
                "created:c/d.rs"
            ]
        );
        assert_eq!(rescan, Some(RescanCause::Overflow));
        // The next batch diffs against the state the rescan reported.
        write(&f.root, "c/d.rs", "edited");
        f.event(2, MODIFY, &["c/d.rs"]);
        assert_eq!(f.flush(), (vec!["changed:c/d.rs".to_string()], None));
    }

    #[test]
    fn a_rescan_after_a_flush_reports_only_what_was_missed() {
        let mut f = fixture(&["a.rs"]);
        write(&f.root, "a.rs", "edited!");
        f.event(0, MODIFY, &["a.rs"]);
        assert_eq!(f.flush().0, ["changed:a.rs"]);
        f.engine.rescan(f.at(1), RescanCause::Requested).unwrap();
        assert_eq!(f.flush(), (Vec::new(), Some(RescanCause::Requested)));
    }

    #[test]
    fn other_watch_errors_trigger_a_rescan() {
        let mut f = fixture(&["a.rs"]);
        f.engine
            .handle(f.at(0), Err(notify::Error::generic("lost track")))
            .unwrap();
        let (_, rescan) = f.flush();
        assert!(matches!(rescan, Some(RescanCause::WatchError(m)) if m.contains("lost track")));
    }

    #[test]
    fn a_watch_limit_fails_start_with_the_limit_error() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a/b/c.rs", "x");
        let result = Engine::new(
            fs::canonicalize(dir.path()).unwrap(),
            Vec::new(),
            Box::new(Limited(1)),
            WatchConfig::default(),
            Instant::now(),
        );
        assert!(matches!(result, Err(WatchError::WatchLimit { .. })));
    }

    #[test]
    fn a_watch_limit_on_a_new_directory_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let mut engine = Engine::new(
            root.clone(),
            Vec::new(),
            Box::new(Limited(1)),
            WatchConfig::default(),
            Instant::now(),
        )
        .unwrap();
        write(&root, "new/a.rs", "x");
        let result = engine.handle(
            Instant::now(),
            Ok(Event::new(CREATE).add_path(root.join("new"))),
        );
        assert!(matches!(result, Err(WatchError::WatchLimit { .. })));
        // So is notify's own report of one.
        let error = notify::Error::new(notify::ErrorKind::MaxFilesWatch);
        assert!(matches!(
            engine.handle(Instant::now(), Err(error)),
            Err(WatchError::WatchLimit { .. })
        ));
    }

    /// A real ReadDirectoryChangesW overflow (D35, V56): notify 9.0.0-rc.5
    /// reports it as `Flag::Rescan`, the engine rescans, and the watch keeps
    /// reporting afterwards. notify 8.2 dropped it silently, or unwatched
    /// the directory.
    ///
    /// The overflow is forced, not hoped for. notify calls the handler from
    /// the completion routine, on its one watcher thread, so stalling the
    /// handler on the first event stalls the next completion too. Windows
    /// keeps buffering changes for the directory handle meanwhile, in a
    /// buffer the size of notify's read buffer (16 KiB), and discards them
    /// all once that fills: the next read completes with no data or with
    /// `ERROR_NOTIFY_ENUM_DIR`. A burst of 2,000 creates with 100-character
    /// names writes about 420 KiB of records, so it fills many times over.
    #[cfg(windows)]
    #[test]
    fn a_real_buffer_overflow_rescans_and_the_watch_keeps_reporting() {
        use std::sync::atomic::{AtomicBool, Ordering};
        const FILES: usize = 2_000;
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let (tx, rx) = mpsc::channel();
        let stall = Arc::new(AtomicBool::new(true));
        let first = Arc::clone(&stall);
        let mut notifier = notify::recommended_watcher(move |event| {
            if first.swap(false, Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(3));
            }
            let _ = tx.send(event);
        })
        .unwrap();
        notifier.watch(&root, RecursiveMode::Recursive).unwrap();
        let mut engine = Engine::new(
            root.clone(),
            Vec::new(),
            Box::new(Recursive(notifier)),
            WatchConfig::default(),
            Instant::now(),
        )
        .unwrap();

        let long = "x".repeat(100);
        for i in 0..FILES {
            fs::write(root.join(format!("{i:05}-{long}.rs")), "").unwrap();
        }

        // Feeds events to the engine until a flushed batch satisfies `done`.
        let pump = |engine: &mut Engine, what: &str, done: &dyn Fn(&Batch) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(60);
            let mut seen = Vec::new();
            loop {
                assert!(
                    Instant::now() < deadline,
                    "no batch with {what} within 60 s; batches seen (rescan, changes): {seen:?}"
                );
                if let Ok(event) = rx.recv_timeout(Duration::from_millis(100)) {
                    engine.handle(Instant::now(), event).unwrap();
                }
                let far = Instant::now() + Duration::from_secs(3600);
                while let Some(batch) = engine.poll(far) {
                    if done(&batch) {
                        return batch;
                    }
                    seen.push((batch.rescan.clone(), batch.changes.len()));
                }
            }
        };

        let rescanned = pump(&mut engine, "a rescan for the overflow", &|batch| {
            batch.rescan == Some(RescanCause::Overflow)
        });
        assert!(!stall.load(Ordering::SeqCst), "the handler never stalled");
        // The rescan reports what the lost events did not, against the last
        // batch: every file the burst made is accounted for, by events
        // flushed before it or by the rescan itself.
        assert!(rescanned.changes.len() <= FILES);

        // The watch survived: a later write is reported by an event.
        fs::write(root.join("after.rs"), "x").unwrap();
        let after = pump(&mut engine, "after.rs", &|batch| {
            batch
                .changes
                .iter()
                .any(|change| change.path == Path::new("after.rs"))
        });
        assert_eq!(
            after.rescan, None,
            "after.rs came from a rescan, not an event"
        );
    }

    /// A fake helper on a Unix socket: welcomes one client, accepts its
    /// subscription, then sends what arrives on the returned channel, and
    /// hangs up when that channel closes.
    #[cfg(unix)]
    fn fake_helper(dir: &Path) -> (PathBuf, mpsc::Sender<crate::privhelper::proto::Reply>) {
        use crate::privhelper::proto::{Reply, Request, VERSION, read_frame};
        use std::os::unix::net::UnixListener;
        let socket = dir.join("helper.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let (tx, rx) = mpsc::channel::<Reply>();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let (tag, body) = read_frame(&mut stream).unwrap().unwrap();
            assert_eq!(
                Request::decode(tag, &body).unwrap(),
                Request::Hello { version: VERSION }
            );
            Reply::Welcome {
                version: VERSION,
                backend: "fanotify".into(),
            }
            .write_to(&mut stream)
            .unwrap();
            let (tag, body) = read_frame(&mut stream).unwrap().unwrap();
            let Request::Subscribe { root } = Request::decode(tag, &body).unwrap() else {
                panic!("expected a subscription");
            };
            Reply::Accepted { root }.write_to(&mut stream).unwrap();
            for reply in rx {
                reply.write_to(&mut stream).unwrap();
            }
        });
        (socket, tx)
    }

    /// Waits for batches until their merged changes equal `expected`.
    #[cfg(unix)]
    fn expect(watcher: &Watcher, expected: &[(&str, ChangeKind)]) -> Vec<Batch> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = std::collections::BTreeMap::new();
        let mut batches = Vec::new();
        let expected: std::collections::BTreeMap<String, ChangeKind> = expected
            .iter()
            .map(|(path, kind)| (path.to_string(), *kind))
            .collect();
        while seen != expected {
            assert!(Instant::now() < deadline, "saw {seen:?}, not {expected:?}");
            if let Some(batch) = watcher.recv_timeout(Duration::from_millis(50)).unwrap() {
                for change in &batch.changes {
                    seen.insert(change.path.to_string_lossy().into_owned(), change.kind);
                }
                batches.push(batch);
            }
        }
        batches
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_helper_falls_back_to_notify() {
        let dir = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let config = WatchConfig {
            helper: HelperSocket::At(dir.path().join("absent.sock")),
            ..WatchConfig::default()
        };
        let watcher = Watcher::start(project.path(), config).unwrap();
        let mode = watcher.mode();
        assert_eq!(mode.source(), "notify");
        assert!(
            mode.to_string().contains("no privileged helper at"),
            "{mode}"
        );
        write(project.path(), "a.rs", "x");
        expect(&watcher, &[("a.rs", ChangeKind::Created)]);
    }

    #[cfg(unix)]
    #[test]
    fn helper_events_feed_the_coalescer_and_a_lost_helper_falls_back() {
        use crate::privhelper::proto::{EventKind as Kind, HelperEvent, Reply};
        let dir = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let (socket, helper) = fake_helper(dir.path());
        let config = WatchConfig {
            helper: HelperSocket::At(socket),
            ..WatchConfig::default()
        };
        let watcher = Watcher::start(project.path(), config).unwrap();
        assert_eq!(watcher.mode().source(), "fanotify");
        let root = watcher.root().to_path_buf();

        // Only the helper's events count: notify is not running, so a write
        // the helper does not report is not seen until it does.
        write(&root, "a.rs", "x");
        assert!(
            watcher
                .recv_timeout(Duration::from_millis(500))
                .unwrap()
                .is_none()
        );
        helper
            .send(Reply::Event(HelperEvent {
                path: root.join("a.rs"),
                kind: Kind::Created,
                pid: Some(1),
            }))
            .unwrap();
        expect(&watcher, &[("a.rs", ChangeKind::Created)]);

        // An overflow rescans.
        write(&root, "b.rs", "x");
        helper.send(Reply::Overflow).unwrap();
        let batches = expect(&watcher, &[("b.rs", ChangeKind::Created)]);
        assert!(
            batches
                .iter()
                .any(|b| b.rescan == Some(RescanCause::HelperOverflow))
        );

        // The helper goes away: the watcher rescans, and notify takes over.
        write(&root, "c.rs", "x");
        drop(helper);
        let batches = expect(&watcher, &[("c.rs", ChangeKind::Created)]);
        assert!(
            batches
                .iter()
                .any(|b| matches!(b.rescan, Some(RescanCause::HelperLost(_))))
        );
        assert_eq!(watcher.mode().source(), "notify");
        write(&root, "d.rs", "x");
        expect(&watcher, &[("d.rs", ChangeKind::Created)]);
    }
}
