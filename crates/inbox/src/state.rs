//! The fold (I:internal/inbox/state.go): rows, the in-memory state, replay,
//! `emit` and the `unlock` that turns touched agents into WakeReady and
//! AppendReady edges.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use gocompat::json::{self, Out, Schema};
use gocompat::sub::Queue;
use gocompat::time::GoTime;
use serde::Serialize;
use store::{Cursor, Event, Filter, MAX_LIST_LIMIT, Store};

use crate::ids::IdGen;
use crate::tm;
use crate::{
    API_KEY_PREFIX, AgentCard, Change, ChangeKind, Delivery, Error, FileRef, GONE_GRACE,
    KeyPredicate, MAX_ALIAS_LEN, MAX_ALIASES, MAX_WAKE_TRIES, Message, NO_CARD_GRACE, NowFn,
    OnGoneFn, OnPassedFn, Options, Policy, PrOwnersFn, QUIET_LIMIT, REATTACH_GRACE, Result, Roster,
    SUB_BUFFER, SYSTEM_KEY, StaleFn, UNENGAGED_LIMIT, Wake, kinds, mode, path, policy_mode, st,
};

pub(crate) const WAKE_TOKEN_PREFIX: &str = "[irori-wake:";
pub(crate) const REMINDER_TOKEN_PREFIX: &str = "[irori-reminder:";
pub(crate) const APPEND_TOKEN_PREFIX: &str = "[irori-append:";

/// The replicated rows New folds.
pub(crate) const REPLAY_KINDS: &[&str] = &[
    kinds::MESSAGE_SENT,
    kinds::DELIVERY_STATE,
    kinds::WAKE_ISSUED,
    kinds::AGENT_ALIAS,
    kinds::AGENT_NAME,
    kinds::TURN_ENGAGED,
    kinds::TURN_ENDED,
    kinds::AGENT_SEEN,
    kinds::DELIVERY_DECIDED,
    kinds::RECEIVE_SET,
    kinds::AUTONOMY_SET,
];

/// The device-local rows New folds.
const LOCAL_REPLAY_KINDS: &[&str] = &[kinds::POLICY_SET];

fn is_false(b: &bool) -> bool {
    !*b
}

fn zero(t: &GoTime) -> bool {
    t.is_zero()
}

// ---- rows -----------------------------------------------------------------

