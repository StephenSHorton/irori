//! Claude usage left (card #599): the plan's rate-limit windows (the
//! 5-hour and weekly ones, or a gateway's spend limit) as this device's
//! Claude Code sessions last saw them.
//!
//! The Irori mod posts each session's `session.measure` windows to
//! `POST /v1/usage/limits`; the Core keeps the newest reading and the
//! window draws it. `usage-limits.json` in the data dir holds it across a
//! restart, written through `usage-limits.json.tmp`, 0600.
//!
//! Every session reports the same account, so readings merge by window
//! kind: a session that hasn't made a request in a while (one reattaching
//! after an Irori restart, say) must not pull a newer, higher figure back.
//! For one kind, a later reset time is a newer window and wins; the same
//! reset time keeps the higher figure (use only grows inside a window); an
//! earlier reset time is an old window and is ignored. A window with no
//! reset time (a gateway's spend limit) has no window to compare, so the
//! newest reading that reports it wins. A kind a reading leaves out stays:
//! until its reset time passes, or, with no reset time, until a reading
//! reports it again. A window past its reset time is dropped wherever it
//! is read.
//!
//! A reading may name the Claude account it came from (`account`, an opaque
//! id the mod reads from the session's login; card #1040). When it names a
//! different account from the one the kept windows belong to, or the kept
//! windows have no account yet, they are another account's (a subscription
//! switch) and are replaced by the reading, not merged. A reading without
//! an account (an older mod, or a login the mod couldn't read) merges as
//! above and leaves the kept account alone.
//!
//! A reading is checked window by window: one with a kind or reset time
//! the Core can't take is logged and left out, and the rest still count,
//! so a new kind from a newer Claude Code can't blank the others.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use gocompat::time::GoTime;
use serde::{Deserialize, Serialize};

use crate::reset::write_private;

pub const FILE: &str = "usage-limits.json";
/// At most this many windows in one reading (there are three kinds today).
pub const MAX_WINDOWS: usize = 8;
/// A window's kind: ASCII letters, digits, `_`, `-` or `.` (as in
/// `seven_day.opus`), at most this long.
pub const MAX_KIND: usize = 32;
/// The highest figure kept: past 100 is an exceeded spend limit.
pub const MAX_PERCENT: f64 = 1000.0;
/// The longest account id kept.
pub const MAX_ACCOUNT: usize = 128;
/// Two reset times this close are the same window (a server's rounding).
const SAME_RESET_MS: i64 = 60_000;

/// One window as a session reported it (Claude Code's `SessionRateLimit`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimit {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub percent_used: f64,
    /// ISO 8601 (RFC 3339).
    #[serde(default)]
    pub resets_at: Option<String>,
}

/// One kept window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    /// `five_hour`, `seven_day`, a gateway's `spend_limit`, or another.
    pub kind: String,
    /// 0 to 100 (past 100 on an exceeded spend limit).
    pub percent_used: f64,
    /// When it resets, Unix ms; `None` when the session didn't say.
    pub resets_at_ms: Option<i64>,
}

/// The newest reading.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageLimits {
    /// When the Core took the newest reading, Unix ms; 0 for none yet.
    pub observed_at_ms: i64,
    pub windows: Vec<UsageWindow>,
    /// The account these windows belong to, when a reading named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

impl UsageLimits {
    /// Without the windows whose reset time is before `now_ms`.
    pub fn fresh(mut self, now_ms: i64) -> UsageLimits {
        self.windows.retain(|w| !stale(w, now_ms));
        self
    }
}

fn stale(w: &UsageWindow, now_ms: i64) -> bool {
    w.resets_at_ms.is_some_and(|r| r <= now_ms)
}

/// An account id as kept: trimmed, 1-[`MAX_ACCOUNT`] printable ASCII.
/// Anything else is no account (logged by the caller's reading, not refused).
pub fn good_account(account: Option<&str>) -> Option<String> {
    let a = account?.trim();
    (!a.is_empty() && a.len() <= MAX_ACCOUNT && a.bytes().all(|b| b.is_ascii_graphic()))
        .then(|| a.to_string())
}

