//! The seam (mesh.md §0; I:internal/mesh/mesh.go:59-142) is
//! `irori-meshseam`'s `Transport` / `Room` / `Conn`: everything above it is
//! this crate, everything below is `irori-net` (iroh) or [`FakeNet`]
//! (crate::fake::FakeNet) in tests. Both implement the one trait set.
//!
//! What stays here is the Node's view of one connection, [`PeerConn`]:
//! - the seam pushes the streams a peer opens to a handler; the session
//!   pulls them with [`PeerConn::accept_stream`] from a queue that handler
//!   fills (streams the peer opens before the session asks wait there);
//! - the seam's `done()` future becomes a [`CancellationToken`];
//! - a seam stream's two halves become one byte stream ([`Stream`]).
//!
//! A [`Room`] is one person-key epoch. Under iroh it has no envelope bus:
//! `connect` dials with that epoch's open proof (mesh.md §5.2), and `send`
//! may deliver nothing. The Node therefore also calls `connect` for every
//! allowed device on its presence tick, so peers meet without envelopes.
//!
//! Streams are byte streams: the opener writes its request and the server
//! writes its reply, then finishes (`shutdown`). An EOF ends the reply; a
//! refusal is a finish with no bytes.

use std::sync::Arc;
use std::time::Duration;

pub use irori_meshseam::{
    AcceptFn, AllowFn, BoxFut, Conn, ConnError, Envelope, Refused, Room, RoomConfig, Transport,
    TransportError,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// A byte stream on its own channel (blob/ and board/ streams).
pub trait ByteStream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> ByteStream for T {}

/// An open stream, both halves in one.
pub type Stream = Box<dyn ByteStream>;

/// Go `ErrRefused`: Accept's answer to a device that has no valid,
/// unrevoked cert for this person, or proved a key its cert doesn't name.
pub const ERR_REFUSED: &str = "mesh: device is not one of this person's";

/// One authenticated connection as the Node uses it.
pub struct PeerConn {
    c: Arc<dyn Conn>,
    streams: tokio::sync::Mutex<mpsc::UnboundedReceiver<(String, Stream)>>,
    done: CancellationToken,
}

impl PeerConn {
    /// Wraps one seam connection: its pushed streams go to a queue, its
    /// `done()` cancels a token.
    pub fn new(c: Arc<dyn Conn>) -> Arc<PeerConn> {
        let (tx, rx) = mpsc::unbounded_channel();
        c.on_stream(Arc::new(move |st: irori_meshseam::Stream| {
            let label = st.label.clone();
            let both: Stream = Box::new(tokio::io::join(st.recv, st.send));
            let _ = tx.send((label, both));
        }));
        let done = CancellationToken::new();
        {
            let (d, fut) = (done.clone(), c.done());
            tokio::spawn(async move {
                fut.await;
                d.cancel();
            });
        }
        Arc::new(PeerConn {
            c,
            streams: tokio::sync::Mutex::new(rx),
            done,
        })
    }

    fn err_text(&self) -> String {
        self.c
            .err()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "mesh: connection down".into())
    }

    /// The seam connection under it.
    pub fn raw(&self) -> &dyn Conn {
        self.c.as_ref()
    }

    /// The peer's device id.
    pub fn id(&self) -> String {
        self.c.id().to_string()
    }

    /// The key its handshake proved.
    pub fn pub_key(&self) -> [u8; 32] {
        self.c.pub_key()
    }

    /// One whole message on the reliable, ordered sync channel.
    pub async fn send(&self, msg: Vec<u8>) -> Result<(), String> {
        self.c.send(&msg).await.map_err(|e| e.to_string())
    }

    /// The next whole message; an error once the connection is down.
    pub async fn recv(&self) -> Result<Vec<u8>, String> {
        self.c.recv().await.map_err(|e| e.to_string())
    }

    /// Opens a byte stream with a label (`blob/…`, `board/…`).
    pub async fn open_stream(&self, label: &str) -> Result<Stream, String> {
        let st = self.c.open_stream(label).await.map_err(|e| e.to_string())?;
        Ok(Box::new(tokio::io::join(st.recv, st.send)) as Stream)
    }

    /// The next stream the peer opened, with its label.
    pub async fn accept_stream(&self) -> Result<(String, Stream), String> {
        let mut rx = self.streams.lock().await;
        tokio::select! {
            s = rx.recv() => s.ok_or_else(|| self.err_text()),
            _ = self.done.cancelled() => Err(self.err_text()),
        }
    }

    /// Cancelled once the connection is down.
    pub fn done(&self) -> CancellationToken {
        // The seam's done() resolves on a task; a connection already known
        // to be down is down now.
        if self.c.err().is_some() {
            self.done.cancel();
        }
        self.done.clone()
    }

    /// Why it went down (`None` while up).
    pub fn err(&self) -> Option<String> {
        self.c.err().map(|e| e.to_string())
    }

    /// Closes it with a reason the peer sees.
    pub fn close(&self, reason: &str) {
        self.c.close(reason)
    }

    /// "direct" or "relay".
    pub fn path(&self) -> String {
        self.c.path()
    }

    /// The latest round trip.
    pub fn rtt(&self) -> Duration {
        self.c.rtt()
    }

    /// The pairing SAS inputs (local, remote).
    pub fn fingerprints(&self) -> (String, String) {
        self.c.fingerprints()
    }
}