/// The delivery.state payload: a full snapshot of the mutable fields.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct DeliveryRow {
    #[serde(rename = "deliveryId")]
    pub delivery_id: String,
    #[serde(rename = "messageId")]
    pub message_id: String,
    pub to: String,
    #[serde(rename = "repoKey", skip_serializing_if = "String::is_empty")]
    pub repo_key: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub mode: String,
    pub state: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub hold: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub via: String,
    #[serde(rename = "wakeId", skip_serializing_if = "String::is_empty")]
    pub wake_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(rename = "for", skip_serializing_if = "String::is_empty")]
    pub for_: String,
    #[serde(skip_serializing_if = "zero")]
    pub until: GoTime,
    #[serde(rename = "movedTo", skip_serializing_if = "String::is_empty")]
    pub moved_to: String,
    #[serde(rename = "movedFrom", skip_serializing_if = "String::is_empty")]
    pub moved_from: String,
    #[serde(skip_serializing_if = "is_false")]
    pub reminded: bool,
    #[serde(rename = "engagedOnce", skip_serializing_if = "is_false")]
    pub engaged_once: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub appended: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub by: String,
    #[serde(skip_serializing_if = "zero")]
    pub at: GoTime,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct WakeRow {
    #[serde(rename = "wakeId")]
    pub wake_id: String,
    pub to: String,
    pub token: String,
    pub reminder: bool,
    #[serde(rename = "deliveryIds")]
    pub delivery_ids: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub mode: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub overflow: Vec<String>,
    #[serde(skip_serializing_if = "zero")]
    pub at: GoTime,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct AliasRow {
    pub key: String,
    pub alias: String,
    #[serde(skip_serializing_if = "zero")]
    pub at: GoTime,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct NameRow {
    pub key: String,
    pub name: String,
    #[serde(skip_serializing_if = "zero")]
    pub at: GoTime,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct PolicyRow {
    pub mode: String,
    pub block: Vec<String>,
    pub allow: Vec<String>,
    pub paused: bool,
    pub by: String,
    #[serde(skip_serializing_if = "zero")]
    pub at: GoTime,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct TurnRow {
    pub agent: String,
    #[serde(rename = "wakeId")]
    pub wake_id: String,
    #[serde(rename = "turnId")]
    pub turn_id: String,
    #[serde(skip_serializing_if = "zero")]
    pub at: GoTime,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct DecidedRow {
    #[serde(rename = "deliveryId")]
    pub delivery_id: String,
    pub choice: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub to: String,
    pub by: String,
    #[serde(rename = "atDevice", skip_serializing_if = "String::is_empty")]
    pub at_device: String,
    #[serde(skip_serializing_if = "zero")]
    pub at: GoTime,
    #[serde(skip_serializing_if = "is_false")]
    pub undo: bool,
}

// ---- decoding (Go json.Unmarshal into the row structs) ---------------------

pub(crate) fn s(out: &Out, name: &str) -> String {
    out.get_str(name).to_string()
}

pub(crate) fn b(out: &Out, name: &str) -> bool {
    matches!(out.get(name), Some(Out::Bool(true)))
}

pub(crate) fn i(out: &Out, name: &str) -> i64 {
    match out.get(name) {
        Some(Out::Int(n)) => *n,
        _ => 0,
    }
}

pub(crate) fn t(out: &Out, name: &str) -> GoTime {
    match out.get(name) {
        Some(Out::Time(t)) => *t,
        _ => GoTime::ZERO,
    }
}

pub(crate) fn strs(out: &Out, name: &str) -> Option<Vec<String>> {
    match out.get(name) {
        Some(Out::Slice(Some(v))) => Some(v.iter().map(|x| x.str().to_string()).collect()),
        _ => None,
    }
}

pub(crate) fn str_fields(names: &[&'static str]) -> Vec<(&'static str, Schema)> {
    names.iter().map(|n| (*n, Schema::String)).collect()
}

fn message_schema() -> Schema {
    let mut f = vec![
        ("id", Schema::String),
        ("from", Schema::object(str_fields(&["key", "name", "kind"]))),
        ("to", Schema::object(str_fields(&["kind", "value"]))),
    ];
    f.extend(str_fields(&["text", "reason", "replyTo", "parentId"]));
    f.push(("hop", Schema::Int(64)));
    f.push(("trust", Schema::String));
    f.push(("sentAt", Schema::Time));
    f.push(("ref", Schema::String));
    f.push(("notice", Schema::Bool));
    f.push(("board", crate::bridge::board_from_schema()));
    f.push((
        "attachments",
        Schema::Slice(Box::new(Schema::object(vec![
            ("sha256", Schema::String),
            ("name", Schema::String),
            ("size", Schema::Int(64)),
            ("mime", Schema::String),
            ("device", Schema::String),
        ]))),
    ));
    Schema::object(f)
}

pub(crate) fn decode_message(payload: &[u8]) -> std::result::Result<Message, json::Error> {
    let o = json::unmarshal(payload, &message_schema())?;
    let sub = |name: &str| o.get(name).cloned().unwrap_or(Out::Struct(vec![]));
    let from = sub("from");
    let to = sub("to");
    let attachments = o
        .get("attachments")
        .map(|a| {
            a.items()
                .iter()
                .map(|f| FileRef {
                    sha256: s(f, "sha256"),
                    name: s(f, "name"),
                    size: i(f, "size"),
                    mime: s(f, "mime"),
                    device: s(f, "device"),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Message {
        id: s(&o, "id"),
        from: crate::Sender {
            key: s(&from, "key"),
            name: s(&from, "name"),
            kind: s(&from, "kind"),
        },
        to: crate::Address {
            kind: s(&to, "kind"),
            value: s(&to, "value"),
        },
        text: s(&o, "text"),
        reason: s(&o, "reason"),
        reply_to: s(&o, "replyTo"),
        parent_id: s(&o, "parentId"),
        hop: i(&o, "hop"),
        trust: s(&o, "trust"),
        sent_at: t(&o, "sentAt"),
        ref_: s(&o, "ref"),
        notice: b(&o, "notice"),
        board: o
            .get("board")
            .map(crate::bridge::decode_board_from)
            .unwrap_or_default(),
        attachments,
        writer: String::new(),
    })
}

fn delivery_schema() -> Schema {
    let mut f = str_fields(&[
        "deliveryId",
        "messageId",
        "to",
        "repoKey",
        "mode",
        "state",
        "hold",
        "via",
        "wakeId",
        "note",
        "for",
    ]);
    f.push(("until", Schema::Time));
    f.extend(str_fields(&["movedTo", "movedFrom"]));
    f.push(("reminded", Schema::Bool));
    f.push(("engagedOnce", Schema::Bool));
    f.push(("appended", Schema::Bool));
    f.push(("by", Schema::String));
    f.push(("at", Schema::Time));
    Schema::object(f)
}

pub(crate) fn decode_delivery(payload: &[u8]) -> std::result::Result<DeliveryRow, json::Error> {
    let o = json::unmarshal(payload, &delivery_schema())?;
    Ok(DeliveryRow {
        delivery_id: s(&o, "deliveryId"),
        message_id: s(&o, "messageId"),
        to: s(&o, "to"),
        repo_key: s(&o, "repoKey"),
        mode: s(&o, "mode"),
        state: s(&o, "state"),
        hold: s(&o, "hold"),
        via: s(&o, "via"),
        wake_id: s(&o, "wakeId"),
        note: s(&o, "note"),
        for_: s(&o, "for"),
        until: t(&o, "until"),
        moved_to: s(&o, "movedTo"),
        moved_from: s(&o, "movedFrom"),
        reminded: b(&o, "reminded"),
        engaged_once: b(&o, "engagedOnce"),
        appended: b(&o, "appended"),
        by: s(&o, "by"),
        at: t(&o, "at"),
    })
}

fn decode_wake(payload: &[u8]) -> std::result::Result<WakeRow, json::Error> {
    let mut f = str_fields(&["wakeId", "to", "token"]);
    f.push(("reminder", Schema::Bool));
    f.push(("deliveryIds", Schema::Slice(Box::new(Schema::String))));
    f.extend(str_fields(&["path", "mode"]));
    f.push(("overflow", Schema::Slice(Box::new(Schema::String))));
    f.push(("at", Schema::Time));
    let o = json::unmarshal(payload, &Schema::object(f))?;
    Ok(WakeRow {
        wake_id: s(&o, "wakeId"),
        to: s(&o, "to"),
        token: s(&o, "token"),
        reminder: b(&o, "reminder"),
        delivery_ids: strs(&o, "deliveryIds").unwrap_or_default(),
        path: s(&o, "path"),
        mode: s(&o, "mode"),
        overflow: strs(&o, "overflow").unwrap_or_default(),
        at: t(&o, "at"),
    })
}

fn decode_kv(
    payload: &[u8],
    k: &'static str,
    v: &'static str,
) -> std::result::Result<(String, String, GoTime), json::Error> {
    let mut f = str_fields(&[k, v]);
    f.push(("at", Schema::Time));
    let o = json::unmarshal(payload, &Schema::object(f))?;
    Ok((s(&o, k), s(&o, v), t(&o, "at")))
}

fn decode_policy(payload: &[u8]) -> std::result::Result<PolicyRow, json::Error> {
    let f = vec![
        ("mode", Schema::String),
        ("block", Schema::Slice(Box::new(Schema::String))),
        ("allow", Schema::Slice(Box::new(Schema::String))),
        ("paused", Schema::Bool),
        ("by", Schema::String),
        ("at", Schema::Time),
    ];
    let o = json::unmarshal(payload, &Schema::object(f))?;
    Ok(PolicyRow {
        mode: s(&o, "mode"),
        block: strs(&o, "block").unwrap_or_default(),
        allow: strs(&o, "allow").unwrap_or_default(),
        paused: b(&o, "paused"),
        by: s(&o, "by"),
        at: t(&o, "at"),
    })
}

fn decode_turn(payload: &[u8]) -> std::result::Result<TurnRow, json::Error> {
    let mut f = str_fields(&["agent", "wakeId", "turnId"]);
    f.push(("at", Schema::Time));
    let o = json::unmarshal(payload, &Schema::object(f))?;
    Ok(TurnRow {
        agent: s(&o, "agent"),
        wake_id: s(&o, "wakeId"),
        turn_id: s(&o, "turnId"),
        at: t(&o, "at"),
    })
}

pub(crate) fn decode_decided(payload: &[u8]) -> std::result::Result<DecidedRow, json::Error> {
    let mut f = str_fields(&["deliveryId", "choice", "to", "by", "atDevice"]);
    f.push(("at", Schema::Time));
    f.push(("undo", Schema::Bool));
    let o = json::unmarshal(payload, &Schema::object(f))?;
    Ok(DecidedRow {
        delivery_id: s(&o, "deliveryId"),
        choice: s(&o, "choice"),
        to: s(&o, "to"),
        by: s(&o, "by"),
        at_device: s(&o, "atDevice"),
        at: t(&o, "at"),
        undo: b(&o, "undo"),
    })
}

/// presence.AgentCard as Go decodes an agent.seen row: the whole struct
/// (a type error anywhere skips the row), of which key and name are used.
fn decode_seen(payload: &[u8]) -> Option<(String, String)> {
    let mut f = str_fields(&["key", "deviceId", "agent", "sessionId"]);
    f.push(("pid", Schema::Int(64)));
    f.extend(str_fields(&[
        "name",
        "title",
        "nameSource",
        "aiTitle",
        "cwd",
        "status",
        "sessionKind",
        "entrypoint",
        "version",
    ]));
    f.push(("peerFeatures", Schema::Slice(Box::new(Schema::String))));
    f.push(("startedAt", Schema::Time));
    f.push(("updatedAt", Schema::Time));
    f.push(("lastActivity", Schema::Time));
    f.extend(str_fields(&[
        "lastText",
        "transcript",
        "repoKey",
        "permissionMode",
        "deviceState",
    ]));
    let o = json::unmarshal(payload, &Schema::object(f)).ok()?;
    Some((s(&o, "key"), s(&o, "name")))
}

fn decode_target(payload: &[u8], field: &'static str) -> Option<String> {
    let o = json::unmarshal(payload, &Schema::object(vec![(field, Schema::String)])).ok()?;
    Some(s(&o, field))
}

// ---- state ----------------------------------------------------------------

/// A delivery plus the sweeper's bookkeeping. `delivery.message` stays
/// empty; [`Inner::copy_out`] fills it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Drec {
    pub d: Delivery,
    pub moved_from: String,
    pub assigned_at: GoTime,
    pub engaged_once: bool,
    pub first_writer: String,
    pub last_writer: String,
    /// Memory only: this device, the host, ran hold() on another device's
    /// queued row.
    pub host_checked: bool,
    pub wake_tries: i64,
    pub appended: bool,
}

impl Drec {
    pub fn next_turn(&self) -> bool {
        self.d.mode == mode::NEXT_TURN
    }

    /// A wake item its host's mod was handed MaxWakeTries times without
    /// confirming one (card #23).
    pub fn untaken(&self) -> bool {
        !self.next_turn() && self.wake_tries >= MAX_WAKE_TRIES
    }

    /// Goes out by append on the mod path, not by a wake.
    pub fn for_append(&self) -> bool {
        self.next_turn() || self.untaken()
    }

    /// The row to change and append.
    pub fn row(&self) -> DeliveryRow {
        let d = &self.d;
        DeliveryRow {
            delivery_id: d.id.clone(),
            message_id: d.message_id.clone(),
            to: d.to.clone(),
            repo_key: d.repo_key.clone(),
            mode: d.mode.clone(),
            state: d.state.clone(),
            hold: d.hold.clone(),
            via: d.via.clone(),
            wake_id: d.wake_id.clone(),
            note: d.note.clone(),
            for_: d.for_.clone(),
            until: d.until,
            moved_to: d.moved_to.clone(),
            moved_from: self.moved_from.clone(),
            reminded: d.reminded,
            engaged_once: self.engaged_once,
            appended: self.appended,
            by: String::new(),
            at: GoTime::ZERO,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WakeRec {
    pub w: Wake,
    pub path: String,
    pub issued_at: GoTime,
}

impl WakeRec {
    pub fn is_append(&self) -> bool {
        self.w.token.starts_with(APPEND_TOKEN_PREFIX)
    }

    /// Holds back the agent's next wake until delivered and engaged.
    pub fn gates(&self) -> bool {
        !self.w.reminder && !self.is_append() && self.w.mode != mode::NEXT_TURN
    }
}

/// An agent's wake paths (memory only).
#[derive(Debug, Clone, Default)]
pub(crate) struct ReachRec {
    pub md: bool,
    pub channel: bool,
    pub mod_lost: GoTime,
    pub first_at: GoTime,
}

#[derive(Debug, Clone)]
pub(crate) struct PassRec {
    pub msg: String,
    pub to: String,
    pub since: GoTime,
}

/// Limits with defaults filled in, in nanoseconds.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub unengaged: i128,
    pub gone: i128,
    pub quiet: i128,
    pub reattach: i128,
    pub no_card: i128,
}

pub(crate) struct Inner {
    pub st: Arc<Store>,
    pub roster: Arc<dyn Roster>,
    pub owners: Option<PrOwnersFn>,
    pub on_gone: Option<OnGoneFn>,
    pub on_passed: Option<OnPassedFn>,
    pub stale: Option<StaleFn>,
    pub dev: String,
    pub owner_fn: KeyPredicate,
    pub ignored: i64,
    pub ignored_by: BTreeMap<String, i64>,
    pub last: BTreeMap<String, i64>,
    pub handled: HashSet<String>,
    pub requests: Vec<DecidedRow>,

    pub now_fn: NowFn,
    pub lim: Limits,
    pub ids: IdGen,

    pub msgs: HashMap<String, Message>,
    pub dels: HashMap<String, Drec>,
    pub order: Vec<String>,
    /// Ids of deliveries whose state is open.
    pub open: BTreeSet<String>,
    pub counted: HashSet<String>,
    pub wakes: HashMap<String, WakeRec>,
    pub tokens: HashMap<String, String>,
    pub aliases: BTreeMap<String, Vec<String>>,
    pub known: BTreeMap<String, String>,
    pub pol: Policy,
    pub running: BTreeMap<String, TurnRow>,
    pub pairs: HashMap<String, Vec<GoTime>>,
    /// receive.set rows by (boardId, personId).
    pub receive: BTreeMap<(String, String), crate::bridge::Setting<crate::ReceiveSetting>>,
    /// autonomy.set rows by boardId.
    pub autonomy: BTreeMap<String, crate::bridge::Setting<crate::AutonomySetting>>,
    /// Board rows bridged, as `<boardId>/<row>><agent>`.
    pub bridged: HashSet<String>,

    pub reach: BTreeMap<String, ReachRec>,
    pub gone_since: BTreeMap<String, GoTime>,
    pub ended_since: BTreeMap<String, GoTime>,
    pub no_card_since: BTreeMap<String, GoTime>,
    pub passing: BTreeMap<String, PassRec>,
    pub reminders: BTreeMap<String, Vec<String>>,
    pub submit: HashMap<String, bool>,
    pub appendable: HashMap<String, bool>,
    pub visible_at: HashMap<String, GoTime>,
    pub held_first: HashSet<String>,
    pub subs: Vec<Arc<Queue<Change>>>,

    pub replaying: bool,
    pub changes: Vec<Change>,
    pub touched: BTreeSet<String>,
}

/// The messaging core. Safe for concurrent use.
pub struct Inbox {
    pub(crate) inner: Mutex<Inner>,
}

/// A change stream from [`Inbox::subscribe`]. Dropping it (or
/// [`Subscription::cancel`]) unsubscribes; the queue then drains and ends.
pub struct Subscription {
    q: Arc<Queue<Change>>,
}

impl Subscription {
    /// Blocks for the next change; `None` once cancelled and drained.
    pub fn recv(&self) -> Option<Change> {
        self.q.recv()
    }

    /// The next change if one is buffered.
    pub fn try_recv(&self) -> Option<Change> {
        match self.q.try_recv() {
            gocompat::sub::TryRecv::Item(c) => Some(c),
            _ => None,
        }
    }

    /// Every change buffered now.
    pub fn drain(&self) -> Vec<Change> {
        let mut out = vec![];
        while let Some(c) = self.try_recv() {
            out.push(c);
        }
        out
    }

    /// Go `len(ch)`.
    pub fn len(&self) -> usize {
        self.q.len()
    }

    pub fn is_empty(&self) -> bool {
        self.q.is_empty()
    }

    /// The queue, for async readers (`recv_async`).
    pub fn queue(&self) -> &Arc<Queue<Change>> {
        &self.q
    }

    pub fn cancel(&self) {
        self.q.close();
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.q.close();
    }
}

fn lock(m: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Inbox {
    /// Replays the log into memory and returns a ready inbox.
    pub fn new(st: Arc<Store>, roster: Arc<dyn Roster>, opts: Options) -> Result<Inbox> {
        let pick = |v: std::time::Duration, def: std::time::Duration| {
            if v.is_zero() { tm::d(def) } else { tm::d(v) }
        };
        let owner_fn = opts.is_owner.clone().unwrap_or_else(|| {
            let me = format!("user:{}", opts.device_id);
            Arc::new(move |k: &str| k == me)
        });
        let mut b = Inner {
            st,
            roster,
            owners: opts.pr_owners,
            on_gone: opts.on_recipient_gone,
            on_passed: opts.on_passed,
            stale: opts.stale,
            dev: opts.device_id,
            owner_fn,
            ignored: 0,
            ignored_by: BTreeMap::new(),
            last: BTreeMap::new(),
            handled: HashSet::new(),
            requests: vec![],
            now_fn: opts.now.unwrap_or_else(|| Arc::new(tm::system_now)),
            lim: Limits {
                unengaged: pick(opts.unengaged_limit, UNENGAGED_LIMIT),
                gone: pick(opts.gone_grace, GONE_GRACE),
                quiet: pick(opts.quiet_limit, QUIET_LIMIT),
                reattach: pick(opts.reattach_grace, REATTACH_GRACE),
                no_card: pick(opts.no_card_grace, NO_CARD_GRACE),
            },
            ids: IdGen::new(),
            msgs: HashMap::new(),
            dels: HashMap::new(),
            order: vec![],
            open: BTreeSet::new(),
            counted: HashSet::new(),
            wakes: HashMap::new(),
            tokens: HashMap::new(),
            aliases: BTreeMap::new(),
            known: BTreeMap::new(),
            pol: Policy::default(),
            running: BTreeMap::new(),
            pairs: HashMap::new(),
            receive: BTreeMap::new(),
            autonomy: BTreeMap::new(),
            bridged: HashSet::new(),
            reach: BTreeMap::new(),
            gone_since: BTreeMap::new(),
            ended_since: BTreeMap::new(),
            no_card_since: BTreeMap::new(),
            passing: BTreeMap::new(),
            reminders: BTreeMap::new(),
            submit: HashMap::new(),
            appendable: HashMap::new(),
            visible_at: HashMap::new(),
            held_first: HashSet::new(),
            subs: vec![],
            replaying: false,
            changes: vec![],
            touched: BTreeSet::new(),
        };
        b.replay()?;
        Ok(Inbox {
            inner: Mutex::new(b),
        })
    }

    /// Runs a mutation under the lock, then Go's `unlock()`: edges for the
    /// touched agents, then the changes to subscribers.
    pub(crate) fn mutate<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        let mut g = lock(&self.inner);
        let r = f(&mut g);
        g.unlock();
        r
    }

    /// A read under the lock (Go `mu.Lock(); defer mu.Unlock()`).
    pub(crate) fn read<R>(&self, f: impl FnOnce(&Inner) -> R) -> R {
        let g = lock(&self.inner);
        f(&g)
    }

    /// Whether the inbox lock is free now (tests check hooks run without it).
    #[doc(hidden)]
    pub fn lock_is_free(&self) -> bool {
        self.inner.try_lock().is_ok()
    }

    /// Changes until the subscription is dropped (buffered, 64; a full
    /// queue drops the change and the reader should re-read).
    pub fn subscribe(&self) -> Subscription {
        let q = Arc::new(Queue::new(SUB_BUFFER));
        lock(&self.inner).subs.push(q.clone());
        Subscription { q }
    }
}

impl Inner {
    pub(crate) fn now(&self) -> GoTime {
        (self.now_fn)()
    }

    pub(crate) fn is_owner(&self, key: &str) -> bool {
        (self.owner_fn)(key)
    }

    /// Replays every inbox row, oldest first, paging past List's cap.
    pub(crate) fn replay(&mut self) -> Result<()> {
        self.replaying = true;
        let r = self.replay_rows();
        self.replaying = false;
        r?;
        // Baseline for WakeReady edges; replay itself notifies nobody.
        for agent in self.agents_with_open() {
            let s = self.has_submit(&agent);
            let a = self.has_append(&agent);
            self.submit.insert(agent.clone(), s);
            self.appendable.insert(agent, a);
        }
        self.changes.clear();
        self.touched.clear();
        Ok(())
    }

    fn replay_rows(&mut self) -> Result<()> {
        for local in [false, true] {
            let kinds = if local {
                LOCAL_REPLAY_KINDS
            } else {
                REPLAY_KINDS
            };
            let mut after = Cursor::default();
            loop {
                let evs = self
                    .st
                    .list(&Filter {
                        kinds: kinds.iter().map(|k| k.to_string()).collect(),
                        after: after.clone(),
                        limit: MAX_LIST_LIMIT,
                        local,
                        ..Filter::default()
                    })
                    .map_err(|e| Error::other(format!("inbox: replay: {e}")))?;
                for ev in &evs {
                    self.apply(ev).map_err(|e| {
                        Error::other(format!(
                            "inbox: replay {} {}/{}: {e}",
                            ev.kind, ev.device_id, ev.seq
                        ))
                    })?;
                    if !local && ev.seq > self.last.get(&ev.device_id).copied().unwrap_or(0) {
                        self.last.insert(ev.device_id.clone(), ev.seq);
                    }
                    after = ev.cursor();
                }
                if (evs.len() as i64) < MAX_LIST_LIMIT {
                    break;
                }
            }
        }
        Ok(())
    }

    /// Appends one row and folds it.
    pub(crate) fn emit<P: Serialize>(&mut self, by: &str, kind: &str, payload: &P) -> Result<()> {
        let ev = self
            .st
            .append(&self.actor_for(by), kind, payload)
            .map_err(|e| Error::other(format!("inbox: append {kind}: {e}")))?;
        self.apply(&ev).map_err(|e| Error::other(e.to_string()))
    }

    /// Appends one device-local row (policy.set) and folds it.
    pub(crate) fn emit_local<P: Serialize>(
        &mut self,
        by: &str,
        kind: &str,
        payload: &P,
    ) -> Result<()> {
        let ev = self
            .st
            .append_local(&self.actor_for(by), kind, payload)
            .map_err(|e| Error::other(format!("inbox: append {kind}: {e}")))?;
        self.apply(&ev).map_err(|e| Error::other(e.to_string()))
    }

    /// Stamps and appends a delivery snapshot; a held or dismissed one let
    /// through queues its message for its card.
    pub(crate) fn put_delivery(&mut self, mut r: DeliveryRow, by: &str) -> Result<()> {
        let was_held = self
            .dels
            .get(&r.delivery_id)
            .is_some_and(|d| d.d.state == st::HELD || d.d.state == st::DISMISSED);
        r.by = by.to_string();
        r.at = self.now();
        self.emit(by, kinds::DELIVERY_STATE, &r)?;
        if was_held && r.state == st::QUEUED {
            self.passed(&r.delivery_id);
        }
        Ok(())
    }

    /// The store actor for a caller.
    pub(crate) fn actor_for(&self, by: &str) -> String {
        if by.is_empty() || by == "irori" {
            "irori".into()
        } else if self.is_owner(by) {
            "user".into()
        } else if by.starts_with(API_KEY_PREFIX) {
            by.into()
        } else {
            format!("agent:{by}")
        }
    }

    /// Folds one row. Err only for a payload Go would fail to decode.
    pub(crate) fn apply(&mut self, ev: &Event) -> std::result::Result<(), String> {
        if !self.writer_ok(ev) {
            self.ignore(&ev.device_id);
            return Ok(());
        }
        let stamp = |at: GoTime| {
            if at.is_zero() {
                GoTime::unix_milli(ev.hlc.wall_ms())
            } else {
                at
            }
        };
        match ev.kind.as_str() {
            kinds::MESSAGE_SENT => {
                let mut m = decode_message(&ev.payload).map_err(|e| e.to_string())?;
                if m.id.is_empty() {
                    return Err("message without id".into());
                }
                m.writer = ev.device_id.clone();
                self.fold_bridged(&m);
                let from = m.from.clone();
                self.msgs.insert(m.id.clone(), m);
                if !from.key.is_empty() && !self.is_person(&from) && !from.is_system() {
                    self.known.entry(from.key).or_default();
                }
            }
            kinds::DELIVERY_STATE => {
                let r = decode_delivery(&ev.payload).map_err(|e| e.to_string())?;
                if r.delivery_id.is_empty() {
                    return Err("delivery without id".into());
                }
                self.apply_delivery(ev, r, stamp(GoTime::ZERO));
            }
            kinds::WAKE_ISSUED => {
                let r = decode_wake(&ev.payload).map_err(|e| e.to_string())?;
                let md = if r.mode.is_empty() {
                    mode::WAKE.to_string()
                } else {
                    r.mode.clone()
                };
                let w = WakeRec {
                    w: Wake {
                        id: r.wake_id.clone(),
                        to: r.to.clone(),
                        token: r.token.clone(),
                        reminder: r.reminder,
                        mode: md,
                        delivery_ids: r.delivery_ids.clone(),
                        overflow: r.overflow.clone(),
                        prompt: String::new(),
                    },
                    path: r.path.clone(),
                    issued_at: stamp(r.at),
                };
                let gates = w.gates() && w.path != path::CHANNEL;
                self.wakes.insert(r.wake_id.clone(), w);
                self.tokens.insert(r.token.clone(), r.wake_id.clone());
                for id in &r.delivery_ids {
                    let Some(d) = self.dels.get_mut(id) else {
                        continue;
                    };
                    if r.reminder {
                        d.d.reminded = true;
                    } else if d.d.state == st::QUEUED {
                        d.d.wake_id = r.wake_id.clone();
                        if gates {
                            d.wake_tries += 1;
                        }
                    }
                }
                if r.reminder {
                    self.reminders.remove(&r.to);
                }
                self.touch(&[&r.to]);
                self.note(ChangeKind::Updated, &r.to, "");
            }
            kinds::AGENT_ALIAS => {
                let (key, alias, _) =
                    decode_kv(&ev.payload, "key", "alias").map_err(|e| e.to_string())?;
                self.add_alias(&key, &alias);
            }
            kinds::AGENT_NAME => {
                let (key, name, _) =
                    decode_kv(&ev.payload, "key", "name").map_err(|e| e.to_string())?;
                if !key.is_empty() {
                    self.known.insert(key, lower_trim(&name));
                }
            }
            kinds::POLICY_SET => {
                let r = decode_policy(&ev.payload).map_err(|e| e.to_string())?;
                self.pol = norm_policy(Policy {
                    mode: r.mode,
                    block: r.block,
                    allow: r.allow,
                    paused: r.paused,
                });
                for agent in self.agents_with_open() {
                    self.touched.insert(agent);
                }
                self.note(ChangeKind::Updated, "", "");
            }
            kinds::TURN_ENGAGED => {
                let mut r = decode_turn(&ev.payload).map_err(|e| e.to_string())?;
                r.at = stamp(r.at);
                let agent = r.agent.clone();
                self.running.insert(agent.clone(), r);
                self.touch(&[&agent]);
            }
            kinds::TURN_ENDED => {
                let r = decode_turn(&ev.payload).map_err(|e| e.to_string())?;
                if self
                    .running
                    .get(&r.agent)
                    .is_some_and(|cur| cur.wake_id == r.wake_id)
                {
                    self.running.remove(&r.agent);
                }
                self.touch(&[&r.agent]);
            }
            kinds::DELIVERY_DECIDED => {
                let r = decode_decided(&ev.payload).map_err(|e| e.to_string())?;
                // Only the person decides a held delivery.
                if ev.actor != "user" || !self.is_owner(&r.by) {
                    self.ignore(&ev.device_id);
                    return Ok(());
                }
                let id = r.delivery_id.clone();
                if self.replaying {
                    self.requests.push(r);
                }
                self.note(ChangeKind::Updated, "", &id);
            }
            kinds::RECEIVE_SET | kinds::AUTONOMY_SET => self.apply_setting(ev)?,
            kinds::AGENT_SEEN => {
                if let Some((key, name)) = decode_seen(&ev.payload)
                    && !key.is_empty()
                {
                    self.known.insert(key, lower_trim(&name));
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn apply_delivery(&mut self, ev: &Event, r: DeliveryRow, hlc_at: GoTime) {
        let at = if r.at.is_zero() { hlc_at } else { r.at };
        let existed = self.dels.contains_key(&r.delivery_id);
        let (mut prev_to, mut prev_state) = (String::new(), String::new());
        if !existed {
            let d = Drec {
                d: Delivery {
                    id: r.delivery_id.clone(),
                    created_at: at,
                    ..Delivery::default()
                },
                first_writer: ev.device_id.clone(),
                assigned_at: at,
                ..Drec::default()
            };
            self.dels.insert(r.delivery_id.clone(), d);
            self.order.push(r.delivery_id.clone());
            if !self.counted.contains(&r.message_id) {
                self.counted.insert(r.message_id.clone());
                if let Some(m) = self.msgs.get(&r.message_id)
                    && r.moved_from.is_empty()
                    && self.counts_for_rate(&m.from.key, &r.to)
                {
                    let k = pair_key(&crate::bridge::rate_from(m), &r.to);
                    let sent = m.sent_at;
                    self.pairs.entry(k).or_default().push(sent);
                }
            }
        } else {
            let d = &self.dels[&r.delivery_id];
            prev_to = d.d.to.clone();
            prev_state = d.d.state.clone();
            if prev_to != r.to {
                self.dels.get_mut(&r.delivery_id).unwrap().assigned_at = at;
            }
        }
        let is_owner_to = !r.to.is_empty() && self.is_owner(&r.to);
        let d = self.dels.get_mut(&r.delivery_id).expect("delivery");
        if r.state != st::QUEUED || r.to != prev_to {
            d.wake_tries = 0;
        }
        d.d.message_id = r.message_id.clone();
        d.d.to = r.to.clone();
        d.d.repo_key = r.repo_key.clone();
        d.d.mode = if r.mode.is_empty() {
            mode::WAKE.to_string() // rows from before next-turn delivery
        } else {
            r.mode.clone()
        };
        if existed
            && (prev_state == st::HELD || prev_state == st::DISMISSED)
            && r.state != prev_state
        {
            d.d.decided_by = r.by.clone();
            d.d.decided_at = at;
        }
        if r.state == st::HELD && !r.hold.is_empty() {
            d.d.held_for = r.hold.clone();
        }
        d.d.state = r.state.clone();
        d.d.hold = r.hold.clone();
        d.d.via = r.via.clone();
        d.d.wake_id = r.wake_id.clone();
        d.d.note = r.note.clone();
        d.d.for_ = r.for_.clone();
        d.d.until = r.until;
        d.d.moved_to = r.moved_to.clone();
        d.d.reminded = r.reminded;
        d.moved_from = r.moved_from.clone();
        d.engaged_once = r.engaged_once;
        d.appended = r.appended && r.state == st::DELIVERED;
        d.d.updated_at = at;
        d.last_writer = ev.device_id.clone();
        let (id, msg_id, to, state) = (
            d.d.id.clone(),
            d.d.message_id.clone(),
            d.d.to.clone(),
            d.d.state.clone(),
        );
        if st::open(&state) {
            self.open.insert(id.clone());
        } else {
            self.open.remove(&id);
        }
        if let std::collections::hash_map::Entry::Vacant(e) = self.visible_at.entry(msg_id) {
            if state == st::HELD || state == st::DISMISSED {
                self.held_first.insert(e.key().clone());
            } else {
                e.insert(at);
            }
        }
        if !to.is_empty() && !is_owner_to {
            self.known.entry(to.clone()).or_default();
        }
        self.touch(&[&prev_to, &to]);
        if state == st::HELD && (!existed || prev_state != st::HELD) {
            self.note(ChangeKind::HeldNew, &to, &id);
        } else if state == st::MOVED {
            self.note(ChangeKind::MovedAway, &prev_to, &id);
        } else {
            self.note(ChangeKind::Updated, &to, &id);
        }
    }

    /// Records alias for key: newest last, at most MaxAliases.
    pub(crate) fn add_alias(&mut self, key: &str, alias: &str) {
        let alias = norm_alias(alias);
        if key.is_empty() || alias.is_empty() {
            return;
        }
        let list = self.aliases.entry(key.to_string()).or_default();
        list.retain(|a| *a != alias);
        list.push(alias);
        if list.len() > MAX_ALIASES {
            let cut = list.len() - MAX_ALIASES;
            list.drain(..cut);
        }
    }

    pub(crate) fn touch(&mut self, agents: &[&str]) {
        for a in agents {
            if !a.is_empty() {
                self.touched.insert(a.to_string());
            }
        }
    }

    pub(crate) fn note(&mut self, kind: ChangeKind, to: &str, did: &str) {
        if !self.replaying {
            self.changes.push(Change {
                kind,
                to: to.into(),
                delivery_id: did.into(),
            });
        }
    }

    /// Go `unlock()`'s work: WakeReady/AppendReady edges for the touched
    /// agents, then every change to every subscriber (dropped when full).
    pub(crate) fn unlock(&mut self) {
        let touched = std::mem::take(&mut self.touched);
        for agent in touched {
            let now = self.has_submit(&agent);
            if now && !self.submit.get(&agent).copied().unwrap_or(false) {
                self.changes.push(Change {
                    kind: ChangeKind::WakeReady,
                    to: agent.clone(),
                    delivery_id: String::new(),
                });
            }
            self.submit.insert(agent.clone(), now);
            let app = self.has_append(&agent);
            if app && !self.appendable.get(&agent).copied().unwrap_or(false) {
                self.changes.push(Change {
                    kind: ChangeKind::AppendReady,
                    to: agent.clone(),
                    delivery_id: String::new(),
                });
            }
            self.appendable.insert(agent, app);
        }
        self.subs.retain(|q| !q.is_closed());
        for c in self.changes.drain(..) {
            for q in &self.subs {
                let _ = q.push(c.clone()); // slow reader; it re-reads
            }
        }
    }

    /// Every recipient of an open delivery, plus agents with a running turn
    /// or a due reminder.
    pub(crate) fn agents_with_open(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for id in &self.open {
            let to = &self.dels[id].d.to;
            if !to.is_empty() {
                out.insert(to.clone());
            }
        }
        out.extend(self.running.keys().cloned());
        out.extend(self.reminders.keys().cloned());
        out
    }

    /// The open deliveries, in id order.
    pub(crate) fn open_recs(&self) -> impl Iterator<Item = &Drec> {
        self.open.iter().map(|id| &self.dels[id])
    }

    /// NextWake on the mod path would return a wake for agent now.
    pub(crate) fn has_submit(&self, agent: &str) -> bool {
        if agent.is_empty() || self.pol.paused || !self.local(agent) {
            return false;
        }
        if self.outstanding(agent).is_some() {
            return false;
        }
        if self.reminders.get(agent).is_some_and(|r| !r.is_empty()) {
            return true;
        }
        self.open_recs().any(|d| {
            d.d.to == agent && d.d.state == st::QUEUED && d.d.wake_id.is_empty() && !d.for_append()
        })
    }

    /// NextAppend would return an append for agent now.
    pub(crate) fn has_append(&self, agent: &str) -> bool {
        if agent.is_empty()
            || self.pol.paused
            || !self.local(agent)
            || self.pending_append(agent).is_some()
        {
            return false;
        }
        let behind = self.queued_behind(agent);
        self.open_recs().any(|d| {
            d.d.to == agent
                && d.d.state == st::QUEUED
                && d.d.wake_id.is_empty()
                && (behind || d.for_append())
        })
    }

    /// The append issued to agent and not yet confirmed delivered.
    pub(crate) fn pending_append(&self, agent: &str) -> Option<&WakeRec> {
        let mut best: Option<&WakeRec> = None;
        for d in self.open_recs() {
            if d.d.to != agent || d.d.state != st::QUEUED || d.d.wake_id.is_empty() {
                continue;
            }
            if let Some(w) = self.wakes.get(&d.d.wake_id)
                && w.is_append()
                && best.is_none_or(|b| tm::before(&w.issued_at, &b.issued_at))
            {
                best = Some(w);
            }
        }
        best
    }

    /// The wake that gates agent: issued and not yet delivered (Queued),
    /// else delivered and not yet engaged (Delivered) within the unengaged
    /// limit.
    pub(crate) fn outstanding(&self, agent: &str) -> Option<(&WakeRec, &'static str)> {
        let mut delivered: Option<&WakeRec> = None;
        let now = self.now();
        for d in self.open_recs() {
            if d.d.to != agent || d.d.wake_id.is_empty() {
                continue;
            }
            let Some(w) = self.wakes.get(&d.d.wake_id) else {
                continue;
            };
            if !w.gates() {
                continue;
            }
            match d.d.state.as_str() {
                st::QUEUED => return Some((w, st::QUEUED)),
                st::DELIVERED
                    if !d.appended
                        && tm::sub(&now, &d.d.updated_at) < self.lim.unengaged
                        && delivered.is_none_or(|b| tm::before(&w.issued_at, &b.issued_at)) =>
                {
                    delivered = Some(w);
                }
                _ => {}
            }
        }
        delivered.map(|w| (w, st::DELIVERED))
    }

    /// The host already holds a wake prompt it has not started (card #28).
    pub(crate) fn queued_behind(&self, agent: &str) -> bool {
        matches!(self.outstanding(agent), Some((_, st::DELIVERED)))
    }

    /// d with its message filled in.
    pub(crate) fn copy_out(&self, d: &Drec) -> Delivery {
        let mut out = d.d.clone();
        if let Some(m) = self.msgs.get(&d.d.message_id) {
            out.message = m.clone();
        }
        out
    }

    pub(crate) fn copy_id(&self, id: &str) -> Delivery {
        self.copy_out(&self.dels[id])
    }

    /// Both ends are agents.
    pub(crate) fn counts_for_rate(&self, from: &str, to: &str) -> bool {
        !from.is_empty()
            && !to.is_empty()
            && !self.is_owner(from)
            && !self.is_owner(to)
            && from != SYSTEM_KEY
            && to != SYSTEM_KEY
    }

    /// The roster minus gone cards, by key, recording nothing.
    pub(crate) fn roster_cards(&self) -> BTreeMap<String, AgentCard> {
        let mut out = BTreeMap::new();
        for c in self.roster.agents() {
            if c.key.is_empty() || c.status == "gone" {
                continue;
            }
            out.insert(c.key.clone(), c);
        }
        out
    }

    /// The owner table (v0.4 §6.0) for one row before it folds.
    fn writer_ok(&self, ev: &Event) -> bool {
        use irori_identity::owner::may_write;
        match ev.kind.as_str() {
            kinds::DELIVERY_STATE => {
                let Ok(r) = decode_delivery(&ev.payload) else {
                    return true; // apply reports it
                };
                match self.dels.get(&r.delivery_id) {
                    None if !r.to.is_empty() && !may_write(&r.to, &ev.device_id) => {
                        r.state == st::QUEUED && r.hold.is_empty() && r.wake_id.is_empty()
                    }
                    None => true,
                    Some(d) if !d.d.to.is_empty() => may_write(&d.d.to, &ev.device_id),
                    Some(d) => d.first_writer.is_empty() || d.first_writer == ev.device_id,
                }
            }
            kinds::WAKE_ISSUED => match decode_target(&ev.payload, "to") {
                Some(to) => may_write(&to, &ev.device_id),
                None => true,
            },
            kinds::TURN_ENGAGED | kinds::TURN_ENDED => match decode_target(&ev.payload, "agent") {
                Some(a) => may_write(&a, &ev.device_id),
                None => true,
            },
            _ => true,
        }
    }

    /// The person at a GUI: kind user and an owner key.
    pub(crate) fn is_person(&self, s: &crate::Sender) -> bool {
        s.is_user() && self.is_owner(&s.key)
    }

    pub(crate) fn ignore(&mut self, dev: &str) {
        self.ignored += 1;
        *self.ignored_by.entry(dev.to_string()).or_default() += 1;
    }
}

/// Go `strings.TrimSpace`.
pub(crate) fn trim(s: &str) -> &str {
    s.trim_matches(gocompat::text::is_space)
}

/// Go `strings.ToLower(strings.TrimSpace(s))`.
pub(crate) fn lower_trim(s: &str) -> String {
    gocompat::text::to_lower_str(trim(s))
}

/// Lowercases and trims a name; `""` when it can't be an alias.
pub(crate) fn norm_alias(s: &str) -> String {
    let s = lower_trim(s);
    if s.is_empty() || s.chars().count() > MAX_ALIAS_LEN {
        return String::new();
    }
    s
}

pub(crate) fn norm_policy(mut p: Policy) -> Policy {
    if p.mode != policy_mode::ALLOWLIST && p.mode != policy_mode::OPEN {
        p.mode = policy_mode::EVERYONE.into();
    }
    p
}

pub(crate) fn pair_key(a: &str, c: &str) -> String {
    if a > c {
        format!("{c}|{a}")
    } else {
        format!("{a}|{c}")
    }
}
