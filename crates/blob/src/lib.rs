//! Port of Go `internal/blob` (capture.md §0 I4, §2.6, §3 "attach-path
//! rules", §4, §7.15):
//!
//! - [`Store`]: capture's content-addressed store for patches and
//!   compaction summaries, at `<dir>/<first 2 hex>/<sha256 hex>`,
//!   gzip-compressed, written once and never changed. The id is the SHA-256
//!   of the stored (possibly truncated) content, before compression.
//! - [`Files`]: the attachment store, same layout, stored as is (no
//!   compression, no cap below 256 MiB, nothing scrubbed), with resumable
//!   partial fetches in `<dir>/partial/`.
//! - [`check_attach_path`], [`safe_name`], [`within`]: the never-leave rules
//!   and the name a row's attachment may be written under.
//! - [`mark_downloaded`]: Mark of the Web (Windows `Zone.Identifier`
//!   stream, macOS quarantine attribute).
//!
//! Both stores are safe for two processes at once (Go and Rust side by
//! side): a final file is never replaced, and an existing one is success.

mod attachpath;
mod error;
mod files;
mod fsutil;
mod mime;
mod motw;
mod store;

pub use attachpath::{RECEIVED_DIR, check_attach_path, safe_name, within};
pub use error::{Error, ErrorKind, Result};
pub use files::{FileRef, Files, MAX_FILE_SIZE, same_content};
pub use mime::{detect_content_type, mime_of, type_by_extension};
pub use motw::mark_downloaded;
pub use store::{MAX_SIZE, Store, TRUNCATED_MARKER, valid_id};

/// Unexported in Go; public here for the golden replays.
#[doc(hidden)]
pub use attachpath::never_leaves;
#[doc(hidden)]
pub use store::truncate;
