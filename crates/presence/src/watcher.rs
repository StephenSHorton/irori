//! Go `watcher.go`: the roster state, scans and change publishing.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, SystemTime};

use gocompat::sub::{Queue, TryRecv};
use gocompat::time::GoTime;

use crate::localtime::now_local;
use crate::session::{Session, is_session_file, read_session};
use crate::transcript::{
    DEFAULT_MODE_SCAN_BYTES, DEFAULT_TAIL_BYTES, Tail, find_transcript, instant, read_tail,
    scan_mode,
};
use crate::{
    AgentCard, Change, ChangeKind, ExitFn, LiveFn, OpenFn, Options, SOURCE_IRORI, SOURCE_USER,
};

const DEFAULT_INTERVAL: Duration = Duration::from_secs(2);
const DEFAULT_SAFETY_INTERVAL: Duration = Duration::from_secs(30);
/// Gathers a burst of file events (a streaming reply appends many rows)
/// into one scan.
const DEFAULT_SETTLE: Duration = Duration::from_millis(100);
/// Each subscriber's buffer; a slow reader drops changes.
const SUB_BUFFER: usize = 64;

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// One transcript, re-read only when it moves.
struct TailCache {
    path: String,
    size: u64,
    mtime: Option<SystemTime>,
    tail: Tail,
    /// The bounded backward scan for a permission mode has run on this
    /// file, so a file with no mode row isn't rescanned on every change.
    mode_scanned: bool,
}

/// Scan-owned state (Go: guarded by scanMu).
#[derive(Default)]
struct ScanState {
    tails: HashMap<String, TailCache>,
    sessions: HashMap<String, Session>,
    repos: HashMap<String, String>,
}

/// Published cards and subscribers (Go: guarded by mu).
#[derive(Default)]
struct PubState {
    cards: HashMap<String, AgentCard>,
    subs: BTreeMap<u64, Arc<Queue<Change>>>,
    next: u64,
    closed: bool,
}

pub(crate) struct Inner {
    pub(crate) opts: Options,
    pub(crate) live: LiveFn,
    pub(crate) open: OpenFn,
    pub(crate) watch_exit: Option<ExitFn>,
    pub(crate) interval: Duration,
    pub(crate) safety_interval: Duration,
    pub(crate) settle: Duration,
    tail_bytes: u64,
    mode_scan_bytes: u64,
    scan: Mutex<ScanState>,
    pubs: Mutex<PubState>,
}

/// Keeps the roster current (Go `Watcher`). Cheap to clone; every clone is
/// the same watcher. Safe for concurrent use.
#[derive(Clone)]
pub struct Watcher {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher")
            .field("opts", &self.inner.opts)
            .finish_non_exhaustive()
    }
}

fn nonzero(d: Duration, def: Duration) -> Duration {
    if d.is_zero() { def } else { d }
}

impl Watcher {
    /// Go `New`: nothing runs until [`Watcher::run`].
    pub fn new(opts: Options) -> Watcher {
        let live = opts.live.clone().unwrap_or_else(crate::os::default_live);
        let open: OpenFn = opts
            .open
            .clone()
            .unwrap_or_else(|| Arc::new(|p: &Path| std::fs::File::open(p)));
        let watch_exit = opts
            .watch_exit
            .clone()
            .or_else(crate::os::default_watch_exit);
        Watcher {
            inner: Arc::new(Inner {
                interval: nonzero(opts.interval, DEFAULT_INTERVAL),
                safety_interval: nonzero(opts.safety_interval, DEFAULT_SAFETY_INTERVAL),
                settle: nonzero(opts.settle, DEFAULT_SETTLE),
                tail_bytes: if opts.tail_bytes == 0 {
                    DEFAULT_TAIL_BYTES
                } else {
                    opts.tail_bytes
                },
                mode_scan_bytes: if opts.mode_scan_bytes == 0 {
                    DEFAULT_MODE_SCAN_BYTES
                } else {
                    opts.mode_scan_bytes
                },
                opts,
                live,
                open,
                watch_exit,
                scan: Mutex::default(),
                pubs: Mutex::default(),
            }),
        }
    }

