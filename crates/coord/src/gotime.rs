//! `time.Time` arithmetic over [`GoTime`]: comparisons ignore the offset
//! (Go's `Before`/`After`/`Equal`), durations are nanoseconds.

use std::cmp::Ordering;
use std::time::Duration;

use gocompat::time::GoTime;

/// Nanoseconds since the Unix epoch, wide enough for Go's zero time.
pub fn ns(t: &GoTime) -> i128 {
    i128::from(t.secs) * 1_000_000_000 + i128::from(t.nanos)
}

/// The instant `n` nanoseconds after the epoch, in UTC.
pub fn from_ns(n: i128) -> GoTime {
    let secs = n.div_euclid(1_000_000_000) as i64;
    let nanos = n.rem_euclid(1_000_000_000) as i64;
    GoTime::unix(secs, nanos)
}

pub fn cmp(a: &GoTime, b: &GoTime) -> Ordering {
    ns(a).cmp(&ns(b))
}

pub fn before(a: &GoTime, b: &GoTime) -> bool {
    ns(a) < ns(b)
}

pub fn after(a: &GoTime, b: &GoTime) -> bool {
    ns(a) > ns(b)
}

/// `t.Add(d)`, keeping the offset.
pub fn add(t: &GoTime, d: Duration) -> GoTime {
    from_ns(ns(t) + d.as_nanos() as i128).with_offset(t.offset)
}

/// `a.Sub(b)` in nanoseconds (Go saturates at ±292 years; never reached).
pub fn since(a: &GoTime, b: &GoTime) -> i128 {
    ns(a) - ns(b)
}

/// `a.Sub(b) >= d`.
pub fn at_least(a: &GoTime, b: &GoTime, d: Duration) -> bool {
    since(a, b) >= d.as_nanos() as i128
}

/// The wall time of an HLC: `time.UnixMilli(hlc >> 16).UTC()`.
pub fn hlc_wall(h: hlc::Timestamp) -> GoTime {
    GoTime::unix_milli(h.wall_ms())
}
