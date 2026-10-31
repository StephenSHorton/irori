//! Shared boards, steps 1-3 (I:internal/core/boards.go; design
//! v0.5-shared-boards.md §2, §12): each board is its own log,
//! `boards/<boardId>.db`, opened by the store in board mode with this
//! device's Signer and a board Verifier, and folded by its own coord. The
//! Core holds them all and routes a taskId to the one log it lives in
//! ([`Services::route`]); tools and the window go through that.
//!
//! Each board's Verifier is [`BoardVerifier`] (§3): one chain per pinned
//! member, seeded from the member's pin and grown by the `member.*` rows
//! each member's devices write ([`Services::keep_current`] writes this
//! person's). The device-local `board.pin` row is the board's anchor: its
//! creator as this device trusts it, which `board.created` must match.
//! Every other member is pinned by the membership fold from owners'
//! `board.member.added` rows, which also cuts removed and departed members.
//!
//! Each board's secret (§6.1) sits beside its log, `boards/<id>.secrets`,
//! sealed by the keystore and never in a row. [`Services::rotate_secret`]
//! mints the next epoch's when a removal, a leave or a member device
//! revoke folds.
//!
//! Step 4 (one person, several boards) is [`super::boards_own`].

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use coord::roster::{AgentCard, Agents};
use coord::{Change, Coord, ErrorKind as CK, Notice};
use gocompat::sub::Queue;
use irori_identity::keystore::Keystore;
use irori_identity::membership::{BoardPerson, Conflict};
use irori_identity::{
    BoardDevice, BoardSecrets, BoardVerifier, Identity, MemberKeyRotated, MemberPin,
    USER_KEY_PREFIX,
};
use serde::{Deserialize, Serialize};
use store::kinds::{self, AGENT_ACTOR_PREFIX, PERSON_ACTOR_PREFIX};
use store::{Event, Filter, Store};
use sys::cancel::Cancel;

use super::Services;
use crate::timers::Ticker;
use crate::{CLOSE_WAIT, logf};

pub use super::boards_own::{BoardSummary, SharePreview, SharedCopy};

/// Where the board logs live, under the data dir.
pub const BOARDS_DIR: &str = "boards";

const MAX_LIST: i64 = 1000;