    /// Go `Snapshot`: the live cards, newest activity first.
    pub fn snapshot(&self) -> Vec<AgentCard> {
        let mut out: Vec<AgentCard> = lock(&self.inner.pubs).cards.values().cloned().collect();
        sort_cards(&mut out);
        out
    }

    /// Go `Rescan`: reads every session file again now and publishes what
    /// changed (a namer that moved a name calls it so the card follows).
    pub fn rescan(&self) {
        self.inner.scan();
    }

    /// Go `Subscribe`: changes after the call until the subscription is
    /// cancelled or dropped (buffered; a slow reader drops changes and
    /// should re-read the snapshot).
    pub fn subscribe(&self) -> Subscription {
        let q = Arc::new(Queue::new(SUB_BUFFER));
        let mut p = lock(&self.inner.pubs);
        if p.closed {
            q.close();
        }
        let id = p.next;
        p.next += 1;
        p.subs.insert(id, q.clone());
        Subscription {
            q,
            id,
            inner: Arc::downgrade(&self.inner),
        }
    }

    /// Go's `name()`, for the precedence tests.
    #[doc(hidden)]
    pub fn name_of(&self, s: &Session, t: &Tail) -> (String, String) {
        self.inner.name(s, t)
    }

    /// Whether the bounded mode scan ran on session `id`'s transcript
    /// (Go tests read `tails[id].modeScanned`).
    #[doc(hidden)]
    pub fn mode_scanned(&self, id: &str) -> Option<bool> {
        lock(&self.inner.scan).tails.get(id).map(|t| t.mode_scanned)
    }

    /// Whether a repo key is cached for `cwd` (Go tests read `repos`).
    #[doc(hidden)]
    pub fn repo_cached(&self, cwd: &str) -> bool {
        lock(&self.inner.scan).repos.contains_key(cwd)
    }
}

/// A subscription's stream of changes (Go's `<-chan Change`). `None` from a
/// receive means it ended.
pub struct Subscription {
    q: Arc<Queue<Change>>,
    id: u64,
    inner: Weak<Inner>,
}

impl Subscription {
    /// The next change, waiting on this thread.
    pub fn recv(&self) -> Option<Change> {
        self.q.recv()
    }

    /// The next change, waiting at most `timeout`; `Err(())` on timeout.
    #[allow(clippy::result_unit_err)]
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Option<Change>, ()> {
        self.q.recv_timeout(timeout)
    }

    /// The next change if one is queued.
    pub fn try_recv(&self) -> TryRecv<Change> {
        self.q.try_recv()
    }

    /// The next change, from async code.
    pub async fn recv_async(&self) -> Option<Change> {
        self.q.recv_async().await
    }

