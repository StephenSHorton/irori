//! The Board (I:internal/coord; coord.md): a fold over the replicated log
//! of `task.*`, `lease.*`, `blocker.*` and `take.*` rows.
//!
//! [`Coord`] holds the fold behind one lock. Every mutation checks, appends
//! its rows through the store and folds them before it returns; changes go
//! to subscribers when the lock is released. coord never routes a message:
//! calls someone should hear about return [`Notice`]s for the caller to
//! send (coord.md §1.1).
//!
//! What runs on a clock is a plain call here, reading the time from
//! `Options::now` (M4 wires the timers): [`Coord::handle`] is the Follow
//! loop's body, [`Coord::tick`] its 1 s holder duties, [`Coord::next_expiry`]
//! and [`Coord::expire`] the expiry timer, [`Coord::sweep_notices`] and
//! [`Coord::sweep_asks`] the Core's sweeps.

mod asks;
mod board_search;
mod cards;
mod comment_edit;
mod comments;
mod desk;
mod desk_person;
mod desk_run;
mod error;
mod fold;
mod idle;
mod links;
mod main_red;
pub use main_red::{
    MAIN_RED_SCOPE, MainRed, main_red_scope, main_red_test, main_reds, parse_main_red,
};
mod mention;
mod stranded;
mod version_wait;
mod wait_chain;
pub use stranded::Stranded;
pub use version_wait::{
    MAX_VERSION_DEVICES, cmp_version, parse_version_wait, valid_version, version_meets,
};
pub use wait_chain::{WaitChain, task_waits, wait_chain};
mod mirror;
mod notes;
mod offers;
mod owner;
mod parts;
mod placed;
mod presence;
mod prstate;
mod threads;
pub use offers::{
    IdleReport, OFFER_HANDED, OFFER_PASSED, OFFER_WAITING, Unoffered, machine_needed,
};
pub use parts::PART_QUESTION;
pub use spawn::{
    CRASHED_QUESTION, CRASHED_START, GIVEN_NOT_NOW, GIVEN_QUESTION, GIVEN_START, GONE_USAGE_LIMIT,
    MAX_SPAWN_CRASHES, PERMISSION_MODES, SPAWN_CANCELED, SPAWN_FAILED, SPAWN_GRACE,
    SPAWN_REQUESTED, SPAWN_RETRY, Spawn, SpawnAsk, SpawnHold, Spawned, USAGE_LIMIT, crash_question,
    is_crash_question, usage_limited,
};
mod resource;
mod review;
pub mod rows;
mod said;
pub use said::{SAID_CHARS, SAID_DAYS, SAID_MAX, SAID_PER_CARD, Said};
mod short;
mod side_gate;
pub use side_gate::ASK_THE_PERSON;
mod spawn;
mod subtasks;
mod tasks;
mod types;
mod util;
mod waits;

use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant};

use gocompat::sub::Queue;
use gocompat::time::GoTime;
use store::{Event, Store, kinds};

pub use asks::{
    ASK_UNCLAIMED, ASK_WAIT, MAX_OUTCOME, OUTCOME_ANSWERED, OUTCOME_CARD, OUTCOME_EDIT,
    OUTCOME_NONE, TAKE_CLAIM, TAKE_GONE, TAKE_HANDED, TAKE_PASSED, TAKE_POSTED, TAKE_RELEASE,
    TAKE_RELEASED, TAKE_REOPEN, TAKE_REOPENED, TAKE_TAKEN, TAKE_UNCLAIMED, ask_title,
};
pub use board_search::{DEFAULT_DOC_PAGE, SHOW_COMMENT_CUT, SHOW_COMMENTS};
pub use cards::{SentBack, merge_watch, watch_text};
pub use desk::{
    DESK_LIMIT, Desk, DeskCheck, DeskHoldRow, DeskOkRow, DeskPauseRow, DeskPr, DeskPrsRow,
    DeskRunRow, MERGE_MERGE, MERGE_REBASE, MERGE_SQUASH, RepoDeskRow, github_repo, new_desk,
};
pub use desk_person::DESK_NOTE_MAX;
pub use desk_run::{DESK_NEVER, DeskWake, NOTICE_DESK, desk_notice, desk_run_text};
pub use error::{Error, ErrorKind, Result};
pub use fold::blocker_gist;
pub use fold::{
    FOLLOW_KINDS, Fold, LEFT_FINISHED, LEFT_GONE, LEFT_REMOVED, MADE_KIND, REASON_CONFLICT,
    REASON_DESK_HOLDER_CHANGED, REASON_EXPIRED, REASON_HOLDER_GONE, REASON_RELEASED, REASON_STOLEN,
    SUB_BUFFER, followed,
};
pub use links::{MAX_LINKS, link_label, parse_link, pr_link};
pub use mention::{board_form, mention_into, without_mentioned};
pub use mirror::RELEASE_REQUEST;
pub use notes::{
    NOTE_ACCEPTED, NOTE_CHANGED, NOTE_KINDS, NOTE_MAX_TEXT, NOTE_PROPOSED, NOTE_REMOVED,
    NOTES_PER_WAKE, NewNote, Note, NoteChange, NoteContext, NoteEdit, NoteEvent, NoteMatch,
    TAG_BOARD, TAG_EVERYWHERE, TAG_MACHINE, TAG_PATH, TAG_REPO, TAG_TOPIC, clean_tag,
};
pub use owner::{OWNER_ACCEPT, OWNER_DECLINE};
pub use placed::MAX_PLACE;
pub use presence::{NOTICE_PRESENCE, PRESENCE_GRACE};
pub use prstate::{
    NOTICE_PR, PR_CHANGES_REQUESTED, PR_CLOSED, PR_MERGED, PR_OPEN, PR_REVIEW_REQUIRED, PrCheck,
    PrNews, PrState,
};
pub use resource::{
    DESK_PREFIX, RESOURCE_NAMED, RESOURCE_PATH, RESOURCE_PORT, Resource, check_repo_key,
    desk_resource, is_desk, normalize_glob, normalize_resource, overlaps, parse_resource,
};
pub use review::{
    NOTICE_REVIEW, REVIEW_REQUESTED, Review, ReviewRequest, VERDICT_APPROVED, VERDICT_CHANGES,
};
pub use short::{parse_ref, parse_short, short_suffix};
pub use threads::{
    NOT_YOURS_TO_CLOSE as THREAD_NOT_YOURS, READING_LINES, TAKE_ANSWERED, agents_now_default,
    names_person,
};
pub use types::*;
/// Talk's post and channel ids are made as comments' are (card #389).
pub use util::new_id;
pub use waits::{
    MAX_WAITS, UNTIL_BLOCKED, UNTIL_CHANGED, UNTIL_DONE, UNTIL_UNBLOCKED, WAIT_CARD_GONE,
    WAIT_EXPIRED, WAIT_GONE, WAIT_TTL, until_text, wait_line,
};

use crate::roster::{AgentCard, Agents};
use fold::Sub;

pub const DEFAULT_TTL: Duration = Duration::from_secs(30 * 60);
pub const MAX_TTL: Duration = Duration::from_secs(4 * 3600);
/// A holder gone this long releases its leases.
pub const GONE_GRACE: Duration = Duration::from_secs(3 * 60);
/// A holder gone this long is taken off its cards.
pub const HOLDER_GRACE: Duration = Duration::from_secs(10 * 60);
/// A holder whose session ended (agent.gone: wound down or closed) is
/// taken off its cards this long after, so the card goes to the next free
/// agent within a minute (card #338); a quick --resume keeps it.
pub const ENDED_GRACE: Duration = Duration::from_secs(30);
/// A run holder's agent part starts with this (`<device>/run-<name>`):
/// a process such as `scripts/check.sh` that holds a slot lease without
/// being an agent (card #437). The gone sweep leaves it alone; its lease
/// ends at its until, so the run renews it while it lives.
pub const RUN_HOLDER: &str = "run-";
/// The most slots one slot lease spreads over.
pub const MAX_SLOTS: usize = 16;

/// Whether `key` is a run holder (see [`RUN_HOLDER`]).
pub fn is_run_holder(key: &str) -> bool {
    key.split_once('/')
        .is_some_and(|(dev, rest)| !dev.is_empty() && rest.starts_with(RUN_HOLDER))
}

/// The resource of `base`'s slot `n` (1-based): `<base>/slot-<n>`.
pub fn slot_resource(base: &str, n: usize) -> String {
    format!("{base}/slot-{n}")
}

pub const MAX_COMMENT: usize = 4000;
/// What the person types (a comment, an ask, a blocker answer) is held to
/// this many bytes, not to an agent's character caps (card #265): a pasted
/// log fits, and a row stays well inside the mesh's 8 MiB message cap even
/// JSON-escaped. Agents still hear it clipped, with a show pointer.
pub const MAX_PERSON_TEXT: usize = 512 << 10;
pub const MAX_ATTACHMENTS: usize = 10;
pub const MAX_REASON: usize = 500;
pub const MAX_REF: usize = 300;
pub const MAX_KEY: usize = 200;
pub const DEFAULT_DONE_CARD: i64 = 20;
pub const MAX_DONE_CARD: i64 = 200;

pub const MAX_QUESTION: usize = 200;
pub const MAX_CONTEXT: usize = 800;
/// A Left off line (card #137).
pub const MAX_LEFT_OFF: usize = 300;
/// A done check's question, its one answer, and its context when the
/// agent gave no note (card #112).
pub const DONE_QUESTION: &str = "Is this done?";
pub const DONE_ANSWER: &str = "Done";
/// Why an agent's completed without proof is refused (card #140).
pub const NEEDS_PROOF: &str = "completed needs proof: give proof with what changed, how you checked it (tests run, and what you saw in the real app), the commit or branch, and any caveats; attach screenshots of a UI change. The person reads it to decide Done";
/// How an agent's next completed resolves the done check it replaces
/// (card #112): the window doesn't count it as the agent's answer.
pub const ASKED_AGAIN: &str = "asked again: ready for the person";
pub const DONE_CONTEXT: &str =
    "The agent says the work is done. Mark it done, or say why it isn't and it goes back to them.";
pub const MAX_ANSWER: usize = 200;
pub const MAX_WHY: usize = 400;
pub const MAX_OWN_ANSWER: usize = 800;
pub const MAX_OPTIONS: usize = 6;
pub const MAX_OPEN_BLOCKERS: usize = 8;
pub const KEEP_RESOLVED: usize = 10;
pub const MAX_STEPS: usize = 20;
pub const MAX_STEP_TEXT: usize = 200;
pub const MAX_ACTIVITY: usize = 500;
pub const MAX_MISSING: usize = 6;
pub const MAX_MISSING_TEXT: usize = 200;
pub const MAX_BLOCKER_LINKS: usize = 8;
/// Rows a board coord sets aside at most.
pub const MAX_ASIDE: usize = 2048;

