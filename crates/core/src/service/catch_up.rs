//! Loading the person's history quietly (card #1094). A device that holds
//! little of it, freshly Reset or newly paired, gets all of it from the
//! first of the person's devices it reaches. Every old blocker, done check
//! and message in it reaches the folds as a live change, and each would
//! show a notification. While such a load runs this device is *loading*:
//! notify's Watch seeds what arrives without cards, and the shared-board
//! and Talk cards hold off. Only what happens after it has caught up is
//! news.
//!
//! A load starts at the hello of a session with an own device, before any
//! of its rows, when this device holds less than half of the rows that
//! hello names ([`Node::catch_up_target`](mesh::Node::catch_up_target)).
//! A device that was merely offline holds nearly all of them, so it loads
//! nothing quietly. The load ends in two steps:
//!
//! 1. Loading: until the store holds every row the hello named. If no row
//!    arrives for [`Times::stall`] it goes on to settling all the same: a
//!    stuck writer must not keep the device quiet.
//! 2. Settling: the store has the rows, but the folds may still be reading
//!    them. It ends once no board, inbox or ask change has come for
//!    [`Times::idle`], or after [`Times::cap`] on a busy mesh.
//!
//! When the session drops, the load stops: what came is seeded, and the
//! next hello decides again on what is still missing.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use store::{Store, Vector};

/// How a load is timed. [`Times::default`] is the app's.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Times {
    /// How often a running load is looked at.
    pub tick: Duration,
    /// Settling ends once no change has come for this long.
    pub idle: Duration,
    /// Settling ends after this long, changes or not.
    pub cap: Duration,
    /// Loading goes on to settling once no row has arrived for this long.
    pub stall: Duration,
}

