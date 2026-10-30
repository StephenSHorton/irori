//! Go `time.Time` arithmetic on [`GoTime`]: nanosecond instants,
//! `Sub` (saturating like Go's Duration), `Add`, comparisons.

use std::cmp::Ordering;
use std::time::Duration;

use gocompat::time::GoTime;

const NS: i128 = 1_000_000_000;

/// Nanoseconds since the Unix epoch.
pub(crate) fn ns(t: &GoTime) -> i128 {
    i128::from(t.secs) * NS + i128::from(t.nanos)
}

/// Go `a.Sub(b)` in nanoseconds, saturating at the Duration range.
pub(crate) fn sub(a: &GoTime, b: &GoTime) -> i128 {
    (ns(a) - ns(b)).clamp(i128::from(i64::MIN), i128::from(i64::MAX))
}

/// Go `t.Add(d)`, keeping t's offset.
pub(crate) fn add(t: &GoTime, d: i128) -> GoTime {
    let n = ns(t) + d;
    GoTime {
        secs: n.div_euclid(NS) as i64,
        nanos: n.rem_euclid(NS) as u32,
        offset: t.offset,
    }
}

/// Go `a.Compare(b)`.
pub(crate) fn cmp(a: &GoTime, b: &GoTime) -> Ordering {
    ns(a).cmp(&ns(b))
}

/// Go `a.Before(b)`.
pub(crate) fn before(a: &GoTime, b: &GoTime) -> bool {
    ns(a) < ns(b)
}

/// A Duration in nanoseconds.
pub(crate) fn d(x: Duration) -> i128 {
    x.as_nanos() as i128
}

/// The system clock.
pub(crate) fn system_now() -> GoTime {
    GoTime::from_system(std::time::SystemTime::now())
}