pub(crate) const MAX_TITLE: usize = 200;
pub const MAX_OBJECTIVE: usize = 8000;
/// A hand-off note on board_assign (card #85).
pub(crate) const MAX_HANDOFF_NOTE: usize = 2000;
pub(crate) const MAX_SCOPE: usize = 64;

/// The state row's note when a card is reopened.
pub const REOPEN_NOTE: &str = "reopened";
pub const REOPEN_COMMENT_NOTE: &str = "reopened by the person's comment";

type KeyFn = Arc<dyn Fn(&str) -> bool + Send + Sync>;
type StrFn = Arc<dyn Fn(&str) -> String + Send + Sync>;
/// [`Options::hop_of`].
pub type HopFn = Arc<dyn Fn(&str) -> i64 + Send + Sync>;
/// [`Options::busy_elsewhere`].
pub type BusyFn = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;
/// Whether a person owned the board at a row's place (`None`: now).
pub type OwnerAtFn = Arc<dyn Fn(&str, Option<&RowPos>) -> bool + Send + Sync>;

/// Whether an agent takes work Irori hands out ([`Options::offers`]).
pub type OffersFn = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// The tag of another board in a space, by id (card #392); None for an id
/// the space doesn't hold.
pub type BoardTagFn = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;
/// Every tag a space's log has named for a board inside it (card #558).
pub type BoardTagsFn = Arc<dyn Fn() -> Vec<String> + Send + Sync>;

/// See [`Options::device_version`].
pub type DeviceVersionFn = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// Configures a [`Coord`] (Go `coord.Options`).
#[derive(Clone, Default)]
pub struct Options {
    /// `None` is the system clock.
    pub now: Option<Arc<dyn Fn() -> GoTime + Send + Sync>>,
    /// Zero means [`GONE_GRACE`].
    pub gone_grace: Duration,
    /// Zero means [`HOLDER_GRACE`].
    pub holder_grace: Duration,
    /// The person's keys; `None` accepts exactly `user:<store device>` (on
    /// a board, exactly `person`).
    pub is_owner: Option<KeyFn>,
    /// A removed device: its leases end everywhere. `None` revokes nobody.
    pub revoked: Option<KeyFn>,
    /// A working card agent `key` (as this log knows it) holds in another
    /// of the Core's logs, described for a refusal (`wind#4 "title"`), or
    /// `None` (card #422). This log's own cards count by themselves; the
    /// hook adds the rest, so one card at a time holds across the private
    /// board and every shared board. Called under this coord's lock: it
    /// must never wait on a coord that may wait on this one. `None`: only
    /// this log counts.
    pub busy_elsewhere: Option<BusyFn>,
    /// Fold a shared board's log.
    pub board: bool,
    /// An agent's completed waits on the person's Done (card #112). The
    /// app turns it on; off, completed closes the card as it always did.
    pub done_check: bool,
    /// On a board, the person at this device's key.
    pub person: String,
    /// On a board, the member a `person:`/`agent:` key is.
    pub key_person: Option<StrFn>,
    /// On a board, the member a writing device is certified to.
    pub device_person: Option<StrFn>,
    /// A board's short handle, read as rows fold.
    pub tag: Option<Arc<dyn Fn() -> String + Send + Sync>>,
    /// On a space (card #392): the tag of another board in it, by id;
    /// None for an id the space doesn't hold.
    pub board_tag: Option<BoardTagFn>,
    /// On a space (card #558): whether it's a hidden one, which takes no
    /// card.
    pub no_cards: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    /// On a space (card #558): every tag its log has named for a board
    /// inside it, for a ref to a board that holds no card.
    pub board_tags: Option<BoardTagsFn>,
    /// A person who owned the board just before the row at that place,
    /// or now when there is no row (the cancel rule reads the owners at
    /// the row's time, v0.5 §5.3).
    pub board_owner: Option<OwnerAtFn>,
    /// On a board, the hop an agent's row carries (v0.5 §7.1): its running
    /// turn's hop plus one, by the actor as the board knows it. The rows
    /// in [`HOP_KINDS`] get it as `hop` when it is above zero. Called under
    /// this coord's lock and takes the inbox's, so what runs under the
    /// inbox lock must never wait on this coord ([`with_inbox_lock_held`]).
    pub hop_of: Option<HopFn>,
    /// On a board, whether this device holds an attachment (by sha256).
    /// A board row naming a file this device holds is preceded by this
    /// device's `attach.offered` for it in the board log, once (v0.5
    /// §5.5): the closure check wants the offer, and it is what lets this
    /// person's devices serve the file to the board's members. `None`
    /// offers nothing.
    pub holds_file: Option<KeyFn>,
    /// Whether this log's cards get a spawned agent each instead of an
    /// idle one (spawned agents §6, card #446): the board's switch, set in
    /// the app only. Read when the hand-out runs. `None` or false: idle
    /// agents, as before.
    pub spawning: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    /// Whether an agent Irori didn't start (a terminal agent, spawned
    /// agents §5) takes work Irori hands out: unheld cards, asks and
    /// threads (card #453). Only when the person turned on "Offer it
    /// cards" for it; everything else reaches it only when given (assign,
    /// a mention, its own board_join or ask_claim). Read when the hand-out
    /// runs. `None`: every agent does, as before.
    pub offers: Option<OffersFn>,
    /// How many of a card's agents may crash in a row before Irori stops
    /// and asks the person (card #576). Read when it's needed. `None` or
    /// below 1: [`MAX_SPAWN_CRASHES`].
    pub crash_limit: Option<Arc<dyn Fn() -> i64 + Send + Sync>>,
    /// On a board, the names of the live leases this person's agents hold
    /// on their private board (card #339). Named leases never show on a
    /// board, so a board card's external wait that names some held ones is
    /// judged on this person's devices: the sweep closes it once none of
    /// them is held. Read under this coord's lock; it must never wait on a
    /// board coord. `None`: no such waits.
    pub outside_leases: Option<Arc<dyn Fn() -> Vec<String> + Send + Sync>>,
    /// The app version a device last started with (card #485), for waits
    /// on a version: its newest `device.started` row's `version`, or "".
    /// `None` reads this coord's own log (a board's keeps none, so a board
    /// passes its person's private log).
    pub device_version: Option<DeviceVersionFn>,
}

/// The board rows that carry their writer's hop across the bridge.
pub const HOP_KINDS: &[&str] = &[
    store::kinds::TASK_COMMENT,
    store::kinds::TASK_ASSIGNED,
    store::kinds::TASK_STATUS,
    store::kinds::BLOCKER_CLOSED,
    store::kinds::BLOCKER_OPTION,
    store::kinds::BLOCKER_RESOLVED,
];

/// Whether a board row of kind carries `hop` (board asks too, §9).
pub fn carries_hop(kind: &str) -> bool {
    HOP_KINDS.contains(&kind) || kind.starts_with("board.ask.")
}

/// A board row's `hop` (0 when it has none), for the bridge.
pub fn row_hop(payload: &[u8]) -> i64 {
    serde_json::from_slice::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| v.get("hop").and_then(serde_json::Value::as_i64))
        .unwrap_or(0)
        .max(0)
}

thread_local! {
    static UNDER_INBOX_LOCK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether this thread is in [`with_inbox_lock_held`].
pub fn under_inbox_lock() -> bool {
    UNDER_INBOX_LOCK.get()
}

/// Runs an inbox hook that the inbox calls under its lock, and marks the
/// thread for that long (card #561). A board coord's write takes the inbox
/// lock under its own ([`Options::hop_of`]), so nothing there may wait on a
/// board coord: in a debug build that wait panics in [`Coord`]'s lock, so
/// any test reaching it fails rather than hanging one time in a thousand.
pub fn with_inbox_lock_held<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            UNDER_INBOX_LOCK.set(self.0);
        }
    }
    let _restore = Restore(UNDER_INBOX_LOCK.replace(true));
    f()
}

/// The lock on the fold; changes go out when it drops (Go's `unlock`).
struct Locked<'a>(MutexGuard<'a, Fold>);

impl std::ops::Deref for Locked<'_> {
    type Target = Fold;
    fn deref(&self) -> &Fold {
        &self.0
    }
}

impl std::ops::DerefMut for Locked<'_> {
    fn deref_mut(&mut self) -> &mut Fold {
        &mut self.0
    }
}

impl Drop for Locked<'_> {
    fn drop(&mut self) {
        self.0.flush();
        if let Some(f) = &self.0.on_unlock {
            f();
        }
    }
}

/// Tasks, leases, comments and asks. Safe for concurrent use.
pub struct Coord {
    /// One catch-up at a time, taken before `f`, never with it held. A pass
    /// lets go of `f` between pages (card #567), and a refold must not land
    /// between a page's read and its fold: the page may hold rows a cut moved
    /// away. It blocks, never tries: a caller relies on the rows committed
    /// before its call being folded when it returns, and a running pass may
    /// have read its vector too early to cover them.
    ///
    /// It holds the row each device's rows were last read to, unfollowed
    /// ones too, so the next pass starts past them. That is apart from the
    /// fold's `last`, the newest followed row folded: only a cut of followed
    /// rows rebuilds the fold, so a revoke that cuts a tail of unfollowed
    /// rows doesn't.
    catching: Mutex<std::collections::HashMap<String, Event>>,
    f: Mutex<Fold>,
    stale: Arc<RwLock<fold::StaleMap>>,
    spawned: Arc<RwLock<std::collections::HashSet<String>>>,
    board: bool,
}

impl Coord {
    /// Replays the store's log.
    pub fn new(st: Arc<Store>, roster: Option<Arc<dyn Agents>>, o: Options) -> Result<Coord> {
        let dev = st.device_id().to_string();
        let mut f = Fold::new(Some(st), &dev, roster, o);
        f.replay_store()?;
        Ok(Coord::wrap(f))
    }

    /// A read-only coord folded from `rows` in the order given (log order
    /// for a replay), as if this were device `device_id`. Writes fail.
    /// The replay goldens and the differential checks use it.
    pub fn from_rows(
        device_id: &str,
        rows: &[Event],
        roster: Option<Arc<dyn Agents>>,
        o: Options,
    ) -> Coord {
        let mut f = Fold::new(None, device_id, roster, o);
        f.replay_page(rows);
        f.end_replay();
        Coord::wrap(f)
    }

    /// Folds rows one at a time as they would arrive live (not a replay:
    /// short refs settle per row, links re-derive per row), with no duties
    /// run and no repeat check (Go's `impl.apply`). For checks that live
    /// and replay folds agree, and tests that hand the fold a row.
    pub fn fold_live(&self, rows: &[Event]) {
        let mut f = self.lock();
        for ev in rows {
            let last = f.last.entry(ev.device_id.clone()).or_default();
            *last = (*last).max(ev.seq);
            f.apply(ev);
        }
    }

