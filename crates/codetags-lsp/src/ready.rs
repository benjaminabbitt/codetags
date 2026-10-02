//! The readiness gate (P3.12; brief §4.3, R38): an agent session's requests
//! wait until the server has loaded the workspace, with a bound.
//!
//! An agent cannot tell "not loaded yet" from "no such symbol": a request
//! that races the server's first load gets an empty answer, which the agent
//! reports as a fact (V128, V141). Editors show the server's progress
//! themselves, so only agent sessions are held. Notifications and responses
//! are never held.
//!
//! The signal is rust-analyzer's `experimental/serverStatus` notification,
//! which it sends only when the `initialize` it received asked for it, and
//! then only when the status changes (V150). lspmux sends a server's
//! notifications to every client attached at the time, and caches the first
//! `initialize`, so every shim asks for it ([`request_status`]) and every
//! shim sees each change while it is attached (V151). A session that joins
//! a server that already finished loading sees no status at all. So a
//! session that has seen none [`GRACE`] after its `initialize` was answered
//! asks the other shims: each shim that last saw the server not quiescent
//! holds a shared lock on a file named after the instance ([`LoadingMark`]).
//! If any does, the server is still loading and the session waits for the
//! next status; otherwise it is ready.
//!
//! The bound ([`DEFAULT_TIMEOUT`], `--ready-timeout`,
//! `CODETAGS_LSP_READY_TIMEOUT`) limits how long a request waits, from when
//! the oldest held request arrived. When it expires the held requests are
//! forwarded anyway, and the shim says so on stderr; the gate then stays open
//! until the server is next quiescent. A server that later reports itself
//! busy again (a reload after a change on disk) closes the gate again.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The notification rust-analyzer sends when its status changes.
pub const STATUS_METHOD: &str = "experimental/serverStatus";
/// The servers known to send [`STATUS_METHOD`] when asked, by the name in
/// their `initialize` answer's `serverInfo`. The gate holds nothing for
/// any other server.
pub const STATUS_SERVERS: [&str; 1] = ["rust-analyzer"];
/// The bound when none is given: long enough for this repository's first
/// load under contention (V141; 23 s uncontended, V152).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
/// The variable that sets the bound, in seconds; `0` turns the gate off.
pub const TIMEOUT_ENV: &str = "CODETAGS_LSP_READY_TIMEOUT";
/// How long a session waits for a first status before asking the other
/// shims. rust-analyzer sends its first status as soon as its main loop
/// starts, right after `initialized` (V150).
pub const GRACE: Duration = Duration::from_secs(2);

/// The bound for a session: `explicit` (`--ready-timeout`), else
/// [`TIMEOUT_ENV`], else [`DEFAULT_TIMEOUT`].
pub fn timeout(explicit: Option<u64>) -> Result<Duration, String> {
    if let Some(seconds) = explicit {
        return Ok(Duration::from_secs(seconds));
    }
    match std::env::var(TIMEOUT_ENV) {
        Ok(text) if !text.trim().is_empty() => text
            .trim()
            .parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|_| format!("{TIMEOUT_ENV}={text:?}: expected a whole number of seconds")),
        _ => Ok(DEFAULT_TIMEOUT),
    }
}

/// Adds `capabilities.experimental.serverStatusNotification: true` to an
/// `initialize` request. Returns whether the client had asked for it itself,
/// so its shim knows whether to pass the notifications on.
pub fn request_status(initialize: &mut Value) -> bool {
    let Some(params) = initialize.get_mut("params").and_then(Value::as_object_mut) else {
        return false;
    };
    let capabilities = params.entry("capabilities").or_insert_with(|| json!({}));
    if !capabilities.is_object() {
        *capabilities = json!({});
    }
    let experimental = &mut capabilities["experimental"];
    if !experimental.is_object() {
        *experimental = json!({});
    }
    let asked = experimental["serverStatusNotification"] == Value::Bool(true);
    experimental["serverStatusNotification"] = Value::Bool(true);
    asked
}

/// The `quiescent` flag of a [`STATUS_METHOD`] notification, if `message`
/// is one.
pub fn quiescent(message: &Value) -> Option<bool> {
    if message.get("method").and_then(Value::as_str) != Some(STATUS_METHOD)
        || message.get("id").is_some()
    {
        return None;
    }
    message
        .pointer("/params/quiescent")
        .and_then(Value::as_bool)
}

/// Whether the server named in an `initialize` answer sends
/// [`STATUS_METHOD`] when asked.
pub fn reports_status(answer: &Value) -> bool {
    answer
        .pointer("/result/serverInfo/name")
        .and_then(Value::as_str)
        .is_some_and(|name| STATUS_SERVERS.contains(&name))
}

