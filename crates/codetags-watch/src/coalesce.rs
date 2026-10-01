//! The coalescer (brief §4.3, PLAN.md P4.2): pure logic over a caller's clock.
//!
//! The watcher feeds it classified events and polls it; it never reads the
//! clock or the file system itself, so it is tested with a fake clock.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::change::{Batch, Change, ChangeKind, RescanCause, merge};

/// When the coalescer flushes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoalesceConfig {
    /// Flush once no event has arrived for this long.
    ///
    /// Default 200 ms. An editor's save, `sed -i` and `git checkout` each
    /// emit their events within a few milliseconds of one another, so 200 ms
    /// gathers each into one batch on every OS (FSEvents delivers within
    /// tens of milliseconds at notify's zero latency), while an agent that
    /// edits and then queries still sees the change well inside a second.
    /// Watchman's 20 ms settle is tuned for queries, not for starting a
    /// reindex, where a split batch costs a whole provider run.
    pub quiet: Duration,
    /// Flush once the oldest pending event has waited this long, even if
    /// events keep arriving. Default 2 s: ten quiet windows, so a long
    /// formatter run or a build writing into watched directories still
    /// reaches the servers every two seconds.
    pub max_wait: Duration,
    /// Stop pausing for `.git/index.lock` once it has existed this long.
    ///
    /// Default 30 s. A checkout of a large repository finishes well inside
    /// it, and a lock left behind by a crashed git would otherwise hold every
    /// event until someone deletes it. Past the cap the coalescer flushes as
    /// if there were no lock; a checkout still running then is reported in
    /// several batches, which is slower but not wrong.
    pub lock_cap: Duration,
}

impl Default for CoalesceConfig {
    fn default() -> Self {
        Self {
            quiet: Duration::from_millis(200),
            max_wait: Duration::from_secs(2),
            lock_cap: Duration::from_secs(30),
        }
    }
}

/// Merges events per path and decides when to flush them as a [`Batch`].
#[derive(Debug, Clone)]
pub struct Coalescer {
    config: CoalesceConfig,
    pending: BTreeMap<PathBuf, ChangeKind>,
    /// When the oldest event of the pending batch arrived.
    first: Option<Instant>,
    /// When the latest event arrived.
    last: Option<Instant>,
    /// When `.git/index.lock` was first seen, while it exists.
    lock_since: Option<Instant>,
    /// When the lock went away while events were pending: flush then.
    released: Option<Instant>,
    /// Set when a rescan replaced the pending events.
    rescan: Option<RescanCause>,
}

impl Coalescer {
    /// A coalescer with nothing pending and no index lock.
    pub fn new(config: CoalesceConfig) -> Self {
        Self {
            config,
            pending: BTreeMap::new(),
            first: None,
            last: None,
            lock_since: None,
            released: None,
            rescan: None,
        }
    }

    /// Records an event on `path` at `now`, merged with any pending event on
    /// the same path.
    pub fn record(&mut self, now: Instant, path: PathBuf, kind: ChangeKind) {
        let merged = match self.pending.get(&path) {
            Some(&prev) => merge(prev, kind),
            None => Some(kind),
        };
        match merged {
            Some(kind) => {
                self.pending.insert(path, kind);
            }
            None => {
                self.pending.remove(&path);
            }
        }
        self.touch(now);
    }

    /// Replaces every pending event with a rescan's difference from the last
    /// batch, and marks the next batch as a rescan for `cause`. The next
    /// batch is flushed even if `changes` is empty, so consumers learn of
    /// the rescan.
    pub fn replace_all(&mut self, now: Instant, changes: Vec<Change>, cause: RescanCause) {
        self.pending = changes
            .into_iter()
            .map(|change| (change.path, change.kind))
            .collect();
        self.rescan = Some(cause);
        self.touch(now);
    }

