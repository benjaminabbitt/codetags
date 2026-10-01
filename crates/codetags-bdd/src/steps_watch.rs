//! Steps for `features/watch/`: the coalescer over a fake clock (PLAN.md
//! P4.2).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use codetags_watch::{Batch, ChangeKind, CoalesceConfig, Coalescer};
use cucumber::{given, then, when};

use crate::CodetagsWorld;

/// State the watch steps share within one scenario.
#[derive(Debug, Default)]
pub struct WatchState {
    /// The coalescer under test, and the fake clock's zero.
    coalescer: Option<(Coalescer, Instant)>,
    /// The batch the last coalescer `Then` flushed.
    flushed: Option<Batch>,
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
