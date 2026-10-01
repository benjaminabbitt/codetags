//! Steps for `features/watch/`: the coalescer over a fake clock, and a real
//! watcher over a scratch project (PLAN.md P4.1, P4.2).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use codetags_watch::{Batch, ChangeKind, CoalesceConfig, Coalescer, WatchConfig, Watcher, merge};
use cucumber::{given, then, when};

use crate::CodetagsWorld;

/// How long the watcher's report must stay unchanged after it first matches.
const SETTLE: Duration = Duration::from_secs(1);

/// How long one wait for a batch lasts while polling for a deadline.
const TICK: Duration = Duration::from_millis(50);

/// State the watch steps share within one scenario.
#[derive(Debug, Default)]
pub struct WatchState {
    /// The coalescer under test, and the fake clock's zero.
    coalescer: Option<(Coalescer, Instant)>,
    /// The batch the last coalescer `Then` flushed.
    flushed: Option<Batch>,
    /// The running watcher. Declared before `project` so it stops before
    /// the project directory is deleted.
    watcher: Option<Watcher>,
    /// The scratch directory holding the project.
    project: Option<tempfile::TempDir>,
    /// Writes so far, so each write's content differs.
    writes: u64,
    /// The watcher's batches since the last assertion, merged per path.
    seen: BTreeMap<String, ChangeKind>,
    /// Whether any batch since the watch started was a rescan.
    rescanned: bool,
}

/// Parses a *changes* string: `kind:path` tokens, as a sorted map.
fn changes(spec: &str) -> BTreeMap<String, ChangeKind> {
    spec.split_whitespace()
        .map(|token| {
            let (kind, path) = token
                .split_once(':')
                .unwrap_or_else(|| panic!("{token:?} is not <kind>:<path>"));
            (path.to_string(), kind.parse().unwrap())
        })
        .collect()
}

/// Renders a batch's changes as a map from `/`-separated path to kind.
fn batch_changes(batch: &Batch) -> BTreeMap<String, ChangeKind> {
    batch
        .changes
        .iter()
        .map(|change| (slashed(&change.path), change.kind))
        .collect()
}

fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn render(changes: &BTreeMap<String, ChangeKind>) -> String {
    changes
        .iter()
        .map(|(path, kind)| format!("{kind}:{path}"))
        .collect::<Vec<_>>()
        .join(" ")
}

impl WatchState {
    fn coalescer(&mut self) -> &mut (Coalescer, Instant) {
        self.coalescer
            .as_mut()
            .expect("an earlier step created a coalescer")
    }

    fn at(&mut self, ms: u64) -> Instant {
        self.coalescer().1 + Duration::from_millis(ms)
    }

    fn project(&mut self) -> &Path {
        self.project
            .get_or_insert_with(|| tempfile::tempdir().expect("create project dir"))
            .path()
    }

    /// Writes `rel` (creating its parents) with content unique to this write.
    fn write(&mut self, rel: &str) {
        self.writes += 1;
        let content = format!("{rel} {}\n", self.writes);
        let path = self.project().join(rel);
        fs::create_dir_all(path.parent().expect("a file has a parent")).expect("create parents");
        fs::write(&path, content).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    }

    fn watcher(&self) -> &Watcher {
        self.watcher
            .as_ref()
            .expect("an earlier step started the watcher")
    }

    /// Waits up to `timeout` for batches, merging them into `seen`.
    fn drain(&mut self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let batch = match self.watcher().recv_timeout(left.min(TICK)) {
                Ok(batch) => batch,
                Err(error) => panic!("the watcher failed: {error}"),
            };
            if let Some(batch) = batch {
                self.rescanned |= batch.rescan.is_some();
                for change in &batch.changes {
                    let path = slashed(&change.path);
                    let merged = match self.seen.get(&path) {
                        Some(&prev) => merge(prev, change.kind),
                        None => Some(change.kind),
                    };
                    match merged {
                        Some(kind) => self.seen.insert(path, kind),
                        None => self.seen.remove(&path),
                    };
                }
            }
            if left.is_zero() {
                return;
            }
        }
    }

    /// Passes once `seen` equals `expected` within `seconds`, and stays so
    /// for [`SETTLE`]; then starts afresh.
    fn expect_within(&mut self, seconds: u64, expected: &BTreeMap<String, ChangeKind>) {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        while &self.seen != expected {
            assert!(
                Instant::now() < deadline,
                "after {seconds} s the watcher had reported {:?}, not {:?}",
                render(&self.seen),
                render(expected)
            );
            self.drain(TICK);
        }
        self.drain(SETTLE);
        assert_eq!(
            render(&self.seen),
            render(expected),
            "the watcher reported more after the expected changes"
        );
        self.seen.clear();
    }
}