    /// Records whether `.git/index.lock` exists at `now`.
    pub fn set_index_lock(&mut self, now: Instant, held: bool) {
        match (held, self.lock_since) {
            (true, None) => self.lock_since = Some(now),
            (false, Some(_)) => {
                self.lock_since = None;
                self.released = self.has_batch().then_some(now);
            }
            _ => {}
        }
    }

    /// Whether flushing is paused for the index lock at `now`.
    pub fn paused(&self, now: Instant) -> bool {
        self.lock_since
            .is_some_and(|since| now.saturating_duration_since(since) < self.config.lock_cap)
    }

    /// Flushes the pending batch if it is due at `now`.
    pub fn poll(&mut self, now: Instant) -> Option<Batch> {
        let due = self.next_deadline().is_some_and(|deadline| now >= deadline);
        if !due || self.paused(now) {
            return None;
        }
        let changes = std::mem::take(&mut self.pending)
            .into_iter()
            .map(|(path, kind)| Change { path, kind })
            .collect();
        let rescan = self.rescan.take();
        self.first = None;
        self.last = None;
        self.released = None;
        Some(Batch { changes, rescan })
    }

    /// The next time [`Coalescer::poll`] may flush, if anything is pending.
    pub fn next_deadline(&self) -> Option<Instant> {
        if !self.has_batch() {
            return None;
        }
        let (first, last) = (self.first?, self.last?);
        let due = match self.released {
            Some(released) => released,
            None => (last + self.config.quiet).min(first + self.config.max_wait),
        };
        // A held lock defers the flush to the cap at the earliest.
        Some(match self.lock_since {
            Some(since) => due.max(since + self.config.lock_cap),
            None => due,
        })
    }

    /// Whether a batch is pending: changes, or a rescan to report.
    fn has_batch(&self) -> bool {
        !self.pending.is_empty() || self.rescan.is_some()
    }

