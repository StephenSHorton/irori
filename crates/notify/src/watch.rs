//! Watch (I:internal/notify/watch.go): posts and clears cards for what
//! needs the person, driven only by the inbox, coord and ask change
//! streams (no polling), until stopped:
//!
//! - held deliveries, one card per sender, and messages to the person:
//!   `inbox::watch::Watcher` (the inbox half, crates/inbox), run here
//!   over a [`Notifier`] sink;
//! - a Board card an agent (not the person) moved into the person's Needs
//!   you lane, with Answer and Later; cleared when it leaves the lane;
//! - an ask nobody took (`unclaimed:<id>`, Wake them and Close) and a card
//!   that came back because its agent went away (`returned:<id>`);
//! - the person's approvals as rows (`ask:<id>`), answered as the person.
//!
//! Held messages, messages to the person and waiting asks already there at
//! start get their cards then; the Board seeds quietly. Both change streams
//! drop changes for a slow reader, so neither is taken as complete: the
//! Board is reconciled on every coord change and the inbox against a fresh
//! snapshot of its open deliveries whenever its queue has drained.
//!
//! The Go select loop runs on its own thread over a current-thread tokio
//! runtime that awaits the three subscriber queues and the cards' presses.
//! One difference from Go: a held card's Release or Dismiss runs on the
//! loop (the inbox `Watcher` owns the card state), not on the card's own
//! goroutine; presses queue in a 16-slot channel and a full one drops the
//! press, as Go drops its `failed` report.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::thread::JoinHandle;

use coord::approvals::{Ask as Approval, Book};
use coord::{BoardFilter, BoardView, Card, Coord, Task};
use gocompat::sub::Queue;
use gocompat::time::GoTime;
use inbox::watch::{CardAct, InboxSource, NameFn, OwnerFn, Sink, Watcher};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub use inbox::watch::{hold_reason, preview, upper};

use crate::{
    Action, Level, Note, Notifier, TAB_BOARD, Target, ask_id, needs_id, parse_level, returned_id,
    unclaimed_id,
};

/// The inbox Watch follows and acts through.
pub trait InboxFeed: InboxSource + Send + Sync {
    /// The change queue (Go `Subscribe`); dropping or closing it ends it.
    fn subscribe(&self) -> inbox::Subscription;
}

impl InboxFeed for inbox::Inbox {
    fn subscribe(&self) -> inbox::Subscription {
        inbox::Inbox::subscribe(self)
    }
}

/// The part of the Board Watch reads (Go `BoardSource`).
pub trait BoardSource: Send + Sync {
    fn subscribe(&self) -> Arc<Queue<coord::Change>>;
    fn board(&self, f: &BoardFilter) -> BoardView;
}

impl BoardSource for Coord {
    fn subscribe(&self) -> Arc<Queue<coord::Change>> {
        Coord::subscribe(self)
    }
    fn board(&self, f: &BoardFilter) -> BoardView {
        Coord::board(self, f)
    }
}

/// The person's approvals as rows (Go `AskSource`, *ask.Book).
pub trait AskSource: Send + Sync {
    fn subscribe(&self) -> Arc<Queue<String>>;
    fn get(&self, id: &str) -> Option<Approval>;
    fn open(&self) -> Vec<Approval>;
    fn answer(&self, id: &str, choice: &str, by: &str) -> Result<(), String>;
}

impl AskSource for Book {
    fn subscribe(&self) -> Arc<Queue<String>> {
        Book::subscribe(self)
    }
    fn get(&self, id: &str) -> Option<Approval> {
        Book::get(self, id)
    }
    fn open(&self) -> Vec<Approval> {
        Book::open(self)
    }
    fn answer(&self, id: &str, choice: &str, by: &str) -> Result<(), String> {
        Book::answer(self, id, choice, by).map_err(|e| e.to_string())
    }
}

/// What the unclaimed-ask card acts through: asking the room again, and
/// closing it (board.md "Asks").
pub trait RoomSource: Send + Sync {
    fn ask_again(&self, by: &str, id: &str) -> Result<(), String>;
    fn set_task_state(&self, by: &str, id: &str, state: &str, note: &str) -> Result<(), String>;
}

