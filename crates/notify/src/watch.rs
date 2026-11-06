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
//! Board is reconciled after every coord change and the inbox against a
//! fresh snapshot of its open deliveries whenever its queue has drained.
//! A Board reconcile reads the whole Board under coord's lock, so a burst
//! of changes shares them: a lone change reconciles at once, the rest at
//! most once per [`BOARD_EVERY`], and always once more after the last
//! change (card #554). A change to a card in the Needs you lane never
//! waits: answered and asked back inside one interval, it would look as if
//! it never left, and the new question would get no card.
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
use coord::notifications::{self as history, PostedRow};
use coord::{BoardFilter, BoardView, Card, Coord, Task};
use gocompat::sub::{Queue, TryRecv};
use gocompat::time::GoTime;
use inbox::watch::{CardAct, InboxSource, NameFn, OwnerFn, Sink, Watcher};
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

pub use inbox::watch::{hold_reason, preview, upper};

use crate::{
    Action, Level, Note, Notifier, TAB_BOARD, Target, ask_id, needs_id, parse_level, returned_id,
    unclaimed_id,
};

/// The least time between two Board reconciles, from the end of one to the
/// start of the next: the pace of the window's own Board pump.
pub const BOARD_EVERY: Duration = Duration::from_millis(250);

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
    /// The person's Done on a card's open done check (card #112, #130):
    /// resolve it with Done and close the card, as the window's Done does.
    fn mark_done(&self, by: &str, id: &str) -> Result<(), String>;
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
    fn mark_done(&self, by: &str, id: &str) -> Result<(), String> {
        mark_done(self, by, id).map(|_| ())
    }
}

/// The person's Done on card `id`'s done check, from a finished card's
/// Mark Done (cards #112, #130): resolve it with Done and close the card as
/// `by` in one step, as the window's Done does (card #315). The notices are for the agent that
/// finished it; the caller routes them. Refused while the card has any
/// other open question (one raised after the card went up), or no check.
pub fn mark_done(co: &Coord, by: &str, id: &str) -> Result<Vec<coord::Notice>, String> {
    let t = co
        .get_task(id)
        .ok_or_else(|| format!("no such card {id}"))?;
    let open = t.open_blockers();
    if open.iter().any(|b| !b.done_check) {
        return Err(format!(
            "{} has an open question besides its done check: open it",
            t.short_ref()
        ));
    }
    let Some(b) = open
        .iter()
        .rev()
        .find(|b| b.options.iter().any(|o| o.answer == coord::DONE_ANSWER))
    else {
        return Err(format!("{} isn't waiting on a done check", t.short_ref()));
    };
    let (_, ns) = co.say_done(by, id, &b.id, &[]).map_err(|e| e.to_string())?;
    Ok(ns)
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
    /// A repo key's short name on this device (its checkout folder's, card
    /// #488); `None` shows its short form ("local repo 5a3c1e0").
    pub repo_name: Option<NameFn>,
    /// When set, an ask nobody took gets a card with Wake them and Close.
    pub room: Option<Arc<dyn RoomSource>>,
    /// An attachment's local path by sha256, when this device holds it
    /// (never a mesh fetch): a card's thumbnail comes from it.
    pub file: Option<FileFn>,
    /// Which message cards earlier runs showed, so a restart doesn't toast
    /// them again (#364); `None` remembers only within this run.
    pub seen: Option<Arc<dyn inbox::watch::SeenStore>>,
}

