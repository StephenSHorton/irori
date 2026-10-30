//! Boards stage 2 above the seam: one board mesh per board, the board sync
//! frames, `board.secret` and `board.server` (docs/design/
//! boards2-transport.md, v0.5-shared-boards §6.1-6.4).
//!
//! A [`BoardMesh`] joins one board room per board epoch it holds a secret
//! for (the newest, and the one before until it settles) on a
//! [`BoardTransport`] (`irori/board/1` on iroh). Every connection's
//! admission is that board's [`BoardVerifier`] only: a counted, uncut cert
//! whose `devicePub` is the key the handshake proved. On each connection:
//!
//! - `sync` carries JSON frames tagged by `type`: `hello` ([`BoardHello`],
//!   first, each way; a `v` other than [`BOARD_PROTOCOL`] closes `version`),
//!   `vector` (the sender's board vector; one ahead of ours pulls now),
//!   `ping`/`pong`, and `board.secret` ([`BoardSecretFrame`], a newer
//!   epoch's secret sealed under this connection's epoch key).
//! - Rows move on `board/<boardId>` pull streams ([`crate::boards`]); any
//!   other stream label is refused.
//!
//! The private mesh (`irori/mesh/1`) never carries a board row.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use base64::Engine as _;
use crypto_secretbox::aead::{Aead, KeyInit};
use crypto_secretbox::{Nonce, XSalsa20Poly1305};
use hkdf::Hkdf;
use identity::{BoardSecrets, BoardVerifier};
use irori_meshseam::{
    BoardRoom, BoardTransport, Conn, MSG_PRESENCE, Refused, Room, RoomConfig, Secret,
    TransportError, reason,
};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use store::Store;
use tokio::io::AsyncWriteExt as _;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::boards::{BOARD_LABEL_PREFIX, pull_board, serve_pull};
use crate::node::Intervals;
use crate::servers::PublicAddr;
use crate::transport::Stream;

/// The board sync protocol version a board hello carries (§6.3 "Version").
pub const BOARD_PROTOCOL: i64 = 1;

/// The frame that hands a board secret to a device: on the private mesh
/// `sync` channel to your own devices (sealed under the mesh epoch's
/// person-secret key, as `person.secret`), and after a rotation on each
/// board connection to current member devices (sealed under that
/// connection's board epoch key).
pub const FRAME_BOARD_SECRET: &str = "board.secret";

/// How long a connection the verifier no longer admits stays open after
/// our last vector: long enough for the peer to pull the row that shut it
/// out (its removal, or the answer to its leave).
const CLOSE_GRACE: std::time::Duration = std::time::Duration::from_millis(1500);

/// The kind of a board server row.
pub const KIND_BOARD_SERVER: &str = "board.server";

/// The board connection's hello (§6.4): first frame on `sync` after the
/// board-open message. A `v` or `coreVersion` the receiver refuses closes
/// the connection with reason `"version"`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardHello {
    pub v: i64,
    pub device_id: String,
    pub person_id: String,
    pub board_id: String,
    pub board_epoch: i64,
    pub core_version: String,
    pub schema: i64,
    pub vector: BTreeMap<String, i64>,
    pub wall_ms: i64,
}

/// `board.secret {boardId, boardEpoch, maker, sealed}`: `sealed` is
/// base64 `nonce(24) || secretbox(secret)`; `maker` is the device that
/// minted it (the lower-maker rule, §6.1).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardSecretFrame {
    pub board_id: String,
    pub board_epoch: i64,
    pub maker: String,
    pub sealed: String,
}

/// The HKDF salt of a `board.secret` sealing key.
const BOARD_SECRET_APP: &str = "irori/board-secret/1";
/// NaCl secretbox's tag length.
const OVERHEAD: usize = 16;
/// What [`open_board_secret`] says when the box does not open.
pub const ERR_BAD_BOARD_SECRET: &str = "mesh: board.secret does not open";

fn board_secret_key(
    seal_key: &[u8; 32],
    board_id: &str,
    epoch: i64,
    recipient: &str,
) -> Zeroizing<[u8; 32]> {
    let mut info = board_id.as_bytes().to_vec();
    info.extend_from_slice(&(epoch as u64).to_be_bytes());
    info.extend_from_slice(recipient.as_bytes());
    let mut k = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(Some(BOARD_SECRET_APP.as_bytes()), seal_key)
        .expand(&info, &mut k[..])
        .expect("32 bytes always fits");
    k
}

