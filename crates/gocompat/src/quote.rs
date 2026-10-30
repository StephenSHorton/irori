//! Go `strconv.Quote`, which is also `fmt`'s `%q` for strings (surface.md
//! §7.9, coord.md §7.7).
//!
//! `"` and `\` are backslashed; printable runes (Go's `unicode.IsPrint`,
//! Unicode 15 tables from Go) stay raw; `\a \b \f \n \r \t \v` get short
//! escapes; other bytes below 0x20 and 0x7F are `\xNN`; other non-printable
//! runes are `\uNNNN` or `\UNNNNNNNN`; each invalid UTF-8 byte is `\xNN`.
//! Rust's `{:?}` differs on most of these.

use crate::text::is_print;
use crate::utf8::{RUNE_ERROR, decode_rune};

const HEX: &[u8; 16] = b"0123456789abcdef";

/// `strconv.Quote(s)`.
pub fn quote(s: &str) -> String {
    quote_bytes(s.as_bytes())
}

/// `strconv.Quote(string(b))`: bytes that are not UTF-8 are allowed.
pub fn quote_bytes(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len() + 2);
    out.push('"');
    let mut i = 0;
    while i < b.len() {
        let (r, w) = decode_rune(&b[i..]);
        if w == 1 && r == RUNE_ERROR {
            out.push_str("\\x");
            out.push(HEX[(b[i] >> 4) as usize] as char);
            out.push(HEX[(b[i] & 0xf) as usize] as char);
            i += 1;
            continue;
        }
        escaped_rune(&mut out, r);
        i += w;
    }
    out.push('"');
    out
}

fn escaped_rune(out: &mut String, r: char) {
    if r == '"' || r == '\\' {
        out.push('\\');
        out.push(r);
        return;
    }
    if is_print(r) {
        out.push(r);
        return;
    }
    match r {
        '\u{7}' => out.push_str("\\a"),
        '\u{8}' => out.push_str("\\b"),
        '\u{c}' => out.push_str("\\f"),
        '\n' => out.push_str("\\n"),
        '\r' => out.push_str("\\r"),
        '\t' => out.push_str("\\t"),
        '\u{b}' => out.push_str("\\v"),
        _ => {
            let c = r as u32;
            if c < 0x20 || c == 0x7f {
                out.push_str("\\x");
                out.push(HEX[(c >> 4) as usize] as char);
                out.push(HEX[(c & 0xf) as usize] as char);
            } else if c < 0x10000 {
                out.push_str("\\u");
                for s in (0..4).rev() {
                    out.push(HEX[((c >> (s * 4)) & 0xf) as usize] as char);
                }
            } else {
                out.push_str("\\U");
                for s in (0..8).rev() {
                    out.push(HEX[((c >> (s * 4)) & 0xf) as usize] as char);
                }
            }
        }
    }
}
