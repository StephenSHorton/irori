//! A decoded JSON tree with Go's decoding rules, shared by
//! [`canonical`](super::canonical), [`lenient`](super::lenient) and
//! [`anymap`](super::anymap).
//!
//! - Input is checked with Go's scanner first, so whatever `json.Valid`
//!   refuses is refused here, with Go's message.
//! - Strings are unquoted the way Go's `unquote` does it: each invalid UTF-8
//!   byte becomes U+FFFD, and a `\u` surrogate that is not half of a valid
//!   pair becomes U+FFFD.
//! - Objects keep every member in input order, duplicates included; each
//!   consumer applies Go's "last one wins" rule its own way.
//! - Numbers keep their literal text.
//! - Parsing and dropping never recurse, so Go's full 10000-level depth is
//!   safe on any thread.
//! - [`write_compact`] (rewriting a golden file) does recurse: it is for
//!   files the repo holds, not for untrusted input.

use super::Error;
use super::scanner::check_valid;
use crate::utf8::{RUNE_ERROR, decode_rune};

/// One JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// The number's literal text, as written.
    Number(String),
    String(JsonStr),
    Array(Vec<Value>),
    /// Members in input order, duplicates kept.
    Object(Vec<(JsonStr, Value)>),
}

/// A decoded string, plus whether its literal held any backslash escape.
/// Go's `time.Time.UnmarshalJSON` reads the literal without unescaping it,
/// so a time spelled with an escape is refused there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonStr {
    pub text: String,
    pub escaped: bool,
    /// The literal's bytes between the quotes, before unescaping.
    pub raw: Vec<u8>,
}

impl Drop for Value {
    fn drop(&mut self) {
        // Iterative drop: a deeply nested tree would otherwise recurse once
        // per level.
        let mut stack: Vec<Value> = Vec::new();
        match self {
            Value::Array(v) => stack.append(v),
            Value::Object(m) => stack.extend(m.drain(..).map(|(_, v)| v)),
            _ => return,
        }
        while let Some(mut v) = stack.pop() {
            match &mut v {
                Value::Array(a) => stack.append(a),
                Value::Object(m) => stack.extend(m.drain(..).map(|(_, v)| v)),
                _ => {}
            }
        }
    }
}

impl Value {
    /// The kind name used in Go's `UnmarshalTypeError` texts.
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }
}

fn getu4(s: &[u8]) -> Option<u32> {
    if s.len() < 6 || s[0] != b'\\' || s[1] != b'u' {
        return None;
    }
    let h = std::str::from_utf8(&s[2..6]).ok()?;
    u32::from_str_radix(h, 16).ok()
}

/// Go's `unquoteBytes` over the bytes between the quotes of a string literal
/// that the scanner already accepted.
pub(crate) fn unquote(s: &[u8]) -> String {
    let mut out = String::with_capacity(s.len());
    let mut r = 0;
    while r < s.len() {
        let c = s[r];
        if c == b'\\' {
            r += 1;
            match s.get(r).copied().unwrap_or(0) {
                b'"' => out.push('"'),
                b'\\' => out.push('\\'),
                b'/' => out.push('/'),
                b'\'' => out.push('\''),
                b'b' => out.push('\u{8}'),
                b'f' => out.push('\u{c}'),
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'u' => {
                    let rr = getu4(&s[r - 1..]).unwrap_or(0xfffd);
                    r += 4;
                    if (0xd800..0xe000).contains(&rr) {
                        if let Some(rr1) = getu4(&s[r + 1..])
                            && (0xd800..0xdc00).contains(&rr)
                            && (0xdc00..0xe000).contains(&rr1)
                        {
                            let cp = 0x10000 + ((rr - 0xd800) << 10) + (rr1 - 0xdc00);
                            out.push(char::from_u32(cp).unwrap_or(RUNE_ERROR));
                            // Past this escape's last digit and the whole second one.
                            r += 7;
                            continue;
                        }
                        out.push(RUNE_ERROR);
                    } else {
                        out.push(char::from_u32(rr).unwrap_or(RUNE_ERROR));
                    }
                }
                _ => {}
            }
            r += 1;
        } else if c < 0x80 {
            out.push(c as char);
            r += 1;
        } else {
            let (ch, w) = decode_rune(&s[r..]);
            out.push(ch);
            r += w.max(1);
        }
    }
    out
}

