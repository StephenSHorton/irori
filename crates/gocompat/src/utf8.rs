//! Go's `unicode/utf8` decoding rules.
//!
//! Go decodes one rune at a time and turns each byte of a bad sequence into
//! its own U+FFFD, width 1. Rust's `String::from_utf8_lossy` replaces a whole
//! truncated sequence with one U+FFFD, so `"\xf0\x9f\x98"` is three
//! replacement characters in Go and one in Rust. Everything in this crate that
//! walks bytes uses [`decode_rune`] instead.

/// U+FFFD, Go's `utf8.RuneError`.
pub const RUNE_ERROR: char = '\u{FFFD}';

/// Go's `utf8.DecodeRune`: the first rune of `b` and its width. An empty
/// input gives `(RUNE_ERROR, 0)`; an invalid or truncated sequence gives
/// `(RUNE_ERROR, 1)`. A real U+FFFD in the input has width 3.
pub fn decode_rune(b: &[u8]) -> (char, usize) {
    let Some(&b0) = b.first() else {
        return (RUNE_ERROR, 0);
    };
    if b0 < 0x80 {
        return (b0 as char, 1);
    }
    let (n, lo, hi) = match b0 {
        0xc2..=0xdf => (2, 0x80, 0xbf),
        0xe0 => (3, 0xa0, 0xbf),
        0xe1..=0xec | 0xee..=0xef => (3, 0x80, 0xbf),
        0xed => (3, 0x80, 0x9f),
        0xf0 => (4, 0x90, 0xbf),
        0xf1..=0xf3 => (4, 0x80, 0xbf),
        0xf4 => (4, 0x80, 0x8f),
        _ => return (RUNE_ERROR, 1),
    };
    if b.len() < n {
        // Go still checks the bytes that are there, but the answer is the
        // same: width 1.
        return (RUNE_ERROR, 1);
    }
    let b1 = b[1];
    if b1 < lo || b1 > hi {
        return (RUNE_ERROR, 1);
    }
    for &c in &b[2..n] {
        if !(0x80..=0xbf).contains(&c) {
            return (RUNE_ERROR, 1);
        }
    }
    let cp = match n {
        2 => (u32::from(b0 & 0x1f) << 6) | u32::from(b1 & 0x3f),
        3 => (u32::from(b0 & 0x0f) << 12) | (u32::from(b1 & 0x3f) << 6) | u32::from(b[2] & 0x3f),
        _ => {
            (u32::from(b0 & 0x07) << 18)
                | (u32::from(b1 & 0x3f) << 12)
                | (u32::from(b[2] & 0x3f) << 6)
                | u32::from(b[3] & 0x3f)
        }
    };
    // The ranges above exclude surrogates and values past U+10FFFF.
    (char::from_u32(cp).unwrap_or(RUNE_ERROR), n)
}

/// Iterate `b` as Go's `for _, r := range string(b)` does: every invalid
/// byte yields U+FFFD on its own. Items are `(byte offset, rune, width)`.
pub fn runes(b: &[u8]) -> Runes<'_> {
    Runes { b, i: 0 }
}

/// Iterator returned by [`runes`].
pub struct Runes<'a> {
    b: &'a [u8],
    i: usize,
}

impl Iterator for Runes<'_> {
    type Item = (usize, char, usize);

    fn next(&mut self) -> Option<Self::Item> {
        if self.i >= self.b.len() {
            return None;
        }
        let (r, w) = decode_rune(&self.b[self.i..]);
        let at = self.i;
        self.i += w;
        Some((at, r, w))
    }
}

/// Go's `string([]rune(string(b)))` for bytes: every invalid byte becomes
/// U+FFFD, one per byte.
pub fn to_valid_string(b: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(b) {
        return s.to_string();
    }
    runes(b).map(|(_, r, _)| r).collect()
}