/// Seals board `board_id`'s secret for `epoch` to `recipient` under
/// `seal_key` (the mesh key on the private channel, the board topic key on
/// a board connection): `nonce(24) || secretbox(secret)` with the box key
/// HKDF-SHA256(seal_key, salt `irori/board-secret/1`, info boardId ||
/// u64be(epoch) || recipient).
pub fn seal_board_secret(
    seal_key: &[u8; 32],
    board_id: &str,
    epoch: i64,
    secret: &[u8],
    recipient: &str,
) -> Result<Vec<u8>, String> {
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut nonce).map_err(|e| format!("random: {e}"))?;
    let k = board_secret_key(seal_key, board_id, epoch, recipient);
    let boxed = XSalsa20Poly1305::new((&*k).into())
        .encrypt(&Nonce::from(nonce), secret)
        .map_err(|_| "mesh: seal board secret".to_string())?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&boxed);
    Ok(out)
}

/// Opens a [`seal_board_secret`] result on the recipient `self_id`.
pub fn open_board_secret(
    seal_key: &[u8; 32],
    board_id: &str,
    epoch: i64,
    sealed: &[u8],
    self_id: &str,
) -> Result<Vec<u8>, String> {
    if sealed.len() < 24 + OVERHEAD {
        return Err(ERR_BAD_BOARD_SECRET.into());
    }
    let k = board_secret_key(seal_key, board_id, epoch, self_id);
    let mut nonce = [0u8; 24];
    nonce.copy_from_slice(&sealed[..24]);
    XSalsa20Poly1305::new((&*k).into())
        .decrypt(&Nonce::from(nonce), &sealed[24..])
        .map_err(|_| ERR_BAD_BOARD_SECRET.into())
}

/// A `board.server` payload (B1, §6.2): written by a server device into
/// each board its person is in, only when it changes. `public: None` (JSON
/// `null`) when the server is off or outside reach is off. Never LAN or v6
/// addresses.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BoardServer {
    pub device_id: String,
    pub port: i64,
    pub public: Option<PublicAddr>,
}

/// The board's `board.server` fold: the newest row per server device whose
/// writer is that device and a counted, uncut member device, by device id.
/// A server whose newest row says `public: null` stays in the list (it is
/// off or unreachable from outside: nothing to dial).
pub fn board_servers(
    log: &Store,
    verifier: &BoardVerifier,
) -> Result<Vec<BoardServer>, TransportError> {
    const PAGE: i64 = 1000;
    let mut newest: BTreeMap<String, BoardServer> = BTreeMap::new();
    let mut after = store::Cursor::default();
    loop {
        let rows = log
            .list(&store::Filter {
                kinds: vec![KIND_BOARD_SERVER.into()],
                after: after.clone(),
                limit: PAGE,
                ..Default::default()
            })
            .map_err(|e| TransportError(format!("board.server: {e}")))?;
        for ev in &rows {
            let Ok(s) = serde_json::from_slice::<BoardServer>(&ev.payload) else {
                continue;
            };
            if s.device_id != ev.device_id {
                continue; // only the server's own device announces it
            }
            newest.insert(s.device_id.clone(), s);
        }
        match rows.last() {
            Some(last) if rows.len() as i64 == PAGE => after = last.cursor(),
            _ => break,
        }
    }
    Ok(newest
        .into_values()
        .filter(|s| verifier.cert(&s.device_id).is_some() && verifier.cut(&s.device_id).is_none())
        .collect())
}