// --- coalescer ---------------------------------------------------------------

fn start_coalescer(world: &mut CodetagsWorld, config: CoalesceConfig) {
    world.watch.coalescer = Some((Coalescer::new(config), Instant::now()));
}

#[given(expr = "a coalescer with a quiet window of {int} ms and a maximum wait of {int} ms")]
fn coalescer(world: &mut CodetagsWorld, quiet: u64, max_wait: u64) {
    start_coalescer(
        world,
        CoalesceConfig {
            quiet: Duration::from_millis(quiet),
            max_wait: Duration::from_millis(max_wait),
            ..CoalesceConfig::default()
        },
    );
}

#[given(
    expr = "a coalescer with a quiet window of {int} ms, a maximum wait of {int} ms and an index-lock cap of {int} ms"
)]
fn coalescer_with_cap(world: &mut CodetagsWorld, quiet: u64, max_wait: u64, cap: u64) {
    start_coalescer(
        world,
        CoalesceConfig {
            quiet: Duration::from_millis(quiet),
            max_wait: Duration::from_millis(max_wait),
            lock_cap: Duration::from_millis(cap),
        },
    );
}

#[when(expr = "at {int} ms the watcher sees {string} {word}")]
fn sees(world: &mut CodetagsWorld, ms: u64, path: String, kind: String) {
    let now = world.watch.at(ms);
    let kind: ChangeKind = kind.parse().unwrap();
    world
        .watch
        .coalescer()
        .0
        .record(now, PathBuf::from(path), kind);
}

#[when(expr = "at {int} ms git takes its index lock")]
fn coalescer_lock(world: &mut CodetagsWorld, ms: u64) {
    let now = world.watch.at(ms);
    world.watch.coalescer().0.set_index_lock(now, true);
}

#[when(expr = "at {int} ms git releases its index lock")]
fn coalescer_unlock(world: &mut CodetagsWorld, ms: u64) {
    let now = world.watch.at(ms);
    world.watch.coalescer().0.set_index_lock(now, false);
}

#[when(expr = "at {int} ms a rescan finds {string}")]
fn rescan_finds(world: &mut CodetagsWorld, ms: u64, found: String) {
    let now = world.watch.at(ms);
    let found = changes(&found)
        .into_iter()
        .map(|(path, kind)| codetags_watch::Change {
            path: PathBuf::from(path),
            kind,
        })
        .collect();
    world
        .watch
        .coalescer()
        .0
        .replace_all(now, found, codetags_watch::RescanCause::Overflow);
}

#[then(expr = "at {int} ms the coalescer flushes exactly {string}")]
fn flushes_exactly(world: &mut CodetagsWorld, ms: u64, expected: String) {
    let now = world.watch.at(ms);
    let batch = world.watch.coalescer().0.poll(now);
    let expected = changes(&expected);
    match &batch {
        None => assert!(
            expected.is_empty(),
            "the coalescer flushed nothing, not {:?}",
            render(&expected)
        ),
        Some(batch) => assert_eq!(render(&batch_changes(batch)), render(&expected)),
    }
    world.watch.flushed = batch;
}

#[then(expr = "at {int} ms the coalescer flushes nothing")]
fn flushes_nothing(world: &mut CodetagsWorld, ms: u64) {
    let now = world.watch.at(ms);
    if let Some(batch) = world.watch.coalescer().0.poll(now) {
        panic!("the coalescer flushed {:?}", render(&batch_changes(&batch)));
    }
}

#[then("the flushed batch is marked as a rescan")]
fn flushed_rescan(world: &mut CodetagsWorld) {
    let batch = world
        .watch
        .flushed
        .as_ref()
        .expect("an earlier step flushed a batch");
    assert!(batch.rescan.is_some(), "{batch:?}");
}

// --- watcher -----------------------------------------------------------------

#[given(expr = "a project directory holding the files {string}")]
fn project_files(world: &mut CodetagsWorld, files: String) {
    world.watch.project();
    for file in files.split_whitespace() {
        world.watch.write(file);
    }
}

