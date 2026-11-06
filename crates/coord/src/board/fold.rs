//! The fold (I:internal/coord/impl.go, mesh.go, boardlog.go,
//! subscribe.go; coord.md §3.2-3.4, §3.16): state rebuilt from the log,
//! one row at a time, and the write path that appends then folds.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, RwLock, Weak};
use std::time::Duration;

use gocompat::sub::Queue;
use gocompat::time::GoTime;
use irori_identity::owner;
use store::{Event, Store, kinds};

use super::asks::AskBook;
use super::error::{Error, ErrorKind, Result};
use super::resource::{Resource, normalize_resource, parse_resource};
use super::rows::*;
use super::types::*;
use super::util::{SWEEPER, cmp_str, collapse_space, contains, cut, remove_str};
use super::{MAX_ASIDE, Options};
use crate::gotime;
use crate::names::trim_space;
use crate::roster::Agents;

/// Release reasons in lease.released rows.
impl Fold {
    /// How a notice names agent `key` (card #98): "name (key)" when the
    /// roster knows its name, else the key. The person and Irori stay as
    /// the caller gave them.
    pub(crate) fn named(&self, key: &str) -> String {
        if key.is_empty() || key == SWEEPER || self.is_owner(key) {
            return key.to_string();
        }
        let name = self
            .roster
            .as_ref()
            .map(|r| r.name_of(key))
            .unwrap_or_default();
        let name = name.trim();
        if name.is_empty() || name == key {
            key.to_string()
        } else {
            format!("{name} ({key})")
        }
    }
}

/// How much of a hand-off note the thread line quotes.
const MAX_HANDOFF_LINE: usize = 300;

/// An assignee who acts on the card as itself (a step tick, a comment)
/// has taken it up (card #85): it stops being only offered.
pub(crate) fn take_up(t: &mut Task, by: &str) -> bool {
    if !contains(&t.assigned, by) {
        return false;
    }
    t.assigned = remove_str(&t.assigned, by);
    sync_holder(t);
    if t.state == SUBMITTED {
        t.state = WORKING.into();
    }
    true
}

pub const REASON_RELEASED: &str = "released";
pub const REASON_STOLEN: &str = "stolen";
pub const REASON_HOLDER_GONE: &str = "holder_gone";
pub const REASON_EXPIRED: &str = "expired";
pub const REASON_CONFLICT: &str = "conflict";

/// A task.left reason: Irori took a holder that went away off the card.
pub const LEFT_GONE: &str = "gone";
/// A task.left reason: the card went to Not yet with the holder on it
/// (card #570).
pub const LEFT_PARKED: &str = "parked";
/// A task.left reason: an owner removed the holder's person from the board
/// (v0.5 §4.3).
pub const LEFT_REMOVED: &str = "member_removed";
/// A task.left reason: the agent Irori started for the card ended (card
/// #446). Nobody is told the card is open again; the next agent starts
/// from what is on it.
pub const LEFT_FINISHED: &str = "finished";

/// Which leases found lapsed at replay still get their expiry row.
const LAPSED_WITHIN: Duration = Duration::from_secs(24 * 3600);
/// How far two devices' wall clocks may differ.
const CLOCK_SKEW: Duration = Duration::from_secs(120);
/// Each subscriber's queue size.
pub const SUB_BUFFER: usize = 64;

/// The row prefixes coord folds.
///
/// A row kind that must reach other devices or survive a restart goes
/// here: the live follow ([`super::Coord::handle`]) and the store replay
/// fold only these kinds, and so does [`super::Coord::fold_live`], so a
/// test that folds a kind missing here fails as production would (card
/// #614; `pr.state` was missed until #542).
pub const FOLLOW_KINDS: [&str; 16] = [
    "task.",
    "lease.",
    "blocker.",
    "take.",
    "board.ask.",
    "review.",
    "wait.",
    "note.",
    // The merge desk (card #415).
    "repo.desk",
    "desk.",
    // The sides each device runs (card #494): the exact kind, not
    // "device.", which would pull in names and colours.
    kinds::DEVICE_SIDES,
    // A GitHub repo an agent works in has a desk by itself (card #521).
    kinds::REPO_SEEN,
    // A computer takes an ask (card #541).
    kinds::ASK_MOVED,
    // A board's row can land after its cards (card #558), and a card's
    // board and tag come from the board list (card #563); who may delete a
    // board comes from the members at the row's place.
    "space.board.",
    "board.member.",
    // What GitHub said about a card's PR (card #542): a phone has no gh, so
    // these rows are all it knows, and a restart must keep them.
    kinds::PR_STATE,
];

/// Whether coord folds `kind`.
pub fn followed(kind: &str) -> bool {
    FOLLOW_KINDS.iter().any(|p| kind.starts_with(p))
}

#[derive(Debug, Clone)]
pub(crate) struct LeaseRec {
    pub lease: Lease,
    pub res: Resource,
    /// The device that wrote its lease.acquired: on a board, the holder's
    /// mirroring Core, which a `agent:` holder key doesn't name.
    pub dev: String,
}

#[derive(Debug, Clone)]
pub(crate) struct RefusalRec {
    pub r: Refusal,
    pub res: Resource,
}

