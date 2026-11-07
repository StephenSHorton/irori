//! Claude usage left (card #599): the React app's lib/usage.ts, its pure
//! helpers. How much of the plan's 5-hour and weekly windows is left, as
//! Claude Code last reported it on this computer
//! ([`Window::usage_limits`] and `usage:changed`).
//!
//! Times read in the local zone ([`sys::tz::local_offset`]); the helpers
//! take the offset as a function so the tests read the same anywhere.
//!
//! [`Window::usage_limits`]: irori_window::Window::usage_limits

use irori_window::usage::{UsageLimits, UsageWindow};

const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;
const DAY: i64 = 24 * HOUR;
/// A reading older than this says when it was taken.
pub const STALE_MS: i64 = 10 * MIN;

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const ORDER: [&str; 2] = ["five_hour", "seven_day"];

/// The local zone's offset, in seconds east of UTC, at a Unix second.
pub type Offset = fn(i64) -> i32;

/// This machine's zone.
pub fn local(secs: i64) -> i32 {
    sys::tz::local_offset(secs)
}

/// A local wall time: the day number since 1970-01-01, the weekday (0 is
/// Sunday), hours and minutes.
struct Wall {
    day: i64,
    weekday: usize,
    hour: i64,
    minute: i64,
}

fn wall(ms: i64, off: Offset) -> Wall {
    let local = ms + off(ms.div_euclid(1000)) as i64 * 1000;
    let day = local.div_euclid(DAY);
    let in_day = local.rem_euclid(DAY);
    Wall {
        day,
        // 1970-01-01 was a Thursday.
        weekday: (day + 4).rem_euclid(7) as usize,
        hour: in_day / HOUR,
        minute: (in_day % HOUR) / MIN,
    }
}

/// There is a reading with at least one window.
pub fn has_usage(u: &UsageLimits) -> bool {
    u.observed_at_ms > 0 && !u.windows.is_empty()
}

/// The window has started over since the reading.
pub fn is_reset(w: &UsageWindow, now: i64) -> bool {
    w.resets_at_ms.is_some_and(|r| r <= now)
}

/// What is left of a window, a whole 0 to 100: all of it once the window
/// has started over.
pub fn percent_left(w: &UsageWindow, now: i64) -> i64 {
    if is_reset(w, now) {
        return 100;
    }
    (100.0 - w.percent_used).clamp(0.0, 100.0).round() as i64
}

/// A window's name: "5-hour", "Weekly", "Spend limit".
pub fn window_label(kind: &str) -> String {
    match kind {
        "five_hour" => "5-hour".into(),
        "seven_day" => "Weekly".into(),
        "seven_day_opus" => "Weekly (Opus)".into(),
        "seven_day_sonnet" => "Weekly (Sonnet)".into(),
        _ => {
            let words: Vec<&str> = kind.split(['_', '-']).filter(|w| !w.is_empty()).collect();
            let words = words.join(" ");
            let mut c = words.chars();
            match c.next() {
                Some(f) => f.to_uppercase().chain(c).collect(),
                None => kind.to_string(),
            }
        }
    }
}

fn rank(kind: &str) -> usize {
    ORDER.iter().position(|k| *k == kind).unwrap_or(ORDER.len())
}

/// The 5-hour and weekly windows first, the rest after (a stable sort).
pub fn sort_windows(ws: &[UsageWindow]) -> Vec<UsageWindow> {
    let mut v = ws.to_vec();
    v.sort_by_key(|w| rank(&w.kind));
    v
}

/// A local "14:05".
pub fn clock(ms: i64, off: Offset) -> String {
    let w = wall(ms, off);
    format!("{:02}:{:02}", w.hour, w.minute)
}

/// When a window starts over: "resets in 2h 10m" within a day, "resets
/// Thu 14:00" further off, "reset" once it has.
pub fn reset_line(resets_at_ms: Option<i64>, now: i64, off: Offset) -> Option<String> {
    let r = resets_at_ms?;
    let left = r - now;
    Some(if left <= 0 {
        "reset".into()
    } else if left < MIN {
        "resets in under a minute".into()
    } else if left < DAY {
        let h = left / HOUR;
        let m = (left % HOUR) / MIN;
        if h > 0 {
            format!("resets in {h}h {m}m")
        } else {
            format!("resets in {m}m")
        }
    } else {
        format!("resets {} {}", DAYS[wall(r, off).weekday], clock(r, off))
    })
}

/// When an old reading was taken: "as of 14:05", "as of Tue 14:05" on
/// another day; None when it is recent.
pub fn stale_line(observed_at_ms: i64, now: i64, off: Offset) -> Option<String> {
    if observed_at_ms <= 0 || now - observed_at_ms <= STALE_MS {
        return None;
    }
    let seen = wall(observed_at_ms, off);
    let day = if seen.day == wall(now, off).day {
        String::new()
    } else {
        format!("{} ", DAYS[seen.weekday])
    };
    Some(format!("as of {day}{}", clock(observed_at_ms, off)))
}

/// The top bar's reading of what is left: `text` is "62% · 39% left"
/// (5-hour, then weekly), `title` spells it out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripLine {
    pub text: String,
    pub title: String,
}