    /// End the subscription (Go: its context ends). Queued changes can
    /// still be received; then the stream ends.
    pub fn cancel(&self) {
        if let Some(inner) = self.inner.upgrade() {
            lock(&inner.pubs).subs.remove(&self.id);
        }
        self.q.close();
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Go `names.Fold`.
fn fold(name: &str) -> String {
    gocompat::text::to_lower_str(name.trim_matches(gocompat::text::is_space))
}

impl Inner {
    fn now(&self) -> GoTime {
        match &self.opts.now {
            Some(f) => f(),
            None => now_local(),
        }
    }

    /// Go `scan`: reads every session file once and publishes what changed.
    pub(crate) fn scan(&self) {
        let mut st = lock(&self.scan);
        let mut next: HashMap<String, AgentCard> = HashMap::new();
        let mut seen: HashMap<String, Session> = HashMap::new();
        let mut cwds: HashSet<String> = HashSet::new();
        let dir = gocompat::filepath::join(&[self.opts.claude_dir.as_str(), "sessions"]);
        let mut names: Vec<String> = match std::fs::read_dir(&dir) {
            Ok(rd) => rd
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect(),
            Err(_) => Vec::new(),
        };
        names.sort();
        for name in names {
            if !is_session_file(&name) {
                continue;
            }
            let path = gocompat::filepath::join(&[dir.as_str(), &name]);
            let Some(s) = self.read_session_file(&path) else {
                continue;
            };
            if !(self.live)(s.pid, s.proc_start) {
                continue;
            }
            cwds.insert(s.cwd.clone());
            let c = self.card(&mut st, &s);
            seen.insert(s.id.clone(), s);
            next.insert(c.key.clone(), c);
        }
        st.tails.retain(|id, _| seen.contains_key(id));
        st.sessions = seen;
        st.repos.retain(|cwd, _| cwds.contains(cwd));
        self.publish(next);
    }

    /// Go `rescanTails`: rebuilds only the cards of the given session ids
    /// from their last-read session file, for a transcript change.
    pub(crate) fn rescan_tails(&self, ids: &HashSet<String>) {
        let mut st = lock(&self.scan);
        let live: Vec<Session> = ids
            .iter()
            .filter_map(|id| st.sessions.get(id).cloned())
            .collect();
        if live.is_empty() {
            return;
        }
        let fresh: Vec<AgentCard> = live.iter().map(|s| self.card(&mut st, s)).collect();
        // publish treats a missing key as Removed, so start from every card.
        let mut next = lock(&self.pubs).cards.clone();
        for c in fresh {
            next.insert(c.key.clone(), c);
        }
        self.publish(next);
        drop(st);
    }

    /// Go `livePIDs`.
    pub(crate) fn live_pids(&self) -> HashSet<i64> {
        lock(&self.scan).sessions.values().map(|s| s.pid).collect()
    }

    fn read_session_file(&self, path: &str) -> Option<Session> {
        let f = (self.open)(Path::new(path)).ok()?;
        read_session(f)
    }

    /// Go `card`: the roster entry for a live session, refreshing its tail.
    fn card(&self, st: &mut ScanState, s: &Session) -> AgentCard {
        let mut c = AgentCard {
            key: format!("{}/{}", self.opts.device_id, s.id),
            device_id: self.opts.device_id.clone(),
            agent: "claude".into(),
            session_id: s.id.clone(),
            pid: s.pid,
            cwd: s.cwd.clone(),
            status: s.status.clone(),
            session_kind: s.kind.clone(),
            entrypoint: s.entrypoint.clone(),
            version: s.version.clone(),
            peer_features: s.peer_features.clone(),
            started_at: s.started_at,
            updated_at: s.updated_at,
            repo_key: self.repo_key(st, &s.cwd),
            ..AgentCard::default()
        };
        let mut t = Tail::default();
        if let Some(tc) = self.refresh_tail(st, s) {
            t = tc.tail.clone();
            c.transcript = tc.path.clone();
            c.title = t.title.clone();
            c.last_text = t.last_text.clone();
            c.last_activity = t.last_activity;
            c.permission_mode = t.permission_mode.clone();
        }
        c.ai_title = t.ai.clone();
        (c.name, c.name_source) = self.name(s, &t);
        c
    }

    /// Go `name`: what the roster calls a session, and where that came from.
    fn name(&self, s: &Session, t: &Tail) -> (String, String) {
        let names = self.opts.names.as_deref();
        let assigned = names.map(|n| n.assigned(&s.id)).unwrap_or_default();
        let mut chosen = String::new();
        if !s.name.is_empty() && (s.name_source.is_empty() || s.name_source == SOURCE_USER) {
            chosen = s.name.clone();
        } else if !t.custom.is_empty() {
            chosen = t.custom.clone();
        } else if !s.name.is_empty()
            && !assigned.is_empty()
            && names.is_some_and(|n| n.gave(&s.id, &s.name))
        {
            chosen = s.name.clone();
        }
        if !chosen.is_empty() {
            if !assigned.is_empty() && names.is_some_and(|n| n.gave(&s.id, &chosen)) {
                if fold(&chosen) == fold(&assigned) {
                    return (chosen, SOURCE_IRORI.into());
                }
                return (assigned, SOURCE_IRORI.into());
            }
            return (chosen, SOURCE_USER.into());
        }
        if let Some(n) = names {
            let given = n.assign(&s.id);
            if !given.is_empty() {
                return (given, SOURCE_IRORI.into());
            }
        }
        (display_name(s, &t.title), String::new())
    }

    /// Go `repoKey`: computed once per cwd while any live session uses it.
    fn repo_key(&self, st: &mut ScanState, cwd: &str) -> String {
        let Some(f) = &self.opts.repo_key else {
            return String::new();
        };
        if cwd.is_empty() {
            return String::new();
        }
        if let Some(k) = st.repos.get(cwd) {
            return k.clone();
        }
        let k = f(cwd);
        st.repos.insert(cwd.to_string(), k.clone());
        k
    }

    /// Go `refreshTail`: resolves (once) and re-reads a transcript tail when
    /// its size or mtime changed. None means no transcript was found.
    fn refresh_tail<'a>(&self, st: &'a mut ScanState, s: &Session) -> Option<&'a TailCache> {
        let mut meta = None;
        if let Some(tc) = st.tails.get(&s.id) {
            match std::fs::metadata(&tc.path) {
                Ok(m) => meta = Some(m),
                Err(_) => {
                    st.tails.remove(&s.id);
                }
            }
        }
        if meta.is_none() {
            let p = find_transcript(&self.opts.claude_dir, &s.cwd, &s.id);
            if p.is_empty() {
                st.tails.remove(&s.id);
                return None;
            }
            match std::fs::metadata(&p) {
                Ok(m) => meta = Some(m),
                Err(_) => {
                    st.tails.remove(&s.id);
                    return None;
                }
            }
            st.tails.insert(
                s.id.clone(),
                TailCache {
                    path: p,
                    size: 0,
                    mtime: None,
                    tail: Tail::default(),
                    mode_scanned: false,
                },
            );
        }
        let meta = meta?;
        let tc = st.tails.get_mut(&s.id)?;
        if meta.len() == tc.size && meta.modified().ok() == tc.mtime {
            return Some(tc);
        }
        let Ok(mut f) = (self.open)(Path::new(&tc.path)) else {
            return Some(tc);
        };
        let Ok(fi) = f.metadata() else {
            return Some(tc);
        };
        let size = fi.len();
        let mut t = read_tail(&mut f, size, self.tail_bytes);
        if size >= tc.size {
            // Title rows carry no timestamp and can scroll out of the window.
            let old = &tc.tail;
            if t.title.is_empty() {
                t.title = old.title.clone();
            }
            if t.custom.is_empty() {
                t.custom = old.custom.clone();
            }
            if t.ai.is_empty() {
                t.ai = old.ai.clone();
            }
            if t.last_text.is_empty() {
                t.last_text = old.last_text.clone();
            }
            // Mode rows are written only when the mode is set, so they
            // scroll out like titles do.
            if t.permission_mode.is_empty() {
                t.permission_mode = old.permission_mode.clone();
            }
            if instant(&t.last_activity) < instant(&old.last_activity) {
                t.last_activity = old.last_activity;
            }
        } else {
            tc.mode_scanned = false; // rewritten file: look again
        }
        // Cold start (or a rewrite): the tail holds no mode row and nothing
        // is cached, so look further back once.
        if t.permission_mode.is_empty() && !tc.mode_scanned {
            tc.mode_scanned = true;
            t.permission_mode = scan_mode(&mut f, size, self.mode_scan_bytes);
        }
        tc.size = size;
        tc.mtime = fi.modified().ok();
        tc.tail = t;
        Some(tc)
    }

