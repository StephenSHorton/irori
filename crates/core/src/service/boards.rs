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
use irori_identity::membership::{BoardPerson, Conflict, MEMBER_LEFT};
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

pub use super::boards_own::{
    BoardSummary, SharePreview, SharedCopy, SubtaskPreview, SummaryMember,
};

/// Where the space logs live, under the data dir (card #392, decision
/// 45). A space is what v0.5 called a shared board: its id is the board's
/// id, and its log, secrets, topic and members are the board's.
pub const SPACES_DIR: &str = "spaces";

/// Where v0.5 kept them. A start moves each board's files from here to
/// SPACES_DIR (migrate_board_files) before anything opens them.
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
        Error::NoBoard(format!("core: no such space on this device: {id}"))
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

/// One agent card on a shared board ([`super::Core::board_agents`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BoardAgent {
    /// The board's key for it: agent:<cardId>.
    pub key: String,
    pub name: String,
    /// busy, idle or gone (§2.4); live for the person's own agents.
    pub status: String,
    pub board_id: String,
    pub person_id: String,
    /// The member's name on the board, "" when unknown.
    pub person: String,
    /// The agent's face as its card carries it, checked (card #407); ""
    /// for the default.
    pub avatar: String,
    /// The member's own colour pick on their member.profile, "" for none
    /// (card #385); the window places it beside the viewer's devices.
    pub person_color: String,
    /// The member's avatar on their member.profile, "" for none (card
    /// #393).
    pub person_avatar: String,
}

/// Another person on a shared board ([`super::Core::board_people`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BoardMember {
    /// The board's key for them: person:<personId>.
    pub key: String,
    /// Their member.profile name, "" when unknown.
    pub name: String,
    pub board_id: String,
    /// Their own colour pick on their member.profile, "" for none (card
    /// #385).
    pub color: String,
    /// Their avatar on their member.profile, "" for none (card #393).
    pub avatar: String,
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
    /// The agent's face (card #407), open-bot-avatar JSON; "" for the
    /// default. Readers keep it only after coord::avatar::check.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub avatar: String,
    /// Irori started this agent for a card (spawned agents §9, card
    /// #455): it serves that card only, and no member's device offers it
    /// another. Written by the person's own device with the card.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub spawned: bool,
    /// The device that wrote its newest row: the session's host, as only
    /// that device writes a card's status (card #372). Never in the row.
    #[serde(skip)]
    pub device_id: String,
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
        let p = BoardCard {
            device_id: ev.device_id.clone(),
            ..p
        };
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

/// Whether a board row changes what the boards list shows (card #233): a
/// resync, a board.* / member.* row (name, tag, ready, people, devices), or
/// a space.board.* row (the boards in a space, card #558).
/// A board opened from another device's board.joined is empty until its
/// rows arrive, so the window has to read the list again as they land.
/// agent.card rows don't change the list and come often, so they don't.
pub(crate) fn summary_row(ev: &Event) -> bool {
    ev.seq == 0
        || (kinds::is_board_identity_kind(&ev.kind) && ev.kind != kinds::AGENT_CARD)
        || ev.kind.starts_with("space.board.")
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
    /// Card #232: the boards on disk open after the start, on the boards
    /// thread. A lookup that misses waits for that first pass.
    opening: Opening,
    /// One open (or close) of a board log at a time, so two callers can't
    /// open one file twice. Held across the open; `inner` is taken only
    /// briefly, so lookups and the window don't wait out a slow open
    /// (card #232). Order: `open_mu`, then `inner`.
    pub(crate) open_mu: Mutex<()>,
}

/// Whether the first open of the boards on disk is still running, and on
/// which thread (that thread never waits on itself).
#[derive(Default)]
struct Opening {
    state: Mutex<OpeningState>,
    done: std::sync::Condvar,
}

#[derive(Default)]
struct OpeningState {
    /// Set when the start hands the first open to the boards thread; a
    /// Boards that never opens any (tests) never waits.
    pending: bool,
    finished: bool,
    thread: Option<std::thread::ThreadId>,
}

/// Ends the first open pass when dropped, however [`Services::open_boards`]
/// returns.
struct OpenedOnDrop<'a>(&'a Opening);

impl Drop for OpenedOnDrop<'_> {
    fn drop(&mut self) {
        let mut st = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        st.finished = true;
        st.thread = None;
        self.0.done.notify_all();
    }
}

