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

use coord::names::{fold, pick};
use coord::roster::AgentCard;
use gocompat::json::lenient::{self, Out, Schema};
use gocompat::time::GoTime;
use serde::Serialize;

use crate::reset::write_private;

pub const NAMES_FILE: &str = "names.json";
/// A name stays reserved this long for a session that isn't running.
pub const NAMES_KEEP: Duration = Duration::from_secs(30 * 24 * 3600);
/// A session whose name was in use this recently holds it against new picks.
pub const NAMES_BUSY: Duration = Duration::from_secs(2 * 3600);

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
type NowFn = Arc<dyn Fn() -> GoTime + Send + Sync>;

/// Gives sessions a name ([`presence::Namer`]) and settles clashes with
/// other devices' agents.
pub struct Namer {
    path: PathBuf,
    self_id: String,
    now: RwLock<NowFn>,
    /// Every agent on every device (Core's mesh roster), set once it exists.
    mesh: RwLock<Option<CardsFn>>,
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
            m: Mutex::new(m),
        }
    }

    /// Sets the mesh roster the names are checked against.
    pub fn set_mesh(&self, f: impl Fn() -> Vec<AgentCard> + Send + Sync + 'static) {
        *self.mesh.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(f));
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

    fn cards(&self) -> Vec<AgentCard> {
        let f = self.mesh.read().unwrap_or_else(|e| e.into_inner()).clone();
        f.map(|f| f()).unwrap_or_default()
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
        let cards = self.cards();
        let now = self.now();
        let mut m = self.lock();
        if let Some(e) = m.get(id) {
            return e.name.clone();
        }
        let name = self.pick_locked(&m, id, &cards, &now);
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
    /// here used lately.
    fn pick_locked(
        &self,
        m: &BTreeMap<String, NameEntry>,
        id: &str,
        cards: &[AgentCard],
        now: &GoTime,
    ) -> String {
        let key = format!("{}/{id}", self.self_id);
        let mut taken = HashSet::new();
        for c in cards {
            if c.key != key && c.status != "gone" {
                taken.insert(fold(&c.name));
            }
        }
        for (sid, e) in m {
            if sid != id && since(now, &e.at) < nanos(NAMES_BUSY) {
                taken.insert(fold(&e.name));
            }
        }
        pick(id, Some(&|s: &str| taken.contains(s)))
    }

    /// Gives a new name to each session of this device whose Irori name
    /// another live agent also has, when that agent keeps it, and forgets
    /// names of sessions not seen for [`NAMES_KEEP`]. Whether a name moved.
    pub fn settle(&self) -> bool {
        let cards = self.cards();
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
                let name = self.pick_locked(&m, &c.session_id, &cards, &now);
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