/// Resolves an attachment's sha256 to its file on this device.
pub type FileFn = Arc<dyn Fn(&str) -> Option<std::path::PathBuf> + Send + Sync>;

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
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
            {
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
        inbox: {
            let w = Watcher::new(&s.user, s.is_owner.clone(), s.name.clone())
                .with_repo_name(s.repo_name.clone());
            match s.seen.clone() {
                Some(st) => w.with_seen(st),
                None => w,
            }
        },
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

    // When the last reconcile ended, and when the next is due: a change
    // inside BOARD_EVERY of the last one waits for `due`.
    let mut last = Instant::now();
    let mut due: Option<Instant> = None;

    let _ = ready.send(());
    while ich.is_some() || cch.is_some() || ach.is_some() {
        enum Ev {
            Inbox(Option<inbox::Change>),
            Board(Option<coord::Change>),
            BoardDue,
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
                _ = tokio::time::sleep_until(due.unwrap_or(last)), if due.is_some() => Ev::BoardDue,
                c = recv(aq.as_deref()), if aq.is_some() => Ev::Ask(c),
                Some(p) = prx.recv() => Ev::Press(p),
            }
        };
        match ev {
            Ev::Stop => {
                // The next start seeds quietly: what waits gets its card now.
                if due.is_some() || cch.as_ref().is_some_and(|q| !q.is_empty()) {
                    w.board(true);
                }
                return;
            }
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
            Ev::Board(None) => {
                cch = None;
                if due.take().is_some() {
                    w.board(true); // the changes that were waiting for it
                }
            }
            Ev::Board(Some(c)) if due.is_some() && !w.in_lane(&c.task_id) => {} // it reads this too
            Ev::Board(Some(c)) if last.elapsed() < BOARD_EVERY && !w.in_lane(&c.task_id) => {
                due = Some(last + BOARD_EVERY);
            }
            Ev::Board(Some(_)) | Ev::BoardDue => {
                due = None;
                // The view is read after this, so what is queued now needs
                // no turn of its own; a later change queues and comes back.
                if let Some(q) = &cch {
                    while let TryRecv::Item(_) = q.try_recv() {}
                }
                w.board(true);
                last = Instant::now();
            }
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
        let about = inbox_about(&note);
        let actions = note
            .actions
            .into_iter()
            .map(|a| Action {
                label: a.label,
                run: self.act(a.run),
            })
            .collect();
        let n = Note {
            about,
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
    file: Option<FileFn>,
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
            file: s.file.clone(),
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
            file: &self.file,
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
            file: &self.file,
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
            file: &self.file,
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
    file: &'a Option<FileFn>,
}

impl State {
    fn ctx(&self) -> Ctx<'_> {
        Ctx {
            n: &self.n,
            user: &self.s.user,
            is_owner: &self.s.is_owner,
            name: &self.s.name,
            room: &self.s.room,
            file: &self.s.file,
        }
    }

    /// Whether card `id` was in the Needs you lane at the last reconcile.
    fn in_lane(&self, id: &str) -> bool {
        self.needs.iter().any(|k| k.split('#').next() == Some(id))
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
            about: Some(PostedRow {
                kind: history::KIND_ASK.into(),
                ask_id: id.to_string(),
                from: a.by.clone(),
                ..PostedRow::default()
            }),
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

    /// The name a gone agent went by (card #483): the name source (the
    /// roster, then the names agents went by), then the card's spawn
    /// record for it; never its key.
    fn gone_name(&self, t: &Task, key: &str) -> Option<String> {
        if key.is_empty() {
            return None;
        }
        self.name
            .as_ref()
            .and_then(|f| history::agent_name_of(key, &f(key)))
            .or_else(|| history::spawned_name(t, key))
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
        // A card's key is its id, plus its done check while that is all it
        // waits on (card #140): new proof, or the last other question
        // closing, changes the key and posts the card again in place.
        let mut now = HashSet::new();
        let mut here = HashSet::new();
        for c in &v.needs_you {
            let key = needs_key_of(&c.task);
            here.insert(c.task.id.clone());
            now.insert(key.clone());
            if needs.contains(&key) {
                continue;
            }
            needs.insert(key);
            // Card #520: a wait no agent will end is news even on the
            // person's own card, which has no agent left to name.
            if post && (self.agent_moved(&c.task) || !c.gone_waits.is_empty()) {
                self.n.post(self.needs_note(c));
            }
        }
        let left: Vec<String> = needs
            .iter()
            .filter(|k| !now.contains(*k))
            .cloned()
            .collect();
        for k in left {
            needs.remove(&k);
            let id = k.split('#').next().unwrap_or_default();
            if !here.contains(id) {
                self.n.clear(&needs_id(id));
            }
        }
        if self.room.is_some() {
            self.unclaimed_asks(v, post, unclaimed);
        }
        self.returned_cards(v, post, returned);
    }

    /// The open lanes these cards watch. Not yet cards (card #570, the
    /// later lane) are left out on purpose: one put aside isn't the
    /// person's to hand to an agent ("returned"), and an ask is never
    /// held. A card that comes back lands in these lanes again.
    fn lanes(v: &BoardView) -> impl Iterator<Item = &Card> {
        v.needs_you
            .iter()
            .chain(&v.blocked)
            .chain(&v.working)
            .chain(&v.open)
    }

    /// A card for each card that came back to the person because the agent
    /// on it went away, cleared once someone is on it again or it ends.
    /// Not while it waits on the person (its Needs you lane, card #483):
    /// "assign it" means nothing on a card waiting for their Done or their
    /// answer, and once that leaves it with nobody on it the card comes.
    fn returned_cards(&self, v: &BoardView, post: bool, returned: &mut HashMap<String, String>) {
        let mut now = HashSet::new();
        let waits: HashSet<&str> = v.needs_you.iter().map(|c| c.task.id.as_str()).collect();
        for c in Self::lanes(v) {
            if waits.contains(c.task.id.as_str()) {
                continue;
            }
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
            title += &format!(" ({})", t.short_ref());
        }
        let was = t
            .returned
            .as_ref()
            .map(|r| r.was.clone())
            .unwrap_or_default();
        let mut about = about(history::KIND_RETURNED, t, &was);
        about.instance = t
            .returned
            .as_ref()
            .map(|r| {
                if r.row.is_empty() {
                    r.at.format_rfc3339_nano()
                } else {
                    r.row.clone()
                }
            })
            .unwrap_or_default();
        Note {
            id: returned_id(&t.id),
            about: Some(about),
            title,
            body: history::returned_body(self.gone_name(t, &was).as_deref()),
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
            // Irori is starting its agent (card #470): nobody to wake.
            if !a.is_open() || !a.unclaimed || !a.taker.is_empty() || c.task.spawn_pending() {
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
            title += &format!(" ({})", t.short_ref());
        }
        let a = t.ask.clone().unwrap_or_default();
        let woken = a.reason == coord::TAKE_REOPENED;
        // Nobody free: waking again would only reopen it and post this card
        // again. A waiting ask goes out by itself when an agent goes idle.
        let body = if a.told > 0 && a.thread {
            // A thread goes out at once (card #512): the agents it names
            // were woken, and nobody waits on them.
            format!(
                "{} agents it names were woken, and no agent with Offer it cards on is free.",
                a.told
            )
        } else if a.told > 0 {
            format!("{} agents were told; none took it.", a.told)
        } else if woken {
            "You woke them, but nobody is free. It goes to the first agent that is.".to_string()
        } else {
            "Nobody was free. It goes to the first agent that is.".to_string()
        };
        // No button ends the ask: the person just posted it (#164).
        let (room, user, id) = (self.room.clone(), self.user.to_string(), t.id.clone());
        let open = self.opener(t);
        let open2 = open.clone();
        // Two buttons at most: told > 0 trades Open for Wake them, and a
        // click on the card still opens it.
        let first = if a.told > 0 {
            Action::new("Wake them", move || {
                if let Some(r) = &room
                    && let Err(e) = r.ask_again(&user, &id)
                {
                    eprintln!("notify: ask again {id}: {e}");
                }
            })
        } else {
            Action::new("Open", move || open2())
        };
        let actions = vec![first, Action::close("Later")];
        let mut about = about(history::KIND_UNCLAIMED, t, "");
        about.instance = a.since.format_rfc3339_nano();
        Note {
            id: unclaimed_id(&t.id),
            about: Some(about),
            title,
            body,
            level: Level::Warn,
            sticky: true,
            actions,
            on_click: Some(open),
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
        // An agent's completed waits on the person's Done (card #112): good
        // news, so green with a check, worded as ready to look over, never
        // "blocked on you" (card #130). Any other open question wins.
        let open = t.open_blockers();
        if !open.is_empty() && open.iter().all(|b| b.done_check) {
            return self.finished_note(t, &agent);
        }
        let (kind, instance) = if t.state == coord::BLOCKED {
            (history::KIND_BLOCKER, String::new())
        } else if t.state == coord::INPUT_REQUIRED {
            (history::KIND_INPUT, String::new())
        } else if t.holders.iter().any(|h| self.owner(h)) {
            (history::KIND_HANDED, t.updated_at.format_rfc3339_nano())
        } else {
            (history::KIND_BLOCKER, String::new())
        };
        let mut about = about(kind, t, &self.needs_key(t));
        // The agent asked back on the person's answer (card #482): Irori
        // read the answered item for them, so the ask-back is its own.
        let instance = if instance.is_empty()
            && t.blockers
                .iter()
                .any(|b| b.id == about.blocker_id && b.open() && b.status == coord::BLOCKER_REPLIED)
        {
            format!("replied:{}", t.updated_at.format_rfc3339_nano())
        } else {
            instance
        };
        // A question without a blocker item (an older card): which time.
        if about.blocker_id.is_empty() || !instance.is_empty() {
            about.instance = if instance.is_empty() {
                t.updated_at.format_rfc3339_nano()
            } else {
                instance
            };
        }
        // Card #509: a wait on a PR's merge, with nothing else to answer,
        // says which PR is the person's to merge.
        // Card #520: a PR wait coord counts as a gone wait (its PR merged
        // or closed, or answered) is not a merge.
        let gone_ids: Vec<&str> = c.gone_waits.iter().map(|s| s.blocker_id.as_str()).collect();
        let merge = t
            .blockers
            .iter()
            .filter(|b| !gone_ids.contains(&b.id.as_str()))
            .find_map(|b| b.merge_wait())
            .filter(|_| {
                !t.blockers.iter().any(|b| {
                    b.open()
                        && b.merge_wait().is_none()
                        && b.on
                            .as_ref()
                            .is_none_or(|on| on.kind == coord::BLOCKED_ON_PERSON)
                })
            });
        // Card #520: nobody holds it and its wait's agent is gone, with
        // nothing else to answer (an answered item waits on its agent, as
        // coord counts it): say so, not "blocked on you".
        let asks = t.blockers.iter().any(|b| {
            b.open()
                && b.status != coord::BLOCKER_ANSWERED
                && b.on
                    .as_ref()
                    .is_none_or(|on| on.kind == coord::BLOCKED_ON_PERSON)
        });
        let gone = c
            .gone_waits
            .first()
            .filter(|_| merge.is_none() && !asks && t.state != coord::INPUT_REQUIRED);
        let (mut title, body) = if t.state == coord::BLOCKED {
            let mut body = t.reason.clone();
            if !t.title.is_empty() && t.title != t.reason {
                body += &format!(" · {}", t.title);
            }
            let pr = merge
                .as_deref()
                .and_then(|u| coord::parse_link(u).ok())
                .and_then(|l| coord::pr_link(&l));
            match (pr, gone) {
                (Some((_, n)), _) => (format!("{agent} waits on you to merge PR #{n}"), body),
                (None, Some(s)) => {
                    let who = self.name(&s.agent);
                    let title = if s.waited_on {
                        format!("Nobody holds it, and {who}, whom it waits on, is gone")
                    } else {
                        format!("Nobody holds it, and {who}, who raised its wait, is gone")
                    };
                    (title, body)
                }
                (None, None) => (format!("{agent} is blocked on you"), body),
            }
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
            title += &format!(" ({})", t.short_ref()); // what the person says to an agent about it
        }
        let open = self.opener(t);
        let open2 = open.clone();
        Note {
            id: needs_id(&t.id),
            title,
            body: preview(&body),
            about: Some(about),
            level: Level::Warn,
            sticky: true,
            actions: vec![
                // Card #520: nothing to answer on a gone wait; the card
                // has Hand to an agent.
                Action::new(if gone.is_some() { "Open" } else { "Answer" }, move || {
                    open2()
                }),
                Action::close("Later"),
            ],
            on_click: Some(open),
            ..Note::default()
        }
    }

    /// A card an agent finished, waiting on the person's Done (card #130):
    /// Open shows it; Mark Done closes it from here when this device can
    /// act on the room, else Later.
    fn finished_note(&self, t: &Task, agent: &str) -> Note {
        let mut title = format!("{agent} finished “{}”", preview(&t.title));
        if !t.short.is_empty() {
            title += &format!(" ({})", t.short_ref());
        }
        let open = self.opener(t);
        let open2 = open.clone();
        let mut actions = vec![Action::new("Open", move || open2())];
        match &self.room {
            Some(r) => {
                let (r, user, id) = (r.clone(), self.user.to_string(), t.id.clone());
                actions.push(Action::new("Mark Done", move || {
                    if let Err(e) = r.mark_done(&user, &id) {
                        eprintln!("notify: mark done {id}: {e}");
                    }
                }));
            }
            None => actions.push(Action::close("Later")),
        }
        let mut about = about(history::KIND_DONE_CHECK, t, &self.needs_key(t));
        if let Some(b) = t.blockers.iter().rev().find(|b| b.open() && b.done_check) {
            about.blocker_id = b.id.clone();
        }
        Note {
            id: needs_id(&t.id),
            title,
            about: Some(about),
            body: finished_body(t),
            image: finished_shot(t)
                .and_then(|sha| crate::thumb::attachment_thumbnail(self.file.as_ref(), sha)),
            level: Level::Success,
            sticky: true,
            actions,
            on_click: Some(open),
            ..Note::default()
        }
    }
}

/// A finished card's body (card #140): the first line of the agent's
/// proof, and how many screenshots came with it; the plain "ready" line
/// when there is no proof. The line is taken before [`preview`], which
/// flattens the whole text into one.
fn finished_body(t: &Task) -> String {
    const READY: &str = "Ready for you to look over and mark Done.";
    let Some(check) = t.blockers.iter().rev().find(|b| b.open() && b.done_check) else {
        return READY.into();
    };
    let first = check
        .context
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .filter(|_| check.context.trim() != coord::DONE_CONTEXT);
    let shots = check
        .attachments
        .iter()
        .filter(|f| f.mime.starts_with("image/"))
        .count();
    let shots = match shots {
        0 => String::new(),
        1 => " · 1 screenshot".into(),
        n => format!(" · {n} screenshots"),
    };
    match first {
        Some(l) => format!("{}{shots}", preview(l)),
        None if !shots.is_empty() => format!("{READY}{shots}"),
        None => READY.into(),
    }
}

/// The sha256 of the first screenshot on t's newest open done check, the
/// same check [`finished_body`] reads (card #149).
fn finished_shot(t: &Task) -> Option<&str> {
    t.blockers
        .iter()
        .rev()
        .find(|b| b.open() && b.done_check)?
        .attachments
        .iter()
        .find(|f| f.mime.starts_with("image/"))
        .map(|f| f.sha256.as_str())
}

/// What an inbox card is about (card #377): a message to the person is
/// its delivery; a held card is its newest held message, so the same
/// messages posted again stay one item and a new one is news.
fn inbox_about(note: &inbox::watch::Note) -> Option<PostedRow> {
    // The one card for several that waited (card #482): every message it
    // stands for, keyed by the newest so the same set stays one item.
    if note.id == inbox::watch::WAITING_ID {
        let newest = note.ids.iter().max()?.clone();
        return Some(PostedRow {
            kind: history::KIND_WAITED.into(),
            delivery_id: newest,
            delivery_ids: note.ids.clone(),
            ..PostedRow::default()
        });
    }
    if let Some(d) = note.id.strip_prefix("msg:") {
        return Some(PostedRow {
            kind: history::KIND_MESSAGE.into(),
            delivery_id: d.into(),
            ..PostedRow::default()
        });
    }
    let sender = note.id.strip_prefix("held:")?;
    let ids = note.actions.iter().find_map(|a| match &a.run {
        Some(CardAct::Held { ids, .. }) => Some(ids.clone()),
        _ => None,
    })?;
    let newest = ids.iter().max()?.clone();
    Some(PostedRow {
        kind: history::KIND_HELD.into(),
        delivery_id: newest,
        from: sender.into(),
        ..PostedRow::default()
    })
}

/// What a card's notification is about (card #377): the card, its
/// newest open blocker, and who it is from.
fn about(kind: &str, t: &Task, from: &str) -> PostedRow {
    let asks = [
        history::KIND_BLOCKER,
        history::KIND_INPUT,
        history::KIND_DONE_CHECK,
    ];
    PostedRow {
        kind: kind.into(),
        task_id: t.id.clone(),
        blocker_id: if asks.contains(&kind) {
            t.open_blockers()
                .last()
                .map(|b| b.id.clone())
                .unwrap_or_default()
        } else {
            String::new()
        },
        from: from.into(),
        ..PostedRow::default()
    }
}

/// The Needs you set's key for t: its id, and `#` and the done check it
/// waits on when every open item is one (see `board`). Task ids are ULIDs,
/// never holding a `#`.
fn needs_key_of(t: &Task) -> String {
    let open = t.open_blockers();
    match open.last() {
        Some(b) if open.iter().all(|b| b.done_check) => format!("{}#{}", t.id, b.id),
        _ => t.id.clone(),
    }
}