    /// Go `publish`: swaps in `next` and tells subscribers what changed.
    fn publish(&self, mut next: HashMap<String, AgentCard>) {
        let now = self.now();
        let mut p = lock(&self.pubs);
        let mut changes = Vec::new();
        let mut keys: Vec<String> = next.keys().cloned().collect();
        keys.sort();
        for k in keys {
            let c = &next[&k];
            match p.cards.get(&k) {
                None => changes.push(Change {
                    kind: ChangeKind::Added,
                    card: c.clone(),
                }),
                Some(old) if changed(old, c) => changes.push(Change {
                    kind: ChangeKind::Updated,
                    card: c.clone(),
                }),
                Some(old) => {
                    // Keep the published card so sub-second activity drift
                    // doesn't accumulate unseen.
                    let old = old.clone();
                    next.insert(k, old);
                }
            }
        }
        let mut gone: Vec<&String> = p.cards.keys().filter(|k| !next.contains_key(*k)).collect();
        gone.sort();
        for k in gone {
            let mut old = p.cards[k].clone();
            old.status = "gone".into();
            old.updated_at = now;
            changes.push(Change {
                kind: ChangeKind::Removed,
                card: old,
            });
        }
        p.cards = next;
        for ch in changes {
            for sub in p.subs.values() {
                // A slow reader drops it; it re-reads the snapshot.
                sub.push(ch.clone());
            }
        }
    }
}