/// Where the gate stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The server has not answered `initialize` yet (since this instant):
    /// closed, with no bound, since nothing can be answered before then.
    Starting(Instant),
    /// The server does not report its status, or the gate is turned off.
    Off,
    /// No status seen since the `initialize` answer at this instant.
    Unknown(Instant),
    /// Not quiescent; closed since this instant. `observed` when this shim
    /// saw the status itself, rather than inferring it from the others.
    Loading {
        /// When the gate closed.
        since: Instant,
        /// Whether this shim saw a status saying so.
        observed: bool,
    },
    /// Quiescent.
    Ready,
    /// The bound expired; open until the server is next quiescent.
    TimedOut,
}

/// What changed when the gate opened.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Opened {
    /// The held frames, in order, to forward now.
    pub frames: Vec<Vec<u8>>,
    /// A line for stderr, when something was held.
    pub note: Option<String>,
}

/// The gate's state for one session.
#[derive(Debug)]
pub struct Gate {
    /// Whether this session's requests are held (agent role, a non-zero
    /// bound).
    holds: bool,
    timeout: Duration,
    grace: Duration,
    phase: Phase,
    /// Frames held while closed, in order.
    held: Vec<Vec<u8>>,
    /// When the first frame now held arrived.
    held_since: Option<Instant>,
    mark: Option<LoadingMark>,
}

impl Gate {
    /// A gate for a session starting at `now`: holding requests if `holds`,
    /// with the bound `timeout` (zero: never holds) and the grace period
    /// `grace`, and sharing what it sees through `mark`.
    pub fn new(
        holds: bool,
        timeout: Duration,
        grace: Duration,
        mark: Option<LoadingMark>,
        now: Instant,
    ) -> Self {
        Self {
            holds: holds && !timeout.is_zero(),
            timeout,
            grace,
            phase: Phase::Starting(now),
            held: Vec::new(),
            held_since: None,
            mark,
        }
    }

    /// The current phase.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// How many requests are held.
    pub fn holding(&self) -> usize {
        self.held.len()
    }

    /// The bound.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Whether requests pass now.
    pub fn is_open(&self) -> bool {
        !self.holds || matches!(self.phase, Phase::Off | Phase::Ready | Phase::TimedOut)
    }

    /// Holds a request's `frame` if the gate is closed; otherwise gives it
    /// back to be forwarded.
    pub fn pass(&mut self, frame: Vec<u8>, now: Instant) -> Option<Vec<u8>> {
        if self.is_open() {
            return Some(frame);
        }
        self.held_since.get_or_insert(now);
        self.held.push(frame);
        None
    }

    /// Releases everything held, whatever the phase (before `shutdown`).
    pub fn release(&mut self) -> Vec<Vec<u8>> {
        self.held_since = None;
        std::mem::take(&mut self.held)
    }

    /// The server answered `initialize`; `reports_status` says whether it
    /// will send [`STATUS_METHOD`].
    pub fn initialized(&mut self, reports_status: bool, now: Instant) -> Opened {
        if !matches!(self.phase, Phase::Starting(_)) {
            return Opened::default();
        }
        if reports_status {
            self.phase = Phase::Unknown(now);
            Opened::default()
        } else {
            self.phase = Phase::Off;
            self.open(now, None)
        }
    }

    /// A [`STATUS_METHOD`] notification said `quiescent`.
    pub fn status(&mut self, quiescent: bool, now: Instant) -> Opened {
        if quiescent {
            self.set_mark(false);
            let was_open = self.is_open();
            self.phase = Phase::Ready;
            if was_open {
                return Opened::default();
            }
            return self.open(now, Some("the server is ready"));
        }
        let since = match self.phase {
            Phase::TimedOut => return Opened::default(),
            Phase::Unknown(since) | Phase::Loading { since, .. } => since,
            Phase::Starting(_) | Phase::Off | Phase::Ready => now,
        };
        self.phase = Phase::Loading {
            since,
            observed: true,
        };
        self.set_mark(true);
        Opened::default()
    }

    /// Advances time to `now`: the grace period and the bound.
    pub fn tick(&mut self, now: Instant) -> Opened {
        // The bound limits how long a request waits: it runs from when the
        // oldest held request arrived. Before the server has answered
        // `initialize` nothing could be answered anyway, however long that
        // takes (a daemon starting on Windows), so it does not run then.
        let waited_too_long = self
            .held_since
            .is_some_and(|since| now.saturating_duration_since(since) >= self.timeout);
        match self.phase {
            Phase::Loading { .. } | Phase::Unknown(_) if waited_too_long => self.time_out(now),
            Phase::Unknown(since) if now.saturating_duration_since(since) >= self.grace => {
                let others = self.mark.as_ref().is_some_and(LoadingMark::others_loading);
                if others {
                    self.phase = Phase::Loading {
                        since,
                        observed: false,
                    };
                    Opened::default()
                } else {
                    self.phase = Phase::Ready;
                    self.open(now, Some("no session saw the server loading"))
                }
            }
            _ => Opened::default(),
        }
    }

