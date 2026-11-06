//! The inbox half of I:internal/notify/watch.go as pure logic: held cards
//! (one per sender, decision 34), cards for messages to the person, and
//! the reconcile that survives dropped changes. No toasts here: a [`Sink`]
//! posts and clears cards, and the Go select loop becomes calls:
//! [`Watcher::start`], [`Watcher::on_change`] (pass whether the change
//! queue is now empty: Go reconciles once it drains), [`Watcher::press`]
//! for a held card's Release/Dismiss, [`Watcher::forget`] for its x.
//!
//! The Board's Needs you, unclaimed-ask, returned-card and ask cards need
//! coord's BoardView and the ask book, so they live in crates/notify
//! (watch.rs), whose loop runs this watcher over a Notifier sink; [`Note`]
//! maps onto notify::Note, and a [`CardAct`] is what the card's buttons and
//! clicks run.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use crate::{Change, ChangeKind, Delivery, Inbox, Result, hold, st};

/// The window tabs a card opens.
pub const TAB_INBOX: &str = "inbox";
pub const TAB_BOARD: &str = "board";
pub const TAB_HELD: &str = "held";
/// Talk with a channel or DM open (card #395); the ref is its id.
pub const TAB_TALK: &str = "talk";

/// Card ids (I:internal/notify/notify.go:416-421).
pub fn held_id(sender_key: &str) -> String {
    format!("held:{sender_key}")
}
pub fn message_id(delivery_id: &str) -> String {
    format!("msg:{delivery_id}")
}
/// The one quiet card at start for messages that waited while Irori was
/// closed (#364).
pub const WAITING_ID: &str = "msg:waiting";
/// A Talk post's card (card #395): the conversation, then the post.
pub fn talk_id(conv: &str, post_id: &str) -> String {
    format!("talk:{conv}/{post_id}")
}
/// The conversation of a [`talk_id`].
pub fn talk_conv(id: &str) -> Option<&str> {
    id.strip_prefix("talk:")?.split_once('/').map(|(c, _)| c)
}
pub fn needs_id(task_id: &str) -> String {
    format!("needs:{task_id}")
}
pub fn ask_id(id: &str) -> String {
    format!("ask:{id}")
}
pub fn unclaimed_id(task_id: &str) -> String {
    format!("unclaimed:{task_id}")
}
pub fn returned_id(task_id: &str) -> String {
    format!("returned:{task_id}")
}

/// A card's level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    #[default]
    Info,
    Success,
    Warn,
    Error,
}

/// Go `ParseLevel`: lowercased and trimmed; `""` is info.
pub fn parse_level(s: &str) -> Option<Level> {
    match gocompat::text::to_lower_str(crate::state::trim(s)).as_str() {
        "" | "info" => Some(Level::Info),
        "success" => Some(Level::Success),
        "warn" => Some(Level::Warn),
        "error" => Some(Level::Error),
        _ => None,
    }
}

/// Where a card's Open takes the person.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Target {
    pub tab: String,
    pub ref_: String,
}

/// What a card's button, body click or x runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardAct {
    /// Release (verb "release") or Dismiss (verb "dismiss") every id the
    /// card counted when it was built, as the person.
    Held {
        key: String,
        verb: &'static str,
        ids: Vec<String>,
        what: &'static str,
    },
    /// Open the window at a target.
    Open(Target),
    /// The held card was closed with its x: the messages stay held.
    Forget(String),
}

/// One card button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub label: String,
    /// `None`: the button only closes the card.
    pub run: Option<CardAct>,
}

/// One card (notify.Note).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub body: String,
    pub level: Level,
    pub silent: bool,
    pub sticky: bool,
    pub actions: Vec<Action>,
    pub on_click: Option<CardAct>,
    pub on_dismiss: Option<CardAct>,
    /// The deliveries a summary card stands for (the waited card), so its
    /// history item can tell when none waits any more (card #482).
    pub ids: Vec<String>,
}

/// Where cards go (the Notifier).
pub trait Sink {
    fn post(&mut self, note: Note);
    /// Closes the card without callbacks.
    fn clear(&mut self, id: &str);
    /// The card is up (posted, not cleared, not pressed or closed).
    fn live(&self, id: &str) -> bool;
}

