//! Hybrid logical clock, a port of I:internal/hlc/hlc.go (store.md §2.6).
//!
//! A [`Timestamp`] is a `u64`: Unix milliseconds in the high 48 bits and a
//! logical counter in the low 16, so timestamps from every device order
//! sensibly and never go backwards on one device. Zero means unset.
//!
//! Go's arithmetic is kept exactly, including where it wraps:
//! - `now` takes the wall clock's `UnixMilli` (negative before 1970), casts it
//!   to `u64` and shifts left 16, dropping the high bits;
//! - a counter overflow simply carries into the wall bits;
//! - `u64::MAX + 1` wraps to 0, as Go's `c.last++` does.

use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// An HLC reading. Compare with `<` and `>`; zero means unset. In JSON it is a
/// bare integer (`/v1/events` `hlc`), above 2^53 like Go's.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct Timestamp(pub u64);

impl Timestamp {
    /// The wall-clock part in Unix milliseconds (`int64(t >> 16)`).
    pub fn wall_ms(self) -> i64 {
        (self.0 >> 16) as i64
    }

    /// The wall-clock part (Go `Wall()`, `time.UnixMilli(int64(t >> 16))`).
    pub fn wall(self) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(self.0 >> 16)
    }

    /// The counter part (Go `Logical()`).
    pub fn logical(self) -> u16 {
        self.0 as u16
    }

    /// The SQLite column value: a plain bit cast (store.md §2.4).
    pub fn as_i64(self) -> i64 {
        self.0 as i64
    }

    /// From the SQLite column value.
    pub fn from_i64(v: i64) -> Timestamp {
        Timestamp(v as u64)
    }

    /// Go's `Timestamp(UnixMilli(t)) << 16` for a wall time.
    pub fn from_unix_ms(ms: i64) -> Timestamp {
        Timestamp((ms as u64).wrapping_shl(16))
    }
}

impl std::fmt::Display for Timestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<u64> for Timestamp {
    fn from(v: u64) -> Self {
        Timestamp(v)
    }
}

impl From<Timestamp> for u64 {
    fn from(t: Timestamp) -> Self {
        t.0
    }
}

/// Go `time.Time.UnixMilli()` of a `SystemTime`: floor division, negative
/// before 1970.
pub fn unix_milli(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => {
            let d = e.duration();
            // Floor: -0.5 ms is -1.
            let ms = d.as_millis() as i64;
            if d.as_nanos() % 1_000_000 != 0 {
                -ms - 1
            } else {
                -ms
            }
        }
    }
}

/// A wall-clock source; tests inject a fixed one (Go `New(now)`).
pub type NowFn = Box<dyn Fn() -> SystemTime + Send + Sync>;

/// Hands out increasing timestamps. Safe for concurrent use.
pub struct Clock {
    now: NowFn,
    last: Mutex<u64>,
}

impl std::fmt::Debug for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Clock").field("last", &self.last()).finish()
    }
}

impl Default for Clock {
    fn default() -> Self {
        Clock::new()
    }
}

impl Clock {
    /// A clock on the system time (Go `New(nil)`).
    pub fn new() -> Clock {
        Clock::with_now(Box::new(SystemTime::now))
    }

    /// A clock reading wall time from `now` (Go `New(now)`).
    pub fn with_now(now: NowFn) -> Clock {
        Clock {
            now,
            last: Mutex::new(0),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, u64> {
        // A panic while holding the lock cannot leave `last` half-written.
        self.last.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A timestamp later than every one this clock has issued or seen.
    pub fn now(&self) -> Timestamp {
        // Go reads the wall clock under the lock too.
        let mut last = self.lock();
        let wall = Timestamp::from_unix_ms(unix_milli((self.now)())).0;
        if wall > *last {
            *last = wall;
        } else {
            *last = last.wrapping_add(1);
        }
        Timestamp(*last)
    }

    /// Fold in a timestamp from another device so later `now` calls exceed it.
    pub fn observe(&self, remote: Timestamp) {
        let mut last = self.lock();
        if remote.0 > *last {
            *last = remote.0;
        }
    }

    /// `observe` with `remote` capped at `max`, so a peer's row stamped far in
    /// the future cannot drag this clock with it.
    pub fn observe_bounded(&self, remote: Timestamp, max: Timestamp) {
        self.observe(remote.min(max));
    }

    /// The last timestamp issued or observed (zero for a fresh clock).
    pub fn last(&self) -> Timestamp {
        Timestamp(*self.lock())
    }
}
