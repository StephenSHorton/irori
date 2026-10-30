//! Go `json.Marshal` as a serde `Serializer`.
//!
//! What this reproduces (store.md §7.2, inbox.md §7.1, surface.md §7.8):
//! - struct fields in declaration order (serde's order), names escaped like
//!   strings;
//! - map entries sorted by key bytes whatever the map type, integer keys
//!   written as decimal strings and sorted as strings;
//! - strings through [`append_string`]: `<>&` escaped unless
//!   [`Options::escape_html`] is off, U+2028/2029 always escaped;
//! - `serialize_bytes` as standard base64 with padding (Go `[]byte`);
//! - floats by Go's `'f'`/`'e'` rule; NaN and infinities are errors;
//! - [`RawJson`] passed through compacted, HTML-escaped like a Go
//!   `json.RawMessage`;
//! - `None`, `()` and unit structs as `null`.
//!
//! What stays with the caller, because serde has no notion of it:
//! `omitempty` and `omitzero` are `#[serde(skip_serializing_if = ...)]`.
//! Go's `omitempty` never omits a struct or a `time.Time`, so a zero
//! [`GoTime`](crate::time::GoTime) field with `omitempty` is still written;
//! `omitzero` does omit it. A Go nil slice or map is `null`: model it as
//! `Option<Vec<_>>` / `Option<BTreeMap<_, _>>`.

use std::fmt::Display;

use base64::Engine as _;
use serde::ser::{self, Serialize};

use super::Error;
use super::encode::{append_string, format_f32, format_f64};
use super::scanner;

/// The newtype name [`RawJson`] serializes under, which this serializer
/// recognises (the same trick serde_json's `RawValue` uses).
pub const RAW_TOKEN: &str = "$gocompat::json::RawJson";

/// Encoder options.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Go's default `true`; `Encoder.SetEscapeHTML(false)` is `false`.
    pub escape_html: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { escape_html: true }
    }
}

/// Go `json.Marshal(v)`.
pub fn marshal<T: Serialize + ?Sized>(v: &T) -> Result<Vec<u8>, Error> {
    marshal_with(v, Options::default())
}

/// `json.Marshal` with options (e.g. HTML escaping off).
pub fn marshal_with<T: Serialize + ?Sized>(v: &T, opts: Options) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    v.serialize(&mut Serializer {
        out: &mut out,
        opts,
    })?;
    Ok(out)
}

/// Go `json.MarshalIndent(v, prefix, indent)`.
pub fn marshal_indent<T: Serialize + ?Sized>(
    v: &T,
    prefix: &str,
    indent: &str,
) -> Result<Vec<u8>, Error> {
    let raw = marshal(v)?;
    scanner::indent(&raw, prefix, indent)
}

/// Go `json.NewEncoder(w).Encode(v)` with the given options: the value plus
/// a trailing `\n`.
pub fn encode_line<T: Serialize + ?Sized>(v: &T, opts: Options) -> Result<Vec<u8>, Error> {
    let mut out = marshal_with(v, opts)?;
    out.push(b'\n');
    Ok(out)
}

/// A pre-encoded JSON value, Go's `json.RawMessage`. Marshal compacts it and
/// (with HTML escaping on) escapes `<>&` and U+2028/2029 in it. An empty
/// `RawJson` is an error, as in Go; wrap it in `Option` for `null`.
#[derive(Clone, PartialEq, Eq, Hash, Default)]
pub struct RawJson(pub Vec<u8>);

impl std::fmt::Debug for RawJson {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RawJson({})", String::from_utf8_lossy(&self.0))
    }
}

struct RawBytes<'a>(&'a [u8]);

impl Serialize for RawBytes<'_> {
    fn serialize<S: ser::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(self.0)
    }
}

impl Serialize for RawJson {
    fn serialize<S: ser::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_newtype_struct(RAW_TOKEN, &RawBytes(&self.0))
    }
}

impl ser::Error for Error {
    fn custom<T: Display>(msg: T) -> Self {
        Error::Marshal(msg.to_string())
    }
}

/// The serializer. Use [`marshal`] rather than building one.
pub struct Serializer<'a> {
    out: &'a mut Vec<u8>,
    opts: Options,
}

impl<'a> Serializer<'a> {
    fn child<'b>(&'b mut self) -> Serializer<'b> {
        Serializer {
            out: self.out,
            opts: self.opts,
        }
    }

    fn string(&mut self, s: &[u8]) {
        append_string(self.out, s, self.opts.escape_html);
    }
}