impl RoomSource for Coord {
    fn ask_again(&self, by: &str, id: &str) -> Result<(), String> {
        Coord::ask_again(self, by, id).map_err(|e| e.to_string())
    }
    fn set_task_state(&self, by: &str, id: &str, state: &str, note: &str) -> Result<(), String> {
        Coord::set_task_state(self, by, id, state, note)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// What Watch follows and acts through.
#[derive(Default, Clone)]
pub struct Sources {
    pub inbox: Option<Arc<dyn InboxFeed>>,
    pub board: Option<Arc<dyn BoardSource>>,
    /// Every desktop device shows each open ask, any of them answers, and
    /// the card clears everywhere once the asking device resolves it.
    pub asks: Option<Arc<dyn AskSource>>,
    /// The person at this device (`user:<device>`): presses act as them,
    /// and the Board's Needs you means them.
    pub user: String,
    /// Whether a key is the person's on any of their devices; `None`
    /// matches exactly `user`.
    pub is_owner: Option<OwnerFn>,
    /// An agent key's display name; `None` shows the key.
    pub name: Option<NameFn>,
    /// When set, an ask nobody took gets a card with Wake them and Close.
    pub room: Option<Arc<dyn RoomSource>>,
}

/// A running Watch. [`WatchHandle::stop`] ends it and waits.
pub struct WatchHandle {
    cancel: CancellationToken,
    thread: Option<JoinHandle<()>>,
}

impl WatchHandle {
    pub fn stop(mut self) {
        self.cancel.cancel();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Starts Watch on its own thread; `None` (at once) for a headless
/// Notifier.
pub fn watch(n: &Notifier, s: Sources) -> Option<WatchHandle> {
    if n.is_headless() {
        return None;
    }
    let cancel = CancellationToken::new();
    let (n, c) = (n.clone(), cancel.clone());
    // Watch returns once it has subscribed and taken its first look, so a
    // change made right after it starts is never missed.
    let (ready_tx, ready) = std::sync::mpsc::channel::<()>();
    let thread = std::thread::Builder::new()
        .name("notify-watch".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread().build() {
                Ok(rt) => rt,
                Err(e) => {
                    eprintln!("irori: notifications: watch: {e}");
                    return;
                }
            };
            rt.block_on(run(n, s, c, ready_tx));
        })
        .ok()?;
    let _ = ready.recv(); // an error: the thread ended (no runtime)
    Some(WatchHandle {
        cancel,
        thread: Some(thread),
    })
}

/// A held card's press or x, from the card's callback thread to the loop.
type Presses = mpsc::Sender<CardAct>;

async fn run(
    n: Notifier,
    s: Sources,
    cancel: CancellationToken,
    ready: std::sync::mpsc::Sender<()>,
) {
    let (ptx, mut prx) = mpsc::channel::<CardAct>(16);
    let mut w = State {
        n: n.clone(),
        inbox: Watcher::new(&s.user, s.is_owner.clone(), s.name.clone()),
        presses: ptx,
        needs: HashSet::new(),
        asks: HashSet::new(),
        unclaimed: HashMap::new(),
        returned: HashMap::new(),
        s,
    };

    let mut ach = None;
    if let Some(a) = w.s.asks.clone() {
        ach = Some(a.subscribe());
        for x in a.open() {
            w.ask(&x.id); // waiting asks get their card: it is the only way to them
        }
    }
    let mut ich = None;
    if let Some(ib) = w.s.inbox.clone() {
        ich = Some(ib.subscribe());
        w.inbox.start(ib.as_ref(), &mut sink(&w.n, &w.presses)); // what waits gets its card
    }
    let mut cch = None;
    if let Some(b) = w.s.board.clone() {
        cch = Some(b.subscribe());
        w.board(false); // seed: what is already waiting stays quiet
    }

    let _ = ready.send(());
    while ich.is_some() || cch.is_some() || ach.is_some() {
        enum Ev {
            Inbox(Option<inbox::Change>),
            Board(Option<coord::Change>),
            Ask(Option<String>),
            Press(CardAct),
            Stop,
        }
        let ev = {
            let iq = ich
                .as_ref()
                .map(|s: &inbox::Subscription| s.queue().clone());
            let cq = cch.clone();
            let aq = ach.clone();
            tokio::select! {
                _ = cancel.cancelled() => Ev::Stop,
                c = recv(iq.as_deref()), if iq.is_some() => Ev::Inbox(c),
                c = recv(cq.as_deref()), if cq.is_some() => Ev::Board(c),
                c = recv(aq.as_deref()), if aq.is_some() => Ev::Ask(c),
                Some(p) = prx.recv() => Ev::Press(p),
            }
        };
        match ev {
            Ev::Stop => return,
            Ev::Ask(None) => ach = None,
            Ev::Ask(Some(id)) => w.ask(&id),
            Ev::Inbox(None) => ich = None,
            Ev::Inbox(Some(c)) => {
                let drained = ich.as_ref().is_some_and(|q| q.is_empty());
                if let Some(ib) = w.s.inbox.clone() {
                    w.inbox
                        .on_change(&c, drained, ib.as_ref(), &mut sink(&w.n, &w.presses));
                }
            }
            Ev::Board(None) => cch = None,
            Ev::Board(Some(_)) => w.board(true),
            Ev::Press(act) => {
                if let Some(ib) = w.s.inbox.clone() {
                    let target = w
                        .inbox
                        .press(&act, ib.as_ref(), &mut sink(&w.n, &w.presses));
                    if let Some(t) = target {
                        w.n.open(t);
                    }
                }
            }
        }
    }
}

async fn recv<T>(q: Option<&Queue<T>>) -> Option<T> {
    match q {
        Some(q) => q.recv_async().await,
        None => std::future::pending().await,
    }
}

/// The inbox watcher's cards, shown by the Notifier.
struct NoteSink<'a> {
    n: &'a Notifier,
    presses: &'a Presses,
}

fn sink<'a>(n: &'a Notifier, presses: &'a Presses) -> NoteSink<'a> {
    NoteSink { n, presses }
}

impl NoteSink<'_> {
    fn act(&self, a: Option<CardAct>) -> Option<crate::Callback> {
        let a = a?;
        Some(match a {
            CardAct::Open(t) => {
                let n = self.n.clone();
                Arc::new(move || n.open(t.clone()))
            }
            other => {
                let tx = self.presses.clone();
                Arc::new(move || {
                    // Full: the messages stay held either way.
                    let _ = tx.try_send(other.clone());
                })
            }
        })
    }
}

impl Sink for NoteSink<'_> {
    fn post(&mut self, note: inbox::watch::Note) {
        let actions = note
            .actions
            .into_iter()
            .map(|a| Action {
                label: a.label,
                run: self.act(a.run),
            })
            .collect();
        let n = Note {
            id: note.id,
            title: note.title,
            body: note.body,
            level: note.level,
            silent: note.silent,
            sticky: note.sticky,
            actions,
            on_click: self.act(note.on_click),
            on_dismiss: self.act(note.on_dismiss),
            ..Note::default()
        };
        self.n.post(n);
    }
    fn clear(&mut self, id: &str) {
        self.n.clear(id);
    }
    fn live(&self, id: &str) -> bool {
        self.n.live(id)
    }
}