impl Default for Times {
    fn default() -> Times {
        Times {
            tick: Duration::from_millis(250),
            idle: Duration::from_secs(2),
            cap: Duration::from_secs(30),
            stall: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Phase {
    Idle,
    /// Until the store holds `target`.
    Loading {
        target: Vector,
        /// The devices whose sessions are bringing it.
        peers: BTreeSet<String>,
        /// Rows of `target` held at the last look, and when that grew.
        held: i64,
        grew: Instant,
        since: Instant,
    },
    /// Until the folds have read what came.
    Settling { since: Instant, started: Instant },
}

#[derive(Debug)]
struct Inner {
    phase: Phase,
    /// When the last board, inbox or ask change came.
    changed: Option<Instant>,
}

/// Whether this device is loading the person's history, and the load's
/// steps. Shared by the mesh's hello hook, the change feeds, the tick and
/// notify's Watch.
pub(crate) struct CatchUp {
    store: Arc<Store>,
    times: Times,
    g: Mutex<Inner>,
}

/// Rows of `target` that `ours` holds, and how many it names.
fn held_of(ours: &Vector, target: &Vector) -> (i64, i64) {
    target.iter().fold((0, 0), |(held, want), (w, seq)| {
        let have = ours.get(w).copied().unwrap_or(0).clamp(0, *seq);
        (held + have, want + seq)
    })
}

impl CatchUp {
    pub(crate) fn new(store: Arc<Store>, times: Times) -> CatchUp {
        CatchUp {
            store,
            times,
            g: Mutex::new(Inner {
                phase: Phase::Idle,
                changed: None,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.g.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn times(&self) -> Times {
        self.times
    }

    /// Whether this device is loading the person's history now: its cards
    /// hold off.
    pub(crate) fn loading(&self) -> bool {
        self.lock().phase != Phase::Idle
    }

    /// A board, inbox or ask change came: settling waits for them to stop.
    pub(crate) fn changed(&self) {
        self.lock().changed = Some(Instant::now());
    }

    /// A session with own device `peer` came up, its hello naming `target`
    /// (before any of its rows). Starts a load when this device holds less
    /// than half of it; a load already running takes it on too. The log
    /// line, if any.
    pub(crate) fn hello(&self, peer: &str, target: &Vector) -> Option<String> {
        let ours = self.store.vector().ok()?;
        self.hello_at(peer, target, &ours, Instant::now())
    }

    fn hello_at(
        &self,
        peer: &str,
        target: &Vector,
        ours: &Vector,
        now: Instant,
    ) -> Option<String> {
        let (held, want) = held_of(ours, target);
        let mut g = self.lock();
        match &mut g.phase {
            Phase::Loading {
                target: t, peers, ..
            } => {
                for (w, seq) in target {
                    let e = t.entry(w.clone()).or_insert(0);
                    *e = (*e).max(*seq);
                }
                peers.insert(peer.into());
                None
            }
            // Its rows are folding now: the new session's are few, or it
            // would have counted them at its own hello.
            Phase::Settling { .. } => None,
            Phase::Idle => {
                if held >= want - held || want == 0 {
                    return None;
                }
                g.phase = Phase::Loading {
                    target: target.clone(),
                    peers: BTreeSet::from([peer.to_string()]),
                    held,
                    grew: now,
                    since: now,
                };
                g.changed = None;
                Some(format!(
                    "catch-up: loading {} rows of history from {peer} without notifications (this device holds {held})",
                    want - held
                ))
            }
        }
    }

    /// One look at a running load; `up` says whether a session with a
    /// device is up. The log line when a step ends.
    pub(crate) fn tick(&self, up: impl Fn(&str) -> bool) -> Option<String> {
        if !self.loading() {
            return None;
        }
        let ours = self.store.vector().ok()?;
        self.tick_at(&ours, up, Instant::now())
    }

    fn tick_at(&self, ours: &Vector, up: impl Fn(&str) -> bool, now: Instant) -> Option<String> {
        let t = self.times;
        let mut g = self.lock();
        let changed = g.changed;
        let (next, line) = match &mut g.phase {
            Phase::Idle => return None,
            Phase::Loading {
                target,
                peers,
                held,
                grew,
                since,
            } => {
                let (now_held, want) = held_of(ours, target);
                if now_held > *held {
                    *held = now_held;
                    *grew = now;
                }
                if !peers.iter().any(|p| up(p)) {
                    (
                        Phase::Idle,
                        format!(
                            "catch-up: the session dropped with {} of {want} rows here; what came is seeded quietly",
                            now_held
                        ),
                    )
                } else if now_held >= want {
                    (
                        Phase::Settling {
                            since: now,
                            started: *since,
                        },
                        String::new(),
                    )
                } else if now.duration_since(*grew) >= t.stall {
                    (
                        Phase::Settling {
                            since: now,
                            started: *since,
                        },
                        format!(
                            "catch-up: no rows for {} s with {now_held} of {want} here; going on",
                            t.stall.as_secs()
                        ),
                    )
                } else {
                    return None;
                }
            }
            Phase::Settling { since, started } => {
                let quiet_for = changed.map_or(Duration::MAX, |c| now.saturating_duration_since(c));
                let settled = now.duration_since(*since) >= t.idle && quiet_for >= t.idle;
                if !settled && now.duration_since(*since) < t.cap {
                    return None;
                }
                (
                    Phase::Idle,
                    format!(
                        "catch-up: history loaded in {:.1} s; notifications are on",
                        now.duration_since(*started).as_secs_f64()
                    ),
                )
            }
        };
        g.phase = next;
        (!line.is_empty()).then_some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vec_of(rows: &[(&str, i64)]) -> Vector {
        rows.iter().map(|(d, s)| (d.to_string(), *s)).collect()
    }

    fn times() -> Times {
        Times {
            tick: Duration::from_millis(10),
            idle: Duration::from_secs(2),
            cap: Duration::from_secs(30),
            stall: Duration::from_secs(60),
        }
    }

    fn catch_up() -> CatchUp {
        // The store is read only by hello and tick, which these tests skip.
        let dir = tempfile::tempdir().unwrap();
        let st = Store::open(dir.path().join("t.db"), "me", Arc::new(store::Clock::new())).unwrap();
        std::mem::forget(dir);
        CatchUp::new(Arc::new(st), times())
    }

    fn secs(t0: Instant, s: u64) -> Instant {
        t0 + Duration::from_secs(s)
    }

    #[test]
    fn a_device_holding_little_loads_quietly_until_settled() {
        let c = catch_up();
        let t0 = Instant::now();
        let target = vec_of(&[("pc", 1000), ("old", 3000)]);
        assert!(c.hello_at("pc", &target, &Vector::new(), t0).is_some());
        assert!(c.loading());
        // Rows still coming.
        c.tick_at(&vec_of(&[("pc", 1000), ("old", 10)]), |_| true, secs(t0, 1));
        assert!(c.loading());
        // All here: settling, still quiet while the folds read.
        c.tick_at(&target, |_| true, secs(t0, 2));
        assert!(c.loading());
        c.changed();
        c.tick_at(&target, |_| true, secs(t0, 3));
        assert!(c.loading(), "settling waits {:?} of no change", times().idle);
        // No change since: done.
        let line = c.tick_at(&target, |_| true, Instant::now() + Duration::from_secs(5));
        assert!(line.is_some_and(|l| l.contains("history loaded")));
        assert!(!c.loading());
    }

    #[test]
    fn a_device_holding_most_of_it_loads_nothing_quietly() {
        let c = catch_up();
        let t0 = Instant::now();
        let target = vec_of(&[("pc", 1000), ("old", 3000)]);
        // Offline for a while: most of it is here.
        let ours = vec_of(&[("pc", 900), ("old", 3000)]);
        assert!(c.hello_at("pc", &target, &ours, t0).is_none());
        assert!(!c.loading());
        // Nothing missing at all.
        assert!(c.hello_at("pc", &target, &target, t0).is_none());
        assert!(c.hello_at("pc", &Vector::new(), &Vector::new(), t0).is_none());
        assert!(!c.loading());
    }

    #[test]
    fn a_dropped_session_ends_the_load() {
        let c = catch_up();
        let t0 = Instant::now();
        let target = vec_of(&[("pc", 1000)]);
        c.hello_at("pc", &target, &Vector::new(), t0);
        let line = c.tick_at(&vec_of(&[("pc", 10)]), |_| false, secs(t0, 1));
        assert!(line.is_some_and(|l| l.contains("dropped")));
        assert!(!c.loading());
        // The next hello decides again on what is missing.
        assert!(c.hello_at("pc", &target, &vec_of(&[("pc", 10)]), secs(t0, 2)).is_some());
        assert!(c.loading());
    }

    #[test]
    fn a_stalled_load_and_a_busy_mesh_still_end() {
        let c = catch_up();
        let t0 = Instant::now();
        let target = vec_of(&[("pc", 1000)]);
        c.hello_at("pc", &target, &Vector::new(), t0);
        let part = vec_of(&[("pc", 400)]);
        c.tick_at(&part, |_| true, secs(t0, 10));
        // Rows arrived at 10 s: no stall yet at 60 s.
        c.tick_at(&part, |_| true, secs(t0, 60));
        assert!(c.loading());
        let line = c.tick_at(&part, |_| true, secs(t0, 71));
        assert!(line.is_some_and(|l| l.contains("no rows")));
        assert!(c.loading(), "settling next");
        // Changes keep coming: the cap ends it.
        let settled_at = secs(t0, 71);
        for s in [80, 90, 100] {
            c.lock().changed = Some(secs(t0, s));
            c.tick_at(&part, |_| true, secs(t0, s));
            assert!(c.loading());
        }
        c.lock().changed = Some(settled_at + Duration::from_secs(30));
        c.tick_at(&part, |_| true, settled_at + Duration::from_secs(30));
        assert!(!c.loading());
    }

    #[test]
    fn a_second_device_joins_a_running_load() {
        let c = catch_up();
        let t0 = Instant::now();
        c.hello_at("pc", &vec_of(&[("pc", 100)]), &Vector::new(), t0);
        c.hello_at("phone", &vec_of(&[("phone", 50), ("pc", 120)]), &Vector::new(), t0);
        // The pc's session drops; the phone's still brings rows.
        c.tick_at(&vec_of(&[("pc", 100)]), |p| p == "phone", secs(t0, 1));
        assert!(c.loading());
        c.tick_at(&vec_of(&[("pc", 120), ("phone", 50)]), |p| p == "phone", secs(t0, 2));
        assert!(c.loading(), "settling");
    }
}