    fn time_out(&mut self, now: Instant) -> Opened {
        self.set_mark(false);
        self.phase = Phase::TimedOut;
        let reason = format!(
            "the server was not ready within the bound ({}s; --ready-timeout or {TIMEOUT_ENV})",
            self.timeout.as_secs()
        );
        self.open(now, Some(&reason))
    }

    fn open(&mut self, now: Instant, reason: Option<&str>) -> Opened {
        let frames = std::mem::take(&mut self.held);
        let note = match (self.held_since.take(), reason) {
            (Some(since), Some(reason)) if !frames.is_empty() => Some(format!(
                "readiness gate: {reason}; forwarding {} request(s) held for {:.1}s",
                frames.len(),
                now.saturating_duration_since(since).as_secs_f64()
            )),
            _ => None,
        };
        Opened { frames, note }
    }

    fn set_mark(&mut self, loading: bool) {
        if let Some(mark) = &mut self.mark {
            mark.set(loading);
        }
    }
}

/// The shared sign that some attached shim last saw an instance not
/// quiescent: a shared lock on `ready-<key hash>.lock` in the daemon's state
/// directory, held while it is so. The operating system drops it when the
/// shim ends.
#[derive(Debug)]
pub struct LoadingMark {
    path: PathBuf,
    held: Option<File>,
}

impl LoadingMark {
    /// The mark for the instance with `key` (server, arguments, passed
    /// variables and root, as text), in `dir`.
    pub fn new(dir: &Path, key: &str) -> Self {
        Self {
            path: dir.join(format!("ready-{:016x}.lock", fnv1a(key.as_bytes()))),
            held: None,
        }
    }

    /// Takes or drops this shim's shared lock.
    fn set(&mut self, loading: bool) {
        if !loading {
            self.held = None;
            return;
        }
        if self.held.is_some() {
            return;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&self.path);
        if let Ok(file) = file
            && file.lock_shared().is_ok()
        {
            self.held = Some(file);
        }
    }

    /// Whether another shim holds the lock: an exclusive lock cannot be
    /// taken. Called only while this shim holds none.
    fn others_loading(&self) -> bool {
        let Ok(file) = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&self.path)
        else {
            return false;
        };
        match file.try_lock() {
            Ok(()) => {
                let _ = file.unlock();
                false
            }
            Err(std::fs::TryLockError::WouldBlock) => true,
            Err(std::fs::TryLockError::Error(_)) => false,
        }
    }
}