/// One opening of an ask (posted, reopened, given back, or its taker
/// gone): when, and why. When alone is no identity: the clock coord
/// stamps rows with is coarse, so a post and the reopen of its Wake them
/// can carry the same since; every reopening row also names its reason.
#[derive(Debug, Clone, PartialEq)]
struct Opening {
    since: GoTime,
    reason: String,
}

/// The loop's state: the inbox half, and the Board and ask cards.
struct State {
    n: Notifier,
    s: Sources,
    inbox: Watcher,
    presses: Presses,
    /// Task ids in Needs you (card posted, or seeded).
    needs: HashSet<String>,
    /// Ask ids with a card up.
    asks: HashSet<String>,
    /// Board asks nobody took, by the opening each card showed.
    unclaimed: HashMap<String, Opening>,
    /// Cards that came back to the person, by the row that returned them.
    returned: HashMap<String, String>,
}

/// The Board half of the watcher, also usable without the loop (tests).
pub struct BoardCards {
    n: Notifier,
    user: String,
    is_owner: Option<OwnerFn>,
    name: Option<NameFn>,
    room: Option<Arc<dyn RoomSource>>,
    needs: HashSet<String>,
    unclaimed: HashMap<String, Opening>,
    returned: HashMap<String, String>,
}

impl BoardCards {
    pub fn new(n: &Notifier, s: &Sources) -> BoardCards {
        BoardCards {
            n: n.clone(),
            user: s.user.clone(),
            is_owner: s.is_owner.clone(),
            name: s.name.clone(),
            room: s.room.clone(),
            needs: HashSet::new(),
            unclaimed: HashMap::new(),
            returned: HashMap::new(),
        }
    }