/// Writes `row` into board log `log` when it differs from this device's
/// newest `board.server` there (rows only on change, as `node.addr`).
/// Returns whether a row was written.
pub fn write_board_server(log: &Store, row: &BoardServer) -> Result<bool, String> {
    let last = log
        .list(&store::Filter {
            kinds: vec![KIND_BOARD_SERVER.into()],
            device_id: row.device_id.clone(),
            limit: 1,
            desc: true,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
    if let Some(ev) = last.first()
        && serde_json::from_slice::<BoardServer>(&ev.payload)
            .ok()
            .as_ref()
            == Some(row)
    {
        return Ok(false);
    }
    log.append("irori", KIND_BOARD_SERVER, row)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// What a [`BoardMesh`] runs on.
#[derive(Clone)]
pub struct BoardMeshOpts {
    pub board_id: String,
    pub self_id: String,
    pub person_id: String,
    pub core_version: String,
    pub log: Arc<Store>,
    pub verifier: Arc<BoardVerifier>,
    pub secrets: Arc<BoardSecrets>,
    pub transport: Arc<dyn BoardTransport>,
    /// This person's other devices from its private chain (certified, not
    /// revoked). They are admitted and dialled before the board log holds
    /// their certs, so a device that opened the board from board.joined
    /// can pull it (and those certs) from its own devices. `None`: the
    /// board's verifier only.
    pub own_devices: Option<OwnDevicesFn>,
}

/// This person's other devices (see [`BoardMeshOpts::own_devices`]).
pub type OwnDevicesFn = Arc<dyn Fn() -> Vec<MemberDevice> + Send + Sync>;

/// One counted member device of a board (a dial target).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberDevice {
    pub person_id: String,
    pub device_id: String,
    /// Its certified key, hex (the EndpointId on iroh).
    pub device_pub: String,
}

/// A board connection's `sync` frames.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum Wire {
    #[serde(rename = "hello")]
    Hello(BoardHello),
    #[serde(rename = "vector")]
    Vector { vector: BTreeMap<String, i64> },
    #[serde(rename = "ping")]
    Ping {},
    #[serde(rename = "pong")]
    Pong {},
    #[serde(rename = "board.secret")]
    Secret(BoardSecretFrame),
}

type LogFn = Arc<dyn Fn(&str) + Send + Sync>;
type RowsFn = Arc<dyn Fn() + Send + Sync>;

/// What a member device last said on a board connection: its hello's
/// person and core version, and its newest board vector (from the hello or
/// a `vector` frame). Kept after the connection closes, so a leaving
/// device can tell another member holds its `left` row (v0.5 §4.4) and the
/// Boards dialog can compare versions (§6.3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerHeard {
    pub device_id: String,
    pub person_id: String,
    pub core_version: String,
    pub vector: BTreeMap<String, i64>,
    /// Unix milliseconds of the last frame.
    pub at_ms: i64,
    /// Per stream in `vector`: when its seq last moved (Unix ms), for the
    /// stalled-forwarder flag (§6.4).
    pub advanced_ms: BTreeMap<String, i64>,
}

/// One board connection.
struct Peer {
    conn: Arc<dyn Conn>,
    epoch: i64,
    hello: AtomicBool,
    wake: Notify,
}

struct JoinedRoom {
    room: Arc<dyn Room>,
    key: [u8; 32],
}

struct Shared {
    opts: BoardMeshOpts,
    iv: Mutex<Intervals>,
    rooms: tokio::sync::Mutex<BTreeMap<i64, JoinedRoom>>,
    /// Room keys by epoch, readable without the async lock.
    keys: Mutex<BTreeMap<i64, [u8; 32]>>,
    rooms_list: Mutex<Vec<Arc<dyn Room>>>,
    peers: Mutex<HashMap<(i64, String), Arc<Peer>>>,
    cancel: CancellationToken,
    started: AtomicBool,
    log: Mutex<Option<LogFn>>,
    on_rows: Mutex<Option<RowsFn>>,
    heard: Mutex<BTreeMap<String, PeerHeard>>,
}

/// One board's mesh: a board room per held epoch (current and previous
/// while members move), `board/<id>` pulls on each connection, the board
/// hello, `board.secret` after rotations. Admission is the board's
/// verifier only.
pub struct BoardMesh {
    s: Arc<Shared>,
}

fn admitted(v: &BoardVerifier, peer: &str, pubkey: &[u8; 32]) -> bool {
    v.admitted_by_pub(&hex::encode(pubkey))
        .is_some_and(|c| c.device_id == peer)
}

/// [`admitted`], or one of this person's own devices.
fn admits(v: &BoardVerifier, own: &Option<OwnDevicesFn>, peer: &str, pubkey: &[u8; 32]) -> bool {
    admitted(v, peer, pubkey)
        || own.as_ref().is_some_and(|f| {
            let k = hex::encode(pubkey);
            f().iter().any(|d| d.device_id == peer && d.device_pub == k)
        })
}

