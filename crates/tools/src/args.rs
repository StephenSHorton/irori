//! Strict argument decoding (I:internal/tools/set.go `decode`): Go's
//! `json.Decoder` with `DisallowUnknownFields` into a tool's args struct.
//!
//! Reproduced: a key matches a field by its exact JSON name, else by Go's
//! case fold; every key applies in order (the last wins); `null` leaves a
//! field alone (a pointer or slice becomes nil); an unknown key or a type
//! mismatch is kept as the first error and decoding goes on, so the first
//! one in document order is reported. A step that is neither a string nor
//! an object fails at once (Go's `stepArg.UnmarshalJSON` returns its error
//! up the stack), and that error wins over one kept earlier.

use std::collections::BTreeMap;

use gocompat::json::Value;
use gocompat::text::fold_name;

/// A field's Go type.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Ty {
    Str,
    /// `*string`.
    OptStr,
    Bool,
    /// `*bool`.
    OptBool,
    Int,
    /// `*int`.
    OptInt,
    /// `[]string`.
    Strs,
    /// `[]stepArg`: a string or `{id, text, done, doing}` each.
    Steps,
    /// `[]struct{...}` decoded strictly with these fields.
    Structs(&'static [(&'static str, Ty)]),
}

/// One step argument.
#[derive(Debug, Clone, Default)]
pub(crate) struct StepArg {
    pub id: String,
    pub text: String,
    pub done: bool,
    pub doing: bool,
}

/// A decoded value.
#[derive(Debug, Clone)]
pub(crate) enum V {
    Str(String),
    OptStr(Option<String>),
    Bool(bool),
    OptBool(Option<bool>),
    Int(i64),
    OptInt(Option<i64>),
    Strs(Vec<String>),
    Steps(Vec<StepArg>),
    Structs(Vec<Args>),
}

fn zero(t: Ty) -> V {
    match t {
        Ty::Str => V::Str(String::new()),
        Ty::OptStr => V::OptStr(None),
        Ty::Bool => V::Bool(false),
        Ty::OptBool => V::OptBool(None),
        Ty::Int => V::Int(0),
        Ty::OptInt => V::OptInt(None),
        Ty::Strs => V::Strs(vec![]),
        Ty::Steps => V::Steps(vec![]),
        Ty::Structs(_) => V::Structs(vec![]),
    }
}

/// A decoded args struct.
#[derive(Debug, Clone, Default)]
pub(crate) struct Args(BTreeMap<&'static str, V>);

impl Args {
    pub fn str(&self, k: &str) -> &str {
        match self.0.get(k) {
            Some(V::Str(s)) => s,
            _ => "",
        }
    }
    pub fn opt_str(&self, k: &str) -> Option<String> {
        match self.0.get(k) {
            Some(V::OptStr(s)) => s.clone(),
            _ => None,
        }
    }
    pub fn bool(&self, k: &str) -> bool {
        matches!(self.0.get(k), Some(V::Bool(true)))
    }
    pub fn opt_bool(&self, k: &str) -> Option<bool> {
        match self.0.get(k) {
            Some(V::OptBool(b)) => *b,
            _ => None,
        }
    }
    pub fn int(&self, k: &str) -> i64 {
        match self.0.get(k) {
            Some(V::Int(n)) => *n,
            _ => 0,
        }
    }
    pub fn opt_int(&self, k: &str) -> Option<i64> {
        match self.0.get(k) {
            Some(V::OptInt(n)) => *n,
            _ => None,
        }
    }
    pub fn strs(&self, k: &str) -> Vec<String> {
        match self.0.get(k) {
            Some(V::Strs(v)) => v.clone(),
            _ => vec![],
        }
    }
    pub fn steps(&self, k: &str) -> Vec<StepArg> {
        match self.0.get(k) {
            Some(V::Steps(v)) => v.clone(),
            _ => vec![],
        }
    }
    pub fn structs(&self, k: &str) -> Vec<Args> {
        match self.0.get(k) {
            Some(V::Structs(v)) => v.clone(),
            _ => vec![],
        }
    }
}

/// Go's `kindName` over the Go kind of the type a value didn't fit.
fn kind_name(t: Ty) -> &'static str {
    match t {
        Ty::Str | Ty::OptStr => "a string",
        Ty::Bool | Ty::OptBool => "true or false",
        Ty::Int | Ty::OptInt => "a whole number",
        Ty::Strs | Ty::Steps | Ty::Structs(_) => "an array of strings",
    }
}

