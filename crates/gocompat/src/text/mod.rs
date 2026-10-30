//! Go's `unicode` predicates and case mappings, and the `strings` functions
//! built on them (capture.md §7.2, §7.8; coord.md §7.8, §7.18).
//!
//! The tables in [`tables`] are generated from Go's own `unicode` package
//! (Unicode 15.0.0 in Go 1.26) by `tools/goldens/export.sh`. Rust's `char`
//! methods follow a newer Unicode and use full (not simple) case mappings,
//! so they are not used here.
//!
//! - [`to_lower`] / [`to_upper`] are Go's one-rune simple mappings, and
//!   [`to_lower_str`] is `strings.ToLower`: no mapping changes the length,
//!   so `İ` (U+0130) lowers to a plain `i`, where Rust's `to_lowercase`
//!   gives `i` plus U+0307.
//! - [`simple_fold`] walks Go's fold orbits (`K` → `k` → U+212A → `K`).
//! - [`equal_fold`] is `strings.EqualFold`, byte for byte of Go's loop.

#[rustfmt::skip]
pub mod tables;

use crate::utf8::{decode_rune, runes};

pub use tables::UNICODE_VERSION;

fn in_ranges(t: &[(u32, u32)], r: u32) -> bool {
    let i = t.partition_point(|&(lo, _)| lo <= r);
    i > 0 && r <= t[i - 1].1
}

fn map_case(t: &[(u32, u32, u32, i32)], r: char) -> char {
    let c = r as u32;
    let i = t.partition_point(|&(lo, ..)| lo <= c);
    if i == 0 {
        return r;
    }
    let (lo, hi, stride, delta) = t[i - 1];
    if c > hi || !(c - lo).is_multiple_of(stride) {
        return r;
    }
    char::from_u32((i64::from(c) + i64::from(delta)) as u32).unwrap_or(r)
}

/// `unicode.IsPrint`: letters, marks, numbers, punctuation, symbols and the
/// ASCII space (not U+00A0 or other spaces).
pub fn is_print(r: char) -> bool {
    in_ranges(tables::PRINT, r as u32)
}
/// `unicode.IsGraphic`: `IsPrint` plus the Zs spaces.
pub fn is_graphic(r: char) -> bool {
    in_ranges(tables::GRAPHIC, r as u32)
}
/// `unicode.IsLetter` (category L).
pub fn is_letter(r: char) -> bool {
    in_ranges(tables::LETTER, r as u32)
}
/// `unicode.IsDigit` (category Nd).
pub fn is_digit(r: char) -> bool {
    in_ranges(tables::DIGIT, r as u32)
}
/// `unicode.IsNumber` (category N).
pub fn is_number(r: char) -> bool {
    in_ranges(tables::NUMBER, r as u32)
}
/// `unicode.IsSpace`: White_Space, which includes U+0085 and U+00A0.
pub fn is_space(r: char) -> bool {
    in_ranges(tables::SPACE, r as u32)
}
/// `unicode.IsControl`: C0, DEL and C1 only (Go checks Latin-1 alone).
pub fn is_control(r: char) -> bool {
    in_ranges(tables::CONTROL, r as u32)
}
/// `unicode.IsUpper`.
pub fn is_upper(r: char) -> bool {
    in_ranges(tables::UPPER, r as u32)
}
/// `unicode.IsLower`.
pub fn is_lower(r: char) -> bool {
    in_ranges(tables::LOWER, r as u32)
}
/// `unicode.IsPunct` (category P).
pub fn is_punct(r: char) -> bool {
    in_ranges(tables::PUNCT, r as u32)
}
/// `unicode.IsMark` (category M).
pub fn is_mark(r: char) -> bool {
    in_ranges(tables::MARK, r as u32)
}
/// `unicode.IsSymbol` (category S).
pub fn is_symbol(r: char) -> bool {
    in_ranges(tables::SYMBOL, r as u32)
}
/// `unicode.Is(unicode.Cf, r)`: format characters (U+200B, U+FEFF, ...).
pub fn is_cf(r: char) -> bool {
    in_ranges(tables::CF, r as u32)
}

