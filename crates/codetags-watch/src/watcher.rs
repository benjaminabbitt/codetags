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
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use notify::event::{AccessKind, AccessMode, ModifyKind};
use notify::{Event, EventKind, RecursiveMode, Watcher as _};

use crate::change::{Batch, Change, ChangeKind, RescanCause};
use crate::coalesce::{CoalesceConfig, Coalescer};
use crate::error::WatchError;
use crate::exclude::{INDEX_LOCK, PathClass, classify};
use crate::snapshot::{self, Entry, Snapshot};

/// How often the watcher re-checks `.git/index.lock` while it exists, in
/// case its removal event is missed.
const LOCK_POLL: Duration = Duration::from_millis(100);

/// How long the watcher thread sleeps when nothing is pending.
const IDLE: Duration = Duration::from_secs(3600);

/// Watcher settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WatchConfig {
    /// When batches are flushed.
    pub coalesce: CoalesceConfig,
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
    Rescan,
    Stop,
}

/// A running watcher over one project root.
///
/// It watches with notify (inotify, FSEvents or ReadDirectoryChangesW) on a
/// thread of its own, and sends [`Batch`]es of changes relative to the root.
/// Dropping it stops the thread.
pub struct Watcher {
    root: PathBuf,
    control: Sender<Message>,
    batches: Receiver<Result<Batch, WatchError>>,
    thread: Option<JoinHandle<()>>,
}

impl Watcher {
    /// Starts watching `root` recursively, excluding build output,
    /// dependencies, git's internals and codetags' own index
    /// ([`crate::exclude`]).
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
        let events = control.clone();
        let mut notifier = notify::recommended_watcher(move |event| {
            let _ = events.send(Message::Event(event));
        })
        .map_err(WatchError::from_notify)?;
        let dirs: Box<dyn DirWatcher> = if cfg!(any(target_os = "macos", windows)) {
            notifier
                .watch(&canonical, RecursiveMode::Recursive)
                .map_err(WatchError::from_notify)?;
            Box::new(Recursive(notifier))
        } else {
            Box::new(PerDirectory(notifier))
        };
        let engine = Engine::new(canonical.clone(), aliases, dirs, config, Instant::now())?;

        let (outbox, batches) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("codetags-watch".into())
            .spawn(move || run(engine, &inbox, &outbox))
            .map_err(|error| WatchError::Notify(notify::Error::io(error)))?;
        Ok(Self {
            root: canonical,
            control,
            batches,
            thread: Some(thread),
        })
    }

    /// The canonical root that batch paths are relative to.
    pub fn root(&self) -> &Path {
        &self.root
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
            .finish_non_exhaustive()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.control.send(Message::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The watcher thread: feeds events to the engine and sends due batches.
fn run(mut engine: Engine, inbox: &Receiver<Message>, outbox: &Sender<Result<Batch, WatchError>>) {
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
}
