//! Go `encoding/json` decoding helpers on top of gocompat (capture.md §2.4,
//! §7 trap 5): reading a parsed [`Value`] the way Go fills a struct, and
//! small accessors over decoded [`Out`] values.

use gocompat::json::{Out, Schema, Value, unmarshal_value};
use gocompat::text::fold_name;
use gocompat::time::GoTime;

/// Go's struct field lookup for one key: exact name first, else the first
/// field whose folded name equals the key's.
pub fn field_index(fields: &[&str], key: &str) -> Option<usize> {
    if let Some(i) = fields.iter().position(|n| *n == key) {
        return Some(i);
    }
    let fk = fold_name(key.as_bytes());
    fields.iter().position(|n| fold_name(n.as_bytes()) == fk)
}

/// `json.Unmarshal` into a struct of string fields that keeps going past
/// type errors, as Go does when its caller ignores the error: each member
/// is applied in order, a string sets its field, anything else leaves it.
pub fn lenient_strings(v: &Value, fields: &[&str]) -> Vec<String> {
    let mut out = vec![String::new(); fields.len()];
    if let Value::Object(members) = v {
        for (k, val) in members {
            if let (Some(i), Value::String(s)) = (field_index(fields, &k.text), val) {
                out[i] = s.text.clone();
            }
        }
    }
    out
}

/// A `map[string]json.RawMessage` view of an object: exact keys, the last
/// duplicate wins. `None` when `v` is not an object.
pub fn exact_members(v: &Value) -> Option<Vec<(&str, &Value)>> {
    let Value::Object(members) = v else {
        return None;
    };
    let mut out: Vec<(&str, &Value)> = Vec::new();
    for (k, val) in members {
        if let Some(slot) = out.iter_mut().find(|(n, _)| *n == k.text) {
            slot.1 = val;
        } else {
            out.push((&k.text, val));
        }
    }
    Some(out)
}

/// The value at an exact key of an [`exact_members`] view.
pub fn get<'a>(m: &[(&str, &'a Value)], key: &str) -> Option<&'a Value> {
    m.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// `json.Unmarshal(raw, &field)` with the error ignored: the decoded value,
/// or the zero value when decoding failed.
pub fn field_or_zero(v: Option<&Value>, s: &Schema) -> Out {
    match v {
        Some(v) => unmarshal_value(v, s).unwrap_or_else(|_| Out::zero(s)),
        None => Out::zero(s),
    }
}

// --- Out accessors ---

pub fn out_str(o: Option<&Out>) -> String {
    match o {
        Some(Out::String(s)) => s.clone(),
        Some(Out::Ptr(Some(b))) => out_str(Some(b)),
        _ => String::new(),
    }
}

pub fn out_bool(o: Option<&Out>) -> bool {
    matches!(o, Some(Out::Bool(true)))
}

pub fn out_int(o: Option<&Out>) -> i64 {
    match o {
        Some(Out::Int(n)) => *n,
        Some(Out::Uint(n)) => *n as i64,
        _ => 0,
    }
}

pub fn out_time(o: Option<&Out>) -> GoTime {
    match o {
        Some(Out::Time(t)) => *t,
        _ => GoTime::ZERO,
    }
}

/// A `RawMessage` field's bytes (`None` when nil).
pub fn out_raw(o: Option<&Out>) -> Option<Vec<u8>> {
    match o {
        Some(Out::Raw(r)) => r.clone(),
        _ => None,
    }
}

/// A `*string` field (`None` when nil).
pub fn out_opt_str(o: Option<&Out>) -> Option<String> {
    match o {
        Some(Out::Ptr(Some(b))) => match &**b {
            Out::String(s) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// A `[]string` field: `None` when nil.
pub fn out_strings(o: Option<&Out>) -> Option<Vec<String>> {
    match o {
        Some(Out::Slice(Some(v))) => Some(v.iter().map(|x| out_str(Some(x))).collect()),
        _ => None,
    }
}

/// The items of a slice field (empty when nil).
pub fn out_items(o: Option<&Out>) -> &[Out] {
    match o {
        Some(Out::Slice(Some(v))) => v,
        _ => &[],
    }
}

/// `bytes.TrimSpace`.
pub fn trim_space_bytes(b: &[u8]) -> &[u8] {
    match std::str::from_utf8(b) {
        Ok(s) => {
            let t = trim_space(s);
            let start = t.as_ptr() as usize - s.as_ptr() as usize;
            &b[start..start + t.len()]
        }
        Err(_) => {
            // Invalid UTF-8: trim ASCII space only (enough for JSON input).
            let start = b
                .iter()
                .position(|c| !c.is_ascii_whitespace())
                .unwrap_or(b.len());
            let end = b
                .iter()
                .rposition(|c| !c.is_ascii_whitespace())
                .map_or(start, |i| i + 1);
            &b[start..end.max(start)]
        }
    }
}

/// `strings.TrimSpace` (Go's `unicode.IsSpace`).
pub fn trim_space(s: &str) -> &str {
    s.trim_matches(gocompat::text::is_space)
}
