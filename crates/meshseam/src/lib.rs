//! The mesh transport seam: Go's `mesh.Transport` / `Room` / `Conn`
//! (I:internal/mesh/mesh.go:99-138, mesh.md §0), as Rust traits.
//!
//! Above the seam is the sync protocol (the `mesh` crate: Node, session,
//! frames, files, boards, FakeNet). Below it is how bytes move: the iroh
//! transport in `irori-net` (mesh.md §5.2). Both depend on this crate, which
//! has no iroh dependency, so the FakeNet suite builds without iroh.
//!
//! Differences from Go, each forced by iroh (mesh.md §5.2-5.3):
//! - [`Secret`] carries its `epoch`: an iroh connection has no topic, so the
//!   first message on its `sync` stream names the epoch and proves the
//!   sender holds that epoch's key (the epoch-open proof).
//! - [`RoomConfig`] has no `Pub`/`Sign`: the transport is keyed by the device
//!   key itself (D2), so the TLS handshake proves the key.
//! - [`Transport::join`] is async: the endpoint binds lazily on the first join.
//! - [`Room::send`] has no signaling bus behind it on iroh: a `presence` send
//!   is a dial of every allowed peer in the address book, and an arriving
//!   connection is reported to `on_msg` as that peer's `presence`.
//! - [`Conn::fingerprints`] returns the session's EKM (`EXPORTER-irori-sas`)
//!   on both sides, in place of DTLS certificate fingerprints (D5).

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use hkdf::Hkdf;
use sha2::Sha256;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub mod reason;

/// A boxed `Send` future (the traits here are used as `dyn`).
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The mesh topic's app name: the HKDF salt and the topic prefix (Go `MeshApp`).
pub const MESH_APP: &str = "irori/mesh/1";

/// The largest sync message (Go `mesh.MaxMessage`, frames.go:33).
pub const MAX_MESSAGE: usize = 8 << 20;

/// The longest stream label (mesh.md §5.2).
pub const MAX_LABEL: usize = 255;

/// The label of the one reliable, ordered message stream.
pub const SYNC_LABEL: &str = "sync";

/// Signaling envelope types the mesh sends (node.go:25-34).
pub const MSG_PRESENCE: &str = "presence";
pub const MSG_LEAVE: &str = "leave";

/// One room: the key that gates it and its topic (Go `mesh.Secret`), plus
/// the person-key epoch it belongs to (0 for a room with no epoch, such as
/// the pairing room).
#[derive(Clone, PartialEq, Eq)]
pub struct Secret {
    pub key: [u8; 32],
    pub topic: String,
    pub epoch: i64,
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Secret")
            .field("key", &"(redacted)")
            .field("topic", &self.topic)
            .field("epoch", &self.epoch)
            .finish()
    }
}

/// Go `MeshInfo`: the HKDF info of an epoch's mesh topic, `personId || u64be(epoch)`.
pub fn mesh_info(person_id: &str, epoch: i64) -> Vec<u8> {
    let mut v = person_id.as_bytes().to_vec();
    v.extend_from_slice(&(epoch as u64).to_be_bytes());
    v
}

/// Go `MeshSecret` (mesh.go:32-56, mesh.md §2.4):
/// HKDF-SHA256(ikm = person secret, salt = "irori/mesh/1",
/// info = personId || u64be(epoch)) → 32-byte key || 16-byte topic suffix;
/// topic = "irori/mesh/1/" + hex(suffix).
pub fn mesh_secret(seed: &[u8], person_id: &str, epoch: i64) -> Secret {
    let mut out = zeroize::Zeroizing::new([0u8; 48]);
    Hkdf::<Sha256>::new(Some(MESH_APP.as_bytes()), seed)
        .expand(&mesh_info(person_id, epoch), &mut out[..])
        .expect("48 bytes always fits");
    let mut key = [0u8; 32];
    key.copy_from_slice(&out[..32]);
    Secret {
        key,
        topic: format!("{MESH_APP}/{}", hex::encode(&out[32..])),
        epoch,
    }
}

/// One signaling message from another member of a room (Go `mesh.Envelope`).
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope {
    pub from: String,
    pub typ: String,
    pub body: serde_json::Value,
}

