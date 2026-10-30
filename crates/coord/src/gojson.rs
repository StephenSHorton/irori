//! Row payloads in and out, with Go's `encoding/json` rules (coord.md §2.1).
//!
//! Decoding is `json.Unmarshal` into a Go struct, through
//! `gocompat::json::lenient`: case-insensitive keys, the last duplicate
//! wins, unknown fields dropped, `null` leaves a field zero, and a type
//! error anywhere fails the whole payload (the fold then drops the row).
//! [`go_struct!`] declares a struct once and derives both its `Serialize`
//! (Go field order, with each field's `omitempty` written as a serde
//! `skip_serializing_if`) and its [`GoDecode`] schema.
//!
//! Writing goes through `store::Store::append`, which marshals with
//! `gocompat::json::marshal`, so new rows escape `<>&` and format times as
//! Go does. A row read from the log is never re-marshalled.

use std::collections::BTreeMap;

use gocompat::json::{self, Out, Schema};
use gocompat::time::GoTime;

/// A Go type decoded with `json.Unmarshal` semantics.
pub trait GoDecode: Sized + Default {
    fn schema() -> Schema;
    fn from_out(o: &Out) -> Self;
}

/// `json.Unmarshal(payload, &v)`; `None` where Go returns an error.
pub fn decode<T: GoDecode>(payload: &[u8]) -> Option<T> {
    json::unmarshal(payload, &T::schema())
        .ok()
        .map(|o| T::from_out(&o))
}

/// A struct field of `o` by JSON name, decoded (zero when absent).
pub fn field<T: GoDecode>(o: &Out, name: &str) -> T {
    o.get(name).map(T::from_out).unwrap_or_default()
}

impl GoDecode for String {
    fn schema() -> Schema {
        Schema::String
    }
    fn from_out(o: &Out) -> Self {
        o.str().to_string()
    }
}

impl GoDecode for bool {
    fn schema() -> Schema {
        Schema::Bool
    }
    fn from_out(o: &Out) -> Self {
        matches!(o, Out::Bool(true))
    }
}

impl GoDecode for i64 {
    fn schema() -> Schema {
        Schema::Int(64)
    }
    fn from_out(o: &Out) -> Self {
        match o {
            Out::Int(v) => *v,
            _ => 0,
        }
    }
}

impl GoDecode for f64 {
    fn schema() -> Schema {
        Schema::Float(64)
    }
    fn from_out(o: &Out) -> Self {
        match o {
            Out::Float(v) => *v,
            _ => 0.0,
        }
    }
}

impl GoDecode for GoTime {
    fn schema() -> Schema {
        Schema::Time
    }
    fn from_out(o: &Out) -> Self {
        match o {
            Out::Time(t) => *t,
            _ => GoTime::ZERO,
        }
    }
}

impl<T: GoDecode> GoDecode for Vec<T> {
    fn schema() -> Schema {
        Schema::Slice(Box::new(T::schema()))
    }
    fn from_out(o: &Out) -> Self {
        o.items().iter().map(T::from_out).collect()
    }
}

/// A Go pointer (`*T`): `None` is nil.
impl<T: GoDecode> GoDecode for Option<T> {
    fn schema() -> Schema {
        Schema::Ptr(Box::new(T::schema()))
    }
    fn from_out(o: &Out) -> Self {
        match o {
            Out::Ptr(Some(inner)) => Some(T::from_out(inner)),
            _ => None,
        }
    }
}

impl<T: GoDecode> GoDecode for BTreeMap<String, T> {
    fn schema() -> Schema {
        Schema::Map(Box::new(T::schema()))
    }
    fn from_out(o: &Out) -> Self {
        match o {
            Out::Map(Some(m)) => m.iter().map(|(k, v)| (k.clone(), T::from_out(v))).collect(),
            _ => BTreeMap::new(),
        }
    }
}

/// Go's `omitempty` test for the field types the payloads use.
pub trait Empty {
    fn is_empty_go(&self) -> bool;
}

