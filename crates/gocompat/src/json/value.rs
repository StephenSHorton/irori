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