/// The top bar's Claude reading, or None with no reading. With no 5-hour
/// or weekly window (a gateway's spend limit only) it shows the first
/// window there is.
pub fn strip_line(u: &UsageLimits, now: i64) -> Option<StripLine> {
    if !has_usage(u) {
        return None;
    }
    let known: Vec<UsageWindow> = u
        .windows
        .iter()
        .filter(|w| ORDER.contains(&w.kind.as_str()))
        .cloned()
        .collect();
    let shown = if known.is_empty() {
        sort_windows(&u.windows).into_iter().take(1).collect()
    } else {
        sort_windows(&known)
    };
    let pcts: Vec<String> = shown
        .iter()
        .map(|w| format!("{}%", percent_left(w, now)))
        .collect();
    let title = shown
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let label = window_label(&w.kind);
            let label = if i > 0 { label.to_lowercase() } else { label };
            format!("{label} {} left", pcts[i])
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(StripLine {
        text: format!("{} left", pcts.join(" · ")),
        title,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(_: i64) -> i32 {
        0
    }

    /// Unix ms for a UTC date and time (month 1-12).
    fn at(y: i64, mo: i64, d: i64, h: i64, mi: i64) -> i64 {
        // Howard Hinnant's days_from_civil.
        let y = if mo <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (mo + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        days * DAY + h * HOUR + mi * MIN
    }

    fn win(kind: &str, used: f64, resets: Option<i64>) -> UsageWindow {
        UsageWindow {
            kind: kind.into(),
            percent_used: used,
            resets_at_ms: resets,
        }
    }

    // Thu 5 Nov 2026 14:15.
    fn now() -> i64 {
        at(2026, 11, 5, 14, 15)
    }

    fn sample(now: i64) -> UsageLimits {
        UsageLimits {
            observed_at_ms: now - 2 * MIN,
            windows: vec![
                win("five_hour", 38.0, Some(now + 2 * HOUR + 10 * MIN)),
                win("seven_day", 61.0, Some(now + 3 * DAY)),
            ],
        }
    }

    #[test]
    fn percent_left_all_of_it_once_reset() {
        let n = now();
        assert_eq!(percent_left(&win("five_hour", 38.0, Some(n + HOUR)), n), 62);
        assert_eq!(percent_left(&win("five_hour", 38.0, Some(n - 1)), n), 100);
        assert_eq!(percent_left(&win("spend_limit", 130.0, None), n), 0);
        assert_eq!(percent_left(&win("seven_day", 0.4, None), n), 100);
    }

    #[test]
    fn reset_countdown() {
        let n = now();
        let r = |t| reset_line(t, n, utc);
        assert_eq!(
            r(Some(n + 2 * HOUR + 10 * MIN)).as_deref(),
            Some("resets in 2h 10m")
        );
        assert_eq!(r(Some(n + 45 * MIN)).as_deref(), Some("resets in 45m"));
        assert_eq!(
            r(Some(n + 20_000)).as_deref(),
            Some("resets in under a minute")
        );
        assert_eq!(
            r(Some(at(2026, 11, 12, 14, 0))).as_deref(),
            Some("resets Thu 14:00")
        );
        assert_eq!(r(Some(n - 1)).as_deref(), Some("reset"));
        assert_eq!(r(None), None);
    }

    #[test]
    fn staleness() {
        let n = now();
        let s = |t| stale_line(t, n, utc);
        assert_eq!(s(n - 2 * MIN), None);
        assert_eq!(s(at(2026, 11, 5, 14, 5)), None);
        assert_eq!(s(at(2026, 11, 5, 14, 4)).as_deref(), Some("as of 14:04"));
        assert_eq!(
            s(at(2026, 11, 3, 9, 30)).as_deref(),
            Some("as of Tue 09:30")
        );
        assert_eq!(s(0), None);
    }

    #[test]
    fn the_strips_line() {
        let n = now();
        assert_eq!(
            strip_line(&sample(n), n),
            Some(StripLine {
                text: "62% · 39% left".into(),
                title: "5-hour 62% left, weekly 39% left".into(),
            })
        );
        // A gateway's spend limit only: the strip still says something.
        let spend = UsageLimits {
            observed_at_ms: n,
            windows: vec![win("spend_limit", 60.0, None)],
        };
        assert_eq!(
            strip_line(&spend, n),
            Some(StripLine {
                text: "40% left".into(),
                title: "Spend limit 40% left".into(),
            })
        );
        let wk = UsageLimits {
            observed_at_ms: n,
            windows: vec![win("seven_day", 10.0, None)],
        };
        assert_eq!(
            strip_line(&wk, n),
            Some(StripLine {
                text: "90% left".into(),
                title: "Weekly 90% left".into(),
            })
        );
        assert_eq!(strip_line(&UsageLimits::default(), n), None);
    }

    #[test]
    fn names_and_order() {
        assert_eq!(window_label("five_hour"), "5-hour");
        assert_eq!(window_label("seven_day"), "Weekly");
        assert_eq!(window_label("spend_limit"), "Spend limit");
        let ws = sort_windows(&[
            win("spend_limit", 1.0, None),
            win("seven_day", 1.0, None),
            win("five_hour", 1.0, None),
        ]);
        let kinds: Vec<&str> = ws.iter().map(|w| w.kind.as_str()).collect();
        assert_eq!(kinds, ["five_hour", "seven_day", "spend_limit"]);
    }

    #[test]
    fn no_answer_is_no_reading() {
        assert!(!has_usage(&UsageLimits::default()));
        let u = UsageLimits {
            observed_at_ms: 5,
            windows: vec![win("five_hour", 1.0, None)],
        };
        assert!(has_usage(&u));
    }

    #[test]
    fn local_wall_time_follows_the_offset() {
        let ten_past_two = at(2026, 11, 5, 14, 10);
        fn plus_nine(_: i64) -> i32 {
            9 * 3600
        }
        assert_eq!(clock(ten_past_two, utc), "14:10");
        assert_eq!(clock(ten_past_two, plus_nine), "23:10");
    }
}