/// Go `ErrRefused`: the admission rule said no.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("mesh: refused")]
pub struct Refused;

/// Admission once the handshake proved the peer's key: `(peer id, pub)`.
pub type AcceptFn = Arc<dyn Fn(&str, &[u8; 32]) -> Result<(), Refused> + Send + Sync>;
/// Asked before any connection is attempted or accepted with a peer.
pub type AllowFn = Arc<dyn Fn(&str) -> bool + Send + Sync>;
/// Whether this side makes the connection to a peer (Go `Offerer`).
pub type OffererFn = Arc<dyn Fn(&str) -> bool + Send + Sync>;
pub type MsgFn = Arc<dyn Fn(Envelope) + Send + Sync>;
pub type ConnectFn = Arc<dyn Fn() + Send + Sync>;
pub type PeerFn = Arc<dyn Fn(Arc<dyn Conn>) + Send + Sync>;
/// Handles one stream the peer opened.
pub type StreamHandler = Arc<dyn Fn(Stream) + Send + Sync>;

/// This device in one room (Go `mesh.RoomConfig`, without `Pub`/`Sign`).
#[derive(Clone)]
pub struct RoomConfig {
    /// This device's id.
    pub self_id: String,
    /// Decides a peer once the handshake verified the key it holds. An error
    /// refuses it before any sync frame (the mesh's `CheckDevice`).
    pub accept: AcceptFn,
    /// Asked before any connection is attempted with peer.
    pub allow: AllowFn,
    /// Whether this side dials peer. `None` is the mesh rule: the lower id.
    /// Both sides must agree.
    pub offerer: Option<OffererFn>,
    /// Every envelope for everyone or for self.
    pub on_msg: MsgFn,
    /// Fires when the room is (re)connected to the network: announce presence.
    pub on_connect: ConnectFn,
    /// Each connection once it is up.
    pub on_peer: PeerFn,
    /// Pairing seam (M7): opens the room to guests for this long.
    pub guest_ttl: Option<Duration>,
    /// Pairing seam (M7): a typed home-server address on the joining device.
    pub hub_addr: Option<String>,
}

impl RoomConfig {
    /// A config with the mesh defaults: admit and allow everyone, no handlers.
    pub fn new(self_id: impl Into<String>) -> RoomConfig {
        RoomConfig {
            self_id: self_id.into(),
            accept: Arc::new(|_, _| Ok(())),
            allow: Arc::new(|_| true),
            offerer: None,
            on_msg: Arc::new(|_| {}),
            on_connect: Arc::new(|| {}),
            on_peer: Arc::new(|_| {}),
            guest_ttl: None,
            hub_addr: None,
        }
    }

    /// Whether this side dials `peer`: `offerer`, else the lower id.
    pub fn offers_to(&self, peer: &str) -> bool {
        match &self.offerer {
            Some(f) => f(peer),
            None => self.self_id.as_str() < peer,
        }
    }
}

impl fmt::Debug for RoomConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RoomConfig")
            .field("self_id", &self.self_id)
            .field("guest_ttl", &self.guest_ttl)
            .field("hub_addr", &self.hub_addr)
            .finish_non_exhaustive()
    }
}

/// Why a connection ended (Go `Conn.Err()`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnError {
    /// This side closed it, with this reason.
    Local(String),
    /// The peer closed it, with this reason.
    Remote(String),
    /// The path failed (timeout, reset, network gone).
    Transport(String),
}

impl ConnError {
    /// The close reason, or the failure text.
    pub fn reason(&self) -> &str {
        match self {
            ConnError::Local(r) | ConnError::Remote(r) | ConnError::Transport(r) => r,
        }
    }

    /// Whether the transport would reconnect on its own (R:mesh.go:553-565):
    /// only after a failure, never after a local close, a peer's close (bye),
    /// a refusal or a replace.
    pub fn retry(&self) -> bool {
        matches!(self, ConnError::Transport(_))
    }

    /// The peer or this side refused the connection.
    pub fn refused(&self) -> bool {
        self.reason() == reason::REFUSED
    }

    /// A newer connection to the same peer replaced this one.
    pub fn replaced(&self) -> bool {
        self.reason() == reason::REPLACED
    }
}