/// Parse one JSON value with Go's rules. Errors are Go's syntax errors.
pub fn parse(data: &[u8]) -> Result<Value, Error> {
    check_valid(data)?;
    Ok(Parser { b: data, i: 0 }.run())
}

enum Frame {
    Array(Vec<Value>),
    Object(Vec<(JsonStr, Value)>, Option<JsonStr>),
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\r' | b'\n') {
            self.i += 1;
        }
    }

    fn string_lit(&mut self) -> JsonStr {
        // At the opening quote of a literal the scanner accepted.
        let start = self.i + 1;
        let mut j = start;
        let mut escaped = false;
        while self.b[j] != b'"' {
            if self.b[j] == b'\\' {
                escaped = true;
                j += 2;
            } else {
                j += 1;
            }
        }
        self.i = j + 1;
        let raw = &self.b[start..j];
        JsonStr {
            text: unquote(raw),
            escaped,
            raw: raw.to_vec(),
        }
    }

    fn scalar(&mut self) -> Value {
        match self.b[self.i] {
            b'"' => Value::String(self.string_lit()),
            b't' => {
                self.i += 4;
                Value::Bool(true)
            }
            b'f' => {
                self.i += 5;
                Value::Bool(false)
            }
            b'n' => {
                self.i += 4;
                Value::Null
            }
            _ => {
                let start = self.i;
                while self.i < self.b.len()
                    && matches!(
                        self.b[self.i],
                        b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E'
                    )
                {
                    self.i += 1;
                }
                Value::Number(String::from_utf8_lossy(&self.b[start..self.i]).into_owned())
            }
        }
    }

    /// The input is known valid, so this only has to build the tree.
    fn run(mut self) -> Value {
        let mut stack: Vec<Frame> = Vec::new();
        loop {
            self.ws();
            // Start a value.
            let mut done = match self.b[self.i] {
                b'[' => {
                    self.i += 1;
                    self.ws();
                    if self.b[self.i] == b']' {
                        self.i += 1;
                        Some(Value::Array(Vec::new()))
                    } else {
                        stack.push(Frame::Array(Vec::new()));
                        None
                    }
                }
                b'{' => {
                    self.i += 1;
                    self.ws();
                    if self.b[self.i] == b'}' {
                        self.i += 1;
                        Some(Value::Object(Vec::new()))
                    } else {
                        let k = self.string_lit();
                        self.ws();
                        self.i += 1; // ':'
                        stack.push(Frame::Object(Vec::new(), Some(k)));
                        None
                    }
                }
                _ => Some(self.scalar()),
            };
            // Close finished values into their parents.
            while let Some(v) = done.take() {
                let Some(top) = stack.last_mut() else {
                    return v;
                };
                self.ws();
                match top {
                    Frame::Array(items) => {
                        items.push(v);
                        if self.b[self.i] == b',' {
                            self.i += 1;
                        } else {
                            self.i += 1; // ']'
                            let Some(Frame::Array(items)) = stack.pop() else {
                                unreachable!()
                            };
                            done = Some(Value::Array(items));
                        }
                    }
                    Frame::Object(members, key) => {
                        members.push((key.take().expect("object key"), v));
                        if self.b[self.i] == b',' {
                            self.i += 1;
                            self.ws();
                            let k = self.string_lit();
                            self.ws();
                            self.i += 1; // ':'
                            *key = Some(k);
                        } else {
                            self.i += 1; // '}'
                            let Some(Frame::Object(members, _)) = stack.pop() else {
                                unreachable!()
                            };
                            done = Some(Value::Object(members));
                        }
                    }
                }
            }
        }
    }
}

impl JsonStr {
    /// A string as Go's `json.Marshal` writes it (HTML escaping on).
    pub fn new(text: &str) -> JsonStr {
        JsonStr::new_with(text, true)
    }

    /// A string as Go writes it, with HTML escaping (`<>&` as `<`...)
    /// on or off (`Encoder.SetEscapeHTML(false)`).
    pub fn new_with(text: &str, escape_html: bool) -> JsonStr {
        let q = super::encode::quote_string(text, escape_html);
        let raw = q.as_bytes()[1..q.len() - 1].to_vec();
        JsonStr {
            text: text.to_string(),
            escaped: raw.contains(&b'\\'),
            raw,
        }
    }
}

impl Value {
    /// A string value as Go writes it.
    pub fn string(text: &str) -> Value {
        Value::String(JsonStr::new(text))
    }