/// Why a board action failed (Go's `ErrNoBoard`, `ErrShared`,
/// `ErrPreviewStale`, or a coord or store error), with Go's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No board log with that id on this device.
    NoBoard(String),
    /// A write to a private card that was shared to a board (§5.2).
    Shared(String),
    /// The card changed between its share preview and the confirm.
    PreviewStale(String),
    Coord(coord::Error),
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn no_board(id: &str) -> Error {
        Error::NoBoard(format!("core: no such board on this device: {id}"))
    }
    pub(crate) fn coord(kind: CK, detail: impl std::fmt::Display) -> Error {
        Error::Coord(coord::Error::new(kind, detail))
    }
    pub(crate) fn other(e: impl std::fmt::Display) -> Error {
        Error::Other(e.to_string())
    }

    /// The error as coord's, for the tools' router: kinds that coord has no
    /// sentinel for keep their text.
    pub fn into_coord(self) -> coord::Error {
        match self {
            Error::Coord(e) => e,
            Error::NoBoard(m) => coord::Error::raw(CK::NotFound, m),
            Error::Shared(m) => coord::Error::raw(CK::NotAllowed, m),
            Error::PreviewStale(m) => coord::Error::raw(CK::BadState, m),
            Error::Other(m) => coord::Error::raw(CK::Store, m),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Coord(e) => e.fmt(f),
            Error::NoBoard(m) | Error::Shared(m) | Error::PreviewStale(m) | Error::Other(m) => {
                f.write_str(m)
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<coord::Error> for Error {
    fn from(e: coord::Error) -> Error {
        Error::Coord(e)
    }
}

impl From<store::Error> for Error {
    fn from(e: store::Error) -> Error {
        Error::Other(e.to_string())
    }
}

impl From<irori_identity::Error> for Error {
    fn from(e: irori_identity::Error) -> Error {
        Error::Other(e.to_string())
    }
}

/// Go `strings.TrimSpace`.
pub(crate) fn trim(s: &str) -> &str {
    s.trim_matches(gocompat::text::is_space)
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A fresh boardId: 16 random bytes, hex (§2.1). It names nothing.
pub fn new_board_id() -> Result<String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(Error::other)?;
    Ok(hex::encode(b))
}

/// A boardId as [`new_board_id`] makes them, so a file name can't leave the
/// boards dir.
pub fn valid_board_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// A folded `agent.card` row.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct BoardCard {
    pub card_id: String,
    pub name: String,
    pub kind: String,
    pub person_id: String,
    pub status: String,
}

/// A `board_cards` row: which session a board's agent card is (§2.4). It is
/// a private row replicated to the person's own devices
/// (`board.cardof`, with boardId), so every one of them translates a card
/// both ways. Rows from before it are device-local in the board store
/// (`board.cardkey`, no boardId) and are still read.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct CardKeyRow {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub board_id: String,
    pub card_id: String,
    pub agent_key: String,
}

/// A board's cards and the person's board_cards for it.
#[derive(Default)]
pub(crate) struct CardMaps {
    /// cardId -> its agent.card, as folded.
    pub cards: HashMap<String, BoardCard>,
    /// agentKey -> cardId: the lowest card of the agent.
    pub keys: HashMap<String, String>,
    /// cardId -> agentKey.
    pub by_id: HashMap<String, String>,
}

impl CardMaps {
    /// Records that board card `card_id` is the session `agent`. Two of the
    /// person's devices can mint a card for the same agent while apart:
    /// both cards map back to it, and the lower cardId is the one written
    /// from here on, so every device settles on the same card.
    fn add_key(&mut self, card_id: &str, agent: &str) {
        self.by_id.insert(card_id.into(), agent.into());
        let cur = self.keys.entry(agent.into()).or_default();
        if cur.is_empty() || card_id < cur.as_str() {
            *cur = card_id.into();
        }
    }
}

/// One open board.
pub(crate) struct BoardLog {
    pub id: String,
    pub store: Arc<Store>,
    pub coord: OnceLock<Arc<Coord>>,
    pub verifier: Arc<BoardVerifier>,
    pub secrets: Arc<BoardSecrets>,
    /// One keep_current or rotate_secret at a time.
    pub id_mu: Mutex<()>,
    /// Ends the board's threads, so one board can be forgotten while the
    /// Core runs.
    stop: Cancel,
    threads: Mutex<Vec<JoinHandle<()>>>,
    pub maps: Mutex<CardMaps>,
    /// When this device opened it, for the status lines' grace.
    pub opened: Instant,
}

impl BoardLog {
    pub fn coord(&self) -> &Arc<Coord> {
        self.coord
            .get()
            .expect("a board's coord is set before it is listed")
    }

    fn fold_key(m: &mut CardMaps, board_id: &str, ev: &Event) {
        let Ok(r) = serde_json::from_slice::<CardKeyRow>(&ev.payload) else {
            return;
        };
        if r.card_id.is_empty() || r.agent_key.is_empty() {
            return;
        }
        if ev.kind == kinds::BOARD_CARD_OF && r.board_id != board_id {
            return;
        }
        m.add_key(&r.card_id, &r.agent_key);
    }

    fn fold_card(&self, ev: &Event) {
        let Ok(p) = serde_json::from_slice::<BoardCard>(&ev.payload) else {
            return;
        };
        if p.card_id.is_empty() {
            return;
        }
        lock(&self.maps).cards.insert(p.card_id.clone(), p);
    }

    /// Folds the log's agent cards and the person's board_cards for it: the
    /// private rows (`priv_st`) and this device's older local ones.
    pub(crate) fn load(&self, priv_st: &Store) -> Result<()> {
        each_row(
            &self.store,
            Filter {
                kinds: vec![kinds::AGENT_CARD.into()],
                ..Default::default()
            },
            |ev| self.fold_card(ev),
        )?;
        let mut keys = Vec::new();
        each_row(
            &self.store,
            Filter {
                kinds: vec![kinds::BOARD_CARD_KEY.into()],
                local: true,
                ..Default::default()
            },
            |ev| keys.push(ev.clone()),
        )?;
        each_row(
            priv_st,
            Filter {
                kinds: vec![kinds::BOARD_CARD_OF.into()],
                ..Default::default()
            },
            |ev| keys.push(ev.clone()),
        )?;
        let mut m = lock(&self.maps);
        for ev in &keys {
            BoardLog::fold_key(&mut m, &self.id, ev);
        }
        Ok(())
    }

    /// Every task.created row's taskId in the log.
    fn task_ids(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        each_row(
            &self.store,
            Filter {
                kinds: vec![kinds::TASK_CREATED.into()],
                ..Default::default()
            },
            |ev| {
                let id = created_task_id(ev);
                if !id.is_empty() {
                    out.push(id);
                }
            },
        )?;
        Ok(out)
    }

    /// Whether `dev` already wrote a member.key.rotated to `epoch` here.
    fn wrote_rotation(&self, dev: &str, epoch: i64) -> Result<bool> {
        let mut found = false;
        each_row(
            &self.store,
            Filter {
                kinds: vec![kinds::MEMBER_KEY_ROTATED.into()],
                device_id: dev.into(),
                ..Default::default()
            },
            |ev| {
                if irori_identity::decode::<MemberKeyRotated>(&ev.payload)
                    .is_some_and(|r| r.person_epoch == epoch)
                {
                    found = true;
                }
            },
        )?;
        Ok(found)
    }

    /// Ends the board's threads and waits for them, at most [`CLOSE_WAIT`].
    pub(crate) fn stop_threads(&self) {
        self.stop.cancel();
        let threads = std::mem::take(&mut *lock(&self.threads));
        let deadline = Instant::now() + CLOSE_WAIT;
        for h in threads {
            while !h.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if h.is_finished() {
                let _ = h.join();
            }
        }
    }

    fn spawn(&self, name: &str, f: impl FnOnce() + Send + 'static) {
        let h = std::thread::Builder::new()
            .name(format!("irori-board-{name}"))
            .spawn(f)
            .expect("spawn a board thread");
        lock(&self.threads).push(h);
    }
}

/// Calls `f` for every row `f` matches, in log order.
pub(crate) fn each_row(st: &Store, mut f: Filter, mut fun: impl FnMut(&Event)) -> Result<()> {
    f.limit = MAX_LIST;
    loop {
        let evs = st.list(&f)?;
        for ev in &evs {
            fun(ev);
        }
        if (evs.len() as i64) < f.limit {
            return Ok(());
        }
        f.after = evs[evs.len() - 1].cursor();
    }
}

pub(crate) fn created_task_id(ev: &Event) -> String {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct P {
        #[serde(rename = "taskId")]
        task_id: String,
    }
    serde_json::from_slice::<P>(&ev.payload)
        .unwrap_or_default()
        .task_id
}

/// The Core's set of board logs, the taskId index and step 4's state. It
/// is made before the mesh; each open board's connections run on its own
/// `BoardMesh` (`Core::open_board_net`).
#[derive(Default)]
pub(crate) struct Boards {
    pub(crate) inner: Mutex<BoardsInner>,
    /// One reconcile of board.joined and board.forgot at a time.
    pub(crate) rec: Mutex<()>,
    subs: Mutex<Vec<Weak<Queue<Change>>>>,
    follow: OnceLock<()>,
    /// Kicks every open board's `BoardMesh` when a board opens or a row
    /// lands, so its member devices pull now.
    pub(crate) kicker: OnceLock<Arc<dyn Fn() + Send + Sync>>,
    /// The Core's stop; every board's threads end with it.
    pub(crate) stop: OnceLock<Cancel>,
    /// The thread keeping every board's member.* rows current.
    pub(crate) identity: Mutex<Vec<JoinHandle<()>>>,
    /// (board, maker) pairs one of this person's own devices vouched for by
    /// sending that maker's secret on board.secret: they count as makers
    /// before the board's rows certify them here.
    pub(crate) vouched: Arc<Mutex<std::collections::HashSet<(String, String)>>>,
}

#[derive(Default)]
pub(crate) struct BoardsInner {
    pub logs: BTreeMap<String, Arc<BoardLog>>,
    /// taskId -> boardId, from each log's task.created rows.
    pub task_of: HashMap<String, String>,
    /// This device's shared copies (§5.2) by private task id.
    pub copies: HashMap<String, super::boards_own::SharedCopyRow>,
}

impl Boards {
    pub(crate) fn new(stop: Cancel) -> Boards {
        let b = Boards::default();
        let _ = b.stop.set(stop);
        b
    }

    pub(crate) fn kick(&self) {
        if let Some(k) = self.kicker.get() {
            k();
        }
    }

    /// Whether a board open here has a member other than this person (B1:
    /// the home server runs for it).
    pub(crate) fn shared_with_others(&self) -> bool {
        self.list().iter().any(|b| {
            let m = b.verifier.membership();
            m.members.iter().filter(|p| m.active(&p.person_id)).count() > 1
        })
    }

    /// The open board logs, sorted by id.
    pub(crate) fn list(&self) -> Vec<Arc<BoardLog>> {
        lock(&self.inner).logs.values().cloned().collect()
    }

    pub(crate) fn get(&self, id: &str) -> Result<Arc<BoardLog>> {
        lock(&self.inner)
            .logs
            .get(id)
            .cloned()
            .ok_or_else(|| Error::no_board(id))
    }

    pub(crate) fn subscribe(&self) -> Arc<Queue<Change>> {
        let q = Arc::new(Queue::new(64));
        let mut s = lock(&self.subs);
        s.retain(|w| w.strong_count() > 0);
        s.push(Arc::downgrade(&q));
        q
    }

    /// Hands every subscriber `ch`; a slow reader redraws on the next one.
    pub(crate) fn tell(&self, ch: Change) {
        let mut s = lock(&self.subs);
        s.retain(|w| match w.upgrade() {
            Some(q) if !q.is_closed() => {
                q.push(ch.clone());
                true
            }
            _ => false,
        });
    }

    /// Tells the window a board opened, went or was renamed.
    pub(crate) fn moved(&self) {
        self.tell(Change::default());
    }

    /// Closes every board log once the Core stops (Go `closeBoards`).
    pub(crate) fn close_all(&self) {
        for h in std::mem::take(&mut *lock(&self.identity)) {
            let deadline = Instant::now() + CLOSE_WAIT;
            while !h.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if h.is_finished() {
                let _ = h.join();
            }
        }
        let logs: Vec<_> = std::mem::take(&mut lock(&self.inner).logs)
            .into_values()
            .collect();
        for b in logs {
            b.stop_threads();
            let _ = b.store.close();
        }
    }
}

/// The identity's keystore for a board's secrets file.
struct IdentKeystore(Arc<Identity>);

impl Keystore for IdentKeystore {
    fn name(&self) -> &'static str {
        self.0.keystore().name()
    }
    fn seal(&self, plain: &[u8]) -> std::result::Result<Vec<u8>, irori_identity::Error> {
        self.0.keystore().seal(plain)
    }
    fn unseal(
        &self,
        blob: &[u8],
    ) -> std::result::Result<zeroize::Zeroizing<Vec<u8>>, irori_identity::Error> {
        self.0.keystore().unseal(blob)
    }
}

/// One board as this device folds it, for the Boards dialog and
/// `?detail=1`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardInfo {
    pub board_id: String,
    pub name: String,
    pub tag: String,
    pub creator: String,
    pub members: Vec<BoardPerson>,
    /// board.* rows the fold ignored, with why (§4.2).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<Conflict>,
    /// The board secret's epoch by the fold.
    pub epoch: i64,
    /// The newest epoch this device holds a secret for.
    pub secret_epoch: i64,
}

/// What a board's Verifier holds about one member, for tests and
/// `?detail=1`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardIdentity {
    pub person_id: String,
    pub epoch: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub devices: Vec<BoardDevice>,
}

