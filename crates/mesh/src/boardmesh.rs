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
//!   `ping`/`pong`, `board.secret` ([`BoardSecretFrame`], a newer epoch's
//!   secret sealed under this connection's epoch key), and `dm`
//!   ([`DmFrame`], a step of a DM hand-off, handed to [`DmHooks`]).
//! - Rows move on `board/<boardId>` pull streams ([`crate::boards`]). A
//!   writer whose row this device refused goes on hold from that peer, with
//!   a wait that doubles, and [`BoardMesh::stalled`] says so; the vector
//!   frames and hello keep saying where this device really is.
//! - Files on the board's cards move on `blob/<16 hex>` streams, as on the
//!   private mesh (v0.5 §5.5): served only to a counted member device, and
//!   only a hash this person's devices offered in this board's log
//!   ([`BoardMesh::may_serve`]).
//! - Any other stream label is refused.
//!
//! Each hello's `boardEpoch` is kept on its connection. Once every
//! remaining member device (the verifier's counted devices and this
//! person's own) has said hello with the newest epoch, the mesh settles
//! [`BoardSecrets`] there and leaves the older epoch's room, so a removed
//! person, who never gets the new secret, has no topic left in common
//! with the board (§6.1).
//!
//! Beside each room the mesh joins that epoch's gossip topic when the
//! transport runs one ([`BoardTransport::board_gossip`], boards2-transport
//! §5): [`BoardMesh::kick`] (a local write) announces this device's vector
//! there, and an announcement ahead of the local log pulls at once, so rows
//! cross without waiting for the vector tick.
//!
//! The private mesh (`irori/mesh/1`) never carries a board row.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Weak};

use base64::Engine as _;
use blob::FileRef;
use crypto_secretbox::aead::{Aead, KeyInit};
use crypto_secretbox::{Nonce, XSalsa20Poly1305};
use hkdf::Hkdf;
use identity::{BoardSecrets, BoardVerifier};
use irori_meshseam::{
    BoardGossip, BoardRoom, BoardTransport, Conn, MSG_PRESENCE, Refused, Room, RoomConfig, Secret,
    TransportError, reason,
};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use store::Store;
use tokio::io::AsyncWriteExt as _;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::boards::{BOARD_LABEL_PREFIX, RefusedRow, pull_board, serve_pull};
use crate::files::{
    BLOB_LABEL_PREFIX, FetchError, FileOffer, check_ref, fetch_over, holders_in, serve_blob,
};
use crate::node::Intervals;
use crate::servers::PublicAddr;
use crate::transport::Stream;

/// The board sync protocol version a board hello carries (§6.3 "Version").
pub const BOARD_PROTOCOL: i64 = 1;

/// The space rows this build reads (card #392, decision 46): a board
/// hello's `spaces`. A device that never said it (an older Irori) refuses
/// a kind it doesn't know, and every later row of that writer's stream
/// then waits on the gap, so a space writes no space-only kind (a second
/// board, a channel) while a member device is known to read less than this
/// (no-server.md §5: one nobody has any word from gets a warning instead).
/// It's a hello field, not `v`, so older and newer Irori keep syncing.
pub const SPACES_PROTOCOL: i64 = 1;

/// The spawned-agent rows this build reads in a board log (`task.spawn`,
/// `task.spawned`; spawned agents part 11, card #455): a board hello's
/// `spawns`. Like [`SPACES_PROTOCOL`], an older Irori refuses them and the
/// writer's stream then waits on the gap, so a device starts agents for a
/// board's cards only once every member device has said at least this.
pub const SPAWNS_PROTOCOL: i64 = 1;

/// The `board.ask.moved` row this build reads (card #541): a board hello's
/// `moves`. As with [`SPAWNS_PROTOCOL`], an older Irori refuses it and the
/// writer's stream waits on the gap, so a computer takes a board's ask only
/// once every member device has said at least this.
pub const MOVES_PROTOCOL: i64 = 1;

/// The hand-off of a person-to-person DM this build speaks (`dm` frames,
/// card #538): a board hello's `dms`. Built like [`SPAWNS_PROTOCOL`]: an
/// older Irori skips the frame and says 0, so its device is never asked.
pub const DMS_PROTOCOL: i64 = 1;

/// The call layer this build speaks (`call` frames, card #654): a board
/// hello's `calls`. Built like [`DMS_PROTOCOL`]: an older Irori skips the
/// frame and says 0, so its device is never rung.
pub const CALLS_PROTOCOL: i64 = 1;

/// This build's tolerant reader (no-server.md §3.5): a board hello's
/// `tolerant`. A device saying at least this stores a pulled `task.*` or
/// `space.*` kind it doesn't know instead of stalling on it, so once a
/// space's devices are past it a new kind of those needs no gate.
pub const TOLERANT_PROTOCOL: i64 = 1;

/// The longest a refused writer waits before it's tried again from a peer
/// (no-server.md §4.3). The first wait is one vector tick, then it doubles.
const HOLD_MAX: std::time::Duration = std::time::Duration::from_secs(3600);

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
/// The kind of a saved hello ([`SavedHello`]), a device-local row.
pub const KIND_BOARD_HELLO: &str = "board.hello";

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
    /// The space rows the sender reads ([`SPACES_PROTOCOL`]); 0 (absent)
    /// from an Irori before spaces.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub spaces: i64,
    /// The spawned-agent rows the sender reads ([`SPAWNS_PROTOCOL`]); 0
    /// (absent) from an Irori before them.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub spawns: i64,
    /// The ask moves the sender reads ([`MOVES_PROTOCOL`]); 0 (absent)
    /// from an Irori before them.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub moves: i64,
    /// The DM hand-off the sender speaks ([`DMS_PROTOCOL`]); 0 (absent)
    /// from an Irori before it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub dms: i64,
    /// The call layer the sender speaks ([`CALLS_PROTOCOL`]); 0 (absent)
    /// from an Irori before it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub calls: i64,
    /// Whether the sender stores kinds it doesn't know
    /// ([`TOLERANT_PROTOCOL`]); 0 (absent) from an Irori before it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tolerant: i64,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
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

/// `dm {boardId, step, body}`: one step of a DM hand-off (card #538,
/// docs/design/talk-dms.md §2). It rides on a connection of a space both
/// people share, so `board_id` names the hidden DM space, not that space.
/// `body` is opaque here: invite depends on mesh, so its Bundle can't be
/// named.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DmFrame {
    pub board_id: String,
    pub step: String,
    #[serde(default)]
    pub body: serde_json::Value,
}

/// `call {spaceId, callId, op, convId, epoch, devices, atMs}`: one step of
/// a voice call's ringing (card #654). It rides on a connection of the
/// space the call is in, so `space_id` must name this board.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallFrame {
    pub space_id: String,
    /// The call it's about: the same id on every frame of one call.
    pub call_id: String,
    /// `ring`, `join`, `leave` or `end`. A string, not an enum, so a newer
    /// op still reaches the hooks instead of dropping the frame.
    pub op: String,
    /// The conversation the call is in: a channel or a DM.
    #[serde(default)]
    pub conv_id: String,
    /// The topic epoch the sender proves call connections with.
    #[serde(default)]
    pub epoch: i64,
    /// The devices in the call this sender knows of, so each one dials the
    /// rest (a full mesh).
    #[serde(default)]
    pub devices: Vec<String>,
    /// When it was sent, Unix ms.
    #[serde(default)]
    pub at_ms: i64,
}

/// Leaves `body` out: an `offer`'s holds the space's secret, as `Bundle`'s
/// own `Debug` knows.
impl std::fmt::Debug for DmFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DmFrame")
            .field("board_id", &self.board_id)
            .field("step", &self.step)
            .finish_non_exhaustive()
    }
}

/// The least time between two gossip announcements of one board (kicks
/// in between coalesce into the next one).
const ANNOUNCE_GAP: std::time::Duration = std::time::Duration::from_millis(200);
/// The least time between two out-of-turn dials of missing member devices
/// (a kick or an announcement while one is not connected).
const DIAL_GAP: std::time::Duration = std::time::Duration::from_secs(1);

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

/// A `board.server` payload (B1, §6.2): written by every device into each
/// board its person is in, only when it changes. `public: None` (JSON
/// `null`) when the server is off, outside reach is off, or the device has
/// no server at all (`port` 0: a phone, a desktop that never served). Never
/// LAN or v6 addresses.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BoardServer {
    pub device_id: String,
    pub port: i64,
    pub public: Option<PublicAddr>,
    /// What the device reads (card #577). `None` from a build before this
    /// field means no word, not level 0: every older home server would
    /// read as too old otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reads: Option<BoardReads>,
    /// The device's core version ("" from a build before this field).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub version: String,
}

/// The levels a device reads, as its board hello says them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BoardReads {
    pub spaces: i64,
    pub spawns: i64,
    pub tolerant: i64,
}