/// The part of the inbox the watcher reads and acts through.
pub trait InboxSource {
    fn get(&self, id: &str) -> Option<Delivery>;
    fn open(&self) -> Vec<Delivery>;
    fn release(&self, id: &str, by: &str) -> Result<()>;
    fn dismiss(&self, id: &str, by: &str) -> Result<()>;
}

impl InboxSource for Inbox {
    fn get(&self, id: &str) -> Option<Delivery> {
        Inbox::get(self, id)
    }
    fn open(&self) -> Vec<Delivery> {
        Inbox::open(self)
    }
    fn release(&self, id: &str, by: &str) -> Result<()> {
        Inbox::release(self, id, by)
    }
    fn dismiss(&self, id: &str, by: &str) -> Result<()> {
        Inbox::dismiss(self, id, by)
    }
}

/// The message cards the person has already had (posted, or counted on the
/// start card), kept across restarts so a launch doesn't toast them again
/// (#364). `load` of nothing yet is empty; a failed `save` only costs a
/// repeat card next launch.
pub trait SeenStore: Send + Sync {
    fn load(&self) -> HashSet<String>;
    fn save(&self, ids: &HashSet<String>);
}

/// Shows an agent key as its display name.
pub type NameFn = Arc<dyn Fn(&str) -> String + Send + Sync>;
/// Whether a key is the person's on any of their devices.
pub type OwnerFn = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// A flush's change to a sender's card; the strongest since the last flush
/// wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum HeldChange {
    UpdateIfUp,
    PostQuiet,
    PostLoud,
}

#[derive(Debug, Default)]
struct HeldGroup {
    ids: Vec<String>,
    dels: HashMap<String, Delivery>,
}

/// The watcher's state (Go `watcher`, inbox half).
pub struct Watcher {
    user: String,
    is_owner: Option<OwnerFn>,
    name_fn: Option<NameFn>,
    /// A repo key's short name here (card #488); `None` uses repo_short.
    repo_fn: Option<NameFn>,
    held: HashMap<String, String>,
    groups: BTreeMap<String, HeldGroup>,
    dirty: BTreeMap<String, HeldChange>,
    closed: HashSet<String>,
    msgs: HashSet<String>,
    fails: HashMap<String, String>,
    /// Delivery ids whose message card the person already had (#364).
    seen: HashSet<String>,
    seen_store: Option<Arc<dyn SeenStore>>,
    /// During start: unseen waiting messages go here, for one quiet card,
    /// instead of a card each.
    starting: Option<Vec<Delivery>>,
    /// What the start card counts; it clears once none of them waits.
    waiting: HashSet<String>,
}

/// A hold the person is asked about (Pause all is their own).
fn card_held(d: &Delivery) -> bool {
    d.state == st::HELD && d.hold != hold::PAUSED && d.hold != hold::EDITING
}

/// A hold in the person's words (card #488: no internal reasons; the
/// window's own sentences, frontend/src/lib/talk.ts holdText, also name
/// who).
pub fn hold_reason(h: &str) -> String {
    match h {
        hold::BLOCKED => "the sender is on your block list",
        hold::NOT_ALLOWED => "the sender isn't on your allow list",
        hold::BYPASS_RECEIVER => "an older Irori held it, by a rule that's gone now",
        hold::HOP_LIMIT => "these agents kept messaging each other without stopping",
        hold::UNOWNED => "no agent is working in that repo to take it yet",
        hold::PAUSED => "you paused wakes",
        hold::MEMBER_WAKE => "another person asked to wake your agent, and your receive setting for that space asks you first",
        hold::CROSS_PERSON => "it comes from another person in a shared space, and your receive setting for that space asks you first",
        hold::MEMBER_LEASE => "another person asks your agent to give up a lease",
        hold::MEMBER_REMOVED => "it comes from a person who is no longer in that space",
        hold::EDITING => "you are editing it",
        hold::OFF_SIDE => "Irori picked this agent for a card on a side its device doesn't run",
        _ => "Irori held it",
    }
    .to_string()
}