impl BoardMesh {
    pub fn new(opts: BoardMeshOpts) -> BoardMesh {
        BoardMesh {
            s: Arc::new(Shared {
                opts,
                iv: Mutex::new(Intervals::default().with_defaults()),
                rooms: tokio::sync::Mutex::new(BTreeMap::new()),
                keys: Mutex::new(BTreeMap::new()),
                rooms_list: Mutex::new(Vec::new()),
                peers: Mutex::new(HashMap::new()),
                cancel: CancellationToken::new(),
                started: AtomicBool::new(false),
                log: Mutex::new(None),
                on_rows: Mutex::new(None),
                heard: Mutex::new(BTreeMap::new()),
            }),
        }
    }

    /// Shorter presence, ping and vector ticks (tests); zero fields keep
    /// the mesh defaults. Set before [`start`](Self::start).
    pub fn set_intervals(&self, iv: Intervals) {
        *self.s.iv.lock().expect("iv") = iv.with_defaults();
    }

    /// Where the mesh logs (pull failures, refusals).
    pub fn set_log(&self, f: LogFn) {
        *self.s.log.lock().expect("log") = Some(f);
    }

    /// Called after rows from another member landed in the board log (the
    /// Core refeeds the board's peers and its views).
    pub fn on_rows(&self, f: RowsFn) {
        *self.s.on_rows.lock().expect("on_rows") = Some(f);
    }

    pub fn board_id(&self) -> &str {
        &self.s.opts.board_id
    }

    /// The board's log.
    pub fn log(&self) -> &Arc<Store> {
        &self.s.opts.log
    }

    /// The board's verifier.
    pub fn verifier(&self) -> &Arc<BoardVerifier> {
        &self.s.opts.verifier
    }

    /// The board's counted member devices other than this one: every
    /// active member's certified devices the verifier admits.
    pub fn member_devices(&self) -> Vec<MemberDevice> {
        self.s.member_devices()
    }

    /// The epochs whose rooms are joined.
    pub fn epochs(&self) -> Vec<i64> {
        self.s.keys.lock().expect("keys").keys().copied().collect()
    }

    /// The devices with a live connection that said hello, by epoch.
    pub fn peers(&self) -> Vec<(i64, String)> {
        let mut v: Vec<(i64, String)> = self
            .s
            .peers
            .lock()
            .expect("peers")
            .iter()
            .filter(|(_, p)| p.hello.load(Ordering::SeqCst))
            .map(|(k, _)| k.clone())
            .collect();
        v.sort();
        v
    }

    /// Joins the board's rooms for every epoch `secrets` holds, and keeps
    /// dialling its member devices on the presence tick until
    /// [`close`](Self::close).
    pub async fn start(&self) -> Result<(), TransportError> {
        self.s.refresh().await?;
        if self.s.started.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let w = Arc::downgrade(&self.s);
        let cancel = self.s.cancel.clone();
        let every = self.s.iv.lock().expect("iv").presence;
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(every) => {}
                }
                let Some(s) = w.upgrade() else { return };
                if let Err(e) = s.refresh().await {
                    s.logf(&format!("refresh: {e}"));
                }
            }
        });
        Ok(())
    }

    /// Pulls from every connected member device now and tells them this
    /// device's vector (a gossip announcement or a local write).
    pub fn kick(&self) {
        for p in self.s.peer_list() {
            p.wake.notify_one();
        }
    }

    /// What each member device last said here (closed connections too).
    pub fn heard(&self) -> Vec<PeerHeard> {
        self.s
            .heard
            .lock()
            .expect("heard")
            .values()
            .cloned()
            .collect()
    }

    /// Re-checks live connections against the verifier (a revoke, removal
    /// or leave folded) and closes those no longer admitted with `revoked`;
    /// then joins the rooms of newly held epochs, hands newer secrets to the
    /// member devices still on an older epoch, and dials new members.
    pub fn recheck(&self) {
        self.s.recheck_conns();
        let w = Arc::downgrade(&self.s);
        tokio::spawn(async move {
            if let Some(s) = w.upgrade()
                && let Err(e) = s.refresh().await
            {
                s.logf(&format!("refresh: {e}"));
            }
        });
    }

    /// Leaves every room of the board.
    pub fn close(&self) {
        self.s.cancel.cancel();
        let rooms = std::mem::take(&mut *self.s.rooms_list.lock().expect("rooms"));
        for r in rooms {
            r.close();
        }
        self.s.keys.lock().expect("keys").clear();
        let peers: Vec<_> = self.s.peers.lock().expect("peers").drain().collect();
        for (_, p) in peers {
            p.conn.close(reason::LEAVING);
        }
    }
}

