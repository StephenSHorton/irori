//! The person's approvals as rows (I:internal/ask, coord.md §2.4, §3.11):
//! an agent's question with buttons (the notify tool), and the host's
//! question before it revives a session for another device. Every desktop
//! device shows the card; any of them may answer; only the asking device
//! decides.
//!
//! ```text
//! ask.posted       {askId, by, name, title, text, level, actions, until, target?, deliveryId?}
//! ask.answered     {askId, choice, target?, deliveryId?, by, atDevice}   actor "user"
//! ask.resolved     {askId, choice, answered, answerRef}                  the asking device
//! revive.requested {target, deliveryId, by, atDevice}                    actor "user"
//! ```
//!
//! Go's `Book.run` loop is [`Book::follow`] (a thread on the store
//! subscription that also runs the duties every second); every duty is a
//! plain call that reads the clock from `Options::now`.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use gocompat::sub::Queue;
use gocompat::time::GoTime;
use irori_identity::owner::device_of;
use store::{Event, Filter, Store, kinds};

use crate::gojson::{decode, nil_vec, omit, omit_zero_time};
use crate::gotime;

pub const KIND_POSTED: &str = kinds::ASK_POSTED;
pub const KIND_ANSWERED: &str = kinds::ASK_ANSWERED;
pub const KIND_RESOLVED: &str = kinds::ASK_RESOLVED;
pub const KIND_REVIVE_REQUESTED: &str = kinds::REVIVE_REQUESTED;

const KINDS: [&str; 4] = [
    KIND_POSTED,
    KIND_ANSWERED,
    KIND_RESOLVED,
    KIND_REVIVE_REQUESTED,
];

/// The revive ask's buttons; the first one revives.
pub const CHOICE_REVIVE: &str = "Revive";
pub const CHOICE_NOT_NOW: &str = "Not now";

/// How long a revive ask waits for an answer.
pub const REVIVE_WAIT: Duration = Duration::from_secs(30 * 60);

/// Subscriber buffer (Go `make(chan string, 64)`).
pub const SUB_BUFFER: usize = 64;