/// How long a lookup waits for the boards to open before it answers
/// without them.
const OPENING_WAIT: Duration = Duration::from_secs(60);

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

    /// [`Boards::list`] without waiting (card #422): `None` while another
    /// thread holds the list. A coord's busy hook reads it under that
    /// coord's lock.
    fn try_list(&self) -> Option<Vec<Arc<BoardLog>>> {
        let inner = match self.inner.try_lock() {
            Ok(g) => g,
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return None,
        };
        Some(inner.logs.values().cloned().collect())
    }

    pub(crate) fn get(&self, id: &str) -> Result<Arc<BoardLog>> {
        let get = || lock(&self.inner).logs.get(id).cloned();
        if let Some(b) = get() {
            return Ok(b);
        }
        // It may be one of the boards still opening after the start.
        if self.wait_opened()
            && let Some(b) = get()
        {
            return Ok(b);
        }
        Err(Error::no_board(id))
    }

    /// Marks the first open pass as coming (before its thread runs), so a
    /// lookup made meanwhile waits for it.
    pub(crate) fn opening_pending(&self) {
        let mut st = lock(&self.opening.state);
        if !st.finished {
            st.pending = true;
        }
    }

    /// Starts the first open pass on this thread; the guard ends it.
    fn begin_opening(&self) -> OpenedOnDrop<'_> {
        let mut st = lock(&self.opening.state);
        st.thread = Some(std::thread::current().id());
        OpenedOnDrop(&self.opening)
    }

    /// Waits until the first open of the boards on disk has finished (at
    /// most [`OPENING_WAIT`]). False when it had already finished, so there
    /// is nothing new to look at, or when this is the opening thread.
    pub(crate) fn wait_opened(&self) -> bool {
        let mut st = lock(&self.opening.state);
        if !st.pending || st.finished || st.thread == Some(std::thread::current().id()) {
            return false;
        }
        let deadline = Instant::now() + OPENING_WAIT;
        while !st.finished {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            st = self
                .opening
                .done
                .wait_timeout(st, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        true
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
        let _one = lock(&self.open_mu);
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
pub(super) fn roster_of(s: &Services, b: &BoardLog) -> Vec<AgentCard> {
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
        // Cards whose host device the meshes answer for (card #372), read
        // once the maps lock is dropped: device_presence takes the boards
        // list and reads board stores.
        let mut others: Vec<(usize, String)> = Vec::new();
        for (id, card) in &m.cards {
            let mut a = AgentCard {
                key: format!("{AGENT_ACTOR_PREFIX}{id}"),
                name: card.name.clone(),
                agent: card.kind.clone(),
                status: card.status.clone(),
                ..Default::default()
            };
            let mut placed = false;
            if let Some(agent) = m.by_id.get(id) {
                let mine = irori_identity::owner::device_of(agent) == me;
                match live.get(agent) {
                    None if mine => continue,
                    Some(l) if mine || l.status != "gone" => {
                        a.status = l.status.clone();
                        a.updated_at = l.updated_at;
                        // The person's own session: its device as the
                        // private roster has it (card #372).
                        a.device_id = l.device_id.clone();
                        a.device_state = l.device_state.clone();
                        a.device_seen = l.device_seen;
                        a.device_via = l.device_via.clone();
                        placed = true;
                    }
                    _ => {}
                }
            }
            if !placed && !card.device_id.is_empty() {
                // Another member's agent (or one of the person's no live
                // roster knows): its host device as the meshes see it.
                others.push((out.len(), card.device_id.clone()));
            }
            out.push(a);
        }
        drop(m);
        // One presence read per device, not per card.
        let mut seen: HashMap<String, super::liveness::DevicePresence> = HashMap::new();
        for (i, dev) in others {
            let p = seen
                .entry(dev.clone())
                .or_insert_with(|| s.device_presence(&dev))
                .clone();
            let a = &mut out[i];
            a.device_id = dev;
            a.device_state = p.state;
            a.device_seen = p.seen;
            a.device_via = p.via;
        }
        out
    }
}

/// The files of one board's (now one space's) log, in the order a move
/// takes them: the .db last, so a start that finds `<id>.db` still in
/// BOARDS_DIR knows the move isn't finished and takes what's left.
fn board_files(id: &str) -> [String; 4] {
    [
        format!("{id}.db-wal"),
        format!("{id}.db-shm"),
        format!("{id}.secrets"),
        format!("{id}.db"),
    ]
}

/// Moves every v0.5 board log from `<data>/boards` to `<data>/spaces`
/// (card #392): nothing in it is rewritten, so the move loses nothing. It
/// runs at the start, before any log is open. A file already in SPACES_DIR
/// is never overwritten: a crash half way leaves the rest for the next
/// start, and the space opens only once its .db has moved. The empty
/// BOARDS_DIR is removed.
pub fn migrate_board_files(data_dir: &std::path::Path) -> std::io::Result<usize> {
    let from = data_dir.join(BOARDS_DIR);
    let ents = match std::fs::read_dir(&from) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut ids: Vec<String> = ents
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_suffix(".db")
                .map(str::to_string)
        })
        .filter(|id| valid_board_id(id))
        .collect();
    ids.sort();
    let to = data_dir.join(SPACES_DIR);
    if !ids.is_empty() {
        std::fs::create_dir_all(&to)?;
    }
    let mut moved = 0;
    for id in &ids {
        for name in board_files(id) {
            let (src, dst) = (from.join(&name), to.join(&name));
            if !src.exists() {
                continue;
            }
            if dst.exists() {
                // Both copies of one file: the space's wins, and the old one
                // stays where it is for a person to look at.
                logf(format_args!(
                    "spaces: {name} is in both {BOARDS_DIR} and {SPACES_DIR}; keeping {SPACES_DIR}'s"
                ));
                continue;
            }
            std::fs::rename(&src, &dst)?;
        }
        moved += 1;
    }
    // Only an empty dir goes; anything left in it stays for a look.
    let _ = std::fs::remove_dir(&from);
    if moved > 0 {
        logf(format_args!(
            "spaces: moved {moved} board log(s) from {BOARDS_DIR} to {SPACES_DIR}"
        ));
    }
    Ok(moved)
}

