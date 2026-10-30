//! The clock-driven parts M3 left as plain calls (architecture.md §8 M4).
//!
//! - [`run_expiry`]: coord's expiry timer (Go `armExpiry`/`expire`,
//!   I:internal/coord/subscribe.go:53-93). Leases have no row when they run
//!   out, so this is how subscribers hear of it without a poll. It sleeps
//!   until [`coord::Coord::next_expiry`] (the soonest Until of this
//!   device's leases while anyone is subscribed) and calls
//!   [`coord::Coord::expire`]; every coord unlock signals it to re-read.
//! - [`Kick`]: a coalescing wake-up (Go's `chan struct{}` of 1 with a
//!   non-blocking send).
//! - [`Ticker`]: Go `time.NewTicker` for a loop that also waits on a queue.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use coord::Coord;
use gocompat::time::GoTime;
use sys::cancel::Cancel;

/// A wake-up that coalesces: any number of [`Kick::kick`]s before a wait
/// make it return once.
#[derive(Default)]
pub struct Kick {
    state: Mutex<(u64, bool)>,
    cond: Condvar,
}

impl Kick {
    pub fn new() -> Arc<Kick> {
        Arc::new(Kick::default())
    }

    /// Never blocks.
    pub fn kick(&self) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.0 = s.0.wrapping_add(1);
        self.cond.notify_all();
    }

    /// Ends every wait for good.
    pub fn close(&self) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.1 = true;
        self.cond.notify_all();
    }

    /// The current generation, to wait past.
    pub fn generation(&self) -> u64 {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).0
    }

    /// Waits until a kick after generation `seen`, `timeout` passes
    /// (`None`: no limit) or the kick closes. True when kicked or closed.
    pub fn wait_past(&self, seen: u64, timeout: Option<Duration>) -> bool {
        let deadline = timeout.map(|d| Instant::now() + d);
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if s.1 || s.0 != seen {
                return true;
            }
            match deadline {
                None => s = self.cond.wait(s).unwrap_or_else(|e| e.into_inner()),
                Some(at) => {
                    let now = Instant::now();
                    if now >= at {
                        return false;
                    }
                    s = self
                        .cond
                        .wait_timeout(s, at - now)
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
            }
        }
    }

    /// Waits for the next kick: true when kicked, false once closed.
    pub fn wait(&self, seen: &mut u64) -> bool {
        self.wait_past(*seen, None);
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        *seen = s.0;
        !s.1
    }

    pub fn is_closed(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).1
    }
}

/// Go `time.NewTicker(d)`: fires every `d` from its start, whatever else
/// the loop waited on.
pub struct Ticker {
    every: Duration,
    next: Instant,
}

impl Ticker {
    pub fn new(every: Duration) -> Ticker {
        Ticker {
            every,
            next: Instant::now() + every,
        }
    }

    /// How long until the next tick (zero when due).
    pub fn until(&self) -> Duration {
        self.next.saturating_duration_since(Instant::now())
    }

    /// Takes a due tick: true at most once per period (a late loop skips
    /// the ticks it missed, as Go's ticker drops them).
    pub fn take(&mut self) -> bool {
        let now = Instant::now();
        if now < self.next {
            return false;
        }
        while self.next <= now {
            self.next += self.every;
        }
        true
    }
}

/// Go `a.Sub(b)`, clamped at zero.
fn until(at: &GoTime, now: &GoTime) -> Duration {
    let d = (i128::from(at.secs) - i128::from(now.secs)) * 1_000_000_000 + i128::from(at.nanos)
        - i128::from(now.nanos);
    if d <= 0 {
        Duration::ZERO
    } else {
        Duration::from_nanos(d.min(u64::MAX as i128) as u64)
    }
}

/// Wires `co`'s unlock to `rearm` (do this before [`run_expiry`] starts).
pub fn arm(co: &Coord, rearm: &Arc<Kick>) {
    let k = rearm.clone();
    co.set_on_unlock(move || k.kick());
}

/// The expiry timer, until `stop` is canceled. `now` is the coord's clock
/// (Go `c.stamp()`).
pub fn run_expiry(
    co: &Coord,
    rearm: &Kick,
    now: &(dyn Fn() -> GoTime + Send + Sync),
    stop: &Cancel,
) {
    loop {
        if stop.is_canceled() || rearm.is_closed() {
            return;
        }
        let seen = rearm.generation();
        match co.next_expiry() {
            None => {
                rearm.wait_past(seen, None);
            }
            Some(at) => {
                let d = until(&at, &now());
                if !d.is_zero() && rearm.wait_past(seen, Some(d)) {
                    continue; // something changed: re-read
                }
                if stop.is_canceled() {
                    return;
                }
                // The timer fired. Only while it is still armed for a lease
                // that ran out: the last subscriber may have left.
                if co
                    .next_expiry()
                    .is_some_and(|at| until(&at, &now()).is_zero())
                {
                    co.expire();
                }
            }
        }
    }
}
