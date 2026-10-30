//! Go `time.Time` as JSON and RFC 3339 text (identity.md §3.3, coord.md
//! §7.2, capture.md §7.6, surface.md §7.8(e)).
//!
//! A [`GoTime`] is an instant (Unix seconds and nanoseconds) plus the UTC
//! offset it is shown in, which is what Go keeps and prints. Formatting is
//! RFC3339Nano with trailing fraction zeros trimmed and the fraction left out
//! when zero, `Z` for offset 0 and `±hh:mm` otherwise. Marshal refuses years
//! outside 0..=9999 and offsets of 24 hours or more, as Go's `MarshalJSON`
//! does. Parsing is `time.Time.UnmarshalJSON`'s: Go's strict RFC 3339 fast
//! path, then `time.Parse(time.RFC3339, s)`, whose extra leniency (a
//! one-digit hour, a `,` before the fraction, offsets up to `+24:00`) is
//! reproduced.
//!
//! Go's location names are not kept: a parsed time whose offset matches the
//! local zone gets `time.Local` in Go, and the same offset here; both print
//! the same.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Seconds from 0001-01-01T00:00:00Z (Go's zero time) to the Unix epoch.
const ZERO_UNIX: i64 = -62_135_596_800;

/// An instant and the offset it is shown in.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct GoTime {
    /// Unix seconds.
    pub secs: i64,
    /// Nanoseconds within the second, `0..1_000_000_000`.
    pub nanos: u32,
    /// Seconds east of UTC.
    pub offset: i32,
}

impl fmt::Debug for GoTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GoTime({})", self.format_rfc3339_nano())
    }
}

impl fmt::Display for GoTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format_rfc3339_nano())
    }
}

impl Default for GoTime {
    fn default() -> Self {
        GoTime::ZERO
    }
}

/// Days from 1970-01-01 to the civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of a day count from 1970-01-01.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn is_leap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

fn days_in(m: u32, y: i64) -> u32 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Go's `appendInt(b, x, width)`: zero-padded, with a leading `-` for
/// negatives.
fn push_int(out: &mut String, x: i64, width: usize) {
    if x < 0 {
        out.push('-');
    }
    let digits = x.unsigned_abs().to_string();
    for _ in digits.len()..width {
        out.push('0');
    }
    out.push_str(&digits);
}

/// Why [`GoTime::marshal_json`] refused a time.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimeError {
    #[error("Time.MarshalJSON: year outside of range [0,9999]")]
    YearRange,
    #[error("Time.MarshalJSON: timezone hour outside of range [0,23]")]
    ZoneRange,
}

impl GoTime {
    /// Go's zero `time.Time`, `0001-01-01T00:00:00Z`.
    pub const ZERO: GoTime = GoTime {
        secs: ZERO_UNIX,
        nanos: 0,
        offset: 0,
    };

    /// `time.Unix(secs, nanos).UTC()`; `nanos` outside a second is carried.
    pub fn unix(secs: i64, nanos: i64) -> GoTime {
        let s = secs + nanos.div_euclid(1_000_000_000);
        let n = nanos.rem_euclid(1_000_000_000) as u32;
        GoTime {
            secs: s,
            nanos: n,
            offset: 0,
        }
    }

    /// `time.UnixMilli(ms).UTC()`.
    pub fn unix_milli(ms: i64) -> GoTime {
        GoTime::unix(ms.div_euclid(1000), ms.rem_euclid(1000) * 1_000_000)
    }

    /// The same instant shown at `offset` seconds east of UTC (`t.In(zone)`).
    pub fn with_offset(self, offset: i32) -> GoTime {
        GoTime { offset, ..self }
    }

    /// `t.UTC()`.
    pub fn utc(self) -> GoTime {
        self.with_offset(0)
    }

    /// A `SystemTime` as a UTC `GoTime` (`time.Now().UTC()` for the clock).
    pub fn from_system(t: SystemTime) -> GoTime {
        match t.duration_since(UNIX_EPOCH) {
            Ok(d) => GoTime::unix(d.as_secs() as i64, i64::from(d.subsec_nanos())),
            Err(e) => {
                let d = e.duration();
                GoTime::unix(-(d.as_secs() as i64), -i64::from(d.subsec_nanos()))
            }
        }
    }

    /// The instant as a `SystemTime`.
    pub fn to_system(self) -> SystemTime {
        if self.secs >= 0 {
            UNIX_EPOCH + Duration::new(self.secs as u64, self.nanos)
        } else {
            UNIX_EPOCH - Duration::from_secs(self.secs.unsigned_abs())
                + Duration::from_nanos(u64::from(self.nanos))
        }
    }

