//! A round trip through Go's `map[string]any` / `any` (surface.md §7.8(h)).
//!
//! `json.Unmarshal` into `any` turns every number into a float64 (a number
//! out of float64 range is an error), objects into maps (last duplicate
//! wins), and `json.Marshal` then writes keys sorted and floats by Go's
//! `'f'`/`'e'` rule. So an integer above 2^53, such as an `hlc` inside a
//! payload, comes back as the nearest float64's integer: a different number,
//! exactly as Go prints it.

use std::collections::BTreeMap;

use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};

use super::Error;
use super::marshal::marshal;
use super::value::{Value, parse};

/// A Go `any` holding decoded JSON.
#[derive(Debug, Clone, PartialEq)]
pub enum AnyValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<AnyValue>),
    Object(BTreeMap<String, AnyValue>),
}

/// Go's `strconv.ParseFloat(s, 64)` over a JSON number literal: `None` when
/// it is out of range (Go's `ErrRange`).
pub(crate) fn parse_f64(s: &str) -> Option<f64> {
    let f: f64 = s.parse().ok()?;
    if f.is_infinite() { None } else { Some(f) }
}

impl AnyValue {
    /// Convert a decoded tree as `json.Unmarshal` into `any` does.
    pub fn from_value(v: &Value) -> Result<AnyValue, Error> {
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
            Ok(match v {
                Value::Null => AnyValue::Null,
                Value::Bool(b) => AnyValue::Bool(*b),
                Value::Number(n) => AnyValue::Number(parse_f64(n).ok_or_else(|| {
                    Error::Type(format!(
                        "json: cannot unmarshal number {n} into Go value of type float64"
                    ))
                })?),
                Value::String(s) => AnyValue::String(s.text.clone()),
                Value::Array(items) => AnyValue::Array(
                    items
                        .iter()
                        .map(AnyValue::from_value)
                        .collect::<Result<_, _>>()?,
                ),
                Value::Object(members) => {
                    let mut m = BTreeMap::new();
                    for (k, v) in members {
                        m.insert(k.text.clone(), AnyValue::from_value(v)?);
                    }
                    AnyValue::Object(m)
                }
            })
        })
    }
}

impl Drop for AnyValue {
    fn drop(&mut self) {
        let mut stack: Vec<AnyValue> = Vec::new();
        match self {
            AnyValue::Array(v) => stack.append(v),
            AnyValue::Object(m) => stack.extend(std::mem::take(m).into_values()),
            _ => return,
        }
        while let Some(mut v) = stack.pop() {
            match &mut v {
                AnyValue::Array(a) => stack.append(a),
                AnyValue::Object(m) => stack.extend(std::mem::take(m).into_values()),
                _ => {}
            }
        }
    }
}

impl Serialize for AnyValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            AnyValue::Null => s.serialize_unit(),
            AnyValue::Bool(b) => s.serialize_bool(*b),
            AnyValue::Number(f) => s.serialize_f64(*f),
            AnyValue::String(t) => s.serialize_str(t),
            AnyValue::Array(items) => {
                let mut seq = s.serialize_seq(Some(items.len()))?;
                for it in items {
                    seq.serialize_element(it)?;
                }
                seq.end()
            }
            AnyValue::Object(m) => {
                let mut map = s.serialize_map(Some(m.len()))?;
                for (k, v) in m {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

/// `json.Unmarshal(data, &any)`.
pub fn unmarshal_any(data: &[u8]) -> Result<AnyValue, Error> {
    AnyValue::from_value(&parse(data)?)
}

/// `json.Marshal` of `json.Unmarshal(data, &any)`.
pub fn round_trip(data: &[u8]) -> Result<Vec<u8>, Error> {
    marshal(&unmarshal_any(data)?)
}