    /// [`string`](Value::string) with HTML escaping on or off.
    pub fn string_with(text: &str, escape_html: bool) -> Value {
        Value::String(JsonStr::new_with(text, escape_html))
    }

    /// The string's text, if this is a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(&s.text),
            _ => None,
        }
    }

    /// An object member by key, the last one when the key repeats (Go's
    /// "last one wins").
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(m) => m.iter().rev().find(|(k, _)| k.text == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// [`get`](Value::get), mutable.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        match self {
            Value::Object(m) => m
                .iter_mut()
                .rev()
                .find(|(k, _)| k.text == key)
                .map(|(_, v)| v),
            _ => None,
        }
    }
}

/// Writes a parsed tree back as compact JSON, keeping member order,
/// duplicates and each literal's own bytes, so `write_compact(&parse(b)?)` is
/// `compact(b)`. For rewriting part of a file Go wrote (a golden) while the
/// rest stays byte for byte. Recursive: for files, not untrusted input.
pub fn write_compact(v: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write(&mut out, v);
    out
}

fn write(out: &mut Vec<u8>, v: &Value) {
    match v {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Number(n) => out.extend_from_slice(n.as_bytes()),
        Value::String(s) => write_str(out, s),
        Value::Array(a) => {
            out.push(b'[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write(out, x);
            }
            out.push(b']');
        }
        Value::Object(m) => {
            out.push(b'{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_str(out, k);
                out.push(b':');
                write(out, x);
            }
            out.push(b'}');
        }
    }
}

fn write_str(out: &mut Vec<u8>, s: &JsonStr) {
    out.push(b'"');
    out.extend_from_slice(&s.raw);
    out.push(b'"');
}

/// Rewrites a JSON file Go wrote with `json.MarshalIndent(v, "", "  ")`:
/// `edit` changes the parsed tree, and the file is written back indented
/// the same way, with its trailing newline and its line endings (CRLF in a
/// Windows checkout) as they were. Untouched parts keep their bytes.
/// Returns whether the file changed.
pub fn rewrite_indented(
    path: &std::path::Path,
    edit: impl FnOnce(&mut Value),
) -> std::io::Result<bool> {
    let old = std::fs::read(path)?;
    let bad = |e: Error| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string());
    let mut v = parse(&old).map_err(bad)?;
    edit(&mut v);
    let mut new = super::scanner::indent(&write_compact(&v), "", "  ").map_err(bad)?;
    if old.ends_with(b"\n") {
        new.push(b'\n');
    }
    if old.windows(2).any(|w| w == b"\r\n") {
        let mut o = Vec::with_capacity(new.len() + new.len() / 16);
        for &c in &new {
            if c == b'\n' {
                o.push(b'\r');
            }
            o.push(c);
        }
        new = o;
    }
    if new == old {
        return Ok(false);
    }
    std::fs::write(path, new)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_keeps_order_and_literals() {
        let src = br#"{"b":1,"a":"x\u003cy\/","b":[true,null,{}],"n":1.50e3}"#;
        assert_eq!(write_compact(&parse(src).unwrap()), src.to_vec());
    }

    #[test]
    fn new_strings_escape_as_go_does() {
        let mut v = parse(br#"{"k":"old"}"#).unwrap();
        *v.get_mut("k").unwrap() = Value::string("<a & \"b\">\n");
        assert_eq!(
            write_compact(&v),
            br#"{"k":"\u003ca \u0026 \"b\"\u003e\n"}"#.to_vec()
        );
        *v.get_mut("k").unwrap() = Value::string_with("<a & b>", false);
        assert_eq!(write_compact(&v), br#"{"k":"<a & b>"}"#.to_vec());
    }

    #[test]
    fn rewrite_keeps_untouched_bytes_and_crlf() {
        let dir = std::env::temp_dir().join(format!("gocompat-rewrite-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("g.json");
        let file =
            "{\r\n  \"z\": \"\\u003c\",\r\n  \"a\": [\r\n    1,\r\n    \"old\"\r\n  ]\r\n}\r\n";
        std::fs::write(&p, file).unwrap();
        assert!(!rewrite_indented(&p, |_| {}).unwrap());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), file);
        let changed = rewrite_indented(&p, |v| {
            let Some(Value::Array(a)) = v.get_mut("a") else {
                panic!("no a")
            };
            a[1] = Value::string("new");
        })
        .unwrap();
        assert!(changed);
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            file.replace("old", "new")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