impl BoardReads {
    /// What this build reads.
    pub fn ours() -> BoardReads {
        BoardReads {
            spaces: SPACES_PROTOCOL,
            spawns: SPAWNS_PROTOCOL,
            tolerant: TOLERANT_PROTOCOL,
        }
    }
}

/// The last hello this device heard from `device_id` on a board (card
/// #577): a `board.hello` row in the board log's device-local rows, written
/// when what it says changes, so a restart forgets nothing. Kept apart from
/// [`PeerHeard`], which says who is talking now.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SavedHello {
    pub device_id: String,
    pub person_id: String,
    pub core_version: String,
    pub spaces: i64,
    pub spawns: i64,
    pub tolerant: i64,
    /// How many rows `device_id` had written to the board when it said
    /// this (its own entry in the hello's vector). It orders the hello
    /// against that device's `board.server` rows on its own counter, so no
    /// clock decides which is newer.
    pub seq: i64,
    /// Unix milliseconds this device saved it (for display only).
    pub at_ms: i64,
}

impl SavedHello {
    fn says_the_same(&self, o: &SavedHello) -> bool {
        (
            &self.person_id,
            &self.core_version,
            self.spaces,
            self.spawns,
            self.tolerant,
        ) == (
            &o.person_id,
            &o.core_version,
            o.spaces,
            o.spawns,
            o.tolerant,
        )
    }
}

/// The newest [`SavedHello`] of each device in board log `log`.
pub fn saved_hellos(log: &Store) -> Result<BTreeMap<String, SavedHello>, TransportError> {
    let mut newest = BTreeMap::new();
    each_row(log, KIND_BOARD_HELLO, true, |ev| {
        if let Ok(h) = serde_json::from_slice::<SavedHello>(&ev.payload) {
            newest.insert(h.device_id.clone(), h);
        }
    })?;
    Ok(newest)
}

/// What a board log holds on which devices read what, with no connection
/// needed (card #577, no-server.md §5): saved hellos and `board.server`
/// rows. A live hello beats both, and the caller holds those
/// ([`BoardMesh::live_spaces`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceWords {
    hellos: BTreeMap<String, SavedHello>,
    /// Each device's newest row that says `reads`, and that row's seq in
    /// the device's own stream. A newest row without it (a downgrade to an
    /// older home server) is no word.
    rows: BTreeMap<String, (BoardReads, i64)>,
}

impl DeviceWords {
    pub fn read(log: &Store) -> Result<DeviceWords, TransportError> {
        let mut rows = BTreeMap::new();
        each_row(log, KIND_BOARD_SERVER, false, |ev| {
            let Ok(s) = serde_json::from_slice::<BoardServer>(&ev.payload) else {
                return;
            };
            if s.device_id != ev.device_id {
                return;
            }
            match s.reads {
                Some(r) => rows.insert(s.device_id, (r, ev.seq)),
                None => rows.remove(&s.device_id),
            };
        })?;
        Ok(DeviceWords {
            hellos: saved_hellos(log)?,
            rows,
        })
    }

    /// The `spaces` level of `dev`'s latest word, None when there is none.
    /// A saved hello and a row go by which `dev` wrote later, on its own
    /// row counter: an updated device's row can come through a third
    /// device long after its last hello here, and a hello after a row is a
    /// downgrade.
    pub fn spaces(&self, dev: &str) -> Option<i64> {
        match (self.hellos.get(dev), self.rows.get(dev)) {
            (Some(h), Some((r, seq))) if *seq > h.seq => Some(r.spaces),
            (Some(h), _) => Some(h.spaces),
            (None, Some((r, _))) => Some(r.spaces),
            (None, None) => None,
        }
    }
}

impl DeviceWords {
    /// Whether hello `h` adds anything to what's known. The same hello
    /// again doesn't, unless a row of that device's since says otherwise:
    /// then the hello is its latest word again (a downgrade after an
    /// update).
    fn needs_saving(&self, h: &SavedHello) -> bool {
        !(self
            .hellos
            .get(&h.device_id)
            .is_some_and(|s| s.says_the_same(h))
            && self.spaces(&h.device_id) == Some(h.spaces))
    }
}

/// Calls `f` with every `kind` row of `log`, oldest first.
fn each_row(
    log: &Store,
    kind: &str,
    local: bool,
    mut f: impl FnMut(&store::Event),
) -> Result<(), TransportError> {
    const PAGE: i64 = 1000;
    let mut after = store::Cursor::default();
    loop {
        let rows = log
            .list(&store::Filter {
                kinds: vec![kind.into()],
                after: after.clone(),
                limit: PAGE,
                local,
                ..Default::default()
            })
            .map_err(|e| TransportError(format!("{kind}: {e}")))?;
        rows.iter().for_each(&mut f);
        match rows.last() {
            Some(last) if rows.len() as i64 == PAGE => after = last.cursor(),
            _ => return Ok(()),
        }
    }
}

/// The board's `board.server` fold: the newest row per server device whose
/// writer is that device and a counted, uncut member device, by device id.
/// A server whose newest row says `public: null` stays in the list (it is
/// off or unreachable from outside: nothing to dial).
pub fn board_servers(
    log: &Store,
    verifier: &BoardVerifier,
) -> Result<Vec<BoardServer>, TransportError> {
    let mut newest: BTreeMap<String, BoardServer> = BTreeMap::new();
    each_row(log, KIND_BOARD_SERVER, false, |ev| {
        let Ok(s) = serde_json::from_slice::<BoardServer>(&ev.payload) else {
            return;
        };
        // Only the server's own device announces it.
        if s.device_id == ev.device_id {
            newest.insert(s.device_id.clone(), s);
        }
    })?;
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

/// When `dev` wrote its row `seq` in `log`, by its HLC, in Unix ms.
fn newest_wall_ms(log: &Store, dev: &str, seq: i64) -> Option<i64> {
    if seq < 1 {
        return None;
    }
    let evs = log
        .list(&store::Filter {
            device_id: dev.into(),
            after_seq: seq - 1,
            limit: 1,
            ..store::Filter::default()
        })
        .ok()?;
    let ev = evs.into_iter().find(|e| e.seq == seq)?;
    let wall = ev.hlc.wall();
    wall.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
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
    /// This device's attachment store: files on the board's cards are
    /// served from it and fetched into it. `None`: no files on this board.
    pub files: Option<Arc<blob::Files>>,
    /// Who hears about DM hand-offs on this board's connections. `None`:
    /// `dm` frames are dropped.
    pub dms: Option<Arc<dyn DmHooks>>,
    /// Who hears about call ringing on this board's connections (card
    /// #654). `None`: `call` frames are dropped.
    pub calls: Option<Arc<dyn CallHooks>>,
}

/// This person's other devices (see [`BoardMeshOpts::own_devices`]).
pub type OwnDevicesFn = Arc<dyn Fn() -> Vec<MemberDevice> + Send + Sync>;

/// What a board's connections tell the Core for DMs (card #538). `via` is
/// the board whose connection it was. Each call runs on that connection's
/// own task, so it must return at once: a Core that folds a bundle or
/// touches a store queues the work, or the connection stalls.
pub trait DmHooks: Send + Sync {
    /// `peer` said hello, whatever it said about `dms`: a device that
    /// doesn't speak the hand-off is told apart from one not heard at all.
    fn hello(&self, via: &str, peer: &LivePeer);
    /// A vector tick on `peer`'s connection. A hello goes once per
    /// connection, so a pending hand-off retries here.
    fn tick(&self, via: &str, peer: &LivePeer);
    /// `peer` sent `frame`. It comes only from a device `via`'s verifier
    /// admits now: one of this person's devices that isn't certified there
    /// has nothing to say in a hand-off.
    fn frame(&self, via: &str, peer: &LivePeer, frame: DmFrame);
}

/// What a board's connections tell the Core for calls (card #654). `via` is
/// the board whose connection it was. As with [`DmHooks`], each call runs
/// on that connection's own task, so it must return at once.
pub trait CallHooks: Send + Sync {
    /// `peer` said hello, whatever it said about `calls`: a device too old
    /// for calls is told apart from one not heard at all.
    fn hello(&self, via: &str, peer: &LivePeer);
    /// `peer` sent `frame` about a call in this space. It comes only from
    /// a device `via`'s verifier admits now.
    fn frame(&self, via: &str, peer: &LivePeer, frame: CallFrame);
}

/// A device with a live connection that said hello.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LivePeer {
    pub device_id: String,
    /// The key its handshake proved, hex.
    pub device_pub: String,
    /// The person the board's verifier certifies it to (this person, for
    /// an own device it doesn't count yet). Never its hello's word.
    pub person_id: String,
    /// The `dms` its last hello said.
    pub dms: i64,
    /// The `calls` its last hello said.
    pub calls: i64,
}

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
    #[serde(rename = "dm")]
    Dm(DmFrame),
    #[serde(rename = "call")]
    Call(CallFrame),
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
    /// The `spaces` its last hello said (card #392).
    pub spaces: i64,
    /// The `spawns` its last hello said (card #455).
    pub spawns: i64,
    /// The `moves` its last hello said (card #541).
    pub moves: i64,

    /// When a pull from it last left this device holding every row its
    /// newest vector named (card #580); 0 never. That vector may have come
    /// by gossip rather than this connection: either way it was its own.
    pub synced_ms: i64,
    /// The `dms` its last hello said (card #538).
    pub dms: i64,
    /// The `tolerant` its last hello said (card #579).
    pub tolerant: i64,
    /// The `calls` its last hello said (card #654).
    pub calls: i64,
}