/// The roster a board's coord reads liveness from: each of the log's agent
/// cards, keyed `agent:<cardId>`. A card whose session is on this device is
/// as live as the session (missing when the session is). Only that device
/// says a session went (v0.4 §6.0), and a board key names no device, so a
/// card of another device's session, or of nobody's this device knows,
/// never reads as gone here: it shows its session's status while that is
/// live, else its card row's.
struct BoardRoster {
    s: Weak<Services>,
    b: Weak<BoardLog>,
}

impl Agents for BoardRoster {
    fn agents(&self) -> Vec<AgentCard> {
        let (Some(s), Some(b)) = (self.s.upgrade(), self.b.upgrade()) else {
            return Vec::new();
        };
        roster_of(&s, &b)
    }
}

/// [`BoardRoster`]'s agents for board `b`.
fn roster_of(s: &Services, b: &BoardLog) -> Vec<AgentCard> {
    {
        let live: HashMap<String, AgentCard> = s
            .roster
            .agents()
            .into_iter()
            .map(|a| (a.key.clone(), a))
            .collect();
        let me = s.device.id.as_str();
        let m = lock(&b.maps);
        let mut out = Vec::with_capacity(m.cards.len());
        for (id, card) in &m.cards {
            let mut a = AgentCard {
                key: format!("{AGENT_ACTOR_PREFIX}{id}"),
                name: card.name.clone(),
                agent: card.kind.clone(),
                status: card.status.clone(),
                ..Default::default()
            };
            if let Some(agent) = m.by_id.get(id) {
                let mine = irori_identity::owner::device_of(agent) == me;
                match live.get(agent) {
                    None if mine => continue,
                    Some(l) if mine || l.status != "gone" => {
                        a.status = l.status.clone();
                        a.updated_at = l.updated_at;
                    }
                    _ => {}
                }
            }
            out.push(a);
        }
        out
    }
}

impl Services {
    fn boards_dir(&self) -> PathBuf {
        self.data_dir.join(BOARDS_DIR)
    }

    pub(crate) fn person_id(&self) -> String {
        self.chain.person_id()
    }

    pub(crate) fn person_actor(&self) -> String {
        format!("{PERSON_ACTOR_PREFIX}{}", self.person_id())
    }

    pub(crate) fn board(&self, id: &str) -> Result<Arc<BoardLog>> {
        self.boards.get(id)
    }