/// The names of the `count` numbered files in `dir`.
fn numbered(dir: &str, count: usize) -> Vec<String> {
    (1..=count).map(|k| format!("{dir}/f{k:03}.rs")).collect()
}

#[given(expr = "a project directory holding {int} files in {string}")]
fn project_numbered(world: &mut CodetagsWorld, count: usize, dir: String) {
    for file in numbered(&dir, count) {
        world.watch.write(&file);
    }
}

#[given("the project is being watched")]
fn watched(world: &mut CodetagsWorld) {
    let root = world.watch.project().to_path_buf();
    let watcher = Watcher::start(&root, WatchConfig::default())
        .unwrap_or_else(|error| panic!("start watching {}: {error}", root.display()));
    world.watch.watcher = Some(watcher);
}

#[when(expr = "the file {string} is written")]
fn file_written(world: &mut CodetagsWorld, file: String) {
    world.watch.write(&file);
}

#[when(expr = "the file {string} is replaced the way sed -i does it")]
fn sed_i(world: &mut CodetagsWorld, file: String) {
    let target = world.watch.project().join(&file);
    let temp = target.with_file_name(format!("sed{:06}", world.watch.writes));
    world.watch.writes += 1;
    let content = format!("{file} {} (sed)\n", world.watch.writes);
    fs::write(&temp, content).expect("write the temporary file");
    fs::rename(&temp, &target).expect("rename it over the target");
}

#[when(expr = "a formatter rewrites every file in {string}")]
fn formatter(world: &mut CodetagsWorld, dir: String) {
    let mut files: Vec<PathBuf> = fs::read_dir(world.watch.project().join(&dir))
        .expect("read the directory")
        .map(|entry| entry.expect("read an entry").path())
        .collect();
    files.sort();
    for file in files {
        world.watch.writes += 1;
        fs::write(&file, format!("formatted {}\n", world.watch.writes)).expect("rewrite");
    }
}

#[when(expr = "the file {string} is deleted")]
fn file_deleted(world: &mut CodetagsWorld, file: String) {
    fs::remove_file(world.watch.project().join(file)).expect("delete the file");
}

#[when(expr = "the directory {string} is created holding the files {string}")]
fn directory_created(world: &mut CodetagsWorld, dir: String, files: String) {
    fs::create_dir_all(world.watch.project().join(&dir)).expect("create the directory");
    for file in files.split_whitespace() {
        world.watch.write(&format!("{dir}/{file}"));
    }
}

#[when(expr = "the directory {string} is deleted")]
fn directory_deleted(world: &mut CodetagsWorld, dir: String) {
    fs::remove_dir_all(world.watch.project().join(dir)).expect("delete the directory");
}

#[when("git takes its index lock")]
fn lock(world: &mut CodetagsWorld) {
    let git = world.watch.project().join(".git");
    fs::create_dir_all(&git).expect("create .git");
    fs::write(git.join("index.lock"), "").expect("create index.lock");
}

#[when("git releases its index lock")]
fn unlock(world: &mut CodetagsWorld) {
    let git = world.watch.project().join(".git");
    fs::rename(git.join("index.lock"), git.join("index")).expect("rename index.lock");
}

#[when("the watcher is told its events overflowed")]
fn overflowed(world: &mut CodetagsWorld) {
    world.watch.watcher().rescan();
}

#[then(expr = "within {int} seconds the watcher reports exactly {string}")]
fn reports_exactly(world: &mut CodetagsWorld, seconds: u64, expected: String) {
    world.watch.expect_within(seconds, &changes(&expected));
}

#[then(
    expr = "within {int} seconds the watcher reports {int} changed files in {string} and nothing else"
)]
fn reports_numbered(world: &mut CodetagsWorld, seconds: u64, count: usize, dir: String) {
    let expected = numbered(&dir, count)
        .into_iter()
        .map(|file| (file, ChangeKind::Changed))
        .collect();
    world.watch.expect_within(seconds, &expected);
}

#[then(expr = "the watcher reports nothing for {int} seconds")]
fn reports_nothing(world: &mut CodetagsWorld, seconds: u64) {
    world.watch.drain(Duration::from_secs(seconds));
    assert!(
        world.watch.seen.is_empty(),
        "the watcher reported {:?}",
        render(&world.watch.seen)
    );
}

#[then("a batch the watcher reported was marked as a rescan")]
fn reported_rescan(world: &mut CodetagsWorld) {
    assert!(world.watch.rescanned, "no batch was marked as a rescan");
}
