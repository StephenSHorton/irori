//! The identity side plugged into a real `store::Store` (store.md §1.2,
//! identity.md §3.5): the Core opens its logs with these.
//!
//! - [`Chain`] and [`BoardVerifier`] implement `store::Verifier`: each
//!   `store::Event` is read through a [`RowRef`], and a [`VerifyError`]
//!   becomes the `store::Error` of the same sentinel with Go's full text.
//! - [`Identity`] implements `store::Signer` (Go passes `ident` as
//!   `OpenOptions.Signer`).
//! - `store::Store` implements [`Appender`], so `EnsureSelfCert` appends
//!   through the store and the row is folded by the store's Verifier.

use store::{Error, ErrorKind, Event};

use crate::board::BoardVerifier;
use crate::chain::Chain;
use crate::person::Identity;
use crate::row::{Appender, CutClaim, Row, RowRef, VerifyError};

impl<'a> From<&'a Event> for RowRef<'a> {
    fn from(ev: &'a Event) -> Self {
        RowRef {
            device_id: &ev.device_id,
            seq: ev.seq,
            hlc: ev.hlc.0,
            actor: &ev.actor,
            kind: &ev.kind,
            payload: &ev.payload,
            sig: &ev.sig,
        }
    }
}

impl From<Event> for Row {
    fn from(ev: Event) -> Self {
        Row {
            device_id: ev.device_id,
            seq: ev.seq,
            hlc: ev.hlc.0,
            actor: ev.actor,
            kind: ev.kind,
            payload: ev.payload,
            sig: ev.sig,
        }
    }
}

impl From<&Row> for Event {
    fn from(r: &Row) -> Self {
        Event {
            device_id: r.device_id.clone(),
            seq: r.seq,
            hlc: store::Timestamp(r.hlc),
            actor: r.actor.clone(),
            kind: r.kind.clone(),
            payload: r.payload.clone(),
            sig: r.sig.clone(),
        }
    }
}

impl VerifyError {
    /// The store sentinel this error stands for.
    pub fn kind(&self) -> ErrorKind {
        match self {
            VerifyError::NoCert(_) => ErrorKind::NoCert,
            VerifyError::BadSig(_) => ErrorKind::BadSig,
            VerifyError::BoardActor(_) => ErrorKind::BoardActor,
            VerifyError::PastCut(_) => ErrorKind::PastCut,
        }
    }
}

impl From<VerifyError> for Error {
    fn from(e: VerifyError) -> Error {
        Error::new(e.kind(), e.to_string())
    }
}

/// `store::Verifier` for a fold that already implements the identity
/// [`crate::row::Verifier`] on [`RowRef`]s.
macro_rules! store_verifier {
    ($t:ty) => {
        impl store::Verifier for $t {
            fn verify_row(&self, ev: &Event) -> store::Result<()> {
                <$t>::verify_row(self, ev.into()).map_err(Error::from)
            }
            fn cut(&self, device_id: &str) -> Option<i64> {
                <$t>::cut(self, device_id)
            }
            fn cut_claims(&self) -> Vec<CutClaim> {
                <$t>::cut_claims(self)
            }
            fn observe(&self, ev: &Event) {
                <$t>::observe(self, ev.into())
            }
            fn reset(&self) {
                <$t>::reset(self)
            }
        }
    };
}

store_verifier!(Chain);
store_verifier!(BoardVerifier);

impl store::Signer for Identity {
    fn sign_row(&self, msg: &[u8]) -> store::Result<Vec<u8>> {
        Ok(Identity::sign_row(self, msg))
    }
}

/// `payload` is already Go `json.Marshal` output; as a raw value the
/// store's marshal keeps it byte for byte.
impl Appender for store::Store {
    fn append(&self, actor: &str, kind: &str, payload: &[u8]) -> Result<Row, String> {
        store::Store::append(
            self,
            actor,
            kind,
            &gocompat::json::RawJson(payload.to_vec()),
        )
        .map(Row::from)
        .map_err(|e| e.to_string())
    }
}
