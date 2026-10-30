//! The local zone's UTC offset at an instant: what Go's `time.Local` shows
//! a time in. Capture writes `time.Now()` stamps and `commit.seen.at`
//! (`time.Unix(ct, 0)`) in the local zone (capture.md §2.4, §7 trap 10).

/// Seconds east of UTC of the local zone at Unix second `secs`; 0 when
/// the OS can't say.
pub fn local_offset(secs: i64) -> i32 {
    imp::local_offset(secs)
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{
        FileTimeToSystemTime, SystemTimeToFileTime, SystemTimeToTzSpecificLocalTime,
    };

    /// 100 ns ticks from 1601-01-01 to 1970-01-01.
    const EPOCH_TICKS: i64 = 116_444_736_000_000_000;

    fn ticks(ft: &FILETIME) -> i64 {
        ((u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)) as i64
    }

    pub fn local_offset(secs: i64) -> i32 {
        let t = secs.saturating_mul(10_000_000).saturating_add(EPOCH_TICKS);
        if t < 0 {
            return 0;
        }
        let ft = FILETIME {
            dwLowDateTime: t as u32,
            dwHighDateTime: (t >> 32) as u32,
        };
        // SAFETY: plain out-parameter calls on stack values.
        unsafe {
            let mut utc: SYSTEMTIME = std::mem::zeroed();
            if FileTimeToSystemTime(&ft, &mut utc) == 0 {
                return 0;
            }
            let mut local: SYSTEMTIME = std::mem::zeroed();
            if SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0 {
                return 0;
            }
            let mut lft: FILETIME = std::mem::zeroed();
            if SystemTimeToFileTime(&local, &mut lft) == 0 {
                return 0;
            }
            ((ticks(&lft) - t) / 10_000_000) as i32
        }
    }
}

#[cfg(unix)]
mod imp {
    pub fn local_offset(secs: i64) -> i32 {
        let t = secs as libc::time_t;
        // SAFETY: localtime_r fills the zeroed tm it is given.
        unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&t, &mut tm).is_null() {
                return 0;
            }
            tm.tm_gmtoff as i32
        }
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    pub fn local_offset(_: i64) -> i32 {
        0
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn offset_is_plausible() {
        let off = super::local_offset(1_793_000_000);
        assert!(off.abs() <= 14 * 3600, "{off}");
        assert_eq!(off % 60, 0);
    }
}
