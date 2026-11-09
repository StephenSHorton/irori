//! How long callers wait for and hold a hot lock (card #1234): each board
//! log's (coord's `Coord`) and the inbox's.
//!
//! Every such lock is timed: the wait before it is taken and
//! the hold until it drops (its flush and unlock hook included). Totals go
//! to two places:
//!
//! - per caller (the `file:line` that took it), summed until
//!   [`take_summary`] reads and clears them;
//! - per thread, so a command can say how much of its own time was lock
//!   wait ([`thread_mark`], [`thread_since`]).
//!
//! A single wait or hold of [`SLOW`] or more goes to the log at once, with
//! its caller and thread. With no logger set ([`set_logger`]) nothing is
//! written. The per-thread totals cost two clock reads per lock; the
//! per-caller totals take a global mutex per lock, so they are kept only
//! once [`keep_totals`] turns them on (the dev probe does, for a scratch
//! run's per-minute line).

use std::cell::Cell;
use std::collections::HashMap;
use std::panic::Location;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

/// A wait or hold this long is logged on its own.
pub const SLOW: Duration = Duration::from_millis(1000);

static TOTALS: AtomicBool = AtomicBool::new(false);

/// Keeps per-caller totals for [`summary_line`] from now on.
pub fn keep_totals() {
    TOTALS.store(true, Ordering::Relaxed);
}

/// Whether per-caller totals are kept ([`keep_totals`]).
pub fn totals_kept() -> bool {
    TOTALS.load(Ordering::Relaxed)
}

type LogFn = Box<dyn Fn(&str) + Send + Sync>;

static LOGGER: RwLock<Option<LogFn>> = RwLock::new(None);

/// Where slow waits and holds are written (the Core sends them to
/// irori.log).
pub fn set_logger(f: impl Fn(&str) + Send + Sync + 'static) {
    *LOGGER.write().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
}

/// Writes a line through the logger [`set_logger`] set (none: nowhere).
pub fn log(line: &str) {
    if let Some(f) = LOGGER.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        f(line);
    }
}

/// One caller's totals.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stat {
    pub takes: u64,
    pub wait: Duration,
    pub wait_max: Duration,
    pub hold: Duration,
    pub hold_max: Duration,
}

static STATS: Mutex<Option<HashMap<&'static Location<'static>, Stat>>> = Mutex::new(None);

thread_local! {
    static THREAD: Cell<(u64, Duration, Duration)> = const { Cell::new((0, Duration::ZERO, Duration::ZERO)) };
}

/// This thread's lock totals so far: takes, wait, hold.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mark {
    pub takes: u64,
    pub wait: Duration,
    pub hold: Duration,
}

/// Where this thread's totals stand; pass it to [`thread_since`].
pub fn thread_mark() -> Mark {
    let (takes, wait, hold) = THREAD.with(Cell::get);
    Mark { takes, wait, hold }
}

/// This thread's takes, wait and hold since `m`.
pub fn thread_since(m: Mark) -> Mark {
    let now = thread_mark();
    Mark {
        takes: now.takes - m.takes,
        wait: now.wait.saturating_sub(m.wait),
        hold: now.hold.saturating_sub(m.hold),
    }
}

fn thread_name() -> String {
    let t = std::thread::current();
    match t.name() {
        Some(n) => n.to_string(),
        None => format!("{:?}", t.id()),
    }
}

fn short(at: &Location<'_>) -> String {
    let f = at.file();
    let f = f.rsplit_once("crates/").map_or(f, |(_, r)| r);
    let f = f.replacen("/src/", "/", 1);
    format!("{f}:{}", at.line())
}

/// A lock was taken at `at` after waiting `wait`.
pub fn waited(at: &'static Location<'static>, wait: Duration) {
    THREAD.with(|c| {
        let (n, w, h) = c.get();
        c.set((n + 1, w + wait, h));
    });
    if wait >= SLOW {
        log(&format!(
            "lock: waited {} ms at {} on {}",
            wait.as_millis(),
            short(at),
            thread_name()
        ));
    }
    if !totals_kept() {
        return;
    }
    let mut s = STATS.lock().unwrap_or_else(|e| e.into_inner());
    let e = s.get_or_insert_with(HashMap::new).entry(at).or_default();
    e.takes += 1;
    e.wait += wait;
    e.wait_max = e.wait_max.max(wait);
}

/// The lock taken at `at` was held `hold`.
pub fn held(at: &'static Location<'static>, hold: Duration) {
    THREAD.with(|c| {
        let (n, w, h) = c.get();
        c.set((n, w, h + hold));
    });
    if hold >= SLOW {
        log(&format!(
            "lock: held {} ms at {} on {}",
            hold.as_millis(),
            short(at),
            thread_name()
        ));
    }
    if !totals_kept() {
        return;
    }
    let mut s = STATS.lock().unwrap_or_else(|e| e.into_inner());
    let e = s.get_or_insert_with(HashMap::new).entry(at).or_default();
    e.hold += hold;
    e.hold_max = e.hold_max.max(hold);
}

/// Times one take of a lock: [`Taken::new`] after the wait, dropped after
/// the hold.
pub struct Taken {
    at: &'static Location<'static>,
    since: Instant,
}

impl Taken {
    pub fn new(at: &'static Location<'static>, asked: Instant) -> Taken {
        let since = Instant::now();
        waited(at, since - asked);
        Taken { at, since }
    }
}

impl Drop for Taken {
    fn drop(&mut self) {
        held(self.at, self.since.elapsed());
    }
}

/// Every caller's totals since the last call, most held first, and clears
/// them.
pub fn take_summary() -> Vec<(String, Stat)> {
    let m = STATS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .unwrap_or_default();
    let mut v: Vec<(String, Stat)> = m.into_iter().map(|(k, s)| (short(k), s)).collect();
    v.sort_by_key(|(_, s)| std::cmp::Reverse(s.hold + s.wait));
    v
}

/// One line for the log: the top `n` callers by hold plus wait.
pub fn summary_line(n: usize) -> Option<String> {
    let v = take_summary();
    if v.is_empty() {
        return None;
    }
    let (takes, wait, hold) = v
        .iter()
        .fold((0, Duration::ZERO, Duration::ZERO), |a, (_, s)| {
            (a.0 + s.takes, a.1 + s.wait, a.2 + s.hold)
        });
    let top: Vec<String> = v
        .iter()
        .take(n)
        .map(|(at, s)| {
            format!(
                "{at} {}x hold {}/{} wait {}/{}",
                s.takes,
                s.hold.as_millis(),
                s.hold_max.as_millis(),
                s.wait.as_millis(),
                s.wait_max.as_millis()
            )
        })
        .collect();
    Some(format!(
        "locks: {takes} takes, hold {} ms, wait {} ms; top (sum/max ms): {}",
        hold.as_millis(),
        wait.as_millis(),
        top.join(", ")
    ))
}