fn good_kind(kind: &str) -> bool {
    !kind.is_empty()
        && kind.len() <= MAX_KIND
        && kind
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// One reported window as kept, or why it can't be.
fn check(r: &RateLimit) -> Result<UsageWindow, String> {
    let kind = r.kind.trim();
    if !good_kind(kind) {
        return Err(format!(
            "kind {kind:?} is not 1-{MAX_KIND} of A-Z a-z 0-9 _ - ."
        ));
    }
    if !r.percent_used.is_finite() {
        return Err(format!("{kind}: percentUsed is not a number"));
    }
    let resets_at_ms = match r.resets_at.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(s) => Some(
            GoTime::parse_rfc3339(s)
                .map(|t| t.unix_milli_value())
                .ok_or_else(|| format!("{kind}: resetsAt {s:?} is not an RFC 3339 time"))?,
        ),
    };
    Ok(UsageWindow {
        kind: kind.to_string(),
        percent_used: r.percent_used.clamp(0.0, MAX_PERCENT),
        resets_at_ms,
    })
}

/// Checks a reading window by window. A window the Core can't take (its
/// kind, figure or reset time) is logged and left out; so is one already
/// past its reset time. A kind given twice keeps its last. `Err`, the
/// 400's text, is only for a reading too long to be one.
pub fn validate(input: &[RateLimit], now_ms: i64) -> Result<Vec<UsageWindow>, String> {
    if input.len() > MAX_WINDOWS {
        return Err(format!("at most {MAX_WINDOWS} rate-limit windows"));
    }
    let mut out: Vec<UsageWindow> = Vec::new();
    for r in input {
        let w = match check(r) {
            Ok(w) => w,
            Err(e) => {
                crate::logf(format_args!("usage limits: window left out: {e}"));
                continue;
            }
        };
        if stale(&w, now_ms) {
            continue;
        }
        out.retain(|o| o.kind != w.kind);
        out.push(w);
    }
    Ok(out)
}

/// A kept reading from the file, held to what [`validate`] lets in.
fn sane(mut m: UsageLimits) -> UsageLimits {
    let mut windows: Vec<UsageWindow> = Vec::new();
    for mut w in m.windows {
        if !good_kind(&w.kind) || !w.percent_used.is_finite() {
            continue;
        }
        w.percent_used = w.percent_used.clamp(0.0, MAX_PERCENT);
        windows.retain(|o| o.kind != w.kind);
        windows.push(w);
    }
    windows.truncate(MAX_WINDOWS);
    m.windows = windows;
    m.account = good_account(m.account.as_deref());
    m
}

/// `kept` with `reading` merged in (the module's rules), stale windows
/// gone, in kind order.
pub fn merge(kept: &[UsageWindow], reading: Vec<UsageWindow>, now_ms: i64) -> Vec<UsageWindow> {
    let mut out: Vec<UsageWindow> = kept.iter().filter(|w| !stale(w, now_ms)).cloned().collect();
    for r in reading {
        match out.iter_mut().find(|o| o.kind == r.kind) {
            None => out.push(r),
            Some(o) => match (o.resets_at_ms, r.resets_at_ms) {
                (Some(old), Some(new)) if (new - old).abs() <= SAME_RESET_MS => {
                    o.percent_used = o.percent_used.max(r.percent_used);
                    o.resets_at_ms = Some(old.max(new));
                }
                (Some(old), Some(new)) if new < old => {} // an old window
                _ => *o = r,
            },
        }
    }
    out.sort_by(|a, b| a.kind.cmp(&b.kind));
    out
}

/// The kept reading and its file.
pub struct Usage {
    path: PathBuf,
    m: Mutex<UsageLimits>,
}

impl Usage {
    /// Loads `<data_dir>/usage-limits.json`, held to the same checks as a
    /// posted reading; an absent or unreadable file is no reading.
    pub fn new(data_dir: &Path) -> Usage {
        let path = data_dir.join(FILE);
        let m = std::fs::read(&path)
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .map(sane)
            .unwrap_or_default();
        Usage {
            path,
            m: Mutex::new(m),
        }
    }

    /// The reading as of `now_ms` (stale windows left out).
    pub fn get(&self, now_ms: i64) -> UsageLimits {
        self.lock().clone().fresh(now_ms)
    }