    /// Reconciles the cards with one Board view (`post` false seeds).
    pub fn view(&mut self, v: &BoardView, post: bool) {
        let st = Ctx {
            n: &self.n,
            user: &self.user,
            is_owner: &self.is_owner,
            name: &self.name,
            room: &self.room,
        };
        st.board(
            v,
            post,
            &mut self.needs,
            &mut self.unclaimed,
            &mut self.returned,
        );
    }

    /// Go `returnedCards` alone.
    pub fn returned(&mut self, v: &BoardView, post: bool) {
        let st = Ctx {
            n: &self.n,
            user: &self.user,
            is_owner: &self.is_owner,
            name: &self.name,
            room: &self.room,
        };
        st.returned_cards(v, post, &mut self.returned);
    }

    /// Whether a task's move into Needs you is news to the person.
    pub fn agent_moved(&self, t: &Task) -> bool {
        self.ctx().agent_moved(t)
    }

    /// Who a Needs you card is about.
    pub fn needs_key(&self, t: &Task) -> String {
        self.ctx().needs_key(t)
    }

    fn ctx(&self) -> Ctx<'_> {
        Ctx {
            n: &self.n,
            user: &self.user,
            is_owner: &self.is_owner,
            name: &self.name,
            room: &self.room,
        }
    }
}

/// The read-only parts the Board and ask cards need.
struct Ctx<'a> {
    n: &'a Notifier,
    user: &'a str,
    is_owner: &'a Option<OwnerFn>,
    name: &'a Option<NameFn>,
    room: &'a Option<Arc<dyn RoomSource>>,
}

impl State {
    fn ctx(&self) -> Ctx<'_> {
        Ctx {
            n: &self.n,
            user: &self.s.user,
            is_owner: &self.s.is_owner,
            name: &self.s.name,
            room: &self.s.room,
        }
    }

    fn board(&mut self, post: bool) {
        let Some(b) = self.s.board.clone() else {
            return;
        };
        let v = b.board(&BoardFilter {
            viewer: self.s.user.clone(),
            done_limit: 1,
            ..BoardFilter::default()
        });
        let (mut needs, mut unclaimed, mut returned) = (
            std::mem::take(&mut self.needs),
            std::mem::take(&mut self.unclaimed),
            std::mem::take(&mut self.returned),
        );
        self.ctx()
            .board(&v, post, &mut needs, &mut unclaimed, &mut returned);
        self.needs = needs;
        self.unclaimed = unclaimed;
        self.returned = returned;
    }

    /// Brings one ask's card in line: posted while it waits for an answer,
    /// cleared once it is resolved (on any device's answer, or at its end).
    fn ask(&mut self, id: &str) {
        let Some(asks) = self.s.asks.clone() else {
            return;
        };
        let Some(a) = asks.get(id) else { return };
        if a.resolved {
            if self.asks.remove(id) {
                self.n.clear(&ask_id(id));
            }
            return;
        }
        if self.asks.contains(id) {
            return;
        }
        let left = a.until.unix_nano() as i128
            - GoTime::from_system(std::time::SystemTime::now()).unix_nano() as i128;
        if left <= 0 {
            return;
        }
        self.asks.insert(id.to_string());
        let mut title = a.name.trim().to_string();
        if title.is_empty() {
            title = self.ctx().name(&a.by);
        }
        if !a.title.is_empty() {
            title = format!("{title}: {}", a.title);
        }
        let level = parse_level(&a.level).unwrap_or_default();
        let user = self.s.user.clone();
        let answer = {
            let asks = asks.clone();
            let id = id.to_string();
            move |choice: &str| {
                if let Err(e) = asks.answer(&id, choice, &user) {
                    eprintln!("irori: answer {id}: {e}");
                }
            }
        };
        let answer = Arc::new(answer);
        let actions = a
            .actions
            .iter()
            .map(|l| {
                let (f, l2) = (answer.clone(), l.clone());
                Action::new(l.clone(), move || f(&l2))
            })
            .collect();
        let f = answer.clone();
        self.n.post(Note {
            id: ask_id(id),
            title,
            body: a.text.clone(),
            level,
            expire: std::time::Duration::from_nanos(left as u64),
            actions,
            on_dismiss: Some(Arc::new(move || f(""))),
            ..Note::default()
        });
    }
}

