//! Go's string and float encoders (encode.go `appendString`,
//! `floatEncoder.encode`).

use super::Error;
use crate::utf8::{RUNE_ERROR, decode_rune};

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Go's `safeSet` / `htmlSafeSet` for one ASCII byte.
fn safe(b: u8, escape_html: bool) -> bool {
    if b < 0x20 || b == b'"' || b == b'\\' {
        return false;
    }
    !(escape_html && (b == b'<' || b == b'>' || b == b'&'))
}

/// Go's `appendString`: `src` as a quoted JSON string.
///
/// - `"` `\` `\b` `\f` `\n` `\r` `\t` get short escapes, other bytes below
///   0x20 `\u00xx` (lowercase hex), U+007F stays raw;
/// - with `escape_html`, `<` `>` `&` become `<` `>` `&`;
/// - U+2028 and U+2029 are always escaped;
/// - each invalid UTF-8 byte becomes `�`.
pub fn append_string(dst: &mut Vec<u8>, src: &[u8], escape_html: bool) {
    dst.push(b'"');
    let mut start = 0;
    let mut i = 0;
    while i < src.len() {
        let b = src[i];
        if b < 0x80 {
            if safe(b, escape_html) {
                i += 1;
                continue;
            }
            dst.extend_from_slice(&src[start..i]);
            match b {
                b'\\' | b'"' => dst.extend_from_slice(&[b'\\', b]),
                0x08 => dst.extend_from_slice(b"\\b"),
                0x0c => dst.extend_from_slice(b"\\f"),
                b'\n' => dst.extend_from_slice(b"\\n"),
                b'\r' => dst.extend_from_slice(b"\\r"),
                b'\t' => dst.extend_from_slice(b"\\t"),
                _ => {
                    dst.extend_from_slice(b"\\u00");
                    dst.push(HEX[(b >> 4) as usize]);
                    dst.push(HEX[(b & 0xf) as usize]);
                }
            }
            i += 1;
            start = i;
            continue;
        }
        let (c, size) = decode_rune(&src[i..]);
        if c == RUNE_ERROR && size == 1 {
            dst.extend_from_slice(&src[start..i]);
            dst.extend_from_slice(b"\\ufffd");
            i += 1;
            start = i;
            continue;
        }
        if c == '\u{2028}' || c == '\u{2029}' {
            dst.extend_from_slice(&src[start..i]);
            dst.extend_from_slice(b"\\u202");
            dst.push(HEX[(c as u32 & 0xf) as usize]);
            i += size;
            start = i;
            continue;
        }
        i += size;
    }
    dst.extend_from_slice(&src[start..]);
    dst.push(b'"');
}

/// [`append_string`] into a new `String` (the output is always valid UTF-8).
pub fn quote_string(src: &str, escape_html: bool) -> String {
    let mut v = Vec::with_capacity(src.len() + 2);
    append_string(&mut v, src.as_bytes(), escape_html);
    String::from_utf8(v).expect("append_string writes UTF-8")
}

/// Go's float64 encoding: `'f'` notation for `1e-6 <= |x| < 1e21` (and 0),
/// `'e'` outside it with the exponent unpadded (`1e+21`, `1e-7`). NaN and
/// the infinities are `json: unsupported value`.
pub fn format_f64(f: f64) -> Result<String, Error> {
    if !f.is_finite() {
        return Err(Error::UnsupportedValue(go_g(f)));
    }
    let abs = f.abs();
    if abs != 0.0 && !(1e-6..1e21).contains(&abs) {
        Ok(e_form(&format!("{f:e}")))
    } else {
        Ok(format!("{f}"))
    }
}

/// Go's float32 encoding: the same rule with float32 cutoffs and the
/// shortest float32 digits.
pub fn format_f32(f: f32) -> Result<String, Error> {
    if !f.is_finite() {
        return Err(Error::UnsupportedValue(go_g(f64::from(f))));
    }
    let abs = f.abs();
    if abs != 0.0 && !(1e-6f32..1e21f32).contains(&abs) {
        Ok(e_form(&format!("{f:e}")))
    } else {
        Ok(format!("{f}"))
    }
}

/// Rust `{:e}` (`1.5e-7`, `1e21`) to Go's JSON form (`1.5e-7`, `1e+21`).
fn e_form(s: &str) -> String {
    let (mant, exp) = s.split_once('e').expect("{:e} has an exponent");
    if let Some(neg) = exp.strip_prefix('-') {
        format!("{mant}e-{neg}")
    } else if exp.len() < 2 {
        format!("{mant}e+0{exp}")
    } else {
        format!("{mant}e+{exp}")
    }
}

fn go_g(f: f64) -> String {
    if f.is_nan() {
        "NaN".into()
    } else if f > 0.0 {
        "+Inf".into()
    } else {
        "-Inf".into()
    }
}