    /// Takes a session's reading at `now_ms` and saves it. `Ok(false)`
    /// when no window in it survived the checks: nothing changes, not even
    /// the observed time.
    pub fn record(
        &self,
        input: &[RateLimit],
        account: Option<&str>,
        now_ms: i64,
    ) -> Result<bool, String> {
        let reading = validate(input, now_ms)?;
        if reading.is_empty() {
            return Ok(false);
        }
        let account = match account {
            Some(a) => match good_account(Some(a)) {
                None => {
                    crate::logf(format_args!(
                        "usage limits: account left out: not 1-{MAX_ACCOUNT} printable ASCII"
                    ));
                    None
                }
                some => some,
            },
            None => None,
        };
        let mut m = self.lock();
        // Another account's windows (or ones of no known account) are not
        // this reading's to merge with: a subscription switch.
        let switched = account.is_some() && m.account != account;
        let kept: &[UsageWindow] = if switched { &[] } else { &m.windows };
        let windows = merge(kept, reading, now_ms);
        *m = UsageLimits {
            observed_at_ms: now_ms,
            windows,
            account: account.or_else(|| m.account.take()),
        };
        self.save(&m);
        Ok(true)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, UsageLimits> {
        self.m.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, m: &UsageLimits) {
        let Ok(raw) = serde_json::to_vec_pretty(m) else {
            return;
        };
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        if let Err(e) = write_private(&tmp, &raw) {
            crate::logf(format_args!("usage limits: {e}"));
            return;
        }
        if let Err(e) = std::fs::rename(&tmp, &self.path) {
            crate::logf(format_args!("usage limits: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn rl(kind: &str, pct: f64, resets_at: Option<&str>) -> RateLimit {
        RateLimit {
            kind: kind.into(),
            percent_used: pct,
            resets_at: resets_at.map(Into::into),
        }
    }

    fn win(kind: &str, pct: f64, resets_at_ms: Option<i64>) -> UsageWindow {
        UsageWindow {
            kind: kind.into(),
            percent_used: pct,
            resets_at_ms,
        }
    }

    fn iso(ms: i64) -> String {
        GoTime::unix_milli(ms).utc().format_rfc3339()
    }

    #[test]
    fn reset_times_parse_with_and_without_fractions() {
        let w = validate(
            &[
                rl("five_hour", 23.5, Some("2027-01-16T08:00:00.000Z")),
                rl("seven_day", 41.2, Some("2027-01-16T08:00:00+02:00")),
                rl("spend_limit", 7.0, None),
            ],
            NOW,
        )
        .unwrap();
        assert_eq!(w[0].resets_at_ms, Some(NOW + 86_400_000));
        assert_eq!(
            w[1].resets_at_ms,
            Some(w[0].resets_at_ms.unwrap() - 7_200_000)
        );
        assert_eq!(w[2].resets_at_ms, None);
    }

    #[test]
    fn a_bad_window_is_left_out_and_the_rest_kept() {
        let future = iso(NOW + 3_600_000);
        for bad in [
            rl("", 1.0, None),
            rl("five hour", 1.0, None),
            rl("é", 1.0, None),
            rl(&"k".repeat(MAX_KIND + 1), 1.0, None),
            rl("five_hour", f64::NAN, None),
            rl("five_hour", f64::INFINITY, None),
            rl("five_hour", 1.0, Some("tomorrow")),
            rl("five_hour", 1.0, Some("2027-01-16T08:00:00")),
        ] {
            assert_eq!(
                validate(&[bad.clone(), rl("seven_day", 9.0, Some(&future))], NOW),
                Ok(vec![win("seven_day", 9.0, Some(NOW + 3_600_000))]),
                "{bad:?}"
            );
        }
        // A kind with a dot, as a per-model weekly window might be.
        assert_eq!(
            validate(&[rl("seven_day.opus", 3.0, Some(&future))], NOW),
            Ok(vec![win("seven_day.opus", 3.0, Some(NOW + 3_600_000))])
        );
        // Only a reading too long to be one is refused whole.
        let many: Vec<_> = (0..=MAX_WINDOWS)
            .map(|i| rl(&format!("k{i}"), 1.0, Some(&future)))
            .collect();
        assert!(validate(&many, NOW).is_err());
    }

    #[test]
    fn figures_clamp_and_stale_windows_drop() {
        let w = validate(
            &[
                rl("five_hour", -4.0, Some(&iso(NOW + 60_000))),
                rl("spend_limit", 5000.0, None),
                rl("seven_day", 50.0, Some(&iso(NOW - 60_000))),
                rl("five_hour", 12.5, Some(&iso(NOW + 60_000))),
            ],
            NOW,
        )
        .unwrap();
        assert_eq!(
            w,
            vec![
                win("spend_limit", MAX_PERCENT, None),
                win("five_hour", 12.5, Some(NOW + 60_000))
            ]
        );
    }

    #[test]
    fn readings_merge_by_kind() {
        let (h5, d7) = (NOW + 3_600_000, NOW + 86_400_000);
        let kept = vec![
            win("five_hour", 40.0, Some(h5)),
            win("seven_day", 20.0, Some(d7)),
        ];
        // The same window from a session that hasn't heard since: the higher figure stays.
        assert_eq!(
            merge(&kept, vec![win("five_hour", 30.0, Some(h5 + 1000))], NOW),
            vec![
                win("five_hour", 40.0, Some(h5 + 1000)),
                win("seven_day", 20.0, Some(d7))
            ]
        );
        // A newer window replaces it, lower figure and all.
        assert_eq!(
            merge(
                &kept,
                vec![win("five_hour", 2.0, Some(h5 + 5 * 3_600_000))],
                NOW
            )[0],
            win("five_hour", 2.0, Some(h5 + 5 * 3_600_000))
        );
        // An older window is ignored.
        assert_eq!(
            merge(
                &kept,
                vec![win("five_hour", 90.0, Some(h5 - 3_600_000))],
                NOW
            )[0],
            win("five_hour", 40.0, Some(h5))
        );
        // A stale kept window goes. One with no reset time stays through a
        // reading that leaves it out, and the newest that reports it wins,
        // lower figure and all.
        let kept = vec![
            win("five_hour", 40.0, Some(NOW - 1)),
            win("spend_limit", 3.0, None),
        ];
        assert_eq!(
            merge(&kept, vec![], NOW),
            vec![win("spend_limit", 3.0, None)]
        );
        assert_eq!(
            merge(&kept, vec![win("seven_day", 5.0, Some(d7))], NOW),
            vec![
                win("seven_day", 5.0, Some(d7)),
                win("spend_limit", 3.0, None)
            ]
        );
        assert_eq!(
            merge(&kept, vec![win("spend_limit", 1.0, None)], NOW),
            vec![win("spend_limit", 1.0, None)]
        );
    }

    #[test]
    fn a_new_account_replaces_the_kept_windows() {
        let d = tempfile::tempdir().unwrap();
        let u = Usage::new(d.path());
        let (h5, d7) = (NOW + 3_600_000, NOW + 86_400_000);
        let both = |five: f64, seven: f64| {
            vec![
                rl("five_hour", five, Some(&iso(h5))),
                rl("seven_day", seven, Some(&iso(d7))),
            ]
        };
        u.record(&both(80.0, 60.0), Some("acct-a"), NOW).unwrap();
        // The same account keeps the higher figure.
        u.record(&both(10.0, 70.0), Some("acct-a"), NOW + 1)
            .unwrap();
        assert_eq!(
            u.get(NOW + 1).windows,
            vec![
                win("five_hour", 80.0, Some(h5)),
                win("seven_day", 70.0, Some(d7))
            ]
        );
        // A reading with no account merges and leaves the account alone.
        u.record(&both(5.0, 5.0), None, NOW + 2).unwrap();
        assert_eq!(u.get(NOW + 2).account.as_deref(), Some("acct-a"));
        assert_eq!(u.get(NOW + 2).windows[0].percent_used, 80.0);
        // Another account: lower figures, an earlier reset and a kind the
        // old account had and the new one doesn't report are all replaced.
        let early = h5 - 1_800_000;
        u.record(
            &[rl("five_hour", 3.0, Some(&iso(early)))],
            Some("acct-b"),
            NOW + 3,
        )
        .unwrap();
        let got = u.get(NOW + 3);
        assert_eq!(got.account.as_deref(), Some("acct-b"));
        assert_eq!(got.windows, vec![win("five_hour", 3.0, Some(early))]);
        // It holds across a restart, and a switch back replaces again.
        let again = Usage::new(d.path());
        assert_eq!(again.get(NOW + 3), got);
        again
            .record(&both(1.0, 1.0), Some("acct-a"), NOW + 4)
            .unwrap();
        assert_eq!(again.get(NOW + 4).windows.len(), 2);
        assert_eq!(again.get(NOW + 4).windows[0].percent_used, 1.0);
        // A bad account id is no account: it merges and keeps the kept one.
        again
            .record(&both(0.5, 0.5), Some("not ok"), NOW + 5)
            .unwrap();
        assert_eq!(again.get(NOW + 5).account.as_deref(), Some("acct-a"));
        assert_eq!(again.get(NOW + 5).windows[0].percent_used, 1.0);
        // A file from before accounts: the first named reading replaces it.
        std::fs::write(
            d.path().join(FILE),
            serde_json::json!({"observedAtMs": NOW, "windows": [
                {"kind": "five_hour", "percentUsed": 90.0, "resetsAtMs": h5}]})
            .to_string(),
        )
        .unwrap();
        let old = Usage::new(d.path());
        old.record(
            &[rl("five_hour", 2.0, Some(&iso(h5)))],
            Some("acct-b"),
            NOW + 6,
        )
        .unwrap();
        assert_eq!(
            old.get(NOW + 6).windows,
            vec![win("five_hour", 2.0, Some(h5))]
        );
    }

    #[test]
    fn a_reading_survives_a_restart() {
        let d = tempfile::tempdir().unwrap();
        let u = Usage::new(d.path());
        assert_eq!(u.get(NOW), UsageLimits::default());
        let h5 = NOW + 3_600_000;
        assert!(
            u.record(&[rl("five_hour", 23.5, Some(&iso(h5)))], None, NOW)
                .unwrap()
        );
        // The same figures again: taken, so the observed time moves.
        assert!(
            u.record(&[rl("five_hour", 23.5, Some(&iso(h5)))], None, NOW + 5)
                .unwrap()
        );
        // Nothing usable in it: not taken, nothing moves.
        for empty in [
            vec![],
            vec![rl("bad kind", 1.0, None)],
            vec![rl("seven_day", 1.0, Some(&iso(NOW - 1)))],
        ] {
            assert!(!u.record(&empty, None, NOW + 7).unwrap(), "{empty:?}");
        }
        let again = Usage::new(d.path());
        assert_eq!(
            again.get(NOW + 10),
            UsageLimits {
                observed_at_ms: NOW + 5,
                windows: vec![win("five_hour", 23.5, Some(h5))],
                account: None,
            }
        );
        // Past its reset time it reads as gone.
        assert!(again.get(h5).windows.is_empty());
        std::fs::write(d.path().join(FILE), b"junk").unwrap();
        assert_eq!(Usage::new(d.path()).get(NOW), UsageLimits::default());
    }

    #[test]
    fn a_kept_file_is_held_to_the_same_checks() {
        let d = tempfile::tempdir().unwrap();
        let h5 = NOW + 3_600_000;
        let mut windows = vec![
            serde_json::json!({"kind": "five hour", "percentUsed": 5.0, "resetsAtMs": h5}),
            serde_json::json!({"kind": "five_hour", "percentUsed": 1.0, "resetsAtMs": h5}),
            serde_json::json!({"kind": "five_hour", "percentUsed": 4000.0, "resetsAtMs": h5}),
        ];
        windows.extend((0..MAX_WINDOWS).map(|i| {
            serde_json::json!({"kind": format!("k{i}"), "percentUsed": -3.0, "resetsAtMs": null})
        }));
        let raw = serde_json::json!({"observedAtMs": NOW, "windows": windows});
        std::fs::write(d.path().join(FILE), raw.to_string()).unwrap();
        let got = Usage::new(d.path()).get(NOW);
        assert_eq!(got.observed_at_ms, NOW);
        assert_eq!(got.windows.len(), MAX_WINDOWS);
        assert_eq!(got.windows[0], win("five_hour", MAX_PERCENT, Some(h5)));
        assert_eq!(got.windows[1], win("k0", 0.0, None));
    }
}
