//! The `time.Time` arithmetic capture needs on [`GoTime`]: comparisons by
//! instant, `Add`, `Sub`, `Truncate(time.Second)`, `time.Now()` in the local
//! zone, and `time.Unix(ct, 0)` in the local zone.

use std::time::{Duration, SystemTime};

use gocompat::time::GoTime;

/// The instant as (seconds, nanoseconds), Go's comparison order.
fn key(t: &GoTime) -> (i64, u32) {
    (t.secs, t.nanos)
}

/// Go `a.Before(b)`.
pub fn before(a: &GoTime, b: &GoTime) -> bool {
    key(a) < key(b)
}

/// Go `a.After(b)`.
pub fn after(a: &GoTime, b: &GoTime) -> bool {
    key(a) > key(b)
}

/// Go `a.Sub(b)` in nanoseconds (not saturated: capture's spans are small).
pub fn sub_nanos(a: &GoTime, b: &GoTime) -> i128 {
    (i128::from(a.secs) - i128::from(b.secs)) * 1_000_000_000 + i128::from(a.nanos)
        - i128::from(b.nanos)
}

/// Go `t.Add(d)` for a signed nanosecond count, keeping the offset.
pub fn add_nanos(t: &GoTime, d: i128) -> GoTime {
    let total = i128::from(t.nanos) + d;
    let secs = t.secs as i128 + total.div_euclid(1_000_000_000);
    let nanos = total.rem_euclid(1_000_000_000) as u32;
    GoTime {
        secs: secs.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64,
        nanos,
        offset: t.offset,
    }
}

/// Go `t.Add(d)`.
pub fn add(t: &GoTime, d: Duration) -> GoTime {
    add_nanos(t, d.as_nanos() as i128)
}

/// Go `t.Add(-d)`.
pub fn sub(t: &GoTime, d: Duration) -> GoTime {
    add_nanos(t, -(d.as_nanos() as i128))
}

/// Go `t.Truncate(time.Second)` (absolute-time rounding; for times after
/// year 1 this drops the fraction).
pub fn truncate_second(t: &GoTime) -> GoTime {
    GoTime { nanos: 0, ..*t }
}

/// An instant shown in the local zone, as Go's `time.Unix(s, n)` is.
pub fn local(t: GoTime) -> GoTime {
    t.with_offset(sys::tz::local_offset(t.secs))
}

/// Go `time.Now()`: the system clock in the local zone.
pub fn now() -> GoTime {
    local(GoTime::from_system(SystemTime::now()))
}

/// Go `time.Unix(secs, 0)` (local zone).
pub fn unix_local(secs: i64) -> GoTime {
    local(GoTime::unix(secs, 0))
}

/// Go `t.Equal(u)`.
pub fn equal(a: &GoTime, b: &GoTime) -> bool {
    a.equal(b)
}

/// Ordering by instant.
pub fn cmp(a: &GoTime, b: &GoTime) -> std::cmp::Ordering {
    key(a).cmp(&key(b))
}