struct Dec {
    /// The first error kept (Go `saveError`).
    saved: Option<String>,
}

impl Dec {
    fn save(&mut self, msg: String) {
        if self.saved.is_none() {
            self.saved = Some(msg);
        }
    }

    fn type_err(&mut self, path: &[&str], kind: &str) {
        self.save(format!("argument {} should be {kind}", path.join(".")));
    }

    /// Decodes an object into a struct with `fields`. `Err` is a hard
    /// error (a bad step) that ends the whole decode.
    fn object(
        &mut self,
        v: &Value,
        fields: &'static [(&'static str, Ty)],
        path: &mut Vec<&'static str>,
        what: Option<&'static str>,
    ) -> Result<Args, String> {
        let mut out = Args::default();
        for (name, t) in fields {
            out.0.insert(name, zero(*t));
        }
        let Value::Object(kv) = v else {
            // A non-object into a struct: Go's kind name for a struct.
            if !matches!(v, Value::Null) {
                let _ = what;
                self.save(format!("argument {} should be struct", path.join(".")));
            }
            return Ok(out);
        };
        for (k, val) in kv {
            let key = k.text.as_str();
            let field = fields.iter().find(|(n, _)| *n == key).or_else(|| {
                let fk = fold_name(key.as_bytes());
                fields.iter().find(|(n, _)| fold_name(n.as_bytes()) == fk)
            });
            let Some((name, t)) = field else {
                self.save(format!("unknown argument {}", gocompat::quote::quote(key)));
                continue;
            };
            path.push(name);
            let r = self.value(val, *t, path, out.0.get(name).cloned());
            path.pop();
            if let Some(nv) = r? {
                out.0.insert(name, nv);
            }
        }
        Ok(out)
    }

    /// One field's value; `None` leaves the field as it was.
    fn value(
        &mut self,
        v: &Value,
        t: Ty,
        path: &mut Vec<&'static str>,
        _cur: Option<V>,
    ) -> Result<Option<V>, String> {
        if let Value::Null = v {
            return Ok(match t {
                Ty::OptStr => Some(V::OptStr(None)),
                Ty::OptBool => Some(V::OptBool(None)),
                Ty::OptInt => Some(V::OptInt(None)),
                Ty::Strs => Some(V::Strs(vec![])),
                Ty::Steps => Some(V::Steps(vec![])),
                Ty::Structs(_) => Some(V::Structs(vec![])),
                _ => None,
            });
        }
        match (t, v) {
            (Ty::Str, Value::String(s)) => Ok(Some(V::Str(s.text.clone()))),
            (Ty::OptStr, Value::String(s)) => Ok(Some(V::OptStr(Some(s.text.clone())))),
            (Ty::Bool, Value::Bool(b)) => Ok(Some(V::Bool(*b))),
            (Ty::OptBool, Value::Bool(b)) => Ok(Some(V::OptBool(Some(*b)))),
            (Ty::Int | Ty::OptInt, Value::Number(n)) => match n.parse::<i64>() {
                Ok(i) if !n.starts_with('+') => Ok(Some(if matches!(t, Ty::Int) {
                    V::Int(i)
                } else {
                    V::OptInt(Some(i))
                })),
                _ => {
                    self.type_err(path, kind_name(t));
                    Ok(None)
                }
            },
            (Ty::Strs, Value::Array(items)) => {
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    match it {
                        Value::String(s) => out.push(s.text.clone()),
                        Value::Null => out.push(String::new()),
                        _ => {
                            self.type_err(path, "a string");
                            out.push(String::new());
                        }
                    }
                }
                Ok(Some(V::Strs(out)))
            }
            (Ty::Steps, Value::Array(items)) => {
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    out.push(step(it, path)?);
                }
                Ok(Some(V::Steps(out)))
            }
            (Ty::Structs(fields), Value::Array(items)) => {
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    let what = path.last().copied();
                    out.push(self.object(it, fields, path, what)?);
                }
                Ok(Some(V::Structs(out)))
            }
            _ => {
                self.type_err(path, kind_name(t));
                Ok(None)
            }
        }
    }
}