    /// Opens every board log in the data dir. A board that fails to open
    /// is logged and left out: the private board keeps working.
    pub(crate) fn open_boards(self: &Arc<Self>) {
        let ents = match std::fs::read_dir(self.boards_dir()) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                logf(format_args!("boards: {e}"));
                return;
            }
        };
        let mut ids: Vec<String> = ents
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| !t.is_dir()))
            .filter_map(|e| {
                e.file_name()
                    .to_str()?
                    .strip_suffix(".db")
                    .map(str::to_string)
            })
            .filter(|id| valid_board_id(id))
            .collect();
        ids.sort();
        for id in ids {
            if let Err(e) = self.open_board(&id, false, None) {
                logf(format_args!("board {id}: {e}"));
            }
        }
    }

    /// Makes a new board log on this device and returns its id (§2.1). It
    /// anchors this person with this device as the board root, writes this
    /// person's member.* rows (§3) and then board.created, signed by the
    /// person key: the self-cert comes first in this device's stream, so a
    /// peer can check board.created's device signature against it. The
    /// board's first secret is made with it (§6.1). `tag` is its short
    /// handle in refs (`mlg#12`, B3), unique among the person's boards.
    pub(crate) fn create_board(self: &Arc<Self>, name: &str, tag: &str) -> Result<String> {
        let name = trim(name);
        if !irori_identity::membership::valid_board_name(name) {
            return Err(Error::coord(
                CK::Invalid,
                format_args!(
                    "a board name is 1 to {} characters",
                    irori_identity::membership::MAX_BOARD_NAME
                ),
            ));
        }
        if !irori_identity::membership::valid_board_tag(tag) {
            return Err(Error::coord(
                CK::Invalid,
                "a board tag is a lowercase letter, then up to 15 lowercase letters, digits or '-'",
            ));
        }
        let other = self.tag_holder(tag);
        if !other.is_empty() {
            return Err(Error::coord(
                CK::Invalid,
                format_args!(
                    "the tag {} is already board {other}'s",
                    gocompat::quote::quote(tag)
                ),
            ));
        }
        let id = new_board_id()?;
        let b = self.open_board(&id, true, None)?;
        // Until board.joined is written, a failure takes the board's files
        // with it: a half-made board would open again at the next start,
        // nameless and never ready.
        let fail = |e: Error| {
            if let Err(re) = self.remove_board(&id) {
                logf(format_args!("board {id}: clean up a failed creation: {re}"));
            }
            Err(e)
        };
        let pin = b.verifier.pinned(&self.person_id()).unwrap_or_default();
        let row = match self
            .ident
            .board_created_row(&id, name, tag, &pin, (self.now)())
        {
            Ok(r) => r,
            Err(e) => return fail(e.into()),
        };
        if let Err(e) = b.store.append(
            &format!("{PERSON_ACTOR_PREFIX}{}", pin.person_id),
            kinds::BOARD_CREATED,
            &row,
        ) {
            return fail(e.into());
        }
        if !b.verifier.membership().created {
            return fail(Error::other(
                "core: the board's board.created did not count",
            ));
        }
        // The person's other devices open it from this row (§4.1 step 7,
        // own devices only until invites): it carries the creator's pin,
        // which they trust because one of their own devices wrote it.
        let joined = super::boards_own::BoardJoinedRow {
            board_id: id.clone(),
            name: name.into(),
            tag: tag.into(),
            creator: pin,
            at: (self.now)(),
        };
        if let Err(e) = self.store.append("irori", kinds::BOARD_JOINED, &joined) {
            return fail(e.into());
        }
        // The board is made; a secret that fails here is made again when
        // the board next opens or a row moves its epoch.
        if let Err(e) = self.rotate_secret(&b) {
            logf(format_args!("board {id}: secret: {e}"));
        }
        self.boards.moved();
        Ok(id)
    }

    /// The board that already has `tag` among the person's boards: one
    /// open here, or one any of the person's devices made or joined
    /// (board.joined) that hasn't opened or folded here yet. "" when none.
    pub(crate) fn tag_holder(&self, tag: &str) -> String {
        for b in self.boards.list() {
            if b.verifier.membership().tag == tag {
                return b.id.clone();
            }
        }
        let mut joined: BTreeMap<String, bool> = BTreeMap::new();
        let _ = each_row(
            &self.store,
            Filter {
                kinds: vec![kinds::BOARD_JOINED.into(), kinds::BOARD_FORGOT.into()],
                ..Default::default()
            },
            |ev| {
                if ev.kind == kinds::BOARD_JOINED {
                    if let Some(r) = super::boards_own::BoardJoinedRow::decode(&ev.payload)
                        && r.tag == tag
                    {
                        joined.insert(r.board_id, true);
                    }
                } else if let Some(r) = super::boards_own::BoardForgotRow::decode(&ev.payload)
                    && let Some(live) = joined.get_mut(&r.board_id)
                {
                    *live = false;
                }
            },
        );
        joined
            .into_iter()
            .find(|(_, live)| *live)
            .map(|(id, _)| id)
            .unwrap_or_default()
    }

    /// Renames a board (§4.2: owners only), with board.renamed signed by
    /// this person's key.
    pub(crate) fn rename_board(&self, board_id: &str, name: &str) -> Result<()> {
        let name = trim(name);
        if !irori_identity::membership::valid_board_name(name) {
            return Err(Error::coord(
                CK::Invalid,
                format_args!(
                    "a board name is 1 to {} characters",
                    irori_identity::membership::MAX_BOARD_NAME
                ),
            ));
        }
        let b = self.board(board_id)?;
        let me = self.person_id();
        if !b.verifier.membership().owner(&me) {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!("only an owner renames board {board_id}"),
            ));
        }
        let epoch = b.verifier.epoch(&me);
        if epoch != self.ident.current().epoch {
            return Err(Error::coord(
                CK::NotAllowed,
                "this device's person key isn't the board's newest yet",
            ));
        }
        let row = self.ident.board_renamed_row(epoch, name, (self.now)())?;
        b.store.append(
            &format!("{PERSON_ACTOR_PREFIX}{me}"),
            kinds::BOARD_RENAMED,
            &row,
        )?;
        self.boards.moved();
        Ok(())
    }

    /// `board_id`'s membership and secret epoch.
    pub(crate) fn board_info(&self, board_id: &str) -> Result<BoardInfo> {
        let b = self.board(board_id)?;
        let m = b.verifier.membership();
        Ok(BoardInfo {
            board_id: board_id.into(),
            name: m.name,
            tag: m.tag,
            creator: m.creator,
            members: m.members,
            conflicts: m.conflicts,
            epoch: b.verifier.board_epoch(),
            secret_epoch: b.secrets.epoch(),
        })
    }

    /// Makes this device's secret for the board's epoch when it holds none
    /// yet (§6.1): at board.created, and after a removal, a leave or a
    /// member device revoke folds. Only a device of a current owner, or of
    /// a member whose own device was revoked, rotates. Sending it to the
    /// other member devices is step 5's.
    pub(crate) fn rotate_secret(&self, b: &BoardLog) -> Result<()> {
        let _g = lock(&b.id_mu);
        let target = b.verifier.board_epoch();
        if target == 0 || b.secrets.epoch() >= target {
            return Ok(());
        }
        if b.verifier.cut(&self.device.id).is_some() {
            return Ok(()); // the device a rotation shuts out makes no secret for it
        }
        let me = self.person_id();
        if target == 1
            && b.verifier
                .pinned(&me)
                .is_none_or(|p| p.root_device_id != self.device.id)
        {
            // The board's first secret is its creating device's (§6.1);
            // the person's other devices get it in a board.secret frame
            // (step 5) rather than each making a different one.
            return Ok(());
        }
        let m = b.verifier.membership();
        let may =
            m.owner(&me) || (m.active(&me) && b.verifier.devices(&me).iter().any(|d| d.revoked));
        if !may {
            return Ok(());
        }
        b.secrets.mint(target, &self.device.id)?;
        // The board mesh joins the new epoch's room now, hands the secret
        // on, and settles once every member moved (card #67), rather than
        // on its next presence tick.
        self.recheck_board_mesh(&b.id);
        self.boards.kick(); // the person's other devices get it
        Ok(())
    }

    /// Opens (creating) one board log and its coord, and starts following
    /// it. `create` pins this person on it with this device as the board
    /// root; `pin` is the creator's, from a board.joined row one of this
    /// person's devices wrote (or, in stage 2, an invite bundle).
    pub(crate) fn open_board(
        self: &Arc<Self>,
        id: &str,
        create: bool,
        pin: Option<&MemberPin>,
    ) -> Result<Arc<BoardLog>> {
        if !valid_board_id(id) {
            return Err(Error::other(format!(
                "core: {} is not a board id",
                gocompat::quote::quote(id)
            )));
        }
        // One open at a time, so two callers can't open one file twice.
        let mut inner = lock(&self.boards.inner);
        if let Some(b) = inner.logs.get(id) {
            return Ok(b.clone());
        }
        let dir = self.boards_dir();
        let secrets = Arc::new(
            BoardSecrets::open(
                &dir.join(format!("{id}.secrets")),
                id,
                Box::new(IdentKeystore(self.ident.clone())),
            )
            .map_err(|e| Error::other(format!("board secrets: {e}")))?,
        );
        let v = Arc::new(BoardVerifier::new_for(id, &[]));
        let now = self.now.clone();
        let wall = move || now().to_system();
        let st = Arc::new(
            Store::open_with(
                dir.join(format!("{id}.db")),
                &self.device.id,
                Arc::new(store::Clock::with_now(Box::new(wall.clone()))),
                store::OpenOptions {
                    signer: Some(self.ident.clone()),
                    verifier: Some(v.clone()),
                    now: Some(Arc::new(wall) as store::WallFn),
                    board: true,
                    ..Default::default()
                },
            )
            .map_err(|e| Error::other(format!("open board log: {e}")))?,
        );
        {
            let v = v.clone();
            // A maker counts once the board certifies it, or, before the
            // board's rows arrive, when it is one of this person's own
            // devices or one of them vouched for it (board.secret on the
            // private mesh).
            let (chain, vouched, board) = (
                self.chain.clone(),
                self.boards.vouched.clone(),
                id.to_string(),
            );
            secrets.set_makers(Some(Box::new(move |dev: &str| {
                v.cut(dev).is_none()
                    && (v.cert(dev).is_some()
                        || chain
                            .devices()
                            .iter()
                            .any(|d| d.cert.device_id == dev && !d.revoked)
                        || lock(&vouched).contains(&(board.clone(), dev.to_string())))
            })));
        }
        let stop = self
            .boards
            .stop
            .get()
            .map(Cancel::child)
            .unwrap_or_default();
        let b = Arc::new(BoardLog {
            id: id.into(),
            store: st.clone(),
            coord: OnceLock::new(),
            verifier: v.clone(),
            secrets,
            id_mu: Mutex::new(()),
            stop,
            threads: Mutex::new(Vec::new()),
            maps: Mutex::new(CardMaps::default()),
            opened: Instant::now(),
        });
        let close = |e: Error| {
            let _ = st.close();
            Err(e)
        };
        if let Err(e) = self.pin_board(&b, create, pin) {
            return close(e);
        }
        if let Err(e) = b.load(&self.store) {
            return close(e);
        }
        if let Err(e) = self.keep_current(&b) {
            // The board still opens: its rows verify as far as they can,
            // and the next identity change retries.
            logf(format_args!("board {id}: identity rows: {e}"));
        }
        if let Err(e) = self.rotate_secret(&b) {
            logf(format_args!("board {id}: secret: {e}"));
        }
        let me = self.person_actor();
        let co = {
            let (ch, v1, v2, v3, v4, v5) = (
                self.chain.clone(),
                v.clone(),
                v.clone(),
                v.clone(),
                v.clone(),
                v.clone(),
            );
            let person_key = me.clone();
            let self_dev = self.device.id.clone();
            Coord::new(
                st.clone(),
                Some(Arc::new(BoardRoster {
                    s: Arc::downgrade(self),
                    b: Arc::downgrade(&b),
                })),
                coord::Options {
                    now: Some(self.now.clone()),
                    board: true,
                    person: me.clone(),
                    is_owner: Some(Arc::new(move |k: &str| k == person_key)),
                    revoked: Some(Arc::new(move |dev: &str| v1.cut(dev).is_some())),
                    key_person: Some(Arc::new(move |k: &str| v2.key_person(k))),
                    device_person: Some(Arc::new(move |dev: &str| {
                        v3.person_of(dev).unwrap_or_else(|| {
                            if dev == self_dev {
                                ch.person_id()
                            } else {
                                String::new()
                            }
                        })
                    })),
                    tag: Some(Arc::new(move || v4.membership().tag)),
                    // The cancel rule reads the owners at the row's place
                    // (v0.5 §5.3); no place means now.
                    board_owner: Some(Arc::new(move |p: &str, at: Option<&coord::RowPos>| {
                        let m = v5.membership();
                        match at {
                            None => m.owner(p),
                            Some(at) => m.owner_at(
                                p,
                                &irori_identity::membership::RowAt {
                                    hlc: at.hlc,
                                    device: at.dev.clone(),
                                    seq: at.seq,
                                },
                            ),
                        }
                    })),
                    // Under the board coord's lock: takes the inbox's, which
                    // never waits on a board coord (its stale hook reads
                    // only the private one).
                    hop_of: Some({
                        let s = Arc::downgrade(self);
                        Arc::new(move |actor: &str| s.upgrade().map_or(0, |s| s.board_hop(actor)))
                    }),
                    // A file on a board row this device holds is offered in
                    // the board's log first (§5.5).
                    holds_file: Some({
                        let s = Arc::downgrade(self);
                        Arc::new(move |sha: &str| s.upgrade().is_some_and(|s| s.files.has(sha)))
                    }),
                    ..Default::default()
                },
            )
        };
        let co = match co {
            Ok(c) => Arc::new(c),
            Err(e) => return close(Error::other(format!("board coord: {e}"))),
        };
        let _ = b.coord.set(co);
        let tasks = match b.task_ids() {
            Ok(t) => t,
            Err(e) => return close(e),
        };
        for t in tasks {
            inner.task_of.insert(t, id.into());
        }
        inner.logs.insert(id.into(), b.clone());
        drop(inner);
        self.start_board(&b);
        self.boards
            .follow
            .get_or_init(|| self.start_follow_identity());
        self.open_board_net(&b);
        self.boards.kick();
        Ok(b)
    }

    /// Opens the board on the network (boards stage 2): its member devices,
    /// this person's other devices among them, are dialled on
    /// `irori/board/1` and its rows move only there.
    fn open_board_net(&self, b: &BoardLog) {
        let parts = crate::BoardParts {
            board_id: b.id.clone(),
            person_id: self.person_id(),
            log: b.store.clone(),
            verifier: b.verifier.clone(),
            secrets: b.secrets.clone(),
        };
        let mesh = self.mesh.clone();
        self.pairing.handle.spawn(async move {
            let id = parts.board_id.clone();
            if let Err(e) = mesh.open_board(parts).await {
                logf(format_args!("board {id}: open on the network: {e}"));
            }
        });
    }

    /// The board's threads: coord's Follow, the taskId and card fold, and
    /// its changes to [`Boards::subscribe`].
    fn start_board(self: &Arc<Self>, b: &Arc<BoardLog>) {
        {
            let sub = Arc::new(b.store.subscribe());
            let reg = {
                let s = sub.clone();
                b.stop.on_cancel(move || s.cancel())
            };
            let co = b.coord().clone();
            let id = b.id.clone();
            b.spawn("coord", move || {
                let _reg = reg;
                if let Err(e) = co.catch_up() {
                    logf(format_args!("board {id}: coord: {e}"));
                }
                let mut t = Ticker::new(Duration::from_secs(1));
                loop {
                    match sub.recv_timeout(t.until()) {
                        Ok(None) => return,
                        Ok(Some(ev)) => {
                            if let Err(e) = co.handle(&ev) {
                                logf(format_args!("board {id}: coord: {e}"));
                            }
                        }
                        Err(()) => {}
                    }
                    if t.take() {
                        co.tick();
                    }
                }
            });
        }
        {
            let sub = Arc::new(b.store.subscribe());
            let reg = {
                let s = sub.clone();
                b.stop.on_cancel(move || s.cancel())
            };
            let (s, wb) = (Arc::downgrade(self), Arc::downgrade(b));
            b.spawn("follow", move || {
                let _reg = reg;
                while let Some(ev) = sub.recv() {
                    let (Some(s), Some(b)) = (s.upgrade(), wb.upgrade()) else {
                        return;
                    };
                    s.follow_board_row(&b, &ev);
                }
            });
        }
        {
            let q = b.coord().subscribe();
            let reg = {
                let q = q.clone();
                b.stop.on_cancel(move || q.close())
            };
            let s = Arc::downgrade(self);
            b.spawn("changes", move || {
                let _reg = reg;
                while let Some(ch) = q.recv() {
                    match s.upgrade() {
                        Some(s) => s.boards.tell(ch),
                        None => return,
                    }
                }
            });
        }
    }

    /// One row of `b`'s log (Go `followBoard`): keeps the taskId index and
    /// the cards current, rebuilding them on a resync, and rotates the
    /// board secret when a membership or identity row moves the epoch.
    fn follow_board_row(&self, b: &BoardLog, ev: &Event) {
        self.boards.kick(); // the board's member devices pull it now
        if (ev.seq == 0 || kinds::is_board_identity_kind(&ev.kind))
            && let Err(e) = self.rotate_secret(b)
        {
            logf(format_args!("board {}: secret: {e}", b.id));
        }
        if ev.kind == kinds::BOARD_MEMBER_REMOVED || ev.kind == kinds::BOARD_MEMBER_LEFT {
            // Someone is gone (§4.3, §4.4): hold what they sent that
            // hasn't reached an agent, and close their connections.
            self.hold_from_gone(b);
            self.recheck_board_mesh(&b.id);
        }
        if ev.seq == 0 {
            if let Ok(ids) = b.task_ids() {
                let mut inner = lock(&self.boards.inner);
                for id in ids {
                    inner.task_of.insert(id, b.id.clone());
                }
            }
            let _ = b.load(&self.store);
        } else if ev.kind == kinds::TASK_CREATED {
            let id = created_task_id(ev);
            if !id.is_empty() {
                lock(&self.boards.inner).task_of.insert(id, b.id.clone());
            }
        } else if ev.kind == kinds::AGENT_CARD {
            b.fold_card(ev);
        }
        self.bridge_row(b, ev);
    }

    /// Folds a private board_cards row that reached this device into its
    /// board, when that board is open here.
    pub(crate) fn fold_card_key(&self, ev: &Event) {
        let Ok(r) = serde_json::from_slice::<CardKeyRow>(&ev.payload) else {
            return;
        };
        if let Ok(b) = self.board(&r.board_id) {
            BoardLog::fold_key(&mut lock(&b.maps), &b.id, ev);
        }
    }

    /// The one routing function from a taskId to its log (§2.1): "" and the
    /// private coord for a card on the private board, else the board whose
    /// log holds its task.created row. An id in no log is NotFound.
    pub(crate) fn route(
        &self,
        task_id: &str,
    ) -> std::result::Result<(String, Arc<Coord>), coord::Error> {
        if self.coord.get_task(task_id).is_some() {
            return Ok((String::new(), self.coord.clone()));
        }
        let mut inner = lock(&self.boards.inner);
        if let Some(id) = inner.task_of.get(task_id)
            && let Some(b) = inner.logs.get(id)
        {
            return Ok((id.clone(), b.coord().clone()));
        }
        // A row this device just appended reaches the follower a moment
        // later; its coord already has it.
        let hit = inner
            .logs
            .iter()
            .find(|(_, b)| b.coord().get_task(task_id).is_some())
            .map(|(id, b)| (id.clone(), b.coord().clone()));
        if let Some((id, co)) = hit {
            inner.task_of.insert(task_id.into(), id.clone());
            return Ok((id, co));
        }
        Err(coord::Error::new(
            CK::NotFound,
            format_args!("task {}", gocompat::quote::quote(task_id)),
        ))
    }

    /// Routes `task_id` ([`Services::route`]) and gives each of `keys` as
    /// the caller is known in that log: unchanged on the private board, a
    /// board key on a shared one ([`Services::board_key`]). An id in no log
    /// goes to the private coord, which reports it as not found in its own
    /// words. Tools and the window act on a card through it.
    pub(crate) fn task_coord(
        self: &Arc<Self>,
        task_id: &str,
        keys: &[String],
    ) -> std::result::Result<(Arc<Coord>, Vec<String>), coord::Error> {
        let board_id = match self.route(task_id) {
            Ok((id, _)) if !id.is_empty() => id,
            _ => {
                // A write to a private card that was shared to a board is
                // refused, never rerouted (§5.2).
                if !keys.is_empty()
                    && let Some(e) = self.shared_refusal(task_id)
                {
                    return Err(e.into_coord());
                }
                return Ok((self.coord.clone(), keys.to_vec()));
            }
        };
        let co = self
            .board(&board_id)
            .map_err(Error::into_coord)?
            .coord()
            .clone();
        let out = keys
            .iter()
            .map(|k| self.board_key(&board_id, k))
            .collect::<Result<Vec<_>>>()
            .map_err(Error::into_coord)?;
        Ok((co, out))
    }

    /// `key` as it is written on board `board_id` (§2.4): the person at this
    /// device becomes `person:<personId>`; an agent on any of the person's
    /// devices becomes `agent:<cardId>`, its card written the first time it
    /// touches the board; "irori" and keys already in board form are kept.
    /// Anything else is refused: a made-up user: key is no one, and an
    /// agent the roster doesn't have is not the person's.
    pub(crate) fn board_key(&self, board_id: &str, key: &str) -> Result<String> {
        let key = trim(key);
        if key.is_empty() || key == "irori" {
            return Ok(key.into());
        }
        if key.starts_with(PERSON_ACTOR_PREFIX) || key.starts_with(AGENT_ACTOR_PREFIX) {
            return Ok(key.into());
        }
        if self.chain.is_owner(key) {
            return Ok(self.person_actor());
        }
        if key.starts_with(USER_KEY_PREFIX) {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!(
                    "{} is not a person on this board",
                    gocompat::quote::quote(key)
                ),
            ));
        }
        let agent = key.strip_prefix("api:").unwrap_or(key);
        if irori_identity::owner::device_of(agent).is_empty() {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!(
                    "{} is not an agent of this person",
                    gocompat::quote::quote(agent)
                ),
            ));
        }
        let b = self.board(board_id)?;
        self.card_for(&b, agent)
    }

    /// `agent`'s cardId key on `b`, minting the card on first use: a random
    /// cardId per (agent, board), so two boards can't be joined on it, an
    /// agent.card with nothing but its name, kind and person, and the
    /// private board_cards row that says which session it is (§2.4).
    fn card_for(&self, b: &BoardLog, agent: &str) -> Result<String> {
        let mut m = lock(&b.maps);
        if let Some(id) = m.keys.get(agent) {
            return Ok(format!("{AGENT_ACTOR_PREFIX}{id}"));
        }
        let Some(card) = self.roster.agents().into_iter().find(|a| a.key == agent) else {
            return Err(Error::coord(
                CK::NotFound,
                format_args!(
                    "no agent {} on the person's devices",
                    gocompat::quote::quote(agent)
                ),
            ));
        };
        let id = new_board_id()?;
        let kind = if card.agent.is_empty() {
            "claude".to_string()
        } else {
            card.agent.clone()
        };
        let row = BoardCard {
            card_id: id.clone(),
            name: card.name.clone(),
            kind,
            person_id: self.person_id(),
            status: card_status(Some(&card.status)).into(),
        };
        // The mapping first: a card whose session nobody knows is worse
        // than a mapping to a card that failed to append (it is retried).
        self.store.append(
            "irori",
            kinds::BOARD_CARD_OF,
            &CardKeyRow {
                board_id: b.id.clone(),
                card_id: id.clone(),
                agent_key: agent.into(),
            },
        )?;
        b.store.append(
            &format!("{AGENT_ACTOR_PREFIX}{id}"),
            kinds::AGENT_CARD,
            &row,
        )?;
        m.add_key(&id, agent);
        m.cards.insert(id, row);
        Ok(format!("{AGENT_ACTOR_PREFIX}{}", m.keys[agent]))
    }

    /// Rewrites the agent.card of each of this device's agents on every
    /// board whose status moved (§2.4: `active`, `idle` or `gone`, written
    /// only when it changes). Only the device hosting the session writes
    /// it, so two devices never race on one card.
    pub(crate) fn sync_card_status(&self) {
        let live: HashMap<String, String> = self
            .roster
            .agents()
            .into_iter()
            .map(|a| (a.key, a.status))
            .collect();
        let me = self.device.id.as_str();
        for b in self.boards.list() {
            let mut m = lock(&b.maps);
            let mine: Vec<(String, String)> = m
                .by_id
                .iter()
                .filter(|(_, a)| irori_identity::owner::device_of(a) == me)
                .map(|(id, a)| (id.clone(), a.clone()))
                .collect();
            for (id, agent) in mine {
                let status = card_status(live.get(&agent).map(String::as_str));
                let Some(card) = m.cards.get(&id) else {
                    continue;
                };
                if card.status == status {
                    continue;
                }
                let row = BoardCard {
                    status: status.into(),
                    ..card.clone()
                };
                match b.store.append(
                    &format!("{AGENT_ACTOR_PREFIX}{id}"),
                    kinds::AGENT_CARD,
                    &row,
                ) {
                    Ok(_) => {
                        m.cards.insert(id, row);
                    }
                    Err(e) => logf(format_args!("board {}: card {id} status: {e}", b.id)),
                }
            }
        }
    }

    /// A board key as this device knows it: the session key of one of the
    /// person's cards (board_cards), the person at this device for its own
    /// person: key, or `key` unchanged.
    pub(crate) fn private_key(&self, key: &str) -> String {
        if key == self.person_actor() {
            return self.user_sender().key;
        }
        let Some(id) = key.strip_prefix(AGENT_ACTOR_PREFIX) else {
            return key.into();
        };
        for b in self.boards.list() {
            if let Some(agent) = lock(&b.maps).by_id.get(id) {
                return agent.clone();
            }
        }
        key.into()
    }

    /// Pins `b`'s Verifier from its board.pin rows (§3). A board with none
    /// is pinned to this person, rooted at this device, when this device
    /// creates it or wrote every row it holds (a board from before pins); a
    /// board holding another device's rows and no pin is left unpinned, so
    /// nothing in it verifies, and says so. `given` is the creator's pin
    /// from a board.joined row (or an invite bundle).
    fn pin_board(&self, b: &BoardLog, create: bool, given: Option<&MemberPin>) -> Result<()> {
        let evs = b.store.list(&Filter {
            kinds: vec![kinds::BOARD_PIN.into()],
            local: true,
            limit: MAX_LIST,
            ..Default::default()
        })?;
        let mut pinned = false;
        for ev in &evs {
            let Some(p) = irori_identity::decode::<MemberPin>(&ev.payload) else {
                continue;
            };
            if let Err(e) = b.verifier.pin(&p) {
                logf(format_args!("board {}: {e}", b.id));
                continue;
            }
            pinned = true;
        }
        if !pinned && let Some(p) = given {
            b.verifier.pin(p).map_err(Error::Other)?;
            b.store.append_local("irori", kinds::BOARD_PIN, p)?;
            pinned = true;
        }
        if !pinned {
            let vec = b.store.vector()?;
            if !create && vec.keys().any(|dev| *dev != self.device.id) {
                logf(format_args!(
                    "board {}: holds rows from another device but pins nobody; nothing in it verifies",
                    b.id
                ));
                return Ok(());
            }
            let p = self.ident.board_pin();
            b.store.append_local("irori", kinds::BOARD_PIN, &p)?;
            b.verifier.pin(&p).map_err(Error::Other)?;
        }
        // The rows already observed fold again, now pinned, and the store
        // applies their cuts.
        b.store.reload_verifier();
        Ok(())
    }

    /// Keeps every board's member.* rows current as the private log's
    /// person.* rows change: a device paired, revoked, or the key rotated
    /// (§3, "Keeping boards current").
    fn start_follow_identity(self: &Arc<Self>) {
        let Some(stop) = self.boards.stop.get() else {
            return;
        };
        let sub = Arc::new(self.store.subscribe());
        let reg = {
            let s = sub.clone();
            stop.on_cancel(move || s.cancel())
        };
        let s = Arc::downgrade(self);
        let h = std::thread::Builder::new()
            .name("irori-board-identity".into())
            .spawn(move || {
                let _reg = reg;
                while let Some(ev) = sub.recv() {
                    if ev.seq != 0 && !kinds::is_person_kind(&ev.kind) {
                        continue;
                    }
                    let Some(s) = s.upgrade() else { return };
                    for b in s.boards.list() {
                        if let Err(e) = s.keep_current(&b) {
                            logf(format_args!("board {}: identity rows: {e}", b.id));
                        }
                    }
                }
            })
            .expect("spawn a board thread");
        lock(&self.boards.identity).push(h);
    }

    /// Writes into `b` what this person's identity in it lacks (§3): this
    /// device's self-cert when it is the board root, then, from a device
    /// whose board cert counts, each rotation this device made (only the
    /// rotating device can, it alone signed prevSig), the revoke of each
    /// device the private chain revoked with a cut over b's own stream, a
    /// cert for each certified device of this person, and the profile.
    /// Rows are signed for the board's newest epoch of this person, so they
    /// are written only while this device's key is that epoch's.
    pub(crate) fn keep_current(&self, b: &BoardLog) -> Result<()> {
        let _g = lock(&b.id_mu);
        let (v, me_dev) = (&b.verifier, self.device.id.as_str());
        let me = self.ident.current();
        let Some(pin) = v.pinned(&me.id) else {
            return Ok(());
        };
        let m = v.membership();
        if m.created && !m.active(&me.id) {
            return Ok(()); // removed or left: nothing more of ours counts here
        }
        let role = self
            .chain
            .cert(me_dev)
            .map(|c| c.role)
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| irori_identity::objects::ROLE_DESKTOP.into());
        if v.cert(me_dev).is_none() {
            if pin.root_device_id != me_dev {
                return Ok(()); // another device of this person certifies this one
            }
            let add = self.ident.member_cert_for(
                pin.person_epoch,
                me_dev,
                &hex::encode(self.ident.device_pub()),
                &role,
                (self.now)(),
            )?;
            b.store.append("irori", kinds::MEMBER_DEVICE_ADDED, &add)?;
            if v.cert(me_dev).is_none() {
                return Err(Error::other("the self-cert did not certify this device"));
            }
        }
        if v.cut(me_dev).is_some() {
            return Ok(());
        }
        loop {
            let next = v.epoch(&me.id) + 1;
            let Some(rot) = self.own_rotation(next)? else {
                break;
            };
            if b.wrote_rotation(me_dev, next)? {
                return Ok(()); // written before and it didn't count (it lost a tie): not again
            }
            b.store.append("irori", kinds::MEMBER_KEY_ROTATED, &rot)?;
            if v.epoch(&me.id) < next {
                break; // it didn't count; don't write it again
            }
        }
        let epoch = v.epoch(&me.id);
        if epoch != self.ident.current().epoch || !v.writable(&me.id) {
            return Ok(()); // this key isn't the board's newest for this person
        }
        let devs = self.chain.devices();
        let vec = b.store.vector()?;
        for d in &devs {
            let dev = d.cert.device_id.as_str();
            if !d.revoked || v.cert(dev).is_none() || dev == me_dev || v.cut(dev).is_some() {
                continue;
            }
            let cut = vec.get(dev).copied().unwrap_or(0);
            let mut hash = String::new();
            if cut > 0 {
                let evs = b.store.list(&Filter {
                    device_id: dev.into(),
                    after_seq: cut - 1,
                    limit: 1,
                    ..Default::default()
                })?;
                if evs.len() == 1 && evs[0].seq == cut {
                    hash = store::row_hash(&evs[0]);
                }
            }
            let rev = self
                .ident
                .member_revoke_of(epoch, dev, cut, &hash, (self.now)())?;
            b.store
                .append("irori", kinds::MEMBER_DEVICE_REVOKED, &rev)?;
        }
        for d in &devs {
            let dev = d.cert.device_id.as_str();
            if d.revoked || v.cert(dev).is_some() {
                continue;
            }
            let add = self.ident.member_cert_for(
                epoch,
                dev,
                &d.cert.device_pub,
                &d.cert.role,
                (self.now)(),
            )?;
            b.store.append("irori", kinds::MEMBER_DEVICE_ADDED, &add)?;
        }
        if v.profile(&me.id).is_none() {
            let prof = self.ident.member_profile_row(epoch)?;
            b.store.append("irori", kinds::MEMBER_PROFILE, &prof)?;
        }
        Ok(())
    }

    /// The private person.key.rotated row this device wrote for `epoch`, as
    /// a board's member.key.rotated. `None` when this device made no such
    /// rotation, or made it before v0.5 (no prevSig).
    fn own_rotation(&self, epoch: i64) -> Result<Option<MemberKeyRotated>> {
        let me = self.person_id();
        let mut out = None;
        each_row(
            &self.store,
            Filter {
                kinds: vec![kinds::PERSON_KEY_ROTATED.into()],
                device_id: self.device.id.clone(),
                ..Default::default()
            },
            |ev| {
                if out.is_some() {
                    return;
                }
                let Some(r) = irori_identity::decode::<irori_identity::KeyRotated>(&ev.payload)
                else {
                    return;
                };
                if r.person_id != me || r.epoch != epoch {
                    return;
                }
                let (m, ok) = irori_identity::member_rotation(&r);
                if ok && m.by == self.device.id {
                    out = Some(m);
                }
            },
        )?;
        Ok(out)
    }

    /// This device's view of `person_id` on `board_id`.
    pub(crate) fn board_member(&self, board_id: &str, person_id: &str) -> Result<BoardIdentity> {
        let b = self.board(board_id)?;
        Ok(BoardIdentity {
            person_id: person_id.into(),
            epoch: b.verifier.epoch(person_id),
            name: b.verifier.profile(person_id).unwrap_or_default(),
            devices: b.verifier.devices(person_id),
        })
    }

    /// The agents a board ask on `b` is told to (§9): the board's agent
    /// cards, less this device's own sessions under Pause all (as the
    /// private room's ask_agents).
    fn board_ask_agents(&self, b: &BoardLog) -> Vec<AgentCard> {
        let all = roster_of(self, b);
        if !self.inbox.policy().paused {
            return all;
        }
        let me = self.device.id.as_str();
        let m = lock(&b.maps);
        all.into_iter()
            .filter(|a| {
                let id = a.key.trim_start_matches(AGENT_ACTOR_PREFIX);
                m.by_id
                    .get(id)
                    .is_none_or(|agent| irori_identity::owner::device_of(agent) != me)
            })
            .collect()
    }

    /// An ask notice's recipients this person's devices host, by their
    /// private keys: the other people's agents hear the ask through their
    /// own devices' bridge (§7.1), from the board.ask.state row.
    fn own_ask_notice(&self, b: &BoardLog, mut n: Notice) -> Notice {
        let m = lock(&b.maps);
        n.to.retain(|k| {
            k.strip_prefix(AGENT_ACTOR_PREFIX)
                .is_some_and(|id| m.by_id.contains_key(id))
        });
        drop(m);
        self.private_notice(n)
    }

    /// The person asks board `board_id`'s room (§9): the ask's card in the
    /// board log, owned by this device, told to the free agents of every
    /// member. This person's agents hear it from the returned notice (by
    /// private keys), which the caller routes as it chose; the others'
    /// from their own devices' bridge. A board's agent cards carry no repo, so a board ask names
    /// none.
    pub(crate) fn board_ask(
        &self,
        board_id: &str,
        input: coord::NewAsk,
    ) -> Result<(coord::Task, Notice)> {
        if !input.repo_key.trim().is_empty() {
            return Err(Error::coord(
                CK::Invalid,
                "a board ask goes to every member's free agents; leave the repo out",
            ));
        }
        let b = self.board(board_id)?;
        let agents = self.board_ask_agents(&b);
        let (t, n) = b
            .coord()
            .post_ask(&self.person_actor(), input, &agents)
            .map_err(Error::Coord)?;
        Ok((self.private_task(t), self.own_ask_notice(&b, n)))
    }

    /// Board coords' sweep notices and asks' timed rounds, routed like the
    /// private board's (Go `sweepBoards`).
    pub(crate) fn sweep_boards(&self) -> std::result::Result<(), String> {
        let mut errs = Vec::new();
        for b in self.boards.list() {
            let (mut ns, err) = b.coord().sweep_notices_partial();
            if let Err(e) = err {
                errs.push(e.to_string());
            }
            let (asks, aerr) = b.coord().sweep_asks_partial(&self.board_ask_agents(&b));
            if let Err(e) = aerr {
                errs.push(e.to_string());
            }
            ns.extend(asks.into_iter().map(|n| self.own_ask_notice(&b, n)));
            for n in ns {
                let from = if n.person {
                    self.user_sender()
                } else {
                    inbox::system_sender()
                };
                if let Some(e) = crate::notice::notice_err(&self.route_notice(&from, &n)) {
                    logf(format_args!(
                        "board {} notice {} {}: {e}",
                        b.id, n.kind, n.reference
                    ));
                }
            }
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs.join("\n"))
        }
    }
}

