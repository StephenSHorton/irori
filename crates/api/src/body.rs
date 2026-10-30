//! POST bodies decoded as Go's `readBody` does (`json.Unmarshal` into the
//! route's struct), with Go's error texts: case-insensitive keys, the last
//! duplicate wins, unknown keys ignored, `null` leaves a field alone, and
//! the first type error is reported once the whole value is read.

use gocompat::json::{JsonStr, Value};

/// The POST body cap (Go `maxRequestBody`).
pub const MAX_REQUEST_BODY: usize = 256 << 10;

/// A field's Go type.
#[derive(Clone, Copy)]
pub(crate) enum Ty {
    /// `string`, or a named string type (its Go name).
    String(&'static str),
    Bool,
    /// `json.RawMessage`.
    Raw,
    /// `[]string`.
    Strings,
}

/// A decoded field.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Val {
    /// Never set (absent, or `null`).
    Unset,
    Str(String),
    Bool(bool),
    Raw(Vec<u8>),
    /// `None` is a nil slice.
    Strings(Option<Vec<String>>),
}

/// The struct a body decodes into: its Go type text (for a top-level type
/// error), its name (for field errors; `""` for an anonymous struct) and
/// its fields.
pub(crate) struct Shape {
    pub go_type: &'static str,
    pub name: &'static str,
    pub fields: &'static [(&'static str, Ty)],
}

pub(crate) struct Decoded(Vec<Val>);

impl Decoded {
    pub fn str(&self, i: usize) -> String {
        match &self.0[i] {
            Val::Str(s) => s.clone(),
            _ => String::new(),
        }
    }
    pub fn bool(&self, i: usize) -> bool {
        matches!(self.0[i], Val::Bool(true))
    }
    pub fn raw(&self, i: usize) -> Option<Vec<u8>> {
        match &self.0[i] {
            Val::Raw(r) => Some(r.clone()),
            _ => None,
        }
    }
    pub fn strings(&self, i: usize) -> Option<Vec<String>> {
        match &self.0[i] {
            Val::Strings(v) => v.clone(),
            _ => None,
        }
    }
}

/// Go `readBody`: whitespace only is an empty body (every field zero);
/// anything else must decode into `shape`. The error is the full text the
/// route writes with 400.
pub(crate) fn read(raw: &[u8], too_large: bool, shape: &Shape) -> Result<Decoded, String> {
    if too_large {
        return Err("body: http: request body too large".into());
    }
    let zero = Decoded(shape.fields.iter().map(|_| Val::Unset).collect());
    // strings.TrimSpace(string(raw)): Unicode spaces, the body read as text.
    if String::from_utf8_lossy(raw)
        .trim_matches(gocompat::text::is_space)
        .is_empty()
    {
        return Ok(zero);
    }
    decode(raw, shape, zero).map_err(|e| format!("body must be a JSON object: {e}"))
}

fn decode(raw: &[u8], shape: &Shape, mut out: Decoded) -> Result<Decoded, String> {
    let v = gocompat::json::parse(raw).map_err(|e| e.to_string())?;
    let members = match &v {
        Value::Null => return Ok(out),
        Value::Object(m) => m,
        other => {
            return Err(format!(
                "json: cannot unmarshal {} into Go value of type {}",
                other.kind(),
                shape.go_type
            ));
        }
    };
    let mut first_err: Option<String> = None;
    for (k, val) in members {
        // Go: an exact name first, else a case-folded one.
        let idx = shape
            .fields
            .iter()
            .position(|(n, _)| *n == k.text)
            .or_else(|| {
                shape
                    .fields
                    .iter()
                    .position(|(n, _)| gocompat::text::equal_fold(n, &k.text))
            });
        let Some(i) = idx else { continue };
        let (fname, ty) = shape.fields[i];
        let type_err = |value: &str, go: &str| {
            format!(
                "json: cannot unmarshal {value} into Go struct field {}.{fname} of type {go}",
                shape.name
            )
        };
        match (ty, val) {
            (Ty::Raw, v) => out.0[i] = Val::Raw(write_value(v)),
            (_, Value::Null) => {
                if let Ty::Strings = ty {
                    out.0[i] = Val::Strings(None);
                }
            }
            (Ty::String(_), Value::String(s)) => out.0[i] = Val::Str(s.text.clone()),
            (Ty::String(go), v) => {
                first_err.get_or_insert_with(|| type_err(v.kind(), go));
            }
            (Ty::Bool, Value::Bool(b)) => out.0[i] = Val::Bool(*b),
            (Ty::Bool, v) => {
                first_err.get_or_insert_with(|| type_err(v.kind(), "bool"));
            }
            (Ty::Strings, Value::Array(items)) => {
                let mut list = Vec::with_capacity(items.len());
                for it in items {
                    match it {
                        Value::String(s) => list.push(s.text.clone()),
                        Value::Null => list.push(String::new()),
                        v => {
                            first_err.get_or_insert_with(|| type_err(v.kind(), "string"));
                            list.push(String::new());
                        }
                    }
                }
                out.0[i] = Val::Strings(Some(list));
            }
            (Ty::Strings, v) => {
                first_err.get_or_insert_with(|| type_err(v.kind(), "[]string"));
            }
        }
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(out),
    }
}

