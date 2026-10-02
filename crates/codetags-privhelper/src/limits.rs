//! Resource limits: how much of the host any local user can make the
//! helper spend (threat model §6.5).
//!
//! Every client costs a thread pair, a checker process, a queue and a
//! descriptor per root, all taken from root's budget, which `RLIMIT_NPROC`
//! does not bound. So each is capped: connections in total and per uid,
//! checker processes, subscriptions per connection, and each connection's
//! queue of events. A cap is met with a protocol refusal, never a crash or
//! a wait.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::server::lock;

/// The caps, each settable by a flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Open connections, in total (`--max-connections`).
    pub connections: usize,
    /// Open connections per uid (`--max-connections-per-uid`).
    pub connections_per_uid: usize,
    /// Running checker processes (`--max-checkers`).
    pub checkers: usize,
    /// Roots per connection (`--max-subscriptions`).
    pub subscriptions: usize,
    /// Events queued per connection before it is sent an overflow and
    /// events are dropped (`--queue`).
    pub queue: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            connections: 128,
            connections_per_uid: 16,
            checkers: 64,
            subscriptions: 32,
            queue: 8 * 1024,
        }
    }
}

impl Limits {
    /// Sets the cap a flag names to `value`; `Err` for a flag that is not
    /// a limit's, or a value that is not a positive number.
    pub fn set(&mut self, flag: &str, value: &str) -> Result<(), String> {
        let slot = match flag {
            "--max-connections" => &mut self.connections,
            "--max-connections-per-uid" => &mut self.connections_per_uid,
            "--max-checkers" => &mut self.checkers,
            "--max-subscriptions" => &mut self.subscriptions,
            "--queue" => &mut self.queue,
            _ => return Err(format!("unknown argument {flag:?}")),
        };
        *slot = value
            .parse::<usize>()
            .ok()
            .filter(|&n| n > 0)
            .ok_or_else(|| format!("{flag} needs a positive number, not {value:?}"))?;
        Ok(())
    }
}

/// Counts open connections, in total and per uid.
#[derive(Debug)]
pub struct Admission {
    total: usize,
    per_uid: usize,
    open: Mutex<HashMap<u32, usize>>,
}

/// One admitted connection; dropping it frees its place.
#[derive(Debug)]
pub struct Ticket {
    admission: Arc<Admission>,
    uid: u32,
}

impl Admission {
    /// Admits at most `total` connections, and `per_uid` from any one uid.
    pub fn new(total: usize, per_uid: usize) -> Arc<Self> {
        Arc::new(Self {
            total,
            per_uid,
            open: Mutex::new(HashMap::new()),
        })
    }

    /// Admits a connection from `uid`, or says why not.
    pub fn admit(self: &Arc<Self>, uid: u32) -> Result<Ticket, String> {
        let mut open = lock(&self.open);
        if open.values().sum::<usize>() >= self.total {
            return Err(format!(
                "the helper has its maximum of {} connections open; try again later",
                self.total
            ));
        }
        let mine = open.entry(uid).or_default();
        if *mine >= self.per_uid {
            return Err(format!(
                "uid {uid} has its maximum of {} connections to the helper open",
                self.per_uid
            ));
        }
        *mine += 1;
        Ok(Ticket {
            admission: Arc::clone(self),
            uid,
        })
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        let mut open = lock(&self.admission.open);
        if let Some(count) = open.get_mut(&self.uid) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                open.remove(&self.uid);
            }
        }
    }
}

/// Counts something held, up to a limit: here, checker processes.
#[derive(Debug)]
pub struct Gate {
    limit: usize,
    held: Mutex<usize>,
}

/// One place in a [`Gate`]; dropping it frees the place.
#[derive(Debug)]
pub struct Pass(Arc<Gate>);

impl Gate {
    /// A gate with `limit` places.
    pub fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit,
            held: Mutex::new(0),
        })
    }

    /// Takes a place, or `None` if all are held.
    pub fn enter(self: &Arc<Self>) -> Option<Pass> {
        let mut held = lock(&self.held);
        if *held >= self.limit {
            return None;
        }
        *held += 1;
        Some(Pass(Arc::clone(self)))
    }

    /// Its limit.
    pub fn limit(&self) -> usize {
        self.limit
    }
}

impl Drop for Pass {
    fn drop(&mut self) {
        let mut held = lock(&self.0.held);
        *held = held.saturating_sub(1);
    }
}

/// The roots one connection has subscribed to, up to a limit.
#[derive(Debug)]
pub struct Subscriptions {
    limit: usize,
    roots: HashSet<PathBuf>,
}

impl Subscriptions {
    /// At most `limit` distinct roots.
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            roots: HashSet::new(),
        }
    }

    /// Whether `root` may be added: one already held always may.
    pub fn check(&self, root: &Path) -> Result<(), String> {
        if self.roots.contains(root) || self.roots.len() < self.limit {
            Ok(())
        } else {
            Err(format!(
                "this connection has its maximum of {} subscriptions",
                self.limit
            ))
        }
    }

    /// Records `root` as subscribed.
    pub fn insert(&mut self, root: PathBuf) {
        self.roots.insert(root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connections_are_capped_in_total() {
        let admission = Admission::new(2, 5);
        let a = admission.admit(1000).unwrap();
        let _b = admission.admit(1001).unwrap();
        let refused = admission.admit(1002).unwrap_err();
        assert!(refused.contains("maximum of 2 connections"), "{refused}");
        drop(a);
        admission.admit(1002).unwrap();
    }

    #[test]
    fn connections_are_capped_per_uid() {
        let admission = Admission::new(10, 2);
        let a = admission.admit(1000).unwrap();
        let _b = admission.admit(1000).unwrap();
        let refused = admission.admit(1000).unwrap_err();
        assert!(refused.contains("uid 1000"), "{refused}");
        // Another uid is unaffected.
        let _c = admission.admit(1001).unwrap();
        drop(a);
        admission.admit(1000).unwrap();
    }

    #[test]
    fn checkers_are_capped() {
        let gate = Gate::new(2);
        let a = gate.enter().unwrap();
        let _b = gate.enter().unwrap();
        assert!(gate.enter().is_none());
        drop(a);
        assert!(gate.enter().is_some());
        assert_eq!(gate.limit(), 2);
    }

    #[test]
    fn subscriptions_are_capped_per_connection() {
        let mut subscriptions = Subscriptions::new(2);
        for root in ["/a", "/b"] {
            subscriptions.check(Path::new(root)).unwrap();
            subscriptions.insert(root.into());
        }
        let refused = subscriptions.check(Path::new("/c")).unwrap_err();
        assert!(refused.contains("maximum of 2 subscriptions"), "{refused}");
        // Subscribing again to a root already held costs nothing.
        subscriptions.check(Path::new("/a")).unwrap();
    }

    #[test]
    fn limits_are_set_by_flag_and_must_be_positive() {
        let mut limits = Limits::default();
        limits.set("--max-connections", "3").unwrap();
        limits.set("--max-connections-per-uid", "2").unwrap();
        limits.set("--max-checkers", "4").unwrap();
        limits.set("--max-subscriptions", "5").unwrap();
        limits.set("--queue", "6").unwrap();
        assert_eq!(
            limits,
            Limits {
                connections: 3,
                connections_per_uid: 2,
                checkers: 4,
                subscriptions: 5,
                queue: 6,
            }
        );
        assert!(limits.set("--queue", "0").is_err());
        assert!(limits.set("--queue", "many").is_err());
        assert!(limits.set("--other", "1").is_err());
    }
}