/// A repo key as a person reads it, with `root` this device's checkout of
/// it if any (card #488): a local repo (root:<sha>) by its checkout
/// folder's name ("ww-multiplayer"), else as [`repo_short`].
pub fn repo_short_name(key: &str, root: Option<&str>) -> String {
    let folder = root
        .filter(|_| key.starts_with("root:"))
        .map(|r| r.trim_end_matches(['/', '\\']))
        .and_then(|r| r.rsplit(['/', '\\']).next())
        .filter(|f| !f.is_empty());
    match folder {
        Some(f) => f.to_string(),
        None => repo_short(key),
    }
}

/// A repo key in a notification (card #488): its last part ("irori" for
/// remote:github.com/o/irori), or "local repo 5a3c1e0" for a root-commit
/// key, never the whole key. The window names a local repo by its folder.
pub fn repo_short(key: &str) -> String {
    if let Some(sha) = key.strip_prefix("root:") {
        return format!("local repo {}", sha.get(..7).unwrap_or(sha));
    }
    let k = key.trim_end_matches('/');
    let k = k.strip_suffix(".git").unwrap_or(k);
    match k.rfind(['/', ':', '\\']) {
        Some(i) if i < k.len() - 1 => k[i + 1..].to_string(),
        _ => k.to_string(),
    }
}

/// Text on one line, cut to what two card lines hold (160 runes).
pub fn preview(s: &str) -> String {
    let s = crate::search::one_line(s);
    const MAX: usize = 160;
    if s.chars().count() > MAX {
        let cut: String = s.chars().take(MAX - 1).collect();
        return format!("{}…", crate::state::trim(&cut));
    }
    s
}

/// Uppercases the first rune.
pub fn upper(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(r) => {
            let mut out = gocompat::text::to_upper_str(&r.to_string());
            out.push_str(c.as_str());
            out
        }
    }
}

impl Watcher {
    /// `user` is the person at this device (`user:<device>`).
    pub fn new(user: &str, is_owner: Option<OwnerFn>, name: Option<NameFn>) -> Watcher {
        Watcher {
            user: user.into(),
            is_owner,
            name_fn: name,
            repo_fn: None,
            held: HashMap::new(),
            groups: BTreeMap::new(),
            dirty: BTreeMap::new(),
            closed: HashSet::new(),
            msgs: HashSet::new(),
            fails: HashMap::new(),
            seen: HashSet::new(),
            seen_store: None,
            starting: None,
            waiting: HashSet::new(),
        }
    }

    /// Names repos by their checkout folder here (card #488).
    pub fn with_repo_name(mut self, f: Option<NameFn>) -> Watcher {
        self.repo_fn = f;
        self
    }

    /// Remembers across restarts which message cards the person had.
    pub fn with_seen(mut self, store: Arc<dyn SeenStore>) -> Watcher {
        self.seen = store.load();
        self.seen_store = Some(store);
        self
    }

    /// At start: held messages from before get their cards. Messages to
    /// the person that waited get one quiet card between them, and only
    /// those no earlier run showed (#364): a restart is not news.
    pub fn start(&mut self, src: &dyn InboxSource, sink: &mut dyn Sink) {
        self.starting = Some(Vec::new());
        self.reconcile(src, sink, true);
        let fresh = self.starting.take().unwrap_or_default();
        match fresh.as_slice() {
            [] => {}
            [d] => {
                let mut note = self.message_note(d);
                note.silent = true;
                sink.post(note);
            }
            _ => {
                self.waiting = fresh.iter().map(|d| d.id.clone()).collect();
                sink.post(self.waiting_note(&fresh));
            }
        }
        if !fresh.is_empty() {
            self.seen.extend(fresh.iter().map(|d| d.id.clone()));
            self.save_seen();
        }
    }

    fn save_seen(&self) {
        if let Some(s) = &self.seen_store {
            s.save(&self.seen);
        }
    }

    /// One inbox change; `drained` is Go's `len(ich) == 0` after reading it.
    pub fn on_change(
        &mut self,
        c: &Change,
        drained: bool,
        src: &dyn InboxSource,
        sink: &mut dyn Sink,
    ) {
        if !c.delivery_id.is_empty()
            && let Some(d) = src.get(&c.delivery_id)
        {
            self.delivery(&d, c.kind == ChangeKind::HeldNew, true, sink);
            self.flush(sink);
        }
        if drained {
            self.reconcile(src, sink, true);
        }
    }