/// FNV-1a, 64-bit: a hash that is the same in every build, for file names.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const BOUND: Duration = Duration::from_secs(10);

    fn gate(holds: bool) -> (Gate, Instant) {
        let now = Instant::now();
        (Gate::new(holds, BOUND, GRACE, None, now), now)
    }

    fn secs(start: Instant, seconds: u64) -> Instant {
        start + Duration::from_secs(seconds)
    }

    #[test]
    fn the_capability_is_added_and_the_clients_own_choice_reported() {
        let mut message = json!({"method": "initialize", "params": {"capabilities": {}}});
        assert!(!request_status(&mut message));
        assert_eq!(
            message["params"]["capabilities"]["experimental"]["serverStatusNotification"],
            json!(true)
        );
        let mut asked = json!({"method": "initialize", "params": {"capabilities":
            {"experimental": {"serverStatusNotification": true, "other": 1}}}});
        assert!(request_status(&mut asked));
        assert_eq!(
            asked["params"]["capabilities"]["experimental"]["other"],
            json!(1)
        );
        let mut bare = json!({"method": "initialize", "params": {}});
        request_status(&mut bare);
        assert_eq!(
            bare["params"]["capabilities"]["experimental"]["serverStatusNotification"],
            json!(true)
        );
    }

    #[test]
    fn status_notifications_and_server_names_are_recognized() {
        let status = json!({"jsonrpc": "2.0", "method": STATUS_METHOD,
            "params": {"health": "ok", "quiescent": false}});
        assert_eq!(quiescent(&status), Some(false));
        assert_eq!(
            quiescent(&json!({"method": "window/logMessage", "params": {}})),
            None
        );
        assert!(reports_status(
            &json!({"id": 1, "result": {"serverInfo": {"name": "rust-analyzer"}}})
        ));
        assert!(!reports_status(
            &json!({"id": 1, "result": {"serverInfo": {"name": "gopls"}}})
        ));
        assert!(!reports_status(&json!({"id": 1, "result": {}})));
    }

    #[test]
    fn requests_wait_for_quiescence() {
        let (mut gate, start) = gate(true);
        assert_eq!(gate.pass(b"early".to_vec(), start), None);
        assert!(gate.initialized(true, start).frames.is_empty());
        gate.status(false, secs(start, 1));
        assert_eq!(gate.pass(b"a".to_vec(), secs(start, 1)), None);
        assert!(gate.tick(secs(start, 5)).frames.is_empty());
        let opened = gate.status(true, secs(start, 6));
        assert_eq!(opened.frames, vec![b"early".to_vec(), b"a".to_vec()]);
        assert!(opened.note.unwrap().contains("ready"));
        assert_eq!(
            gate.pass(b"b".to_vec(), secs(start, 7)),
            Some(b"b".to_vec())
        );
    }

    #[test]
    fn the_bound_forwards_held_requests_anyway() {
        let (mut gate, start) = gate(true);
        gate.initialized(true, start);
        gate.status(false, start);
        gate.pass(b"a".to_vec(), start);
        assert!(gate.tick(secs(start, 9)).frames.is_empty());
        let opened = gate.tick(secs(start, 10));
        assert_eq!(opened.frames, vec![b"a".to_vec()]);
        assert!(opened.note.unwrap().contains("within the bound"));
        assert_eq!(gate.phase(), Phase::TimedOut);
        // Still loading: stays open until the next quiescence.
        gate.status(false, secs(start, 11));
        assert!(gate.is_open());
        gate.status(true, secs(start, 12));
        gate.status(false, secs(start, 13));
        assert!(!gate.is_open());
    }

    #[test]
    fn a_late_joiner_with_nobody_loading_opens_after_the_grace_period() {
        let (mut gate, start) = gate(true);
        gate.initialized(true, start);
        gate.pass(b"a".to_vec(), start);
        assert!(gate.tick(start + GRACE / 2).frames.is_empty());
        let opened = gate.tick(start + GRACE);
        assert_eq!(opened.frames, vec![b"a".to_vec()]);
        assert_eq!(gate.phase(), Phase::Ready);
    }

    #[test]
    fn servers_without_a_status_and_editors_are_never_held() {
        let (mut gate, start) = gate(true);
        gate.pass(b"early".to_vec(), start);
        assert_eq!(
            gate.initialized(false, start).frames,
            vec![b"early".to_vec()]
        );
        assert!(gate.is_open());
        let (mut editor, start) = gate_editor();
        editor.initialized(true, start);
        editor.status(false, start);
        assert_eq!(editor.pass(b"a".to_vec(), start), Some(b"a".to_vec()));
    }

    fn gate_editor() -> (Gate, Instant) {
        gate(false)
    }

    #[test]
    fn a_zero_bound_turns_the_gate_off() {
        let now = Instant::now();
        let mut gate = Gate::new(true, Duration::ZERO, GRACE, None, now);
        assert_eq!(gate.pass(b"a".to_vec(), now), Some(b"a".to_vec()));
    }

    #[test]
    fn a_reload_closes_the_gate_again() {
        let (mut gate, start) = gate(true);
        gate.initialized(true, start);
        gate.status(true, start);
        assert!(gate.is_open());
        gate.status(false, secs(start, 1));
        assert!(!gate.is_open());
        gate.pass(b"a".to_vec(), secs(start, 1));
        assert!(gate.tick(secs(start, 10)).frames.is_empty());
        assert_eq!(gate.tick(secs(start, 11)).frames, vec![b"a".to_vec()]);
    }

    #[test]
    fn the_mark_tells_a_late_joiner_that_another_shim_saw_loading() {
        let dir = tempfile::tempdir().unwrap();
        let start = Instant::now();
        let mut first = Gate::new(
            false,
            BOUND,
            GRACE,
            Some(LoadingMark::new(dir.path(), "key")),
            start,
        );
        first.initialized(true, start);
        first.status(false, start);
        let mut late = Gate::new(
            true,
            BOUND,
            GRACE,
            Some(LoadingMark::new(dir.path(), "key")),
            start,
        );
        late.initialized(true, start);
        late.pass(b"a".to_vec(), start);
        assert!(late.tick(start + GRACE).frames.is_empty());
        assert_eq!(
            late.phase(),
            Phase::Loading {
                since: start,
                observed: false
            }
        );
        // The first shim sees quiescence and drops its lock; so does the
        // late joiner, which is attached by now.
        first.status(true, secs(start, 3));
        assert_eq!(
            late.status(true, secs(start, 3)).frames,
            vec![b"a".to_vec()]
        );
        // Another key is another instance.
        let mut other = Gate::new(
            true,
            BOUND,
            GRACE,
            Some(LoadingMark::new(dir.path(), "other")),
            start,
        );
        other.initialized(true, start);
        other.tick(start + GRACE);
        assert_eq!(other.phase(), Phase::Ready);
    }

    #[test]
    fn fnv1a_is_stable() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
