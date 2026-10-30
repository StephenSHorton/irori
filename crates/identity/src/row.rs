//! The row boundary between `identity` and `store` (store.md §1.2).
//!
//! The folds read rows through [`RowRef`], a borrowed view of a store
//! `Event` (so they also run over rows that are not in a store), and report
//! through [`VerifyError`], whose variants are the store errors callers
//! branch on (`ErrNoCert`, `ErrBadSig`, `ErrBoardActor`, `ErrPastCut`).
//! [`crate::wire`] implements `store::Verifier` for the folds by building a
//! `RowRef` from each `Event` and mapping [`VerifyError`] onto
//! `store::Error`.
//!
//! [`signing_bytes`] and [`row_hash`] call the store's one implementation
//! (I:internal/store/store.go:171-176, 297-311) on a [`RowRef`]. The
//! adapters that plug the folds and the device key into a real
//! `store::Store` are in [`crate::wire`].

/// A borrowed row, the fields of a store `Event` the folds read.
#[derive(Clone, Copy, Debug)]
pub struct RowRef<'a> {
    pub device_id: &'a str,
    pub seq: i64,
    /// The row's HLC (`hlc.Timestamp`, a u64).
    pub hlc: u64,
    pub actor: &'a str,
    pub kind: &'a str,
    /// The stored payload bytes, never re-encoded.
    pub payload: &'a [u8],
    /// The device signature; empty when unsigned.
    pub sig: &'a [u8],
}

/// An owned row, for tests and callers that build rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub device_id: String,
    pub seq: i64,
    pub hlc: u64,
    pub actor: String,
    pub kind: String,
    pub payload: Vec<u8>,
    pub sig: Vec<u8>,
}

impl Row {
    pub fn as_ref(&self) -> RowRef<'_> {
        RowRef {
            device_id: &self.device_id,
            seq: self.seq,
            hlc: self.hlc,
            actor: &self.actor,
            kind: &self.kind,
            payload: &self.payload,
            sig: &self.sig,
        }
    }

    /// Sign the row with a device key (what the store's Signer does).
    pub fn sign(&mut self, key: &ed25519_dalek::SigningKey) {
        use ed25519_dalek::Signer as _;
        self.sig = key.sign(&signing_bytes(self.as_ref())).to_bytes().to_vec();
    }
}

impl<'a> From<&'a Row> for RowRef<'a> {
    fn from(r: &'a Row) -> Self {
        r.as_ref()
    }
}

/// `store.SigningBytes`:
/// `"irori/row/1" || lp(device) || u64be(seq) || u64be(hlc) || lp(actor) || lp(kind) || sha256(payload)`.
/// The store's implementation, over a borrowed row.
pub fn signing_bytes(ev: RowRef<'_>) -> Vec<u8> {
    store::signing_bytes_of(ev.device_id, ev.seq, ev.hlc, ev.actor, ev.kind, ev.payload)
}

/// `store.RowHash`: hex sha256 of the signing bytes then the signature.
pub fn row_hash(ev: RowRef<'_>) -> String {
    store::row_hash_of(&signing_bytes(ev), ev.sig)
}

/// `store.CutClaim`: an honoured revoke names the row at its cut by hash; a
/// held row there with other bytes is a fork.
pub use store::CutClaim;

/// The store errors a Verifier returns, each with Go's context text.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    /// `store.ErrNoCert`: the writer holds no cert yet (the row can wait).
    #[error("store: row's device is not certified: {0}")]
    NoCert(String),
    /// `store.ErrBadSig`.
    #[error("store: row signature does not verify: {0}")]
    BadSig(String),
    /// `store.ErrBoardActor`: the row claims someone its device isn't.
    #[error("store: actor is not allowed in a board log: {0}")]
    BoardActor(String),
    /// `store.ErrPastCut`.
    #[error("store: row is past its device's revocation cut: {0}")]
    PastCut(String),
}

/// `store.Verifier`: the fold the store consults for every identity row.
/// The store calls `observe` for each identity row it holds (at open and
/// on every write), `verify_row` before storing any row, and `cut` /
/// `cut_claims` to move rows past a revocation and to spot forks.
pub trait Verifier: Send + Sync {
    fn observe(&self, ev: RowRef<'_>);
    fn verify_row(&self, ev: RowRef<'_>) -> Result<(), VerifyError>;
    fn cut(&self, device_id: &str) -> Option<i64>;
    fn cut_claims(&self) -> Vec<CutClaim>;
    fn reset(&self);
}

/// What `EnsureSelfCert` needs of the store: append one row as this
/// device and fold it into the store's Verifier (store `Append`). `payload`
/// is Go `json.Marshal` output (gocompat), so the store's own marshal of it
/// as a raw message leaves it unchanged.
pub trait Appender {
    fn append(&self, actor: &str, kind: &str, payload: &[u8]) -> Result<Row, String>;
}

/// A row's place in fold order: HLC, then writer id bytewise, then seq
/// (Go `rowPos.less`). The derived order is exactly that.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct RowPos {
    pub hlc: u64,
    pub writer: String,
    pub seq: i64,
}

impl RowPos {
    pub(crate) fn of(ev: RowRef<'_>) -> RowPos {
        RowPos {
            hlc: ev.hlc,
            writer: ev.device_id.to_string(),
            seq: ev.seq,
        }
    }
}