/// A writer this device has stopped taking rows from: what the device says
/// about itself (no-server.md §4.3). Others see the same stall from its
/// vector, as [`PeerHeard`]'s lag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfStall {
    /// The writer.
    pub device_id: String,
    /// The row it stopped at.
    pub seq: i64,
    /// Unix milliseconds of the first refusal of that row.
    pub since_ms: i64,
    /// This build doesn't know the row's kind, so updating heals it.
    pub unread: bool,
}

/// A writer on hold from one peer: its stream stopped at `seq`, so pulls
/// from that peer skip it until `until`. Per peer, so a bad row one peer
/// sends can't hold up the same writer's rows from another. In memory: a
/// restart, which is how an update lands, tries again at once.
#[derive(Debug, Clone)]
struct Hold {
    seq: i64,
    tries: u32,
    until: std::time::Instant,
    since_ms: i64,
    unread: bool,
    past_cut: bool,
}

/// The writers on hold, by (writer, peer).
#[derive(Debug, Default)]
struct Holds(BTreeMap<(String, String), Hold>);

impl Holds {
    /// The writers to skip on a pull from `peer` at `now`, given this
    /// device's `vector`. A hold ends once its row is held, however it got
    /// here.
    fn skip(
        &mut self,
        peer: &str,
        vector: &BTreeMap<String, i64>,
        now: std::time::Instant,
    ) -> Vec<String> {
        self.0
            .retain(|(dev, _), h| vector.get(dev).copied().unwrap_or(0) < h.seq);
        self.0
            .iter()
            .filter(|((_, from), h)| from == peer && h.until > now)
            .map(|((dev, _), _)| dev.clone())
            .collect()
    }

    /// Puts each writer `peer` sent a refused row of on hold: one `tick`
    /// the first time, twice as long each time after, up to [`HOLD_MAX`].
    /// A new row refused starts over.
    fn hold(
        &mut self,
        peer: &str,
        refused: &[RefusedRow],
        tick: std::time::Duration,
        now: std::time::Instant,
        now_ms: i64,
    ) {
        for r in refused {
            let e = self
                .0
                .entry((r.device_id.clone(), peer.to_string()))
                .or_insert(Hold {
                    seq: r.seq,
                    tries: 0,
                    until: now,
                    since_ms: now_ms,
                    unread: r.unread,
                    past_cut: r.past_cut,
                });
            if e.seq != r.seq {
                e.seq = r.seq;
                e.tries = 0;
                e.since_ms = now_ms;
            }
            e.unread = r.unread;
            e.past_cut = r.past_cut;
            let wait = tick
                .saturating_mul(2u32.saturating_pow(e.tries))
                .min(HOLD_MAX);
            e.tries = e.tries.saturating_add(1);
            e.until = now + wait;
        }
    }

    /// One line per writer stopped short of `vector`'s next rows. A row a
    /// revoked device wrote past its cut isn't a stall: it's refused for
    /// good, and rightly. Nor is a row `written` says nobody has: a junk row
    /// past a stream's end, or one a revoke has since cut off (a vector
    /// stops at the cut), would otherwise show for good.
    fn stalled(
        &self,
        vector: &BTreeMap<String, i64>,
        written: impl Fn(&str, i64) -> bool,
    ) -> Vec<SelfStall> {
        let mut out: BTreeMap<&str, SelfStall> = BTreeMap::new();
        for ((dev, _), h) in &self.0 {
            if h.past_cut || vector.get(dev).copied().unwrap_or(0) >= h.seq || !written(dev, h.seq)
            {
                continue;
            }
            let s = out.entry(dev).or_insert(SelfStall {
                device_id: dev.clone(),
                seq: h.seq,
                since_ms: h.since_ms,
                unread: false,
            });
            s.since_ms = s.since_ms.min(h.since_ms);
            s.unread |= h.unread;
        }
        out.into_values().collect()
    }
}

/// One board connection.
struct Peer {
    conn: Arc<dyn Conn>,
    epoch: i64,
    hello: AtomicBool,
    /// The board epoch the peer's hello named (at least the room's: the
    /// board-open proof shows it holds that one). 0 until the hello.
    said_epoch: AtomicI64,
    /// A removed person's device in its close grace ([`GRACE_NONE`],
    /// [`GRACE_OPEN`], [`GRACE_DONE`]): the board does not settle while
    /// one is open, so the removal reaches it on the older epoch first.
    grace: AtomicU8,
    wake: Notify,
}

const GRACE_NONE: u8 = 0;
const GRACE_OPEN: u8 = 1;
const GRACE_DONE: u8 = 2;

struct JoinedRoom {
    room: Arc<dyn Room>,
    key: [u8; 32],
    /// The epoch's gossip topic, when the transport runs gossip.
    gossip: Option<Arc<dyn BoardGossip>>,
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
    /// Every joined epoch's gossip topic, readable without the async lock.
    gossips: Mutex<BTreeMap<i64, Arc<dyn BoardGossip>>>,
    /// Wakes the announcer ([`BoardMesh::kick`]).
    announce: Notify,
    /// When missing member devices were last dialled out of turn.
    last_dial: Mutex<Option<std::time::Instant>>,
    /// The runtime [`BoardMesh::start`] ran on: a kick comes from any
    /// thread (a board's follower writes outside the runtime), and a dial
    /// spawns on it.
    rt: std::sync::OnceLock<tokio::runtime::Handle>,
    heard: Mutex<BTreeMap<String, PeerHeard>>,
    /// Refused writers, by (writer, peer).
    holds: Mutex<Holds>,
    /// Each connected peer's vector as its last frame said it. Unlike
    /// [`PeerHeard::vector`] it can go down (a revoke's cut), and it goes
    /// when the connection does.
    latest: Mutex<BTreeMap<String, BTreeMap<String, i64>>>,
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
                gossips: Mutex::new(BTreeMap::new()),
                announce: Notify::new(),
                last_dial: Mutex::new(None),
                rt: std::sync::OnceLock::new(),
                heard: Mutex::new(BTreeMap::new()),
                holds: Mutex::new(Holds::default()),
                latest: Mutex::new(BTreeMap::new()),
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

