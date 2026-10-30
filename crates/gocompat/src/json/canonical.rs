//! identity's `Canonical` (I:internal/identity/rows.go:111-129):
//! `json.Marshal(v)`, decode into `any` with `UseNumber`, re-encode with
//! `SetEscapeHTML(false)`, trim the trailing newline.
//!
//! The result is compact JSON with object keys sorted bytewise at every
//! level (a duplicate key keeps its last value, as a Go map does), numbers
//! copied literally, `<>&` raw, U+2028/2029 escaped, `\b` and `\f` in short
//! form, and invalid UTF-8 as `�` (identity.md §3.2, §3.4).

use std::collections::BTreeMap;

use serde::Serialize;

use super::Error;
use super::encode::append_string;
use super::marshal::marshal;
use super::value::{Value, parse};

/// `Canonical(v)` for a value Rust marshals the Go way first.
pub fn canonical<T: Serialize + ?Sized>(v: &T) -> Result<Vec<u8>, Error> {
    let raw = marshal(v)?;
    canonical_raw(&raw)
}

/// `Canonical(json.RawMessage(raw))`: the canonical form of JSON bytes.
pub fn canonical_raw(raw: &[u8]) -> Result<Vec<u8>, Error> {
    let v = parse(raw)?;
    let mut out = Vec::with_capacity(raw.len());
    write(&mut out, &v);
    Ok(out)
}

fn write(out: &mut Vec<u8>, v: &Value) {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || match v {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(n) => out.extend_from_slice(n.as_bytes()),
        Value::String(s) => append_string(out, s.text.as_bytes(), false),
        Value::Array(items) => {
            out.push(b'[');
            for (i, it) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write(out, it);
            }
            out.push(b']');
        }
        Value::Object(members) => {
            let mut m: BTreeMap<&[u8], &Value> = BTreeMap::new();
            for (k, v) in members {
                m.insert(k.text.as_bytes(), v);
            }
            out.push(b'{');
            for (i, (k, v)) in m.into_iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                append_string(out, k, false);
                out.push(b':');
                write(out, v);
            }
            out.push(b'}');
        }
    })
}
