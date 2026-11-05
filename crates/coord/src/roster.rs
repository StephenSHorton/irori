//! The mesh roster (I:internal/roster, coord.md §2.6, §3.12): this device's
//! live presence plus every other device's agents, folded from their
//! `device.started` / `agent.*` rows.
//!
//! A remote card lives until its device writes `agent.gone` for it (or an
//! `agent.status` gone), or a newer `device.started` from that device. An
//! offline device's cards stay, with `DeviceState` `offline`.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use gocompat::sub::Queue;
use gocompat::time::GoTime;
use irori_identity::owner::device_of;
use store::{Event, Filter, Store, kinds};

use crate::gojson::{decode, nil_vec, omit, omit_zero_time};

/// Another device's liveness, as `AgentCard::device_state` shows it.
pub const CONNECTED: &str = "connected";
pub const RECENT: &str = "recent";
pub const OFFLINE: &str = "offline";

/// The status of a live, idle session whose wakes went unread (card #110):
/// it shows as not answering, gets no new cards or asks, and a card handed
/// to it moves on. Its host sets it; it travels in `agent.status` rows.
pub const NOT_ANSWERING: &str = "unresponsive";

/// How an agent's device reads to other agents (card #372): "online",
/// "recent, seen 3m ago", "offline since 2026-11-03T14:02:00Z (2h ago)",
/// "offline, never heard from here". A card with no state is this
/// device's own: "online, this device". `via` names the device its rows
/// come through (card #544): "reachable through Phone, seen 5s ago".
pub fn device_phrase(state: &str, seen: &GoTime, via: &str, now: &GoTime) -> String {
    let ago = || {
        let s = (crate::gotime::since(now, seen) / 1_000_000_000).max(0) as i64;
        match s {
            0..10 => "just now".to_string(),
            10..60 => format!("{s}s ago"),
            60..3600 => format!("{}m ago", s / 60),
            3600..172_800 => format!("{}h ago", s / 3600),
            _ => format!("{}d ago", s / 86_400),
        }
    };
    match state {
        "" => "online, this device".into(),
        CONNECTED => "online".into(),
        RECENT if !via.is_empty() => {
            let via: String = via.chars().filter(|c| !c.is_control()).take(40).collect();
            format!("reachable through {via}, seen {}", ago())
        }
        RECENT if seen.is_zero() => "recent".into(),
        RECENT => format!("recent, seen {}", ago()),
        OFFLINE if seen.is_zero() => "offline, never heard from here".into(),
        OFFLINE => format!(
            "offline since {} ({})",
            (*seen).with_offset(0).format_rfc3339(),
            ago()
        ),
        other => other.chars().filter(|c| !c.is_control()).take(40).collect(),
    }
}

/// Whether an agent with roster status `s` is there to answer: not gone
/// and not [`NOT_ANSWERING`].
pub fn answers(s: &str) -> bool {
    s != "gone" && s != NOT_ANSWERING
}

const KINDS: [&str; 4] = [
    kinds::DEVICE_STARTED,
    kinds::AGENT_SEEN,
    kinds::AGENT_STATUS,
    kinds::AGENT_GONE,
];

crate::go_struct! {
    /// What the roster shows for one session (Go `presence.AgentCard`, the
    /// `agent.seen` payload). Field order and omit rules are Go's.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct AgentCard {
        pub key: String = "key",
        pub device_id: String = "deviceId",
        pub agent: String = "agent",
        pub session_id: String = "sessionId",
        pub pid: i64 = "pid",
        pub name: String = "name",
        pub title: String = "title",
        pub name_source: String = "nameSource" "omit",
        pub ai_title: String = "aiTitle" "omit",
        pub cwd: String = "cwd",
        pub status: String = "status",
        pub session_kind: String = "sessionKind",
        pub entrypoint: String = "entrypoint",
        pub version: String = "version",
        #[serde(serialize_with = "nil_vec")]
        pub peer_features: Vec<String> = "peerFeatures",
        pub started_at: GoTime = "startedAt",
        pub updated_at: GoTime = "updatedAt",
        pub last_activity: GoTime = "lastActivity",
        pub last_text: String = "lastText",
        pub transcript: String = "transcript",
        pub repo_key: String = "repoKey",
        pub permission_mode: String = "permissionMode",
        pub last_tool: String = "lastTool" "omit",
        pub device_state: String = "deviceState" "omit",
        /// While `status` is "waiting", what for ("permission prompt"),
        /// from Claude Code's session file (card #445).
        pub waiting_for: String = "waitingFor" "omit",
        /// When its device was last heard from (card #372): set beside
        /// `device_state` on read, never written.
        pub device_seen: GoTime = "deviceSeen" "omit_zero_time",
        /// The name of the device its rows come through while there is no
        /// session with it (card #544): set on read, never written.
        pub device_via: String = "deviceVia" "omit",
        /// When the current status began (presence's `status_since`), so
        /// the window can say "running a job · 12 min". Rust-only and
        /// omitted when zero, so Go's JSON still holds (card #489).
        pub status_since: GoTime = "statusSince" "omit_zero_time",
    }
}