// ---- the Core's shared boards API ---------------------------------------------

impl super::Core {
    /// Makes a board between this person's own devices, named `name` with
    /// the short tag its card refs carry (`mlg#12`); returns its id.
    pub fn create_board(&self, name: &str, tag: &str) -> Result<String> {
        self.s.create_board(name, tag)
    }

    /// The person asks board `board_id`'s room (§9): every member's free
    /// agents hear it, this person's from the returned notice (by private
    /// keys; route it with [`Core::route_notices`]), the others' through
    /// their devices' bridge.
    pub fn board_ask(&self, board_id: &str, input: coord::NewAsk) -> Result<(coord::Task, Notice)> {
        self.s.board_ask(board_id, input)
    }

    /// Renames a board (owners).
    pub fn rename_board(&self, board_id: &str, name: &str) -> Result<()> {
        self.s.rename_board(board_id, name)
    }

    /// Drops a board on this device only; the person's other devices keep
    /// it (§4.6).
    pub fn forget_board(&self, board_id: &str) -> Result<()> {
        self.s.forget_board(board_id)
    }

    /// Takes the person off a board on every one of their devices: now
    /// when nobody else is on it, else after "Leaving…" (§4.4).
    pub fn leave_board(&self, board_id: &str) -> Result<()> {
        self.s.leave_board(board_id)
    }

