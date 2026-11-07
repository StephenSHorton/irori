//! Friendly names for sessions nobody named (I:internal/core/names.go,
//! surface.md §2.1, card #21). Device-local: only the device that hosts a
//! session names it, and the name reaches other devices on its card.
//!
//! `names.json` is a JSON object keyed by session id →
//! `{"name", "at", "gave"?}`, written through `names.json.tmp`, 0600.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::Duration;

use coord::names::{fold, pick, pick_free, pick_free_in};
use coord::roster::AgentCard;
use gocompat::json::lenient::{self, Out, Schema};
use gocompat::time::GoTime;
use serde::Serialize;

use crate::reset::write_private;

pub const NAMES_FILE: &str = "names.json";
/// The names.json key prefix of a word held for an agent Irori is
/// starting (card #472), not a session id.
pub const SPAWN_PREFIX: &str = "spawn:";
/// Another person's agent whose device hasn't been seen this long no longer
/// holds its word against a new pick (it still counts as used): a friend
/// gone for good doesn't keep a word forever.
pub const OTHERS_STALE: Duration = Duration::from_secs(7 * 24 * 3600);
/// A name stays reserved this long for a session that isn't running.
pub const NAMES_KEEP: Duration = Duration::from_secs(30 * 24 * 3600);
/// A session whose name was in use this recently holds it against new picks.
pub const NAMES_BUSY: Duration = Duration::from_secs(2 * 3600);
/// A new session never gets a name any agent went by this recently (card
/// #224), while the list has a word left that isn't: the same name the
/// next day reads as the same agent, and a gone agent's name still sits
/// on its cards as holder or commenter.
pub const NAMES_RECENT: Duration = Duration::from_secs(7 * 24 * 3600);

/// One `names.json` entry (Go field order).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NameEntry {
    pub name: String,
    /// When it was given, or last seen in use.
    pub at: GoTime,
    /// Names this session had before a clash moved it (at most 8).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub gave: Vec<String>,
}

type CardsFn = Arc<dyn Fn() -> Vec<AgentCard> + Send + Sync>;
/// Every agent name remembered lately, any device: (key, name, last seen).
type SeenFn = Arc<dyn Fn() -> Vec<(String, String, GoTime)> + Send + Sync>;
type NowFn = Arc<dyn Fn() -> GoTime + Send + Sync>;
type ThemesFn = Arc<dyn Fn() -> Vec<String> + Send + Sync>;

/// Gives sessions a name ([`presence::Namer`]) and settles clashes with
/// other devices' agents.
pub struct Namer {
    path: PathBuf,
    self_id: String,
    now: RwLock<NowFn>,
    /// Every agent on every device (Core's mesh roster), set once it exists.
    mesh: RwLock<Option<CardsFn>>,
    /// Other people's agents on the shared boards this device holds (card
    /// #472): a new name never takes a word a friend's live agent has. Not
    /// used to move names ([`Namer::settle`]): only this person's roster is.
    others: RwLock<Option<CardsFn>>,
    /// The agent names remembered on this device (agent-names.json).
    seen: RwLock<Option<SeenFn>>,
    /// The themes the person turned off (card #702): `coord::names::THEMES`
    /// ids; none means every theme is on.
    themes_off: RwLock<Option<ThemesFn>>,
    m: Mutex<BTreeMap<String, NameEntry>>,
}

impl Namer {
    /// Loads `<data_dir>/names.json`; an unreadable file starts over.
    pub fn new(data_dir: &std::path::Path, self_id: &str) -> Namer {
        let path = data_dir.join(NAMES_FILE);
        let m = match std::fs::read(&path) {
            Ok(raw) => parse(&raw).unwrap_or_else(|e| {
                crate::logf(format_args!(
                    "names: {} unreadable, starting over: {e}",
                    path.display()
                ));
                BTreeMap::new()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => {
                crate::logf(format_args!("names: {e}"));
                BTreeMap::new()
            }
        };
        Namer {
            path,
            self_id: self_id.to_string(),
            now: RwLock::new(Arc::new(presence::now_local)),
            mesh: RwLock::new(None),
            others: RwLock::new(None),
            seen: RwLock::new(None),
            themes_off: RwLock::new(None),
            m: Mutex::new(m),
        }
    }

    /// Sets the mesh roster the names are checked against.
    pub fn set_mesh(&self, f: impl Fn() -> Vec<AgentCard> + Send + Sync + 'static) {
        *self.mesh.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(f));
    }

    /// Sets where other people's agents on shared boards come from (card
    /// #472): their live names are taken for a new pick.
    pub fn set_others(&self, f: impl Fn() -> Vec<AgentCard> + Send + Sync + 'static) {
        *self.others.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(f));
    }