    /// Go `t.IsZero()`: the instant is the zero time, whatever the offset.
    pub fn is_zero(&self) -> bool {
        self.secs == ZERO_UNIX && self.nanos == 0
    }

    /// Go `t.Equal(u)`: same instant.
    pub fn equal(&self, other: &GoTime) -> bool {
        self.secs == other.secs && self.nanos == other.nanos
    }

    /// Go `t.UnixMilli()`.
    pub fn unix_milli_value(&self) -> i64 {
        self.secs * 1000 + i64::from(self.nanos / 1_000_000)
    }

    /// Go `t.UnixNano()` (wraps outside int64 range as Go does).
    pub fn unix_nano(&self) -> i64 {
        self.secs
            .wrapping_mul(1_000_000_000)
            .wrapping_add(i64::from(self.nanos))
    }

    /// The wall-clock fields at the time's offset:
    /// `(year, month, day, hour, minute, second)`.
    pub fn civil(&self) -> (i64, u32, u32, u32, u32, u32) {
        let local = self.secs + i64::from(self.offset);
        let days = local.div_euclid(86_400);
        let sod = local.rem_euclid(86_400);
        let (y, m, d) = civil_from_days(days);
        (
            y,
            m,
            d,
            (sod / 3600) as u32,
            (sod / 60 % 60) as u32,
            (sod % 60) as u32,
        )
    }

    fn push_date_time(&self, out: &mut String, sep: char) {
        let (y, mo, d, h, mi, s) = self.civil();
        push_int(out, y, 4);
        out.push('-');
        push_int(out, i64::from(mo), 2);
        out.push('-');
        push_int(out, i64::from(d), 2);
        out.push(sep);
        push_int(out, i64::from(h), 2);
        out.push(':');
        push_int(out, i64::from(mi), 2);
        out.push(':');
        push_int(out, i64::from(s), 2);
    }

    fn push_zone(&self, out: &mut String) {
        if self.offset == 0 {
            out.push('Z');
            return;
        }
        // Go divides toward zero, so an offset under a minute prints +00:00.
        let mut zone = self.offset / 60;
        if zone < 0 {
            out.push('-');
            zone = -zone;
        } else {
            out.push('+');
        }
        push_int(out, i64::from(zone / 60), 2);
        out.push(':');
        push_int(out, i64::from(zone % 60), 2);
    }

    /// `t.Format(time.RFC3339Nano)`.
    pub fn format_rfc3339_nano(&self) -> String {
        let mut out = String::with_capacity(35);
        self.push_date_time(&mut out, 'T');
        if self.nanos != 0 {
            let frac = format!("{:09}", self.nanos);
            out.push('.');
            out.push_str(frac.trim_end_matches('0'));
        }
        self.push_zone(&mut out);
        out
    }

    /// `t.Format(time.RFC3339)` (whole seconds).
    pub fn format_rfc3339(&self) -> String {
        let mut out = String::with_capacity(25);
        self.push_date_time(&mut out, 'T');
        self.push_zone(&mut out);
        out
    }

    /// `t.Format("2006-01-02 15:04:05")` at the time's offset.
    pub fn format_date_time(&self) -> String {
        let mut out = String::with_capacity(19);
        self.push_date_time(&mut out, ' ');
        out
    }

    /// `t.Format("01-02 15:04:05")` at the time's offset.
    pub fn format_short(&self) -> String {
        let full = self.format_date_time();
        // "YYYY-MM-DD hh:mm:ss" -> "MM-DD hh:mm:ss"; a year wider than four
        // digits still drops only through the first '-'.
        match full.find('-') {
            Some(i) if i > 0 => full[i + 1..].to_string(),
            _ => full,
        }
    }

    /// The JSON string Go's `MarshalJSON` writes, without the quotes.
    pub fn marshal_text(&self) -> Result<String, TimeError> {
        let s = self.format_rfc3339_nano();
        let b = s.as_bytes();
        if b.get(4) != Some(&b'-') {
            return Err(TimeError::YearRange);
        }
        if !s.ends_with('Z') {
            let c = b[b.len() - 6];
            let hh = (b[b.len() - 5] - b'0') * 10 + (b[b.len() - 4] - b'0');
            if c.is_ascii_digit() || hh >= 24 {
                return Err(TimeError::ZoneRange);
            }
        }
        Ok(s)
    }

    /// Go `t.MarshalJSON()`: the quoted RFC3339Nano text.
    pub fn marshal_json(&self) -> Result<String, TimeError> {
        Ok(format!("\"{}\"", self.marshal_text()?))
    }