impl Empty for String {
    fn is_empty_go(&self) -> bool {
        self.is_empty()
    }
}
impl Empty for bool {
    fn is_empty_go(&self) -> bool {
        !*self
    }
}
impl Empty for i64 {
    fn is_empty_go(&self) -> bool {
        *self == 0
    }
}
impl Empty for f64 {
    fn is_empty_go(&self) -> bool {
        *self == 0.0
    }
}
impl<T> Empty for Vec<T> {
    fn is_empty_go(&self) -> bool {
        self.is_empty()
    }
}
impl<T> Empty for Option<T> {
    fn is_empty_go(&self) -> bool {
        self.is_none()
    }
}
impl<K, V> Empty for BTreeMap<K, V> {
    fn is_empty_go(&self) -> bool {
        self.is_empty()
    }
}

/// `omitempty`, as a serde `skip_serializing_if`.
pub fn omit<T: Empty>(v: &T) -> bool {
    v.is_empty_go()
}

/// `omitzero` on a `time.Time`.
pub fn omit_zero_time(t: &GoTime) -> bool {
    t.is_zero()
}

/// Serializes a `Vec` as Go marshals a nil slice when it is empty: `null`.
pub fn nil_vec<T: serde::Serialize, S: serde::Serializer>(
    v: &[T],
    s: S,
) -> Result<S::Ok, S::Error> {
    if v.is_empty() {
        s.serialize_none()
    } else {
        s.collect_seq(v)
    }
}

/// Fields written after an embedded struct's (Go struct embedding): the
/// base marshals with Go's rules, then these follow in order. Only
/// `gocompat::json::marshal` writes the result verbatim, as with every
/// raw value.
#[derive(Default)]
pub struct Extra {
    fields: Vec<(&'static str, Vec<u8>)>,
}

impl Extra {
    pub fn field<T: serde::Serialize + ?Sized, E: serde::ser::Error>(
        &mut self,
        name: &'static str,
        v: &T,
    ) -> Result<(), E> {
        let raw = json::marshal(v).map_err(E::custom)?;
        self.fields.push((name, raw));
        Ok(())
    }

    pub fn embed<S: serde::Serializer, B: serde::Serialize>(
        self,
        s: S,
        base: &B,
    ) -> Result<S::Ok, S::Error> {
        use serde::Serialize as _;
        let mut out = json::marshal(base).map_err(serde::ser::Error::custom)?;
        if out.last() != Some(&b'}') {
            return Err(serde::ser::Error::custom("embedded value is not an object"));
        }
        out.pop();
        for (k, v) in self.fields {
            if out.len() > 1 {
                out.push(b',');
            }
            out.push(b'"');
            out.extend_from_slice(k.as_bytes());
            out.extend_from_slice(b"\":");
            out.extend_from_slice(&v);
        }
        out.push(b'}');
        json::RawJson(out).serialize(s)
    }
}

/// Declares a Go struct: its fields in Go's declaration order, each with
/// its JSON name, and `omitempty` where Go has it, spelled as the path of
/// a skip function after the name (`"omit"` for `omitempty`,
/// `"omit_zero_time"` for `omitzero`; the path resolves where the
/// struct is declared). Derives `Serialize` and
/// [`GoDecode`].
///
/// ```ignore
/// go_struct! {
///     #[derive(Debug, Clone, Default, PartialEq)]
///     pub struct Step {
///         pub id: String = "id",
///         pub done: bool = "done",
///         pub doing: bool = "doing" "omit",
///     }
/// }
/// ```
#[macro_export]
macro_rules! go_struct {
    (
        $(#[$m:meta])*
        $vis:vis struct $name:ident {
            $(
                $(#[$fm:meta])*
                $fvis:vis $f:ident : $ty:ty = $json:literal $($skip:literal)?
            ),* $(,)?
        }
    ) => {
        $(#[$m])*
        #[derive(serde::Serialize)]
        $vis struct $name {
            $(
                $(#[$fm])*
                #[serde(rename = $json)]
                $(#[serde(skip_serializing_if = $skip)])?
                $fvis $f: $ty,
            )*
        }

        impl $crate::gojson::GoDecode for $name {
            fn schema() -> gocompat::json::Schema {
                gocompat::json::Schema::Struct(vec![
                    $(($json, <$ty as $crate::gojson::GoDecode>::schema()),)*
                ])
            }
            fn from_out(o: &gocompat::json::Out) -> Self {
                #[allow(clippy::needless_update)]
                $name {
                    $($f: $crate::gojson::field::<$ty>(o, $json),)*
                }
            }
        }
    };
}
