//! Go `json.Unmarshal` into a struct, driven by a [`Schema`] that describes
//! the Go type (store.md §7.5, capture.md §7.5, identity.md §7.3 trap 7).
//!
//! The rules reproduced:
//! - the input must pass `json.Valid` first, or nothing is decoded;
//! - a key matches a field by exact name first, else by Go's case fold
//!   (`bytes.EqualFold`), taking the first field in declaration order with
//!   that fold; unknown keys are skipped;
//! - every key is applied in order, so the last key that maps to a field
//!   wins, whatever its spelling; a struct field (or pointer to struct)
//!   decoded twice is merged, a map field gains keys, a slice is refilled
//!   element by element;
//! - `null` sets a slice, map, pointer, `any` or raw field to nil and leaves
//!   every other field alone; a top-level `null` is a no-op;
//! - a type mismatch anywhere, an integer that is not an integer or
//!   overflows its Go type, a float out of range, or a bad time makes the
//!   whole call fail. Go goes on filling fields after a type error, but every
//!   Irori caller discards the value on any error, so this returns only the
//!   error;
//! - `time.Time` reads the string literal without unescaping it, as Go does.
//!
//! Not modelled: Go `[N]T` arrays, `,string` tags, `[]byte` fields, and the
//! reuse of slice elements past the current length that Go's `reflect`
//! capacity games allow.

use std::collections::BTreeMap;

use super::Error;
use super::anymap::{AnyValue, parse_f64};
use super::value::{Value, parse};
use crate::text::fold_name;
use crate::time::GoTime;