    /// Sets where the names agents went by lately come from: (agent key,
    /// name, when last seen with it), e.g. agent-names.json.
    pub fn set_seen(&self, f: impl Fn() -> Vec<(String, String, GoTime)> + Send + Sync + 'static) {
        *self.seen.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(f));
    }

    /// Sets where the themes the person turned off come from (card #702):
    /// new names come from the other themes' words while one is free.
    pub fn set_themes_off(&self, f: impl Fn() -> Vec<String> + Send + Sync + 'static) {
        *self.themes_off.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(f));
    }

    /// Replaces the clock (tests).
    pub fn set_now(&self, f: impl Fn() -> GoTime + Send + Sync + 'static) {
        *self.now.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(f);
    }

    /// Sets one entry as is (tests reach Go's `n.m` this way).
    #[doc(hidden)]
    pub fn set_entry(&self, session_id: &str, name: &str, at: GoTime) {
        self.lock().insert(
            session_id.into(),
            NameEntry {
                name: name.into(),
                at,
                gave: vec![],
            },
        );
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, NameEntry>> {
        self.m.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn now(&self) -> GoTime {
        (self.now.read().unwrap_or_else(|e| e.into_inner()))()
    }

    fn seen(&self) -> Vec<(String, String, GoTime)> {
        let f = self.seen.read().unwrap_or_else(|e| e.into_inner()).clone();
        f.map(|f| f()).unwrap_or_default()
    }

    /// The words of the themes that are on; `None` for the full list (every
    /// theme on, or every one off).
    fn words_on(&self) -> Option<Vec<&'static str>> {
        let f = self
            .themes_off
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        coord::names::words_on(&f.map(|f| f()).unwrap_or_default())
    }

    fn cards(&self) -> Vec<AgentCard> {
        let f = self.mesh.read().unwrap_or_else(|e| e.into_inner()).clone();
        f.map(|f| f()).unwrap_or_default()
    }

    fn others(&self) -> Vec<AgentCard> {
        let f = self
            .others
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        f.map(|f| f()).unwrap_or_default()
    }

    /// A fresh name for an agent Irori is about to start (card #472), from
    /// `seed` (random per start, so a card's next agent gets another word):
    /// one no live agent of any member has, nobody here used lately, and
    /// `taken` doesn't refuse (names other starts already asked for). It is
    /// held in names.json under `spawn:<seed>` for [`NAMES_BUSY`], so a
    /// terminal session named while the agent starts never gets it; the
    /// agent itself keeps it as its own (`--name`) name.
    pub fn pick_new(&self, seed: &str, taken: &dyn Fn(&str) -> bool) -> String {
        let (cards, others) = (self.cards(), self.others());
        let now = self.now();
        let mut m = self.lock();
        let name = self.pick_with(&m, seed, &cards, &others, &now, taken);
        m.insert(
            format!("{SPAWN_PREFIX}{seed}"),
            NameEntry {
                name: name.clone(),
                at: now,
                gave: vec![],
            },
        );
        self.save_locked(&m);
        name
    }

    /// The name session `id` was given, or `""`.
    pub fn assigned(&self, id: &str) -> String {
        self.lock()
            .get(id)
            .map(|e| e.name.clone())
            .unwrap_or_default()
    }

    /// Whether Irori ever gave session `id` this name.
    pub fn gave(&self, id: &str, name: &str) -> bool {
        let m = self.lock();
        let Some(e) = m.get(id) else { return false };
        let f = fold(name);
        fold(&e.name) == f || e.gave.iter().any(|g| fold(g) == f)
    }

    /// Gives session `id` a name if it has none: the word its id hashes to,
    /// or the next one no live agent on any device is called.
    pub fn assign(&self, id: &str) -> String {
        // Presence asks on every scan: a named session costs one lookup,
        // not a read of every shared board (card #472).
        let had = self.assigned(id);
        if !had.is_empty() {
            return had;
        }
        let (cards, others) = (self.cards(), self.others());
        let now = self.now();
        let mut m = self.lock();
        if let Some(e) = m.get(id) {
            return e.name.clone();
        }
        let name = self.pick_locked(&m, id, &cards, &others, &now);
        m.insert(
            id.into(),
            NameEntry {
                name: name.clone(),
                at: now,
                gave: vec![],
            },
        );
        self.save_locked(&m);
        name
    }

    /// A name for `id` that no live agent but `id` has and no other session
    /// here used lately. Among those, it prefers a word nobody went by in
    /// the remembered past (30 days: names.json, agent-names.json, gone
    /// agents still on the roster), then one nobody went by in
    /// [`NAMES_RECENT`], and only then any free word or a numbered name.
    /// With themes turned off (card #702) it first tries the words of the
    /// themes still on the same two ways, then the full list.
    fn pick_locked(
        &self,
        m: &BTreeMap<String, NameEntry>,
        id: &str,
        cards: &[AgentCard],
        others: &[AgentCard],
        now: &GoTime,
    ) -> String {
        self.pick_with(m, id, cards, others, now, &|_| false)
    }

    /// [`Self::pick_locked`], with `extra` refusing more names. `others`
    /// (other people's agents on shared boards, read before names.json is
    /// locked) are taken while live, so a friend's kestrel keeps the only
    /// kestrel (card #472).
    fn pick_with(
        &self,
        m: &BTreeMap<String, NameEntry>,
        id: &str,
        cards: &[AgentCard],
        others: &[AgentCard],
        now: &GoTime,
        extra: &dyn Fn(&str) -> bool,
    ) -> String {
        let key = format!("{}/{id}", self.self_id);
        let mut taken = HashSet::new();
        let mut recent = HashSet::new();
        let mut known = HashSet::new();
        for c in others {
            if c.name.is_empty() {
                continue;
            }
            let stale = c.device_state == coord::roster::OFFLINE
                && (c.device_seen.is_zero() || since(now, &c.device_seen) > nanos(OTHERS_STALE));
            if c.status != "gone" && !stale {
                taken.insert(fold(&c.name));
            } else {
                known.insert(fold(&c.name));
            }
        }
        for c in cards {
            if c.key == key {
                continue;
            }
            if c.status != "gone" {
                taken.insert(fold(&c.name));
            } else {
                known.insert(fold(&c.name));
            }
        }
        for (sid, e) in m {
            if sid == id {
                continue;
            }
            let age = since(now, &e.at);
            if age < nanos(NAMES_BUSY) {
                taken.insert(fold(&e.name));
            } else if age < nanos(NAMES_RECENT) {
                recent.insert(fold(&e.name));
            } else {
                known.insert(fold(&e.name));
            }
        }
        for (k, name, at) in self.seen() {
            if k == key {
                continue;
            }
            // agent-names.json moves `at` at most daily: a day's slack.
            if since(now, &at) < nanos(NAMES_RECENT + Duration::from_secs(24 * 3600)) {
                recent.insert(fold(&name));
            } else {
                known.insert(fold(&name));
            }
        }
        let taken = |s: &str| taken.contains(s) || extra(s);
        let fresh = |s: &str| taken(s) || recent.contains(s) || known.contains(s);
        let not_recent = |s: &str| taken(s) || recent.contains(s);
        if let Some(words) = self.words_on() {
            if let Some(n) = pick_free_in(&words, id, &fresh) {
                return n;
            }
            if let Some(n) = pick_free_in(&words, id, &not_recent) {
                return n;
            }
        }
        if let Some(n) = pick_free(id, &fresh) {
            return n;
        }
        if let Some(n) = pick_free(id, &not_recent) {
            return n;
        }
        pick(id, Some(&taken))
    }

    /// Gives a new name to each session of this device whose Irori name
    /// another live agent also has, when that agent keeps it, and forgets
    /// names of sessions not seen for [`NAMES_KEEP`]. Whether a name moved.
    pub fn settle(&self) -> bool {
        let (cards, others) = (self.cards(), self.others());
        let now = self.now();
        let mut m = self.lock();
        let (mut moved, mut dirty) = (false, false);
        let mut local = HashSet::new();
        for c in &cards {
            if c.device_id != self.self_id || c.agent != "claude" || c.status == "gone" {
                continue;
            }
            local.insert(c.session_id.clone());
            let Some(mut e) = m.get(&c.session_id).cloned() else {
                continue;
            };
            if since(&now, &e.at) > nanos(Duration::from_secs(3600)) {
                e.at = now;
                m.insert(c.session_id.clone(), e.clone());
                dirty = true;
            }
            if c.name_source != presence::SOURCE_IRORI || fold(&c.name) != fold(&e.name) {
                continue;
            }
            for o in &cards {
                if o.key == c.key || o.status == "gone" || fold(&o.name) != fold(&c.name) {
                    continue;
                }
                if !keeps(o, c) {
                    continue;
                }
                let mut gave = e.gave.clone();
                gave.push(e.name.clone());
                if gave.len() > 8 {
                    gave.drain(..gave.len() - 8);
                }
                let name = self.pick_locked(&m, &c.session_id, &cards, &others, &now);
                m.insert(
                    c.session_id.clone(),
                    NameEntry {
                        name,
                        at: now,
                        gave,
                    },
                );
                moved = true;
                dirty = true;
                break;
            }
        }
        let before = m.len();
        m.retain(|sid, e| local.contains(sid) || since(&now, &e.at) <= nanos(NAMES_KEEP));
        dirty |= m.len() != before;
        if dirty {
            self.save_locked(&m);
        }
        moved
    }

    fn save_locked(&self, m: &BTreeMap<String, NameEntry>) {
        let Ok(raw) = gocompat::json::marshal_indent(m, "", "  ") else {
            return;
        };
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        if let Err(e) = write_private(&tmp, &raw) {
            crate::logf(format_args!("names: {e}"));
            return;
        }
        if let Err(e) = std::fs::rename(&tmp, &self.path) {
            crate::logf(format_args!("names: {e}"));
        }
    }
}