/// Anything that lists agents: the roster coord and inbox read
/// (Go `interface{ Agents() []presence.AgentCard }`).
pub trait Agents: Send + Sync {
    fn agents(&self) -> Vec<AgentCard>;
    /// The name `key` goes by, or `""`: its card's, or one remembered after
    /// it went (card #98; the Core keeps those).
    fn name_of(&self, key: &str) -> String {
        self.agents()
            .into_iter()
            .find(|a| a.key == key)
            .map(|a| a.name)
            .unwrap_or_default()
    }
}

type LocalFn = Arc<dyn Fn() -> Vec<AgentCard> + Send + Sync>;
type DeviceFn<T> = Arc<dyn Fn(&str) -> T + Send + Sync>;

/// Configures a [`Roster`].
#[derive(Clone)]
pub struct Options {
    pub store: Arc<Store>,
    /// This device: its rows are presence's, not folded here.
    pub self_id: String,
    /// This device's live roster (presence plus Grok members).
    pub local: Option<LocalFn>,
    /// Another device's state (mesh `Node::liveness`); `None` counts every
    /// other device offline.
    pub liveness: Option<DeviceFn<String>>,
    /// When another device was last heard from (card #372); `None`, or a
    /// zero time, for never.
    pub seen: Option<DeviceFn<GoTime>>,
    /// The name of the device another one's rows come through (card #544),
    /// "" when it needs none; `None` for nobody.
    pub via: Option<DeviceFn<String>>,
    /// A removed device: its cards are left out. `None` revokes nobody.
    pub revoked: Option<DeviceFn<bool>>,
}

#[derive(Default)]
struct State {
    cards: HashMap<String, AgentCard>,
    last: HashMap<String, i64>,
    ignored: usize,
    ignored_by: BTreeMap<String, usize>,
}

/// The mesh roster. Safe for concurrent use.
pub struct Roster {
    o: Options,
    st: Mutex<State>,
    subs: Mutex<Vec<Weak<Queue<()>>>>,
}

impl Roster {
    /// Replays every other device's agent rows. [`Roster::follow`] keeps
    /// folding new ones.
    pub fn new(o: Options) -> store::Result<Arc<Roster>> {
        let r = Arc::new(Roster {
            o,
            st: Mutex::new(State::default()),
            subs: Mutex::new(Vec::new()),
        });
        r.rebuild()?;
        Ok(r)
    }

    /// [`Roster::new`] then [`Roster::follow`] (Go `Start`): the store
    /// subscription is taken before the replay so nothing falls between.
    pub fn start(o: Options) -> store::Result<Arc<Roster>> {
        let sub = o.store.subscribe();
        let r = Roster::new(o)?;
        r.follow_with(sub);
        Ok(r)
    }

    /// Folds the store's new rows on a thread until the store closes or the
    /// roster is dropped.
    pub fn follow(self: &Arc<Self>) {
        let sub = self.o.store.subscribe();
        self.follow_with(sub);
    }