fn nested<T: Serialize + ?Sized>(v: &T, opts: Options) -> Result<Vec<u8>, Error> {
    let mut buf = Vec::new();
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
        v.serialize(&mut Serializer {
            out: &mut buf,
            opts,
        })
    })?;
    Ok(buf)
}

impl<'a, 'b> ser::Serializer for &'b mut Serializer<'a> {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = Seq<'a, 'b>;
    type SerializeTuple = Seq<'a, 'b>;
    type SerializeTupleStruct = Seq<'a, 'b>;
    type SerializeTupleVariant = Seq<'a, 'b>;
    type SerializeMap = Map<'a, 'b>;
    type SerializeStruct = Struct<'a, 'b>;
    type SerializeStructVariant = Struct<'a, 'b>;

    fn serialize_bool(self, v: bool) -> Result<(), Error> {
        self.out
            .extend_from_slice(if v { b"true" } else { b"false" });
        Ok(())
    }
    fn serialize_i8(self, v: i8) -> Result<(), Error> {
        self.serialize_i64(i64::from(v))
    }
    fn serialize_i16(self, v: i16) -> Result<(), Error> {
        self.serialize_i64(i64::from(v))
    }
    fn serialize_i32(self, v: i32) -> Result<(), Error> {
        self.serialize_i64(i64::from(v))
    }
    fn serialize_i64(self, v: i64) -> Result<(), Error> {
        self.out.extend_from_slice(v.to_string().as_bytes());
        Ok(())
    }
    fn serialize_i128(self, v: i128) -> Result<(), Error> {
        self.out.extend_from_slice(v.to_string().as_bytes());
        Ok(())
    }
    fn serialize_u8(self, v: u8) -> Result<(), Error> {
        self.serialize_u64(u64::from(v))
    }
    fn serialize_u16(self, v: u16) -> Result<(), Error> {
        self.serialize_u64(u64::from(v))
    }
    fn serialize_u32(self, v: u32) -> Result<(), Error> {
        self.serialize_u64(u64::from(v))
    }
    fn serialize_u64(self, v: u64) -> Result<(), Error> {
        self.out.extend_from_slice(v.to_string().as_bytes());
        Ok(())
    }
    fn serialize_u128(self, v: u128) -> Result<(), Error> {
        self.out.extend_from_slice(v.to_string().as_bytes());
        Ok(())
    }
    fn serialize_f32(self, v: f32) -> Result<(), Error> {
        self.out.extend_from_slice(format_f32(v)?.as_bytes());
        Ok(())
    }
    fn serialize_f64(self, v: f64) -> Result<(), Error> {
        self.out.extend_from_slice(format_f64(v)?.as_bytes());
        Ok(())
    }
    fn serialize_char(self, v: char) -> Result<(), Error> {
        let mut b = [0u8; 4];
        self.string(v.encode_utf8(&mut b).as_bytes());
        Ok(())
    }
    fn serialize_str(self, v: &str) -> Result<(), Error> {
        self.string(v.as_bytes());
        Ok(())
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<(), Error> {
        let b64 = base64::engine::general_purpose::STANDARD.encode(v);
        self.string(b64.as_bytes());
        Ok(())
    }
    fn serialize_none(self) -> Result<(), Error> {
        self.out.extend_from_slice(b"null");
        Ok(())
    }
    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<(), Error> {
        v.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), Error> {
        self.serialize_none()
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Error> {
        self.serialize_none()
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<(), Error> {
        self.serialize_str(variant)
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        v: &T,
    ) -> Result<(), Error> {
        if name == RAW_TOKEN {
            let raw = v.serialize(RawCapture)?;
            let c = scanner::compact(&raw, self.opts.escape_html).map_err(|e| {
                Error::Marshal(format!(
                    "json: error calling MarshalJSON for type json.RawMessage: {e}"
                ))
            })?;
            self.out.extend_from_slice(&c);
            return Ok(());
        }
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        v: &T,
    ) -> Result<(), Error> {
        self.out.push(b'{');
        self.string(variant.as_bytes());
        self.out.push(b':');
        let inner = nested(v, self.opts)?;
        self.out.extend_from_slice(&inner);
        self.out.push(b'}');
        Ok(())
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Seq<'a, 'b>, Error> {
        self.out.push(b'[');
        Ok(Seq {
            ser: self,
            first: true,
            variant: false,
        })
    }
    fn serialize_tuple(self, n: usize) -> Result<Seq<'a, 'b>, Error> {
        self.serialize_seq(Some(n))
    }
    fn serialize_tuple_struct(self, _: &'static str, n: usize) -> Result<Seq<'a, 'b>, Error> {
        self.serialize_seq(Some(n))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Seq<'a, 'b>, Error> {
        self.out.push(b'{');
        self.string(variant.as_bytes());
        self.out.extend_from_slice(b":[");
        Ok(Seq {
            ser: self,
            first: true,
            variant: true,
        })
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Map<'a, 'b>, Error> {
        Ok(Map {
            ser: self,
            entries: Vec::new(),
            key: None,
        })
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Struct<'a, 'b>, Error> {
        self.out.push(b'{');
        Ok(Struct {
            ser: self,
            first: true,
            variant: false,
        })
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Struct<'a, 'b>, Error> {
        self.out.push(b'{');
        self.string(variant.as_bytes());
        self.out.extend_from_slice(b":{");
        Ok(Struct {
            ser: self,
            first: true,
            variant: true,
        })
    }
}

/// Arrays and tuples.
pub struct Seq<'a, 'b> {
    ser: &'b mut Serializer<'a>,
    first: bool,
    variant: bool,
}

impl Seq<'_, '_> {
    fn elem<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Error> {
        if !self.first {
            self.ser.out.push(b',');
        }
        self.first = false;
        let opts = self.ser.opts;
        let mut child = self.ser.child();
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || v.serialize(&mut child))?;
        let _ = opts;
        Ok(())
    }
    fn finish(self) {
        self.ser.out.push(b']');
        if self.variant {
            self.ser.out.push(b'}');
        }
    }
}

impl ser::SerializeSeq for Seq<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Error> {
        self.elem(v)
    }
    fn end(self) -> Result<(), Error> {
        self.finish();
        Ok(())
    }
}

impl ser::SerializeTuple for Seq<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Error> {
        self.elem(v)
    }
    fn end(self) -> Result<(), Error> {
        self.finish();
        Ok(())
    }
}

impl ser::SerializeTupleStruct for Seq<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Error> {
        self.elem(v)
    }
    fn end(self) -> Result<(), Error> {
        self.finish();
        Ok(())
    }
}

impl ser::SerializeTupleVariant for Seq<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Error> {
        self.elem(v)
    }
    fn end(self) -> Result<(), Error> {
        self.finish();
        Ok(())
    }
}

/// Maps: entries are buffered and written sorted by key, as Go does.
pub struct Map<'a, 'b> {
    ser: &'b mut Serializer<'a>,
    entries: Vec<(String, Vec<u8>)>,
    key: Option<String>,
}

impl ser::SerializeMap for Map<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, k: &T) -> Result<(), Error> {
        self.key = Some(k.serialize(KeyCapture)?);
        Ok(())
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Error> {
        let key = self
            .key
            .take()
            .ok_or_else(|| Error::Marshal("map value without a key".into()))?;
        let val = nested(v, self.ser.opts)?;
        self.entries.push((key, val));
        Ok(())
    }
    fn end(mut self) -> Result<(), Error> {
        // Go sorts with strings.Compare (bytes); a stable sort keeps Rust's
        // duplicate keys (impossible in a Go map) in insertion order.
        self.entries
            .sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        self.ser.out.push(b'{');
        for (i, (k, v)) in self.entries.iter().enumerate() {
            if i > 0 {
                self.ser.out.push(b',');
            }
            self.ser.string(k.as_bytes());
            self.ser.out.push(b':');
            self.ser.out.extend_from_slice(v);
        }
        self.ser.out.push(b'}');
        Ok(())
    }
}

/// Structs: fields in serde order.
pub struct Struct<'a, 'b> {
    ser: &'b mut Serializer<'a>,
    first: bool,
    variant: bool,
}

impl Struct<'_, '_> {
    fn field<T: Serialize + ?Sized>(&mut self, key: &'static str, v: &T) -> Result<(), Error> {
        if !self.first {
            self.ser.out.push(b',');
        }
        self.first = false;
        self.ser.string(key.as_bytes());
        self.ser.out.push(b':');
        let mut child = self.ser.child();
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || v.serialize(&mut child))
    }
    fn finish(self) {
        self.ser.out.push(b'}');
        if self.variant {
            self.ser.out.push(b'}');
        }
    }
}

impl ser::SerializeStruct for Struct<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        v: &T,
    ) -> Result<(), Error> {
        self.field(key, v)
    }
    fn end(self) -> Result<(), Error> {
        self.finish();
        Ok(())
    }
}