impl presence::Namer for Namer {
    fn assigned(&self, session_id: &str) -> String {
        Namer::assigned(self, session_id)
    }
    fn gave(&self, session_id: &str, name: &str) -> bool {
        Namer::gave(self, session_id, name)
    }
    fn assign(&self, session_id: &str) -> String {
        Namer::assign(self, session_id)
    }
}

/// Whether agent `a` keeps a name it shares with `b` (an Irori name): a
/// name someone chose wins, then the earlier start, then the lower key.
fn keeps(a: &AgentCard, b: &AgentCard) -> bool {
    let (ai, bi) = (
        a.name_source == presence::SOURCE_IRORI,
        b.name_source == presence::SOURCE_IRORI,
    );
    if ai != bi {
        return !ai;
    }
    if !a.started_at.equal(&b.started_at) {
        return a.started_at.unix_nano() < b.started_at.unix_nano();
    }
    a.key < b.key
}

/// Go `a.Sub(b)` in nanoseconds.
fn since(a: &GoTime, b: &GoTime) -> i128 {
    (i128::from(a.secs) - i128::from(b.secs)) * 1_000_000_000 + i128::from(a.nanos)
        - i128::from(b.nanos)
}

fn nanos(d: Duration) -> i128 {
    d.as_nanos() as i128
}

fn parse(raw: &[u8]) -> Result<BTreeMap<String, NameEntry>, String> {
    let schema = Schema::Map(Box::new(Schema::object(vec![
        ("name", Schema::String),
        ("at", Schema::Time),
        ("gave", Schema::Slice(Box::new(Schema::String))),
    ])));
    let v = lenient::unmarshal(raw, &schema).map_err(|e| e.to_string())?;
    let mut out = BTreeMap::new();
    if let Out::Map(Some(m)) = v {
        for (k, e) in m {
            out.insert(
                k,
                NameEntry {
                    name: e.get_str("name").to_string(),
                    at: match e.get("at") {
                        Some(Out::Time(t)) => *t,
                        _ => GoTime::ZERO,
                    },
                    gave: e
                        .get("gave")
                        .map(|g| g.items().iter().map(|s| s.str().to_string()).collect())
                        .unwrap_or_default(),
                },
            );
        }
    }
    Ok(out)
}
