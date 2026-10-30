//! A port of Go's `encoding/json` scanner (scanner.go): a byte-at-a-time
//! state machine with an explicit parse stack, so validation never recurses.
//! [`valid`], [`compact`] and [`indent`] are built on it exactly as Go builds
//! `Valid`, `Compact` and `Indent`.

use super::Error;

/// Go's `maxNestingDepth`: an array or object nested deeper is a syntax
/// error, so `valid` and Go's `json.Valid` agree on deep input.
pub const MAX_NESTING_DEPTH: usize = 10000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Op {
    Continue,
    BeginLiteral,
    BeginObject,
    ObjectKey,
    ObjectValue,
    EndObject,
    BeginArray,
    ArrayValue,
    EndArray,
    SkipSpace,
    End,
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum St {
    BeginValueOrEmpty,
    BeginValue,
    BeginStringOrEmpty,
    BeginString,
    EndValue,
    EndTop,
    InString,
    InStringEsc,
    EscU,
    EscU1,
    EscU12,
    EscU123,
    Neg,
    One,
    Zero,
    Dot,
    Dot0,
    E,
    ESign,
    E0,
    T,
    Tr,
    Tru,
    F,
    Fa,
    Fal,
    Fals,
    N,
    Nu,
    Nul,
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Parse {
    ObjectKey,
    ObjectValue,
    ArrayValue,
}

pub(crate) struct Scanner {
    step: St,
    parse: Vec<Parse>,
    end_top: bool,
    err: Option<Error>,
    pub(crate) bytes: usize,
}

fn is_space(c: u8) -> bool {
    c == b' ' || c == b'\t' || c == b'\r' || c == b'\n'
}

fn is_hex(c: u8) -> bool {
    c.is_ascii_hexdigit()
}

/// Go's `quoteChar`, for syntax error texts.
fn quote_char(c: u8) -> String {
    match c {
        b'\'' => r"'\''".to_string(),
        b'"' => "'\"'".to_string(),
        _ => {
            let q = crate::quote::quote_bytes(&[c]);
            format!("'{}'", &q[1..q.len() - 1])
        }
    }
}

impl Scanner {
    pub(crate) fn new() -> Self {
        Scanner {
            step: St::BeginValue,
            parse: Vec::new(),
            end_top: false,
            err: None,
            bytes: 0,
        }
    }

    pub(crate) fn take_err(&mut self) -> Error {
        self.err
            .take()
            .unwrap_or_else(|| Error::Syntax("unexpected end of JSON input".into()))
    }

    /// Go's `scanner.eof`.
    pub(crate) fn eof(&mut self) -> Op {
        if self.err.is_some() {
            return Op::Error;
        }
        if self.end_top {
            return Op::End;
        }
        self.step(b' ');
        if self.end_top {
            return Op::End;
        }
        if self.err.is_none() {
            self.err = Some(Error::Syntax("unexpected end of JSON input".into()));
        }
        Op::Error
    }

    fn error(&mut self, c: u8, context: &str) -> Op {
        self.step = St::Error;
        self.err = Some(Error::Syntax(format!(
            "invalid character {} {context}",
            quote_char(c)
        )));
        Op::Error
    }

    fn push(&mut self, c: u8, p: Parse, ok: Op) -> Op {
        self.parse.push(p);
        if self.parse.len() <= MAX_NESTING_DEPTH {
            ok
        } else {
            self.error(c, "exceeded max depth")
        }
    }

    fn pop(&mut self) {
        self.parse.pop();
        if self.parse.is_empty() {
            self.step = St::EndTop;
            self.end_top = true;
        } else {
            self.step = St::EndValue;
        }
    }

    pub(crate) fn step(&mut self, c: u8) -> Op {
        match self.step {
            St::BeginValueOrEmpty => {
                if is_space(c) {
                    Op::SkipSpace
                } else if c == b']' {
                    self.end_value(c)
                } else {
                    self.begin_value(c)
                }
            }
            St::BeginValue => self.begin_value(c),
            St::BeginStringOrEmpty => {
                if is_space(c) {
                    return Op::SkipSpace;
                }
                if c == b'}' {
                    if let Some(last) = self.parse.last_mut() {
                        *last = Parse::ObjectValue;
                    }
                    return self.end_value(c);
                }
                self.begin_string(c)
            }
            St::BeginString => self.begin_string(c),
            St::EndValue => self.end_value(c),
            St::EndTop => self.end_top_state(c),
            St::InString => {
                if c == b'"' {
                    self.step = St::EndValue;
                    Op::Continue
                } else if c == b'\\' {
                    self.step = St::InStringEsc;
                    Op::Continue
                } else if c < 0x20 {
                    self.error(c, "in string literal")
                } else {
                    Op::Continue
                }
            }
            St::InStringEsc => match c {
                b'b' | b'f' | b'n' | b'r' | b't' | b'\\' | b'/' | b'"' => {
                    self.step = St::InString;
                    Op::Continue
                }
                b'u' => {
                    self.step = St::EscU;
                    Op::Continue
                }
                _ => self.error(c, "in string escape code"),
            },
            St::EscU | St::EscU1 | St::EscU12 | St::EscU123 => {
                if is_hex(c) {
                    self.step = match self.step {
                        St::EscU => St::EscU1,
                        St::EscU1 => St::EscU12,
                        St::EscU12 => St::EscU123,
                        _ => St::InString,
                    };
                    Op::Continue
                } else {
                    self.error(c, "in \\u hexadecimal character escape")
                }
            }
            St::Neg => {
                if c == b'0' {
                    self.step = St::Zero;
                    Op::Continue
                } else if c.is_ascii_digit() {
                    self.step = St::One;
                    Op::Continue
                } else {
                    self.error(c, "in numeric literal")
                }
            }
            St::One => {
                if c.is_ascii_digit() {
                    Op::Continue
                } else {
                    self.zero(c)
                }
            }
            St::Zero => self.zero(c),
            St::Dot => {
                if c.is_ascii_digit() {
                    self.step = St::Dot0;
                    Op::Continue
                } else {
                    self.error(c, "after decimal point in numeric literal")
                }
            }
            St::Dot0 => {
                if c.is_ascii_digit() {
                    Op::Continue
                } else if c == b'e' || c == b'E' {
                    self.step = St::E;
                    Op::Continue
                } else {
                    self.end_value(c)
                }
            }
            St::E => {
                if c == b'+' || c == b'-' {
                    self.step = St::ESign;
                    Op::Continue
                } else {
                    self.e_sign(c)
                }
            }
            St::ESign => self.e_sign(c),
            St::E0 => {
                if c.is_ascii_digit() {
                    Op::Continue
                } else {
                    self.end_value(c)
                }
            }
            St::T => self.lit(c, b'r', St::Tr, "in literal true (expecting 'r')"),
            St::Tr => self.lit(c, b'u', St::Tru, "in literal true (expecting 'u')"),
            St::Tru => self.lit(c, b'e', St::EndValue, "in literal true (expecting 'e')"),
            St::F => self.lit(c, b'a', St::Fa, "in literal false (expecting 'a')"),
            St::Fa => self.lit(c, b'l', St::Fal, "in literal false (expecting 'l')"),
            St::Fal => self.lit(c, b's', St::Fals, "in literal false (expecting 's')"),
            St::Fals => self.lit(c, b'e', St::EndValue, "in literal false (expecting 'e')"),
            St::N => self.lit(c, b'u', St::Nu, "in literal null (expecting 'u')"),
            St::Nu => self.lit(c, b'l', St::Nul, "in literal null (expecting 'l')"),
            St::Nul => self.lit(c, b'l', St::EndValue, "in literal null (expecting 'l')"),
            St::Error => Op::Error,
        }
    }

    fn lit(&mut self, c: u8, want: u8, next: St, ctx: &str) -> Op {
        if c == want {
            self.step = next;
            Op::Continue
        } else {
            self.error(c, ctx)
        }
    }

    fn begin_value(&mut self, c: u8) -> Op {
        if is_space(c) {
            return Op::SkipSpace;
        }
        match c {
            b'{' => {
                self.step = St::BeginStringOrEmpty;
                self.push(c, Parse::ObjectKey, Op::BeginObject)
            }
            b'[' => {
                self.step = St::BeginValueOrEmpty;
                self.push(c, Parse::ArrayValue, Op::BeginArray)
            }
            b'"' => {
                self.step = St::InString;
                Op::BeginLiteral
            }
            b'-' => {
                self.step = St::Neg;
                Op::BeginLiteral
            }
            b'0' => {
                self.step = St::Zero;
                Op::BeginLiteral
            }
            b't' => {
                self.step = St::T;
                Op::BeginLiteral
            }
            b'f' => {
                self.step = St::F;
                Op::BeginLiteral
            }
            b'n' => {
                self.step = St::N;
                Op::BeginLiteral
            }
            b'1'..=b'9' => {
                self.step = St::One;
                Op::BeginLiteral
            }
            _ => self.error(c, "looking for beginning of value"),
        }
    }

    fn begin_string(&mut self, c: u8) -> Op {
        if is_space(c) {
            return Op::SkipSpace;
        }
        if c == b'"' {
            self.step = St::InString;
            return Op::BeginLiteral;
        }
        self.error(c, "looking for beginning of object key string")
    }

    fn zero(&mut self, c: u8) -> Op {
        if c == b'.' {
            self.step = St::Dot;
            Op::Continue
        } else if c == b'e' || c == b'E' {
            self.step = St::E;
            Op::Continue
        } else {
            self.end_value(c)
        }
    }

    fn e_sign(&mut self, c: u8) -> Op {
        if c.is_ascii_digit() {
            self.step = St::E0;
            Op::Continue
        } else {
            self.error(c, "in exponent of numeric literal")
        }
    }

    fn end_value(&mut self, c: u8) -> Op {
        let Some(&ps) = self.parse.last() else {
            self.step = St::EndTop;
            self.end_top = true;
            return self.end_top_state(c);
        };
        if is_space(c) {
            self.step = St::EndValue;
            return Op::SkipSpace;
        }
        match ps {
            Parse::ObjectKey => {
                if c == b':' {
                    *self.parse.last_mut().expect("non-empty") = Parse::ObjectValue;
                    self.step = St::BeginValue;
                    return Op::ObjectKey;
                }
                self.error(c, "after object key")
            }
            Parse::ObjectValue => {
                if c == b',' {
                    *self.parse.last_mut().expect("non-empty") = Parse::ObjectKey;
                    self.step = St::BeginString;
                    return Op::ObjectValue;
                }
                if c == b'}' {
                    self.pop();
                    return Op::EndObject;
                }
                self.error(c, "after object key:value pair")
            }
            Parse::ArrayValue => {
                if c == b',' {
                    self.step = St::BeginValue;
                    return Op::ArrayValue;
                }
                if c == b']' {
                    self.pop();
                    return Op::EndArray;
                }
                self.error(c, "after array element")
            }
        }
    }

    fn end_top_state(&mut self, c: u8) -> Op {
        if !is_space(c) {
            self.error(c, "after top-level value");
        }
        Op::End
    }
}

/// Go's `checkValid`: `Ok` when `data` is one JSON value, with Go's error
/// text otherwise.
pub fn check_valid(data: &[u8]) -> Result<(), Error> {
    let mut s = Scanner::new();
    for &c in data {
        s.bytes += 1;
        if s.step(c) == Op::Error {
            return Err(s.take_err());
        }
    }
    if s.eof() == Op::Error {
        return Err(s.take_err());
    }
    Ok(())
}

/// Go `json.Valid`. Bytes 0x80 and above pass unchecked inside strings
/// (invalid UTF-8 is valid JSON to Go), control bytes below 0x20 do not,
/// whitespace is only space, tab, CR and LF, and nesting stops at
/// [`MAX_NESTING_DEPTH`] (store.md §7.4).
pub fn valid(data: &[u8]) -> bool {
    check_valid(data).is_ok()
}

fn push_u_escape(dst: &mut Vec<u8>, c: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    dst.extend_from_slice(b"\\u00");
    dst.push(HEX[(c >> 4) as usize]);
    dst.push(HEX[(c & 0xf) as usize]);
}

/// Go's `appendCompact`: `src` with insignificant space removed, and with
/// `escape_html` the `<`, `>`, `&`, U+2028 and U+2029 inside strings turned
/// into `\u` escapes (what `json.Marshal` does to a `json.RawMessage`).
pub fn compact(src: &[u8], escape_html: bool) -> Result<Vec<u8>, Error> {
    let mut dst = Vec::with_capacity(src.len());
    let mut s = Scanner::new();
    let mut start = 0;
    let mut i = 0;
    while i < src.len() {
        let c = src[i];
        if escape_html && (c == b'<' || c == b'>' || c == b'&') {
            dst.extend_from_slice(&src[start..i]);
            push_u_escape(&mut dst, c);
            start = i + 1;
        }
        if escape_html
            && c == 0xe2
            && i + 2 < src.len()
            && src[i + 1] == 0x80
            && src[i + 2] & !1 == 0xa8
        {
            dst.extend_from_slice(&src[start..i]);
            dst.extend_from_slice(b"\\u202");
            dst.push(b"0123456789abcdef"[(src[i + 2] & 0xf) as usize]);
            start = i + 3;
        }
        let v = s.step(c);
        if v == Op::Error {
            break;
        }
        if v == Op::SkipSpace || v == Op::End {
            if start < i {
                dst.extend_from_slice(&src[start..i]);
            }
            start = i + 1;
        }
        i += 1;
    }
    if s.eof() == Op::Error {
        return Err(s.take_err());
    }
    if start < src.len() {
        dst.extend_from_slice(&src[start..]);
    }
    Ok(dst)
}

fn newline(dst: &mut Vec<u8>, prefix: &str, indent: &str, depth: usize) {
    dst.push(b'\n');
    dst.extend_from_slice(prefix.as_bytes());
    for _ in 0..depth {
        dst.extend_from_slice(indent.as_bytes());
    }
}

/// Go's `json.Indent`. Leading space is dropped and trailing space kept, as in
/// Go; `MarshalIndent` is `marshal` followed by this.
pub fn indent(src: &[u8], prefix: &str, indent: &str) -> Result<Vec<u8>, Error> {
    let mut dst = Vec::with_capacity(src.len() * 2);
    let mut s = Scanner::new();
    let mut need_indent = false;
    let mut depth: usize = 0;
    for &c in src {
        s.bytes += 1;
        let v = s.step(c);
        if v == Op::SkipSpace {
            continue;
        }
        if v == Op::Error {
            break;
        }
        if need_indent && v != Op::EndObject && v != Op::EndArray {
            need_indent = false;
            depth += 1;
            newline(&mut dst, prefix, indent, depth);
        }
        if v == Op::Continue {
            dst.push(c);
            continue;
        }
        match c {
            b'{' | b'[' => {
                need_indent = true;
                dst.push(c);
            }
            b',' => {
                dst.push(c);
                newline(&mut dst, prefix, indent, depth);
            }
            b':' => {
                dst.push(c);
                dst.push(b' ');
            }
            b'}' | b']' => {
                if need_indent {
                    need_indent = false;
                } else {
                    depth = depth.saturating_sub(1);
                    newline(&mut dst, prefix, indent, depth);
                }
                dst.push(c);
            }
            _ => dst.push(c),
        }
    }
    if s.eof() == Op::Error {
        return Err(s.take_err());
    }
    Ok(dst)
}