/// Go `displayName`: the session's own name, else its title, else its
/// folder, else "claude <id[:8]>".
pub fn display_name(s: &Session, title: &str) -> String {
    if !s.name.is_empty() {
        return s.name.clone();
    }
    if !title.is_empty() {
        return title.to_string();
    }
    // Split on both separators: a Windows cwd may be read on another OS.
    let cwd = s.cwd.trim_end_matches(['/', '\\']);
    let cwd = match cwd.rfind(['/', '\\']) {
        Some(i) => &cwd[i + 1..],
        None => cwd,
    };
    if !cwd.is_empty() {
        return cwd.to_string();
    }
    // Go cuts at byte 8 even inside a rune; JSON then shows each stray byte
    // as U+FFFD, which is what this keeps.
    let end = s.id.len().min(8);
    format!(
        "claude {}",
        gocompat::utf8::to_valid_string(&s.id.as_bytes()[..end])
    )
}

/// Go `changed`: b differs from a in anything but a sub-second
/// LastActivity move.
pub fn changed(a: &AgentCard, b: &AgentCard) -> bool {
    let nanos = |t: &GoTime| i128::from(t.secs) * 1_000_000_000 + i128::from(t.nanos);
    let d = nanos(&b.last_activity) - nanos(&a.last_activity);
    if d >= 1_000_000_000 || d <= -1_000_000_000 {
        return true;
    }
    a.key != b.key
        || a.device_id != b.device_id
        || a.agent != b.agent
        || a.session_id != b.session_id
        || a.pid != b.pid
        || a.name != b.name
        || a.title != b.title
        || a.cwd != b.cwd
        || a.status != b.status
        || a.session_kind != b.session_kind
        || a.entrypoint != b.entrypoint
        || a.version != b.version
        || a.peer_features != b.peer_features
        || !a.started_at.equal(&b.started_at)
        || !a.updated_at.equal(&b.updated_at)
        || a.last_text != b.last_text
        || a.transcript != b.transcript
        || a.repo_key != b.repo_key
        || a.permission_mode != b.permission_mode
        || a.name_source != b.name_source
        || a.ai_title != b.ai_title
}

/// Go `sortCards`: LastActivity (UpdatedAt when there is none), newest
/// first, then by key.
fn sort_cards(cards: &mut [AgentCard]) {
    let at = |c: &AgentCard| {
        if c.last_activity.is_zero() {
            instant(&c.updated_at)
        } else {
            instant(&c.last_activity)
        }
    };
    cards.sort_by(|a, b| at(b).cmp(&at(a)).then_with(|| a.key.cmp(&b.key)));
}

impl Drop for Inner {
    fn drop(&mut self) {
        let mut p = lock(&self.pubs);
        p.closed = true;
        for q in p.subs.values() {
            q.close();
        }
    }
}