#[derive(Debug, Clone)]
pub(crate) struct WatchLine {
    pub by: String,
    pub what: String,
    pub also: Vec<String>,
    /// Recipients this line wakes instead of telling at their next turn: a
    /// split card's parent holders, for a subtask's person blocker, failure
    /// or the last subtask finishing (card #89).
    pub wake: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WatchQueue {
    pub lines: Vec<WatchLine>,
    pub dropped: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ShortClaim {
    pub id: String,
    pub at: RowPos,
    pub num: i64,
    /// The board the card claimed on, as its row says: empty for the
    /// space's first. Raw, so a row that arrives before its
    /// `space.board.created` numbers the same on every device.
    pub board: String,
    /// A move's claim, not a create's (card #563).
    pub moved: bool,
    /// Given by the walk to a card whose board was deleted: no row says it.
    pub displaced: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ParentLink {
    pub child: String,
    pub parent: String,
    pub at: RowPos,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PinRec {
    pub pinned: bool,
    pub rank: f64,
    pub at: RowPos,
}

pub(crate) struct Sub {
    pub q: Weak<Queue<Change>>,
    pub lost: bool,
}

/// What StaleNotice reads, under its own lock (it runs under the inbox's).
#[derive(Debug, Default)]
pub(crate) struct StaleMap {
    pub stale: HashMap<String, String>,
    pub closed: HashSet<String>,
    /// Unheld cards Irori hands out (card #82), by id: (live, the agent it
    /// went to while live, or why it no longer goes out).
    pub offers: HashMap<String, (bool, String)>,
    /// Each card on a side (card #495), by id, and the sides each device
    /// runs: what the inbox's off-side hold reads, without coord's lock.
    pub sides: HashMap<String, SideCard>,
    pub runs: crate::sides::Sides,
}

/// A card's side as the inbox's off-side hold reads it (card #495).
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct SideCard {
    pub side: String,
    /// The agents the person's ask names: they hear it, any side.
    pub named: Vec<String>,
    pub thread: bool,
}

/// The bits of a task `touch` needs, so a handler can touch while it
/// holds the task.
pub(crate) struct Touch {
    pub id: String,
    pub parent: String,
    pub has_watchers: bool,
    pub short_ref: String,
    /// Who works the card (holders and co-holders): they hear another
    /// agent's change to it (card #320).
    pub holders: Vec<String>,
}

impl Touch {
    pub fn of(t: &Task) -> Touch {
        Touch {
            id: t.id.clone(),
            parent: t.parent.clone(),
            has_watchers: !t.watchers.is_empty(),
            short_ref: t.short_ref(),
            holders: t.holders.iter().chain(&t.co_holders).cloned().collect(),
        }
    }
}

type KeyFn = Arc<dyn Fn(&str) -> bool + Send + Sync>;
type StrFn = Arc<dyn Fn(&str) -> String + Send + Sync>;
type NowFn = Arc<dyn Fn() -> GoTime + Send + Sync>;

/// The Board's state: every task, lease, comment and take, folded from
/// the log. Coord holds it behind a lock.
pub struct Fold {
    pub(crate) st: Option<Arc<Store>>,
    pub(crate) dev: String,
    pub(crate) roster: Option<Arc<dyn Agents>>,
    pub(crate) now: NowFn,
    pub(crate) grace: Duration,
    pub(crate) holder_grace: Duration,
    pub(crate) owner_fn: KeyFn,
    pub(crate) revoked: Option<KeyFn>,
    /// See [`super::Options::busy_elsewhere`].
    pub(crate) busy_elsewhere: Option<super::BusyFn>,
    pub(crate) on_board: bool,
    pub(crate) done_check: bool,
    pub(crate) person: String,
    /// The sides each device runs (card #494), from `device.sides` rows.
    pub(crate) sides: crate::sides::Sides,
    /// See [`super::Options::spawning`].
    pub(crate) spawning: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    /// See [`super::Options::agent_devices`].
    pub(crate) agent_devices: Option<super::AgentDevicesFn>,
    /// See [`super::Options::offers`].
    pub(crate) offers: Option<super::OffersFn>,
    /// See [`super::Options::crash_limit`].
    pub(crate) crash_limit: Option<Arc<dyn Fn() -> i64 + Send + Sync>>,
    /// See [`super::Options::outside_leases`].
    pub(crate) outside_leases: Option<Arc<dyn Fn() -> Vec<String> + Send + Sync>>,
    /// See [`super::Options::device_version`].
    pub(crate) device_version: Option<super::DeviceVersionFn>,
    /// "task/blocker" of external waits on this person's private leases
    /// (card #339), noted while one of them was held.
    pub(crate) outside_waits: std::cell::RefCell<HashSet<String>>,
    /// Cards whose failed spawn the person asked to start again now
    /// (Trust and start, "Start an agent"): past [`super::SPAWN_RETRY`]'s
    /// wait (card #464). Dropped once a new request is written.
    pub(crate) spawn_now: HashSet<String>,
    /// On a board, the device that wrote each card's task.created: it runs
    /// the card's hand-out (card #174).
    pub(crate) origin: HashMap<String, String>,
    pub(crate) key_person: Option<StrFn>,
    pub(crate) dev_person: Option<StrFn>,
    pub(crate) tag_fn: Option<Arc<dyn Fn() -> String + Send + Sync>>,
    pub(crate) board_tag_fn: Option<super::BoardTagFn>,
    pub(crate) board_tags_fn: Option<super::BoardTagsFn>,
    pub(crate) board_view_fn: Option<super::BoardViewFn>,
    /// The last [`super::BoardList`] read (card #563).
    pub(crate) board_view: super::BoardList,
    pub(crate) no_cards_fn: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    pub(crate) board_owner: Option<super::OwnerAtFn>,
    pub(crate) hop_of: Option<super::HopFn>,
    pub(crate) holds_file: Option<KeyFn>,
    pub(crate) ignored: usize,
    pub(crate) ignored_by: BTreeMap<String, usize>,
    pub(crate) aside: Vec<Event>,
    pub(crate) aside_drop: usize,

    pub(crate) tasks: BTreeMap<String, Task>,
    pub(crate) order: HashMap<String, u64>,
    pub(crate) next_order: u64,
    pub(crate) held: BTreeMap<String, LeaseRec>,
    pub(crate) first_absent: HashMap<String, GoTime>,
    /// When each of this device's agents last went, from its `agent.gone`
    /// row (cleared by a later `agent.seen`), read at replay: a holder that
    /// went before the Core restarted counts as absent since then, not
    /// since the restart (card #110; #81 stayed held across restarts).
    pub(crate) gone_rows: HashMap<String, GoTime>,
    /// The spawned_for mark (card #446): the card each spawned agent was
    /// started for, kept after it goes.
    pub(crate) spawned: HashMap<String, super::spawn::Spawned>,
    /// The keys in `spawned`, for readers that must not take coord's lock
    /// (the inbox's front desk).
    pub(crate) spawned_keys: Arc<RwLock<HashSet<String>>>,
    /// Spawned agents Irori ended itself (End, or done): they leave their
    /// card as finished, not gone (a crash).
    pub(crate) ended_by_irori: HashSet<String>,
    /// Of those, the ones it ended on a Claude usage limit (card #450).
    pub(crate) usage_limited: HashSet<String>,
    /// (agent, card) pairs already told that the agent isn't answering
    /// (card #110), while it still isn't.
    pub(crate) mute_told: HashSet<(String, String)>,
    /// (agent, card) pairs nudged to name what the card waits on (card
    /// #305), with the card's updated_at then: told again only after the
    /// card changed.
    pub(crate) idle_told: HashMap<(String, String), GoTime>,
    pub(crate) comments: HashMap<String, Comment>,
    pub(crate) by_task: HashMap<String, Vec<String>>,
    pub(crate) refusals: Vec<RefusalRec>,
    pub(crate) blocker_seq: HashMap<String, i64>,
    pub(crate) step_seq: HashMap<String, i64>,
    pub(crate) activity: HashMap<String, Vec<Activity>>,
    pub(crate) watchq: BTreeMap<String, WatchQueue>,
    /// Open waits by id (card #87), the queue of those to fire with what
    /// happened, and the ids fired or canceled (a late wait.created for
    /// one of them is not folded back in).
    pub(crate) waits: BTreeMap<String, Wait>,
    /// Card positions on the canvas (card #93), by card id.
    pub(crate) placed: HashMap<String, (i64, i64)>,
    pub(crate) waitq: Vec<(String, String)>,
    pub(crate) waits_done: HashSet<String>,
    /// The state each open task wait last saw, so the sweep catches a
    /// move that came by another path than task.status (or while off).
    pub(crate) wait_seen: HashMap<String, String>,
    /// Board waits whose card the core found gone (card #574).
    pub(crate) waits_lost: HashSet<String>,

    pub(crate) claims: Vec<ShortClaim>,
    pub(crate) claimed: HashSet<String>,
    /// Each board has its own run (card #558): claims by (board, number).
    pub(crate) short_groups: BTreeMap<(String, i64), Vec<ShortClaim>>,
    /// Card ids by (board, short ref).
    pub(crate) short_idx: BTreeMap<(String, String), String>,
    /// Each board's highest number seen. Not cleared by a refold, as the
    /// one run's top never was: a number a cut card held isn't given out
    /// again here.
    pub(crate) short_tops: HashMap<String, i64>,
    /// Numbers cards moved away from (card #563): each stays claimed, and
    /// its old ref still finds the card unless a card holds it now.
    pub(crate) short_hist: BTreeMap<(String, i64), Vec<(RowPos, String)>>,
    /// Cards a move has named, and those rows by (device, seq).
    pub(crate) moved_ids: HashSet<String>,
    pub(crate) moved_rows: HashSet<(String, i64)>,
    /// How far the last walk went: a claim past it can't change what the
    /// walk gave.
    pub(crate) walked_to: Option<RowPos>,
    pub(crate) short_moved: HashMap<String, GoTime>,

    pub(crate) subs: Vec<Sub>,
    pub(crate) pending: Vec<Change>,
    pub(crate) replaying: bool,
    /// Runs at every unlock, after the changes went out (Go's `unlock`
    /// re-arming the expiry timer): the core's timer re-reads
    /// `next_expiry`. It runs under the lock and must not call the coord.
    pub(crate) on_unlock: Option<Arc<dyn Fn() + Send + Sync>>,

    pub(crate) last: HashMap<String, i64>,
    pub(crate) conflicts: BTreeMap<String, LeaseRec>,
    pub(crate) refused: HashSet<String>,
    pub(crate) stolen: HashSet<String>,
    pub(crate) lapsed: Vec<Lease>,

    pub(crate) asks: AskBook,
    pub(crate) stale: Arc<RwLock<StaleMap>>,
    pub(crate) links: Vec<ParentLink>,
    /// A row changed what [`Fold::relink`] derives parents and subtasks
    /// from: the link rows, which cards exist, or an ask's being open. The
    /// next rows of a catch-up page read those fields, so such a row relinks
    /// at once (card #567). A plain row only moves rollups.
    pub(crate) links_moved: bool,
    pub(crate) pins: HashMap<String, PinRec>,
    /// Done cards the person cleared from the window (card #138), by id,
    /// with the row that last set it. Any id: a board card's too.
    pub(crate) archived: HashMap<String, (bool, RowPos)>,
    /// Cards the person paused (card #207), by id, with the row that last
    /// set it.
    pub(crate) paused: HashMap<String, (bool, RowPos)>,
    /// A board's review requests by card, and its dedicated reviewers.
    pub(crate) reviews: BTreeMap<String, Vec<super::review::ReviewRequest>>,
    /// Agents whose device reads offline, for the presence notices (card
    /// #372). In memory only.
    pub(crate) away: HashMap<String, super::presence::Away>,
    pub(crate) reviewers: super::review::Reviewers,
    /// Notes by id, with the row that made each (card #401).
    pub(crate) notes: BTreeMap<String, (super::notes::Note, RowPos)>,
    /// Cards the person deleted (card #57): never folded again.
    pub(crate) deleted: HashSet<String>,
    /// Cards that have been given to someone on a shared board (card
    /// #573), even if they were taken back since.
    pub(crate) given_ever: HashSet<String>,
    /// Cards that were in_review when they were blocked (card #191): an
    /// unblock restores in_review, not working. Not part of the Task's
    /// Go shape.
    pub(crate) review_before_block: HashSet<String>,
    /// Each card's linked PRs as GitHub last said they stand (card #208).
    pub(crate) pr_states: super::prstate::PrStates,
    /// pr.state rows waiting on their card's holder or wait (card #548).
    pub(crate) pr_ahead: Vec<Event>,
    /// Who a card's pr.state rows count from, ever (card #548): each
    /// holder's key, for any PR, and each merge wait's raiser, for its PR
    /// (empty PR: any). It only grows, so a row is taken in any order
    /// against the holder's leave or the wait's answer.
    pub(crate) pr_vouchers: HashMap<String, Vec<(String, String)>>,
    /// `pr_vouchers` grew since kept rows were last looked at.
    pub(crate) pr_vouched: bool,
    /// Merge desks by repo key (card #415).
    pub(crate) desks: BTreeMap<String, super::desk::Desk>,
    /// The person's repos as the Core last said (card #625): a desk on any
    /// other is stray. `None` until it says, when every desk counts. Not
    /// from rows, so a resync keeps it.
    pub(crate) desk_repos: Option<std::collections::BTreeSet<String>>,
    /// The desks the Core judged when it last said (card #625). A desk
    /// made since (a repo an agent just started in) counts until the
    /// Core's next pass, which runs before any poll.
    pub(crate) desk_judged: std::collections::BTreeSet<String>,
}

/// `stolenKey`: resource, holder and acquiredAt (RFC3339Nano, UTC).
pub(crate) fn stolen_key(resource: &str, holder: &str, acquired_at: &GoTime) -> String {
    format!(
        "{resource}\0{holder}\0{}",
        acquired_at.utc().format_rfc3339_nano()
    )
}

impl Fold {
    pub(crate) fn new(
        st: Option<Arc<Store>>,
        dev: &str,
        roster: Option<Arc<dyn Agents>>,
        o: Options,
    ) -> Fold {
        let dev = dev.to_string();
        let now: NowFn = o
            .now
            .unwrap_or_else(|| Arc::new(|| GoTime::from_system(std::time::SystemTime::now())));
        let on_board = o.board;
        let person = o.person.clone();
        let self_key = if on_board {
            person.clone()
        } else {
            format!("user:{dev}")
        };
        let owner_fn = o
            .is_owner
            .unwrap_or_else(|| Arc::new(move |k: &str| k == self_key));
        Fold {
            st,
            dev,
            roster,
            now,
            grace: if o.gone_grace.is_zero() {
                super::GONE_GRACE
            } else {
                o.gone_grace
            },
            holder_grace: if o.holder_grace.is_zero() {
                super::HOLDER_GRACE
            } else {
                o.holder_grace
            },
            owner_fn,
            revoked: o.revoked,
            busy_elsewhere: o.busy_elsewhere,
            on_board,
            done_check: o.done_check,
            person,
            key_person: o.key_person,
            sides: crate::sides::Sides::new(),
            spawning: o.spawning,
            agent_devices: o.agent_devices,
            offers: o.offers,
            crash_limit: o.crash_limit,
            outside_leases: o.outside_leases,
            device_version: o.device_version,
            outside_waits: Default::default(),
            spawn_now: HashSet::new(),
            origin: HashMap::new(),
            dev_person: o.device_person,
            tag_fn: o.tag,
            board_tag_fn: o.board_tag,
            board_tags_fn: o.board_tags,
            board_view_fn: o.board_view,
            board_view: super::BoardList::default(),
            no_cards_fn: o.no_cards,
            board_owner: o.board_owner,
            hop_of: o.hop_of,
            holds_file: o.holds_file,
            ignored: 0,
            ignored_by: BTreeMap::new(),
            aside: Vec::new(),
            aside_drop: 0,
            tasks: BTreeMap::new(),
            order: HashMap::new(),
            next_order: 0,
            held: BTreeMap::new(),
            first_absent: HashMap::new(),
            gone_rows: HashMap::new(),
            spawned: HashMap::new(),
            spawned_keys: Arc::new(RwLock::new(HashSet::new())),
            ended_by_irori: HashSet::new(),
            usage_limited: HashSet::new(),
            mute_told: HashSet::new(),
            idle_told: HashMap::new(),
            comments: HashMap::new(),
            by_task: HashMap::new(),
            refusals: Vec::new(),
            blocker_seq: HashMap::new(),
            step_seq: HashMap::new(),
            activity: HashMap::new(),
            watchq: BTreeMap::new(),
            waits: BTreeMap::new(),
            placed: HashMap::new(),
            waitq: Vec::new(),
            waits_done: HashSet::new(),
            wait_seen: HashMap::new(),
            waits_lost: HashSet::new(),
            claims: Vec::new(),
            claimed: HashSet::new(),
            short_groups: BTreeMap::new(),
            short_idx: BTreeMap::new(),
            short_tops: HashMap::new(),
            short_hist: BTreeMap::new(),
            moved_ids: HashSet::new(),
            moved_rows: HashSet::new(),
            walked_to: None,
            short_moved: HashMap::new(),
            subs: Vec::new(),
            pending: Vec::new(),
            replaying: false,
            on_unlock: None,
            last: HashMap::new(),
            conflicts: BTreeMap::new(),
            refused: HashSet::new(),
            stolen: HashSet::new(),
            lapsed: Vec::new(),
            asks: AskBook::default(),
            stale: Arc::new(RwLock::new(StaleMap::default())),
            links: Vec::new(),
            links_moved: false,
            pins: HashMap::new(),
            archived: HashMap::new(),
            paused: HashMap::new(),
            reviews: BTreeMap::new(),
            away: HashMap::new(),
            reviewers: BTreeMap::new(),
            notes: BTreeMap::new(),
            deleted: HashSet::new(),
            given_ever: HashSet::new(),
            review_before_block: HashSet::new(),
            pr_states: Default::default(),
            pr_ahead: Vec::new(),
            pr_vouchers: HashMap::new(),
            pr_vouched: false,
            desks: BTreeMap::new(),
            desk_repos: None,
            desk_judged: std::collections::BTreeSet::new(),
        }
    }

    pub(crate) fn is_owner(&self, key: &str) -> bool {
        (self.owner_fn)(key)
    }

    /// `now().UTC().Round(0)`.
    pub(crate) fn stamp(&self) -> GoTime {
        (self.now)().utc()
    }

    /// Replays every task./lease./blocker./take. row from the store in log
    /// order, paged by the full cursor.
    pub(crate) fn replay_store(&mut self) -> Result<()> {
        let Some(st) = self.st.clone() else {
            return Ok(());
        };
        // Rows name boards as the list does, so it's read before them.
        self.read_board_view();
        let mut f = store::Filter {
            kinds: FOLLOW_KINDS.iter().map(|k| k.to_string()).collect(),
            limit: store::MAX_LIST_LIMIT,
            ..store::Filter::default()
        };
        self.replaying = true;
        loop {
            let evs = match st.list(&f) {
                Ok(e) => e,
                Err(e) => {
                    self.replaying = false;
                    return Err(Error::from(e).prefixed("coord: replay"));
                }
            };
            self.replay_page(&evs);
            if (evs.len() as i64) < f.limit {
                break;
            }
            f.after = evs[evs.len() - 1].cursor();
        }
        self.end_replay();
        self.replay_gone(&st)
    }

    /// Reads when this device's agents went (`agent.gone`, undone by a
    /// later `agent.seen`) into [`Fold::gone_rows`].
    fn replay_gone(&mut self, st: &Store) -> Result<()> {
        self.gone_rows.clear();
        let mut f = store::Filter {
            kinds: vec![kinds::AGENT_GONE.to_string(), kinds::AGENT_SEEN.to_string()],
            limit: store::MAX_LIST_LIMIT,
            ..store::Filter::default()
        };
        loop {
            let evs = st
                .list(&f)
                .map_err(|e| Error::from(e).prefixed("coord: replay"))?;
            for ev in &evs {
                if ev.device_id != self.dev {
                    continue;
                }
                let Some(r) = row::<GoneKeyRow>(&ev.payload) else {
                    continue;
                };
                let k = r.key;
                if k.is_empty() {
                    continue;
                }
                if ev.kind == kinds::AGENT_GONE && r.reason == "finished" {
                    self.ended_by_irori.insert(k.clone());
                }
                if ev.kind == kinds::AGENT_GONE && r.reason == super::spawn::GONE_USAGE_LIMIT {
                    self.ended_by_irori.insert(k.clone());
                    self.usage_limited.insert(k.clone());
                }
                if ev.kind == kinds::AGENT_GONE {
                    self.gone_rows.insert(k, gotime::hlc_wall(ev.hlc));
                } else {
                    self.gone_rows.remove(&k);
                }
            }
            if (evs.len() as i64) < f.limit {
                return Ok(());
            }
            f.after = evs[evs.len() - 1].cursor();
        }
    }

    /// Folds one replay page.
    pub(crate) fn replay_page(&mut self, evs: &[Event]) {
        self.replaying = true;
        for ev in evs {
            self.apply(ev);
            let last = self.last.entry(ev.device_id.clone()).or_default();
            if ev.seq > *last {
                *last = ev.seq;
            }
        }
        let now = self.stamp();
        self.prune(&now);
    }

    pub(crate) fn end_replay(&mut self) {
        self.read_board_view();
        self.renumber();
        self.relink();
        self.replaying = false;
    }

    /// The owner table for lease rows and mirrored messages.
    fn writer_ok(&mut self, ev: &Event) -> bool {
        if self.on_board && ev.actor == SWEEPER && !self.sweeper_ok(ev) {
            return false;
        }
        match ev.kind.as_str() {
            kinds::LEASE_ACQUIRED => match row::<LeaseAcquiredRow>(&ev.payload) {
                Some(r) => self.may_write(&r.lease.holder, &ev.device_id),
                None => true,
            },
            kinds::LEASE_RELEASED => match row::<LeaseReleasedRow>(&ev.payload) {
                Some(p) => self.may_write(&p.holder, &ev.device_id),
                None => true,
            },
            kinds::LEASE_REFUSED => match row::<Refusal>(&ev.payload) {
                Some(r) => self.may_write(&r.by, &ev.device_id),
                None => true,
            },
            kinds::TASK_COMMENT => {
                let Some(p) = row::<TaskCommentRow>(&ev.payload) else {
                    return true;
                };
                if !p.msg_id.is_empty() {
                    return may_record(&p.by, &p.to, p.gone || p.fill, &ev.device_id);
                }
                if !p.blocker_id.is_empty() && !self.person_actor(ev) {
                    return owner::may_write(&p.by, &ev.device_id);
                }
                true
            }
            _ => true,
        }
    }

    /// Folds one row, then any rows the take lock and the board's
    /// references kept back that it lets through now.
    pub(crate) fn apply(&mut self, ev: &Event) {
        self.apply_row(ev);
        if !self.replaying {
            self.relink();
        }
    }

    /// [`Fold::apply`] short of the relink.
    pub(crate) fn apply_row(&mut self, ev: &Event) {
        self.fold(ev);
        if let Some(p) = row::<TaskIdRow>(&ev.payload) {
            self.sync_offer_stale(&p.task_id);
        }
        self.fold_deferred();
        self.retry_aside();
        // After both: a claim either let through may be the holder a kept
        // pr.state row waited for (card #548), and replay has no tick.
        self.retry_pr_ahead();
    }

    pub(crate) fn fold(&mut self, ev: &Event) {
        if !self.writer_ok(ev) {
            // A kept pr.state row isn't refused yet (card #548).
            if !self.hold_pr_row(ev) {
                self.ignore(&ev.device_id);
            }
            return;
        }
        if !self.aside_gate(ev) {
            return;
        }
        let (ok, wait) = self.ask_gate(ev);
        if !ok {
            if !wait.is_empty() {
                self.asks.defer_row(&wait, ev);
            }
            return;
        }
        self.fold_kind(ev);
        if let Some(p) = row::<TaskIdRow>(&ev.payload) {
            self.note_pr_vouchers(&p.task_id);
        }
    }

    /// Folds one row the writer rules, the aside and the take lock let in.
    fn fold_kind(&mut self, ev: &Event) {
        let wall = gotime::hlc_wall(ev.hlc);
        let at = |t: GoTime| if t.is_zero() { wall } else { t };
        match ev.kind.as_str() {
            kinds::TASK_CREATED => self.fold_created(ev, &at),
            kinds::SPACE_BOARD_CREATED | kinds::SPACE_BOARD_RENAMED => self.fold_space_board(ev),
            kinds::DEVICE_SIDES => {
                // Private log only: a board log never carries one.
                if !self.on_board && self.sides.apply(ev) {
                    self.index_runs();
                }
            }
            kinds::TASK_CLAIMED => {
                let Some(p) = row::<TaskClaimedRow>(&ev.payload) else {
                    return;
                };
                if p.by.is_empty() {
                    return;
                }
                let Some(mut t) = self.tasks.remove(&p.task_id) else {
                    return;
                };
                add_holder(&mut t, &p.by);
                t.assigned = remove_str(&t.assigned, &p.by);
                sync_holder(&mut t);
                if t.state == SUBMITTED {
                    t.state = WORKING.into();
                }
                t.updated_at = at(p.at);
                t.updated_by = p.by.clone();
                let ti = Touch::of(&t);
                let when = t.updated_at;
                self.tasks.insert(p.task_id.clone(), t);
                self.touch(ev, &ti, when, &p.by, "took it up", true, Vec::new());
                self.note(Change {
                    kind: TASK_CHANGED.into(),
                    task_id: p.task_id,
                    holder: p.by,
                    ..Change::default()
                });
            }
            kinds::TASK_ASSIGNED => {
                let Some(p) = row::<TaskAssignedRow>(&ev.payload) else {
                    return;
                };
                if p.to.is_empty() {
                    return;
                }
                let gave = self.gave_to_person(ev, &p);
                let back = self.takes_back(ev, &p);
                let Some(mut t) = self.tasks.remove(&p.task_id) else {
                    return;
                };
                if let Some(person) = gave {
                    self.given_ever.insert(p.task_id.clone());
                    // A say and a question were about the last give.
                    super::cards::close_given_question(&mut t, at(p.at));
                    t.given = Some(Given {
                        key: p.to.clone(),
                        person,
                        by: p.by.clone(),
                        asker: String::new(),
                        question: String::new(),
                    });
                    t.given_said = None;
                } else if back {
                    // Its author takes it back: the way out when the person
                    // it went to is away for good.
                    super::cards::close_given_question(&mut t, at(p.at));
                    t.given = None;
                    t.given_said = None;
                }
                let engaged = is_holder(&t, &p.to) && !contains(&t.assigned, &p.to);
                add_holder(&mut t, &p.to);
                if !engaged && !contains(&t.assigned, &p.to) {
                    t.assigned.push(p.to.clone());
                }
                let note = trim_space(&p.note).to_string();
                if !engaged && !note.is_empty() {
                    t.handoffs.insert(p.to.clone(), note.clone());
                }
                sync_holder(&mut t);
                t.updated_at = at(p.at);
                t.updated_by = p.by.clone();
                // The one it went to hears the hand-off notice instead.
                let ti = Touch {
                    holders: Vec::new(),
                    ..Touch::of(&t)
                };
                let when = t.updated_at;
                self.tasks.insert(p.task_id.clone(), t);
                self.touch(
                    ev,
                    &ti,
                    when,
                    &p.by,
                    &if note.is_empty() {
                        format!("assigned it to {}", p.to)
                    } else {
                        format!("assigned it to {}: {}", p.to, cut(&note, MAX_HANDOFF_LINE))
                    },
                    true,
                    Vec::new(),
                );
                self.note(Change {
                    kind: TASK_CHANGED.into(),
                    task_id: p.task_id,
                    holder: p.to,
                    ..Change::default()
                });
            }
            kinds::TASK_STATUS => self.fold_status(ev, &at),
            kinds::TASK_JOINED | kinds::TASK_LEFT => {
                let Some(p) = row::<TaskMemberRow>(&ev.payload) else {
                    return;
                };
                if p.who.is_empty() {
                    return;
                }
                let Some(mut t) = self.tasks.remove(&p.task_id) else {
                    return;
                };
                let mut what = "left it".to_string();
                if ev.kind == kinds::TASK_JOINED {
                    if !contains(&t.holders, &p.who) {
                        t.holders.push(p.who.clone());
                    }
                    if t.state == SUBMITTED {
                        t.state = WORKING.into();
                    }
                    what = "joined".into();
                } else {
                    t.holders = remove_str(&t.holders, &p.who);
                    if !contains(&t.left, &p.who) {
                        t.left.push(p.who.clone());
                    }
                    // They came off it: the card is its author's again.
                    let kp = self.key_person.clone();
                    if t.given.as_ref().is_some_and(|g| {
                        g.key == p.who
                            || (p.reason == LEFT_REMOVED
                                && kp.as_ref().is_some_and(|f| f(&p.who) == g.person))
                    }) {
                        super::cards::close_given_question(&mut t, at(p.at));
                        t.given = None;
                        t.given_said = None;
                    }
                    if t.holders.is_empty() {
                        t.let_go = Some((p.who.clone(), at(p.at)));
                    }
                    if p.reason == LEFT_REMOVED {
                        what = "was taken off: their person left the space".into();
                    }
                    // A spawned agent of this card that went away without
                    // finishing crashed (card #447); any other leave of
                    // one breaks the run.
                    if self.spawned.get(&p.who).is_some_and(|m| m.task_id == t.id) {
                        if p.reason == LEFT_GONE {
                            t.spawn_crashes += 1;
                        } else {
                            t.spawn_crashes = 0;
                        }
                    }
                    if p.reason == LEFT_FINISHED {
                        what = "finished: the agent Irori started for it ended".into();
                    }
                    if p.reason == LEFT_GONE {
                        what = "went away".into();
                        if t.holders.is_empty() {
                            t.returned = Some(Returned {
                                was: p.who.clone(),
                                at: at(p.at),
                                person: p.person,
                                row: format!("{}:{}", ev.device_id, ev.seq),
                            });
                        }
                    }
                }
                if p.who != p.by {
                    what.push_str(&format!(" ({})", p.who));
                }
                t.assigned = remove_str(&t.assigned, &p.who);
                sync_holder(&mut t);
                t.updated_at = at(p.at);
                t.updated_by = p.by.clone();
                let ti = Touch::of(&t);
                let when = t.updated_at;
                self.tasks.insert(p.task_id.clone(), t);
                self.touch(ev, &ti, when, &p.by, &what, false, Vec::new());
                self.note(Change {
                    kind: TASK_CHANGED.into(),
                    task_id: p.task_id,
                    holder: p.who,
                    ..Change::default()
                });
            }
            kinds::TASK_SHARED => {
                let Some(p) = row::<TaskSharedRow>(&ev.payload) else {
                    return;
                };
                if let Some(t) = self.tasks.get_mut(&p.task_id) {
                    t.shared = p.shared;
                    t.updated_at = at(p.at);
                    t.updated_by = p.by;
                    let holder = t.holder.clone();
                    self.note(Change {
                        kind: TASK_CHANGED.into(),
                        task_id: p.task_id,
                        holder,
                        ..Change::default()
                    });
                }
            }
            kinds::TASK_LINKED => {
                let Some(p) = row::<TaskLinkedRow>(&ev.payload) else {
                    return;
                };
                if p.link.reference.is_empty() {
                    return;
                }
                let Some(t) = self.tasks.get_mut(&p.task_id) else {
                    return;
                };
                let i = t.links.iter().position(|l| l.reference == p.link.reference);
                match i {
                    Some(i) if p.removed => {
                        t.links.remove(i);
                    }
                    None if !p.removed && t.links.len() < super::links::MAX_LINKS => {
                        t.links.push(p.link.clone());
                    }
                    _ => return,
                }
                t.updated_at = at(p.at);
                t.updated_by = p.by.clone();
                let ti = Touch::of(t);
                let when = t.updated_at;
                let holder = t.holder.clone();
                let is_pr = super::links::pr_link(&p.link).is_some();
                let what = if p.removed {
                    format!("removed the link {}", p.link.label)
                } else {
                    format!("linked {}", p.link.label)
                };
                self.touch(ev, &ti, when, &p.by, &what, is_pr, Vec::new());
                self.note(Change {
                    kind: TASK_CHANGED.into(),
                    task_id: p.task_id,
                    holder,
                    ..Change::default()
                });
            }
            kinds::TASK_COMMENT => self.fold_comment(ev, &at),
            kinds::BLOCKER_CLOSED => {
                let Some(p) = row::<BlockerClosedRow>(&ev.payload) else {
                    return;
                };
                let Some(mut t) = self.tasks.remove(&p.task_id) else {
                    return;
                };
                // Card #635: a close names what its writer saw open. One
                // from another device can land after the card was unblocked
                // and blocked again (the person's done check): what it
                // names is closed already, so it changes nothing; or after
                // a new item was raised, which stays open.
                let ids = &p.blocker_ids;
                let named = |b: &Blocker| ids.is_empty() || ids.contains(&b.id);
                if t.state != BLOCKED || !t.blockers.iter().any(|b| b.open() && named(b)) {
                    self.tasks.insert(p.task_id, t);
                    return;
                }
                let when = at(p.at);
                let others = t.blockers.iter().any(|b| b.open() && !named(b));
                super::cards::close_named(&mut t, ids, &p.by, &p.note, when);
                let mut what = if others {
                    super::cards::sync_reason(&mut t);
                    "closed what it waited on".to_string()
                } else {
                    let mut s = p.restored.clone();
                    if !valid_state(&s) || s == BLOCKED || is_terminal(&s) {
                        s = restore_state(&t).into();
                    }
                    t.state = s;
                    t.reason.clear();
                    t.blocked_by = None;
                    if !p.note.is_empty() {
                        t.note = p.note.clone();
                    }
                    "unblocked it".to_string()
                };
                t.updated_at = when;
                t.updated_by = p.by.clone();
                if !p.note.is_empty() {
                    what.push_str(&format!(": {}", p.note));
                }
                let ti = Touch::of(&t);
                let holder = t.holder.clone();
                self.tasks.insert(p.task_id.clone(), t);
                self.touch(ev, &ti, when, &p.by, &what, true, Vec::new());
                self.note(Change {
                    kind: TASK_CHANGED.into(),
                    task_id: p.task_id,
                    holder,
                    ..Change::default()
                });
            }
            kinds::BLOCKER_OPTION
            | kinds::BLOCKER_RESOLVED
            | kinds::TASK_STEPS
            | kinds::TASK_STEP
            | kinds::TASK_WATCHED => self.apply_card(ev, &at),
            kinds::TASK_PINNED => self.fold_pin(ev),
            kinds::TASK_ARCHIVED => self.fold_archived(ev),
            kinds::TASK_PAUSED => self.fold_paused(ev),
            kinds::TASK_OWNER => self.fold_owner(ev),
            kinds::TASK_PLACED => self.fold_placed(ev),
            kinds::TASK_COMMENT_EDITED => self.fold_comment_edited(ev, &at),
            kinds::TASK_OFFERED => self.fold_offered(ev, &at),
            kinds::TASK_SPAWN => self.fold_spawn(ev, &at),
            kinds::TASK_SPAWNED => self.fold_spawned(ev, &at),
            // Where the work is (card #137): no thread line and no
            // updated_at, so a commit doesn't reorder the grid.
            kinds::TASK_WHERE => {
                let Some(p) = row::<TaskWhereRow>(&ev.payload) else {
                    return;
                };
                // Where goes in the brief: someone else's asks again (#573).
                let edited = self.edited_by_another(ev, &p.task_id, &p.who);
                let Some(t) = self.tasks.get_mut(&p.task_id) else {
                    return;
                };
                let w = Where {
                    who: p.who,
                    cwd: p.cwd,
                    worktree: p.worktree,
                    branch: p.branch,
                    head: p.head,
                    pushed: p.pushed,
                    at: at(p.at),
                };
                if t.where_
                    .as_ref()
                    .is_none_or(|o| !crate::gotime::after(&o.at, &w.at))
                {
                    t.where_ = Some(w);
                    if edited {
                        super::cards::close_given_question(t, at(p.at));
                        t.given_said = None;
                    }
                    let holder = t.holder.clone();
                    self.note(Change {
                        kind: TASK_CHANGED.into(),
                        task_id: p.task_id,
                        holder,
                        ..Change::default()
                    });
                }
            }
            kinds::WAIT_CREATED | kinds::WAIT_FIRED | kinds::WAIT_CANCELED => self.fold_wait(ev),
            kinds::TASK_DELETED => self.fold_deleted(ev),
            kinds::TASK_PARENT => {
                let Some(p) = row::<TaskParentRow>(&ev.payload) else {
                    return;
                };
                if p.task_id.is_empty() {
                    return;
                }
                self.fold_parent(&p.task_id, &p.parent, ev);
                if let Some(t) = self.tasks.get(&p.task_id) {
                    let ti = Touch::of(t);
                    let mut what = "took it out of its parent".to_string();
                    let mut also = Vec::new();
                    if !p.parent.is_empty() {
                        what = format!("made it part of {}", self.ref_of(&p.parent));
                        if let Some(pt) = self.tasks.get(&p.parent) {
                            also = pt.holders.clone();
                        }
                    }
                    self.touch(ev, &ti, at(p.at), &p.by, &what, true, also);
                }
            }
            kinds::TAKE_REQUESTED
            | kinds::TAKE_STATE
            | kinds::TASK_EDITED
            | kinds::BOARD_ASK_REQUESTED
            | kinds::BOARD_ASK_STATE
            | kinds::BOARD_ASK_EDITED => self.apply_take(ev, &at),
            kinds::ASK_MOVED | kinds::BOARD_ASK_MOVED => self.apply_ask_moved(ev),
            kinds::LEASE_REFUSED => {
                let Some(mut r) = row::<Refusal>(&ev.payload) else {
                    return;
                };
                if r.resource.is_empty() || r.by.is_empty() || r.held_by.is_empty() {
                    return;
                }
                let res =
                    parse_resource(&r.resource).unwrap_or_else(|_| Resource::named(&r.resource));
                r.at = at(r.at);
                if r.conflict {
                    self.end_conflict(&res.name, &r.by);
                }
                let (name, by) = (res.name.clone(), r.by.clone());
                self.refusals.push(RefusalRec { r, res });
                self.note(Change {
                    kind: LEASE_REFUSED.into(),
                    resource: name,
                    holder: by,
                    ..Change::default()
                });
            }
            kinds::LEASE_ACQUIRED => {
                let Some(r) = row::<LeaseAcquiredRow>(&ev.payload) else {
                    return;
                };
                if r.lease.resource.is_empty() {
                    return;
                }
                for v in &r.stolen {
                    if self.drop_lease(&v.resource, &v.holder) {
                        self.note(Change {
                            kind: LEASE_CHANGED.into(),
                            resource: v.resource.clone(),
                            holder: v.holder.clone(),
                            reason: REASON_STOLEN.into(),
                            ..Change::default()
                        });
                    }
                }
                let mut p = r.lease.clone();
                let res =
                    parse_resource(&p.resource).unwrap_or_else(|_| Resource::named(&p.resource));
                p.acquired_at = at(p.acquired_at);
                if self
                    .stolen
                    .contains(&stolen_key(&res.name, &p.holder, &p.acquired_at))
                {
                    return;
                }
                let mut rec = LeaseRec {
                    lease: p,
                    res,
                    dev: ev.device_id.clone(),
                };
                let name = rec.res.name.clone();
                if let Some(l) = self.conflicts.get(&name)
                    && l.lease.holder == rec.lease.holder
                {
                    rec.lease.conflict = l.lease.conflict.clone();
                    self.conflicts.insert(name, rec);
                    return;
                }
                if self.conflict_of(&mut rec, &r.stolen) {
                    return;
                }
                if let Some(old) = self.held.get(&name)
                    && old.lease.holder == rec.lease.holder
                {
                    rec.lease.conflict = old.lease.conflict.clone();
                }
                let holder = rec.lease.holder.clone();
                self.held.insert(name.clone(), rec);
                self.note(Change {
                    kind: LEASE_CHANGED.into(),
                    resource: name,
                    holder,
                    ..Change::default()
                });
            }
            kinds::LEASE_STOLEN => self.apply_stolen(ev),
            kinds::REVIEW_REQUESTED | kinds::REVIEW_REVIEWER => self.fold_review(ev, &at),
            kinds::NOTE_PROPOSED
            | kinds::NOTE_ACCEPTED
            | kinds::NOTE_EDITED
            | kinds::NOTE_REMOVED => self.fold_note(ev, &at),
            kinds::PR_STATE => self.fold_pr_state(ev),
            kinds::REPO_DESK | kinds::DESK_PRS | kinds::DESK_OK | kinds::REPO_SEEN => {
                self.fold_desk(ev)
            }
            // Old merge desk rows (card #418: the desk never merges):
            // desk.hold, desk.pause and desk.run are stored and folded past.
            kinds::LEASE_RELEASED => {
                let Some(p) = row::<LeaseReleasedRow>(&ev.payload) else {
                    return;
                };
                let name = normalize_resource(&p.resource).unwrap_or_else(|_| p.resource.clone());
                self.lapsed
                    .retain(|l| !(l.resource == name && l.holder == p.holder));
                if self.end_conflict(&name, &p.holder) {
                    self.note(Change {
                        kind: LEASE_CHANGED.into(),
                        resource: p.resource,
                        holder: p.holder,
                        reason: p.reason,
                        ..Change::default()
                    });
                }
            }
            _ => {}
        }
    }

    fn fold_created(&mut self, ev: &Event, at: &dyn Fn(GoTime) -> GoTime) {
        let Some(p) = row::<TaskCreatedRow>(&ev.payload) else {
            return;
        };
        if p.task_id.is_empty() {
            return;
        }
        if self.deleted.contains(&p.task_id) {
            // Its number still counts, or a device that heard the delete
            // first numbers other cards unlike the rest (card #563).
            self.claim_short(&p.task_id, RowPos::of(ev), p.num, &p.board);
            return;
        }
        let t = at(p.at);
        if !self.tasks.contains_key(&p.task_id) {
            self.next_order += 1;
            self.order.insert(p.task_id.clone(), self.next_order);
        }
        let mut links: Vec<Link> = Vec::new();
        for l in &p.links {
            if links.len() < super::links::MAX_LINKS
                && !l.reference.is_empty()
                && !links.iter().any(|x| x.reference == l.reference)
            {
                links.push(l.clone());
            }
        }
        let mut task = Task {
            id: p.task_id.clone(),
            title: p.title.clone(),
            objective: p.objective.clone(),
            done_criteria: p.done_criteria.clone(),
            // Cards from before #480 may carry any case; read them back in
            // the checkouts' spelling (the signed row stays as it is).
            repo_key: ledger::canonical_repo_key(&p.repo_key),
            scope: p.scope.clone(),
            state: SUBMITTED.into(),
            created_by: p.by.clone(),
            created_at: t,
            updated_at: t,
            updated_by: p.by.clone(),
            shared: p.shared,
            links,
            tag: self.tag_of(&p.board),
            board: self.own_board(&p.board),
            side: crate::sides::clean(&p.side),
            own_side: crate::sides::clean(&p.side),
            from: p.from.clone(),
            ..Task::default()
        };
        self.fold_ask_created(&mut task, p.ask.as_ref(), ev, &p.by);
        if self.on_board {
            self.origin.insert(p.task_id.clone(), ev.device_id.clone());
        }
        // A helper's part (spawned agents §3): marked only by the parent
        // owner's side.
        task.spawn = p.spawn
            && self
                .tasks
                .get(&p.parent)
                .filter(|_| !p.parent.is_empty())
                .is_some_and(|pt| self.may_spawn_on(pt, &ev.device_id));
        self.sync_pin(&mut task);
        self.tasks.insert(p.task_id.clone(), task);
        self.links_moved = true;
        if !p.parent.is_empty() {
            self.fold_parent(&p.task_id, &p.parent, ev);
        }
        self.claim_short(&p.task_id, RowPos::of(ev), p.num, &p.board);
        let ti = Touch::of(&self.tasks[&p.task_id]);
        match self.tasks.get(&p.parent).filter(|_| !p.parent.is_empty()) {
            Some(pt) => {
                let what = format!("put the card up as a subtask of {}", pt.short_ref());
                let also = pt.holders.clone();
                self.touch(ev, &ti, t, &p.by, &what, true, also);
            }
            None => {
                let what = match self.tasks.get(&p.from).filter(|_| !p.from.is_empty()) {
                    Some(th) => format!("put the card up from the thread {}", th.short_ref()),
                    None => "put the card up".to_string(),
                };
                self.touch(ev, &ti, t, &p.by, &what, false, Vec::new())
            }
        }
        if !p.from.is_empty() {
            self.fold_made(ev, &p.from, &p.task_id, t, &p.by, "made");
        }
        self.note(Change {
            kind: TASK_CHANGED.into(),
            task_id: p.task_id,
            ..Change::default()
        });
    }

    fn fold_status(&mut self, ev: &Event, at: &dyn Fn(GoTime) -> GoTime) {
        let Some(p) = row::<TaskStateRow>(&ev.payload) else {
            return;
        };
        if !valid_state(&p.state) || !self.tasks.contains_key(&p.task_id) {
            return;
        }
        if p.state == CANCELED
            && !self.may_cancel(
                &self.tasks[&p.task_id],
                &ev.device_id,
                Some(&RowPos::of(ev)),
            )
        {
            self.ignore(&ev.device_id);
            return;
        }
        if self.tasks[&p.task_id].withdrawn && !self.unwithdraws(&self.tasks[&p.task_id], &p, ev) {
            return;
        }
        let mut t = self.tasks.remove(&p.task_id).expect("checked");
        let was = t.state.clone();
        if p.state == BLOCKED {
            if was == IN_REVIEW {
                self.review_before_block.insert(t.id.clone());
            }
        } else {
            self.review_before_block.remove(&t.id);
        }
        let when = at(p.at);
        if p.state == CANCELED && p.withdrawn && self.on_board {
            t.withdrawn = true;
        } else if !is_terminal(&p.state) {
            t.withdrawn = false;
        }
        // The person reopens their resolved thread (card #169): its ask is
        // live again, and rows after this one fold as an open ask's.
        // On a board every member's fold must agree: the member who started
        // it, not this device's person (card #134).
        let starter = if self.on_board {
            // Or the member whose agent opened it (card #631).
            p.by.starts_with(kinds::PERSON_ACTOR_PREFIX)
                && (p.by == t.created_by
                    || t.ask
                        .as_ref()
                        .is_some_and(|a| self.openers_person(a, &p.by)))
        } else {
            self.is_owner(&p.by)
        };
        if p.reopen_ask
            && !is_terminal(&p.state)
            && starter
            && t.ask.as_ref().is_some_and(|a| a.thread && !a.is_open())
        {
            if let Some(a) = t.ask.as_mut() {
                a.outcome = None;
            }
            self.asks.closed_at.remove(&t.id);
            self.links_moved = true;
        } else if t.open_ask().is_some() {
            let o = p.outcome.clone().unwrap_or_else(|| Outcome {
                kind: super::asks::OUTCOME_NONE.into(),
                note: cmp_str(&p.note, "closed by the person"),
                ..Outcome::default()
            });
            self.close_ask_fold(&mut t, o, RowPos::of(ev));
        }
        // The person's say on the card (an answer putting it back, a
        // reopen) clears a run of crashed spawns (card #447).
        if self.person_row(ev, &p.by) {
            t.person_cleared_crashes();
        }
        // The question raised by two of the person's devices at once is one
        // question: the second row leaves the card as the first left it,
        // which may already be answered, and the lowest device acts on it.
        let again = self.given_raised(&mut t, &p, ev);
        if !again {
            t.state = p.state.clone();
            t.note = p.note.clone();
        }
        t.assigned = remove_str(&t.assigned, &p.by);
        sync_holder(&mut t);
        let mut what = String::new();
        let mut raised = String::new();
        let mut revised = false;
        let mut gist = String::new();
        if p.state == BLOCKED {
            let f = self.fold_blocked(&mut t, &p, ev, was == BLOCKED, when);
            what = f.what;
            raised = f.raised;
            revised = f.revised;
            gist = f.gist;
        } else {
            // The person's completed on a card waiting on its done check is
            // their Done (card #315): the check reads answered Done, not
            // closed, and the card goes straight to completed.
            let person = if self.on_board {
                p.by.starts_with(kinds::PERSON_ACTOR_PREFIX)
            } else {
                self.is_owner(&p.by)
            };
            if p.state == COMPLETED && person {
                super::cards::say_done_open(&mut t, &p.by, when);
            }
            super::cards::close_open(&mut t, &p.by, &p.note, when);
            if is_terminal(&was) && !is_terminal(&p.state) {
                what = "reopened it".into();
            } else if was != p.state {
                what = format!("set it to {}", p.state);
            }
            // The note says why, unless it only repeats what happened.
            let note = trim_space(&p.note);
            let verb = what.split(' ').next().unwrap_or_default().to_string();
            if what.is_empty() || note.is_empty() || what.contains(note) {
            } else if note.starts_with(&verb) {
                // "reopened by the person's comment", not "reopened it: reopened by …"
                what = cut(note, super::asks::MAX_LOG_QUOTE * 2);
            } else {
                what.push_str(&format!(": {}", cut(note, super::asks::MAX_LOG_QUOTE * 2)));
            }
        }
        super::cards::sync_reason(&mut t);
        t.updated_at = when;
        t.updated_by = p.by.clone();
        let mut line = line_of(ev, when, &p.by, &what);
        line.blocker_id = raised.clone();
        line.revised = revised;
        line.blocker_gist = gist;
        let ti = Touch::of(&t);
        let holder = t.holder.clone();
        if !raised.is_empty() {
            super::cards::refresh_blocker(&self.comments, self.by_task.get(&t.id), &mut t, &raised);
        }
        let person_blocker = !raised.is_empty() && !revised && {
            t.blockers
                .iter()
                .find(|b| b.id == raised)
                .is_some_and(|b| b.on.as_ref().is_none_or(|o| o.kind == BLOCKED_ON_PERSON))
        };
        self.tasks.insert(p.task_id.clone(), t);
        let wake = person_blocker
            || p.state == FAILED && was != FAILED
            || is_terminal(&p.state) && !is_terminal(&was) && self.last_subtask(&p.task_id);
        // Another agent's blocker or state change wakes the card's holders;
        // a reopen has its own notice.
        let reopened = is_terminal(&was) && !is_terminal(&p.state);
        let wake_holders = !raised.is_empty() || was != p.state;
        let ti = if reopened {
            Touch {
                holders: Vec::new(),
                ..ti
            }
        } else {
            ti
        };
        self.touch_parent(&ti, line, wake, wake_holders);
        self.waits_on_status(&p.task_id, &was, &p.state);
        self.note(Change {
            kind: TASK_CHANGED.into(),
            task_id: p.task_id,
            holder,
            ..Change::default()
        });
    }

    fn fold_comment(&mut self, ev: &Event, at: &dyn Fn(GoTime) -> GoTime) {
        let Some(mut p) = row::<TaskCommentRow>(&ev.payload) else {
            return;
        };
        if p.comment_id.is_empty() {
            return;
        }
        if self.comments.contains_key(&p.comment_id) {
            self.merge_comment(&p, ev, at);
            return;
        }
        if !p.msg_id.is_empty() {
            p.blocker_id.clear();
            p.pick = 0;
            p.missing.clear();
        }
        if !self.tasks.contains_key(&p.task_id) {
            return;
        }
        let primary = !p.gone && !p.fill;
        let person = self.person_row(ev, &p.by);
        let mut cm = Comment {
            id: p.comment_id.clone(),
            task_id: p.task_id.clone(),
            by: p.by.clone(),
            text: p.text.clone(),
            wake: p.wake,
            at: at(p.at),
            attachments: p.attachments.clone(),
            to: p.to.clone(),
            msg_id: p.msg_id.clone(),
            gone: p.gone,
            held: p.held,
            files: p.files,
            blocker_id: p.blocker_id.clone(),
            pick: p.pick.max(0),
            missing: p.missing.clone(),
            // A board log carries mentions in board form only (card #177);
            // anything else another device wrote is dropped here.
            mentions: if self.on_board {
                p.mentions
                    .iter()
                    .filter(|m| super::mention::board_form(&m.key))
                    .cloned()
                    .collect()
            } else {
                p.mentions.clone()
            },
            device: ev.device_id.clone(),
            hlc: ev.hlc.0,
            seq: ev.seq,
            primary,
            person,
            ..Comment::default()
        };
        cm.unheard = p.unheard && person;
        if self.verdict_counts(&p) {
            cm.verdict = p.verdict.clone();
        }
        if !primary && !p.held {
            cm.passed = p.text.clone();
        }
        let cm_at = cm.at;
        let cm_by = cm.by.clone();
        self.comments.insert(cm.id.clone(), cm);
        self.by_task
            .entry(p.task_id.clone())
            .or_default()
            .push(p.comment_id.clone());
        let t = self.tasks.get_mut(&p.task_id).expect("checked");
        t.comments += 1;
        if primary {
            t.updated_at = cm_at;
        }
        // An assignee's own comment takes the card up (card #85); a
        // recorded message or someone else's comment doesn't.
        if primary && p.msg_id.is_empty() && !person {
            take_up(t, &cm_by);
        }
        let ti = Touch::of(t);
        if !p.blocker_id.is_empty() {
            // The person reads the question, not the blocker's id (card
            // #490); the line still carries the id to link to it.
            let gist = t
                .blockers
                .iter()
                .find(|b| b.id == p.blocker_id)
                .map(|b| blocker_gist(&b.question))
                .unwrap_or_default();
            let what = match (person, gist.is_empty()) {
                (true, false) => format!("answered “{gist}”"),
                (false, false) => format!("replied on “{gist}”"),
                (true, true) => "answered a blocker".to_string(),
                (false, true) => "replied on a blocker".to_string(),
            };
            if let Some(cm) = self.comments.get_mut(&p.comment_id) {
                cm.blocker_gist = gist.clone();
            }
            let mut line = line_of(ev, cm_at, &cm_by, &what);
            line.blocker_id = p.blocker_id.clone();
            line.blocker_gist = gist;
            self.touch_line(&ti, line, false, Vec::new());
            let mut t = self.tasks.remove(&p.task_id).expect("checked");
            super::cards::refresh_blocker(
                &self.comments,
                self.by_task.get(&t.id),
                &mut t,
                &p.blocker_id,
            );
            let holder = t.holder.clone();
            self.tasks.insert(p.task_id.clone(), t);
            self.note(Change {
                kind: TASK_CHANGED.into(),
                task_id: p.task_id.clone(),
                holder,
                ..Change::default()
            });
        } else if p.msg_id.is_empty() {
            self.touch(ev, &ti, cm_at, &cm_by, "commented", false, Vec::new());
        }
        if primary && p.msg_id.is_empty() {
            self.mention_cards(ev, &p.task_id, &p.text, cm_at, &cm_by);
        }
        self.note(Change {
            kind: COMMENT_ADDED.into(),
            task_id: p.task_id,
            ..Change::default()
        });
    }

    /// Card mentions (card #114): each other card a comment names by its
    /// short ref (`#12`, `mlg#12`) gets a line, "mentioned this in #7",
    /// whoever wrote it. Derived here, so no row of its own: every device
    /// folds the same lines from the same comments.
    fn mention_cards(&mut self, ev: &Event, from: &str, text: &str, at: GoTime, by: &str) {
        let Some(src) = self.tasks.get(from).map(|t| t.short_ref()) else {
            return;
        };
        let mut done: HashSet<String> = HashSet::new();
        for (tag, n) in card_refs(text) {
            // What the ref means comes first: `#3` on card #3 names that
            // card, not another board's #3. A moved card's old ref still
            // finds it (card #563).
            let [t] = self.ref_hits(tag.as_deref(), &n)[..] else {
                continue;
            };
            if t.id == from || !done.insert(t.id.clone()) {
                continue;
            }
            let ti = Touch::of(t);
            let mut line = line_of(ev, at, by, &format!("mentioned this in {src}"));
            line.kind = MENTIONED_KIND.into();
            self.touch_line(&ti, line, true, Vec::new());
        }
    }

    /// Folds a row for a comment id already folded: only a mirrored
    /// message has more than one row (coord.md §3.7).
    fn merge_comment(&mut self, p: &TaskCommentRow, ev: &Event, at: &dyn Fn(GoTime) -> GoTime) {
        let Some(cm) = self.comments.get_mut(&p.comment_id) else {
            return;
        };
        if cm.msg_id.is_empty() || p.msg_id != cm.msg_id || p.task_id != cm.task_id {
            return;
        }
        let mut changed = false;
        if p.gone && !cm.gone {
            cm.gone = true;
            changed = true;
        }
        let primary = !p.gone && !p.fill;
        if !primary && !p.held && !p.text.is_empty() && cm.passed.is_empty() {
            cm.passed = p.text.clone();
            if cm.held {
                cm.text = p.text.clone();
                cm.held = false;
                changed = true;
            }
        }
        let mut moved = None;
        if primary && !cm.primary {
            cm.by = p.by.clone();
            cm.to = p.to.clone();
            cm.text = p.text.clone();
            cm.wake = p.wake;
            cm.at = at(p.at);
            cm.held = p.held;
            cm.files = p.files;
            cm.attachments = p.attachments.clone();
            cm.device = ev.device_id.clone();
            cm.hlc = ev.hlc.0;
            cm.seq = ev.seq;
            cm.primary = true;
            if cm.held && !cm.passed.is_empty() {
                cm.text = cm.passed.clone();
                cm.held = false;
            }
            moved = Some(cm.at);
            changed = true;
        }
        let task_id = cm.task_id.clone();
        if let Some(when) = moved
            && let Some(t) = self.tasks.get_mut(&task_id)
        {
            t.updated_at = when;
        }
        if changed {
            self.note(Change {
                kind: COMMENT_ADDED.into(),
                task_id,
                ..Change::default()
            });
        }
    }

    /// Removes resource's lease if holder still holds it.
    fn drop_lease(&mut self, resource: &str, holder: &str) -> bool {
        let name = normalize_resource(resource).unwrap_or_else(|_| resource.to_string());
        if self
            .held
            .get(&name)
            .is_some_and(|l| l.lease.holder == holder)
        {
            self.held.remove(&name);
            return true;
        }
        false
    }

    /// Forgets expired leases; this device's own lapsed leases are queued
    /// for their release row.
    pub(crate) fn prune(&mut self, now: &GoTime) {
        let names: Vec<String> = self.held.keys().cloned().collect();
        for name in names {
            let l = &self.held[&name];
            if self.live(l, now) {
                continue;
            }
            let l = self.held.remove(&name).expect("listed");
            if self.lease_local(&l)
                && (!self.replaying
                    || gotime::since(now, &l.lease.until) < LAPSED_WITHIN.as_nanos() as i128)
            {
                // Under its canonical name, the held key: a row from before
                // #480 may spell the repo in capitals (card #480).
                let mut lease = l.lease.clone();
                lease.resource = name.clone();
                self.lapsed.push(lease);
            }
            self.note(Change {
                kind: LEASE_EXPIRED.into(),
                resource: name,
                holder: l.lease.holder,
                ..Change::default()
            });
        }
        self.gc_refusals(now);
    }

    /// Writes `payload` as `by` and folds it.
    pub(crate) fn write<P: serde::Serialize + ?Sized>(
        &mut self,
        by: &str,
        kind: &str,
        payload: &P,
    ) -> Result<()> {
        let actor = self.actor_for(by);
        self.write_as(&actor, kind, payload)
    }

    /// `write` with the store's actor given.
    pub(crate) fn write_as<P: serde::Serialize + ?Sized>(
        &mut self,
        actor: &str,
        kind: &str,
        payload: &P,
    ) -> Result<()> {
        let Some(st) = self.st.clone() else {
            return Err(Error::raw(
                ErrorKind::Store,
                "coord: a read-only fold (no store)",
            ));
        };
        if self.on_board && self.holds_file.is_some() {
            self.offer_files(&st, payload)?;
        }
        let hop = match &self.hop_of {
            Some(f) if self.on_board && super::carries_hop(kind) => f(actor),
            _ => 0,
        };
        let ev = if hop > 0 {
            let mut v = serde_json::to_value(payload)
                .map_err(|e| Error::raw(ErrorKind::Store, format!("coord: {e}")))?;
            if let Some(o) = v.as_object_mut() {
                o.insert("hop".into(), hop.into());
            }
            st.append(actor, kind, &v)?
        } else {
            st.append(actor, kind, payload)?
        };
        self.apply(&ev);
        Ok(())
    }

    /// On a board: writes this device's `attach.offered` for every file
    /// `payload` names (its `attachments` and its blocker's) that this
    /// device holds and has not offered in this log yet, before the row
    /// that names it (v0.5 §5.5; the closure check wants the offer first).
    fn offer_files<P: serde::Serialize + ?Sized>(&self, st: &Store, payload: &P) -> Result<()> {
        let Some(holds) = &self.holds_file else {
            return Ok(());
        };
        let Ok(v) = serde_json::to_value(payload) else {
            return Ok(());
        };
        let mut files: Vec<&serde_json::Value> = Vec::new();
        for list in [v.get("attachments"), v.pointer("/blocker/attachments")]
            .into_iter()
            .flatten()
        {
            if let Some(a) = list.as_array() {
                files.extend(a.iter());
            }
        }
        if files.is_empty() {
            return Ok(());
        }
        let mut offered = offered_by(st, &self.dev)?;
        for f in files {
            let sha = f.get("sha256").and_then(|s| s.as_str()).unwrap_or_default();
            if sha.is_empty() || offered.contains(sha) || !holds(sha) {
                continue;
            }
            let s = |k: &str| f.get(k).and_then(|x| x.as_str()).unwrap_or_default();
            let row = OfferRow {
                sha256: sha,
                size: f.get("size").and_then(|x| x.as_i64()).unwrap_or(0),
                name: s("name"),
                mime: s("mime"),
            };
            st.append("irori", store::kinds::ATTACH_OFFERED, &row)?;
            offered.insert(sha.to_string());
        }
        Ok(())
    }

    /// A caller as the store's actor.
    pub(crate) fn actor_for(&self, by: &str) -> String {
        if by == SWEEPER {
            return SWEEPER.into();
        }
        if self.on_board {
            return by.into();
        }
        if self.is_owner(by) {
            return "user".into();
        }
        if by.starts_with("api:") {
            return by.into();
        }
        format!("agent:{by}")
    }

    /// Refreshes first-absent times for every holder that matters.
    pub(crate) fn observe(&mut self, now: &GoTime) {
        let Some(roster) = self.roster.clone() else {
            return;
        };
        let mut live = HashSet::new();
        let mut gone_at: HashMap<String, GoTime> = HashMap::new();
        for a in roster.agents() {
            // An agent not answering its wakes (#110) counts as away: its
            // cards can be taken over, and go back on the Board after the
            // holder grace, as a gone agent's do.
            if crate::roster::answers(&a.status) {
                live.insert(a.key);
            } else if a.status == "gone"
                && !a.updated_at.is_zero()
                && gotime::before(&a.updated_at, now)
            {
                gone_at.insert(a.key, a.updated_at);
            }
        }
        let mut tracked = HashSet::new();
        for t in self.tasks.values() {
            if is_terminal(&t.state) {
                continue;
            }
            for h in &t.holders {
                tracked.insert(h.clone());
            }
            // An agent's own thread hears its opener while it lives, held
            // or not (card #631).
            if let Some(a) = t.open_ask().filter(|a| a.thread && !a.opener.is_empty()) {
                tracked.insert(a.opener.clone());
            }
        }
        for l in self.held.values() {
            // A run holder (card #437) has no presence: its lease ends at
            // its until, which the run renews while it lives.
            if self.live(l, now) && !super::is_run_holder(&l.lease.holder) {
                tracked.insert(l.lease.holder.clone());
            }
        }
        for w in self.waits.values() {
            tracked.insert(w.by.clone());
        }
        self.first_absent.retain(|k, _| tracked.contains(k));
        for k in &live {
            self.gone_rows.remove(k);
        }
        for k in tracked {
            if self.is_owner(&k) || live.contains(&k) || !self.local_key(&k) {
                self.first_absent.remove(&k);
                continue;
            }
            let mut first = self.first_absent.get(&k).copied().unwrap_or(*now);
            for g in [gone_at.get(&k), self.gone_rows.get(&k)]
                .into_iter()
                .flatten()
            {
                if gotime::before(g, &first) {
                    first = *g;
                }
            }
            self.first_absent.insert(k, first);
        }
    }

    /// Whether key has been absent for the grace period.
    pub(crate) fn gone(&self, key: &str, now: &GoTime) -> bool {
        self.first_absent
            .get(key)
            .is_some_and(|t| gotime::at_least(now, t, self.grace))
    }

    /// Counts one row of `dev` ignored by the writer rules.
    pub(crate) fn ignore(&mut self, dev: &str) {
        self.ignored += 1;
        *self.ignored_by.entry(dev.to_string()).or_default() += 1;
    }

    // ---- across devices (mesh.go) ----

    /// Whether key lives on this device, or names none.
    pub(crate) fn local_key(&self, key: &str) -> bool {
        let dev = owner::device_of(key);
        dev.is_empty() || dev == self.dev
    }

    /// Whether l still holds at now: by its Until on this device; another
    /// device's always, unless that device was removed.
    /// On a board another person's lease also ends at its Until (plus
    /// the clock skew), so a member who went away can't hold a resource
    /// past it.
    pub(crate) fn live(&self, l: &LeaseRec, now: &GoTime) -> bool {
        if self.lease_local(l) {
            return gotime::after(&l.lease.until, now);
        }
        if self.on_board && !gotime::after(&gotime::add(&l.lease.until, CLOCK_SKEW), now) {
            return false;
        }
        match &self.revoked {
            None => true,
            Some(r) => !r(self.lease_dev(l)),
        }
    }

    /// The device a lease lives on: its holder's on the private board, the
    /// device that mirrored it on a shared one.
    pub(crate) fn lease_dev<'a>(&self, l: &'a LeaseRec) -> &'a str {
        if self.on_board {
            return &l.dev;
        }
        owner::device_of(&l.lease.holder)
    }

    /// Whether this device keeps the lease: refuses it when it loses,
    /// releases it when it lapses.
    pub(crate) fn lease_local(&self, l: &LeaseRec) -> bool {
        let dev = self.lease_dev(l);
        dev.is_empty() || dev == self.dev
    }

    /// Folds an acquire that meets a live lease on the same resource from
    /// another device; reports whether it handled the row.
    fn conflict_of(&mut self, rec: &mut LeaseRec, stolen: &[StolenLease]) -> bool {
        let name = rec.res.name.clone();
        let Some(e) = self.held.get(&name) else {
            return false;
        };
        if e.lease.holder == rec.lease.holder || self.lease_dev(e) == self.lease_dev(rec) {
            return false;
        }
        if stolen.iter().any(|v| v.holder == e.lease.holder) {
            return false;
        }
        if !self.live(e, &self.stamp()) {
            return false;
        }
        if !gotime::after(
            &gotime::add(&rec.lease.until, CLOCK_SKEW),
            &e.lease.acquired_at,
        ) {
            return true;
        }
        if !gotime::after(
            &gotime::add(&e.lease.until, CLOCK_SKEW),
            &rec.lease.acquired_at,
        ) {
            return false;
        }
        let mut e = self.held.remove(&name).expect("checked");
        // The lower device id's lease stands, across people too (v0.4
        // §6.4), so a clock can't win it.
        let rec_wins = self.lease_dev(rec) < self.lease_dev(&e);
        let (mut winner, mut loser) = if rec_wins {
            (rec.clone(), e.clone())
        } else {
            (std::mem::replace(&mut e, rec.clone()), rec.clone())
        };
        winner.lease.conflict = loser.lease.holder.clone();
        loser.lease.conflict = winner.lease.holder.clone();
        let wh = winner.lease.holder.clone();
        self.held.insert(name.clone(), winner);
        self.conflicts.insert(name.clone(), loser);
        self.note(Change {
            kind: LEASE_CHANGED.into(),
            resource: name,
            holder: wh,
            reason: REASON_CONFLICT.into(),
            ..Change::default()
        });
        true
    }

    /// Folds lease.stolen: only the person's count.
    fn apply_stolen(&mut self, ev: &Event) {
        let Some(p) = row::<LeaseStolenRow>(&ev.payload) else {
            return;
        };
        if p.resource.is_empty() || p.holder.is_empty() {
            return;
        }
        // Nobody steals another person's lease (§5.4).
        if !self.person_row(ev, &p.by)
            || self.on_board && self.person_of(&p.by) != self.person_of(&p.holder)
        {
            self.ignore(&ev.device_id);
            return;
        }
        let name = normalize_resource(&p.resource).unwrap_or_else(|_| p.resource.clone());
        self.stolen
            .insert(stolen_key(&name, &p.holder, &p.acquired_at));
        let matches = |l: Option<&LeaseRec>| {
            l.is_some_and(|l| {
                l.lease.holder == p.holder && l.lease.acquired_at.equal(&p.acquired_at)
            })
        };
        if matches(self.conflicts.get(&name)) || matches(self.held.get(&name)) {
            self.end_conflict(&name, &p.holder);
            self.note(Change {
                kind: LEASE_CHANGED.into(),
                resource: name,
                holder: p.holder,
                reason: REASON_STOLEN.into(),
                ..Change::default()
            });
        }
    }

    /// Ends holder's lease on name, held or contested; the contested one
    /// stands alone when the standing one ends.
    pub(crate) fn end_conflict(&mut self, name: &str, holder: &str) -> bool {
        if self
            .conflicts
            .get(name)
            .is_some_and(|l| l.lease.holder == holder)
        {
            self.conflicts.remove(name);
            if let Some(w) = self.held.get_mut(name) {
                w.lease.conflict.clear();
            }
            return true;
        }
        if self
            .held
            .get(name)
            .is_some_and(|l| l.lease.holder == holder)
        {
            self.held.remove(name);
            if let Some(mut o) = self.conflicts.remove(name) {
                o.lease.conflict.clear();
                self.held.insert(name.to_string(), o);
            }
            return true;
        }
        false
    }

    // ---- shared boards (boardlog.go) ----

    /// A row the person wrote as themselves. On a board that is any
    /// member's `person:` row from a device of that member, so one
    /// person's rows count on the other's devices too.
    pub(crate) fn person_row(&self, ev: &Event, by: &str) -> bool {
        if self.on_board {
            if ev.actor != by || !by.starts_with(kinds::PERSON_ACTOR_PREFIX) {
                return false;
            }
            return match (&self.key_person, &self.dev_person) {
                (Some(kp), Some(dp)) => {
                    let p = kp(by);
                    !p.is_empty() && p == dp(&ev.device_id)
                }
                _ => self.is_owner(by),
            };
        }
        ev.actor == "user" && self.is_owner(by)
    }

    /// A row an agent wrote as itself from its own device (card #631): on
    /// a board, a member's agent from a device of that member; privately,
    /// an agent on the device that wrote it. Never a person or Irori.
    pub(crate) fn agent_own_row(&self, ev: &Event, by: &str) -> bool {
        if by.is_empty()
            || by == SWEEPER
            || self.is_person(by)
            || super::threads::names_person(by)
            || by.starts_with("api:")
        {
            return false;
        }
        if self.on_board {
            return ev.actor == by && self.may_write(by, &ev.device_id);
        }
        ev.actor == format!("agent:{by}")
            && owner::may_write(by, &ev.device_id)
            && !owner::device_of(by).is_empty()
    }

    /// See [`super::Coord::written_as`].
    pub(crate) fn written_as(&self, by: &str, dev: &str) -> Option<String> {
        let (true, Some(kp), Some(dp)) = (self.on_board, &self.key_person, &self.dev_person) else {
            return None;
        };
        let p = dp(dev);
        // Irori's lines are its own only from this person's devices: from
        // another member's they are that member's.
        let mismatch = if by == SWEEPER {
            p != kp(&self.person)
        } else {
            kp(by) != p
        };
        (!p.is_empty() && mismatch).then(|| format!("{}{p}", kinds::PERSON_ACTOR_PREFIX))
    }

    /// The owner rule for an owner-only row.
    pub(crate) fn may_write(&self, owner_key: &str, writer: &str) -> bool {
        match (self.on_board, &self.key_person, &self.dev_person) {
            (true, Some(kp), Some(dp)) => {
                let p = kp(owner_key);
                !p.is_empty() && p == dp(writer)
            }
            _ => owner::may_write(owner_key, writer),
        }
    }

    /// The board's short handle, `""` on the private board.
    pub(crate) fn tag(&self) -> String {
        match (&self.tag_fn, self.on_board) {
            (Some(f), true) => f(),
            _ => String::new(),
        }
    }

    /// The tag of board `id` in this space (card #392), Personal included
    /// (card #558); None when the space holds no such board.
    pub(crate) fn board_tag(&self, id: &str) -> Option<String> {
        self.board_tag_fn.as_ref().and_then(|f| f(id))
    }

    /// A `space.board.*` row: a card that folded before its board's row
    /// took the space's tag. A replay reads the whole log, so it needs
    /// nothing here.
    fn fold_space_board(&mut self, ev: &Event) {
        if self.replaying {
            return;
        }
        let Some(p) = row::<SpaceBoardIdRow>(&ev.payload) else {
            return;
        };
        let tag = self.tag_of(&p.board_id);
        let stale: Vec<String> = self
            .tasks
            .values()
            .filter(|t| t.board == p.board_id && t.tag != tag)
            .map(|t| t.id.clone())
            .collect();
        for id in stale {
            let Some(t) = self.tasks.get_mut(&id) else {
                continue;
            };
            t.tag = tag.clone();
            let holder = t.holder.clone();
            self.note(Change {
                kind: TASK_CHANGED.into(),
                task_id: id,
                holder,
                ..Change::default()
            });
        }
    }

    /// The tag a card on board `board` (empty: the space's own) shows.
    pub(crate) fn tag_of(&self, board: &str) -> String {
        if board.is_empty() {
            return self.tag();
        }
        self.board_tag(board).unwrap_or_else(|| self.tag())
    }

    /// Whether `tag` names this space's own board or any the log has named
    /// inside it: a board with no cards has its tag all the same.
    pub(crate) fn has_tag(&self, tag: &str) -> bool {
        tag == self.tag()
            || self
                .board_tags_fn
                .as_ref()
                .is_some_and(|f| f().iter().any(|t| t == tag))
    }

    /// A cancel on a shared card counts only from a device of the card
    /// creator's person or of a person who owned the board at the row's
    /// place (`at`; `None` is a write now).
    pub(crate) fn may_cancel(&self, t: &Task, writer: &str, at: Option<&RowPos>) -> bool {
        let (true, Some(kp), Some(dp)) = (self.on_board, &self.key_person, &self.dev_person) else {
            return true;
        };
        let p = dp(writer);
        if p.is_empty() {
            return false;
        }
        p == kp(&t.created_by) || self.board_owner.as_ref().is_some_and(|f| f(&p, at))
    }

    pub(crate) fn sweeper_ok(&self, ev: &Event) -> bool {
        let Some(p) = row::<TaskIdRow>(&ev.payload) else {
            return false;
        };
        // A row that waits beside the one that lets it in is checked when
        // it folds from there, not dropped now (card #573).
        if self.ahead_of_give(ev) || self.behind_held(ev, &self.aside) {
            return true;
        }
        match ev.kind.as_str() {
            kinds::LEASE_RELEASED => true,
            // The take lock: its fold counts only the asking device's.
            kinds::BOARD_ASK_STATE => true,
            // Its fold checks who may take the ask (card #541).
            kinds::BOARD_ASK_MOVED => true,
            // A hand-out counts from the device that runs it: the one that
            // made the card (card #174).
            kinds::TASK_OFFERED => self.origin.get(&p.task_id) == Some(&ev.device_id),
            kinds::TASK_LEFT => self.may_write(&p.who, &ev.device_id),
            // What GitHub said about a card's PR counts from a device of a
            // person whose agent holds the card: the PR poller runs there
            // (card #208). Without it the row never folded on a board and
            // every poll heard "newly merged" again (#210).
            // Or, for a PR an external wait is on (card #509), from a device
            // of the person whose agent raised the wait: its poller asks
            // whoever holds the card.
            // A row ahead of its card waits beside it (card #542): rows come
            // device by device, and nothing writes a settled PR's row again.
            // One ahead of the card's holder or wait is kept and looked at
            // again (card #548, `hold_pr_row`).
            // Card #548: anyone who ever held it or raised the wait, so the
            // row counts whether it folds before or after they went.
            kinds::PR_STATE => self.tasks.get(&p.task_id).is_none_or(|_| {
                let pr = row::<PrStateRow>(&ev.payload)
                    .map(|r| r.pr)
                    .unwrap_or_default();
                self.pr_vouchers.get(&p.task_id).is_some_and(|vs| {
                    vs.iter().any(|(for_pr, key)| {
                        (for_pr.is_empty() || *for_pr == pr) && self.may_write(key, &ev.device_id)
                    })
                })
            }),
            // A wait that ended by itself (a lease released, card #339)
            // closes from a device of the person whose agent raised it:
            // that device sees what it waited on.
            // Of the items it names, when it names them (card #635).
            kinds::BLOCKER_CLOSED => self.tasks.get(&p.task_id).is_some_and(|t| {
                let ids = row::<BlockerClosedRow>(&ev.payload)
                    .map(|r| r.blocker_ids)
                    .unwrap_or_default();
                t.blockers.iter().any(|b| {
                    b.open()
                        && (ids.is_empty() || ids.contains(&b.id))
                        && self.may_write(&b.raised_by, &ev.device_id)
                })
            }),
            // A spawned agent's request and mark (card #455): from a device
            // of the person it serves, which spawn_writer_ok checks itself.
            kinds::TASK_SPAWN | kinds::TASK_SPAWNED => true,
            // Irori's line on a card about its spawned agent (ended on a
            // usage limit, no hand-off, a kept worktree; card #576): from a
            // device of the card's author, the only one that runs its agent.
            kinds::TASK_COMMENT => {
                let (Some(kp), Some(dp)) = (&self.key_person, &self.dev_person) else {
                    return false;
                };
                self.tasks.get(&p.task_id).is_some_and(|t| {
                    let author = kp(&t.created_by);
                    !author.is_empty() && author == dp(&ev.device_id)
                })
            }
            // Binding a spawned agent assigns the card to it: from a device
            // of the person it serves (the card's author, or who it was
            // given to, card #573), and only to the agent marked as started
            // for this card in this log.
            kinds::TASK_ASSIGNED => {
                let Some(a) = row::<TaskAssignedRow>(&ev.payload) else {
                    return false;
                };
                let Some(dp) = &self.dev_person else {
                    return false;
                };
                self.tasks.get(&p.task_id).is_some_and(|t| {
                    let serves = self.serving_person(t);
                    !serves.is_empty()
                        && serves == dp(&ev.device_id)
                        && self.spawned.get(&a.to).is_some_and(|s| s.task_id == t.id)
                })
            }
            kinds::BLOCKER_RESOLVED if self.raiser_closes(ev) => true,
            kinds::TASK_STATUS | kinds::BLOCKER_RESOLVED => match self.tasks.get(&p.task_id) {
                None => true,
                // Or a given card's person's device putting it back when
                // their agent left it (card #573): a state that isn't an
                // end, or closing a blocker they or their agent raised.
                // Never the giver's card's end or the giver's questions.
                Some(t) => {
                    self.may_cancel(t, &ev.device_id, Some(&RowPos::of(ev)))
                        || self.given_puts_back(t, ev)
                }
            },
            _ => false,
        }
    }

    /// Whether ev, Irori's row from a device of the person card t was
    /// given to, is one that puts the card back after their agent left.
    fn given_puts_back(&self, t: &Task, ev: &Event) -> bool {
        let (Some(g), Some(kp), Some(dp)) = (&t.given, &self.key_person, &self.dev_person) else {
            return false;
        };
        if dp(&ev.device_id) != g.person {
            return false;
        }
        match ev.kind.as_str() {
            kinds::TASK_STATUS => {
                row::<TaskStateRow>(&ev.payload).is_some_and(|p| !is_terminal(&p.state))
            }
            kinds::BLOCKER_RESOLVED => row::<BlockerResolvedRow>(&ev.payload).is_some_and(|p| {
                p.closed
                    && t.blockers
                        .iter()
                        .any(|b| b.id == p.blocker_id && kp(&b.raised_by) == g.person)
            }),
            _ => false,
        }
    }

    /// A sweeper's blocker.resolved that closes one item, written by a
    /// device of the person whose agent raised it (card #339).
    fn raiser_closes(&self, ev: &Event) -> bool {
        let Some(p) = row::<BlockerResolvedRow>(&ev.payload) else {
            return false;
        };
        p.closed
            && self.tasks.get(&p.task_id).is_some_and(|t| {
                t.blockers
                    .iter()
                    .any(|b| b.id == p.blocker_id && self.may_write(&b.raised_by, &ev.device_id))
            })
    }

    fn unwithdraws(&self, t: &Task, p: &TaskStateRow, ev: &Event) -> bool {
        !is_terminal(&p.state)
            && self.person_actor(ev)
            && self.may_cancel(t, &ev.device_id, Some(&RowPos::of(ev)))
    }

    /// A row whose actor is a person.
    pub(crate) fn person_actor(&self, ev: &Event) -> bool {
        if self.on_board {
            return ev.actor.starts_with(kinds::PERSON_ACTOR_PREFIX);
        }
        ev.actor == "user"
    }

    /// The person at this device as a caller.
    pub(crate) fn self_key(&self) -> String {
        if self.on_board {
            return self.person.clone();
        }
        format!("user:{}", self.dev)
    }

    /// Whether ev can't fold yet; `earlier` are the rows already waiting
    /// that came before it.
    fn unresolved(&self, ev: &Event, earlier: &[Event]) -> bool {
        self.st.as_ref().is_some_and(|s| s.unresolved(ev).is_err())
            || self.ahead_of_give(ev)
            || self.behind_held(ev, earlier)
    }

    /// Sets ev aside when it names something the board log doesn't hold
    /// yet; true means fold it now.
    pub(crate) fn aside_gate(&mut self, ev: &Event) -> bool {
        if !self.on_board || !self.unresolved(ev, &self.aside) {
            return true;
        }
        if self.aside.len() >= MAX_ASIDE {
            self.aside_drop += 1;
            return false;
        }
        self.aside.push(ev.clone());
        false
    }

    /// Folds every set-aside row that resolves now, until a pass folds none.
    pub(crate) fn retry_aside(&mut self) {
        while !self.aside.is_empty() {
            let rows = std::mem::take(&mut self.aside);
            // Which rows waited for a give as this pass began: a give that
            // folds mid-pass mustn't let a device's later row past its
            // earlier ones, so those wait for the next pass (card #573).
            let held: Vec<bool> = rows.iter().map(|ev| self.ahead_of_give(ev)).collect();
            let mut behind: Vec<(String, String)> = Vec::new();
            let mut keep = Vec::new();
            let mut folded = false;
            for (ev, held) in rows.into_iter().zip(held) {
                let card = row::<TaskIdRow>(&ev.payload)
                    .map(|p| (ev.device_id.clone(), p.task_id))
                    .filter(|(_, id)| !id.is_empty());
                let after = ev.kind != kinds::TASK_CREATED
                    && card.as_ref().is_some_and(|c| behind.contains(c));
                if after || self.unresolved(&ev, &keep) {
                    if held && let Some(c) = card {
                        behind.push(c);
                    }
                    keep.push(ev);
                    continue;
                }
                folded = true;
                self.fold(&ev);
            }
            // Rows set aside while folding join the ones kept.
            keep.append(&mut self.aside);
            self.aside = keep;
            if !folded {
                return;
            }
        }
    }

    // ---- subscribe (subscribe.go) ----

    /// Queues a change for subscribers (none during replay).
    pub(crate) fn note(&mut self, ch: Change) {
        if !self.replaying {
            self.pending.push(ch);
        }
    }

    /// Hands queued changes to subscribers (Go's `unlock`).
    pub(crate) fn flush(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        self.subs.retain(|s| s.q.strong_count() > 0);
        for x in pending {
            for s in &mut self.subs {
                let Some(q) = s.q.upgrade() else { continue };
                let msg = if s.lost {
                    Change {
                        kind: RESYNC.into(),
                        ..Change::default()
                    }
                } else {
                    x.clone()
                };
                s.lost = !q.push(msg);
            }
        }
    }

    /// The soonest Until among this device's leases while anyone is
    /// subscribed: when the M4 expiry timer should fire (Go armExpiry).
    pub(crate) fn next_expiry(&self) -> Option<GoTime> {
        if self.subs.iter().all(|s| s.q.strong_count() == 0) {
            return None;
        }
        self.held
            .values()
            .filter(|l| self.lease_local(l))
            .map(|l| l.lease.until)
            .min_by(gotime::cmp)
    }
}

/// The writer rule for a mirrored message's row.
pub(crate) fn may_record(by: &str, to: &str, by_recipient: bool, dev: &str) -> bool {
    owner::may_write(by, dev) || by_recipient && owner::may_write(to, dev)
}

/// Records who as working t: added once on a shared task, else replaces.
pub(crate) fn add_holder(t: &mut Task, who: &str) {
    if !t.shared {
        t.holders = vec![who.to_string()];
    } else if !contains(&t.holders, who) {
        t.holders.push(who.to_string());
    }
    sync_holder(t);
}

/// Keeps holder = holders[0] and assigned a subset of holders.
pub(crate) fn sync_holder(t: &mut Task) {
    let holders = t.holders.clone();
    // One that came back on the card is no longer gone from it (card #481).
    t.left.retain(|w| !holders.contains(w));
    t.assigned.retain(|a| holders.contains(a));
    let offered = t.assigned.clone();
    t.handoffs.retain(|k, _| offered.contains(k));
    sync_co_holders(t);
    if t.holders.is_empty() {
        t.holder.clear();
        return;
    }
    t.holder = t.holders[0].clone();
    t.returned = None;
}

/// Derives co_holders: owners of open steps who don't hold the card,
/// sorted (card #86).
pub(crate) fn sync_co_holders(t: &mut Task) {
    let mut co: Vec<String> = t
        .steps
        .iter()
        .filter(|s| !s.done && !s.owner.is_empty() && !t.holders.contains(&s.owner))
        .map(|s| s.owner.clone())
        .collect();
    co.sort();
    co.dedup();
    t.co_holders = co;
    // Once someone is on it again, Irori's own "nobody is on it" note no
    // longer holds (card #490); a note anyone else wrote stays.
    if (!t.holders.is_empty() || !t.co_holders.is_empty()) && is_nobody_note(&t.note) {
        t.note.clear();
    }
}

/// A blocker's question as a thread line quotes it (card #490).
pub fn blocker_gist(question: &str) -> String {
    cut(&collapse_space(question), super::asks::MAX_LOG_QUOTE)
}

/// The start of Irori's own notes for a card nobody holds (card #490):
/// comments.rs writes them when its holder leaves or goes away.
pub(crate) const NOBODY_NOTE: &str = "nobody is on it";
/// The start of back_on_board's note (cards.rs).
pub(crate) const BACK_NOTE: &str = "back on the Board";

/// Says the note is one of Irori's own "nobody holds it" notes.
pub(crate) fn is_nobody_note(n: &str) -> bool {
    n.starts_with(NOBODY_NOTE) || n.starts_with(BACK_NOTE)
}

pub(crate) fn is_holder(t: &Task, key: &str) -> bool {
    contains(&t.holders, key)
}

/// Holds t and has taken it up.
pub(crate) fn engaged(t: &Task, key: &str) -> bool {
    is_holder(t, key) && !contains(&t.assigned, key)
}

/// Where unblocking leaves a task.
impl Fold {
    /// The state an unblock writes back (card #191): in_review when the
    /// card was in review before it was blocked and a holder is still on
    /// it, unless the blocker closing is the done check (the person's
    /// "not done" sends the work back); else [`restore_state`].
    pub(crate) fn restore_for(&self, t: &Task) -> &'static str {
        let s = restore_state(t);
        if s == WORKING
            && self.review_before_block.contains(&t.id)
            && !t.blockers.iter().any(|b| b.open() && b.done_check)
        {
            return IN_REVIEW;
        }
        s
    }
}

pub(crate) fn restore_state(t: &Task) -> &'static str {
    if t.holders.iter().any(|h| !contains(&t.assigned, h)) {
        WORKING
    } else {
        SUBMITTED
    }
}