/// A Go type the decoder fills.
#[derive(Debug, Clone)]
pub enum Schema {
    Bool,
    /// A signed integer of this many bits (8, 16, 32, 64).
    Int(u32),
    /// An unsigned integer of this many bits.
    Uint(u32),
    /// A float of this many bits (32 or 64).
    Float(u32),
    String,
    /// `time.Time`.
    Time,
    /// `json.RawMessage`.
    Raw,
    /// `any`.
    Any,
    /// `[]T`.
    Slice(Box<Schema>),
    /// `map[string]T`.
    Map(Box<Schema>),
    /// A struct: `(json name, type)` in declaration order.
    Struct(Vec<(&'static str, Schema)>),
    /// `*T`.
    Ptr(Box<Schema>),
}

impl Schema {
    /// Convenience constructor for a struct schema.
    pub fn object(fields: Vec<(&'static str, Schema)>) -> Schema {
        Schema::Struct(fields)
    }
}

/// A decoded Go value.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    Bool(bool),
    Int(i64),
    Uint(u64),
    Float(f64),
    /// A `float32` field (marshals with float32 digits).
    Float32(f32),
    String(String),
    Time(GoTime),
    /// `None` is a nil `RawMessage`.
    Raw(Option<Vec<u8>>),
    /// `None` is a nil interface.
    Any(Option<AnyValue>),
    /// `None` is a nil slice (`null`), `Some(empty)` an empty one.
    Slice(Option<Vec<Out>>),
    Map(Option<BTreeMap<String, Out>>),
    Struct(Vec<(&'static str, Out)>),
    Ptr(Option<Box<Out>>),
}

impl Out {
    /// The zero value of a schema, as Go's `reflect.Zero`.
    pub fn zero(s: &Schema) -> Out {
        match s {
            Schema::Bool => Out::Bool(false),
            Schema::Int(_) => Out::Int(0),
            Schema::Uint(_) => Out::Uint(0),
            Schema::Float(32) => Out::Float32(0.0),
            Schema::Float(_) => Out::Float(0.0),
            Schema::String => Out::String(String::new()),
            Schema::Time => Out::Time(GoTime::ZERO),
            Schema::Raw => Out::Raw(None),
            Schema::Any => Out::Any(None),
            Schema::Slice(_) => Out::Slice(None),
            Schema::Map(_) => Out::Map(None),
            Schema::Struct(fields) => {
                Out::Struct(fields.iter().map(|(n, f)| (*n, Out::zero(f))).collect())
            }
            Schema::Ptr(_) => Out::Ptr(None),
        }
    }

    /// A struct field by its JSON name (`None` for a non-struct or an
    /// unknown name). Follows a non-nil pointer.
    pub fn get(&self, name: &str) -> Option<&Out> {
        match self {
            Out::Struct(fields) => fields.iter().find(|(n, _)| *n == name).map(|(_, v)| v),
            Out::Ptr(Some(inner)) => inner.get(name),
            _ => None,
        }
    }

    /// The string value (`""` for anything else), for struct-field reads.
    pub fn str(&self) -> &str {
        match self {
            Out::String(s) => s,
            _ => "",
        }
    }

    /// `get(name)` as a string, `""` when absent.
    pub fn get_str(&self, name: &str) -> &str {
        self.get(name).map(Out::str).unwrap_or("")
    }

    /// The elements of a slice (empty for nil or a non-slice).
    pub fn items(&self) -> &[Out] {
        match self {
            Out::Slice(Some(v)) => v,
            _ => &[],
        }
    }
}

/// `json.Unmarshal(data, &v)` where `v` is a zero value of `schema`.
pub fn unmarshal(data: &[u8], schema: &Schema) -> Result<Out, Error> {
    let tree = parse(data)?;
    let mut out = Out::zero(schema);
    decode(&tree, schema, &mut out)?;
    Ok(out)
}

/// `json.Unmarshal` of one already-parsed value (an object member, an array
/// element) into a zero value of `schema`: for callers that decode a
/// subtree field by field, as Go code does with `map[string]RawMessage`.
pub fn unmarshal_value(v: &Value, schema: &Schema) -> Result<Out, Error> {
    let mut out = Out::zero(schema);
    decode(v, schema, &mut out)?;
    Ok(out)
}

/// Decode into an existing value, as a second `json.Unmarshal` into the same
/// variable would.
pub fn unmarshal_into(data: &[u8], schema: &Schema, out: &mut Out) -> Result<(), Error> {
    let tree = parse(data)?;
    decode(&tree, schema, out)
}

fn type_err(v: &Value, want: &str) -> Error {
    Error::Type(format!(
        "json: cannot unmarshal {} into Go value of type {want}",
        v.kind()
    ))
}

fn go_type(s: &Schema) -> String {
    match s {
        Schema::Bool => "bool".into(),
        Schema::Int(b) => {
            if *b == 64 {
                "int64".into()
            } else {
                format!("int{b}")
            }
        }
        Schema::Uint(b) => format!("uint{b}"),
        Schema::Float(b) => format!("float{b}"),
        Schema::String => "string".into(),
        Schema::Time => "time.Time".into(),
        Schema::Raw => "json.RawMessage".into(),
        Schema::Any => "interface {}".into(),
        Schema::Slice(e) => format!("[]{}", go_type(e)),
        Schema::Map(e) => format!("map[string]{}", go_type(e)),
        Schema::Struct(_) => "struct".into(),
        Schema::Ptr(e) => format!("*{}", go_type(e)),
    }
}

/// A `RawMessage` field's bytes: the item's literal, compacted (strings and
/// keys keep their original spelling). Go keeps the literal with its inner
/// whitespace; `json.Marshal` compacts it, so after a re-marshal both agree.
fn raw_of(v: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write_raw(&mut out, v);
    out
}

fn write_raw(out: &mut Vec<u8>, v: &Value) {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || match v {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Number(n) => out.extend_from_slice(n.as_bytes()),
        Value::String(s) => {
            out.push(b'"');
            out.extend_from_slice(&s.raw);
            out.push(b'"');
        }
        Value::Array(items) => {
            out.push(b'[');
            for (i, it) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_raw(out, it);
            }
            out.push(b']');
        }
        Value::Object(members) => {
            out.push(b'{');
            for (i, (k, v)) in members.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                out.push(b'"');
                out.extend_from_slice(&k.raw);
                out.push(b'"');
                out.push(b':');
                write_raw(out, v);
            }
            out.push(b'}');
        }
    })
}

fn decode(v: &Value, s: &Schema, out: &mut Out) -> Result<(), Error> {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || decode_inner(v, s, out))
}