impl Drop for BoardMesh {
    fn drop(&mut self) {
        self.s.cancel.cancel();
    }
}

impl Shared {
    fn logf(&self, line: &str) {
        let f = self.log.lock().expect("log").clone();
        if let Some(f) = f {
            f(&format!("board {}: {line}", self.opts.board_id));
        }
    }

    fn peer_list(&self) -> Vec<Arc<Peer>> {
        self.peers
            .lock()
            .expect("peers")
            .values()
            .cloned()
            .collect()
    }

    fn member_devices(&self) -> Vec<MemberDevice> {
        let v = &self.opts.verifier;
        let ms = v.membership();
        let mut out = Vec::new();
        for p in &ms.members {
            if !ms.active(&p.person_id) {
                continue;
            }
            for d in v.devices(&p.person_id) {
                if d.revoked
                    || d.cert.device_id == self.opts.self_id
                    || v.admitted_by_pub(&d.cert.device_pub).is_none()
                {
                    continue;
                }
                out.push(MemberDevice {
                    person_id: p.person_id.clone(),
                    device_id: d.cert.device_id.clone(),
                    device_pub: d.cert.device_pub.clone(),
                });
            }
        }
        if let Some(own) = &self.opts.own_devices {
            for d in own() {
                if d.device_id != self.opts.self_id
                    && !out.iter().any(|o| o.device_id == d.device_id)
                {
                    out.push(d);
                }
            }
        }
        out
    }

    fn recheck_conns(self: &Arc<Self>) {
        for p in self.peer_list() {
            if !admits(
                &self.opts.verifier,
                &self.opts.own_devices,
                p.conn.id(),
                &p.conn.pub_key(),
            ) {
                self.logf(&format!("{} is no longer a member device", p.conn.id()));
                // Our vector first, so a device that just left learns its
                // `left` row arrived (v0.5 §4.4), and a removed one pulls
                // the removal (§4.3); then the close.
                // A device revoked while its person is still a member
                // (lost or stolen) closes now; only a person removed or
                // gone gets the grace.
                let v = &self.opts.verifier;
                let person_gone = v
                    .person_of(p.conn.id())
                    .is_some_and(|pid| !v.membership().active(&pid));
                if !person_gone {
                    p.conn.close(reason::REVOKED);
                    continue;
                }
                let me = self.clone();
                match tokio::runtime::Handle::try_current() {
                    Ok(h) => {
                        h.spawn(async move {
                            let vector = me.opts.log.vector().unwrap_or_default();
                            let _ = tokio::time::timeout(
                                std::time::Duration::from_secs(2),
                                me.send(&p, &Wire::Vector { vector }),
                            )
                            .await;
                            tokio::time::sleep(CLOSE_GRACE).await;
                            p.conn.close(reason::REVOKED);
                        });
                    }
                    Err(_) => p.conn.close(reason::REVOKED),
                }
            }
        }
    }

    /// Joins every held epoch's room not joined yet, leaves the ones no
    /// longer held, hands newer secrets on, and dials the member devices.
    async fn refresh(self: &Arc<Self>) -> Result<(), TransportError> {
        if self.cancel.is_cancelled() {
            return Ok(());
        }
        let topics = self
            .opts
            .secrets
            .topics()
            .map_err(|e| TransportError(e.to_string()))?;
        let mut rooms = self.rooms.lock().await;
        for t in &topics {
            if rooms.get(&t.epoch).is_some_and(|r| r.key == t.key) {
                continue;
            }
            let room = self
                .join(Secret {
                    key: t.key,
                    topic: t.topic.clone(),
                    epoch: t.epoch,
                })
                .await?;
            if let Some(old) = rooms.insert(
                t.epoch,
                JoinedRoom {
                    room: room.clone(),
                    key: t.key,
                },
            ) {
                old.room.close();
            }
        }
        let held: Vec<i64> = topics.iter().map(|t| t.epoch).collect();
        let gone: Vec<i64> = rooms
            .keys()
            .copied()
            .filter(|e| !held.contains(e))
            .collect();
        for e in gone {
            if let Some(r) = rooms.remove(&e) {
                r.room.close();
            }
        }
        *self.keys.lock().expect("keys") = rooms.iter().map(|(e, r)| (*e, r.key)).collect();
        *self.rooms_list.lock().expect("rooms") = rooms.values().map(|r| r.room.clone()).collect();
        let list: Vec<Arc<dyn Room>> = rooms.values().map(|r| r.room.clone()).collect();
        drop(rooms);
        if self.cancel.is_cancelled() {
            for r in list {
                r.close();
            }
            return Ok(());
        }
        for p in self.peer_list() {
            self.share_secrets(&p).await;
        }
        self.dial(&list);
        Ok(())
    }