/// One `stepArg`: a string, else Go's plain `json.Unmarshal` into the
/// struct (case-folded keys, unknown keys skipped, first type error).
fn step(v: &Value, path: &[&str]) -> Result<StepArg, String> {
    let mut st = StepArg::default();
    match v {
        Value::Null => return Ok(st),
        Value::String(s) => {
            st.text = s.text.clone();
            return Ok(st);
        }
        Value::Object(_) => {}
        _ => return Err(format!("argument {} should be struct", path.join("."))),
    }
    let Value::Object(kv) = v else { unreachable!() };
    let names = ["id", "text", "done", "doing"];
    let mut err: Option<String> = None;
    for (k, val) in kv {
        let key = k.text.as_str();
        let name = names.iter().find(|n| **n == key).or_else(|| {
            let fk = fold_name(key.as_bytes());
            names.iter().find(|n| fold_name(n.as_bytes()) == fk)
        });
        let Some(name) = name else { continue };
        let bad = |kind: &str| format!("argument {}.{name} should be {kind}", path.join("."));
        match (*name, val) {
            (_, Value::Null) => {}
            ("id", Value::String(s)) => st.id = s.text.clone(),
            ("text", Value::String(s)) => st.text = s.text.clone(),
            ("done", Value::Bool(b)) => st.done = *b,
            ("doing", Value::Bool(b)) => st.doing = *b,
            ("id" | "text", _) => {
                err.get_or_insert_with(|| bad("a string"));
            }
            _ => {
                err.get_or_insert_with(|| bad("true or false"));
            }
        }
    }
    match err {
        Some(e) => Err(e),
        None => Ok(st),
    }
}

/// Decodes `raw` (a JSON object, already checked valid) into `fields`.
/// The error is the text after "Error: ".
pub(crate) fn decode(raw: &[u8], fields: &'static [(&'static str, Ty)]) -> Result<Args, String> {
    let v = gocompat::json::parse(raw).map_err(|e| e.to_string())?;
    let mut d = Dec { saved: None };
    let mut path = Vec::new();
    let out = d.object(&v, fields, &mut path, None)?;
    match d.saved {
        Some(e) => Err(e),
        None => Ok(out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: &[(&str, Ty)] = &[
        ("text", Ty::Str),
        ("minutes", Ty::OptInt),
        ("all", Ty::Bool),
        ("attach", Ty::Strs),
        ("steps", Ty::Steps),
        (
            "options",
            Ty::Structs(&[("answer", Ty::Str), ("recommended", Ty::Bool)]),
        ),
    ];

    fn err(raw: &str) -> String {
        decode(raw.as_bytes(), F).unwrap_err()
    }

    #[test]
    fn errors_read_as_go_does() {
        assert_eq!(err(r#"{"bogus":1}"#), r#"unknown argument "bogus""#);
        assert_eq!(
            err(r#"{"minutes":"soon"}"#),
            "argument minutes should be a whole number"
        );
        assert_eq!(
            err(r#"{"minutes":1.5}"#),
            "argument minutes should be a whole number"
        );
        assert_eq!(err(r#"{"text":5}"#), "argument text should be a string");
        assert_eq!(
            err(r#"{"all":"yes"}"#),
            "argument all should be true or false"
        );
        assert_eq!(
            err(r#"{"attach":"x"}"#),
            "argument attach should be an array of strings"
        );
        assert_eq!(
            err(r#"{"attach":[1]}"#),
            "argument attach should be a string"
        );
        assert_eq!(
            err(r#"{"options":[{"answer":1}]}"#),
            "argument options.answer should be a string"
        );
        assert_eq!(err(r#"{"options":[{"x":1}]}"#), r#"unknown argument "x""#);
        assert_eq!(err(r#"{"steps":[1]}"#), "argument steps should be struct");
        assert_eq!(
            err(r#"{"bogus":1,"steps":[{"done":"x"}]}"#),
            "argument steps.done should be true or false"
        );
        assert_eq!(
            err(r#"{"bogus":1,"text":5}"#),
            r#"unknown argument "bogus""#
        );
    }

    #[test]
    fn keys_fold_and_last_wins() {
        let a = decode(
            br#"{"TEXT":"a","text":"b","Minutes":3,"steps":["x",{"Text":"y","done":true}]}"#,
            F,
        )
        .unwrap();
        assert_eq!(a.str("text"), "b");
        assert_eq!(a.opt_int("minutes"), Some(3));
        let st = a.steps("steps");
        assert_eq!(st[0].text, "x");
        assert!(st[1].done && st[1].text == "y");
        let a = decode(br#"{"minutes":null,"text":null}"#, F).unwrap();
        assert_eq!(a.opt_int("minutes"), None);
        assert_eq!(a.str("text"), "");
    }
}