impl Ctx<'_> {
    fn name(&self, key: &str) -> String {
        if let Some(f) = self.name {
            let s = f(key);
            let s = s.trim();
            if !s.is_empty() {
                return s.to_string();
            }
        }
        if key.is_empty() {
            return "someone".into();
        }
        key.to_string()
    }

    /// Whether key is the person's (`is_owner`, else `user`).
    fn owner(&self, key: &str) -> bool {
        match self.is_owner {
            Some(f) => f(key),
            None => !self.user.is_empty() && key == self.user,
        }
    }

    /// Reconciles the Needs you lane with the cards: a task that entered it
    /// gets a card (when post), one that left has its card cleared.
    fn board(
        &self,
        v: &BoardView,
        post: bool,
        needs: &mut HashSet<String>,
        unclaimed: &mut HashMap<String, Opening>,
        returned: &mut HashMap<String, String>,
    ) {
        let mut now = HashSet::new();
        for c in &v.needs_you {
            now.insert(c.task.id.clone());
            if needs.contains(&c.task.id) {
                continue;
            }
            needs.insert(c.task.id.clone());
            if post && self.agent_moved(&c.task) {
                self.n.post(self.needs_note(c));
            }
        }
        let left: Vec<String> = needs
            .iter()
            .filter(|id| !now.contains(*id))
            .cloned()
            .collect();
        for id in left {
            needs.remove(&id);
            self.n.clear(&needs_id(&id));
        }
        if self.room.is_some() {
            self.unclaimed_asks(v, post, unclaimed);
        }
        self.returned_cards(v, post, returned);
    }

    fn lanes(v: &BoardView) -> impl Iterator<Item = &Card> {
        v.needs_you
            .iter()
            .chain(&v.blocked)
            .chain(&v.working)
            .chain(&v.open)
    }

    /// A card for each card that came back to the person because the agent
    /// on it went away, cleared once someone is on it again or it ends.
    fn returned_cards(&self, v: &BoardView, post: bool, returned: &mut HashMap<String, String>) {
        let mut now = HashSet::new();
        for c in Self::lanes(v) {
            let Some(r) = &c.task.returned else { continue };
            if !r.person || !c.task.holders.is_empty() {
                continue;
            }
            now.insert(c.task.id.clone());
            // The row, not the time: a card returned again within one clock
            // step carries the same at (card #25).
            let key = if r.row.is_empty() {
                r.at.format_rfc3339_nano()
            } else {
                r.row.clone()
            };
            if returned.get(&c.task.id) == Some(&key) {
                continue;
            }
            returned.insert(c.task.id.clone(), key);
            if post {
                self.n.post(self.returned_note(&c.task));
            }
        }
        let left: Vec<String> = returned
            .keys()
            .filter(|id| !now.contains(*id))
            .cloned()
            .collect();
        for id in left {
            returned.remove(&id);
            self.n.clear(&returned_id(&id));
        }
    }

    fn returned_note(&self, t: &Task) -> Note {
        let mut title = format!("Nobody is on “{}”", preview(&t.title));
        if !t.short.is_empty() {
            title += &format!(" (#{})", t.short);
        }
        let was = t
            .returned
            .as_ref()
            .map(|r| r.was.clone())
            .unwrap_or_default();
        Note {
            id: returned_id(&t.id),
            title,
            body: format!(
                "{} went away, so it's open again. Assign it to an agent, or close it.",
                self.name(&was)
            ),
            level: Level::Warn,
            sticky: true,
            on_click: Some(self.opener(t)),
            ..Note::default()
        }
    }

    fn opener(&self, t: &Task) -> crate::Callback {
        let n = self.n.clone();
        let target = Target {
            tab: TAB_BOARD.into(),
            ref_: format!("task:{}", t.id),
        };
        Arc::new(move || n.open(target.clone()))
    }

    /// A card for each open ask nobody took, cleared once it is taken or
    /// closed.
    fn unclaimed_asks(&self, v: &BoardView, post: bool, unclaimed: &mut HashMap<String, Opening>) {
        let mut now = HashSet::new();
        for c in Self::lanes(v) {
            let Some(a) = &c.task.ask else { continue };
            if !a.is_open() || !a.unclaimed || !a.taker.is_empty() {
                continue;
            }
            now.insert(c.task.id.clone());
            let o = Opening {
                since: a.since,
                reason: a.reason.clone(),
            };
            if let Some(was) = unclaimed.get(&c.task.id)
                && was.since.equal(&o.since)
                && was.reason == o.reason
            {
                continue;
            }
            unclaimed.insert(c.task.id.clone(), o);
            if post {
                self.n.post(self.unclaimed_note(&c.task));
            }
        }
        let left: Vec<String> = unclaimed
            .keys()
            .filter(|id| !now.contains(*id))
            .cloned()
            .collect();
        for id in left {
            unclaimed.remove(&id);
            self.n.clear(&unclaimed_id(&id));
        }
    }

    fn unclaimed_note(&self, t: &Task) -> Note {
        let mut title = format!("Nobody took “{}”", preview(&t.title));
        if !t.short.is_empty() {
            title += &format!(" (#{})", t.short);
        }
        let a = t.ask.clone().unwrap_or_default();
        let woken = a.reason == coord::TAKE_REOPENED;
        let body = if woken && a.told == 0 {
            "You woke them, but nobody is free to hear it.".to_string()
        } else if a.told > 0 && (woken || a.wake) {
            format!("{} agents heard it; none took it.", a.told)
        } else if a.told > 0 {
            format!(
                "{} agents heard it at their next turn; none took it.",
                a.told
            )
        } else {
            "Nobody was free to hear it.".to_string()
        };
        let (room, user, id) = (self.room.clone(), self.user.to_string(), t.id.clone());
        let (room2, user2, id2) = (room.clone(), user.clone(), id.clone());
        Note {
            id: unclaimed_id(&t.id),
            title,
            body,
            level: Level::Warn,
            sticky: true,
            actions: vec![
                Action::new("Wake them", move || {
                    if let Some(r) = &room
                        && let Err(e) = r.ask_again(&user, &id)
                    {
                        eprintln!("notify: ask again {id}: {e}");
                    }
                }),
                Action::new("Close", move || {
                    if let Some(r) = &room2
                        && let Err(e) =
                            r.set_task_state(&user2, &id2, coord::CANCELED, "closed by the person")
                    {
                        eprintln!("notify: close ask {id2}: {e}");
                    }
                }),
            ],
            on_click: Some(self.opener(t)),
            ..Note::default()
        }
    }

    /// Whether t's move into Needs you is news to the person: not their own
    /// doing, and about someone besides them.
    fn agent_moved(&self, t: &Task) -> bool {
        if self.owner(&t.updated_by) {
            return false;
        }
        !self.needs_key(t).is_empty()
    }

    /// Who a Needs you card is about: its first holder that isn't the
    /// person, else its creator unless that is the person.
    fn needs_key(&self, t: &Task) -> String {
        if let Some(h) = t.holders.iter().find(|h| !self.owner(h)) {
            return h.clone();
        }
        if !self.owner(&t.created_by) {
            return t.created_by.clone();
        }
        String::new()
    }

    fn needs_note(&self, c: &Card) -> Note {
        let t = &c.task;
        let agent = self.name(&self.needs_key(t));
        let (mut title, body) = if t.state == coord::BLOCKED {
            let mut body = t.reason.clone();
            if !t.title.is_empty() && t.title != t.reason {
                body += &format!(" · {}", t.title);
            }
            (format!("{agent} is blocked on you"), body)
        } else if t.state == coord::INPUT_REQUIRED {
            let mut body = t.title.clone();
            if !t.note.is_empty() {
                body += &format!(" · {}", t.note);
            }
            (format!("{agent} needs your input"), body)
        } else if t.holders.iter().any(|h| self.owner(h)) {
            (format!("{agent} handed you a card"), t.title.clone())
        } else {
            (format!("{agent} needs you"), t.title.clone())
        };
        if !t.short.is_empty() {
            title += &format!(" (#{})", t.short); // what the person says to an agent about it
        }
        let open = self.opener(t);
        let open2 = open.clone();
        Note {
            id: needs_id(&t.id),
            title,
            body: preview(&body),
            level: Level::Warn,
            sticky: true,
            actions: vec![
                Action::new("Answer", move || open2()),
                Action::close("Later"),
            ],
            on_click: Some(open),
            ..Note::default()
        }
    }
}