    /// "Forget now" while a board shows "Leaving…": forgets it on every
    /// device of the person without waiting for another member to hold
    /// the `left` row, who then may not hear of it (§4.4).
    pub fn forget_left_board(&self, board_id: &str) -> Result<()> {
        self.s.forget_everywhere(board_id)
    }

    /// An owner removes member `person_id` from board `board_id` (§4.3),
    /// with `reason` (may be "").
    pub fn remove_member(&self, board_id: &str, person_id: &str, reason: &str) -> Result<()> {
        self.s.remove_member(board_id, person_id, reason)
    }

    /// An owner makes member `person_id` an `owner` or a `member` (§4.2).
    pub fn set_member_role(&self, board_id: &str, person_id: &str, role: &str) -> Result<()> {
        self.s.set_member_role(board_id, person_id, role)
    }

    /// Opens (creating) board `board_id`'s log, pinned to `pin` when the
    /// board has no pin yet: the creator from a board.joined row one of
    /// this person's devices wrote, or from an invite bundle.
    pub fn open_board(&self, board_id: &str, pin: Option<&MemberPin>) -> Result<()> {
        self.s.open_board(board_id, false, pin)?;
        self.s.boards.moved();
        Ok(())
    }

    /// The ids of the board logs open on this device, sorted.
    pub fn boards(&self) -> Vec<String> {
        self.s.boards.list().iter().map(|b| b.id.clone()).collect()
    }

