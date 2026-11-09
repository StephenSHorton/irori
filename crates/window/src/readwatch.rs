//! Where a window read's time goes on the Core side (card #1234): how long
//! it queued for a thread, how long it waited for a board log's lock, and
//! the rest (its own work and any other lock).
//!
//! A read of [`SLOW`] (3 s) or more is logged on its own as `read: <command>`,
//! with its lock wait (a board log's and the inbox's together; the lock
//! lines say which). On a scratch run (lockwatch totals kept) each
//! command's totals are summed until [`summary_line`] reads them, which the
//! window's per-minute lists line does, so the two sit side by side in
//! irori.log.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use gocompat::lockwatch;

/// A read this slow is logged on its own (the lists line counts those
/// over 1 s already; this one says where the time went).
pub const SLOW: Duration = Duration::from_millis(3000);

#[derive(Default, Clone, Copy)]
struct Stat {
    n: u64,
    total: Duration,
    max: Duration,
    queue: Duration,
    wait: Duration,
}

static STATS: Mutex<BTreeMap<&'static str, Stat>> = Mutex::new(BTreeMap::new());

/// One read's timing.
pub(crate) struct Timing {
    name: &'static str,
    asked: Instant,
    queue: Duration,
    lock: lockwatch::Mark,
    ran: Duration,
}

/// Starts timing `name`'s run on this thread, `asked` when the command
/// came in (before any queue for a blocking thread).
pub(crate) fn start(name: &'static str, asked: Instant) -> (Timing, Instant, lockwatch::Mark) {
    let now = Instant::now();
    (
        Timing {
            name,
            asked,
            queue: now - asked,
            lock: lockwatch::Mark::default(),
            ran: Duration::ZERO,
        },
        now,
        lockwatch::thread_mark(),
    )
}

impl Timing {
    /// The run that began at `began`, with the thread's lock totals at
    /// `mark`, has ended.
    pub(crate) fn ran(mut self, began: Instant, mark: lockwatch::Mark) -> Timing {
        self.ran = began.elapsed();
        self.lock = lockwatch::thread_since(mark);
        self
    }

    /// The answer is back on the runtime: record and maybe log it.
    pub(crate) fn done(self) {
        let total = self.asked.elapsed();
        let rest = self.ran.saturating_sub(self.lock.wait);
        if total >= SLOW {
            irori_core::log(&format!(
                "read: {} {} ms (queue {} ms, lock wait {} ms over {} takes, held {} ms, rest {} ms, back {} ms)",
                self.name,
                total.as_millis(),
                self.queue.as_millis(),
                self.lock.wait.as_millis(),
                self.lock.takes,
                self.lock.hold.as_millis(),
                rest.as_millis(),
                total.saturating_sub(self.queue + self.ran).as_millis(),
            ));
        }
        if !lockwatch::totals_kept() {
            return;
        }
        let mut s = STATS.lock().unwrap_or_else(|e| e.into_inner());
        let e = s.entry(self.name).or_default();
        e.n += 1;
        e.total += total;
        e.max = e.max.max(total);
        e.queue += self.queue;
        e.wait += self.lock.wait;
    }
}

/// Times `f` run right here (on the runtime thread the command came in on).
pub(crate) fn here<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    let (t, began, mark) = start(name, Instant::now());
    let r = f();
    t.ran(began, mark).done();
    r
}

/// The commands read since the last call, slowest first by max:
/// `name n/max ms (queue sum, lock wait sum)`, and clears them.
pub fn summary_line() -> Option<String> {
    let m = std::mem::take(&mut *STATS.lock().unwrap_or_else(|e| e.into_inner()));
    if m.is_empty() {
        return None;
    }
    let mut v: Vec<(&str, Stat)> = m.into_iter().collect();
    v.sort_by_key(|(_, s)| std::cmp::Reverse(s.max));
    let parts: Vec<String> = v
        .iter()
        .map(|(k, s)| {
            format!(
                "{k} {}x max {} sum {} (queue {}, lock wait {})",
                s.n,
                s.max.as_millis(),
                s.total.as_millis(),
                s.queue.as_millis(),
                s.wait.as_millis()
            )
        })
        .collect();
    Some(format!("reads (ms): {}", parts.join("; ")))
}