impl Services {
    /// Card #422: a working card the person's agent `agent` (its mesh key)
    /// holds in a log other than `from`'s, as `tag#n "title"`: one card at
    /// a time holds across the private board and every shared board.
    /// `from` is the log asking: `None` the private board, else that
    /// board's id. On a board the agent is `agent:<cardId>` under each of
    /// the cards its devices minted for it.
    ///
    /// Called under the asking coord's lock, so it never waits on a lock
    /// that may be waiting on that coord. A board coord may wait on the
    /// private one (as `outside_leases` does), so a board asks the private
    /// coord outright. Everything else is tried without waiting: the
    /// private coord never waits on a board's and two boards never wait on
    /// each other. A log busy at that instant counts as not busy, i.e. as
    /// before #422: the next hand-out or join reads it again.
    pub(crate) fn busy_outside(&self, agent: &str, from: Option<&str>) -> Option<String> {
        if agent.is_empty() {
            return None;
        }
        if from.is_some()
            && let Some(d) = self.coord.busy_card(agent)
        {
            return Some(d);
        }
        for b in self.boards.try_list()? {
            if from == Some(b.id.as_str()) {
                continue;
            }
            let Some(co) = b.coord.get() else { continue };
            let cards: Vec<String> = match b.maps.try_lock() {
                Ok(m) => m
                    .by_id
                    .iter()
                    .filter(|(_, a)| a.as_str() == agent)
                    .map(|(id, _)| format!("{AGENT_ACTOR_PREFIX}{id}"))
                    .collect(),
                Err(_) => continue,
            };
            if let Some(d) = cards.iter().find_map(|k| co.try_busy_card(k)) {
                return Some(d);
            }
        }
        None
    }