    /// Folds rows as a catch-up does, `page` at a time, with no store: for
    /// checks that the page size changes nothing.
    #[doc(hidden)]
    pub fn fold_pages(&self, rows: &[Event], page: usize) {
        for evs in rows.chunks(page) {
            self.lock().fold_page(evs);
        }
    }

    /// How far `dev`'s rows are folded.
    #[doc(hidden)]
    pub fn folded_to(&self, dev: &str) -> i64 {
        self.lock().last.get(dev).copied().unwrap_or(0)
    }

    /// How far a catch-up has read `dev`'s rows, unfollowed ones too.
    #[doc(hidden)]
    pub fn read_to(&self, dev: &str) -> i64 {
        let read = self.catching.lock().unwrap_or_else(|e| e.into_inner());
        read.get(dev).map_or(0, |m| m.seq).max(self.folded_to(dev))
    }

    /// The board's set-aside gate alone (Go's `impl.asideGate`): `true`
    /// means the row would fold now. For tests of the cap.
    #[doc(hidden)]
    pub fn aside_gate(&self, ev: &Event) -> bool {
        self.lock().aside_gate(ev)
    }

    fn wrap(f: Fold) -> Coord {
        let stale = f.stale.clone();
        let spawned = f.spawned_keys.clone();
        let board = f.on_board;
        Coord {
            catching: Mutex::default(),
            f: Mutex::new(f),
            stale,
            spawned,
            board,
        }
    }