/// `unicode.ToLower`: Go's one-rune simple lowercase.
pub fn to_lower(r: char) -> char {
    if r.is_ascii() {
        return r.to_ascii_lowercase();
    }
    map_case(tables::TO_LOWER, r)
}
/// `unicode.ToUpper`.
pub fn to_upper(r: char) -> char {
    if r.is_ascii() {
        return r.to_ascii_uppercase();
    }
    map_case(tables::TO_UPPER, r)
}
/// `unicode.ToTitle`.
pub fn to_title(r: char) -> char {
    map_case(tables::TO_TITLE, r)
}
/// `unicode.SimpleFold`: the next rune in `r`'s fold orbit, wrapping to the
/// smallest; `r` itself when it folds to nothing else.
pub fn simple_fold(r: char) -> char {
    map_case(tables::SIMPLE_FOLD, r)
}

fn map_bytes(b: &[u8], f: fn(char) -> char) -> String {
    runes(b).map(|(_, r, _)| f(r)).collect()
}

/// `strings.ToLower`: per-rune simple lowercase; each invalid UTF-8 byte
/// becomes U+FFFD.
pub fn to_lower_str(s: &str) -> String {
    s.chars().map(to_lower).collect()
}
/// `strings.ToLower` over bytes that may not be UTF-8.
pub fn to_lower_bytes(b: &[u8]) -> String {
    map_bytes(b, to_lower)
}
/// `strings.ToUpper`.
pub fn to_upper_str(s: &str) -> String {
    s.chars().map(to_upper).collect()
}
/// `strings.ToUpper` over bytes that may not be UTF-8.
pub fn to_upper_bytes(b: &[u8]) -> String {
    map_bytes(b, to_upper)
}

/// `strings.EqualFold(s, t)`.
pub fn equal_fold(s: &str, t: &str) -> bool {
    equal_fold_bytes(s.as_bytes(), t.as_bytes())
}

/// `strings.EqualFold` over bytes: invalid UTF-8 bytes compare as U+FFFD.
pub fn equal_fold_bytes(s: &[u8], t: &[u8]) -> bool {
    // ASCII fast path, as Go's loop.
    let n = s.len().min(t.len());
    let mut i = 0;
    while i < n {
        let (sr, tr) = (s[i], t[i]);
        if (sr | tr) >= 0x80 {
            return equal_fold_unicode(&s[i..], &t[i..]);
        }
        if sr != tr {
            let (lo, hi) = if sr < tr { (sr, tr) } else { (tr, sr) };
            if !(lo.is_ascii_uppercase() && hi == lo + (b'a' - b'A')) {
                return false;
            }
        }
        i += 1;
    }
    s.len() == t.len()
}

fn equal_fold_unicode(s: &[u8], mut t: &[u8]) -> bool {
    for (_, sr, _) in runes(s) {
        if t.is_empty() {
            return false;
        }
        let (tr, size) = decode_rune(t);
        t = &t[size..];
        if tr == sr {
            continue;
        }
        let (sr, tr) = if tr < sr { (tr, sr) } else { (sr, tr) };
        if (tr as u32) < 0x80 {
            if sr.is_ascii_uppercase() && tr as u32 == sr as u32 + 32 {
                continue;
            }
            return false;
        }
        let mut r = simple_fold(sr);
        while r != sr && r < tr {
            r = simple_fold(r);
        }
        if r == tr {
            continue;
        }
        return false;
    }
    t.is_empty()
}

/// encoding/json's `foldName`: a key such that `fold_name(a) == fold_name(b)`
/// exactly when `bytes.EqualFold(a, b)`. ASCII letters are upper-cased, every
/// other rune replaced by the smallest rune of its fold orbit.
pub fn fold_name(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c < 0x80 {
            out.push(c.to_ascii_uppercase());
            i += 1;
            continue;
        }
        let (r, n) = decode_rune(&b[i..]);
        let mut buf = [0u8; 4];
        out.extend_from_slice(fold_rune(r).encode_utf8(&mut buf).as_bytes());
        i += n;
    }
    out
}

/// The smallest rune in `r`'s fold orbit (encoding/json `foldRune`).
fn fold_rune(mut r: char) -> char {
    loop {
        let r2 = simple_fold(r);
        if r2 <= r {
            return r2;
        }
        r = r2;
    }
}