/// The activity line row ev makes.
/// The activity kind of a card-mention line (card #114). Not a row kind:
/// the line is folded from a `task.comment`.
pub(crate) const MENTIONED_KIND: &str = "task.mentioned";

/// The activity kind of a thread's "made #12 from this thread" line (card
/// #155). Not a row kind: folded from the card's `task.created` or
/// `task.edited`.
pub const MADE_KIND: &str = "task.made";

/// The card refs in text: `#12` (tag None) or `mlg#12`, each a whole word
/// (start, whitespace or "(" before; not a digit or letter after). The
/// number is returned as written.
pub(crate) fn card_refs(text: &str) -> Vec<(Option<String>, String)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    for (i, &c) in b.iter().enumerate() {
        if c != b'#' {
            continue;
        }
        let mut j = i + 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j == i + 1 || (j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_')) {
            continue;
        }
        let mut k = i;
        while k > 0 && (b[k - 1].is_ascii_alphanumeric() || b[k - 1] == b'-' || b[k - 1] == b'_') {
            k -= 1;
        }
        if k > 0 && !(b[k - 1].is_ascii_whitespace() || b[k - 1] == b'(') {
            continue;
        }
        let tag = (k < i).then(|| text[k..i].to_string());
        out.push((tag, text[i + 1..j].to_string()));
    }
    out
}