    /// Runs a card action. A held press acts as the person and puts the card
    /// back (quietly) for whatever is still held, with the error if any; an
    /// Open returns its target for the window.
    pub fn press(
        &mut self,
        act: &CardAct,
        src: &dyn InboxSource,
        sink: &mut dyn Sink,
    ) -> Option<Target> {
        match act {
            CardAct::Open(t) => Some(t.clone()),
            CardAct::Forget(key) => {
                self.forget(key);
                None
            }
            CardAct::Held {
                key,
                verb,
                ids,
                what,
            } => {
                let mut first: Option<String> = None;
                for id in ids {
                    let r = if *verb == "release" {
                        src.release(id, &self.user)
                    } else {
                        src.dismiss(id, &self.user)
                    };
                    if let Err(e) = r
                        && src.get(id).is_some_and(|d| d.state == st::HELD)
                    {
                        eprintln!("irori: notifications: {verb} {id}: {e}");
                        first.get_or_insert(e.to_string());
                    }
                }
                let err = first
                    .map(|e| format!("Couldn't {verb} {what}: {e}"))
                    .unwrap_or_default();
                self.failed(key, &err, src, sink);
                None
            }
        }
    }

    /// The held card of sender key was closed with its x: the next reconcile
    /// posts it again, without a sound.
    pub fn forget(&mut self, key: &str) {
        if let Some(g) = self.groups.remove(key) {
            for id in g.ids {
                self.held.remove(&id);
                self.closed.insert(id);
            }
        }
    }

    fn failed(&mut self, key: &str, err: &str, src: &dyn InboxSource, sink: &mut dyn Sink) {
        if let Some(g) = self.groups.get(key) {
            for id in g.ids.clone() {
                match src.get(&id) {
                    Some(d) => self.delivery(&d, false, false, sink),
                    None => self.unhold(&id),
                }
            }
        }
        if err.is_empty() {
            self.fails.remove(key);
        } else {
            self.fails.insert(key.into(), err.into());
        }
        self.bump(key, HeldChange::PostQuiet);
        self.flush(sink);
    }

    fn bump(&mut self, key: &str, c: HeldChange) {
        let e = self
            .dirty
            .entry(key.into())
            .or_insert(HeldChange::UpdateIfUp);
        *e = (*e).max(c);
    }

    fn unhold(&mut self, id: &str) {
        let Some(key) = self.held.remove(id) else {
            return;
        };
        if let Some(g) = self.groups.get_mut(&key) {
            g.ids.retain(|s| s != id);
            g.dels.remove(id);
        }
        self.bump(&key, HeldChange::UpdateIfUp);
    }

    fn flush(&mut self, sink: &mut dyn Sink) {
        let dirty = std::mem::take(&mut self.dirty);
        for (key, c) in dirty {
            let id = held_id(&key);
            if self.groups.get(&key).is_none_or(|g| g.ids.is_empty()) {
                self.groups.remove(&key);
                self.fails.remove(&key);
                sink.clear(&id);
                continue;
            }
            if c == HeldChange::UpdateIfUp && !sink.live(&id) {
                continue;
            }
            let mut note = self.held_note(&key);
            note.silent = c != HeldChange::PostLoud || sink.live(&id);
            sink.post(note);
        }
    }

    fn name(&self, key: &str) -> String {
        if let Some(f) = &self.name_fn {
            let s = f(key);
            let s = crate::state::trim(&s);
            if !s.is_empty() {
                return s.into();
            }
        }
        if key.is_empty() {
            return "someone".into();
        }
        key.into()
    }

    fn owner(&self, key: &str) -> bool {
        match &self.is_owner {
            Some(f) => f(key),
            None => !self.user.is_empty() && key == self.user,
        }
    }

    fn waiting_for(&self, d: &Delivery) -> bool {
        self.owner(&d.to) && (d.state == st::QUEUED || d.state == st::DELIVERED)
    }

