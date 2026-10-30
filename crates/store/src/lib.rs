//! `irori-store`: Irori's per-device event log, a port of
//! I:internal/store (store.md).
//!
//! One SQLite database holds append-only rows `(device_id, seq, hlc, actor,
//! kind, payload, sig)`. Rows never change once written, so replicas never
//! conflict: devices swap `{device_id: max seq}` vectors and send each other
//! the rows the other lacks. Every row this device appends is signed by a
//! [`Signer`], and every replicated row is checked by a [`Verifier`] before
//! it is stored. Device-local kinds live in a second table, `local_events`.
//! A shared board's log is the same store opened with
//! [`OpenOptions::board`], which adds the allow-list, the actor shape and
//! reference closure.
//!
//! Compatibility (architecture.md §0, §5.1): the Rust store opens the files a
//! Go build wrote, in place, and leaves them openable by Go. It writes schema
//! v3 exactly as Go does and adds nothing (decision D8: no Rust-only
//! tables until cutover). Payload bytes are stored and returned exactly,
//! never re-encoded, so [`signing_bytes`] and [`row_hash`] match Go's for
//! every row (store.md §2.5, §7.1).
//!
//! The store holds no keys and does no ed25519: the identity area
//! implements [`Signer`] and [`Verifier`].
//!
//! The API is synchronous. SQLite calls block; async callers run them on a
//! blocking thread (architecture.md §2). Subscriptions are
//! [`gocompat::sub::Queue`]s, readable from a thread or a task.

mod board;
mod db;
mod error;
pub mod kinds;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde::ser::SerializeStruct;
use sha2::{Digest, Sha256};

pub use board::BoardCounts;
pub use db::{Store, Subscription, schema_version};
pub use error::{Error, ErrorKind, Result};
pub use hlc::{Clock, Timestamp};
pub use kinds::{
    Board, Side, board_accepted, board_actor_ok, is_board_identity_kind, is_local, is_person_kind,
    kind_board, kind_side,
};

/// One log row; `(device_id, seq)` is the primary key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Event {
    pub device_id: String,
    /// 1-based, gapless per device. Zero only in a resync signal.
    pub seq: i64,
    pub hlc: Timestamp,
    /// Who caused it: `irori`, `agent:<device>/<sessionId>`, `user`, ...
    pub actor: String,
    /// Dotted, e.g. `agent.seen` (see [`kinds`]).
    pub kind: String,
    /// The exact stored JSON bytes. Empty only for a cut-hash fork's
    /// other row or a zero Event; Go renders it as `null`.
    pub payload: Vec<u8>,
    /// The writer's ed25519 signature over [`signing_bytes`]; empty when
    /// unsigned. Never in JSON.
    pub sig: Vec<u8>,
}

impl Event {
    /// The row's position in the log's total `(hlc, device_id, seq)` order.
    pub fn cursor(&self) -> Cursor {
        Cursor {
            hlc: self.hlc,
            device_id: self.device_id.clone(),
            seq: self.seq,
        }
    }

    /// A zero Event: a subscriber's resync signal.
    pub fn is_resync(&self) -> bool {
        self.seq == 0
    }
}

/// `{"deviceId","seq","hlc","actor","kind","payload"}` with the payload
/// bytes as they are (Go's `/v1/events` shape). Marshal it with
/// `gocompat::json::marshal` for Go's bytes: the payload is a raw value,
/// which only that serializer writes verbatim.
impl serde::Serialize for Event {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("Event", 6)?;
        st.serialize_field("deviceId", &self.device_id)?;
        st.serialize_field("seq", &self.seq)?;
        st.serialize_field("hlc", &self.hlc)?;
        st.serialize_field("actor", &self.actor)?;
        st.serialize_field("kind", &self.kind)?;
        if self.payload.is_empty() {
            st.serialize_field("payload", &Option::<()>::None)?;
        } else {
            st.serialize_field("payload", &gocompat::json::RawJson(self.payload.clone()))?;
        }
        st.end()
    }
}

/// A position in the `(hlc, device_id, seq)` order; pages [`Store::list`]
/// without dropping rows two devices wrote at one HLC.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Cursor {
    pub hlc: Timestamp,
    pub device_id: String,
    pub seq: i64,
}

impl Cursor {
    /// An unset cursor.
    pub fn is_zero(&self) -> bool {
        self.hlc.0 == 0 && self.device_id.is_empty() && self.seq == 0
    }
}

/// Device id to the highest seq held for it.
pub type Vector = BTreeMap<String, i64>;

/// Narrows [`Store::list`]. Zero values mean "any".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// Exact kinds, or prefixes ending in `.` (`agent.`).
    pub kinds: Vec<String>,
    pub device_id: String,
    /// Strictly after. Skips HLC ties at a page boundary; page with `after`.
    pub after_hlc: Timestamp,
    /// Strictly after this position in `(hlc, device_id, seq)` order, even
    /// with `desc` (Go does not flip it).
    pub after: Cursor,
    /// Seq strictly after; meant with `device_id`.
    pub after_seq: i64,
    /// Default 200, max 1000.
    pub limit: i64,
    /// Newest first.
    pub desc: bool,
    /// Read `local_events` instead of `events`.
    pub local: bool,
}

/// Signs this device's rows (Go `Signer`). `msg` is [`signing_bytes`].
pub trait Signer: Send + Sync {
    fn sign_row(&self, msg: &[u8]) -> Result<Vec<u8>>;
}