    /// `time.Time.UnmarshalJSON` of the string literal's text (no quotes).
    /// `None` where Go returns an error.
    pub fn parse_rfc3339(s: &str) -> Option<GoTime> {
        parse_rfc3339(s.as_bytes())
    }
}

impl serde::Serialize for GoTime {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let t = self.marshal_text().map_err(serde::ser::Error::custom)?;
        s.serialize_str(&t)
    }
}

impl<'de> serde::Deserialize<'de> for GoTime {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str> as serde::Deserialize>::deserialize(d)?;
        GoTime::parse_rfc3339(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("parsing time {s:?} as RFC 3339")))
    }
}

/// Go `getnum(s, fixed)`: one or two leading digits (exactly two if fixed).
fn getnum(s: &[u8], fixed: bool) -> Option<(i64, &[u8])> {
    let d0 = *s.first()?;
    if !d0.is_ascii_digit() {
        return None;
    }
    match s.get(1) {
        Some(d1) if d1.is_ascii_digit() => {
            Some((i64::from(d0 - b'0') * 10 + i64::from(d1 - b'0'), &s[2..]))
        }
        _ if fixed => None,
        _ => Some((i64::from(d0 - b'0'), &s[1..])),
    }
}

fn skip(s: &[u8], lit: u8) -> Option<&[u8]> {
    if s.first() == Some(&lit) {
        Some(&s[1..])
    } else {
        None
    }
}

/// Go `parseNanoseconds(value, nbytes)`: value[0] is the separator.
fn parse_nanos(digits: &[u8]) -> Option<u32> {
    // Go keeps at most nine digits (value[:10] including the separator).
    let d = &digits[..digits.len().min(9)];
    let mut ns: u64 = 0;
    for &c in d {
        ns = ns * 10 + u64::from(c - b'0');
    }
    for _ in d.len()..9 {
        ns *= 10;
    }
    u32::try_from(ns).ok()
}

/// `time.Parse(time.RFC3339, s)` with Go's parse rules for that layout.
/// Go's strict fast path accepts a subset of this, with the same results.
fn parse_rfc3339(s: &[u8]) -> Option<GoTime> {
    // "2006": four bytes, the first a digit, all digits (atoi).
    if s.len() < 4 || !s[0].is_ascii_digit() || !s[..4].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let year = s[..4]
        .iter()
        .fold(0i64, |a, c| a * 10 + i64::from(c - b'0'));
    let s = skip(&s[4..], b'-')?;
    let (month, s) = getnum(s, true)?;
    if !(1..=12).contains(&month) {
        return None;
    }
    let s = skip(s, b'-')?;
    let (day, s) = getnum(s, true)?;
    let s = skip(s, b'T')?;
    let (hour, s) = getnum(s, false)?;
    if !(0..24).contains(&hour) {
        return None;
    }
    let s = skip(s, b':')?;
    let (min, s) = getnum(s, true)?;
    if !(0..60).contains(&min) {
        return None;
    }
    let s = skip(s, b':')?;
    let (sec, mut s) = getnum(s, true)?;
    if !(0..60).contains(&sec) {
        return None;
    }
    let mut nanos = 0u32;
    if s.len() >= 2 && (s[0] == b'.' || s[0] == b',') && s[1].is_ascii_digit() {
        let n = 1 + s[1..].iter().take_while(|c| c.is_ascii_digit()).count();
        nanos = parse_nanos(&s[1..n])?;
        s = &s[n..];
    }
    // "Z07:00"
    let offset: i64;
    if s.first() == Some(&b'Z') {
        s = &s[1..];
        offset = 0;
    } else {
        if s.len() < 6 || s[3] != b':' {
            return None;
        }
        let (hr, _) = getnum(&s[1..3], true)?;
        let (mm, _) = getnum(&s[4..6], true)?;
        if hr > 24 || mm > 60 {
            return None;
        }
        let mut off = (hr * 60 + mm) * 60;
        match s[0] {
            b'+' => {}
            b'-' => off = -off,
            _ => return None,
        }
        offset = off;
        s = &s[6..];
    }
    if !s.is_empty() {
        return None;
    }
    if day < 1 || day > i64::from(days_in(month as u32, year)) {
        return None;
    }
    let days = days_from_civil(year, month as u32, day as u32);
    let secs = days * 86_400 + hour * 3600 + min * 60 + sec - offset;
    Some(GoTime {
        secs,
        nanos,
        offset: offset as i32,
    })
}
