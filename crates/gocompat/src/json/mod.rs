//! Go `encoding/json` semantics (architecture.md §1.2): validation, encoding,
//! identity's canonical form, lenient struct decoding, and the `any` round
//! trip. Every module is tested against Go-produced vectors in
//! `testdata/golden/gocompat/`.
//!
//! Stored row payloads are never re-encoded (store.md §7.1); these functions
//! are for checking them, for writing new payloads and files the Go way, and
//! for reading them the way Go reads them.

pub mod anymap;
pub mod canonical;
pub mod encode;
pub mod lenient;
pub mod marshal;
pub mod scanner;
pub mod value;

pub use anymap::{AnyValue, round_trip, unmarshal_any};
pub use canonical::{canonical, canonical_raw};
pub use encode::{append_string, format_f32, format_f64, quote_string};
pub use lenient::{Out, Schema, unmarshal, unmarshal_value};
pub use marshal::{Options, RawJson, encode_line, marshal, marshal_indent, marshal_with};
pub use scanner::{MAX_NESTING_DEPTH, check_valid, compact, indent, valid};
pub use value::{JsonStr, Value, parse};

/// Errors, with Go's message texts where Go has one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Go's `SyntaxError` text, e.g. `invalid character 'x' looking for
    /// beginning of value` or `unexpected end of JSON input`.
    #[error("{0}")]
    Syntax(String),
    /// Go's `UnsupportedValueError` (NaN, ±Inf).
    #[error("json: unsupported value: {0}")]
    UnsupportedValue(String),
    /// A decode type error (`UnmarshalTypeError`, or a time that does not
    /// parse).
    #[error("{0}")]
    Type(String),
    /// Anything else a `Serialize` impl reported.
    #[error("{0}")]
    Marshal(String),
}