    /// Brings d's cards in line with its state (held cards at the flush).
    fn delivery(&mut self, d: &Delivery, mut held_again: bool, post: bool, sink: &mut dyn Sink) {
        let id = d.id.clone();
        let mut accounted = self.held.contains_key(&id);
        if card_held(d) {
            let key = d.message.from.key.clone();
            if accounted && self.held[&id] != key {
                self.unhold(&id);
                accounted = false;
            }
            let g = self.groups.entry(key.clone()).or_default();
            if !accounted {
                g.ids.push(id.clone());
            }
            let prev_hold = g.dels.get(&id).map(|p| p.hold.clone()).unwrap_or_default();
            g.dels.insert(id.clone(), d.clone());
            self.held.insert(id.clone(), key.clone());
            if accounted && held_again && prev_hold == d.hold && sink.live(&held_id(&key)) {
                held_again = false; // the card already shows it, for the same reason
            }
            if (!accounted || held_again) && post {
                let c = if self.closed.contains(&id) && !held_again {
                    HeldChange::PostQuiet // closed with the x: back, without a sound
                } else {
                    HeldChange::PostLoud
                };
                self.bump(&key, c);
            }
            self.closed.remove(&id);
        } else if accounted {
            self.unhold(&id);
        }
        let waiting = self.waiting_for(d);
        if waiting && !self.msgs.contains(&id) {
            self.msgs.insert(id.clone());
            if let Some(fresh) = self.starting.as_mut() {
                if !self.seen.contains(&id) {
                    fresh.push(d.clone());
                }
            } else if post && !self.seen.contains(&id) {
                sink.post(self.message_note(d));
                self.seen.insert(id.clone());
                self.save_seen();
            }
        } else if !waiting && self.msgs.contains(&id) {
            self.msgs.remove(&id);
            sink.clear(&message_id(&id));
            if self.waiting.remove(&id) && self.waiting.is_empty() {
                sink.clear(WAITING_ID);
            }
        }
    }

    /// Re-reads the open deliveries so a dropped change can't leave a card
    /// wrong. post false seeds without cards.
    pub fn reconcile(&mut self, src: &dyn InboxSource, sink: &mut dyn Sink, post: bool) {
        let mut open = HashSet::new();
        for d in src.open() {
            open.insert(d.id.clone());
            self.delivery(&d, false, post, sink);
        }
        let mut left: Vec<String> = self
            .held
            .keys()
            .filter(|id| !open.contains(*id))
            .cloned()
            .collect();
        left.extend(self.msgs.iter().filter(|id| !open.contains(*id)).cloned());
        left.sort();
        left.dedup();
        self.closed.retain(|id| open.contains(id));
        // Only what is still open needs remembering.
        let before = self.seen.len();
        self.seen.retain(|id| open.contains(id));
        if self.seen.len() != before {
            self.save_seen();
        }
        for id in left {
            if let Some(d) = src.get(&id) {
                self.delivery(&d, false, post, sink);
                continue;
            }
            self.unhold(&id);
            if self.msgs.remove(&id) {
                sink.clear(&message_id(&id));
            }
        }
        self.flush(sink);
    }

    /// A repo by its short name: its checkout folder's here, when the
    /// window gave a way to find it, else [`repo_short`] (card #488).
    fn repo_name(&self, key: &str) -> String {
        if let Some(f) = &self.repo_fn {
            let s = f(key);
            if !s.trim().is_empty() {
                return s.trim().into();
            }
        }
        repo_short(key)
    }

    fn held_to(&self, d: &Delivery) -> String {
        if d.to.is_empty() && !d.repo_key.is_empty() {
            return self.repo_name(&d.repo_key);
        }
        if d.to != self.user {
            return self.name(&d.to);
        }
        "you".into()
    }