    /// The epochs whose gossip topics are joined (none when the transport
    /// runs no gossip).
    pub fn gossip_epochs(&self) -> Vec<i64> {
        self.s
            .gossips
            .lock()
            .expect("gossips")
            .keys()
            .copied()
            .collect()
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

    /// The board epoch each connected device's hello named, the highest of
    /// its connections, by device id.
    pub fn peer_epochs(&self) -> BTreeMap<String, i64> {
        self.s.said_epochs()
    }

    /// Joins the board's rooms (and their gossip topics) for every epoch
    /// `secrets` holds, and keeps dialling its member devices on the
    /// presence tick until [`close`](Self::close).
    pub async fn start(&self) -> Result<(), TransportError> {
        self.s.refresh().await?;
        if self.s.started.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let _ = self.s.rt.set(tokio::runtime::Handle::current());
        {
            let (w, cancel) = (Arc::downgrade(&self.s), self.s.cancel.clone());
            tokio::spawn(async move { Shared::announcer(w, cancel).await });
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

    /// A local write (or a board opened): pulls from every connected member
    /// device now, tells them this device's vector on each connection and
    /// on the board's gossip topic, and dials member devices not connected.
    pub fn kick(&self) {
        self.s.pull_now();
        self.s.announce.notify_one();
    }

    /// The `spaces` each device on a live connection said in its hello:
    /// the word that beats any saved one (no-server.md §5).
    pub fn live_spaces(&self) -> BTreeMap<String, i64> {
        let live = self.peers();
        let heard = self.s.heard.lock().expect("heard");
        live.into_iter()
            .filter_map(|(_, d)| heard.get(&d).map(|h| (d, h.spaces)))
            .collect()
    }

    /// The member devices not yet heard to read spawned-agent rows (card
    /// #455): until there are none, this device writes no `task.spawn` or
    /// `task.spawned` to the board. Every device must have been heard, on
    /// a hello now or a saved one: this gate is silent and on the busiest
    /// stream, so it doesn't let an unheard device through as space rows
    /// do (no-server.md §5).
    pub fn not_ready_for_spawns(&self) -> Vec<MemberDevice> {
        let devices = self.s.member_devices();
        let levels: Vec<Option<i64>> = {
            let heard = self.s.heard.lock().expect("heard");
            devices
                .iter()
                .map(|d| heard.get(&d.device_id).map(|h| h.spawns))
                .collect()
        };
        let saved = if levels.iter().any(Option::is_none) {
            saved_hellos(&self.s.opts.log).unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        devices
            .into_iter()
            .zip(levels)
            .filter(|(d, level)| {
                level
                    .or_else(|| saved.get(&d.device_id).map(|h| h.spawns))
                    .is_none_or(|l| l < SPAWNS_PROTOCOL)
            })
            .map(|(d, _)| d)
            .collect()
    }

    /// The member devices not yet heard to read ask moves (card #541):
    /// until there are none, no computer takes this board's asks.
    pub fn not_ready_for_moves(&self) -> Vec<MemberDevice> {
        let heard = self.s.heard.lock().expect("heard");
        self.s
            .member_devices()
            .into_iter()
            .filter(|d| {
                heard
                    .get(&d.device_id)
                    .is_none_or(|h| h.moves < MOVES_PROTOCOL)
            })
            .collect()
    }

    /// How fresh this board log is (card #580, no-server.md §4.3): per
    /// member device but this one, when this device was last in step with
    /// it, in Unix ms (0 never). Connected and holding all it last said it
    /// had: now. Else when a pull from it last did, or, with neither since
    /// Irori started, when it wrote its newest row this log holds (in step
    /// at least that recently).
    pub fn freshness(&self) -> Vec<(String, i64)> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let up: std::collections::BTreeSet<String> =
            self.peers().into_iter().map(|(_, d)| d).collect();
        let heard = self.s.heard.lock().expect("heard").clone();
        let ours = self.s.opts.log.vector().unwrap_or_default();
        let mut devs: std::collections::BTreeSet<String> = self
            .s
            .member_devices()
            .into_iter()
            .map(|d| d.device_id)
            .collect();
        devs.extend(heard.keys().cloned());
        devs.remove(&self.s.opts.self_id);
        devs.into_iter()
            .map(|dev| {
                let h = heard.get(&dev);
                let live = up.contains(&dev) && h.is_some_and(|h| !self.s.lacks(&h.vector));
                let at = if live {
                    now
                } else {
                    let synced = h.map_or(0, |h| h.synced_ms);
                    let wrote = ours
                        .get(&dev)
                        .and_then(|seq| newest_wall_ms(&self.s.opts.log, &dev, *seq))
                        .unwrap_or(0)
                        .min(now);
                    synced.max(wrote)
                };
                (dev, at)
            })
            .collect()
    }

    /// Another device whose newest board vector holds this device's row
    /// `seq` here (card #580): the row has left this device.
    pub fn taken_by(&self, seq: i64) -> Option<String> {
        let me = &self.s.opts.self_id;
        self.s
            .heard
            .lock()
            .expect("heard")
            .iter()
            .find(|(d, h)| *d != me && h.vector.get(me).is_some_and(|s| *s >= seq))
            .map(|(d, _)| d.clone())
    }

    /// The devices connected now that said hello, each once, by device id.
    pub fn live_peers(&self) -> Vec<LivePeer> {
        let mut out: BTreeMap<String, LivePeer> = BTreeMap::new();
        for p in self.s.peer_list() {
            if p.hello.load(Ordering::SeqCst)
                && p.conn.err().is_none()
                && let Some(l) = self.s.live(&p)
            {
                out.insert(l.device_id.clone(), l);
            }
        }
        out.into_values().collect()
    }

    /// Sends `frame` to `device` alone (card #538), on its connection of
    /// the newest epoch. Refused when it isn't connected, its cert no longer
    /// counts here, or it never said it speaks [`DMS_PROTOCOL`], so a device
    /// that doesn't say it is never asked.
    pub async fn send_to(&self, device: &str, frame: DmFrame) -> Result<(), String> {
        let s = &self.s;
        let p = s
            .live_peer(device)
            .ok_or_else(|| format!("{device} isn't connected"))?;
        if !admitted(&s.opts.verifier, device, &p.conn.pub_key()) {
            return Err(format!("{device} isn't a member device here"));
        }
        let said = s
            .heard
            .lock()
            .expect("heard")
            .get(device)
            .map_or(0, |h| h.dms);
        if said < DMS_PROTOCOL {
            return Err(format!("{device} doesn't speak the DM hand-off"));
        }
        s.send(&p, &Wire::Dm(frame)).await
    }

    /// Sends a call `frame` to `device` alone (card #654), on its
    /// connection of the newest epoch. Refused as [`Self::send_to`] is, and
    /// when it never said it speaks [`CALLS_PROTOCOL`].
    pub async fn send_call(&self, device: &str, frame: CallFrame) -> Result<(), String> {
        let s = &self.s;
        let p = s
            .live_peer(device)
            .ok_or_else(|| format!("{device} isn't connected"))?;
        if !admitted(&s.opts.verifier, device, &p.conn.pub_key()) {
            return Err(format!("{device} isn't a member device here"));
        }
        let said = s
            .heard
            .lock()
            .expect("heard")
            .get(device)
            .map_or(0, |h| h.calls);
        if said < CALLS_PROTOCOL {
            return Err(format!("{device}'s Irori is too old for calls"));
        }
        s.send(&p, &Wire::Call(frame)).await
    }

    /// The newest joined epoch and its topic key: what a call connection
    /// is proved with (card #654). `None` before any epoch is joined.
    pub fn newest_key(&self) -> Option<(i64, [u8; 32])> {
        self.s
            .keys
            .lock()
            .expect("keys")
            .last_key_value()
            .map(|(e, k)| (*e, *k))
    }

    /// Every joined epoch's topic key, newest first: the newest, and the
    /// previous until it settles. A call connection's proof is checked
    /// against these (card #654).
    pub fn topic_keys(&self) -> Vec<(i64, [u8; 32])> {
        self.s
            .keys
            .lock()
            .expect("keys")
            .iter()
            .rev()
            .map(|(e, k)| (*e, *k))
            .collect()
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

    /// The writers this device has stopped taking rows from, by writer.
    pub fn stalled(&self) -> Vec<SelfStall> {
        let vector = self.s.opts.log.vector().unwrap_or_default();
        self.s.stalled(&vector)
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

    /// Whether this device serves `sha` on this board (v0.5 §5.5): it holds
    /// the file and one of this person's own devices wrote an
    /// `attach.offered` for it in this board's log (offered it here, or
    /// fetched it for this board). An offer another person wrote never
    /// makes this device serve, so a member who knows or guesses the hash
    /// of a private file can't pull it.
    pub fn may_serve(&self, sha: &str) -> bool {
        self.s.may_serve(sha)
    }

    /// Whether this board's log offers `sha` (any member's device).
    pub fn offers(&self, sha: &str) -> bool {
        holders_in(&self.s.opts.log, sha).is_ok_and(|h| !h.is_empty())
    }

    /// Makes sure `r`'s file (a file on this board's cards) is in this
    /// device's attachment store, fetching it on a `blob/` stream from a
    /// connected member device: the origin first, then every other device
    /// that offered it in the board's log. Once fetched, this device offers
    /// it in the board's log too, so it serves it on. Returns the device it
    /// came from ("" when it was here). The caller holds the fetch turn
    /// ([`crate::Node::lock_fetch`]); dropping the future gives up.
    pub async fn fetch_file(&self, r: &FileRef) -> Result<String, FetchError> {
        let s = &self.s;
        let Some(fs) = s.opts.files.clone() else {
            return Err(FetchError::NoFiles);
        };
        check_ref(&fs, r)?;
        if fs.has(&r.sha256) {
            return Ok(String::new());
        }
        let held =
            holders_in(&s.opts.log, &r.sha256).map_err(|e| FetchError::Other(e.to_string()))?;
        let me = &s.opts.self_id;
        let mut cands: Vec<String> = Vec::new();
        if !r.device.is_empty() && r.device != *me {
            cands.push(r.device.clone());
        }
        for h in held {
            if h != *me && !cands.contains(&h) {
                cands.push(h);
            }
        }
        if cands.is_empty() {
            return Err(FetchError::Offline(Vec::new()));
        }
        let mut last: Option<FetchError> = None;
        for dev in &cands {
            let Some(p) = s.live_peer(dev) else {
                continue;
            };
            match fetch_over(p.conn.as_ref(), &fs, r).await {
                Ok(()) => {
                    if let Err(e) = s.offer(r) {
                        s.logf(&format!("offer {}: {e}", r.sha256));
                    }
                    return Ok(dev.clone());
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or(FetchError::Offline(cands)))
    }

    /// Leaves every room of the board.
    pub fn close(&self) {
        self.s.cancel.cancel();
        let rooms = std::mem::take(&mut *self.s.rooms_list.lock().expect("rooms"));
        for r in rooms {
            r.close();
        }
        self.s.keys.lock().expect("keys").clear();
        self.s.gossips.lock().expect("gossips").clear();
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

    /// Pulls from every connected member device now; dials the member
    /// devices with no live connection (at most once per [`DIAL_GAP`]).
    fn pull_now(&self) {
        let peers = self.peer_list();
        for p in &peers {
            p.wake.notify_one();
        }
        let missing = self.member_devices().iter().any(|d| {
            !peers
                .iter()
                .any(|p| p.conn.id() == d.device_id && p.hello.load(Ordering::SeqCst))
        });
        if !missing || self.cancel.is_cancelled() {
            return;
        }
        {
            let mut last = self.last_dial.lock().expect("last_dial");
            if last.is_some_and(|t| t.elapsed() < DIAL_GAP) {
                return;
            }
            *last = Some(std::time::Instant::now());
        }
        let rooms = self.rooms_list.lock().expect("rooms").clone();
        // Off the runtime (a follower thread's kick), dial on the one the
        // mesh started on; never started, there is nothing to dial with.
        let _g = if tokio::runtime::Handle::try_current().is_ok() {
            None
        } else {
            match self.rt.get() {
                Some(h) => Some(h.enter()),
                None => return,
            }
        };
        self.dial(&rooms);
    }

    /// Announces this device's vector on every gossip topic after each
    /// [`BoardMesh::kick`], when it moved since the last announcement;
    /// kicks closer than [`ANNOUNCE_GAP`] coalesce.
    async fn announcer(w: Weak<Shared>, cancel: CancellationToken) {
        let mut last: Option<BTreeMap<String, i64>> = None;
        loop {
            let Some(s) = w.upgrade() else { return };
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = s.announce.notified() => {}
            }
            let vector = s.opts.log.vector().unwrap_or_default();
            if last.as_ref() != Some(&vector) {
                let gossips: Vec<_> = s
                    .gossips
                    .lock()
                    .expect("gossips")
                    .values()
                    .cloned()
                    .collect();
                for g in gossips {
                    if let Err(e) = g.announce(vector.clone()).await {
                        s.logf(&format!("announce: {e}"));
                    }
                }
                last = Some(vector);
            }
            drop(s);
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(ANNOUNCE_GAP) => {}
            }
        }
    }

    /// Joins epoch `secret.epoch`'s gossip topic, bootstrapping from the
    /// member devices; `None` when the transport runs no gossip or the
    /// join failed (logged: the connections' vector frames still move rows).
    async fn join_gossip(self: &Arc<Self>, secret: Secret) -> Option<Arc<dyn BoardGossip>> {
        let have: irori_meshseam::VectorFn = {
            let log = self.opts.log.clone();
            Arc::new(move || log.vector().unwrap_or_default())
        };
        let kick: irori_meshseam::KickFn = {
            let w = Arc::downgrade(self);
            Arc::new(move || {
                if let Some(s) = w.upgrade() {
                    s.pull_now();
                }
            })
        };
        let epoch = secret.epoch;
        match self
            .opts
            .transport
            .board_gossip(
                BoardRoom {
                    board_id: self.opts.board_id.clone(),
                    secret,
                },
                self.member_keys(),
                have,
                kick,
            )
            .await
        {
            Ok(g) => g.map(Arc::from),
            Err(e) => {
                self.logf(&format!("gossip epoch {epoch}: {e}"));
                None
            }
        }
    }

    /// The member devices' keys (gossip bootstrap).
    fn member_keys(&self) -> Vec<[u8; 32]> {
        self.member_devices()
            .iter()
            .filter_map(|d| hex::decode(&d.device_pub).ok()?.try_into().ok())
            .collect()
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

    /// See [`BoardMesh::may_serve`].
    fn may_serve(&self, sha: &str) -> bool {
        let Some(fs) = &self.opts.files else {
            return false;
        };
        if !blob::valid_id(sha) || !fs.has(sha) {
            return false;
        }
        let me = self.opts.person_id.as_str();
        let Ok(hs) = holders_in(&self.opts.log, sha) else {
            return false;
        };
        if hs.iter().any(|d| {
            *d == self.opts.self_id || self.opts.verifier.person_of(d).as_deref() == Some(me)
        }) {
            return true;
        }
        // This person's devices the board's rows don't certify yet.
        self.opts
            .own_devices
            .as_ref()
            .is_some_and(|f| f().iter().any(|o| hs.contains(&o.device_id)))
    }

    /// Answers one `blob/` stream from `peer`: only while it is a counted
    /// member device (re-checked now: membership may have moved since the
    /// connection opened), and only a hash [`Shared::may_serve`] allows.
    async fn serve_file(&self, peer: &str, pubkey: &[u8; 32], st: &mut Stream) {
        let Some(fs) = self.opts.files.clone() else {
            return;
        };
        if !admitted(&self.opts.verifier, peer, pubkey) {
            return;
        }
        let log = |l: &str| self.logf(&format!("{l} (to {peer})"));
        serve_blob(st, &fs, |sha| self.may_serve(sha), log).await;
    }

    /// A live connection to `dev` that said hello, on the newest epoch.
    fn live_peer(&self, dev: &str) -> Option<Arc<Peer>> {
        self.peers
            .lock()
            .expect("peers")
            .iter()
            .filter(|((_, d), p)| {
                d == dev && p.hello.load(Ordering::SeqCst) && p.conn.err().is_none()
            })
            .max_by_key(|((e, _), _)| *e)
            .map(|(_, p)| p.clone())
    }

    /// `p` as the DM hooks see it; `None` once the board no longer admits
    /// it. The person comes from the verifier: a hello's is the peer's word.
    fn live(&self, p: &Peer) -> Option<LivePeer> {
        let (id, key) = (p.conn.id(), p.conn.pub_key());
        let v = &self.opts.verifier;
        let person_id = if admitted(v, id, &key) {
            v.person_of(id)?
        } else if admits(v, &self.opts.own_devices, id, &key) {
            self.opts.person_id.clone()
        } else {
            return None;
        };
        let (dms, calls) = self
            .heard
            .lock()
            .expect("heard")
            .get(id)
            .map_or((0, 0), |h| (h.dms, h.calls));
        Some(LivePeer {
            device_id: id.to_string(),
            device_pub: hex::encode(key),
            person_id,
            dms,
            calls,
        })
    }

    /// Tells the DM hooks about `p`: `f` gets them, this board and the peer.
    fn tell_dms(&self, p: &Peer, f: impl FnOnce(&dyn DmHooks, &str, &LivePeer)) {
        if let Some(h) = &self.opts.dms
            && let Some(l) = self.live(p)
        {
            f(h.as_ref(), &self.opts.board_id, &l);
        }
    }

    /// Tells the call hooks about `p`, as [`Self::tell_dms`] does.
    fn tell_calls(&self, p: &Peer, f: impl FnOnce(&dyn CallHooks, &str, &LivePeer)) {
        if let Some(h) = &self.opts.calls
            && let Some(l) = self.live(p)
        {
            f(h.as_ref(), &self.opts.board_id, &l);
        }
    }

    /// This device's `attach.offered` for `r` in the board's log, once.
    fn offer(&self, r: &FileRef) -> Result<(), String> {
        let held = holders_in(&self.opts.log, &r.sha256).map_err(|e| e.to_string())?;
        if held.contains(&self.opts.self_id) {
            return Ok(());
        }
        self.opts
            .log
            .append(
                "irori",
                store::kinds::ATTACH_OFFERED,
                &FileOffer {
                    sha256: r.sha256.clone(),
                    size: r.size,
                    name: r.name.clone(),
                    mime: r.mime.clone(),
                },
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
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
                if p.grace
                    .compare_exchange(GRACE_NONE, GRACE_OPEN, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
                {
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
                            p.grace.store(GRACE_DONE, Ordering::SeqCst);
                            // The grace held a settle back: try it now.
                            if let Err(e) = me.refresh().await {
                                me.logf(&format!("refresh: {e}"));
                            }
                        });
                    }
                    Err(_) => {
                        p.grace.store(GRACE_DONE, Ordering::SeqCst);
                        p.conn.close(reason::REVOKED)
                    }
                }
            }
        }
    }

    /// The highest epoch each device has said hello with on a live
    /// connection.
    fn said_epochs(&self) -> BTreeMap<String, i64> {
        let mut out: BTreeMap<String, i64> = BTreeMap::new();
        for p in self.peer_list() {
            if !p.hello.load(Ordering::SeqCst) {
                continue;
            }
            let e = p.said_epoch.load(Ordering::SeqCst);
            let slot = out.entry(p.conn.id().to_string()).or_default();
            *slot = (*slot).max(e);
        }
        out
    }

    /// Settles the board's secrets on the newest epoch once every remaining
    /// member device has said hello with it (§6.1): the older epoch's topic
    /// then closes. A member device not connected keeps it open. Returns
    /// whether it settled now.
    fn maybe_settle(&self) -> bool {
        let secrets = &self.opts.secrets;
        let newest = secrets.epoch();
        if newest < 1 || secrets.settled() >= newest {
            return false;
        }
        let held = match secrets.topics() {
            Ok(t) => t,
            Err(e) => {
                self.logf(&format!("settle: {e}"));
                return false;
            }
        };
        if !held.iter().any(|t| t.epoch < newest) {
            return false;
        }
        // A removed person's device still pulling its removal keeps the
        // older epoch until its grace ends (recheck_conns).
        if self
            .peer_list()
            .iter()
            .any(|p| p.grace.load(Ordering::SeqCst) == GRACE_OPEN)
        {
            return false;
        }
        let said = self.said_epochs();
        let devs = self.member_devices();
        if devs
            .iter()
            .any(|d| said.get(&d.device_id).copied().unwrap_or(0) < newest)
        {
            return false;
        }
        if let Err(e) = secrets.settle(newest) {
            self.logf(&format!("settle {newest}: {e}"));
            return false;
        }
        self.logf(&format!(
            "every member device is on epoch {newest}: the older topic closes"
        ));
        for p in self.peer_list() {
            if p.epoch < newest {
                p.conn.close(reason::EPOCH);
            }
        }
        true
    }

    /// Joins every held epoch's room not joined yet, leaves the ones no
    /// longer held (settled), hands newer secrets on, and dials the member
    /// devices.
    async fn refresh(self: &Arc<Self>) -> Result<(), TransportError> {
        if self.cancel.is_cancelled() {
            return Ok(());
        }
        self.maybe_settle();
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
            let secret = Secret {
                key: t.key,
                topic: t.topic.clone(),
                epoch: t.epoch,
            };
            let room = self.join(secret.clone()).await?;
            let gossip = self.join_gossip(secret).await;
            if let Some(old) = rooms.insert(
                t.epoch,
                JoinedRoom {
                    room: room.clone(),
                    key: t.key,
                    gossip,
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
        // A settled epoch's topic leaves with its room: its handle drops
        // here and in `rooms`.
        let gossips: Vec<Arc<dyn BoardGossip>> =
            rooms.values().filter_map(|r| r.gossip.clone()).collect();
        *self.gossips.lock().expect("gossips") = rooms
            .iter()
            .filter_map(|(e, r)| Some((*e, r.gossip.clone()?)))
            .collect();
        drop(rooms);
        if self.cancel.is_cancelled() {
            for r in list {
                r.close();
            }
            self.gossips.lock().expect("gossips").clear();
            return Ok(());
        }
        // New member devices (and ones whose address only now came) join
        // the gossip topics too.
        let keys = self.member_keys();
        for g in gossips {
            if let Err(e) = g.join(keys.clone()).await {
                self.logf(&format!("gossip join: {e}"));
            }
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
            said_epoch: AtomicI64::new(0),
            grace: AtomicU8::new(GRACE_NONE),
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
            let w: Weak<Shared> = Arc::downgrade(self);
            let (log, board) = (self.opts.log.clone(), self.opts.board_id.clone());
            let (peer_id, peer_pub) = (conn.id().to_string(), conn.pub_key());
            conn.on_stream(Arc::new(move |st: irori_meshseam::Stream| {
                let (w, log, board, peer_id) =
                    (w.clone(), log.clone(), board.clone(), peer_id.clone());
                tokio::spawn(async move {
                    let want = format!("{BOARD_LABEL_PREFIX}{board}");
                    let label = st.label.clone();
                    let mut both: Stream = Box::new(tokio::io::join(st.recv, st.send));
                    if label == want {
                        serve_pull(log, &mut both).await;
                    } else if label.starts_with(BLOB_LABEL_PREFIX)
                        && let Some(s) = w.upgrade()
                    {
                        s.serve_file(&peer_id, &peer_pub, &mut both).await;
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
            spaces: SPACES_PROTOCOL,
            spawns: SPAWNS_PROTOCOL,
            moves: MOVES_PROTOCOL,
            dms: DMS_PROTOCOL,
            calls: CALLS_PROTOCOL,
            tolerant: TOLERANT_PROTOCOL,
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
        // Another connection to it, if any, says its vector again soon.
        self.latest.lock().expect("latest").remove(p.conn.id());
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
                    p.said_epoch
                        .store(h.board_epoch.max(p.epoch), Ordering::SeqCst);
                    p.hello.store(true, Ordering::SeqCst);
                    // Saved first, so whoever sees it heard finds it saved.
                    self.save_hello(&h);
                    self.hear(p.conn.id(), Some(&h), &h.vector);
                    if self.behind(p.conn.id(), &h.vector) {
                        p.wake.notify_one();
                    }
                    self.tell_dms(&p, |d, via, l| d.hello(via, l));
                    self.tell_calls(&p, |c, via, l| c.hello(via, l));
                    self.share_secrets(&p).await;
                    if self.maybe_settle() {
                        let s = self.clone();
                        tokio::spawn(async move {
                            if let Err(e) = s.refresh().await {
                                s.logf(&format!("refresh: {e}"));
                            }
                        });
                    }
                }
                _ if !said_hello => {
                    p.conn.close(reason::FRAME_BEFORE_HELLO);
                    return;
                }
                Wire::Vector { vector } => {
                    self.hear(p.conn.id(), None, &vector);
                    if self.behind(p.conn.id(), &vector) {
                        p.wake.notify_one();
                    }
                }
                Wire::Ping {} => {
                    let _ = self.send(&p, &Wire::Pong {}).await;
                }
                Wire::Pong {} => {}
                Wire::Secret(f) => self.take_secret(&p, f).await,
                Wire::Dm(f) => {
                    if admitted(&self.opts.verifier, p.conn.id(), &p.conn.pub_key()) {
                        self.tell_dms(&p, |d, via, l| d.frame(via, l, f));
                    }
                }
                // Only about a call in this space, from a device it admits.
                Wire::Call(f) => {
                    if f.space_id == self.opts.board_id
                        && admitted(&self.opts.verifier, p.conn.id(), &p.conn.pub_key())
                    {
                        self.tell_calls(&p, |c, via, l| c.frame(via, l, f));
                    }
                }
            }
        }
    }

    /// Records what member device `dev` said (`hello`: its person, core
    /// version and what it reads).
    fn hear(&self, dev: &str, hello: Option<&BoardHello>, vector: &BTreeMap<String, i64>) {
        self.latest
            .lock()
            .expect("latest")
            .insert(dev.to_string(), vector.clone());
        let mut heard = self.heard.lock().expect("heard");
        let h = heard.entry(dev.to_string()).or_default();
        h.device_id = dev.to_string();
        if let Some(said) = hello {
            h.person_id = said.person_id.clone();
            h.core_version = said.core_version.clone();
            h.spaces = said.spaces;
            h.spawns = said.spawns;
            h.moves = said.moves;
            h.dms = said.dms;
            h.calls = said.calls;
            h.tolerant = said.tolerant;
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

    /// A pull from `dev` finished: when it left nothing of what `dev` last
    /// said it had missing here, this log is in step with it (card #580).
    fn note_step(&self, dev: &str) {
        let theirs = match self.heard.lock().expect("heard").get(dev) {
            Some(h) => h.vector.clone(),
            None => return,
        };
        if self.lacks(&theirs) {
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        if let Some(h) = self.heard.lock().expect("heard").get_mut(dev) {
            h.synced_ms = now;
        }
    }

    /// Whether `theirs` holds any row this device's board log lacks, held
    /// writers included: a stalled writer means this log isn't in step.
    fn lacks(&self, theirs: &BTreeMap<String, i64>) -> bool {
        let ours = self.opts.log.vector().unwrap_or_default();
        !crate::node::covers(&ours, theirs)
    }

    /// Saves hello `h` in the board log's local rows, when it says anything
    /// new. A failure costs only what a restart then forgets.
    fn save_hello(&self, h: &BoardHello) {
        let row = SavedHello {
            device_id: h.device_id.clone(),
            person_id: h.person_id.clone(),
            core_version: h.core_version.clone(),
            spaces: h.spaces,
            spawns: h.spawns,
            tolerant: h.tolerant,
            seq: h.vector.get(&h.device_id).copied().unwrap_or(0),
            at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64),
        };
        if let Ok(w) = DeviceWords::read(&self.opts.log)
            && !w.needs_saving(&row)
        {
            return;
        }
        if let Err(e) = self.opts.log.append_local("irori", KIND_BOARD_HELLO, &row) {
            self.logf(&format!("board.hello: {e}"));
        }
    }

    /// The writers to skip on a pull from `peer` now, given this device's
    /// `vector`. A hold ends once its row is held, however it got here.
    fn on_hold(&self, peer: &str, vector: &BTreeMap<String, i64>) -> Vec<String> {
        self.holds
            .lock()
            .expect("holds")
            .skip(peer, vector, std::time::Instant::now())
    }

    /// Puts each writer `peer` sent a refused row of on hold ([`Holds::hold`]).
    fn hold(&self, peer: &str, refused: &[RefusedRow]) {
        let tick = self.iv.lock().expect("iv").vector;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        self.holds.lock().expect("holds").hold(
            peer,
            refused,
            tick,
            std::time::Instant::now(),
            now_ms,
        );
    }

    /// [`Holds::stalled`], a row counting as written when a connected
    /// peer's last vector says so.
    fn stalled(&self, vector: &BTreeMap<String, i64>) -> Vec<SelfStall> {
        let holds = self.holds.lock().expect("holds");
        let latest = self.latest.lock().expect("latest");
        holds.stalled(vector, |dev, seq| {
            latest
                .values()
                .any(|v| v.get(dev).copied().unwrap_or(0) >= seq)
        })
    }

    /// Whether `theirs`, peer `peer`'s vector, holds a row this device's
    /// board log lacks, leaving out the writers on hold from that peer: a
    /// pull wouldn't ask for them, so waking for them only spins.
    fn behind(&self, peer: &str, theirs: &BTreeMap<String, i64>) -> bool {
        let ours = self.opts.log.vector().unwrap_or_default();
        let skip = self.on_hold(peer, &ours);
        theirs
            .iter()
            .filter(|(d, _)| !skip.contains(d))
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
            let skip = self.on_hold(p.conn.id(), &vector);
            if self.send(&p, &Wire::Vector { vector }).await.is_err() {
                return;
            }
            let r = tokio::select! {
                _ = ctx.cancelled() => return,
                r = pull_board(p.conn.as_ref(), &self.opts.board_id, self.opts.log.clone(), &skip) => r,
            };
            if r.is_ok() {
                self.note_step(p.conn.id());
            }
            match r {
                Ok(got) => {
                    for row in &got.refused {
                        self.logf(&format!("pull from {}: {}", p.conn.id(), row.reason));
                    }
                    self.hold(p.conn.id(), &got.refused);
                    if got.landed > 0 {
                        // Rows landed: membership may have moved (a removal,
                        // a new member, a revoked device).
                        self.recheck_conns();
                        let f = self.on_rows.lock().expect("on_rows").clone();
                        if let Some(f) = f {
                            f();
                        }
                        // Say at once what this device holds now (card
                        // #580), not at the next tick: the writer can tell
                        // its rows have left it, as when it syncs before
                        // sleep or quit.
                        let vector = self.opts.log.vector().unwrap_or_default();
                        if self.send(&p, &Wire::Vector { vector }).await.is_err() {
                            return;
                        }
                    }
                }
                Err(e) => self.logf(&format!("pull from {}: {e}", p.conn.id())),
            }
            loop {
                tokio::select! {
                    _ = ctx.cancelled() => return,
                    _ = tick.tick() => {
                        self.tell_dms(&p, |d, via, l| d.tick(via, l));
                        break;
                    }
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
    use std::time::Duration;

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

    /// Card #577: the latest word on a device, from saved hellos and rows.
    #[test]
    fn a_devices_latest_word_is_its_newer_saved_hello_or_row() {
        let hello = |spaces, seq| SavedHello {
            spaces,
            seq,
            ..Default::default()
        };
        let row = |spaces, seq| {
            (
                BoardReads {
                    spaces,
                    ..Default::default()
                },
                seq,
            )
        };
        let words = |h: Option<SavedHello>, r: Option<(BoardReads, i64)>| DeviceWords {
            hellos: h.into_iter().map(|h| ("d".to_string(), h)).collect(),
            rows: r.into_iter().map(|r| ("d".to_string(), r)).collect(),
        };
        assert_eq!(words(None, None).spaces("d"), None, "no word");
        assert_eq!(words(Some(hello(0, 5)), None).spaces("d"), Some(0));
        assert_eq!(words(None, Some(row(1, 5))).spaces("d"), Some(1));
        // It updated, and its row came through another device.
        let w = words(Some(hello(0, 5)), Some(row(1, 9)));
        assert_eq!(w.spaces("d"), Some(1));
        // It went back to an older build after that row.
        let w = words(Some(hello(0, 9)), Some(row(1, 5)));
        assert_eq!(w.spaces("d"), Some(0));
        // Its hello already counted the row: the hello is the later word.
        let w = words(Some(hello(0, 9)), Some(row(1, 9)));
        assert_eq!(w.spaces("d"), Some(0));
        // No clock decides: a hello saved "late" by a fast clock here still
        // loses to a row the device wrote after it.
        let mut h = hello(0, 5);
        h.at_ms = i64::MAX;
        assert_eq!(words(Some(h), Some(row(1, 9))).spaces("d"), Some(1));
    }

    /// Card #577: a hello that says what's saved writes nothing, unless a
    /// later row of that device says otherwise.
    #[test]
    fn the_same_hello_is_saved_again_only_after_a_row_said_otherwise() {
        let hello = |spaces, seq| SavedHello {
            device_id: "d".into(),
            core_version: "0.10.2".into(),
            spaces,
            seq,
            ..Default::default()
        };
        let mut w = DeviceWords::default();
        assert!(w.needs_saving(&hello(0, 5)), "first word");
        w.hellos.insert("d".into(), hello(0, 5));
        assert!(!w.needs_saving(&hello(0, 7)), "nothing new");
        // It updated (its row came through another device), then went back.
        w.rows.insert(
            "d".into(),
            (
                BoardReads {
                    spaces: 1,
                    ..Default::default()
                },
                9,
            ),
        );
        assert_eq!(w.spaces("d"), Some(1));
        let back = hello(0, 12);
        assert!(w.needs_saving(&back));
        w.hellos.insert("d".into(), back);
        assert_eq!(w.spaces("d"), Some(0), "the downgrade is its latest word");
    }

    /// Card #392: an older Irori's hello has no `spaces` and still parses
    /// (as 0, not ready); ours says SPACES_PROTOCOL, and an older Irori
    /// reading ours just skips the field (serde ignores unknown fields).
    #[test]
    fn hello_says_it_reads_spaces() {
        let old: Wire = serde_json::from_str(
            r#"{"type":"hello","v":1,"deviceId":"d","personId":"p","boardId":"b","boardEpoch":1,"coreVersion":"0.10.2","schema":1,"vector":{},"wallMs":0}"#,
        )
        .unwrap();
        let Wire::Hello(h) = old else {
            panic!("not a hello")
        };
        assert_eq!(h.spaces, 0);
        let ours = serde_json::to_value(Wire::Hello(BoardHello {
            v: BOARD_PROTOCOL,
            spaces: SPACES_PROTOCOL,
            ..Default::default()
        }))
        .unwrap();
        assert_eq!(ours["spaces"], SPACES_PROTOCOL);
        assert_eq!(ours["v"], 1, "v stays 1: older Irori keeps syncing");
        let none = serde_json::to_value(Wire::Hello(BoardHello::default())).unwrap();
        assert!(none.get("spaces").is_none(), "0 isn't sent");
    }

    /// Card #541: an older Irori's hello has no `moves` and parses as 0, so
    /// [`BoardMesh::not_ready_for_moves`] counts it and no computer takes
    /// the board's asks; ours says MOVES_PROTOCOL.
    #[test]
    fn hello_says_it_reads_moves() {
        let old: Wire = serde_json::from_str(
            r#"{"type":"hello","v":1,"deviceId":"d","personId":"p","boardId":"b","boardEpoch":1,"coreVersion":"0.12.3","schema":1,"vector":{},"wallMs":0,"spaces":1,"spawns":1}"#,
        )
        .unwrap();
        let Wire::Hello(h) = old else {
            panic!("not a hello")
        };
        assert_eq!(h.moves, 0);
        let ours = serde_json::to_value(Wire::Hello(BoardHello {
            v: BOARD_PROTOCOL,
            moves: MOVES_PROTOCOL,
            ..Default::default()
        }))
        .unwrap();
        assert_eq!(ours["moves"], MOVES_PROTOCOL);
        assert_eq!(ours["v"], 1, "v stays 1: older Irori keeps syncing");
    }

    /// Card #538: `dms` is built like `spawns`. An older hello parses as 0,
    /// ours round-trips, 0 isn't sent, and `v` stays 1.
    #[test]
    fn hello_says_it_speaks_dms() {
        let old: Wire = serde_json::from_str(
            r#"{"type":"hello","v":1,"deviceId":"d","personId":"p","boardId":"b","boardEpoch":1,"coreVersion":"0.10.2","schema":1,"vector":{},"wallMs":0,"spaces":1,"spawns":1}"#,
        )
        .unwrap();
        let Wire::Hello(h) = old else {
            panic!("not a hello")
        };
        assert_eq!(h.dms, 0);
        let ours = BoardHello {
            v: BOARD_PROTOCOL,
            dms: DMS_PROTOCOL,
            ..Default::default()
        };
        let b = serde_json::to_vec(&Wire::Hello(ours.clone())).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(v["dms"], DMS_PROTOCOL);
        assert_eq!(v["v"], 1, "v stays 1: older Irori keeps syncing");
        let Wire::Hello(back) = serde_json::from_slice(&b).unwrap() else {
            panic!("not a hello")
        };
        assert_eq!(back, ours);
        let none = serde_json::to_value(Wire::Hello(BoardHello::default())).unwrap();
        assert!(none.get("dms").is_none(), "0 isn't sent");
    }

    /// Card #654: `calls` is built like `dms`. An older hello parses as 0,
    /// ours round-trips, 0 isn't sent, and `v` stays 1.
    #[test]
    fn hello_says_it_speaks_calls() {
        let old: Wire = serde_json::from_str(
            r#"{"type":"hello","v":1,"deviceId":"d","personId":"p","boardId":"b","boardEpoch":1,"coreVersion":"0.12.4","schema":1,"vector":{},"wallMs":0,"spaces":1,"dms":1}"#,
        )
        .unwrap();
        let Wire::Hello(h) = old else {
            panic!("not a hello")
        };
        assert_eq!(h.calls, 0);
        let ours = BoardHello {
            v: BOARD_PROTOCOL,
            calls: CALLS_PROTOCOL,
            ..Default::default()
        };
        let b = serde_json::to_vec(&Wire::Hello(ours.clone())).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(v["calls"], CALLS_PROTOCOL);
        assert_eq!(v["v"], 1, "v stays 1: older Irori keeps syncing");
        let Wire::Hello(back) = serde_json::from_slice(&b).unwrap() else {
            panic!("not a hello")
        };
        assert_eq!(back, ours);
        let none = serde_json::to_value(Wire::Hello(BoardHello::default())).unwrap();
        assert!(none.get("calls").is_none(), "0 isn't sent");
    }

    /// A `call` frame's shape: tagged `call`, camelCase, only `spaceId`,
    /// `callId` and `op` required, and an op this build doesn't know still
    /// parses.
    #[test]
    fn a_call_frame_is_tagged_and_keeps_any_op() {
        let f = CallFrame {
            space_id: "s".into(),
            call_id: "c".into(),
            op: "ring".into(),
            conv_id: "chan".into(),
            epoch: 2,
            devices: vec!["a1".into(), "b1".into()],
            at_ms: 7,
        };
        let b = serde_json::to_vec(&Wire::Call(f.clone())).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(v["type"], "call");
        assert_eq!(v["spaceId"], "s");
        assert_eq!(v["callId"], "c");
        assert_eq!(v["convId"], "chan");
        assert_eq!(v["atMs"], 7);
        let Wire::Call(back) = serde_json::from_slice(&b).unwrap() else {
            panic!("not a call")
        };
        assert_eq!(back, f);
        let Wire::Call(later) =
            serde_json::from_str(r#"{"type":"call","spaceId":"s","callId":"c","op":"mute"}"#)
                .unwrap()
        else {
            panic!("not a call")
        };
        assert_eq!(later.op, "mute");
        assert!(later.devices.is_empty());
        assert!(serde_json::from_slice::<Wire>(br#"{"type":"call","op":"ring"}"#).is_err());
    }

    /// A `dm` frame's shape, and what its reader does with the rest: an
    /// unknown type, or a `dm` that lacks a field, doesn't parse, and
    /// `Shared::read` skips what doesn't parse.
    #[test]
    fn a_dm_frame_is_tagged_and_keeps_its_body() {
        let f = DmFrame {
            board_id: "dm-space".into(),
            step: "pin".into(),
            body: serde_json::json!({"pin": {"personId": "p"}, "big": u64::MAX}),
        };
        let b = serde_json::to_vec(&Wire::Dm(f.clone())).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(v["type"], "dm");
        assert_eq!(v["boardId"], "dm-space");
        let Wire::Dm(back) = serde_json::from_slice(&b).unwrap() else {
            panic!("not a dm")
        };
        assert_eq!(back, f);
        let Wire::Dm(ask) =
            serde_json::from_str(r#"{"type":"dm","boardId":"b","step":"ask"}"#).unwrap()
        else {
            panic!("not a dm")
        };
        assert!(ask.body.is_null());
        let offer = DmFrame {
            body: serde_json::json!({"bundle": {"secret": "c2VjcmV0"}}),
            ..f
        };
        assert!(!format!("{offer:?}").contains("c2VjcmV0"), "{offer:?}");
        assert!(serde_json::from_slice::<Wire>(br#"{"type":"dm","step":"ask"}"#).is_err());
        assert!(serde_json::from_slice::<Wire>(br#"{"type":"dm.later","x":1}"#).is_err());
    }

    /// Card #579: an older hello parses as 0, ours says the level, and `v`
    /// stays 1 so older Irori keeps syncing.
    #[test]
    fn hello_says_it_stores_unknown_kinds() {
        let old: Wire = serde_json::from_str(
            r#"{"type":"hello","v":1,"deviceId":"d","personId":"p","boardId":"b","boardEpoch":1,"coreVersion":"0.12.4","schema":1,"vector":{},"wallMs":0,"spaces":1,"spawns":1}"#,
        )
        .unwrap();
        let Wire::Hello(h) = old else {
            panic!("not a hello")
        };
        assert_eq!(h.tolerant, 0);
        let ours = serde_json::to_value(Wire::Hello(BoardHello {
            v: BOARD_PROTOCOL,
            tolerant: TOLERANT_PROTOCOL,
            ..Default::default()
        }))
        .unwrap();
        assert_eq!(ours["tolerant"], TOLERANT_PROTOCOL);
        assert_eq!(ours["v"], 1);
        let none = serde_json::to_value(Wire::Hello(BoardHello::default())).unwrap();
        assert!(none.get("tolerant").is_none(), "0 isn't sent");
    }

    fn refused(dev: &str, seq: i64, unread: bool, past_cut: bool) -> RefusedRow {
        RefusedRow {
            device_id: dev.into(),
            seq,
            kind: "board.future".into(),
            reason: "no".into(),
            unread,
            past_cut,
        }
    }

    fn vec_of(pairs: &[(&str, i64)]) -> BTreeMap<String, i64> {
        pairs.iter().map(|(d, s)| (d.to_string(), *s)).collect()
    }

    /// Card #579: a hold is per peer, waits one tick then twice as long
    /// each time up to an hour, starts over on a new row, and ends once
    /// the row is held.
    #[test]
    fn holds_back_off_per_peer_and_end_when_the_row_lands() {
        let tick = Duration::from_millis(300);
        let t0 = std::time::Instant::now();
        let mut h = Holds::default();
        let v = vec_of(&[("w", 4)]);
        h.hold("p1", &[refused("w", 5, true, false)], tick, t0, 1_000);
        // Another peer still sends w's rows.
        assert!(h.skip("p2", &v, t0).is_empty());
        assert_eq!(h.skip("p1", &v, t0), ["w"]);
        assert!(h.skip("p1", &v, t0 + tick).is_empty(), "one tick, then try");
        // Each refusal of the same row waits twice as long, capped.
        let mut want = tick;
        for i in 0..20 {
            let now = t0 + Duration::from_secs(i * 10_000);
            h.hold("p1", &[refused("w", 5, true, false)], tick, now, 2_000);
            want = (want * 2).min(HOLD_MAX);
            let until = h.0[&("w".to_string(), "p1".to_string())].until;
            assert_eq!(until - now, want, "try {}", i + 2);
        }
        assert_eq!(want, HOLD_MAX);
        // The stall dates from the first refusal of the row.
        let all = |_: &str, _: i64| true;
        assert_eq!(h.stalled(&v, all)[0].since_ms, 1_000);
        // A new row starts over.
        h.hold("p1", &[refused("w", 6, false, false)], tick, t0, 3_000);
        let e = &h.0[&("w".to_string(), "p1".to_string())];
        assert_eq!((e.until - t0, e.since_ms), (tick, 3_000));
        // Holding the row ends the hold and the stall.
        let v = vec_of(&[("w", 6)]);
        assert!(h.stalled(&v, all).is_empty());
        assert!(h.skip("p1", &v, t0).is_empty());
        assert!(h.0.is_empty(), "the hold is gone");
    }

    /// Card #579: what the stalled device says. One line per writer across
    /// peers, unread if any peer's refusal was, and nothing for a row past
    /// a revoke's cut or a row no peer says the writer wrote.
    #[test]
    fn stalled_says_only_what_is_backed() {
        let (tick, t0) = (Duration::from_millis(300), std::time::Instant::now());
        let mut h = Holds::default();
        h.hold("p1", &[refused("w", 5, false, false)], tick, t0, 2_000);
        h.hold("p2", &[refused("w", 5, true, false)], tick, t0, 1_000);
        h.hold("p1", &[refused("cut", 9, false, true)], tick, t0, 1_000);
        let v = vec_of(&[("w", 4), ("cut", 8)]);
        let got = h.stalled(&v, |_, _| true);
        assert_eq!(
            got,
            [SelfStall {
                device_id: "w".into(),
                seq: 5,
                since_ms: 1_000,
                unread: true,
            }]
        );
        // Junk past the stream's end, or cut off since: nobody has row 5.
        assert!(h.stalled(&v, |_, seq| seq < 5).is_empty());
    }
}