    /// Updates the event times after an event at `now`. A batch whose events
    /// all cancelled out starts afresh, so its maximum wait does not carry
    /// over.
    fn touch(&mut self, now: Instant) {
        if self.has_batch() {
            self.first.get_or_insert(now);
            self.last = Some(now);
        } else {
            self.first = None;
            self.last = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ChangeKind::{Changed, Created, Deleted};

    fn config(quiet: u64, max_wait: u64, lock_cap: u64) -> CoalesceConfig {
        CoalesceConfig {
            quiet: Duration::from_millis(quiet),
            max_wait: Duration::from_millis(max_wait),
            lock_cap: Duration::from_millis(lock_cap),
        }
    }

    struct Clock(Instant);

    impl Clock {
        fn at(&self, ms: u64) -> Instant {
            self.0 + Duration::from_millis(ms)
        }
    }

    fn paths(batch: &Batch) -> Vec<(String, ChangeKind)> {
        batch
            .changes
            .iter()
            .map(|c| (c.path.to_string_lossy().into_owned(), c.kind))
            .collect()
    }

    fn setup() -> (Clock, Coalescer) {
        (
            Clock(Instant::now()),
            Coalescer::new(config(100, 300, 2000)),
        )
    }

    #[test]
    fn nothing_pending_flushes_nothing() {
        let (clock, mut c) = setup();
        assert_eq!(c.poll(clock.at(10_000)), None);
        assert_eq!(c.next_deadline(), None);
    }

    #[test]
    fn flushes_after_the_quiet_window_with_merged_events() {
        let (clock, mut c) = setup();
        c.record(clock.at(0), "a".into(), Created);
        c.record(clock.at(10), "a".into(), Changed);
        c.record(clock.at(20), "b".into(), Created);
        c.record(clock.at(30), "b".into(), Deleted);
        assert_eq!(c.next_deadline(), Some(clock.at(130)));
        assert_eq!(c.poll(clock.at(129)), None);
        let batch = c.poll(clock.at(130)).unwrap();
        assert_eq!(paths(&batch), vec![("a".to_string(), Created)]);
        assert_eq!(batch.rescan, None);
        assert_eq!(c.poll(clock.at(1000)), None);
    }

    #[test]
    fn a_batch_whose_events_all_cancel_is_not_flushed() {
        let (clock, mut c) = setup();
        c.record(clock.at(0), "a".into(), Created);
        c.record(clock.at(1), "a".into(), Deleted);
        assert_eq!(c.poll(clock.at(500)), None);
        assert_eq!(c.next_deadline(), None);
    }

    #[test]
    fn the_maximum_wait_caps_continuous_writes() {
        let (clock, mut c) = setup();
        for ms in (0..=290).step_by(50) {
            c.record(clock.at(ms), format!("f{ms}").into(), Changed);
        }
        assert_eq!(c.next_deadline(), Some(clock.at(300)));
        assert_eq!(c.poll(clock.at(299)), None);
        assert_eq!(c.poll(clock.at(300)).unwrap().changes.len(), 6);
    }

    #[test]
    fn the_index_lock_pauses_until_it_goes() {
        let (clock, mut c) = setup();
        c.set_index_lock(clock.at(0), true);
        c.record(clock.at(5), "a".into(), Changed);
        assert!(c.paused(clock.at(1000)));
        assert_eq!(c.poll(clock.at(1000)), None);
        assert_eq!(c.next_deadline(), Some(clock.at(2000)));
        c.set_index_lock(clock.at(1500), false);
        assert_eq!(c.next_deadline(), Some(clock.at(1500)));
        assert_eq!(paths(&c.poll(clock.at(1500)).unwrap()).len(), 1);
    }

    #[test]
    fn retaking_the_lock_does_not_restart_the_cap() {
        let (clock, mut c) = setup();
        c.set_index_lock(clock.at(0), true);
        c.set_index_lock(clock.at(500), true);
        assert!(!c.paused(clock.at(2000)));
    }

    #[test]
    fn a_stale_lock_stops_pausing_at_the_cap() {
        let (clock, mut c) = setup();
        c.set_index_lock(clock.at(0), true);
        c.record(clock.at(10), "a".into(), Changed);
        assert_eq!(c.poll(clock.at(1999)), None);
        assert!(c.poll(clock.at(2000)).is_some());
    }

    #[test]
    fn releasing_the_lock_with_nothing_pending_flushes_nothing() {
        let (clock, mut c) = setup();
        c.set_index_lock(clock.at(0), true);
        c.set_index_lock(clock.at(10), false);
        assert_eq!(c.poll(clock.at(10)), None);
        c.record(clock.at(20), "a".into(), Changed);
        assert_eq!(c.poll(clock.at(21)), None, "the release flushed only then");
        assert!(c.poll(clock.at(120)).is_some());
    }

    #[test]
    fn a_rescan_replaces_pending_events_and_marks_the_batch() {
        let (clock, mut c) = setup();
        c.record(clock.at(0), "a".into(), Changed);
        c.replace_all(
            clock.at(10),
            vec![Change {
                path: "b".into(),
                kind: Deleted,
            }],
            RescanCause::Overflow,
        );
        let batch = c.poll(clock.at(110)).unwrap();
        assert_eq!(paths(&batch), vec![("b".to_string(), Deleted)]);
        assert_eq!(batch.rescan, Some(RescanCause::Overflow));
        c.record(clock.at(200), "c".into(), Changed);
        assert_eq!(c.poll(clock.at(300)).unwrap().rescan, None);
    }

    #[test]
    fn an_empty_rescan_still_flushes_a_batch() {
        let (clock, mut c) = setup();
        c.replace_all(clock.at(0), Vec::new(), RescanCause::Requested);
        let batch = c.poll(clock.at(100)).unwrap();
        assert!(batch.changes.is_empty());
        assert_eq!(batch.rescan, Some(RescanCause::Requested));
    }
}