    /// One board's coord.
    pub fn board_coord(&self, board_id: &str) -> Result<Arc<Coord>> {
        Ok(self.s.board(board_id)?.coord().clone())
    }

    /// One board's open log.
    pub fn board_store(&self, board_id: &str) -> Option<Arc<Store>> {
        self.s.board(board_id).ok().map(|b| b.store.clone())
    }

    /// One board's Verifier (its members and their device chains).
    pub fn board_verifier(&self, board_id: &str) -> Option<Arc<BoardVerifier>> {
        self.s.board(board_id).ok().map(|b| b.verifier.clone())
    }

    /// One board's secrets (§6.1).
    pub fn board_secrets(&self, board_id: &str) -> Option<Arc<BoardSecrets>> {
        self.s.board(board_id).ok().map(|b| b.secrets.clone())
    }

    pub fn board_info(&self, board_id: &str) -> Result<BoardInfo> {
        self.s.board_info(board_id)
    }

    pub fn board_member(&self, board_id: &str, person_id: &str) -> Result<BoardIdentity> {
        self.s.board_member(board_id, person_id)
    }

    /// The log a task lives in: "" and the private coord, or its board.
    pub fn route(&self, task_id: &str) -> std::result::Result<(String, Arc<Coord>), coord::Error> {
        self.s.route(task_id)
    }