    pub(crate) fn boards_dir(&self) -> PathBuf {
        self.data_dir.join(SPACES_DIR)
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
        let _opened = self.boards.begin_opening();
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
            // The boards open after the start (card #232): a Core closing
            // meanwhile opens no more.
            if self.boards.stop.get().is_some_and(|c| c.is_canceled()) {
                return;
            }
            match self.open_board(&id, false, None) {
                // The window lists it now.
                Ok(_) => self.boards.moved(),
                Err(e) => logf(format_args!("board {id}: {e}")),
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
                    "a space name is 1 to {} characters",
                    irori_identity::membership::MAX_BOARD_NAME
                ),
            ));
        }
        if !irori_identity::membership::valid_board_tag(tag) {
            return Err(Error::coord(
                CK::Invalid,
                format_args!(
                    "a board tag is a lowercase letter, then lowercase letters, digits or '-', {} characters at most",
                    irori_identity::membership::MAX_BOARD_TAG
                ),
            ));
        }
        // A board inside a space holds its tag too (card #558).
        if let Some(other) = self.board_tag_holder(tag) {
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
        for b in self.visible_boards() {
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
                    "a space name is 1 to {} characters",
                    irori_identity::membership::MAX_BOARD_NAME
                ),
            ));
        }
        let b = self.board(board_id)?;
        let me = self.person_id();
        if !b.verifier.membership().owner(&me) {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!("only an owner renames this space"),
            ));
        }
        let epoch = b.verifier.epoch(&me);
        if epoch != self.ident.current().epoch {
            return Err(Error::coord(
                CK::NotAllowed,
                "this device's person key isn't the space's newest yet",
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
        let _one = lock(&self.boards.open_mu);
        if let Some(b) = lock(&self.boards.inner).logs.get(id) {
            return Ok(b.clone());
        }
        // Card #232: where a board's open goes, in one log line.
        let mut laps = super::StartLaps::new();
        let dir = self.boards_dir();
        let secrets = Arc::new(
            BoardSecrets::open(
                &dir.join(format!("{id}.secrets")),
                id,
                Box::new(IdentKeystore(self.ident.clone())),
            )
            .map_err(|e| Error::other(format!("board secrets: {e}")))?,
        );
        laps.lap("secrets");
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
        laps.lap("store");
        {
            let v = v.clone();
            // A maker counts once the board certifies it, or, before the
            // board's rows arrive, when it is one of this person's own
            // devices or one of them vouched for it (board.secret on the
            // private mesh). A hidden DM space has one secret, its founder's,
            // and when the founder leaves nobody is left who can make
            // another: if that cut its maker, the founder's room would close
            // before the `left` row could go out (card #538). Only leaving
            // does this: a device revoked or removed stays cut.
            let (chain, vouched, board, dms) = (
                self.chain.clone(),
                self.boards.vouched.clone(),
                id.to_string(),
                self.dm_spaces.clone(),
            );
            secrets.set_makers(Some(Box::new(move |dev: &str| {
                let left = || {
                    let m = v.membership();
                    v.person_of(dev)
                        .and_then(|p| m.person(&p).map(|p| p.state == MEMBER_LEFT))
                        .unwrap_or(false)
                };
                (v.cut(dev).is_none() || (dms.get(&board).is_some() && left()))
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
        laps.lap("pin");
        if let Err(e) = b.load(&self.store) {
            return close(e);
        }
        laps.lap("load");
        if let Err(e) = self.keep_current(&b) {
            // The board still opens: its rows verify as far as they can,
            // and the next identity change retries.
            logf(format_args!("board {id}: identity rows: {e}"));
        }
        if let Err(e) = self.rotate_secret(&b) {
            logf(format_args!("board {id}: secret: {e}"));
        }
        laps.lap("identity+secret");
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
                    done_check: true,
                    // A board's own limit, else the person's (card #576).
                    crash_limit: Some(super::spawn::SpawnState::limit_fn(
                        &self.spawn,
                        &b.id,
                        |s| s.crash_limit,
                    )),
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
                    // Under the board coord's lock: takes the inbox's, whose
                    // hooks never wait on a board coord (the stale hook reads
                    // only the private one, the notes hook tries the boards:
                    // coord::with_inbox_lock_held).
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
                    // This person's private leases (card #339): a board
                    // card's external wait naming some held ones closes
                    // when they go. Under the board coord's lock: takes the
                    // private coord's, which never waits on a board coord.
                    // One card at a time across logs (card #422): the
                    // board's agent:<cardId> is the person's session by
                    // this board's card map (another member's agent maps to
                    // none and counts here only); then every other log is
                    // asked by its key there (see busy_outside's locks).
                    busy_elsewhere: Some({
                        let (s, bw) = (Arc::downgrade(self), Arc::downgrade(&b));
                        Arc::new(move |key: &str| {
                            let (s, b) = (s.upgrade()?, bw.upgrade()?);
                            let card = key.strip_prefix(AGENT_ACTOR_PREFIX)?;
                            let agent = lock(&b.maps).by_id.get(card)?.clone();
                            s.busy_outside(&agent, Some(&b.id))
                        })
                    }),
                    agent_devices: Some(self.spawn.agent_devices_fn()),
                    // The person's spawning switch for this board (card
                    // #455): set in the app only, off unless they set it.
                    spawning: Some({
                        let (on, def, id) = (
                            self.spawn.on.clone(),
                            self.spawn.private_default.clone(),
                            b.id.clone(),
                        );
                        Arc::new(move || super::spawn::SpawnState::switch(&on, &def, &id))
                    }),
                    outside_leases: Some({
                        let s = Arc::downgrade(self);
                        Arc::new(move || {
                            s.upgrade().map_or_else(Vec::new, |s| {
                                s.coord.leases().into_iter().map(|l| l.resource).collect()
                            })
                        })
                    }),
                    // Who takes work the board hands out (card #453): the
                    // board's agent:<cardId> is the person's session by
                    // this board's card map; theirs takes it only with
                    // "Offer it cards" on and never when Irori started it.
                    // Another member's agent maps to none: their device
                    // decides whether it offers it here.
                    offers: Some({
                        let (s, bw) = (Arc::downgrade(self), Arc::downgrade(&b));
                        Arc::new(move |key: &str| {
                            let (Some(s), Some(b)) = (s.upgrade(), bw.upgrade()) else {
                                return false;
                            };
                            let Some(card) = key.strip_prefix(AGENT_ACTOR_PREFIX) else {
                                return true;
                            };
                            let maps = lock(&b.maps);
                            // The Office Assistant answers in its channel;
                            // it is never offered a card.
                            if maps
                                .cards
                                .get(card)
                                .is_some_and(|c| c.kind == coord::roster::ASSISTANT_KIND)
                            {
                                return false;
                            }
                            let Some(agent) = maps.by_id.get(card).cloned() else {
                                // Another member's agent: never one their
                                // Irori started for a card (card #455).
                                return !maps.cards.get(card).is_some_and(|c| c.spawned);
                            };
                            drop(maps);
                            !s.coord.is_spawned(&agent) && s.agent_offered(&agent)
                        })
                    }),
                    // The other boards in this space (card #392), read from
                    // the log itself: a card on one shows its tag.
                    board_tag: Some({
                        let (st, vt) = (Arc::downgrade(&st), v.clone());
                        let may = super::spaces::member_wrote;
                        Arc::new(move |board: &str| {
                            let st = st.upgrade()?;
                            super::spaces::board_tag_in(&st, board, &may(&vt))
                        })
                    }),
                    no_cards: Some({
                        let (s, id) = (Arc::downgrade(self), b.id.clone());
                        Arc::new(move || s.upgrade().is_some_and(|s| s.is_hidden_space(&id)))
                    }),
                    board_tags: Some({
                        let st = Arc::downgrade(&st);
                        Arc::new(move || {
                            st.upgrade()
                                .map_or_else(Vec::new, |st| super::spaces::board_tags_in(&st))
                        })
                    }),
                    board_view: Some({
                        let (st, v, id) = (Arc::downgrade(&st), v.clone(), b.id.clone());
                        Arc::new(move || {
                            let st = st.upgrade()?;
                            super::spaces::with_space_rules(&v, |r| {
                                super::spaces::board_list_in(&st, &id, r)
                            })
                        })
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
        laps.lap("coord");
        let tasks = match b.task_ids() {
            Ok(t) => t,
            Err(e) => return close(e),
        };
        {
            let mut inner = lock(&self.boards.inner);
            for t in tasks {
                inner.task_of.insert(t, id.into());
            }
            inner.logs.insert(id.into(), b.clone());
        }
        laps.lap("tasks");
        self.start_board(&b);
        self.fold_space_talk(&b);
        self.boards
            .follow
            .get_or_init(|| self.start_follow_identity());
        self.open_board_net(&b);
        self.boards.kick();
        laps.lap("start+net");
        logf(format_args!("board {id}: opened in {}", laps.summary()));
        Ok(b)
    }

    /// Opens the board on the network (boards stage 2): its member devices,
    /// this person's other devices among them, are dialled on
    /// `irori/board/1` and its rows move only there.
    pub(crate) fn open_board_net(&self, b: &BoardLog) {
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
            let co = Arc::downgrade(b.coord());
            let id = b.id.clone();
            b.spawn("changes", move || {
                let _reg = reg;
                while let Some(ch) = q.recv() {
                    match s.upgrade() {
                        // A card in a hidden DM space reaches no window or
                        // feed (card #538).
                        Some(s) if s.is_dm_space(&id) => {}
                        Some(s) => {
                            // A board thread goes out, and starts its
                            // agent, at once (card #512): its rows kick the
                            // hand-out as the private log's do.
                            if co.upgrade().is_some_and(|c| {
                                c.get_task(&ch.task_id)
                                    .is_some_and(|t| t.open_ask().is_some())
                            }) {
                                s.asks_kick.kick();
                            }
                            s.boards.tell(ch);
                            // A shared card's blocker resolved (card #482).
                            s.settle_notifications();
                        }
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
        let hidden = self.is_dm_space(&b.id);
        // One of this person's agents ticked a step or set a card's state
        // on the board: its message chains start counting again (card
        // #488). Another member's agent keeps its board key and is skipped,
        // and no agent works in a hidden space (card #538).
        if !hidden && let Some((inbox::ProgressBy::Actor(a), at)) = inbox::progress_of(ev) {
            let key = self.private_key(&a);
            if !key.starts_with(AGENT_ACTOR_PREFIX) {
                self.inbox.progressed(&key, at);
            }
        }
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
            // The person left it on another device: its channels leave
            // Talk here too (card #461).
            if self.board_leaving(b) && self.talk.drop_space(&b.id) {
                self.hub.publish(crate::events::CoreEvent::Talk);
            }
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
        if self.board_leaving(b) {
            // A space the person left: its channels stay out of Talk.
        } else if ev.seq == 0 {
            // The space's channels fold again with everything else.
            let _ = self.talk.load();
            self.hub.publish(crate::events::CoreEvent::Talk);
        } else if hidden && irori_identity::kinds::is_membership_kind(&ev.kind) {
            self.dm_refold(b);
        } else if ev.kind == kinds::BOARD_MEMBER_ADDED {
            // Re-added while still "Leaving…" (card #461): the channels
            // drop_space took out fold back; a no-op for anyone else's add.
            self.fold_space_talk(b);
        } else if let Some(row) = self.space_talk_row(&b.id, ev) {
            // A space's channel row (card #392). One this device wrote was
            // folded when it was written; the window still hears it.
            self.talk.apply(&row);
            self.hub.publish(crate::events::CoreEvent::Talk);
            // A DM from the other person rings here (card #538); the
            // private log's follower never sees this row.
            if hidden && row.kind == kinds::TALK_POST && ev.actor != self.person_actor() {
                self.talk_note(&row);
            }
        }
        // Nothing in a hidden space reaches an agent (card #538).
        if !hidden {
            self.bridge_row(b, ev);
        }
        if summary_row(ev) {
            self.boards.moved();
        }
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
    ///
    /// Never holds the boards list while it reads a board coord (card
    /// #561): a coord's holder may wait on the list (`private_key` in the
    /// hop read, `device_presence` in the roster), and the inbox's notes
    /// hook waits on it under the inbox lock.
    pub(crate) fn route(
        &self,
        task_id: &str,
    ) -> std::result::Result<(String, Arc<Coord>), coord::Error> {
        if self.coord.get_task(task_id).is_some() {
            return Ok((String::new(), self.coord.clone()));
        }
        let logs: Vec<Arc<BoardLog>> = {
            let inner = lock(&self.boards.inner);
            // A card in a hidden DM space is nobody's to reach (card #538):
            // only its other member could have written one.
            if let Some(id) = inner.task_of.get(task_id)
                && !self.is_dm_space(id)
                && let Some(b) = inner.logs.get(id)
            {
                return Ok((id.clone(), b.coord().clone()));
            }
            inner.logs.values().cloned().collect()
        };
        // A row this device just appended reaches the follower a moment
        // later; its coord already has it.
        if let Some(b) = logs
            .iter()
            .find(|b| !self.is_dm_space(&b.id) && b.coord().get_task(task_id).is_some())
        {
            let mut inner = lock(&self.boards.inner);
            // Not for a board dropped meanwhile.
            if inner.logs.contains_key(&b.id) {
                inner.task_of.insert(task_id.into(), b.id.clone());
            }
            return Ok((b.id.clone(), b.coord().clone()));
        }
        // A card on a board still opening after the start (card #232).
        if self.boards.wait_opened() {
            return self.route(task_id);
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
                    "{} is not a person in this space",
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
            avatar: self.faces.get(&card.name),
            spawned: self.coord.is_spawned(agent),
            device_id: self.device.id.clone(),
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

    /// The id of this person's Office Assistant card on board `b`, minting it
    /// when there is none (assistants talk, card 2). Only the assistant's
    /// computer calls this. The card is an `agent.card` of kind `assistant`
    /// written as itself, with no roster entry and no `board_cards` mapping,
    /// and the person's `member.profile` then points at it. An existing
    /// assistant card of this person's is pointed at rather than a second
    /// minted.
    pub(crate) fn assistant_card(&self, b: &BoardLog) -> Result<Option<String>> {
        let me = self.person_id();
        let v = &b.verifier;
        let epoch = v.epoch(&me);
        if epoch != self.ident.current().epoch
            || !v.writable(&me)
            || v.cert(&self.device.id).is_none()
            || v.cut(&self.device.id).is_some()
        {
            return Ok(None); // not a writable member of this board yet
        }
        let had = v.profile_assistant_card(&me);
        if !had.is_empty() {
            return Ok(Some(had));
        }
        // Reuse a card of this person's before minting another.
        let existing = {
            let m = lock(&b.maps);
            m.cards
                .iter()
                .filter(|(id, c)| {
                    c.kind == coord::roster::ASSISTANT_KIND
                        && v.card_person(id).as_deref() == Some(me.as_str())
                })
                .map(|(id, _)| id.clone())
                .min()
        };
        let id = match existing {
            Some(id) => id,
            None => {
                let id = new_board_id()?;
                let row = BoardCard {
                    card_id: id.clone(),
                    name: format!("{}'s assistant", self.ident.current().name),
                    kind: coord::roster::ASSISTANT_KIND.into(),
                    person_id: me.clone(),
                    status: "idle".into(),
                    avatar: String::new(),
                    spawned: false,
                    device_id: self.device.id.clone(),
                };
                b.store.append(
                    &format!("{AGENT_ACTOR_PREFIX}{id}"),
                    kinds::AGENT_CARD,
                    &row,
                )?;
                lock(&b.maps).cards.insert(id.clone(), row);
                id
            }
        };
        let mut prof = self.ident.member_profile_row(epoch)?;
        prof.color = self.person_color.chosen();
        prof.avatar = self.person_avatar.chosen();
        prof.github_login = v.profile_github_login(&me);
        prof.assistant_card = id.clone();
        b.store.append("irori", kinds::MEMBER_PROFILE, &prof)?;
        Ok(Some(id))
    }

    /// The host sweep's part: on the assistant's computer, every space the
    /// person is in gets its assistant card once.
    pub(crate) fn mint_assistant_cards(&self) {
        if !self.is_assistant_home() {
            return;
        }
        for b in self.visible_boards() {
            if self.board_leaving(&b) {
                continue;
            }
            if let Err(e) = self.assistant_card(&b) {
                logf(format_args!("board {}: assistant card: {e}", b.id));
            }
        }
        // Questions that came while this wasn't the assistant's computer.
        self.assistant_home_moved();
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
        for b in self.visible_boards() {
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
                // The agent's face travels with its card (card #407).
                let avatar = self.faces.get(&card.name);
                let spawned = self.coord.is_spawned(&agent);
                if card.status == status && card.avatar == avatar && card.spawned == spawned {
                    continue;
                }
                let row = BoardCard {
                    status: status.into(),
                    avatar,
                    spawned,
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
        // None yet, or the person renamed themself (card #183), picked a
        // colour (card #385) or an avatar (card #393): the board shows the
        // newest of each. The GitHub login (card #625) once this device's gh
        // has said it: members' merge desks show this person's PRs by it.
        // A profile that has one keeps it, so two devices signed in to
        // different accounts don't take turns rewriting it.
        let color = self.person_color.chosen();
        let avatar = self.person_avatar.chosen();
        let assistant = self.assistant_name.chosen();
        let had = v.profile_github_login(&me.id);
        let had_assistant = v.profile_assistant_card(&me.id);
        let login = self.own_github_login();
        if v.profile(&me.id)
            .is_none_or(|n| n != self.ident.current().name)
            || v.profile_color(&me.id) != color
            || v.profile_avatar(&me.id) != avatar
            || v.profile_assistant_name(&me.id) != assistant
            || (had.is_empty() && !login.is_empty())
        {
            let mut prof = self.ident.member_profile_row(epoch)?;
            prof.color = color;
            prof.avatar = avatar;
            prof.assistant_name = assistant;
            prof.github_login = if had.is_empty() { login } else { had };
            // The assistant pointer rides every rewrite (card 2).
            prof.assistant_card = had_assistant;
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
                "a space ask goes to every member's free agents; leave the repo out",
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

    /// Board coords' sweep notices and asks, routed like the private
    /// board's (Go `sweepBoards`).
    pub(crate) fn sweep_boards(&self) -> std::result::Result<(), String> {
        self.sweep_boards_with(true)
    }

    /// Only the boards' asks: handed out and on (#91).
    pub(crate) fn sweep_board_asks(&self) -> std::result::Result<(), String> {
        self.sweep_boards_with(false)
    }

    /// Puts this person's idle agents that aren't on board `b` yet on it,
    /// one per unheld card there that nobody on the board is free to take
    /// (card #337: Stephen, "a board's pool includes the person's own idle
    /// agents when the person is on that board"). The idle longest go
    /// first; the hand-out sweep that follows picks among them.
    fn card_idle_agents(&self, b: &BoardLog) {
        if self.inbox.policy().paused {
            return;
        }
        let n = b.coord().unserved_offers(&self.board_ask_agents(b));
        if n == 0 {
            return;
        }
        let me = self.device.id.as_str();
        let mut idle: Vec<AgentCard> = {
            let m = lock(&b.maps);
            self.roster
                .agents()
                .into_iter()
                .filter(|a| {
                    a.status == "idle"
                        && a.device_state != coord::roster::OFFLINE
                        && !m.keys.contains_key(&a.key)
                        // Started for one card (card #446): no board's.
                        && !self.coord.is_spawned(&a.key)
                        // A terminal agent only with "Offer it cards" on
                        // (card #453).
                        && self.agent_offered(&a.key)
                        && (a.device_id == me
                            || a.device_id.is_empty()
                                && irori_identity::owner::device_of(&a.key) == me)
                })
                .collect()
        };
        idle.sort_by_key(|a| (a.updated_at.unix_nano(), a.key.clone()));
        for a in idle.into_iter().take(n) {
            if let Err(e) = self.card_for(b, &a.key) {
                logf(format_args!("board {}: put {} on it: {e}", b.id, a.key));
            }
        }
    }

    fn sweep_boards_with(&self, notices: bool) -> std::result::Result<(), String> {
        let mut errs = Vec::new();
        for b in self.visible_boards() {
            let mut ns = Vec::new();
            if notices {
                let (n, err) = b.coord().sweep_notices_partial();
                if let Err(e) = err {
                    errs.push(e.to_string());
                }
                ns = n;
            }
            self.card_idle_agents(&b);
            let (asks, aerr) = b.coord().sweep_asks_partial(&self.board_ask_agents(&b));
            if let Err(e) = aerr {
                errs.push(e.to_string());
            }
            ns.extend(asks.into_iter().map(|n| self.own_ask_notice(&b, n)));
            if notices {
                // Card #372: this person's agents hosted here hear when an
                // agent they work with on the board drops off.
                let me = self.device.id.clone();
                let hosted: std::collections::HashSet<String> = lock(&b.maps)
                    .by_id
                    .iter()
                    .filter(|(_, a)| irori_identity::owner::device_of(a) == me)
                    .map(|(id, _)| format!("{AGENT_ACTOR_PREFIX}{id}"))
                    .collect();
                let agents = roster_of(self, &b);
                let away = b.coord().sweep_presence(&agents, &|k| hosted.contains(k));
                ns.extend(away.into_iter().map(|n| self.private_notice(n)));
            }
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

    /// The ids of the board logs open on this device, sorted, hidden DM
    /// spaces too (card #538): no person or agent reads this, and what
    /// they see is [`Self::board_list`].
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

    /// The member a board key (`person:<id>`, `agent:<cardId>`) of board
    /// `board_id` belongs to, or `""` (card #268: a card's owner).
    /// An agent the board has no card-of row for yet goes by the person
    /// its agent card names, as [`Self::board_agents`] does (card #146).
    pub fn board_key_person(&self, board_id: &str, key: &str) -> String {
        let Ok(b) = self.s.board(board_id) else {
            return String::new();
        };
        let p = b.verifier.key_person(key);
        if !p.is_empty() {
            return p;
        }
        key.strip_prefix(AGENT_ACTOR_PREFIX)
            .and_then(|id| lock(&b.maps).cards.get(id).map(|c| c.person_id.clone()))
            .unwrap_or_default()
    }

    /// A board key as this device knows it.
    pub fn private_key(&self, key: &str) -> String {
        self.s.private_key(key)
    }

    /// A board card with its keys as this device knows them.
    pub fn private_task(&self, t: coord::Task) -> coord::Task {
        self.s.private_task(t)
    }

    /// How fresh the private log and each shared space are (card #580).
    pub fn freshness(&self) -> Vec<crate::service::freshness::LogFreshness> {
        self.s.freshness()
    }

    /// Every board on this device the person sees, by name.
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

    /// Card #432: the person offers shared card `task_id` to member `to`
    /// when they own it, or asks its owner for it; returns the
    /// Accept/Decline item it raised for the other side.
    pub fn transfer_owner(&self, task_id: &str, to: &str) -> Result<coord::Blocker> {
        self.s.transfer_owner(trim(task_id), trim(to))
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
    /// Home's idle line (card #341): the private board's idle agents, and
    /// the unheld cards nobody has been handed yet on the private board
    /// and every shown shared board, with why. A board card waiting
    /// because nobody in the board's pool is free says so (wind#87-91 on
    /// 2026-11-03).
    pub fn idle_report(&self) -> coord::IdleReport {
        let s = &self.s;
        let mut r = s.coord.idle_report(&s.ask_agents());
        for b in s.visible_boards() {
            let br = b.coord().idle_report(&s.board_ask_agents(&b));
            for mut u in br.unoffered {
                if br.idle == 0 {
                    u.why = "nobody in this space takes offered cards".into();
                }
                r.unoffered.push(u);
            }
        }
        r
    }

    /// Every agent card on the boards this device holds, keyed as the
    /// boards write them (agent:<cardId>), with the person it belongs to
    /// (card #146): the window names another member's agents from it, so a
    /// card they hold shows them on it, not "Nobody on it". The person's
    /// own cards come back too, with their live status.
    pub fn board_agents(&self) -> Vec<BoardAgent> {
        let mut out = Vec::new();
        for b in self.s.visible_boards() {
            let people: HashMap<String, String> = b
                .verifier
                .membership()
                .members
                .into_iter()
                .map(|p| (p.person_id, p.name))
                .collect();
            let (person_of, face_of): (HashMap<String, String>, HashMap<String, String>) = {
                let m = lock(&b.maps);
                (
                    m.cards
                        .iter()
                        .map(|(id, c)| (format!("{AGENT_ACTOR_PREFIX}{id}"), c.person_id.clone()))
                        .collect(),
                    m.cards
                        .iter()
                        .map(|(id, c)| (format!("{AGENT_ACTOR_PREFIX}{id}"), c.avatar.clone()))
                        .collect(),
                )
            };
            for a in roster_of(&self.s, &b) {
                let person_id = person_of.get(&a.key).cloned().unwrap_or_default();
                out.push(BoardAgent {
                    person: people.get(&person_id).cloned().unwrap_or_default(),
                    person_color: b.verifier.profile_color(&person_id),
                    avatar: checked_face(face_of.get(&a.key).map(String::as_str).unwrap_or("")),
                    person_avatar: b.verifier.profile_avatar(&person_id),
                    key: a.key,
                    name: a.name,
                    status: a.status,
                    board_id: b.id.clone(),
                    person_id,
                });
            }
        }
        out
    }

    /// The other people on the boards this device holds, keyed as the
    /// boards write them (person:<personId>), named by their member.profile
    /// (else the name they were added with): the @ picker's people and the
    /// names a mention of one shows (card #177). This person isn't listed.
    pub fn board_people(&self) -> Vec<BoardMember> {
        let me = self.s.person_id();
        let mut out = Vec::new();
        for b in self.s.visible_boards() {
            for p in b.verifier.membership().members {
                if p.person_id.is_empty() || p.person_id == me {
                    continue;
                }
                let name = b
                    .verifier
                    .profile(&p.person_id)
                    .filter(|n| !n.is_empty())
                    .unwrap_or(p.name);
                out.push(BoardMember {
                    key: format!("{PERSON_ACTOR_PREFIX}{}", p.person_id),
                    name,
                    board_id: b.id.clone(),
                    color: b.verifier.profile_color(&p.person_id),
                    avatar: b.verifier.profile_avatar(&p.person_id),
                });
            }
        }
        out
    }

    /// The roster board `board_id`'s coord reads liveness from (tests).
    #[doc(hidden)]
    pub fn board_roster(&self, board_id: &str) -> Vec<AgentCard> {
        match self.s.board(board_id) {
            Ok(b) if !self.s.is_dm_space(board_id) => BoardRoster {
                s: Arc::downgrade(&self.s),
                b: Arc::downgrade(&b),
            }
            .agents(),
            _ => Vec::new(),
        }
    }

    /// Reconciles the open boards with the private log's board.joined and
    /// board.forgot rows now (tests; the Core does it as rows arrive).
    #[doc(hidden)]
    pub fn reconcile_boards(&self) {
        self.s.reconcile_boards();
    }

    /// Whether `id` is a hidden DM space by this person's markers (tests;
    /// card #538).
    #[doc(hidden)]
    pub fn is_dm_space(&self, id: &str) -> bool {
        self.s.is_dm_space(id)
    }

    /// DM space `id`'s creator and other person (tests; card #538).
    #[doc(hidden)]
    pub fn dm_space(&self, id: &str) -> Option<(MemberPin, String)> {
        self.s.dm_spaces.get(id).map(|d| (d.creator, d.with))
    }

    /// Writes a DM space's private marker (tests; card #538).
    #[doc(hidden)]
    pub fn write_dm_marker(&self, id: &str, creator: &MemberPin, with: &str) -> Result<()> {
        self.s.write_dm_marker(id, creator, with)
    }

    /// Holds the boards list until the guard drops (tests: nothing may
    /// keep it while waiting on a board coord).
    #[doc(hidden)]
    pub fn hold_boards_list(&self) -> impl Sized + '_ {
        lock(&self.s.boards.inner)
    }
}

/// An agent.card's status (§2.4) for a session's presence status, `None`
/// when the session is gone from the roster.
pub(crate) fn card_status(presence: Option<&str>) -> &'static str {
    match presence {
        None | Some("gone") => "gone",
        // Not answering (#110) stays idle on a shared board: §2.4 has no
        // word for it yet.
        Some("idle") | Some(coord::roster::NOT_ANSWERING) => "idle",
        Some(_) => "active",
    }
}

/// A face from a board row, kept only when it passes coord::avatar::check
/// (card #407): a friend's board never hands the window anything else.
fn checked_face(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|v| coord::avatar::check(&v).ok())
        .map(|a| coord::avatar::to_json(&a))
        .unwrap_or_default()
}