    fn held_note(&self, key: &str) -> Note {
        let g = &self.groups[key];
        let ids = g.ids.clone();
        // The newest by id (ids are minted sortable).
        let last_id = ids
            .iter()
            .max_by(|a, b| {
                let ka = format!("{}{a}", g.dels[*a].message.id);
                let kb = format!("{}{b}", g.dels[*b].message.id);
                ka.cmp(&kb)
            })
            .expect("a held group is never empty");
        let last = &g.dels[last_id];
        let mut from = last.message.from.name.clone();
        if from.is_empty() {
            from = self.name(key);
        }
        let mut to = self.held_to(last);
        if ids.iter().any(|id| self.held_to(&g.dels[id]) != to) {
            to.clear();
        }
        let n = ids.len();
        let title = if n == 1 {
            format!("{from} sent {to} a message Irori held")
        } else if to.is_empty() {
            format!("{from} sent {n} messages Irori held")
        } else {
            format!("{from} sent {to} {n} messages Irori held")
        };
        let (what, release, dismiss) = if n > 1 {
            ("those messages", "Release all", "Dismiss all")
        } else {
            ("that message", "Release", "Dismiss")
        };
        let mut body = format!(
            "{}: {}",
            upper(&hold_reason(&last.hold)),
            preview(&last.message.text)
        );
        let mut level = Level::Warn;
        if let Some(e) = self.fails.get(key).filter(|e| !e.is_empty()) {
            body = format!("{e}. {body}");
            level = Level::Error;
        }
        let act = |verb: &'static str| CardAct::Held {
            key: key.into(),
            verb,
            ids: ids.clone(),
            what,
        };
        // Card #488: the person's own message no agent in its repo can
        // take yet. Release has nobody to deliver to (it only fails); the
        // Held list's Start an agent is what helps.
        let open_held = CardAct::Open(Target {
            tab: TAB_HELD.into(),
            ref_: key.into(),
        });
        let first = if self.owner(key)
            && ids.iter().all(|id| {
                let d = &g.dels[id];
                d.hold == hold::UNOWNED && d.to.is_empty() && !d.repo_key.is_empty()
            }) {
            Action {
                label: format!("Start an agent in {}", self.repo_name(&last.repo_key)),
                run: Some(open_held.clone()),
            }
        } else {
            Action {
                label: release.into(),
                run: Some(act("release")),
            }
        };
        Note {
            id: held_id(key),
            title,
            body,
            level,
            sticky: true,
            actions: vec![
                first,
                Action {
                    label: dismiss.into(),
                    run: Some(act("dismiss")),
                },
            ],
            on_click: Some(CardAct::Open(Target {
                tab: TAB_HELD.into(),
                ref_: key.into(),
            })),
            on_dismiss: Some(CardAct::Forget(key.into())),
            ..Note::default()
        }
    }

    /// The start card for several messages that waited (#364).
    fn waiting_note(&self, ds: &[Delivery]) -> Note {
        let mut names: Vec<String> = Vec::new();
        for d in ds {
            let mut from = d.message.from.name.clone();
            if from.is_empty() {
                from = self.name(&d.message.from.key);
            }
            if !names.contains(&from) {
                names.push(from);
            }
        }
        // Agents' messages are DMs in Talk (card #390): one sender's open
        // their newest message (its DM); several, Talk's list, the unread
        // ones marked (card #482).
        let senders: HashSet<&str> = ds.iter().map(|d| d.message.from.key.as_str()).collect();
        let open = match (senders.len(), ds.iter().max_by(|a, b| a.id.cmp(&b.id))) {
            (1, Some(newest)) => CardAct::Open(Target {
                tab: TAB_INBOX.into(),
                ref_: newest.id.clone(),
            }),
            _ => CardAct::Open(Target {
                tab: TAB_TALK.into(),
                ref_: String::new(),
            }),
        };
        let body = match names.len() {
            1 => format!("From {}", names[0]),
            2 => format!("From {} and {}", names[0], names[1]),
            3 => format!("From {}, {} and {}", names[0], names[1], names[2]),
            n => format!("From {}, {} and {} others", names[0], names[1], n - 2),
        };
        Note {
            id: WAITING_ID.into(),
            title: format!("{} messages waited for you", ds.len()),
            body,
            level: Level::Info,
            silent: true,
            actions: vec![Action {
                label: "Open".into(),
                run: Some(open.clone()),
            }],
            on_click: Some(open),
            ids: ds.iter().map(|d| d.id.clone()).collect(),
            ..Note::default()
        }
    }

    fn message_note(&self, d: &Delivery) -> Note {
        let mut from = d.message.from.name.clone();
        if from.is_empty() {
            from = self.name(&d.message.from.key);
        }
        let open = CardAct::Open(Target {
            tab: TAB_INBOX.into(),
            ref_: d.id.clone(),
        });
        Note {
            id: message_id(&d.id),
            title: format!("{from} sent you a message"),
            body: preview(&d.message.text),
            level: Level::Info,
            actions: vec![Action {
                label: "Open".into(),
                run: Some(open.clone()),
            }],
            on_click: Some(open),
            ..Note::default()
        }
    }
}