impl fmt::Display for ConnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnError::Local(r) => write!(f, "closed: {r}"),
            ConnError::Remote(r) => write!(f, "closed by peer: {r}"),
            ConnError::Transport(r) => write!(f, "connection lost: {r}"),
        }
    }
}

impl std::error::Error for ConnError {}

/// A transport-level failure outside one connection.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct TransportError(pub String);

/// One byte stream on its own channel (Go `io.ReadWriteCloser` from
/// `OpenStream` / `OnStream`). The opener writes its request and shuts its
/// send half down; the server writes the reply and shuts down. A refusal is
/// a shutdown with no bytes (mesh.md §2.3, §7 trap 14).
pub struct Stream {
    pub label: String,
    pub send: Box<dyn AsyncWrite + Send + Unpin>,
    pub recv: Box<dyn AsyncRead + Send + Unpin>,
}

impl fmt::Debug for Stream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stream")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// One authenticated connection to another device (Go `mesh.Conn`).
pub trait Conn: Send + Sync {
    /// The peer's device id.
    fn id(&self) -> &str;
    /// The key its handshake proved.
    fn pub_key(&self) -> [u8; 32];
    /// The epoch of the room this connection belongs to.
    fn epoch(&self) -> i64;
    /// One whole message on the reliable, ordered `sync` stream.
    fn send<'a>(&'a self, msg: &'a [u8]) -> BoxFut<'a, Result<(), ConnError>>;
    fn recv(&self) -> BoxFut<'_, Result<Vec<u8>, ConnError>>;
    /// A byte stream on its own channel (`blob/…`, `board/…`).
    fn open_stream<'a>(&'a self, label: &'a str) -> BoxFut<'a, Result<Stream, ConnError>>;
    /// Handles the streams the peer opens. Streams that arrive before a
    /// handler is set wait for it.
    fn on_stream(&self, h: StreamHandler);
    /// Resolves once the connection is down; `err` then says why.
    fn done(&self) -> BoxFut<'static, ()>;
    fn err(&self) -> Option<ConnError>;
    fn close(&self, reason: &str);
    /// `"direct"` or `"relay"`.
    fn path(&self) -> String;
    /// The latest round trip.
    fn rtt(&self) -> Duration;
    /// The pairing SAS input: on iroh both values are the session's
    /// `EXPORTER-irori-sas` keying material, colon-hex.
    fn fingerprints(&self) -> (String, String);
}

/// This device's place in one room (Go `mesh.Room`).
pub trait Room: Send + Sync {
    /// One envelope to `to` ("" = everyone).
    fn send(&self, to: &str, typ: &str, body: serde_json::Value) -> Result<(), TransportError>;
    /// Make (or ask for) a connection to peer; idempotent. Which side dials
    /// is the transport's rule (the lower id).
    fn connect(&self, peer: &str);
    /// Leave the room and close its connections.
    fn close(&self);
}

/// Opens rooms (Go `mesh.Transport`). FakeNet and the iroh transport implement it.
pub trait Transport: Send + Sync {
    fn join(&self, s: Secret, cfg: RoomConfig)
    -> BoxFut<'_, Result<Arc<dyn Room>, TransportError>>;
}

/// A framing failure on a stream.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame of {0} bytes is over the {1} byte limit")]
    TooLong(usize, usize),
    #[error("empty frame")]
    Empty,
    #[error("stream ended inside a frame")]
    Truncated,
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Write `u32 BE len || bytes`.
pub async fn write_frame<W: AsyncWrite + Unpin + ?Sized>(
    w: &mut W,
    b: &[u8],
    max: usize,
) -> Result<(), FrameError> {
    if b.len() > max {
        return Err(FrameError::TooLong(b.len(), max));
    }
    let mut buf = Vec::with_capacity(4 + b.len());
    buf.extend_from_slice(&(b.len() as u32).to_be_bytes());
    buf.extend_from_slice(b);
    w.write_all(&buf).await?;
    Ok(())
}

