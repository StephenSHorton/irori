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
use super::util::{SWEEPER, cmp_str, contains, remove_str};
use super::{MAX_ASIDE, Options};
use crate::gotime;
use crate::roster::Agents;

/// Release reasons in lease.released rows.
pub const REASON_RELEASED: &str = "released";
pub const REASON_STOLEN: &str = "stolen";
pub const REASON_HOLDER_GONE: &str = "holder_gone";
pub const REASON_EXPIRED: &str = "expired";
pub const REASON_CONFLICT: &str = "conflict";

/// A task.left reason: Irori took a holder that went away off the card.
pub const LEFT_GONE: &str = "gone";

/// Which leases found lapsed at replay still get their expiry row.
const LAPSED_WITHIN: Duration = Duration::from_secs(24 * 3600);
/// How far two devices' wall clocks may differ.
const CLOCK_SKEW: Duration = Duration::from_secs(120);
/// Each subscriber's queue size.
pub const SUB_BUFFER: usize = 64;

/// The row prefixes coord folds.
pub const FOLLOW_KINDS: [&str; 6] = [
    "task.",
    "lease.",
    "blocker.",
    "take.",
    "board.ask.",
    "review.",
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
}

/// The bits of a task `touch` needs, so a handler can touch while it
/// holds the task.
pub(crate) struct Touch {
    pub id: String,
    pub parent: String,
    pub has_watchers: bool,
    pub short_ref: String,
}