fn decode_inner(v: &Value, s: &Schema, out: &mut Out) -> Result<(), Error> {
    // null: nil for reference kinds, a no-op for everything else.
    if let Value::Null = v {
        match (s, &mut *out) {
            (Schema::Raw, o) => *o = Out::Raw(Some(b"null".to_vec())),
            (Schema::Slice(_), o) => *o = Out::Slice(None),
            (Schema::Map(_), o) => *o = Out::Map(None),
            (Schema::Ptr(_), o) => *o = Out::Ptr(None),
            (Schema::Any, o) => *o = Out::Any(None),
            _ => {}
        }
        return Ok(());
    }
    match s {
        Schema::Raw => {
            *out = Out::Raw(Some(raw_of(v)));
            Ok(())
        }
        Schema::Any => {
            *out = Out::Any(Some(AnyValue::from_value(v)?));
            Ok(())
        }
        Schema::Ptr(inner) => {
            if !matches!(out, Out::Ptr(Some(_))) {
                *out = Out::Ptr(Some(Box::new(Out::zero(inner))));
            }
            let Out::Ptr(Some(b)) = out else {
                unreachable!()
            };
            decode(v, inner, b)
        }
        Schema::Time => match v {
            Value::String(js) => {
                // Go parses the literal's bytes, escapes and all.
                let t = if js.escaped {
                    None
                } else {
                    GoTime::parse_rfc3339(&js.text)
                };
                *out = Out::Time(t.ok_or_else(|| {
                    Error::Type(format!(
                        "parsing time {:?} as \"2006-01-02T15:04:05Z07:00\": cannot parse",
                        String::from_utf8_lossy(&js.raw)
                    ))
                })?);
                Ok(())
            }
            _ => Err(Error::Type(
                "Time.UnmarshalJSON: input is not a JSON string".into(),
            )),
        },
        Schema::Bool => match v {
            Value::Bool(b) => {
                *out = Out::Bool(*b);
                Ok(())
            }
            _ => Err(type_err(v, "bool")),
        },
        Schema::String => match v {
            Value::String(js) => {
                *out = Out::String(js.text.clone());
                Ok(())
            }
            _ => Err(type_err(v, "string")),
        },
        Schema::Int(bits) => match v {
            Value::Number(n) => {
                let x: i64 = n.parse().map_err(|_| {
                    Error::Type(format!(
                        "json: cannot unmarshal number {n} into Go value of type {}",
                        go_type(s)
                    ))
                })?;
                let ok = *bits >= 64 || {
                    let lim = 1i64 << (bits - 1);
                    (-lim..lim).contains(&x)
                };
                if !ok {
                    return Err(Error::Type(format!(
                        "json: cannot unmarshal number {n} into Go value of type {}",
                        go_type(s)
                    )));
                }
                *out = Out::Int(x);
                Ok(())
            }
            _ => Err(type_err(v, &go_type(s))),
        },
        Schema::Uint(bits) => match v {
            Value::Number(n) => {
                let x: u64 = n.parse().map_err(|_| {
                    Error::Type(format!(
                        "json: cannot unmarshal number {n} into Go value of type {}",
                        go_type(s)
                    ))
                })?;
                if *bits < 64 && x >= (1u64 << bits) {
                    return Err(Error::Type(format!(
                        "json: cannot unmarshal number {n} into Go value of type {}",
                        go_type(s)
                    )));
                }
                *out = Out::Uint(x);
                Ok(())
            }
            _ => Err(type_err(v, &go_type(s))),
        },
        Schema::Float(bits) => match v {
            Value::Number(n) => {
                let bad = || {
                    Error::Type(format!(
                        "json: cannot unmarshal number {n} into Go value of type {}",
                        go_type(s)
                    ))
                };
                *out = if *bits == 32 {
                    Out::Float32(
                        n.parse::<f32>()
                            .ok()
                            .filter(|f| f.is_finite())
                            .ok_or_else(bad)?,
                    )
                } else {
                    Out::Float(parse_f64(n).ok_or_else(bad)?)
                };
                Ok(())
            }
            _ => Err(type_err(v, &go_type(s))),
        },
        Schema::Slice(elem) => match v {
            Value::Array(items) => {
                let mut prev = match std::mem::replace(out, Out::Slice(None)) {
                    Out::Slice(Some(p)) => p,
                    _ => Vec::new(),
                };
                prev.truncate(items.len());
                let mut next = Vec::with_capacity(items.len());
                let mut old = prev.into_iter();
                for it in items {
                    let mut o = old.next().unwrap_or_else(|| Out::zero(elem));
                    decode(it, elem, &mut o)?;
                    next.push(o);
                }
                *out = Out::Slice(Some(next));
                Ok(())
            }
            _ => Err(type_err(v, &go_type(s))),
        },
        Schema::Map(elem) => match v {
            Value::Object(members) => {
                if !matches!(out, Out::Map(Some(_))) {
                    *out = Out::Map(Some(BTreeMap::new()));
                }
                let Out::Map(Some(m)) = out else {
                    unreachable!()
                };
                for (k, val) in members {
                    // Go decodes each map element into a fresh zero value.
                    let mut o = Out::zero(elem);
                    decode(val, elem, &mut o)?;
                    m.insert(k.text.clone(), o);
                }
                Ok(())
            }
            _ => Err(type_err(v, &go_type(s))),
        },
        Schema::Struct(fields) => match v {
            Value::Object(members) => {
                let Out::Struct(slots) = out else {
                    return Err(Error::Type("struct value does not match its schema".into()));
                };
                for (k, val) in members {
                    let Some(i) = field_index(fields, &k.text) else {
                        continue;
                    };
                    decode(val, &fields[i].1, &mut slots[i].1)?;
                }
                Ok(())
            }
            _ => Err(type_err(v, "struct")),
        },
    }
}

/// Go's field lookup: exact name, else the first field whose folded name
/// equals the key's.
fn field_index(fields: &[(&'static str, Schema)], key: &str) -> Option<usize> {
    if let Some(i) = fields.iter().position(|(n, _)| *n == key) {
        return Some(i);
    }
    let fk = fold_name(key.as_bytes());
    fields
        .iter()
        .position(|(n, _)| fold_name(n.as_bytes()) == fk)
}
