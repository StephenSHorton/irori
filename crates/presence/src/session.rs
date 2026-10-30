//! Go `session.go`: one `<pid>.json` session file.

use std::io::Read;

use gocompat::json::anymap::{AnyValue, unmarshal_any};
use gocompat::time::GoTime;

use crate::localtime::local;

/// Real session files are under 1 KiB; read at most this much.
const MAX_SESSION_FILE: u64 = 64 << 10;

/// The part of a `<pid>.json` file the roster uses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Session {
    pub pid: i64,
    pub id: String,
    pub cwd: String,
    pub name: String,
    /// Who set name: "user" for /rename; other values are Claude Code's.
    pub name_source: String,
    pub status: String,
    pub kind: String,
    pub entrypoint: String,
    pub version: String,
    pub peer_features: Vec<String>,
    /// When the session's process started, in the unit Claude Code wrote
    /// it: FILETIME ticks on Windows; off Windows `ps -o lstart=` text,
    /// read as Unix seconds. 0 when absent or unreadable.
    pub proc_start: u64,
    pub started_at: GoTime,
    pub updated_at: GoTime,
}

/// Go `isSessionFile`: `<digits>.json`. The `<pid>.<hash>.key` files next
/// to them are secrets and must never match.
pub fn is_session_file(name: &str) -> bool {
    match name.strip_suffix(".json") {
        Some(stem) if !stem.is_empty() => stem.bytes().all(|b| b.is_ascii_digit()),
        _ => false,
    }
}

fn str_of(v: Option<&AnyValue>) -> String {
    match v {
        Some(AnyValue::String(s)) => s.clone(),
        _ => String::new(),
    }
}

/// Go `strconv.ParseFloat(s, 64)` as `num` uses it (errors read as 0).
fn parse_float(s: &str) -> f64 {
    let t = s.strip_prefix(['+', '-']).unwrap_or(s);
    // Rust accepts "infinity"/"nan" spellings Go also accepts, and both
    // refuse empty text; hex floats and underscores are Go-only and absent
    // from Claude's files.
    if t.is_empty() {
        return 0.0;
    }
    s.parse().unwrap_or(0.0)
}

fn num(v: Option<&AnyValue>) -> f64 {
    match v {
        Some(AnyValue::Number(n)) => *n,
        Some(AnyValue::String(s)) => parse_float(s),
        _ => 0.0,
    }
}

fn strs(v: Option<&AnyValue>) -> Vec<String> {
    match v {
        Some(AnyValue::Array(items)) => items
            .iter()
            .filter_map(|x| match x {
                AnyValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Go `unixMilli`: zero for ms <= 0, else that instant in the local zone.
fn unix_milli(ms: f64) -> GoTime {
    if ms <= 0.0 || ms.is_nan() {
        return GoTime::ZERO;
    }
    local(GoTime::unix_milli(ms as i64))
}

/// Go `readSession`: parses one session file; None when it has no usable
/// pid or session id. Invalid JSON is unreadable and never repaired: in a
/// Windows path `\t` or `\n` is path text, so any guess would corrupt cwd.
pub fn read_session(r: impl Read) -> Option<Session> {
    let mut raw = Vec::new();
    r.take(MAX_SESSION_FILE).read_to_end(&mut raw).ok()?;
    let AnyValue::Object(ref m) = unmarshal_any(&raw).ok()? else {
        return None;
    };
    let mut s = Session {
        pid: num(m.get("pid")) as i64,
        id: str_of(m.get("sessionId")),
        cwd: str_of(m.get("cwd")),
        name: str_of(m.get("name")),
        name_source: str_of(m.get("nameSource")),
        status: str_of(m.get("status")),
        kind: str_of(m.get("kind")),
        entrypoint: str_of(m.get("entrypoint")),
        version: str_of(m.get("version")),
        peer_features: strs(m.get("peerFeatures")),
        started_at: unix_milli(num(m.get("startedAt"))),
        ..Session::default()
    };
    match m.get("procStart") {
        Some(AnyValue::String(v)) => {
            // strconv.ParseUint(v, 10, 64): digits only, no sign.
            let digits = !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit());
            if let Some(n) = digits.then(|| v.parse::<u64>().ok()).flatten() {
                s.proc_start = n;
            } else if let Some(t) = parse_lstart(v) {
                s.proc_start = t.secs as u64;
            }
        }
        Some(AnyValue::Number(v)) if *v > 0.0 => s.proc_start = *v as u64,
        _ => {}
    }
    let updated = num(m.get("updatedAt")).max(num(m.get("statusUpdatedAt")));
    s.updated_at = unix_milli(updated);
    if s.pid <= 0 || s.id.is_empty() {
        return None;
    }
    Some(s)
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

fn days_in(m: u32, y: i64) -> u32 {
    match m {
        2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Go `parseLstart`: `ps -o lstart=` text ("Mon Jan _2 15:04:05 2006",
/// runs of spaces collapsed first) read as UTC; years before 2000 refused.
pub fn parse_lstart(v: &str) -> Option<GoTime> {
    let f: Vec<&str> = v
        .split(gocompat::text::is_space)
        .filter(|s| !s.is_empty())
        .collect();
    let [wd, mon, day, hms, year] = f.as_slice() else {
        return None;
    };
    // Go matches names ignoring ASCII case, and checks the weekday's
    // spelling, not that it matches the date.
    if !DAYS.iter().any(|x| x.eq_ignore_ascii_case(wd)) {
        return None;
    }
    let m = MONTHS.iter().position(|x| x.eq_ignore_ascii_case(mon))? as u32 + 1;
    let digits = |s: &str, lens: &[usize]| -> Option<u32> {
        (lens.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())?
    };
    // "15" reads one or two digits; "04" and "05" exactly two.
    let two = |s: &str| digits(s, &[2]);
    // `_2`: one digit, or two.
    let d: u32 = match day.len() {
        1 | 2 if day.bytes().all(|b| b.is_ascii_digit()) => day.parse().ok()?,
        _ => return None,
    };
    let parts: Vec<&str> = hms.split(':').collect();
    let [hh, mm, ss] = parts.as_slice() else {
        return None;
    };
    let (hh, mm, ss) = (digits(hh, &[1, 2])?, two(mm)?, two(ss)?);
    if year.len() != 4 || !year.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let y: i64 = year.parse().ok()?;
    if hh > 23 || mm > 59 || ss > 59 || d == 0 || d > days_in(m, y) || y < 2000 {
        return None;
    }
    // Days from the epoch (proleptic Gregorian).
    let (yy, mm_) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * i64::from(mm_) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(GoTime::unix(
        days * 86_400 + i64::from(hh) * 3600 + i64::from(mm) * 60 + i64::from(ss),
        0,
    ))
}