    fn lock(&self) -> Locked<'_> {
        debug_assert!(
            !self.board || !under_inbox_lock(),
            "waiting on a board coord under the inbox lock: a board write holds its coord and takes the inbox's (hop_of)"
        );
        Locked(self.f.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// The working card agent `key` has in hand in this log, as
    /// `tag#n "title"` (card #422): what [`Options::busy_elsewhere`] says of
    /// it from another log. Waits for the lock.
    pub fn busy_card(&self, key: &str) -> Option<String> {
        // A read under the raw guard: nothing to flush, and no unlock hook
        // runs inside the caller's lock.
        let f = self.f.lock().unwrap_or_else(|e| e.into_inner());
        f.busy_with(key, "").map(Fold::busy_text)
    }

    /// A read under the raw guard that never waits: `None` while another
    /// thread holds the lock.
    fn try_read<R>(&self, read: impl FnOnce(&Fold) -> R) -> Option<R> {
        match self.f.try_lock() {
            Ok(f) => Some(read(&f)),
            Err(std::sync::TryLockError::Poisoned(e)) => Some(read(&e.into_inner())),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }

    /// [`Coord::busy_card`] without waiting: `None` while another thread
    /// holds the lock, so a hook reading a coord that may be waiting on
    /// its own can't deadlock (the agent then counts as free here, as it
    /// did before card #422).
    pub fn try_busy_card(&self, key: &str) -> Option<String> {
        self.try_read(|f| f.busy_with(key, "").map(Fold::busy_text))
            .flatten()
    }

    /// [`Coord::get_task`] that never waits, for a caller under the inbox
    /// lock (card #561): the outer `None` means the log is busy.
    pub fn try_get_task(&self, id: &str) -> Option<Option<Task>> {
        self.try_read(|f| f.tasks.get(id).map(|t| f.shown(t)))
    }

    /// [`Coord::notes_for`] that never waits, for a caller under the inbox
    /// lock (card #561): `None` when the log is busy. With the matches, the
    /// keys among their writers and accepters that are this log's person
    /// ([`Coord::is_person`]), so telling them apart needs no second read.
    pub fn try_notes_for(
        &self,
        cx: &NoteContext,
    ) -> Option<(Vec<NoteMatch>, std::collections::HashSet<String>)> {
        self.try_read(|f| {
            let notes = f.notes_for(cx);
            let people = notes
                .iter()
                .flat_map(|m| [&m.note.by, &m.note.accepted_by])
                .filter(|k| f.is_owner(k))
                .cloned()
                .collect();
            (notes, people)
        })
    }

    /// Holds this log's lock until the guard drops, so a test can show a
    /// reader doesn't wait on it.
    #[doc(hidden)]
    pub fn hold_lock(&self) -> impl Sized + '_ {
        self.f.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---- sides (card #494) ----

    /// Pins the work/personal sides this device runs, the first the one
    /// its new cards take; empty runs every side again. Written as the
    /// person on this device, so it counts here and nowhere else.
    pub fn set_device_sides(&self, sides: &[String]) -> Result<Vec<String>> {
        let mut f = self.lock();
        if f.on_board {
            return Err(error::not_allowed("a shared space carries no sides"));
        }
        let sides = crate::sides::clean_list(sides);
        let dev = f.dev.clone();
        f.write_as(
            "user",
            crate::sides::KIND,
            &crate::sides::SidesRow {
                device_id: dev.clone(),
                sides: sides.clone(),
                at_device: dev,
            },
        )?;
        Ok(sides)
    }

    /// Folds a `device.sides` row now, before the follow loop reaches it,
    /// so a reader woken by it sees it (the fold keeps the newest per
    /// device, so folding it twice changes nothing).
    pub fn apply_sides(&self, ev: &Event) -> bool {
        let mut f = self.lock();
        let changed = !f.on_board && f.sides.apply(ev);
        if changed {
            f.index_runs();
        }
        changed
    }

    /// The sides `device` runs; None when it runs every side.
    pub fn sides_of(&self, device: &str) -> Option<Vec<String>> {
        self.lock().sides.of(device).map(<[String]>::to_vec)
    }

    /// Whether `device` runs a card on `side` (no side, or no pin: yes).
    pub fn runs_side(&self, device: &str, side: &str) -> bool {
        self.lock().sides.runs(device, side)
    }

    /// The side a new card typed on this device takes ("" for none).
    pub fn home_side(&self) -> String {
        let f = self.lock();
        f.sides.home(&f.dev)
    }

    /// Every pinned device's sides, by device id.
    pub fn device_sides(&self) -> std::collections::BTreeMap<String, Vec<String>> {
        self.lock().sides.all()
    }

    /// Every side named anywhere: on a device's pin or on a card.
    pub fn side_names(&self) -> Vec<String> {
        self.lock().side_names()
    }

    /// What an agent on `device` is told about sides (card #496); empty
    /// when it runs every side.
    pub fn side_line(&self, device: &str) -> String {
        let f = self.lock();
        crate::sides::agent_line(f.sides.of(device), &f.side_names())
    }

    // ---- tasks ----

    pub fn create_task(&self, by: &str, t: NewTask) -> Result<Task> {
        self.lock().create_task(by, t)
    }
    pub fn list_tasks(&self, f: &TaskFilter) -> Vec<Task> {
        self.lock().list_tasks(f)
    }
    pub fn get_task(&self, id: &str) -> Option<Task> {
        let f = self.lock();
        f.tasks.get(id).map(|t| f.shown(t))
    }
    /// Makes `by` the holder; ErrHeld if another live holder has it.
    pub fn claim_task(&self, by: &str, id: &str) -> Result<Task> {
        self.lock().claim_task(by, id)
    }
    /// The v0.2 assign (the creator, a holder or a person).
    pub fn assign_task(&self, by: &str, id: &str, to: &str) -> Result<Task> {
        self.lock().assign(by, id, to, "").map(|(t, _)| t)
    }
    /// Assign, and the notice for `to`.
    pub fn assign(&self, by: &str, id: &str, to: &str) -> Result<(Task, Notice)> {
        self.lock().assign_notice(by, id, to, "")
    }
    /// [`Self::assign`] with a hand-off note (card #85).
    pub fn hand_off(&self, by: &str, id: &str, to: &str, note: &str) -> Result<(Task, Notice)> {
        self.lock().assign_notice(by, id, to, note)
    }
    pub fn set_task_state(&self, by: &str, id: &str, s: &str, note: &str) -> Result<Task> {
        self.lock().set_task_state(by, id, s, note)
    }
    /// [`Self::set_task_state`] with proof (card #140): an agent's
    /// completed puts `files` on its done check beside the note, and with
    /// `need_proof` refuses ([`NEEDS_PROOF`]) a check with no note, after
    /// every other refusal. The person's own completed needs none.
    pub fn set_task_state_with(
        &self,
        by: &str,
        id: &str,
        s: &str,
        note: &str,
        files: Vec<FileRef>,
        need_proof: bool,
    ) -> Result<Task> {
        self.lock()
            .set_task_state_with(by, id, s, note, files, need_proof)
    }
    /// Whether an agent's completed waits on the person's Done (card #112):
    /// then it must bring proof (card #140).
    pub fn done_check(&self) -> bool {
        self.lock().done_check
    }
    pub fn reopen(&self, by: &str, id: &str) -> Result<(Task, Notice)> {
        self.lock().reopen(by, id, REOPEN_NOTE)
    }
    /// `by` puts its new unheld card `id` up for a free agent (card #82):
    /// Irori hands it to the one in `agents` idle longest, or it waits for
    /// the next to go idle. The notice is for that agent; its `to` is
    /// empty when nobody is free.
    pub fn offer_card(&self, by: &str, id: &str, agents: &[AgentCard]) -> Result<Notice> {
        self.lock().offer_card(by, id, agents)
    }
    /// How many unheld cards this device hands out have nobody among
    /// `agents` free to take them (card #337).
    pub fn unserved_offers(&self, agents: &[AgentCard]) -> usize {
        self.lock().unserved_offers(agents)
    }
    pub fn join(&self, by: &str, id: &str, who: &str) -> Result<Task> {
        self.lock().join(by, id, who)
    }
    pub fn leave(&self, by: &str, id: &str, who: &str) -> Result<Task> {
        self.lock().leave(by, id, who)
    }
    /// Takes board member `person_id`'s keys off every open card (v0.5
    /// §4.3): the remover's cleanup.
    pub fn take_off_person(&self, by: &str, person_id: &str) -> Result<usize> {
        self.lock().take_off_person(by, person_id)
    }
    pub fn set_shared(&self, by: &str, id: &str, shared: bool) -> Result<Task> {
        self.lock().set_shared(by, id, shared)
    }
    pub fn block(&self, by: &str, id: &str, reason: &str, bb: Option<BlockedBy>) -> Result<Task> {
        let mut f = self.lock();
        let t = f.block(by, id, reason, bb)?;
        for b in t.open_blockers() {
            f.note_outside_wait(&t.id, &b);
        }
        Ok(t)
    }
    /// Finds or makes the one open card for main failing `test` (card
    /// #436), reported from card `from`; true when it was made now, so the
    /// caller hands it out. Block `from` on it with a task wait.
    pub fn main_red(&self, by: &str, from: &str, test: &str, commit: &str) -> Result<(Task, bool)> {
        self.lock().main_red(by, from, test, commit)
    }
    pub fn unblock(&self, by: &str, id: &str, note: &str) -> Result<(Task, Notice)> {
        self.lock().unblock(by, id, note)
    }
    /// Takes a card back off a shared board.
    pub fn withdraw(&self, by: &str, id: &str, note: &str) -> Result<Task> {
        self.lock().withdraw(by, id, note)
    }
    /// Whether this device may withdraw card `id` (its creator's person or
    /// a board owner), the rule [`Self::withdraw`] applies; checked for a
    /// whole family before any of it is withdrawn (card #180).
    pub fn may_withdraw(&self, id: &str) -> bool {
        let f = self.lock();
        f.tasks
            .get(id)
            .is_some_and(|t| f.may_cancel(t, &f.dev, None))
    }
    pub fn split(
        &self,
        by: &str,
        id: &str,
        subs: Vec<NewSubtask>,
        adopt: Vec<String>,
    ) -> Result<(Task, Vec<Task>)> {
        self.lock().split(by, id, subs, adopt)
    }
    /// Resolves `#12`, `12b`, `task:#12`, `mlg#12` or a full id.
    pub fn task_ref(&self, r: &str) -> Result<String> {
        self.lock().task_ref_locked(r)
    }
    pub fn is_person(&self, key: &str) -> bool {
        self.lock().is_owner(key)
    }
    /// Whether `key` is a person on this log, not an agent: on a board,
    /// any member's person actor (card #134).
    pub fn is_any_person(&self, key: &str) -> bool {
        self.lock().is_person(key)
    }
    /// Who hears a person's follow-up in thread `id` now (card #134): the
    /// agents that answered, else the ones it went to. Empty when `id`
    /// isn't an open thread.
    pub fn thread_to(&self, id: &str) -> Vec<String> {
        let f = self.lock();
        match f.tasks.get(id) {
            Some(t) if t.open_ask().is_some_and(|a| a.thread) => f.thread_to(t),
            _ => Vec::new(),
        }
    }
    pub fn ignored_rows(&self) -> usize {
        self.lock().ignored
    }
    pub fn ignored_by(&self) -> std::collections::BTreeMap<String, usize> {
        self.lock().ignored_by.clone()
    }
    pub fn aside_dropped(&self) -> usize {
        self.lock().aside_drop
    }
    /// What a board's fold kept out, for `GET /v1/boards?detail=1`.
    pub fn set_aside(&self) -> SetAside {
        let f = self.lock();
        SetAside {
            unresolved: f.aside.len(),
            dropped: f.aside_drop,
            ignored: f.ignored,
            ignored_by: f.ignored_by.clone(),
        }
    }
    pub fn unresolved(&self) -> usize {
        self.lock().aside.len()
    }
    /// The `person:` key of whoever wrote a row from device `dev`, when
    /// it isn't the `by` the row claims (card #573): on a shared board a
    /// row names whoever its writer likes. None when they agree, for
    /// Irori's rows from this person's devices, or off a board.
    pub fn written_as(&self, by: &str, dev: &str) -> Option<String> {
        self.lock().written_as(by, dev)
    }
    pub fn is_board(&self) -> bool {
        self.board
    }

    // ---- cards ----

    pub fn raise(&self, by: &str, id: &str, nb: NewBlocker) -> Result<(Task, Blocker)> {
        let mut f = self.lock();
        let (t, b) = f.raise(by, id, nb)?;
        f.note_outside_wait(&t.id, &b);
        Ok((t, b))
    }
    /// Revises an open blocker (Fold::revise).
    pub fn revise(
        &self,
        by: &str,
        id: &str,
        blocker: &str,
        r: BlockerRevision,
    ) -> Result<(Task, Blocker)> {
        self.lock().revise(by, id, blocker, r)
    }
    pub fn recommend(
        &self,
        by: &str,
        id: &str,
        blocker: &str,
        answer: &str,
        why: &str,
    ) -> Result<Task> {
        self.lock().recommend(by, id, blocker, answer, why)
    }
    pub fn answer(
        &self,
        by: &str,
        id: &str,
        blocker: &str,
        a: BlockerAnswer,
    ) -> Result<(Task, Vec<Notice>)> {
        self.lock().answer(by, id, blocker, a)
    }
    /// On a shared board, the person offers card `id` to person `to` when
    /// they own it, or asks its owner for it when they don't (card #432):
    /// an Accept/Decline item for the other side, answered with
    /// [`Coord::answer`]. The card moves only on Accept.
    pub fn transfer_owner(&self, by: &str, id: &str, to: &str) -> Result<(Task, Blocker)> {
        self.lock().transfer_owner(by, id, to)
    }

    /// The person's Done on done check `blocker` (card #315): the card is
    /// completed in one step, and `[0]` tells its holders so.
    pub fn say_done(
        &self,
        by: &str,
        id: &str,
        blocker: &str,
        mentions: &[Mention],
    ) -> Result<(Task, Vec<Notice>)> {
        self.lock().say_done(by, id, blocker, mentions)
    }
    /// An agent's comment or blocker reply. `[0]` is the comment's own
    /// notice; a `mentioned` notice follows when the input mentions anyone.
    pub fn comment_on(
        &self,
        by: &str,
        id: &str,
        input: CommentInput,
    ) -> Result<(Comment, Vec<Notice>)> {
        self.lock().comment_on(by, id, input)
    }
    /// `by` makes `who` watchers of card `id` (a mention's watchers).
    pub fn watch_for(&self, by: &str, id: &str, who: &[String]) -> Result<Task> {
        self.lock().watch_for(by, id, who)
    }
    /// Mentions on card `id` that nothing stores (a blocker's question, a
    /// resolve note): the live ones become watchers, and the notices that
    /// wake them come back (none: nobody to tell).
    pub fn mention(
        &self,
        by: &str,
        id: &str,
        mentions: &[Mention],
        text: &str,
    ) -> Result<Vec<Notice>> {
        let by = util::caller(by)?;
        let mut f = self.lock();
        let ms = f.check_mentions(&by, mentions)?;
        f.mention_notice(&by, id, &ms, text, &format!("task:{id}"), "")
    }
    /// Renames a card or changes its objective or done-when (board_edit).
    pub fn edit_task(&self, by: &str, id: &str, e: TaskEdit) -> Result<Task> {
        self.lock().edit_task(by, id, e)
    }
    /// [`Self::set_steps`] with a notice for each agent newly given a step
    /// (card #86).
    pub fn set_steps_notice(
        &self,
        by: &str,
        id: &str,
        steps: Vec<Step>,
    ) -> Result<(Task, Vec<Notice>)> {
        self.lock().set_steps_notice(by, id, steps)
    }
    /// Gives a step to an agent ("" takes it off anyone) (card #86).
    pub fn set_step_owner(
        &self,
        by: &str,
        id: &str,
        step: &str,
        owner: &str,
    ) -> Result<(Task, Vec<Notice>)> {
        self.lock().set_step_owner(by, id, step, owner)
    }
    pub fn set_steps(&self, by: &str, id: &str, steps: Vec<Step>) -> Result<Task> {
        self.lock().set_steps(by, id, steps)
    }
    pub fn set_step(&self, by: &str, id: &str, step: &str, u: StepUpdate) -> Result<Task> {
        self.lock().set_step(by, id, step, u, "")
    }
    /// set_step with the holder's one line on what's done and what's next
    /// (card #137), shown on the card as "Left off".
    pub fn set_step_left_off(
        &self,
        by: &str,
        id: &str,
        step: &str,
        u: StepUpdate,
        left_off: &str,
    ) -> Result<Task> {
        self.lock().set_step(by, id, step, u, left_off)
    }
    /// Where the holder's work on card id is (card #137); true when it
    /// wrote a row (the place changed). Private board only.
    pub fn set_where(&self, id: &str, w: Where) -> Result<bool> {
        let mut f = self.lock();
        if f.on_board {
            return Ok(false);
        }
        f.set_where(id, w)
    }
    pub fn watch(&self, by: &str, id: &str) -> Result<Task> {
        self.lock().watch(by, id, true)
    }
    pub fn unwatch(&self, by: &str, id: &str) -> Result<Task> {
        self.lock().watch(by, id, false)
    }
    /// Records where a card was dropped on the Board's canvas (card #93).
    pub fn place(&self, by: &str, id: &str, x: i64, y: i64) -> Result<()> {
        self.lock().place(by, id, x, y)
    }
    /// Every placed card's canvas position, by card id.
    pub fn positions(&self) -> std::collections::HashMap<String, (i64, i64)> {
        self.lock().positions()
    }
    /// Registers a wait (card #87): `by` is woken once when it fires.
    pub fn create_wait(&self, by: &str, spec: WaitSpec) -> Result<Wait> {
        self.lock().create_wait(by, spec)
    }
    /// Cancels one of `by`'s open waits.
    pub fn cancel_wait(&self, by: &str, id: &str) -> Result<Wait> {
        self.lock().cancel_wait(by, id)
    }
    /// Open waits, oldest first; `by` empty for everyone's.
    pub fn waits(&self, by: &str) -> Vec<Wait> {
        self.lock().waits_of(by)
    }
    /// Open branch waits of this device's agents, for the core to check.
    pub fn branch_waits(&self) -> Vec<Wait> {
        self.lock().branch_waits()
    }
    /// Queues a wait to fire with `what` at the next sweep (a branch moved).
    pub fn fire_wait(&self, id: &str, what: &str) {
        self.lock().fire_wait(id, what)
    }
    /// Open waits on shared boards' cards with their waiter on this device
    /// (card #574).
    pub fn board_waits(&self) -> Vec<Wait> {
        self.lock().board_waits()
    }
    /// See [`Fold::board_wait_moved`].
    pub fn board_wait_moved(&self, id: &str, cur: &str) -> Option<String> {
        self.lock().board_wait_moved(id, cur)
    }
    /// See [`Fold::board_wait_lost`].
    pub fn board_wait_lost(&self, id: &str) {
        self.lock().board_wait_lost(id)
    }
    /// What a wait says happened to card `id` of this log, for a wait kept
    /// in another log.
    pub fn wait_what(&self, id: &str, was: &str, now: &str) -> String {
        self.lock().wait_what(id, was, now)
    }
    /// A task's newest activity lines, newest first (`None`: no such task).
    pub fn activity(&self, id: &str) -> Option<Vec<Activity>> {
        self.lock().activity_of(id)
    }
    /// The person deletes a card (card #57); returns it as it was.
    pub fn delete_task(&self, by: &str, id: &str) -> Result<Task> {
        self.lock().delete_task(by, id)
    }
    /// The person clears done cards from the window, or brings them back
    /// (card #138); returns how many changed.
    pub fn archive(&self, by: &str, ids: &[String], archived: bool) -> Result<usize> {
        self.lock().archive(by, ids, archived)
    }
    /// The cards the person cleared (card #138).
    pub fn archived_ids(&self) -> std::collections::HashSet<String> {
        self.lock().archived_ids()
    }
    /// The person pauses a card or resumes it (card #207); returns the
    /// card and whether anything changed.
    pub fn pause(&self, by: &str, id: &str, paused: bool, note: &str) -> Result<(Task, bool)> {
        self.lock().pause(by, id, paused, note)
    }
    /// The cards the person paused (card #207).
    pub fn paused_ids(&self) -> std::collections::HashSet<String> {
        self.lock().paused_ids()
    }
    pub fn pin(&self, by: &str, id: &str, pinned: bool) -> Result<Task> {
        self.lock().pin(by, id, pinned, "", false)
    }
    pub fn move_pin(&self, by: &str, id: &str, before: &str) -> Result<Task> {
        self.lock().pin(by, id, true, before, true)
    }
    /// Pins (or unpins, or moves with `before`) a shared board's card `id`
    /// in the person's own log (card #161); `mv` moves a pinned one.
    pub fn pin_ref(&self, by: &str, id: &str, pinned: bool, before: &str, mv: bool) -> Result<()> {
        self.lock().pin_ref(by, id, pinned, before, mv)
    }
    /// Every pinned card id in this log with its rank, including board
    /// cards pinned with [`Self::pin_ref`].
    pub fn pinned_ranks(&self) -> std::collections::HashMap<String, f64> {
        self.lock().pinned_ranks()
    }

    // ---- comments ----

    pub fn add_comment(
        &self,
        by: &str,
        id: &str,
        text: &str,
        wake: bool,
    ) -> Result<(Comment, Notice)> {
        self.lock().add_comment(by, id, text, wake, Vec::new())
    }
    pub fn add_comment_with(
        &self,
        by: &str,
        id: &str,
        text: &str,
        wake: bool,
        files: Vec<FileRef>,
    ) -> Result<(Comment, Notice)> {
        self.lock().add_comment(by, id, text, wake, files)
    }
    /// The person's comment from the app. `around` runs under coord's lock.
    pub fn person_comment(
        &self,
        by: &str,
        id: &str,
        text: &str,
        wake: bool,
        mentions: &[Mention],
        around: Option<&dyn Fn(&str) -> bool>,
    ) -> Result<(Comment, Vec<Notice>)> {
        self.lock()
            .person_comment(by, id, text, wake, mentions, Vec::new(), around)
    }
    /// person_comment with files on the comment (card #124): the notices
    /// about it carry them, for attachment_get.
    #[allow(clippy::too_many_arguments)]
    pub fn person_comment_files(
        &self,
        by: &str,
        id: &str,
        text: &str,
        wake: bool,
        mentions: &[Mention],
        files: Vec<FileRef>,
        around: Option<&dyn Fn(&str) -> bool>,
    ) -> Result<(Comment, Vec<Notice>)> {
        self.lock()
            .person_comment(by, id, text, wake, mentions, files, around)
    }
    /// The person's new text for their comment on a private card, before
    /// any agent read it (card #105). The caller holds its deliveries.
    pub fn edit_comment(&self, by: &str, comment_id: &str, text: &str) -> Result<Comment> {
        self.lock().edit_comment(by, comment_id, text)
    }
    /// Whether the person may edit this comment (theirs, on a private card).
    pub fn comment_editable(&self, by: &str, comment_id: &str) -> Result<Comment> {
        self.lock().comment_editable(by, comment_id)
    }
    pub fn record_message(&self, m: MessageEntry) -> Result<Comment> {
        self.lock().record_message(m)
    }
    /// What the person said lately on the cards around `id` (card #555).
    pub fn said_near(&self, id: &str) -> Vec<Said> {
        self.lock().said_near(id)
    }
    /// The card to tell `key` about on a wake and what the person said
    /// around it (card #555): the first of `refs` that is its card, or
    /// with `working` the card it works now.
    pub fn said_for(
        &self,
        key: &str,
        refs: &[String],
        working: bool,
    ) -> Option<(String, Vec<Said>)> {
        self.lock().said_for(key, refs, working)
    }
    /// [`Self::said_for`] that never waits: `None` when the log is busy.
    /// For a caller under the inbox lock: a board coord's write takes the
    /// inbox's lock under its own (`hop_of`), so waiting here could
    /// deadlock.
    pub fn try_said_for(
        &self,
        key: &str,
        refs: &[String],
        working: bool,
    ) -> Option<Option<(String, Vec<Said>)>> {
        self.try_read(|f| f.said_for(key, refs, working))
    }
    pub fn comments(&self, id: &str) -> Option<Vec<Comment>> {
        self.lock().list_comments(id)
    }
    /// The done check the person sent back on card `id` (card #476), if
    /// its newest done check ends on their Not done.
    pub fn sent_back(&self, id: &str) -> Option<SentBack> {
        let f = self.lock();
        let t = f.tasks.get(id)?;
        cards::sent_back(&f, t)
    }
    pub fn get_comment(&self, id: &str) -> Option<Comment> {
        self.lock().comments.get(id).cloned()
    }
    pub fn comment_attachment(&self, sha: &str) -> Option<FileRef> {
        self.lock().comment_attachment(sha)
    }

    // ---- links, leases ----

    pub fn add_link(&self, by: &str, id: &str, link: &str) -> Result<Task> {
        self.lock().add_link(by, id, link)
    }
    pub fn remove_link(&self, by: &str, id: &str, r: &str) -> Result<Task> {
        self.lock().remove_link(by, id, r)
    }
    /// Takes or renews a lease; ttl zero is DEFAULT_TTL, capped at MAX_TTL.
    pub fn acquire(
        &self,
        holder: &str,
        resource: &str,
        ttl: Duration,
        steal: bool,
    ) -> Result<Lease> {
        self.lock().acquire(holder, resource, ttl, steal)
    }
    pub fn release(&self, by: &str, resource: &str) -> Result<()> {
        self.lock().release(by, resource)
    }
    /// Takes one of `slots` leases on `base` (`<base>/slot-1`..`-N`, see
    /// [`slot_resource`]): the one holder already has (renewed), else the
    /// first free one. All held by others is Held, naming their holders,
    /// and writes no refusal row, so a run waiting its turn can poll it.
    pub fn acquire_slot(
        &self,
        holder: &str,
        base: &str,
        slots: usize,
        ttl: Duration,
    ) -> Result<Lease> {
        self.lock().acquire_slot(holder, base, slots, ttl)
    }
    /// Releases every slot of `base` that `by` holds; false when it held none.
    pub fn release_slot(&self, by: &str, base: &str) -> Result<bool> {
        self.lock().release_slot(by, base)
    }
    /// The person ending a lease held on any device.
    pub fn steal_lease(&self, by: &str, resource: &str) -> Result<()> {
        self.lock().steal_lease(by, resource)
    }
    pub fn leases(&self) -> Vec<Lease> {
        self.lock().leases()
    }
    /// The shared cards on this board that make card's lease on resource
    /// mirror here (v0.5 §5.4); `all` is the Autonomous preset's "every
    /// lease in shared tasks' repos".
    pub fn mirror_tasks(&self, card: &str, resource: &str, all: bool) -> Vec<String> {
        self.lock().mirror_tasks(card, resource, all)
    }
    /// Mirrors card's lease into this board when a shared card makes it
    /// mirror (`None`: nothing does). Another person's live lease refuses
    /// it, and the private lease must be refused with it.
    pub fn mirror_acquire(
        &self,
        card: &str,
        resource: &str,
        ttl: Duration,
        all: bool,
        steal: bool,
    ) -> Result<Option<Lease>> {
        self.lock().mirror_acquire(card, resource, ttl, all, steal)
    }
    /// Mirrors the release of card's lease; false when none is here.
    pub fn mirror_release(&self, card: &str, resource: &str) -> Result<bool> {
        self.lock().mirror_release(card, resource)
    }
    /// Asks another person's agents to review a PR the card links: `to`,
    /// or their dedicated reviewers (v0.5 §10), one request each. The
    /// notice goes to them.
    pub fn request_review(
        &self,
        by: &str,
        id: &str,
        pr: &str,
        to: &str,
    ) -> Result<(Vec<ReviewRequest>, Notice)> {
        self.lock().request_review(by, id, pr, to)
    }
    /// The person dedicating one of their agents as reviewer for the board
    /// or one repo (empty reviewer: none).
    pub fn set_reviewer(&self, by: &str, reviewer: &str, repo: &str) -> Result<()> {
        self.lock().set_reviewer(by, reviewer, repo)
    }
    // ---- notes (card #401) ----

    /// Writes a new note. A person's is accepted at once; an agent's waits
    /// for a person's accept.
    pub fn propose_note(&self, by: &str, n: NewNote) -> Result<Note> {
        self.lock().propose_note(by, n)
    }
    /// A person accepts a proposed note, or an agent's pending edit or
    /// removal of one.
    pub fn accept_note(&self, by: &str, id: &str) -> Result<Note> {
        self.lock().accept_note(by, id)
    }
    /// Edits a note: a person's applies at once, an agent's on a live note
    /// waits for an accept.
    pub fn edit_note(&self, by: &str, id: &str, e: NoteEdit, why: &str) -> Result<Note> {
        self.lock().edit_note(by, id, e, why)
    }
    /// Removes a note: a person's removes it, an agent's on a live note
    /// waits for an accept.
    pub fn remove_note(&self, by: &str, id: &str, why: &str) -> Result<Note> {
        self.lock().remove_note(by, id, why)
    }
    /// Every note, newest first; removed ones only when asked.
    pub fn notes(&self, removed: bool) -> Vec<Note> {
        self.lock().list_notes(removed)
    }
    pub fn note_by_id(&self, id: &str) -> Option<Note> {
        self.lock().notes.get(id).map(|(n, _)| n.clone())
    }
    /// Notes holding every word of q; `live_only` for what agents read.
    pub fn search_notes(&self, q: &str, live_only: bool) -> Vec<Note> {
        self.lock().search_notes(q, live_only)
    }
    /// The accepted notes for a wake where `cx` says the agent works.
    pub fn notes_for(&self, cx: &NoteContext) -> Vec<NoteMatch> {
        self.lock().notes_for(cx)
    }

    /// A card's review requests, oldest first.
    pub fn reviews(&self, id: &str) -> Vec<ReviewRequest> {
        self.lock().reviews.get(id).cloned().unwrap_or_default()
    }
    /// A card's reviews, one per reviewer, with each one's state (#88).
    pub fn reviews_of(&self, id: &str) -> Vec<Review> {
        self.lock().reviews_of(id)
    }
    /// A member's dedicated reviewer for repo, else for the board.
    pub fn reviewer(&self, person: &str, repo: &str) -> String {
        self.lock().reviewer_of(person, repo)
    }
    /// Every reviewer a member dedicated, as `(repoKey or "", agent)`,
    /// by repo; a cleared one is left out.
    pub fn reviewers_of(&self, person: &str) -> Vec<(String, String)> {
        self.lock()
            .reviewers
            .iter()
            .filter(|((p, _), (_, a))| p == person && !a.is_empty())
            .map(|((_, r), (_, a))| (r.clone(), a.clone()))
            .collect()
    }
    /// Asks the holder of another person's lease to release it, on the
    /// shared card it mirrors through, in place of a steal.
    pub fn request_release(
        &self,
        by: &str,
        resource: &str,
        why: &str,
    ) -> Result<(Comment, Notice)> {
        let mut f = self.lock();
        let now = f.stamp();
        f.request_release(by, resource, why, &now)
    }
    /// Active refusals, newest first.
    pub fn refusals(&self) -> Vec<Refusal> {
        let mut f = self.lock();
        let now = f.stamp();
        f.prune(&now);
        f.refusals_list()
    }
    pub fn board(&self, filter: &BoardFilter) -> BoardView {
        self.lock().board(filter)
    }
    /// SweepNotices with the notices dropped.
    pub fn sweep(&self) -> Result<()> {
        self.sweep_notices().map(|_| ())
    }
    /// Releases gone holders' leases, returns cards whose holder went away,
    /// closes blockers that resolved by themselves, and drains the watch
    /// digests. Core calls it every few seconds and routes the notices.
    pub fn sweep_notices(&self) -> Result<Vec<Notice>> {
        let (out, r) = self.sweep_notices_partial();
        r.map(|_| out)
    }

    /// `sweep_notices` with what was built before a failed write kept:
    /// Core routes those notices (their rows are in the log) and reports
    /// the error, as Go's `SweepNotices` returns both.
    pub fn sweep_notices_partial(&self) -> (Vec<Notice>, Result<()>) {
        self.lock().sweep_notices()
    }

    // ---- asks ----

    /// The agents a thread with this text would go to first: the holders
    /// of the open cards it names (#12), then the mentioned ones.
    pub fn thread_hearers(&self, text: &str, mentions: &[Mention]) -> Vec<String> {
        self.lock().thread_hearers(text, mentions)
    }

    pub fn post_ask(
        &self,
        by: &str,
        input: NewAsk,
        agents: &[AgentCard],
    ) -> Result<(Task, Notice)> {
        self.lock().post_ask(by, input, agents)
    }
    /// Asks for an ask's take and waits up to `wait` for the owner's answer.
    pub fn ask_claim(&self, by: &str, id: &str, wait: Duration) -> Result<ClaimResult> {
        let by = util::caller(by)?;
        let sub = self.subscribe();
        let req = {
            let mut f = self.lock();
            if f.is_owner(&by) || by == util::SWEEPER {
                return Err(error::invalid("an agent takes an ask"));
            }
            match f.ask_claim_start(&by, id)? {
                Ok(done) => return Ok(done),
                Err(i) => i,
            }
        };
        let deadline = Instant::now() + wait;
        loop {
            if let Some(r) = self.lock().ask_claim_poll(&by, id, req, false) {
                return r;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || !matches!(sub.recv_timeout(left), Ok(Some(_))) {
                break;
            }
        }
        self.lock()
            .ask_claim_poll(&by, id, req, true)
            .unwrap_or_else(|| Err(error::not_found(format!("task {id}"))))
    }
    pub fn ask_close(&self, by: &str, id: &str, o: AskOutcome) -> Result<Task> {
        self.lock().ask_close(by, id, o)
    }
    /// The person resolves their thread (card #116): it closes, answered.
    pub fn resolve_thread(&self, by: &str, id: &str) -> Result<Task> {
        self.lock().resolve_thread(by, id)
    }
    /// A person turns an open thread's "Agents answer now" on or off (card
    /// #391).
    pub fn set_agents_now(&self, by: &str, id: &str, on: bool) -> Result<Task> {
        self.lock().set_agents_now(by, id, on)
    }
    /// The person reopens their resolved thread (card #169): live again,
    /// their next reply going to the agents that answered it.
    pub fn reopen_thread(&self, by: &str, id: &str) -> Result<Task> {
        self.lock().reopen_thread(by, id)
    }
    pub fn ask_again(&self, by: &str, id: &str) -> Result<()> {
        self.lock().ask_again(by, id)
    }
    pub fn sweep_asks(&self, agents: &[AgentCard]) -> Result<Vec<Notice>> {
        let (out, r) = self.sweep_asks_partial(agents);
        r.map(|_| out)
    }

    /// `sweep_asks` with the notices told before a failed write kept, as
    /// Go's `SweepAsks` returns both.
    /// It hands on the unheld cards this device's agents posted too
    /// (card #82).
    pub fn sweep_asks_partial(&self, agents: &[AgentCard]) -> (Vec<Notice>, Result<()>) {
        let mut f = self.lock();
        let (mut out, r) = f.sweep_asks(agents);
        if r.is_err() {
            return (out, r);
        }
        let (more, r) = f.sweep_offers(agents);
        out.extend(more);
        (out, r)
    }
    /// The presence notices due now (card #372): an agent this device
    /// hosts (`hosted`) hears once when a fellow holder of an open card,
    /// or an agent its card waits on, has had its device offline for
    /// [`PRESENCE_GRACE`], and once more when it's back. Writes nothing.
    pub fn sweep_presence(
        &self,
        agents: &[AgentCard],
        hosted: &dyn Fn(&str) -> bool,
    ) -> Vec<Notice> {
        let mut f = self.lock();
        let now = f.stamp();
        f.sweep_presence(agents, &now, hosted)
    }

    /// [`Coord::sweep_presence`] at a given time (tests).
    pub fn sweep_presence_at(
        &self,
        agents: &[AgentCard],
        now: &GoTime,
        hosted: &dyn Fn(&str) -> bool,
    ) -> Vec<Notice> {
        self.lock().sweep_presence(agents, now, hosted)
    }

    /// Idle agents and the unheld cards nobody has been handed yet, with
    /// why (card #341): Home's "N agents idle, no open work" line.
    pub fn idle_report(&self, agents: &[AgentCard]) -> IdleReport {
        self.lock().idle_report(agents)
    }

    // ---- spawned agents (card #446) ----

    /// The card agent `key` was started for, if Irori spawned it; still
    /// there after the agent goes (card #452).
    pub fn spawned_card(&self, key: &str) -> Option<String> {
        self.lock().spawned_for(key).map(|s| s.task_id.clone())
    }
    /// Whether this log's cards get a spawned agent each (its spawning
    /// switch, card #446).
    pub fn spawning(&self) -> bool {
        self.lock().spawning_on()
    }

    /// Whether Irori started agent `key` for a card. Never takes coord's
    /// lock, so the inbox may ask under its own.
    pub fn is_spawned(&self, key: &str) -> bool {
        self.spawned
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains(key)
    }
    /// Irori ended agent `key` at `at` (it wrote its `agent.gone`): it
    /// comes off its cards after [`ENDED_GRACE`], not the holder grace.
    pub fn mark_ended(&self, key: &str, at: GoTime) {
        let mut f = self.lock();
        f.gone_rows.insert(key.to_string(), at);
        f.ended_by_irori.insert(key.to_string());
    }
    /// Irori ended agent `key` at `at` on a Claude usage limit (card #450):
    /// as [`Self::mark_ended`], and an ask it served doesn't wait for the
    /// person before its next agent.
    pub fn mark_usage_limited(&self, key: &str, at: GoTime) {
        let mut f = self.lock();
        f.gone_rows.insert(key.to_string(), at);
        f.ended_by_irori.insert(key.to_string());
        f.usage_limited.insert(key.to_string());
    }
    /// The crash limit in force (card #576).
    pub fn crash_limit(&self) -> u32 {
        self.lock().crash_limit()
    }
    /// Asks the person about card `id` if its crashes reached the limit,
    /// it doesn't ask yet and this device hands it out: the limit was
    /// lowered under it, or its agent crashed on another device (card
    /// #576).
    pub fn stop_crashed(&self, id: &str) -> Result<()> {
        let mut f = self.lock();
        let now = f.stamp();
        f.stop_crashed_spawns(id, &now)
    }
    /// The spawned_for marks written at or after `since`, oldest first
    /// (card #450).
    pub fn spawned_since(&self, since: &GoTime) -> Vec<(String, Spawned)> {
        self.lock().spawned_since(since)
    }
    /// Whether Irori ended agent `key` (its own `agent.gone` "finished",
    /// replayed from the log): it is never ended a second time (card #474).
    pub fn ended_by_irori(&self, key: &str) -> bool {
        self.lock().ended_by_irori.contains(key)
    }
    /// Puts holderless card `id` back on the Board for an agent (card
    /// #447's transition): Ok(false) when it is held, assigned, closed,
    /// working, an open ask or a parent.
    pub fn back_on_board(&self, by: &str, id: &str, why: &str) -> Result<bool> {
        self.lock().back_on_board(by, id, why)
    }
    /// Whether agent `key` is gone (card #485): its session ended, or the
    /// roster says gone or no longer lists it.
    pub fn key_gone(&self, key: &str) -> bool {
        self.lock().key_gone(key)
    }
    /// Card `id`'s open items whose agent is gone while nobody holds it
    /// (card #485).
    pub fn stranded(&self, id: &str) -> Vec<Stranded> {
        let f = self.lock();
        f.tasks.get(id).map(|t| f.stranded(t)).unwrap_or_default()
    }
    /// Card `id`'s stranded waits that only Needs you shows (card #520):
    /// waits on a gone agent, and outside waits (not a PR's merge) whose
    /// gone agent raised them, while nobody holds it.
    pub fn gone_waits(&self, id: &str) -> Vec<Stranded> {
        let f = self.lock();
        f.tasks.get(id).map(|t| f.gone_waits(t)).unwrap_or_default()
    }
    /// Why "Hand to an agent" can't move card `id` (a shared board's card,
    /// an open ask, subtasks, already on the Board); "" when it can.
    pub fn hand_on_refusal(&self, id: &str) -> String {
        let f = self.lock();
        f.tasks
            .get(id)
            .map(|t| f.hand_on_refusal(t).to_string())
            .unwrap_or_default()
    }
    /// "Hand to an agent" (card #485): the person puts a card whose items
    /// wait on gone agents back on the Board, taking those agents off it.
    /// Returns the card as it was and those items.
    pub fn hand_on(&self, by: &str, id: &str) -> Result<(Task, Vec<Stranded>)> {
        self.lock().hand_on(by, id)
    }
    /// Whether this device runs card `id`'s hand-out (it made the card):
    /// only then does its own sweep start or offer the card's agent.
    pub fn hands_out_here(&self, id: &str) -> bool {
        let f = self.lock();
        f.tasks.get(id).is_some_and(|t| f.hands_out(t))
    }
    /// The spawned_for mark on agent `key`.
    pub fn spawned(&self, key: &str) -> Option<Spawned> {
        self.lock().spawned_for(key).cloned()
    }
    /// Writes card `a.task_id`'s next spawn request; Ok(None) when one is
    /// pending already or the card no longer needs an agent.
    pub fn request_spawn(&self, by: &str, a: SpawnAsk) -> Result<Option<i64>> {
        self.lock().request_spawn(by, a)
    }
    /// The person asked for card `id`'s agent now: a failed spawn is asked
    /// for again at the next sweep, not after [`SPAWN_RETRY`].
    pub fn spawn_now(&self, id: &str) {
        self.lock().spawn_now.insert(id.to_string());
    }
    /// Whether the person asked for card `id`'s agent now and no request
    /// has been written since ([`Self::spawn_now`]).
    pub fn spawn_asked_now(&self, id: &str) -> bool {
        self.lock().spawn_now.contains(id)
    }
    /// Ends request `generation` of card `id` (its own part) as failed or
    /// canceled with `reason`.
    pub fn end_spawn(
        &self,
        by: &str,
        id: &str,
        generation: i64,
        op: &str,
        reason: &str,
    ) -> Result<bool> {
        self.lock()
            .end_spawn(by, id, ("", ""), generation, op, reason)
    }
    /// Binds agent `key` to request `generation` of card `id`: the mark, and the
    /// card assigned to it.
    pub fn bind_spawn(&self, id: &str, generation: i64, key: &str) -> Result<bool> {
        self.lock().bind_spawn(id, generation, ("", ""), key)
    }
    /// The cards this device hands out that need an agent now and have
    /// none starting (or one starting past [`SPAWN_GRACE`]), oldest first.
    pub fn cards_to_spawn(&self) -> Vec<Task> {
        self.lock().cards_to_spawn()
    }
    /// What card `id` itself says about getting an agent (card #574).
    pub fn spawn_hold(&self, id: &str) -> SpawnHold {
        let f = self.lock();
        f.tasks
            .get(id)
            .map_or(SpawnHold::NoNeed, |t| f.spawn_hold(t))
    }
    /// The device that hands card `id` out; `""` when this log can't say.
    pub fn card_origin(&self, id: &str) -> String {
        let f = self.lock();
        f.tasks
            .get(id)
            .map(|t| f.card_origin(t))
            .unwrap_or_default()
    }
    /// Whether this device's person serves card t on a shared board: they
    /// made it, or it was given to them (card #573).
    pub fn mine_to_serve(&self, t: &Task) -> bool {
        self.lock().mine_to_serve(t)
    }
    /// Whether this device may start or bind card t's request s on a
    /// shared board. See [`Fold::mine_to_start`].
    pub fn mine_to_start(&self, t: &Task, s: &Spawn) -> bool {
        self.lock().mine_to_start(t, s)
    }
    /// Why the agent card t's yes asked for couldn't start (card #573).
    pub fn given_failed(t: &Task) -> Option<String> {
        Fold::given_failed(t)
    }
    /// Makes a card and gives it in one step, so no sweep sees it unheld
    /// and starts its author's agent first (card #573).
    pub fn create_and_assign(&self, by: &str, task: NewTask, to: &str) -> Result<(Task, Notice)> {
        let mut f = self.lock();
        let t = f.create_task(by, task)?;
        f.assign_notice(by, &t.id, to, "")
    }
    /// The cards given to this person that wait on "OK to start an agent
    /// on this?" (card #573). See [`Fold::given_to_ask`].
    pub fn given_to_ask(&self) -> Vec<Task> {
        self.lock().given_to_ask()
    }
    /// The given cards with a yes this device starts now (card #573).
    pub fn given_to_spawn(&self) -> Vec<Task> {
        self.lock().given_to_spawn()
    }
    /// Asks the person `by` about card `id` given to them (card #573).
    pub fn ask_given(&self, by: &str, id: &str, context: &str) -> Result<()> {
        self.lock().ask_given(by, id, context)
    }
    /// The requests naming this device it may start an agent for now
    /// (card #455): another of the person's devices chose it, or it chose
    /// itself. See [`Fold::spawns_here`].
    pub fn spawns_here(&self) -> Vec<(Task, Spawn)> {
        self.lock().spawns_here()
    }
    /// Marks agent `key` as started for card `task_id` of another log
    /// (card #455). See [`Fold::mark_spawned_elsewhere`].
    pub fn mark_spawned_elsewhere(
        &self,
        key: &str,
        task_id: &str,
        generation: i64,
        device: &str,
        board: &str,
        counted: bool,
    ) -> Result<()> {
        self.lock()
            .mark_spawned_elsewhere(key, task_id, generation, device, board, counted)
    }
    /// Whether card t's next agent counts toward "My agents may start
    /// agents" with no new ask (card #575). See [`Fold::in_chain`].
    pub fn in_chain(&self, t: &Task) -> bool {
        self.lock().in_chain(t)
    }
    /// Whether card t was given to this device's person (card #573): it
    /// starts only on their own yes.
    pub fn given_to_me(&self, t: &Task) -> bool {
        let f = self.lock();
        t.given.is_some() && f.mine_to_serve(t)
    }
    /// Whether agent `key`, started by Irori, hasn't gone (card #575).
    pub fn spawned_live(&self, key: &str) -> bool {
        self.lock().spawned_live(key)
    }
    /// The live agents that count toward that limit, by key (card #575).
    pub fn counted_live(&self) -> Vec<String> {
        self.lock().counted_live()
    }
    /// This log's counted requests still starting, oldest first (card #575).
    pub fn counted_pending(&self) -> Vec<(GoTime, String)> {
        self.lock().counted_pending()
    }
    /// This log's device id.
    pub fn device_id(&self) -> String {
        self.lock().dev.clone()
    }
    /// The inbox's Stale hook for a card Irori handed out (card #82): the
    /// note to close a hand-out notice to `to` with once the card went to
    /// someone else, was taken or closed, or `""`. Never takes coord's lock.
    pub fn stale_offer(&self, r: &str, to: &str) -> String {
        let Some(id) = r.strip_prefix("task:") else {
            return String::new();
        };
        let s = self.stale.read().unwrap_or_else(|e| e.into_inner());
        match s.offers.get(id) {
            Some((true, t)) if t != to => "Irori handed it to another agent".into(),
            Some((false, why)) => why.clone(),
            _ => String::new(),
        }
    }
    /// The inbox's off-side hold (card #495): whether a notice of `kind`
    /// about card `r` (`task:<id>`) to agent `to` is a pick Irori made on
    /// its own for a device that doesn't run the card's side. Only a
    /// hand-out (`offer`), a review request, or an ask the person didn't
    /// name `to` in counts.
    /// Best effort on the receiving device: coord and the inbox follow the
    /// store on their own threads, so a card this fold hasn't indexed yet
    /// reads as no side and passes. The sender's own pick filters
    /// (free_for, free_agents, cards_to_spawn) are the first guard.
    /// Never takes coord's lock.
    pub fn off_side(&self, kind: &str, r: &str, to: &str) -> bool {
        let Some(id) = r.strip_prefix("task:") else {
            return false;
        };
        let s = self.stale.read().unwrap_or_else(|e| e.into_inner());
        offers::off_side_in(&s, kind, id, to)
    }
    /// The inbox's Stale hook: never takes coord's lock.
    pub fn stale_notice(&self, r: &str, to: &str) -> String {
        let Some(id) = r.strip_prefix("task:") else {
            return String::new();
        };
        let s = self.stale.read().unwrap_or_else(|e| e.into_inner());
        if s.closed.contains(id) {
            return "the ask closed".into();
        }
        match s.stale.get(id) {
            Some(t) if !t.is_empty() && t != to => format!("taken by {t}"),
            _ => String::new(),
        }
    }

    // ---- subscribe and the clock-driven parts ----

    /// A change after every mutation (buffer 64; a slow reader gets one
    /// resync). Dropping the queue unsubscribes.
    pub fn subscribe(&self) -> Arc<Queue<Change>> {
        let q = Arc::new(Queue::new(SUB_BUFFER));
        self.lock().subs.push(Sub {
            q: Arc::downgrade(&q),
            lost: false,
        });
        q
    }

    /// One row from the store's subscription (Go Follow's body).
    pub fn handle(&self, ev: &Event) -> Result<()> {
        // Before the lock: `catching` comes first.
        if ev.is_resync() {
            return self.catch_up();
        }
        let mut f = self.lock();
        // repo.seen is capture's row, never coord's own write, so this
        // device's ones are folded here (card #521: a GitHub repo an agent
        // works in has a desk by itself).
        if ev.device_id == f.dev && ev.kind == store::kinds::REPO_SEEN {
            f.fold_desk(ev);
            return Ok(());
        }
        if ev.device_id == f.dev || !followed(&ev.kind) {
            if f.on_board {
                f.retry_aside();
            }
            return Ok(());
        }
        f.apply_remote(ev);
        Ok(())
    }

    /// Follow's 1 s tick: refusals for conflicts lost, expiry rows for own
    /// leases, and the ask owner's duties.
    /// Records what GitHub said about PR `check.pr` on card `id` (card
    /// #208) and returns the card and the notices to route.
    pub fn record_pr_state(&self, id: &str, check: PrCheck) -> Result<(Task, Vec<Notice>)> {
        self.lock().record_pr_state(id, check)
    }

    /// The person sets up or changes a repo's merge desk (card #415).
    pub fn set_desk(&self, by: &str, desk: RepoDeskRow) -> Result<()> {
        self.lock().set_desk(by, desk)
    }

    /// Records the open PRs the desk poller heard for `repo_key`; writes a
    /// snapshot only on change. Whether a row was written.
    pub fn record_desk_prs(
        &self,
        repo_key: &str,
        prs: Vec<DeskPr>,
        truncated: bool,
    ) -> Result<bool> {
        self.lock().record_desk_prs(repo_key, prs, truncated, "")
    }

    /// [`Coord::record_desk_prs`], with the GitHub login the poll's gh is
    /// signed in as (`viewer`: the person's own, which the desk trusts;
    /// card #521). An empty `viewer` keeps the one heard before.
    pub fn record_desk_snapshot(
        &self,
        repo_key: &str,
        prs: Vec<DeskPr>,
        truncated: bool,
        viewer: &str,
    ) -> Result<bool> {
        self.lock()
            .record_desk_prs(repo_key, prs, truncated, viewer)
    }

    /// The desk holder records the PR it runs under the desk lease
    /// (`desk.run`, card #418); it must hold the lease.
    pub fn record_desk_run(
        &self,
        by: &str,
        repo_key: &str,
        pr: i64,
        head_sha: &str,
        base_sha: &str,
    ) -> Result<()> {
        self.lock()
            .record_desk_run(by, repo_key, pr, head_sha, base_sha)
    }

    /// The person holds a desk's PR, or lifts the hold (card #417).
    pub fn hold_desk_pr(
        &self,
        by: &str,
        repo_key: &str,
        pr: i64,
        note: &str,
        cleared: bool,
    ) -> Result<()> {
        self.lock().hold_desk_pr(by, repo_key, pr, note, cleared)
    }

    /// The person pauses or resumes a desk (card #417).
    pub fn pause_desk(&self, by: &str, repo_key: &str, paused: bool) -> Result<()> {
        self.lock().pause_desk(by, repo_key, paused)
    }

    /// The person okays an Outside PR at one head (card #417).
    pub fn ok_desk_pr(&self, by: &str, repo_key: &str, pr: i64, head_sha: &str) -> Result<()> {
        self.lock().ok_desk_pr(by, repo_key, pr, head_sha)
    }

    /// Every merge desk in this log, by repo key.
    pub fn desks(&self) -> Vec<Desk> {
        let f = self.lock();
        f.desks.values().map(|d| f.desk_view(d)).collect()
    }

    /// One repo's merge desk, if it has one.
    pub fn desk(&self, repo_key: &str) -> Option<Desk> {
        let f = self.lock();
        f.desks
            .get(&ledger::canonical_repo_key(repo_key))
            .map(|d| f.desk_view(d))
    }

    /// The states of the PRs card `id` links, as last heard (card #208).
    pub fn pr_states(&self, id: &str) -> Vec<PrState> {
        let f = self.lock();
        f.tasks
            .get(id)
            .map(|t| f.pr_states_of(t))
            .unwrap_or_default()
    }

    /// How PR `pr` on card `id` stood when last heard, whether the card
    /// links it or only a wait is on it (card #509).
    pub fn pr_state(&self, id: &str, pr: &str) -> Option<PrState> {
        self.lock().pr_states.get(id)?.get(pr).cloned()
    }

    /// The PRs card `id`'s open external waits ask this person to merge,
    /// not yet known merged or closed (card #509); on a shared board only
    /// their own agents' waits.
    pub fn merge_waits(&self, id: &str) -> Vec<String> {
        let f = self.lock();
        f.tasks
            .get(id)
            .map(|t| f.merge_waits(t))
            .unwrap_or_default()
    }

    /// The PRs of [`Coord::merge_waits`] the poller still asks about, a
    /// closed one too (card #548).
    pub fn polled_waits(&self, id: &str) -> Vec<String> {
        let f = self.lock();
        f.tasks
            .get(id)
            .map(|t| f.polled_waits(t))
            .unwrap_or_default()
    }

    /// What this board's PR rows say, for each device to pass to the
    /// agents it hosts (card #548), told already or not.
    pub fn pr_news(&self) -> Vec<PrNews> {
        self.lock().pr_news()
    }

    pub fn tick(&self) {
        self.lock().holder_duties();
    }

    /// Rebuilds the fold when the log holds fewer of some device's rows
    /// than were folded (a revoke's cut moved them to refused_events).
    pub fn refold_if_cut(&self, vec: &store::Vector) -> Result<()> {
        let _one = self.catching.lock().unwrap_or_else(|e| e.into_inner());
        self.lock().refold_if_cut(vec)
    }

    /// Whether a card's short ref moved at or after `since` (search
    /// re-reads those docs).
    pub fn short_moved_since(&self, id: &str, since: &GoTime) -> bool {
        self.lock().moved_since(id, since)
    }

    /// After a store resync signal: refold if cut, then page each other
    /// device's rows past what was folded. Readers and writers get in between
    /// pages, so a catch-up of thousands of rows holds the lock for one page
    /// at a time (card #567); only a refold, which rebuilds the fold after a
    /// cut, holds it throughout. When it returns, every followed row
    /// committed before the call is folded.
    pub fn catch_up(&self) -> Result<()> {
        self.catch_up_by(store::MAX_LIST_LIMIT)
    }

    /// [`Coord::catch_up`] in pages of `limit` rows.
    #[doc(hidden)]
    pub fn catch_up_by(&self, limit: i64) -> Result<()> {
        // As Store::list clamps it: only the last page is short.
        let limit = if limit <= 0 {
            store::DEFAULT_LIST_LIMIT
        } else {
            limit.min(store::MAX_LIST_LIMIT)
        };
        let mut read = self.catching.lock().unwrap_or_else(|e| e.into_inner());
        // In one hold: a vector read apart from `last` could sit below a row
        // folded since, which reads as a cut.
        let (st, vec, me, on_board) = {
            let mut f = self.lock();
            let Some(st) = f.st.clone() else {
                return Ok(());
            };
            let vec = st.vector()?;
            f.refold_if_cut(&vec)?;
            (st, vec, f.dev.clone(), f.on_board)
        };
        for (dev, top) in vec.iter().filter(|(d, _)| **d != me) {
            // The mark holds only while its row is still in the log: a cut
            // may have taken it, and the device's other branch written new
            // rows, followed ones too, at the seqs it covered. Then this
            // reads on from what is folded, as before #567.
            let mut from = 0;
            if let Some(m) = read.get(dev)
                && m.seq <= *top
                && row_at(&st, dev, m.seq)?.as_ref() == Some(m)
            {
                from = m.seq;
            }
            self.catch_up_device(&st, dev, from, *top, limit)?;
            match row_at(&st, dev, *top)? {
                Some(m) => read.insert(dev.clone(), m),
                None => read.remove(dev),
            };
        }
        if !on_board {
            self.catch_up_own_seen(&st, &me, limit)?;
        }
        self.lock().holder_duties();
        Ok(())
    }

    /// Pages `dev`'s followed rows past `from`, or past what is folded if
    /// that is further, up to `top`, each page read outside the lock. Every
    /// followed row up to `top` was committed before the vector was read, so
    /// when it returns they are all folded and the caller marks the row at
    /// `top` read, past the unfollowed rows at the device's tail.
    fn catch_up_device(
        &self,
        st: &Store,
        dev: &str,
        from: i64,
        top: i64,
        limit: i64,
    ) -> Result<()> {
        let mut after = from.max(self.lock().last.get(dev).copied().unwrap_or(0));
        while after < top {
            let evs = st.list(&store::Filter {
                device_id: dev.to_string(),
                after_seq: after,
                kinds: FOLLOW_KINDS.iter().map(|k| k.to_string()).collect(),
                limit,
                ..store::Filter::default()
            })?;
            let Some(end) = evs.last().map(|e| e.seq) else {
                return Ok(());
            };
            let more = evs.len() as i64 == limit;
            let mut f = self.lock();
            f.fold_page(&evs);
            if !more {
                return Ok(());
            }
            // A live row may have passed the page.
            after = end.max(f.last.get(dev).copied().unwrap_or(0));
        }
        Ok(())
    }

    /// This device's repo.seen rows are capture's, not coord's own writes,
    /// so they reach the fold only live ([`Coord::handle`]); one a full
    /// subscription dropped, or one written between the startup replay and
    /// Follow subscribing, is read here. There is one per worktree root,
    /// and folding one twice changes nothing (card #521).
    fn catch_up_own_seen(&self, st: &Store, me: &str, limit: i64) -> Result<()> {
        let mut after = 0;
        loop {
            let evs = st.list(&store::Filter {
                device_id: me.to_string(),
                after_seq: after,
                kinds: vec![store::kinds::REPO_SEEN.to_string()],
                limit,
                ..store::Filter::default()
            })?;
            let mut f = self.lock();
            for ev in &evs {
                f.fold_desk(ev);
            }
            drop(f);
            match evs.last() {
                Some(l) if evs.len() as i64 == limit => after = l.seq,
                _ => return Ok(()),
            }
        }
    }

    /// When the expiry timer should fire: the soonest Until of this
    /// device's leases while anyone is subscribed.
    pub fn next_expiry(&self) -> Option<GoTime> {
        // A read: no unlock signal. The core's expiry timer calls this right
        // after taking the rearm generation; firing on_unlock here kicked
        // that generation and spun the timer on a whole core (#70).
        self.f
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .next_expiry()
    }

    /// Runs `f` after every unlock (Go's `unlock` re-arming its timer): the
    /// core's expiry timer re-reads [`Coord::next_expiry`] when it fires.
    /// `f` runs under the coord's lock, so it must only signal.
    pub fn set_on_unlock(&self, f: impl Fn() + Send + Sync + 'static) {
        self.lock().on_unlock = Some(Arc::new(f));
    }

    /// The expiry timer: prune, which notes each lease that ran out.
    pub fn expire(&self) {
        let mut f = self.lock();
        let now = f.stamp();
        f.prune(&now);
    }
}

/// `dev`'s row at `seq`, if the log holds it: one seek along the key.
fn row_at(st: &Store, dev: &str, seq: i64) -> Result<Option<Event>> {
    Ok(st
        .list(&store::Filter {
            device_id: dev.to_string(),
            after_seq: seq - 1,
            limit: 1,
            ..store::Filter::default()
        })?
        .into_iter()
        .next()
        .filter(|e| e.seq == seq))
}

impl Fold {
    /// Withdraw (boardlog.go): a board card canceled with withdrawn set.
    fn withdraw(&mut self, by: &str, id: &str, note: &str) -> Result<Task> {
        let by = crate::names::trim_space(by).to_string();
        let note = crate::names::trim_space(note).to_string();
        if !self.on_board {
            return Err(error::invalid(
                "only a card in a shared space is withdrawn; cancel a private card instead",
            ));
        }
        if util::runes(&note) > MAX_OBJECTIVE {
            return Err(error::invalid(format!(
                "note longer than {MAX_OBJECTIVE} characters"
            )));
        }
        let Some(t) = self.tasks.get(id) else {
            return Err(error::not_found(format!(
                "task {}",
                gocompat::quote::quote(id)
            )));
        };
        if t.withdrawn {
            return Ok(t.clone());
        }
        if !self.may_cancel(t, &self.dev, None) {
            return Err(error::not_allowed(format!(
                "only its creator or a space owner withdraws {}",
                t.short_ref()
            )));
        }
        let note = util::cmp_str(&note, "withdrawn from the board");
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_STATUS,
            &rows::TaskStateRow {
                task_id: id.into(),
                state: CANCELED.into(),
                note,
                withdrawn: true,
                by: by.clone(),
                at,
                ..rows::TaskStateRow::default()
            },
        )?;
        Ok(self.tasks[id].clone())
    }
}