    fn dial(&self, rooms: &[Arc<dyn Room>]) {
        let devs = self.member_devices();
        for r in rooms {
            let _ = r.send("", MSG_PRESENCE, serde_json::Value::Null);
            for d in &devs {
                r.connect(&d.device_id);
            }
        }
    }

    async fn join(self: &Arc<Self>, secret: Secret) -> Result<Arc<dyn Room>, TransportError> {
        let me = self.opts.self_id.clone();
        let mut cfg = RoomConfig::new(me.clone());
        {
            let (v, own, me) = (
                self.opts.verifier.clone(),
                self.opts.own_devices.clone(),
                me.clone(),
            );
            cfg.accept = Arc::new(move |peer: &str, pubkey: &[u8; 32]| {
                if peer != me && admits(&v, &own, peer, pubkey) {
                    Ok(())
                } else {
                    Err(Refused)
                }
            });
        }
        cfg.allow = Arc::new(move |peer: &str| peer != me);
        let epoch = secret.epoch;
        let w: Weak<Shared> = Arc::downgrade(self);
        cfg.on_peer = Arc::new(move |c: Arc<dyn Conn>| {
            if let Some(s) = w.upgrade() {
                s.on_peer(epoch, c);
            }
        });
        self.opts
            .transport
            .join_board(
                BoardRoom {
                    board_id: self.opts.board_id.clone(),
                    secret,
                },
                cfg,
            )
            .await
    }

    fn on_peer(self: &Arc<Self>, epoch: i64, conn: Arc<dyn Conn>) {
        let peer = Arc::new(Peer {
            conn: conn.clone(),
            epoch,
            hello: AtomicBool::new(false),
            wake: Notify::new(),
        });
        if let Some(old) = self
            .peers
            .lock()
            .expect("peers")
            .insert((epoch, conn.id().to_string()), peer.clone())
            && !Arc::ptr_eq(&old.conn, &conn)
        {
            old.conn.close(reason::REPLACED);
        }
        {
            let (log, board) = (self.opts.log.clone(), self.opts.board_id.clone());
            conn.on_stream(Arc::new(move |st: irori_meshseam::Stream| {
                let (log, board) = (log.clone(), board.clone());
                tokio::spawn(async move {
                    let want = format!("{BOARD_LABEL_PREFIX}{board}");
                    let ok = st.label == want;
                    let mut both: Stream = Box::new(tokio::io::join(st.recv, st.send));
                    if ok {
                        serve_pull(log, &mut both).await;
                    }
                    let _ = both.shutdown().await;
                });
            }));
        }
        let s = self.clone();
        tokio::spawn(async move { s.run_peer(peer).await });
    }

    fn hello(&self) -> BoardHello {
        BoardHello {
            v: BOARD_PROTOCOL,
            device_id: self.opts.self_id.clone(),
            person_id: self.opts.person_id.clone(),
            board_id: self.opts.board_id.clone(),
            board_epoch: self.opts.secrets.epoch(),
            core_version: self.opts.core_version.clone(),
            schema: 1,
            vector: self.opts.log.vector().unwrap_or_default(),
            wall_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        }
    }

    async fn send(&self, p: &Peer, w: &Wire) -> Result<(), String> {
        let b = serde_json::to_vec(w).map_err(|e| e.to_string())?;
        p.conn.send(&b).await.map_err(|e| e.to_string())
    }