impl ser::SerializeStructVariant for Struct<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        v: &T,
    ) -> Result<(), Error> {
        self.field(key, v)
    }
    fn end(self) -> Result<(), Error> {
        self.finish();
        Ok(())
    }
}

/// Map keys: strings as they are, integers as decimal (Go `resolveKeyName`).
struct KeyCapture;

fn key_err() -> Error {
    Error::Marshal("json: unsupported map key type".into())
}

impl ser::Serializer for KeyCapture {
    type Ok = String;
    type Error = Error;
    type SerializeSeq = ser::Impossible<String, Error>;
    type SerializeTuple = ser::Impossible<String, Error>;
    type SerializeTupleStruct = ser::Impossible<String, Error>;
    type SerializeTupleVariant = ser::Impossible<String, Error>;
    type SerializeMap = ser::Impossible<String, Error>;
    type SerializeStruct = ser::Impossible<String, Error>;
    type SerializeStructVariant = ser::Impossible<String, Error>;

    fn serialize_str(self, v: &str) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_char(self, v: char) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_i8(self, v: i8) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_i16(self, v: i16) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_i32(self, v: i32) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_i64(self, v: i64) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_u8(self, v: u8) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_u16(self, v: u16) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_u32(self, v: u32) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_u64(self, v: u64) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        v: &T,
    ) -> Result<String, Error> {
        v.serialize(self)
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<String, Error> {
        Ok(variant.to_string())
    }
    fn serialize_bool(self, _: bool) -> Result<String, Error> {
        Err(key_err())
    }
    fn serialize_f32(self, _: f32) -> Result<String, Error> {
        Err(key_err())
    }
    fn serialize_f64(self, _: f64) -> Result<String, Error> {
        Err(key_err())
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<String, Error> {
        Err(key_err())
    }
    fn serialize_none(self) -> Result<String, Error> {
        Err(key_err())
    }
    fn serialize_some<T: Serialize + ?Sized>(self, _: &T) -> Result<String, Error> {
        Err(key_err())
    }
    fn serialize_unit(self) -> Result<String, Error> {
        Err(key_err())
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<String, Error> {
        Err(key_err())
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<String, Error> {
        Err(key_err())
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Error> {
        Err(key_err())
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Error> {
        Err(key_err())
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Error> {
        Err(key_err())
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Error> {
        Err(key_err())
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Error> {
        Err(key_err())
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct, Error> {
        Err(key_err())
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Error> {
        Err(key_err())
    }
}

/// Pulls the bytes out of a [`RawJson`].
struct RawCapture;

fn raw_err() -> Error {
    Error::Marshal("RawJson must serialize as bytes".into())
}

impl ser::Serializer for RawCapture {
    type Ok = Vec<u8>;
    type Error = Error;
    type SerializeSeq = ser::Impossible<Vec<u8>, Error>;
    type SerializeTuple = ser::Impossible<Vec<u8>, Error>;
    type SerializeTupleStruct = ser::Impossible<Vec<u8>, Error>;
    type SerializeTupleVariant = ser::Impossible<Vec<u8>, Error>;
    type SerializeMap = ser::Impossible<Vec<u8>, Error>;
    type SerializeStruct = ser::Impossible<Vec<u8>, Error>;
    type SerializeStructVariant = ser::Impossible<Vec<u8>, Error>;

    fn serialize_bytes(self, v: &[u8]) -> Result<Vec<u8>, Error> {
        Ok(v.to_vec())
    }
    fn serialize_bool(self, _: bool) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_i8(self, _: i8) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_i16(self, _: i16) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_i32(self, _: i32) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_i64(self, _: i64) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_u8(self, _: u8) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_u16(self, _: u16) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_u32(self, _: u32) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_u64(self, _: u64) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_f32(self, _: f32) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_f64(self, _: f64) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_char(self, _: char) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_str(self, _: &str) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_none(self) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_some<T: Serialize + ?Sized>(self, _: &T) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_unit(self) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: &T,
    ) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<Vec<u8>, Error> {
        Err(raw_err())
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Error> {
        Err(raw_err())
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Error> {
        Err(raw_err())
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Error> {
        Err(raw_err())
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Error> {
        Err(raw_err())
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Error> {
        Err(raw_err())
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct, Error> {
        Err(raw_err())
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Error> {
        Err(raw_err())
    }
}