pub(crate) fn line_of(ev: &Event, at: GoTime, by: &str, what: &str) -> Activity {
    Activity {
        at,
        by: by.into(),
        what: what.into(),
        kind: ev.kind.clone(),
        device: ev.device_id.clone(),
        hlc: ev.hlc.0,
        seq: ev.seq,
        ..Activity::default()
    }
}

/// An `attach.offered` payload (mesh `FileOffer`'s shape).
#[derive(serde::Serialize)]
struct OfferRow<'a> {
    sha256: &'a str,
    size: i64,
    #[serde(skip_serializing_if = "str::is_empty")]
    name: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    mime: &'a str,
}

/// The hashes device `dev` offered in `st` (its `attach.offered` rows).
fn offered_by(st: &Store, dev: &str) -> Result<HashSet<String>> {
    #[derive(serde::Deserialize, Default)]
    #[serde(default)]
    struct P {
        sha256: String,
    }
    let mut out = HashSet::new();
    let mut f = store::Filter {
        kinds: vec![store::kinds::ATTACH_OFFERED.into()],
        device_id: dev.into(),
        limit: store::MAX_LIST_LIMIT,
        ..Default::default()
    };
    loop {
        let evs = st.list(&f)?;
        for ev in &evs {
            if let Ok(p) = serde_json::from_slice::<P>(&ev.payload) {
                out.insert(p.sha256);
            }
        }
        if (evs.len() as i64) < f.limit {
            return Ok(out);
        }
        f.after = evs.last().map(Event::cursor).unwrap_or_default();
    }
}