    fn follow_with(self: &Arc<Self>, sub: store::Subscription) {
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || {
            while let Some(ev) = sub.recv() {
                let Some(r) = weak.upgrade() else { return };
                r.handle(&ev);
            }
        });
    }

    /// One row from a store subscription (the follow loop's body): a resync
    /// signal rebuilds, another device's agent row folds.
    pub fn handle(&self, ev: &Event) {
        if ev.is_resync() {
            if self.rebuild().is_ok() {
                self.notify();
            }
            return;
        }
        if !KINDS.contains(&ev.kind.as_str()) || ev.device_id == self.o.self_id {
            return;
        }
        let changed = apply(&mut self.lock(), ev);
        if changed {
            self.notify();
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Folds the log from the start.
    pub fn rebuild(&self) -> store::Result<()> {
        let mut st = self.lock();
        *st = State::default();
        let mut f = Filter {
            kinds: KINDS.iter().map(|k| k.to_string()).collect(),
            limit: store::MAX_LIST_LIMIT,
            ..Filter::default()
        };
        loop {
            let evs = self.o.store.list(&f)?;
            for ev in &evs {
                if ev.device_id != self.o.self_id {
                    apply(&mut st, ev);
                }
            }
            if (evs.len() as i64) < f.limit {
                return Ok(());
            }
            f.after = evs[evs.len() - 1].cursor();
        }
    }

    /// This device's live cards, then every other device's by key, each
    /// with its device's liveness; a removed device's cards are left out.
    pub fn agents(&self) -> Vec<AgentCard> {
        let mut out = match &self.o.local {
            Some(l) => l(),
            None => Vec::new(),
        };
        out.extend(self.remote());
        out
    }

    /// The other devices' cards alone, with liveness.
    pub fn remote(&self) -> Vec<AgentCard> {
        let mut remote: Vec<AgentCard> = self.lock().cards.values().cloned().collect();
        remote.sort_by(|a, b| a.key.cmp(&b.key));
        let mut state: HashMap<String, (String, GoTime, String)> = HashMap::new();
        let mut out = Vec::with_capacity(remote.len());
        for mut c in remote {
            if let Some(rev) = &self.o.revoked
                && rev(&c.device_id)
            {
                continue;
            }
            let s = state
                .entry(c.device_id.clone())
                .or_insert_with(|| {
                    let st = match &self.o.liveness {
                        Some(l) => l(&c.device_id),
                        None => OFFLINE.to_string(),
                    };
                    let seen = match &self.o.seen {
                        Some(f) => f(&c.device_id),
                        None => GoTime::default(),
                    };
                    let via = match &self.o.via {
                        Some(f) => f(&c.device_id),
                        None => String::new(),
                    };
                    (st, seen, via)
                })
                .clone();
            (c.device_state, c.device_seen, c.device_via) = s;
            out.push(c);
        }
        out
    }

    /// Agent rows written by a device other than the agent's host.
    pub fn ignored(&self) -> usize {
        self.lock().ignored
    }

    /// The ignored rows per writing device.
    pub fn ignored_by(&self) -> BTreeMap<String, usize> {
        self.lock().ignored_by.clone()
    }

    /// Signals (coalesced, capacity 1) whenever another device's cards
    /// change. Dropping the queue unsubscribes.
    pub fn subscribe(&self) -> Arc<Queue<()>> {
        let q = Arc::new(Queue::new(1));
        let mut subs = self.subs.lock().unwrap_or_else(|e| e.into_inner());
        subs.retain(|w| w.strong_count() > 0);
        subs.push(Arc::downgrade(&q));
        q
    }

    fn notify(&self) {
        let mut subs = self.subs.lock().unwrap_or_else(|e| e.into_inner());
        subs.retain(|w| match w.upgrade() {
            Some(q) => {
                q.push(());
                true
            }
            None => false,
        });
    }
}

impl Agents for Roster {
    fn agents(&self) -> Vec<AgentCard> {
        Roster::agents(self)
    }
}

/// A fixed list of cards (tests, and callers with no mesh).
impl Agents for Vec<AgentCard> {
    fn agents(&self) -> Vec<AgentCard> {
        self.clone()
    }
}

fn ignore(st: &mut State, dev: &str) {
    st.ignored += 1;
    *st.ignored_by.entry(dev.to_string()).or_default() += 1;
}

crate::go_struct! {
    #[derive(Debug, Clone, Default)]
    struct StatusRow {
        key: String = "key",
        status: String = "status",
        name: String = "name",
        permission_mode: String = "permissionMode",
        name_source: String = "nameSource",
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default)]
    struct GoneRow {
        key: String = "key",
    }
}

/// Folds one row of another device. Each device's rows arrive in seq
/// order, so a row at or below the last folded is a repeat.
fn apply(st: &mut State, ev: &Event) -> bool {
    let dev = ev.device_id.as_str();
    if ev.seq <= st.last.get(dev).copied().unwrap_or(0) {
        return false;
    }
    st.last.insert(dev.to_string(), ev.seq);
    match ev.kind.as_str() {
        kinds::DEVICE_STARTED => {
            let before = st.cards.len();
            st.cards.retain(|_, c| c.device_id != dev);
            st.cards.len() != before
        }
        kinds::AGENT_SEEN => {
            let Some(mut c) = decode::<AgentCard>(&ev.payload) else {
                return false;
            };
            if c.key.is_empty() {
                return false;
            }
            if device_of(&c.key) != dev {
                ignore(st, dev);
                return false;
            }
            c.device_id = dev.to_string();
            c.last_text.clear();
            c.last_tool.clear();
            c.transcript.clear();
            c.pid = 0;
            if c.status.is_empty() || c.status == "gone" {
                c.status = "idle".into();
            }
            st.cards.insert(c.key.clone(), c);
            true
        }
        kinds::AGENT_STATUS => {
            let Some(p) = decode::<StatusRow>(&ev.payload) else {
                return false;
            };
            if !st.cards.contains_key(&p.key) || device_of(&p.key) != dev {
                return false;
            }
            if p.status == "gone" {
                st.cards.remove(&p.key);
                return true;
            }
            let c = st.cards.get_mut(&p.key).expect("checked above");
            c.status = p.status;
            c.name = p.name;
            c.name_source = p.name_source;
            if !p.permission_mode.is_empty() {
                c.permission_mode = p.permission_mode;
            }
            true
        }
        kinds::AGENT_GONE => {
            let Some(p) = decode::<GoneRow>(&ev.payload) else {
                return false;
            };
            if device_of(&p.key) != dev {
                return false;
            }
            st.cards.remove(&p.key).is_some()
        }
        _ => false,
    }
}