    async fn run_peer(self: Arc<Self>, p: Arc<Peer>) {
        let done = p.conn.done();
        let ctx = self.cancel.child_token();
        if self.send(&p, &Wire::Hello(self.hello())).await.is_err() {
            p.conn.close(reason::PROTOCOL);
        }
        let read = {
            let (s, p, ctx) = (self.clone(), p.clone(), ctx.clone());
            tokio::spawn(async move { s.read(p, ctx).await })
        };
        let pull = {
            let (s, p, ctx) = (self.clone(), p.clone(), ctx.clone());
            tokio::spawn(async move { s.pull_loop(p, ctx).await })
        };
        tokio::select! {
            _ = done => {}
            _ = ctx.cancelled() => {}
        }
        ctx.cancel();
        let _ = read.await;
        let _ = pull.await;
        let mut peers = self.peers.lock().expect("peers");
        let key = (p.epoch, p.conn.id().to_string());
        if peers.get(&key).is_some_and(|x| Arc::ptr_eq(x, &p)) {
            peers.remove(&key);
        }
    }

    async fn read(self: Arc<Self>, p: Arc<Peer>, ctx: CancellationToken) {
        loop {
            let m = tokio::select! {
                _ = ctx.cancelled() => return,
                m = p.conn.recv() => m,
            };
            let Ok(m) = m else {
                ctx.cancel();
                return;
            };
            // A frame type this version does not know is skipped.
            let Ok(w) = serde_json::from_slice::<Wire>(&m) else {
                continue;
            };
            let said_hello = p.hello.load(Ordering::SeqCst);
            match w {
                Wire::Hello(h) => {
                    if h.v != BOARD_PROTOCOL {
                        p.conn.close(reason::VERSION);
                        return;
                    }
                    if h.board_id != self.opts.board_id || h.device_id != p.conn.id() {
                        p.conn.close(reason::HELLO);
                        return;
                    }
                    p.hello.store(true, Ordering::SeqCst);
                    self.hear(
                        p.conn.id(),
                        Some((&h.person_id, &h.core_version)),
                        &h.vector,
                    );
                    if self.behind(&h.vector) {
                        p.wake.notify_one();
                    }
                    self.share_secrets(&p).await;
                }
                _ if !said_hello => {
                    p.conn.close(reason::FRAME_BEFORE_HELLO);
                    return;
                }
                Wire::Vector { vector } => {
                    self.hear(p.conn.id(), None, &vector);
                    if self.behind(&vector) {
                        p.wake.notify_one();
                    }
                }
                Wire::Ping {} => {
                    let _ = self.send(&p, &Wire::Pong {}).await;
                }
                Wire::Pong {} => {}
                Wire::Secret(f) => self.take_secret(&p, f).await,
            }
        }
    }

