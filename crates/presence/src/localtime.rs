//! Go's `time.Local` for the times presence makes (`time.UnixMilli`,
//! `time.Now()`), which Go shows in the local zone: card JSON then carries
//! the local offset, as the Go build writes it.
//!
//! Windows: Go applies this year's daylight-saving rule to every year
//! (zoneinfo_windows.go), which is what `SystemTimeToTzSpecificLocalTime`
//! with no zone does. Unix: `localtime_r`'s `tm_gmtoff`.
//!
//! Capture needs the same thing for its rows (capture.md §7.6); at the
//! merge this belongs in `gocompat::time`.

use std::time::SystemTime;

use gocompat::time::GoTime;

/// `t` shown in the local zone (Go `t.In(time.Local)`).
pub fn local(t: GoTime) -> GoTime {
    if t.is_zero() {
        return t;
    }
    t.with_offset(offset_at(t.secs))
}

/// Go `time.Now()`: now, in the local zone.
pub fn now_local() -> GoTime {
    local(GoTime::from_system(SystemTime::now()))
}

#[cfg(windows)]
fn offset_at(secs: i64) -> i32 {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{
        FileTimeToSystemTime, SystemTimeToFileTime, SystemTimeToTzSpecificLocalTime,
    };
    // 100 ns ticks since 1601.
    let Some(ticks) = secs
        .checked_add(11_644_473_600)
        .and_then(|s| s.checked_mul(10_000_000))
        .filter(|t| *t >= 0)
    else {
        return 0;
    };
    let ft = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    // SAFETY: plain out-parameter calls on stack values.
    unsafe {
        let mut st: SYSTEMTIME = std::mem::zeroed();
        let mut lt: SYSTEMTIME = std::mem::zeroed();
        let mut lft: FILETIME = std::mem::zeroed();
        if FileTimeToSystemTime(&ft, &mut st) == 0
            || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &st, &mut lt) == 0
            || SystemTimeToFileTime(&lt, &mut lft) == 0
        {
            return 0;
        }
        let lticks = (i64::from(lft.dwHighDateTime) << 32) | i64::from(lft.dwLowDateTime);
        ((lticks - ticks) / 10_000_000) as i32
    }
}

#[cfg(unix)]
fn offset_at(secs: i64) -> i32 {
    let t = secs as libc::time_t;
    // SAFETY: localtime_r writes into the zeroed tm we own.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return 0;
        }
        tm.tm_gmtoff as i32
    }
}

#[cfg(not(any(windows, unix)))]
fn offset_at(_: i64) -> i32 {
    0
}