crate::go_struct! {
    /// One question to the person. The fold fields (`device` on) are on
    /// the wire too: Go marshals the whole struct into `ask.posted`.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct Ask {
        pub id: String = "askId",
        /// The asking agent's key, or `irori` for a revive.
        pub by: String = "by",
        /// Who asks, for the card.
        pub name: String = "name",
        pub title: String = "title" "omit",
        pub text: String = "text",
        pub level: String = "level" "omit",
        #[serde(serialize_with = "nil_vec")]
        pub actions: Vec<String> = "actions",
        pub until: GoTime = "until",
        /// A revive ask's session to revive and the message it is for.
        pub target: String = "target" "omit",
        pub delivery_id: String = "deliveryId" "omit",
        /// The asking device: it decides.
        pub device: String = "device",
        pub resolved: bool = "resolved",
        /// The answer; `""` when dismissed or unanswered.
        pub choice: String = "choice" "omit",
        /// Someone answered (else it ran out).
        pub answered: bool = "answered" "omit",
        pub answer_ref: String = "answerRef" "omit",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct AnsweredRow {
        pub ask_id: String = "askId",
        pub choice: String = "choice",
        pub target: String = "target" "omit",
        pub delivery_id: String = "deliveryId" "omit",
        pub by: String = "by",
        pub at_device: String = "atDevice" "omit",
        pub at: GoTime = "at" "omit_zero_time",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct ResolvedRow {
        pub ask_id: String = "askId",
        pub choice: String = "choice",
        pub answered: bool = "answered",
        pub answer_ref: String = "answerRef" "omit",
        pub at: GoTime = "at" "omit_zero_time",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct ReviveRow {
        pub target: String = "target",
        pub delivery_id: String = "deliveryId" "omit",
        pub by: String = "by",
        pub at_device: String = "atDevice" "omit",
        pub at: GoTime = "at" "omit_zero_time",
    }
}

/// Why a Book call failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Only the person answers an ask or asks for a revive.
    #[error("ask: only the person answers")]
    NotPerson,
    #[error("ask: an ask needs who asks, text, actions and an end")]
    Incomplete,
    #[error("ask: no ask {0}")]
    NoAsk(String),
    #[error("ask: that session is on this device; revive it here")]
    ReviveHere,
    #[error(transparent)]
    Store(#[from] store::Error),
}

type KeyFn = Arc<dyn Fn(&str) -> bool + Send + Sync>;
type NowFn = Arc<dyn Fn() -> GoTime + Send + Sync>;
type ReviveFn = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;
type NameFn = Arc<dyn Fn(&str) -> String + Send + Sync>;
type ResolvedFn = Arc<dyn Fn(&Ask) + Send + Sync>;

/// Configures a [`Book`].
#[derive(Clone)]
pub struct Options {
    pub store: Arc<Store>,
    pub self_id: String,
    /// The person's keys; only their answers and requests count. `None`
    /// accepts exactly `user:<self>`.
    pub is_owner: Option<KeyFn>,
    /// `None` is the system clock.
    pub now: Option<NowFn>,
    /// Wakes a session this device hosts once a revive ask was answered
    /// Revive for it. `None`: a revive ask resolves without waking.
    pub revive: Option<ReviveFn>,
    /// Shows an agent key as its display name; `None` shows the key.
    pub name: Option<NameFn>,
}

impl Options {
    pub fn new(store: Arc<Store>, self_id: &str) -> Options {
        Options {
            store,
            self_id: self_id.into(),
            is_owner: None,
            now: None,
            revive: None,
            name: None,
        }
    }
}

#[derive(Debug, Clone)]
struct Answer {
    row: AnsweredRow,
    /// `<device>/<seq>`.
    reference: String,
}

#[derive(Default)]
struct State {
    asks: HashMap<String, Ask>,
    answers: HashMap<String, Vec<Answer>>,
    last: HashMap<String, i64>,
    ignored: usize,
    ignored_by: BTreeMap<String, usize>,
    seq: u64,
    replaying: bool,
    pending: Vec<String>,
    revives: bool,
}

/// Every ask the log holds. Safe for concurrent use.
pub struct Book {
    o: Options,
    is_owner: KeyFn,
    now: NowFn,
    st: Mutex<State>,
    on_resolved: Mutex<Vec<ResolvedFn>>,
    subs: Mutex<Vec<Weak<Queue<String>>>>,
}

impl Book {
    /// Replays the asks. [`Book::follow`] keeps folding and runs the duties.
    pub fn new(o: Options) -> store::Result<Arc<Book>> {
        let self_key = format!("user:{}", o.self_id);
        let is_owner = o
            .is_owner
            .clone()
            .unwrap_or_else(|| Arc::new(move |k: &str| k == self_key));
        let now = o
            .now
            .clone()
            .unwrap_or_else(|| Arc::new(|| GoTime::from_system(std::time::SystemTime::now())));
        let b = Arc::new(Book {
            o,
            is_owner,
            now,
            st: Mutex::new(State {
                revives: true,
                ..State::default()
            }),
            on_resolved: Mutex::new(Vec::new()),
            subs: Mutex::new(Vec::new()),
        });
        b.replay()?;
        Ok(b)
    }

    /// [`Book::new`] then [`Book::follow`] (Go `Start`).
    pub fn start(o: Options) -> store::Result<Arc<Book>> {
        let sub = o.store.subscribe();
        let b = Book::new(o)?;
        b.follow_with(sub);
        Ok(b)
    }

    /// Folds other devices' rows as they land and runs the duties after
    /// each and every second, on a thread, until the store closes or the
    /// Book is dropped.
    pub fn follow(self: &Arc<Self>) {
        let sub = self.o.store.subscribe();
        self.follow_with(sub);
    }

    fn follow_with(self: &Arc<Self>, sub: store::Subscription) {
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || {
            if let Some(b) = weak.upgrade() {
                b.duties();
            }
            loop {
                let got = sub.recv_timeout(Duration::from_secs(1));
                let Some(b) = weak.upgrade() else { return };
                match got {
                    Ok(None) => return,
                    Ok(Some(ev)) => b.handle(&ev),
                    Err(()) => {
                        b.duties();
                        b.flush();
                    }
                }
            }
        });
    }

    /// One row from a store subscription (Go's run loop body).
    pub fn handle(&self, ev: &Event) {
        if ev.is_resync() {
            self.catch_up();
            return;
        }
        if ev.device_id == self.o.self_id || !KINDS.contains(&ev.kind.as_str()) {
            return;
        }
        self.fold(&mut self.lock(), ev);
        self.duties();
        self.flush();
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn replay(&self) -> store::Result<()> {
        let mut st = self.lock();
        st.replaying = true;
        let mut f = Filter {
            kinds: KINDS.iter().map(|k| k.to_string()).collect(),
            limit: store::MAX_LIST_LIMIT,
            ..Filter::default()
        };
        loop {
            let evs = match self.o.store.list(&f) {
                Ok(e) => e,
                Err(e) => {
                    st.replaying = false;
                    return Err(e);
                }
            };
            for ev in &evs {
                self.fold(&mut st, ev);
            }
            if (evs.len() as i64) < f.limit {
                break;
            }
            f.after = evs[evs.len() - 1].cursor();
        }
        st.replaying = false;
        st.pending.clear();
        Ok(())
    }

    /// After a resync signal: re-fold if a revoke's cut moved rows away,
    /// then page every other device's rows past what was folded.
    pub fn catch_up(&self) {
        let Ok(vec) = self.o.store.vector() else {
            return;
        };
        self.refold_if_cut(&vec);
        for (dev, top) in &vec {
            if *dev == self.o.self_id {
                continue;
            }
            loop {
                let after = self.lock().last.get(dev).copied().unwrap_or(0);
                if after >= *top {
                    break;
                }
                let f = Filter {
                    device_id: dev.clone(),
                    after_seq: after,
                    kinds: KINDS.iter().map(|k| k.to_string()).collect(),
                    limit: store::MAX_LIST_LIMIT,
                    ..Filter::default()
                };
                let evs = match self.o.store.list(&f) {
                    Ok(e) if !e.is_empty() => e,
                    _ => break,
                };
                let mut st = self.lock();
                for ev in &evs {
                    self.fold(&mut st, ev);
                }
                drop(st);
                if (evs.len() as i64) < store::MAX_LIST_LIMIT {
                    break;
                }
            }
        }
        self.duties();
        self.flush();
    }

    fn refold_if_cut(&self, vec: &store::Vector) {
        let was: Vec<String> = {
            let mut st = self.lock();
            let cut = st
                .last
                .iter()
                .any(|(dev, seq)| vec.get(dev).copied().unwrap_or(0) < *seq);
            if !cut {
                return;
            }
            let was = st.asks.keys().cloned().collect();
            let revives = st.revives;
            *st = State {
                revives,
                ..State::default()
            };
            was
        };
        let _ = self.replay();
        self.lock().pending.extend(was);
    }

    /// Folds one row.
    fn fold(&self, st: &mut State, ev: &Event) {
        let last = st.last.get(&ev.device_id).copied().unwrap_or(0);
        if ev.seq <= last {
            return;
        }
        st.last.insert(ev.device_id.clone(), ev.seq);
        match ev.kind.as_str() {
            KIND_POSTED => {
                let Some(mut a) = decode::<Ask>(&ev.payload) else {
                    return;
                };
                if a.id.is_empty() {
                    return;
                }
                if a.by != "irori" && device_of(&a.by) != ev.device_id {
                    ignore(st, &ev.device_id);
                    return;
                }
                if st.asks.contains_key(&a.id) {
                    return;
                }
                a.device = ev.device_id.clone();
                a.resolved = false;
                a.choice.clear();
                a.answered = false;
                a.answer_ref.clear();
                let id = a.id.clone();
                st.asks.insert(id.clone(), a);
                changed(st, &id);
            }
            KIND_ANSWERED => {
                let Some(r) = decode::<AnsweredRow>(&ev.payload) else {
                    return;
                };
                if r.ask_id.is_empty() {
                    return;
                }
                if ev.actor != "user" || !(self.is_owner)(&r.by) {
                    ignore(st, &ev.device_id);
                    return;
                }
                let id = r.ask_id.clone();
                st.answers.entry(id.clone()).or_default().push(Answer {
                    row: r,
                    reference: format!("{}/{}", ev.device_id, ev.seq),
                });
                changed(st, &id);
            }
            KIND_RESOLVED => {
                let Some(r) = decode::<ResolvedRow>(&ev.payload) else {
                    return;
                };
                let Some(a) = st.asks.get_mut(&r.ask_id) else {
                    ignore(st, &ev.device_id);
                    return;
                };
                if a.device != ev.device_id {
                    ignore(st, &ev.device_id);
                    return;
                }
                if a.resolved {
                    return;
                }
                a.resolved = true;
                a.choice = r.choice;
                a.answered = r.answered;
                a.answer_ref = r.answer_ref;
                let id = a.id.clone();
                changed(st, &id);
            }
            KIND_REVIVE_REQUESTED => st.revives = true,
            _ => {}
        }
    }

    fn flush(&self) {
        let ids = std::mem::take(&mut self.lock().pending);
        if ids.is_empty() {
            return;
        }
        let mut subs = self.subs.lock().unwrap_or_else(|e| e.into_inner());
        subs.retain(|w| w.strong_count() > 0);
        for id in ids {
            for q in subs.iter().filter_map(Weak::upgrade) {
                q.push(id.clone());
            }
        }
    }

    /// The run loop's 1 s tick (Go `run`'s ticker case): the duties, then
    /// the changes they made go out. For a core that drives the store
    /// subscription itself instead of [`Book::follow`].
    pub fn tick(&self) {
        self.duties();
        self.flush();
    }

    /// The asking device's part: resolve each own ask on the first answer
    /// folded or at until, post the ask for each revive requested of a
    /// session this device hosts, and run what resolved.
    pub fn duties(&self) {
        let revives = std::mem::replace(&mut self.lock().revives, false);
        if revives && self.post_revives().is_err() {
            self.lock().revives = true;
        }
        let now = (self.now)();
        let mut done = Vec::new();
        {
            let mut st = self.lock();
            let mut todo: Vec<String> = st
                .asks
                .values()
                .filter(|a| a.device == self.o.self_id && !a.resolved)
                .map(|a| a.id.clone())
                .collect();
            todo.sort();
            for id in todo {
                let mut r = ResolvedRow {
                    ask_id: id.clone(),
                    at: now,
                    ..ResolvedRow::default()
                };
                if let Some(first) = st.answers.get(&id).and_then(|a| a.first()) {
                    r.choice = first.row.choice.clone();
                    r.answered = true;
                    r.answer_ref = first.reference.clone();
                } else if gotime::before(&now, &st.asks[&id].until) {
                    continue;
                }
                if self.o.store.append("irori", KIND_RESOLVED, &r).is_err() {
                    continue;
                }
                let a = st.asks.get_mut(&id).expect("listed above");
                a.resolved = true;
                a.choice = r.choice;
                a.answered = r.answered;
                a.answer_ref = r.answer_ref;
                done.push(a.clone());
                st.pending.push(id);
            }
        }
        let fns = self
            .on_resolved
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for a in done {
            if !a.target.is_empty() {
                self.revive_if_answered(&a);
                continue;
            }
            for f in &fns {
                f(&a);
            }
        }
    }

    /// Wakes a revive ask's target only on an answer naming exactly its
    /// target and delivery.
    fn revive_if_answered(&self, a: &Ask) {
        let Some(revive) = &self.o.revive else {
            return;
        };
        if !a.answered || a.choice != CHOICE_REVIVE {
            return;
        }
        let first = self
            .lock()
            .answers
            .get(&a.id)
            .and_then(|v| v.first().cloned());
        let Some(first) = first else { return };
        if first.row.target != a.target || first.row.delivery_id != a.delivery_id {
            return;
        }
        let _ = revive(&a.target);
    }

    /// Turns each `revive.requested` for a session this device hosts into
    /// an ask, once.
    fn post_revives(&self) -> Result<(), Error> {
        let evs = self.o.store.list(&Filter {
            kinds: vec![KIND_REVIVE_REQUESTED.into()],
            limit: store::MAX_LIST_LIMIT,
            desc: true,
            ..Filter::default()
        })?;
        for ev in &evs {
            let Some(r) = decode::<ReviveRow>(&ev.payload) else {
                continue;
            };
            if r.target.is_empty() || device_of(&r.target) != self.o.self_id {
                continue;
            }
            if ev.actor != "user" || !(self.is_owner)(&r.by) {
                continue;
            }
            let id = format!("revive-{}-{}", ev.device_id, ev.seq);
            if self.lock().asks.contains_key(&id) {
                continue;
            }
            let mut name = r.target.clone();
            if let Some(nf) = &self.o.name {
                let n = nf(&r.target);
                let n = crate::names::trim_space(&n);
                if !n.is_empty() {
                    name = n.to_string();
                }
            }
            let a = Ask {
                id,
                by: "irori".into(),
                name: "Irori".into(),
                title: format!("Revive {name}?"),
                text: format!(
                    "Your other device asked to revive {name} here, to deliver a message to it."
                ),
                actions: vec![CHOICE_REVIVE.into(), CHOICE_NOT_NOW.into()],
                until: gotime::add(&(self.now)(), REVIVE_WAIT),
                target: r.target.clone(),
                delivery_id: r.delivery_id.clone(),
                ..Ask::default()
            };
            self.post_ask(a)?;
        }
        Ok(())
    }

    /// Records an agent's question. The first answer folded here resolves
    /// it, and the `on_resolved` callbacks run then (or at until).
    pub fn post(&self, a: Ask) -> Result<Ask, Error> {
        if a.by.is_empty() || a.text.is_empty() || a.actions.is_empty() || a.until.is_zero() {
            return Err(Error::Incomplete);
        }
        self.post_ask(a)
    }

    fn post_ask(&self, mut a: Ask) -> Result<Ask, Error> {
        if a.id.is_empty() {
            let mut st = self.lock();
            st.seq += 1;
            a.id = format!(
                "{}-{}",
                base36((self.now)().unix_nano() as i128),
                base36(i128::from(st.seq))
            );
        }
        a.device.clear();
        let ev = self.o.store.append("irori", KIND_POSTED, &a)?;
        let out = {
            let mut st = self.lock();
            self.fold(&mut st, &ev);
            st.asks.get(&a.id).cloned().unwrap_or_default()
        };
        self.flush();
        Ok(out)
    }

    /// The person's choice on any device (`""` = dismissed). For a revive
    /// ask it names the ask's target and delivery.
    pub fn answer(&self, id: &str, choice: &str, by: &str) -> Result<(), Error> {
        if !(self.is_owner)(by) {
            return Err(Error::NotPerson);
        }
        let (target, delivery) = match self.lock().asks.get(id) {
            Some(a) => (a.target.clone(), a.delivery_id.clone()),
            None => return Err(Error::NoAsk(gocompat::quote::quote(id))),
        };
        let ev = self.o.store.append(
            "user",
            KIND_ANSWERED,
            &AnsweredRow {
                ask_id: id.into(),
                choice: choice.into(),
                target,
                delivery_id: delivery,
                by: by.into(),
                at_device: self.o.self_id.clone(),
                at: (self.now)(),
            },
        )?;
        self.fold(&mut self.lock(), &ev);
        self.duties();
        self.flush();
        Ok(())
    }

    /// Asks target's host to revive it for `delivery_id`.
    pub fn request_revive(&self, target: &str, delivery_id: &str, by: &str) -> Result<(), Error> {
        if !(self.is_owner)(by) {
            return Err(Error::NotPerson);
        }
        let dev = device_of(target);
        if dev.is_empty() || dev == self.o.self_id {
            return Err(Error::ReviveHere);
        }
        self.o.store.append(
            "user",
            KIND_REVIVE_REQUESTED,
            &ReviveRow {
                target: target.into(),
                delivery_id: delivery_id.into(),
                by: by.into(),
                at_device: self.o.self_id.clone(),
                at: (self.now)(),
            },
        )?;
        Ok(())
    }

    /// Runs `f` for each of this device's agent asks once it resolves.
    pub fn on_resolved(&self, f: impl Fn(&Ask) + Send + Sync + 'static) {
        self.on_resolved
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Arc::new(f));
    }

    pub fn get(&self, id: &str) -> Option<Ask> {
        self.lock().asks.get(id).cloned()
    }

    /// The asks not resolved yet, by id.
    pub fn open(&self) -> Vec<Ask> {
        let mut out: Vec<Ask> = self
            .lock()
            .asks
            .values()
            .filter(|a| !a.resolved)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Rows refused by the writer rules.
    pub fn ignored(&self) -> usize {
        self.lock().ignored
    }

    pub fn ignored_by(&self) -> BTreeMap<String, usize> {
        self.lock().ignored_by.clone()
    }

    /// The id of each ask that changed (posted, answered, resolved); a slow
    /// reader drops ids. Dropping the queue unsubscribes.
    pub fn subscribe(&self) -> Arc<Queue<String>> {
        let q = Arc::new(Queue::new(SUB_BUFFER));
        self.subs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Arc::downgrade(&q));
        q
    }
}

fn changed(st: &mut State, id: &str) {
    if !st.replaying {
        st.pending.push(id.to_string());
    }
}

fn ignore(st: &mut State, dev: &str) {
    st.ignored += 1;
    *st.ignored_by.entry(dev.to_string()).or_default() += 1;
}

/// `strconv.FormatInt(n, 36)`.
pub(crate) fn base36(n: i128) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let neg = n < 0;
    let mut u = n.unsigned_abs();
    let mut out = Vec::new();
    while u > 0 {
        out.push(DIGITS[(u % 36) as usize]);
        u /= 36;
    }
    if neg {
        out.push(b'-');
    }
    out.reverse();
    String::from_utf8(out).expect("ascii")
}