    /// Records what member device `dev` said (`hello`: its person and core
    /// version).
    fn hear(&self, dev: &str, hello: Option<(&str, &str)>, vector: &BTreeMap<String, i64>) {
        let mut heard = self.heard.lock().expect("heard");
        let h = heard.entry(dev.to_string()).or_default();
        h.device_id = dev.to_string();
        if let Some((person, version)) = hello {
            h.person_id = person.to_string();
            h.core_version = version.to_string();
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        for (d, s) in vector {
            let e = h.vector.entry(d.clone()).or_insert(0);
            if *s > *e || !h.advanced_ms.contains_key(d) {
                h.advanced_ms.insert(d.clone(), now);
            }
            *e = (*e).max(*s);
        }
        h.at_ms = now;
    }

    /// Whether `theirs` holds a row this device's board log lacks.
    fn behind(&self, theirs: &BTreeMap<String, i64>) -> bool {
        let ours = self.opts.log.vector().unwrap_or_default();
        theirs
            .iter()
            .any(|(d, s)| ours.get(d).copied().unwrap_or(0) < *s)
    }

    async fn pull_loop(self: Arc<Self>, p: Arc<Peer>, ctx: CancellationToken) {
        while !p.hello.load(Ordering::SeqCst) {
            tokio::select! {
                _ = ctx.cancelled() => return,
                _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => {}
            }
        }
        let (every, ping) = {
            let iv = self.iv.lock().expect("iv");
            (iv.vector, iv.ping)
        };
        let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
        let mut pings = tokio::time::interval_at(tokio::time::Instant::now() + ping, ping);
        loop {
            let vector = self.opts.log.vector().unwrap_or_default();
            if self.send(&p, &Wire::Vector { vector }).await.is_err() {
                return;
            }
            let r = tokio::select! {
                _ = ctx.cancelled() => return,
                r = pull_board(p.conn.as_ref(), &self.opts.board_id, self.opts.log.clone()) => r,
            };
            match r {
                Ok(0) => {}
                Ok(_) => {
                    // Rows landed: membership may have moved (a removal, a
                    // new member, a revoked device).
                    self.recheck_conns();
                    let f = self.on_rows.lock().expect("on_rows").clone();
                    if let Some(f) = f {
                        f();
                    }
                }
                Err(e) => self.logf(&format!("pull from {}: {e}", p.conn.id())),
            }
            loop {
                tokio::select! {
                    _ = ctx.cancelled() => return,
                    _ = tick.tick() => break,
                    _ = p.wake.notified() => break,
                    _ = pings.tick() => {
                        let _ = self.send(&p, &Wire::Ping {}).await;
                    }
                }
            }
        }
    }

    /// Hands every epoch newer than `p`'s connection to it, sealed under the
    /// connection's epoch key, while the peer is still admitted.
    async fn share_secrets(&self, p: &Peer) {
        if !p.hello.load(Ordering::SeqCst)
            || !admitted(&self.opts.verifier, p.conn.id(), &p.conn.pub_key())
        {
            return;
        }
        let Some(key) = self.keys.lock().expect("keys").get(&p.epoch).copied() else {
            return;
        };
        let newest = self.opts.secrets.epoch();
        if newest <= p.epoch {
            return;
        }
        let Some((secret, maker)) = self.opts.secrets.secret(newest) else {
            return;
        };
        let board = &self.opts.board_id;
        let Ok(sealed) = seal_board_secret(&key, board, newest, &secret, p.conn.id()) else {
            return;
        };
        let f = BoardSecretFrame {
            board_id: board.clone(),
            board_epoch: newest,
            maker,
            sealed: base64::engine::general_purpose::STANDARD.encode(sealed),
        };
        let _ = self.send(p, &Wire::Secret(f)).await;
    }

    async fn take_secret(self: &Arc<Self>, p: &Peer, f: BoardSecretFrame) {
        if f.board_id != self.opts.board_id || f.board_epoch <= p.epoch || f.maker.is_empty() {
            return;
        }
        let Some(key) = self.keys.lock().expect("keys").get(&p.epoch).copied() else {
            return;
        };
        let Ok(sealed) = base64::engine::general_purpose::STANDARD.decode(&f.sealed) else {
            return;
        };
        let secret = match open_board_secret(
            &key,
            &self.opts.board_id,
            f.board_epoch,
            &sealed,
            &self.opts.self_id,
        ) {
            Ok(s) => Zeroizing::new(s),
            Err(e) => {
                self.logf(&format!("board.secret from {}: {e}", p.conn.id()));
                return;
            }
        };
        if let Err(e) = self.opts.secrets.add(f.board_epoch, &f.maker, &secret) {
            self.logf(&format!("board.secret from {}: {e}", p.conn.id()));
            return;
        }
        let s = self.clone();
        tokio::spawn(async move {
            if let Err(e) = s.refresh().await {
                s.logf(&format!("refresh: {e}"));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_secret_seals_to_one_recipient_and_epoch() {
        let key = [3u8; 32];
        let board = "00112233445566778899aabbccddeeff";
        let secret = [9u8; 32];
        let sealed = seal_board_secret(&key, board, 2, &secret, "mac").unwrap();
        assert_eq!(
            open_board_secret(&key, board, 2, &sealed, "mac").unwrap(),
            secret
        );
        assert!(open_board_secret(&key, board, 2, &sealed, "pc").is_err());
        assert!(open_board_secret(&key, board, 3, &sealed, "mac").is_err());
        assert!(open_board_secret(&[4u8; 32], board, 2, &sealed, "mac").is_err());
        assert!(open_board_secret(&key, board, 2, &sealed[..20], "mac").is_err());
    }

    #[test]
    fn wire_frames_are_tagged_by_type() {
        let b = serde_json::to_value(Wire::Vector {
            vector: [("a".to_string(), 3)].into_iter().collect(),
        })
        .unwrap();
        assert_eq!(b, serde_json::json!({"type": "vector", "vector": {"a": 3}}));
        let h = serde_json::to_value(Wire::Hello(BoardHello {
            v: 1,
            board_id: "b".into(),
            ..Default::default()
        }))
        .unwrap();
        assert_eq!(h["type"], "hello");
        assert_eq!(h["boardId"], "b");
        assert!(serde_json::from_slice::<Wire>(br#"{"type":"later.frame"}"#).is_err());
    }
}