    /// The coord of the log `task_id` lives in, with each of `keys` as that
    /// log knows them (Go `TaskCoord`).
    pub fn task_coord(
        &self,
        task_id: &str,
        keys: &[String],
    ) -> std::result::Result<(Arc<Coord>, Vec<String>), coord::Error> {
        self.s.task_coord(task_id, keys)
    }

    /// `key` as board `board_id` writes it (§2.4).
    pub fn board_key(&self, board_id: &str, key: &str) -> Result<String> {
        self.s.board_key(board_id, key)
    }

    /// A board key as this device knows it.
    pub fn private_key(&self, key: &str) -> String {
        self.s.private_key(key)
    }

    /// A board card with its keys as this device knows them.
    pub fn private_task(&self, t: coord::Task) -> coord::Task {
        self.s.private_task(t)
    }

    /// Every board on this device, by name.
    pub fn board_list(&self) -> Vec<BoardSummary> {
        self.s.board_list()
    }

    /// The board a task lives on, or `None` for a private card.
    pub fn board_of(&self, task_id: &str) -> Option<BoardSummary> {
        self.s.board_of(task_id)
    }

    /// A card ref across every log this device holds (B3).
    pub fn task_ref(&self, r: &str) -> std::result::Result<String, coord::Error> {
        self.s.task_ref(r)
    }

    /// Where private card `task_id` was shared to.
    pub fn shared_copy_of(&self, task_id: &str) -> Option<SharedCopy> {
        self.s.shared_copy_of(task_id)
    }

    /// Exactly what Share to board would write (§5.2).
    pub fn preview_share(&self, task_id: &str, board_id: &str) -> Result<SharePreview> {
        self.s.preview_share(trim(task_id), board_id)
    }

    /// Shares a private card as its confirmed preview showed; returns the
    /// shared card.
    pub fn share_to_board(&self, task_id: &str, board_id: &str, hash: &str) -> Result<coord::Task> {
        self.s.share_to_board(trim(task_id), board_id, hash)
    }

    /// Takes a shared card back off its board (§5.3); returns the private
    /// card the work carries on in, if any.
    pub fn withdraw_card(&self, task_id: &str) -> Result<Option<coord::Task>> {
        self.s.withdraw_card(trim(task_id))
    }

    /// `f`'s Board on every ready board this device holds.
    pub fn board_views(&self, f: &coord::BoardFilter) -> Vec<coord::BoardView> {
        self.s.board_views(f)
    }

    /// Every board coord's changes, and a zero Change when a board opens,
    /// goes or is renamed (the window's pump). Close the queue to stop.
    pub fn subscribe_boards(&self) -> Arc<Queue<Change>> {
        self.s.boards.subscribe()
    }
}

impl super::Core {
    /// The roster board `board_id`'s coord reads liveness from (tests).
    #[doc(hidden)]
    pub fn board_roster(&self, board_id: &str) -> Vec<AgentCard> {
        match self.s.board(board_id) {
            Ok(b) => BoardRoster {
                s: Arc::downgrade(&self.s),
                b: Arc::downgrade(&b),
            }
            .agents(),
            Err(_) => Vec::new(),
        }
    }

    /// Reconciles the open boards with the private log's board.joined and
    /// board.forgot rows now (tests; the Core does it as rows arrive).
    #[doc(hidden)]
    pub fn reconcile_boards(&self) {
        self.s.reconcile_boards();
    }
}

/// An agent.card's status (§2.4) for a session's presence status, `None`
/// when the session is gone from the roster.
pub(crate) fn card_status(presence: Option<&str>) -> &'static str {
    match presence {
        None | Some("gone") => "gone",
        Some("idle") => "idle",
        Some(_) => "active",
    }
}
