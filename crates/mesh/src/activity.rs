//! Whether this device is exchanging rows or blobs with a peer right now
//! (card #1089): the signal behind the header's sync badge.
//!
//! Row batches and blob streams [`Activity::pulse`] as they move, and a
//! blob transfer holds a [`Activity::begin`] guard while it runs. The flag
//! stays true for [`HOLD`] after the last of them, so a sync of a few
//! milliseconds still shows long enough to see and a burst of batches
//! doesn't flicker it.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// How long the flag outlasts the last movement.
pub const HOLD: Duration = Duration::from_secs(1);

#[derive(Default)]
pub struct Activity {
    last: Mutex<Option<Instant>>,
    running: AtomicUsize,
}

/// A transfer in progress; the flag is true while one is held.
pub struct Busy<'a>(&'a Activity);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.running.fetch_sub(1, Ordering::SeqCst);
        self.0.pulse();
    }
}

impl Activity {
    /// Rows or blob bytes just moved.
    pub fn pulse(&self) {
        self.pulse_at(Instant::now());
    }

    pub(crate) fn pulse_at(&self, at: Instant) {
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(at);
    }

    /// A transfer starts; it counts until the guard is dropped.
    pub fn begin(&self) -> Busy<'_> {
        self.running.fetch_add(1, Ordering::SeqCst);
        self.pulse();
        Busy(self)
    }

    /// True while a transfer runs and for [`HOLD`] after the last movement.
    pub fn active(&self) -> bool {
        self.active_at(Instant::now())
    }

    pub(crate) fn active_at(&self, now: Instant) -> bool {
        if self.running.load(Ordering::SeqCst) > 0 {
            return true;
        }
        let last = *self.last.lock().unwrap_or_else(|e| e.into_inner());
        last.is_some_and(|t| now.saturating_duration_since(t) < HOLD)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_for_a_second_after_the_last_pulse() {
        let a = Activity::default();
        let t0 = Instant::now();
        assert!(!a.active_at(t0), "idle until something moves");
        a.pulse_at(t0);
        assert!(a.active_at(t0 + HOLD - Duration::from_millis(1)));
        assert!(!a.active_at(t0 + HOLD));
        // A later pulse extends it.
        a.pulse_at(t0 + Duration::from_millis(800));
        assert!(a.active_at(t0 + HOLD + Duration::from_millis(500)));
    }

    #[test]
    fn a_running_transfer_holds_it_on() {
        let a = Activity::default();
        let g = a.begin();
        assert!(a.active_at(Instant::now() + Duration::from_secs(60)));
        drop(g);
        assert!(a.active(), "the hold starts when the transfer ends");
        assert!(!a.active_at(Instant::now() + HOLD + Duration::from_millis(5)));
    }
}