/// A parsed value written back as JSON, strings as their original literal,
/// so a `json.RawMessage` field carries what was sent (whitespace aside).
fn write_value(v: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write_into(v, &mut out);
    out
}

fn write_str(s: &JsonStr, out: &mut Vec<u8>) {
    out.push(b'"');
    out.extend_from_slice(&s.raw);
    out.push(b'"');
}

fn write_into(v: &Value, out: &mut Vec<u8>) {
    match v {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(n) => out.extend_from_slice(n.as_bytes()),
        Value::String(s) => write_str(s, out),
        Value::Array(items) => {
            out.push(b'[');
            for (i, it) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_into(it, out);
            }
            out.push(b']');
        }
        Value::Object(m) => {
            out.push(b'{');
            for (i, (k, it)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_str(k, out);
                out.push(b':');
                write_into(it, out);
            }
            out.push(b'}');
        }
    }
}

// ---- the routes' bodies ----------------------------------------------------

pub(crate) const CONTROL: Shape = Shape {
    go_type: r#"struct { Token string "json:\"token\""; TurnID string "json:\"turnId\""; Appended bool "json:\"appended\"" }"#,
    name: "",
    fields: &[
        ("token", Ty::String("string")),
        ("turnId", Ty::String("string")),
        ("appended", Ty::Bool),
    ],
};

pub(crate) const TOOL: Shape = Shape {
    go_type: r#"struct { Args json.RawMessage "json:\"args\"" }"#,
    name: "",
    fields: &[("args", Ty::Raw)],
};

pub(crate) const POLICY: Shape = Shape {
    go_type: "inbox.Policy",
    name: "Policy",
    fields: &[
        ("mode", Ty::String("inbox.Mode")),
        ("block", Ty::Strings),
        ("allow", Ty::Strings),
        ("paused", Ty::Bool),
    ],
};

pub(crate) const JOIN: Shape = Shape {
    go_type: r#"struct { Name string "json:\"name\""; Session string "json:\"session\"" }"#,
    name: "",
    fields: &[
        ("name", Ty::String("string")),
        ("session", Ty::String("string")),
    ],
};

pub(crate) const PAIR_JOIN: Shape = Shape {
    go_type: r#"struct { Code string "json:\"code\""; Server string "json:\"server\"" }"#,
    name: "",
    fields: &[
        ("code", Ty::String("string")),
        ("server", Ty::String("string")),
    ],
};

pub(crate) const PAIR_KEY: Shape = Shape {
    go_type: r#"struct { Key string "json:\"key\"" }"#,
    name: "",
    fields: &[("key", Ty::String("string"))],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_texts() {
        let e = |b: &str| read(b.as_bytes(), false, &CONTROL).err().unwrap();
        assert_eq!(
            e("{"),
            "body must be a JSON object: unexpected end of JSON input"
        );
        assert_eq!(
            e("[1]"),
            r#"body must be a JSON object: json: cannot unmarshal array into Go value of type struct { Token string "json:\"token\""; TurnID string "json:\"turnId\""; Appended bool "json:\"appended\"" }"#
        );
        assert_eq!(
            e(r#"{"token":5}"#),
            "body must be a JSON object: json: cannot unmarshal number into Go struct field .token of type string"
        );
        let d = read(
            br#"{"TOKEN":"a","token":"b","x":1,"appended":true}"#,
            false,
            &CONTROL,
        )
        .unwrap();
        assert_eq!(d.str(0), "b");
        assert!(d.bool(2));
        let d = read(b"  \n", false, &CONTROL).unwrap();
        assert_eq!(d.str(0), "");
        assert_eq!(
            read(b"{}", true, &CONTROL).err().unwrap(),
            "body: http: request body too large"
        );
        let p = read(br#"{"block":["a",1]}"#, false, &POLICY).err().unwrap();
        assert_eq!(
            p,
            "body must be a JSON object: json: cannot unmarshal number into Go struct field Policy.block of type string"
        );
        let d = read(br#"{"args": {"a" : [1, "xA"]}}"#, false, &TOOL).unwrap();
        assert_eq!(d.raw(0).unwrap(), br#"{"a":[1,"xA"]}"#);
    }
}
