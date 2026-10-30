//! `gocompat`: one home for the Go behaviours the port must reproduce
//! (architecture.md §1.2). Each module is tested against vectors the Go
//! build produced (`testdata/golden/gocompat/`, exported by
//! `tools/goldens/export.sh` with `go test -overlay`).
//!
//! | Module | Reproduces |
//! |---|---|
//! | [`json`] | `json.Valid`, `Marshal`/`MarshalIndent`/`Encoder`, `Compact`/`Indent`, identity `Canonical`, `Unmarshal` into a struct and into `any` |
//! | [`time`] | `time.Time` RFC3339Nano text, `MarshalJSON`, `UnmarshalJSON` |
//! | [`quote`] | `strconv.Quote` / `%q` |
//! | [`text`] | `unicode.Is*`, simple case mappings, `SimpleFold`, `strings.EqualFold`/`ToLower`/`ToUpper` |
//! | [`filepath`] | `path/filepath` `Clean`, `Join`, `Rel`, `IsAbs`, `VolumeName`, `Abs`, for Unix and Windows |
//! | [`regex`] | RE2's ASCII `\w \d \s \b` on Rust's `regex` |
//! | [`sub`] | a buffered Go `chan` for subscribers: `len`, a send that never blocks, `close` then drain |
//! | [`utf8`] | `utf8.DecodeRune`'s one-U+FFFD-per-bad-byte rule |
//! | [`fnv`] | `hash/fnv` `New32a` |
//! | [`pathmatch`] | `path.Match` and its malformed-pattern rule |
//!
//! Not here yet, though §1.2 lists them: `url` (the `net/url` subset) and
//! `flag`. Their areas add them when they are ported.

pub mod filepath;
pub mod flag;
pub mod fnv;
pub mod json;
pub mod pathmatch;
pub mod quote;
pub mod regex;
pub mod sub;
pub mod text;
pub mod time;
pub mod utf8;