/// Checks rows from other devices and keeps the cert fold current (Go
/// `Verifier`; the identity area's Chain and BoardVerifier implement it).
/// The store serializes every call.
pub trait Verifier: Send + Sync {
    /// `Ok` when `ev` is signed by a device certified to this device's
    /// person. Errors must have kind `NoCert` (no cert yet: hold the row),
    /// `BadSig`, or in a board log `BoardActor`.
    fn verify_row(&self, ev: &Event) -> Result<()>;
    /// The effective revocation cut of `device_id`; `None` if not revoked.
    fn cut(&self, device_id: &str) -> Option<i64>;
    /// For every honoured revoke that names one, the row at its cut by hash.
    fn cut_claims(&self) -> Vec<CutClaim>;
    /// Folds one committed identity row.
    fn observe(&self, ev: &Event);
    /// Forgets everything observed.
    fn reset(&self);
}

/// A wall clock for `max_ahead` (Go `OpenOptions.Now`).
pub type WallFn = Arc<dyn Fn() -> SystemTime + Send + Sync>;

/// Options for [`Store::open_with`]. The default is an unsigned, unverified
/// private log (tests and tools that read a log).
#[derive(Clone, Default)]
pub struct OpenOptions {
    /// Signs every appended row and back-signs a v0.3 log in the one-time
    /// migration. With a Signer, an unsigned own row at open is an error.
    pub signer: Option<Arc<dyn Signer>>,
    /// Checks replicated rows; with one, insert also holds rows stamped
    /// more than `max_ahead` past wall time and accepts own rows back.
    pub verifier: Option<Arc<dyn Verifier>>,
    /// Zero means [`DEFAULT_MAX_AHEAD`].
    pub max_ahead: Duration,
    /// `None` means the system clock.
    pub now: Option<WallFn>,
    /// Open a shared board's log (`boards/<boardId>.db`).
    pub board: bool,
}

impl std::fmt::Debug for OpenOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenOptions")
            .field("signer", &self.signer.is_some())
            .field("verifier", &self.verifier.is_some())
            .field("max_ahead", &self.max_ahead)
            .field("now", &self.now.is_some())
            .field("board", &self.board)
            .finish()
    }
}

/// A revoke's claim: `(device_id, seq)` has [`row_hash`] `hash`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CutClaim {
    pub device_id: String,
    pub seq: i64,
    pub hash: String,
}

/// One row that conflicts with a stored row at the same `(device, seq)`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct Fork {
    /// The row held in `events` there (a zero Event if none).
    pub stored: Event,
    /// The other row; for a cut-hash fork only `device_id` and `seq`.
    pub other: Event,
    #[serde(rename = "otherHash")]
    pub other_hash: String,
    /// [`FORK_TWO_ROWS`] or [`FORK_CUT_HASH`].
    pub reason: String,
}

/// The highest HLC insert accepts: wall 2^46 ms, about the year 4199.
pub const MAX_REPLICATED_HLC: Timestamp = Timestamp(1 << 62);
/// [`OpenOptions::max_ahead`]'s default.
pub const DEFAULT_MAX_AHEAD: Duration = Duration::from_secs(120);
pub const DEFAULT_LIST_LIMIT: i64 = 200;
pub const MAX_LIST_LIMIT: i64 = 1000;
pub const DEFAULT_MISSING_LIMIT: i64 = 1000;
pub const MAX_MISSING_LIMIT: i64 = 10000;
/// Each subscriber's capacity; the last slot is kept for the resync signal.
pub const SUB_BUFFER: usize = 256;
/// The copy of a v0.3 database taken before the v0.4 migration.
pub const BACKUP_SUFFIX: &str = ".v0.3";
/// A second signed row for a held `(device, seq)`.
pub const FORK_TWO_ROWS: &str = "two rows";
/// A revoke's cutHash names another row than the one held at its cut.
pub const FORK_CUT_HASH: &str = "cut hash";

/// What a row's signature covers (store.md §2.5):
///
/// `"irori/row/1" || lp(device_id) || u64be(seq) || u64be(hlc) || lp(actor)
/// || lp(kind) || sha256(payload)`, where `lp` is a u32be byte length then
/// the bytes, and `payload` is the exact stored bytes.
pub fn signing_bytes(ev: &Event) -> Vec<u8> {
    signing_bytes_of(
        &ev.device_id,
        ev.seq,
        ev.hlc.0,
        &ev.actor,
        &ev.kind,
        &ev.payload,
    )
}

/// [`signing_bytes`] over borrowed fields: the one implementation, which
/// the identity crate's row view uses too.
pub fn signing_bytes_of(
    device_id: &str,
    seq: i64,
    hlc: u64,
    actor: &str,
    kind: &str,
    payload: &[u8],
) -> Vec<u8> {
    let sum = Sha256::digest(payload);
    let mut out = Vec::with_capacity(64 + device_id.len() + actor.len() + kind.len());
    out.extend_from_slice(b"irori/row/1");
    let lp = |out: &mut Vec<u8>, s: &str| {
        out.extend_from_slice(&(s.len() as u32).to_be_bytes());
        out.extend_from_slice(s.as_bytes());
    };
    lp(&mut out, device_id);
    out.extend_from_slice(&(seq as u64).to_be_bytes());
    out.extend_from_slice(&hlc.to_be_bytes());
    lp(&mut out, actor);
    lp(&mut out, kind);
    out.extend_from_slice(&sum);
    out
}

/// Lowercase hex sha256 of a row's signed bytes and signature: what a
/// revoke's cutHash names and what the forks table keys on.
pub fn row_hash(ev: &Event) -> String {
    row_hash_of(&signing_bytes(ev), &ev.sig)
}

/// [`row_hash`] from a row's [`signing_bytes`] and its signature.
pub fn row_hash_of(signing_bytes: &[u8], sig: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(signing_bytes);
    h.update(sig);
    hex::encode(h.finalize())
}