/// Read one `u32 BE len || bytes` frame. `Ok(None)` is a clean end of stream
/// at a frame boundary; a length over `max` or an end inside a frame is an
/// error. `allow_empty` false refuses a zero length (blob and board chunks).
pub async fn read_frame<R: AsyncRead + Unpin + ?Sized>(
    r: &mut R,
    max: usize,
    allow_empty: bool,
) -> Result<Option<Vec<u8>>, FrameError> {
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        let n = r.read(&mut len[got..]).await?;
        if n == 0 {
            return if got == 0 {
                Ok(None)
            } else {
                Err(FrameError::Truncated)
            };
        }
        got += n;
    }
    let n = u32::from_be_bytes(len) as usize;
    if n > max {
        return Err(FrameError::TooLong(n, max));
    }
    if n == 0 && !allow_empty {
        return Err(FrameError::Empty);
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            FrameError::Truncated
        } else {
            FrameError::Io(e)
        }
    })?;
    Ok(Some(buf))
}

/// Colon-separated upper-case hex, the fingerprint spelling.
pub fn colon_hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 3);
    for (i, x) in b.iter().enumerate() {
        if i > 0 {
            s.push(':');
        }
        s.push_str(&format!("{x:02X}"));
    }
    s
}

// ---------------------------------------------------------------------------
// Boards stage 2 transport seam (docs/design/boards2-transport.md, card #43).
// Interface only: the iroh side returns `TransportError(UNSUPPORTED)` until
// the transport branch lands.

/// The board topic's app name: the HKDF salt and topic prefix of
/// `identity::derive_board_topic`, and the board connection's ALPN.
pub const BOARD_APP: &str = "irori/board/1";

/// What a not-yet-built stage 2 transport call returns.
pub const UNSUPPORTED: &str = "boards2: transport not built yet";

/// One board room: a board and one of its epochs. `secret` is the board
/// topic of that epoch (`identity::BoardTopic` as a [`Secret`]: key, topic
/// `irori/board/1/<hex>`, epoch = board epoch).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BoardRoom {
    pub board_id: String,
    pub secret: Secret,
}

/// Opens board rooms on `irori/board/1` (stage 2, architecture §3.3). A
/// board room is a [`Room`] whose connections carry only that board:
/// `RoomConfig::accept` is the board's member check (a counted, uncut cert
/// in that board's `BoardVerifier`), never the private chain, and the only
/// streams served on its connections are `board/<boardId>` pulls and the
/// `sync` stream (board `hello`, `board.secret`, `vector`, ping/pong).
pub trait BoardTransport: Send + Sync {
    /// Joins `room` with `cfg`. Rejoining the same board with a new epoch
    /// keeps the previous epoch's room until the caller closes it (§6.1).
    fn join_board(
        &self,
        room: BoardRoom,
        cfg: RoomConfig,
    ) -> BoxFut<'_, Result<Arc<dyn Room>, TransportError>>;

    /// Joins the gossip topic of `room` (one per board epoch; boards2-
    /// transport §5): announcements whose vector is ahead of `have()` call
    /// `kick`, and only member devices of that board are heard. `bootstrap`
    /// is the member devices' keys to dial into the topic. `Ok(None)`: this
    /// transport runs no gossip (the board connections' own vector frames
    /// and ticks still move rows). Dropping the handle leaves the topic.
    fn board_gossip(
        &self,
        room: BoardRoom,
        bootstrap: Vec<[u8; 32]>,
        have: VectorFn,
        kick: KickFn,
    ) -> BoxFut<'_, Result<Option<Box<dyn BoardGossip>>, TransportError>> {
        let _ = (room, bootstrap, have, kick);
        Box::pin(async { Ok(None) })
    }
}

/// A device's vector of a board log: the highest seq held per device.
pub type BoardVector = std::collections::BTreeMap<String, i64>;
/// This device's vector of the board log, read when an announcement comes.
pub type VectorFn = Arc<dyn Fn() -> BoardVector + Send + Sync>;
/// Called when an announcement is ahead of the local log: pull now.
pub type KickFn = Arc<dyn Fn() + Send + Sync>;

/// One board epoch's gossip topic ([`BoardTransport::board_gossip`]).
pub trait BoardGossip: Send + Sync {
    /// Tells the topic this device's vector of the board log.
    fn announce(&self, vector: BoardVector) -> BoxFut<'_, Result<(), TransportError>>;
    /// Adds member devices (by key) to dial into the topic.
    fn join(&self, devices: Vec<[u8; 32]>) -> BoxFut<'_, Result<(), TransportError>>;
}