impl Touch {
    pub fn of(t: &Task) -> Touch {
        Touch {
            id: t.id.clone(),
            parent: t.parent.clone(),
            has_watchers: !t.watchers.is_empty(),
            short_ref: t.short_ref(),
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
    pub(crate) on_board: bool,
    pub(crate) person: String,
    pub(crate) key_person: Option<StrFn>,
    pub(crate) dev_person: Option<StrFn>,
    pub(crate) tag_fn: Option<Arc<dyn Fn() -> String + Send + Sync>>,
    pub(crate) board_owner: Option<super::OwnerAtFn>,
    pub(crate) hop_of: Option<super::HopFn>,
    pub(crate) ignored: usize,
    pub(crate) ignored_by: BTreeMap<String, usize>,
    pub(crate) aside: Vec<Event>,
    pub(crate) aside_drop: usize,

    pub(crate) tasks: BTreeMap<String, Task>,
    pub(crate) order: HashMap<String, u64>,
    pub(crate) next_order: u64,
    pub(crate) held: BTreeMap<String, LeaseRec>,
    pub(crate) first_absent: HashMap<String, GoTime>,
    pub(crate) comments: HashMap<String, Comment>,
    pub(crate) by_task: HashMap<String, Vec<String>>,
    pub(crate) refusals: Vec<RefusalRec>,
    pub(crate) blocker_seq: HashMap<String, i64>,
    pub(crate) step_seq: HashMap<String, i64>,
    pub(crate) activity: HashMap<String, Vec<Activity>>,
    pub(crate) watchq: BTreeMap<String, WatchQueue>,

    pub(crate) claims: Vec<ShortClaim>,
    pub(crate) claimed: HashSet<String>,
    pub(crate) short_groups: BTreeMap<i64, Vec<ShortClaim>>,
    pub(crate) short_idx: HashMap<String, String>,
    pub(crate) short_top: i64,
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
    pub(crate) pins: HashMap<String, PinRec>,
    /// A board's review requests by card, and its dedicated reviewers.
    pub(crate) reviews: BTreeMap<String, Vec<super::review::ReviewRequest>>,
    pub(crate) reviewers: super::review::Reviewers,
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
            on_board,
            person,
            key_person: o.key_person,
            dev_person: o.device_person,
            tag_fn: o.tag,
            board_owner: o.board_owner,
            hop_of: o.hop_of,
            ignored: 0,
            ignored_by: BTreeMap::new(),
            aside: Vec::new(),
            aside_drop: 0,
            tasks: BTreeMap::new(),
            order: HashMap::new(),
            next_order: 0,
            held: BTreeMap::new(),
            first_absent: HashMap::new(),
            comments: HashMap::new(),
            by_task: HashMap::new(),
            refusals: Vec::new(),
            blocker_seq: HashMap::new(),
            step_seq: HashMap::new(),
            activity: HashMap::new(),
            watchq: BTreeMap::new(),
            claims: Vec::new(),
            claimed: HashSet::new(),
            short_groups: BTreeMap::new(),
            short_idx: HashMap::new(),
            short_top: 0,
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
            pins: HashMap::new(),
            reviews: BTreeMap::new(),
            reviewers: BTreeMap::new(),
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
        Ok(())
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
        self.fold(ev);
        self.fold_deferred();
        self.retry_aside();
        if !self.replaying {
            self.relink();
        }
    }

    pub(crate) fn fold(&mut self, ev: &Event) {
        if !self.writer_ok(ev) {
            self.ignore(&ev.device_id);
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
        let wall = gotime::hlc_wall(ev.hlc);
        let at = |t: GoTime| if t.is_zero() { wall } else { t };
        match ev.kind.as_str() {
            kinds::TASK_CREATED => self.fold_created(ev, &at),
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
                let Some(mut t) = self.tasks.remove(&p.task_id) else {
                    return;
                };
                let engaged = is_holder(&t, &p.to) && !contains(&t.assigned, &p.to);
                add_holder(&mut t, &p.to);
                if !engaged && !contains(&t.assigned, &p.to) {
                    t.assigned.push(p.to.clone());
                }
                sync_holder(&mut t);
                t.updated_at = at(p.at);
                t.updated_by = p.by.clone();
                let ti = Touch::of(&t);
                let when = t.updated_at;
                self.tasks.insert(p.task_id.clone(), t);
                self.touch(
                    ev,
                    &ti,
                    when,
                    &p.by,
                    &format!("assigned it to {}", p.to),
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
                if t.state != BLOCKED {
                    self.tasks.insert(p.task_id, t);
                    return;
                }
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
                t.updated_at = at(p.at);
                t.updated_by = p.by.clone();
                let when = t.updated_at;
                super::cards::close_open(&mut t, &p.by, &p.note, when);
                let mut what = "unblocked it".to_string();
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
            repo_key: p.repo_key.clone(),
            scope: p.scope.clone(),
            state: SUBMITTED.into(),
            created_by: p.by.clone(),
            created_at: t,
            updated_at: t,
            updated_by: p.by.clone(),
            shared: p.shared,
            links,
            tag: self.tag(),
            ..Task::default()
        };
        self.fold_ask_created(&mut task, p.ask.as_ref(), ev, &p.by);
        self.sync_pin(&mut task);
        self.tasks.insert(p.task_id.clone(), task);
        if !p.parent.is_empty() {
            self.fold_parent(&p.task_id, &p.parent, ev);
        }
        self.claim_short(&p.task_id, RowPos::of(ev), p.num);
        let ti = Touch::of(&self.tasks[&p.task_id]);
        match self.tasks.get(&p.parent).filter(|_| !p.parent.is_empty()) {
            Some(pt) => {
                let what = format!("put the card up as a subtask of {}", pt.short_ref());
                let also = pt.holders.clone();
                self.touch(ev, &ti, t, &p.by, &what, true, also);
            }
            None => self.touch(ev, &ti, t, &p.by, "put the card up", false, Vec::new()),
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
        let when = at(p.at);
        if p.state == CANCELED && p.withdrawn && self.on_board {
            t.withdrawn = true;
        } else if !is_terminal(&p.state) {
            t.withdrawn = false;
        }
        if t.open_ask().is_some() {
            let o = p.outcome.clone().unwrap_or_else(|| Outcome {
                kind: super::asks::OUTCOME_NONE.into(),
                note: cmp_str(&p.note, "closed by the person"),
                ..Outcome::default()
            });
            self.close_ask_fold(&mut t, o, RowPos::of(ev));
        }
        t.state = p.state.clone();
        t.note = p.note.clone();
        t.assigned = remove_str(&t.assigned, &p.by);
        sync_holder(&mut t);
        let mut what = String::new();
        let mut raised = String::new();
        if p.state == BLOCKED {
            let (w, r) = self.fold_blocked(&mut t, &p, ev, was == BLOCKED, when);
            what = w;
            raised = r;
        } else {
            super::cards::close_open(&mut t, &p.by, &p.note, when);
            if is_terminal(&was) && !is_terminal(&p.state) {
                what = "reopened it".into();
            } else if was != p.state {
                what = format!("set it to {}", p.state);
            }
        }
        super::cards::sync_reason(&mut t);
        t.updated_at = when;
        t.updated_by = p.by.clone();
        let mut line = line_of(ev, when, &p.by, &what);
        line.blocker_id = raised.clone();
        let ti = Touch::of(&t);
        let holder = t.holder.clone();
        if !raised.is_empty() {
            super::cards::refresh_blocker(&self.comments, self.by_task.get(&t.id), &mut t, &raised);
        }
        self.tasks.insert(p.task_id.clone(), t);
        self.touch_line(&ti, line, true, Vec::new());
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
            device: ev.device_id.clone(),
            hlc: ev.hlc.0,
            seq: ev.seq,
            primary,
            person,
            ..Comment::default()
        };
        cm.unheard = p.unheard && person;
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
        let ti = Touch::of(t);
        if !p.blocker_id.is_empty() {
            let what = if person {
                format!("answered blocker {}", p.blocker_id)
            } else {
                format!("replied on blocker {}", p.blocker_id)
            };
            let mut line = line_of(ev, cm_at, &cm_by, &what);
            line.blocker_id = p.blocker_id.clone();
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
        self.note(Change {
            kind: COMMENT_ADDED.into(),
            task_id: p.task_id,
            ..Change::default()
        });
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
                self.lapsed.push(l.lease.clone());
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
            if a.status != "gone" {
                live.insert(a.key);
            } else if !a.updated_at.is_zero() && gotime::before(&a.updated_at, now) {
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
        }
        for l in self.held.values() {
            if self.live(l, now) {
                tracked.insert(l.lease.holder.clone());
            }
        }
        self.first_absent.retain(|k, _| tracked.contains(k));
        for k in tracked {
            if self.is_owner(&k) || live.contains(&k) || !self.local_key(&k) {
                self.first_absent.remove(&k);
                continue;
            }
            let mut first = self.first_absent.get(&k).copied().unwrap_or(*now);
            if let Some(g) = gone_at.get(&k)
                && gotime::before(g, &first)
            {
                first = *g;
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

    fn sweeper_ok(&self, ev: &Event) -> bool {
        let Some(p) = row::<TaskIdRow>(&ev.payload) else {
            return false;
        };
        match ev.kind.as_str() {
            kinds::LEASE_RELEASED => true,
            // The take lock: its fold counts only the asking device's.
            kinds::BOARD_ASK_STATE => true,
            kinds::TASK_LEFT => self.may_write(&p.who, &ev.device_id),
            kinds::TASK_STATUS | kinds::BLOCKER_RESOLVED => match self.tasks.get(&p.task_id) {
                None => true,
                Some(t) => self.may_cancel(t, &ev.device_id, Some(&RowPos::of(ev))),
            },
            _ => false,
        }
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

    fn unresolved(&self, ev: &Event) -> bool {
        self.st.as_ref().is_some_and(|s| s.unresolved(ev).is_err())
    }

    /// Sets ev aside when it names something the board log doesn't hold
    /// yet; true means fold it now.
    pub(crate) fn aside_gate(&mut self, ev: &Event) -> bool {
        if !self.on_board || !self.unresolved(ev) {
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
            let mut keep = Vec::new();
            let mut folded = false;
            for ev in rows {
                if self.unresolved(&ev) {
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
    t.assigned.retain(|a| holders.contains(a));
    if t.holders.is_empty() {
        t.holder.clear();
        return;
    }
    t.holder = t.holders[0].clone();
    t.returned = None;
}

pub(crate) fn is_holder(t: &Task, key: &str) -> bool {
    contains(&t.holders, key)
}

/// Holds t and has taken it up.
pub(crate) fn engaged(t: &Task, key: &str) -> bool {
    is_holder(t, key) && !contains(&t.assigned, key)
}

/// Where unblocking leaves a task.
pub(crate) fn restore_state(t: &Task) -> &'static str {
    if t.holders.iter().any(|h| !contains(&t.assigned, h)) {
        WORKING
    } else {
        SUBMITTED
    }
}

/// The activity line row ev makes.
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
