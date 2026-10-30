//! I:internal/capture/watch.go and the polling half of tailer.go's run:
//! the event loop. Transcripts are tailed when they change, reflogs of hot
//! roots read when logs/HEAD changes, open turns closed by a timer armed
//! for the next quiet deadline, and everything rescanned every
//! SafetyInterval. Where no file watcher exists, every pass runs on a
//! ticker instead.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant};

use gocompat::time::GoTime;
use sys::cancel::Cancel;
use sys::fswatch::{self, Watcher};

use crate::gitlink;
use crate::gopath;
use crate::gotime;
use crate::roots::norm_root;
use crate::rows::RepoSeen;
use crate::service::{Inner, RunState, lock};
use crate::tailer::{Found, TFile, mtime};

/// How soon a due open turn kept open by a busy card is looked at again.
pub const BUSY_RECHECK: Duration = Duration::from_secs(15);
/// Events handled together after one wake-up.
pub const EVENT_BATCH: usize = 512;
/// The loop's event queue.
const EVENTS_CAP: usize = 64;

enum Msg {
    /// A watcher event: `logs` is the normRoot of a reflog directory, ""
    /// for the transcripts tree.
    Ev {
        logs: String,
        wid: u64,
        ev: fswatch::Event,
    },
    /// The watcher stopped.
    Closed { logs: String, wid: u64 },
    /// The token was canceled.
    Wake,
}

struct LogsWatch {
    #[allow(dead_code)]
    dir: String,
    w: Watcher,
    wid: u64,
    roots: Vec<RepoSeen>,
}

struct Loop<'a> {
    c: &'a Inner,
    cancel: Cancel,
    tx: SyncSender<Msg>,
    rx: Receiver<Msg>,
    projects: String,
    tail: Option<(u64, Watcher)>,
    logs: HashMap<String, LogsWatch>,
    polled: Vec<RepoSeen>,
    hot: String,
    synced: bool,
    pending: HashMap<String, bool>,
    next_wid: u64,
    threads: Vec<std::thread::JoinHandle<()>>,
    /// One root's reflog read budget (Options.repo_budget).
    budget: Duration,
}

/// Runs until `cancel` is canceled.
pub(crate) fn run(c: &Inner, parent: &Cancel) {
    {
        let mut rs = lock(&c.run);
        c.load_cursors(&mut rs);
    }
    if !run_events(c, parent) {
        run_polling(c, parent);
    }
}

/// Every pass on a ticker, where no file watcher exists.
fn run_polling(c: &Inner, cancel: &Cancel) {
    {
        let mut rs = lock(&c.run);
        c.tail_pass(&mut rs, cancel);
        c.repo_pass(&mut rs, cancel);
    }
    let mut scan = Instant::now() + c.opts.scan_interval;
    let mut repos = Instant::now() + c.opts.repo_interval;
    loop {
        let next = scan.min(repos);
        if cancel.sleep(next.saturating_duration_since(Instant::now())) {
            return;
        }
        let now = Instant::now();
        let mut rs = lock(&c.run);
        if now >= scan {
            c.tail_pass(&mut rs, cancel);
            scan = Instant::now() + c.opts.scan_interval;
        }
        if now >= repos {
            c.repo_pass(&mut rs, cancel);
            repos = Instant::now() + c.opts.repo_interval;
        }
    }
}

/// The event-driven loop; false (having done nothing) where no watcher
/// exists.
fn run_events(c: &Inner, parent: &Cancel) -> bool {
    let cancel = parent.child();
    let mut l = Loop::new(c, cancel.clone());
    let wake_tx = l.tx.clone();
    let _reg = cancel.on_cancel(move || {
        let _ = wake_tx.try_send(Msg::Wake);
    });
    // Watch before the first pass so nothing written during it is missed.
    if let Err(fswatch::Error::Unsupported) = l.watch_projects() {
        return false;
    }
    {
        let mut rs = lock(&c.run);
        c.tail_pass(&mut rs, &cancel);
        c.repo_pass(&mut rs, &cancel);
        l.sync_logs(&mut rs, true);
    }
    let mut safety = Instant::now() + c.opts.safety_interval;
    let mut poll_tail: Option<Instant> = None;
    let mut poll_repo: Option<Instant> = None;
    let mut follow: Option<Instant> = None;
    while !cancel.is_canceled() {
        if l.tail.is_none() && poll_tail.is_none() {
            poll_tail = Some(Instant::now() + c.opts.scan_interval);
        }
        if !l.polled.is_empty() && poll_repo.is_none() {
            poll_repo = Some(Instant::now() + c.opts.repo_interval);
        }
        let quiet = {
            let rs = lock(&c.run);
            next_quiet(c, &rs).map(|at| {
                let d = gotime::sub_nanos(&at, &c.now()).max(0);
                Instant::now() + Duration::from_nanos(d.min(u64::MAX as i128) as u64)
            })
        };
        let mut next = safety;
        for t in [poll_tail, poll_repo, quiet, follow].into_iter().flatten() {
            next = next.min(t);
        }
        let got =
            l.rx.recv_timeout(next.saturating_duration_since(Instant::now()));
        let mut rs = lock(&c.run);
        match got {
            Ok(Msg::Wake) => break,
            Ok(first) => {
                let mut batch = vec![first];
                while batch.len() < EVENT_BATCH {
                    match l.rx.try_recv() {
                        Ok(m) => batch.push(m),
                        Err(_) => break,
                    }
                }
                l.handle(&mut rs, batch);
            }
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                if now >= safety {
                    c.tail_pass(&mut rs, &cancel);
                    c.repo_pass(&mut rs, &cancel);
                    l.sync_logs(&mut rs, true);
                    safety = Instant::now() + c.opts.safety_interval;
                } else if poll_tail.is_some_and(|t| now >= t) {
                    poll_tail = None;
                    let _ = l.watch_projects();
                    c.tail_pass(&mut rs, &cancel);
                } else if poll_repo.is_some_and(|t| now >= t) {
                    poll_repo = None;
                    let roots = l.polled.clone();
                    l.read_roots(&mut rs, roots);
                    l.sync_logs(&mut rs, true);
                } else if quiet.is_some_and(|t| now >= t) {
                    quiet_pass(c, &mut rs, &cancel);
                } else if follow.is_some_and(|t| now >= t) {
                    follow = None;
                    let roots = l.take_pending();
                    l.read_roots(&mut rs, roots);
                }
            }
        }
        l.sync_logs(&mut rs, false);
        drop(rs);
        if cancel.is_canceled() {
            break;
        }
        // Reflogs left behind: at once when the budget ran out, after
        // RepoInterval when a line failed.
        if let Some(at) = l.follow_up()
            && follow.is_none_or(|f| at < f)
        {
            follow = Some(at);
        }
    }
    cancel.cancel();
    if let Some((_, w)) = l.tail.take() {
        w.close();
    }
    for (_, lw) in l.logs.drain() {
        lw.w.close();
    }
    for t in l.threads.drain(..) {
        let _ = t.join();
    }
    true
}

impl<'a> Loop<'a> {
    fn new(c: &'a Inner, cancel: Cancel) -> Loop<'a> {
        let (tx, rx) = sync_channel(EVENTS_CAP);
        Loop {
            c,
            cancel,
            tx,
            rx,
            projects: c.projects_dir(),
            tail: None,
            logs: HashMap::new(),
            polled: Vec::new(),
            hot: String::new(),
            synced: false,
            pending: HashMap::new(),
            next_wid: 0,
            threads: Vec::new(),
            budget: c.opts.repo_budget,
        }
    }

    /// Starts the transcripts watcher if it isn't running.
    fn watch_projects(&mut self) -> Result<(), fswatch::Error> {
        if self.tail.is_some() {
            return Ok(());
        }
        let w = fswatch::watch(&self.cancel, &self.projects, true, Duration::ZERO)?;
        let wid = self.forward(String::new(), w.clone());
        self.tail = Some((wid, w));
        Ok(())
    }

    /// Copies a watcher's events onto the loop's queue, then reports it
    /// closed. It may wait while the loop is busy: fswatch then drops
    /// events but always delivers an Overflow for them.
    fn forward(&mut self, logs: String, w: Watcher) -> u64 {
        self.next_wid += 1;
        let wid = self.next_wid;
        let tx = self.tx.clone();
        let cancel = self.cancel.clone();
        let send = move |mut m: Msg| loop {
            match tx.try_send(m) {
                Ok(()) => return true,
                Err(TrySendError::Disconnected(_)) => return false,
                Err(TrySendError::Full(back)) => {
                    if cancel.sleep(Duration::from_millis(5)) {
                        return false;
                    }
                    m = back;
                }
            }
        };
        let h = std::thread::spawn(move || {
            while let Some(ev) = w.recv() {
                if !send(Msg::Ev {
                    logs: logs.clone(),
                    wid,
                    ev,
                }) {
                    return;
                }
            }
            send(Msg::Closed { logs, wid });
        });
        self.threads.push(h);
        wid
    }

    /// Acts on one batch of messages.
    fn handle(&mut self, rs: &mut RunState, batch: Vec<Msg>) {
        let c = self.c;
        let (mut tail_all, mut repo_all) = (false, false);
        let mut paths: Vec<String> = Vec::new();
        let mut seen_path = HashSet::new();
        let mut logs_hit: HashSet<String> = HashSet::new();
        let tail_wid = self.tail.as_ref().map(|(w, _)| *w);
        for m in batch {
            match m {
                Msg::Wake => {}
                Msg::Ev { logs, wid, ev } if logs.is_empty() => {
                    if Some(wid) != tail_wid {
                        continue; // a stopped watcher's leftovers
                    }
                    if ev.overflow {
                        tail_all = true;
                    } else if let Some(p) = self.projects_path(&ev.path)
                        && seen_path.insert(p.clone())
                    {
                        paths.push(p);
                    }
                }
                Msg::Closed { logs, wid } if logs.is_empty() => {
                    if Some(wid) == tail_wid {
                        self.tail = None;
                        tail_all = true;
                    }
                }
                Msg::Ev { logs, wid, ev } => {
                    if self.logs.get(&logs).is_none_or(|lw| lw.wid != wid) {
                        continue;
                    }
                    if ev.overflow {
                        repo_all = true;
                    } else if gocompat::text::equal_fold(&gopath::base(&ev.path), "HEAD") {
                        logs_hit.insert(logs);
                    }
                }
                Msg::Closed { logs, wid } => {
                    if self.logs.get(&logs).is_none_or(|lw| lw.wid != wid) {
                        continue;
                    }
                    // Polled (and watched again) at RepoInterval.
                    let lw = self.logs.remove(&logs).expect("checked");
                    self.polled.extend(lw.roots);
                    repo_all = true;
                }
            }
        }
        if tail_all {
            c.tail_pass(rs, &self.cancel);
        } else if !paths.is_empty() {
            c.tail_paths(rs, &self.cancel, &paths);
            // A new session's root is watched as soon as its transcript moves.
            c.note_live_roots(&self.cancel);
        }
        if repo_all {
            c.repo_pass(rs, &self.cancel);
        } else if !logs_hit.is_empty() {
            let roots: Vec<RepoSeen> = logs_hit
                .iter()
                .filter_map(|k| self.logs.get(k))
                .flat_map(|lw| lw.roots.clone())
                .collect();
            self.read_roots(rs, roots);
        }
    }

    /// A watcher path in listFiles' spelling (one tfile whichever way it was
    /// found); `None` for anything that isn't a .jsonl.
    fn projects_path(&self, p: &str) -> Option<String> {
        if !gocompat::text::to_lower_str(p).ends_with(".jsonl") {
            return None;
        }
        let abs = gopath::abs(&self.projects).ok()?;
        let rel = gopath::rel(&abs, p).ok()?;
        if rel == "." || rel.starts_with("..") {
            return None;
        }
        Some(gopath::join(&[&self.projects, &rel]))
    }

    /// Reads the reflogs of `roots`, each with its own budget, and queues
    /// those still behind.
    fn read_roots(&mut self, rs: &mut RunState, mut roots: Vec<RepoSeen>) {
        let c = self.c;
        if roots.is_empty() {
            return;
        }
        roots.sort_by(|a, b| a.root.cmp(&b.root));
        for r in &roots {
            if self.cancel.is_canceled() {
                return;
            }
            let n = norm_root(&r.root);
            let before = reflog_offset(rs, &n);
            rs.repo_deadline = Instant::now() + self.budget;
            c.watch_root(rs, &self.cancel, r);
            if !reflog_behind(rs, &n) {
                self.pending.remove(&n);
                continue;
            }
            let now = self.pending.get(&n).copied().unwrap_or(false)
                || Instant::now() > rs.repo_deadline
                || reflog_offset(rs, &n) > before;
            self.pending.insert(n, now);
        }
        c.save_cursors(rs);
    }

    /// The queued roots, cleared.
    fn take_pending(&mut self) -> Vec<RepoSeen> {
        let f = self.c.read();
        let out = self
            .pending
            .keys()
            .filter_map(|n| f.repo_roots.get(n).cloned())
            .collect();
        self.pending.clear();
        out
    }

    /// When the queued roots are read again.
    fn follow_up(&self) -> Option<Instant> {
        if self.pending.is_empty() {
            return None;
        }
        if self.pending.values().any(|&now| now) {
            return Some(Instant::now());
        }
        Some(Instant::now() + self.c.opts.repo_interval)
    }

    /// Makes the reflog watchers match the hot roots (known roots a live
    /// session works in): only those keep a logs handle open, because an
    /// open handle stops the repo folder from being renamed or removed.
    /// Without `force` it runs only when the hot set changed. A new
    /// watcher's roots are read at once.
    fn sync_logs(&mut self, rs: &mut RunState, force: bool) {
        let (roots, hot) = hot_roots(self.c);
        if !force && self.synced && hot == self.hot {
            return;
        }
        self.hot = hot;
        self.synced = true;
        let mut want: HashMap<String, (String, Vec<RepoSeen>)> = HashMap::new();
        for r in roots {
            let Ok(gd) = gitlink::git_dir(&r.root) else {
                continue; // a removed worktree
            };
            let dir = gopath::join(&[&gd, "logs"]);
            let k = norm_root(&dir);
            want.entry(k).or_insert_with(|| (dir, Vec::new())).1.push(r);
        }
        let gone: Vec<String> = self
            .logs
            .keys()
            .filter(|k| !want.contains_key(*k))
            .cloned()
            .collect();
        for k in gone {
            if let Some(lw) = self.logs.remove(&k) {
                lw.w.close();
            }
        }
        self.polled.clear();
        let mut fresh = Vec::new();
        for (k, (dir, roots)) in want {
            if let Some(lw) = self.logs.get_mut(&k) {
                lw.roots = roots;
                continue;
            }
            match fswatch::watch(&self.cancel, &dir, false, Duration::ZERO) {
                Err(_) => self.polled.extend(roots),
                Ok(w) => {
                    let wid = self.forward(k.clone(), w.clone());
                    fresh.extend(roots.iter().cloned());
                    self.logs.insert(k, LogsWatch { dir, w, wid, roots });
                }
            }
        }
        self.read_roots(rs, fresh);
    }
}

fn reflog_offset(rs: &RunState, n: &str) -> i64 {
    rs.reflogs.get(n).map_or(-1, |st| st.offset)
}

fn reflog_behind(rs: &RunState, n: &str) -> bool {
    let Some(st) = rs.reflogs.get(n) else {
        return false;
    };
    std::fs::metadata(&st.log_path).is_ok_and(|m| m.len() as i64 > st.offset)
}

/// The known roots some live session works in, and a key for that set.
fn hot_roots(c: &Inner) -> (Vec<RepoSeen>, String) {
    let live = c.live_cards();
    let f = c.read();
    let mut roots = Vec::new();
    let mut keys = Vec::new();
    for (n, r) in &f.repo_roots {
        if f.root_users
            .get(n)
            .is_some_and(|users| users.iter().any(|sid| live.contains_key(sid)))
        {
            roots.push(r.clone());
            keys.push(n.clone());
        }
    }
    keys.sort();
    (roots, keys.join("|"))
}

fn quiet_due(c: &Inner, f: &TFile) -> GoTime {
    let at = gotime::add(&f.last_grow, c.opts.quiet_after);
    if gotime::after(&f.quiet_at, &at) {
        f.quiet_at
    } else {
        at
    }
}

/// The earliest moment an open turn may close as quiet.
fn next_quiet(c: &Inner, rs: &RunState) -> Option<GoTime> {
    rs.files
        .values()
        .filter(|f| f.parser.is_open())
        .map(|f| quiet_due(c, f))
        .min_by(gotime::cmp)
}

/// Tails every open turn that is due, which closes it unless its live card
/// is busy; one that stays open is looked at again after BUSY_RECHECK.
fn quiet_pass(c: &Inner, rs: &mut RunState, cancel: &Cancel) {
    let now = c.now();
    let due: Vec<String> = rs
        .files
        .values()
        .filter(|f| f.parser.is_open() && !gotime::after(&quiet_due(c, f), &now))
        .map(|f| f.path.clone())
        .collect();
    if due.is_empty() {
        return;
    }
    let live = c.live_cards();
    for p in due {
        if cancel.is_canceled() {
            return;
        }
        let (src, size, modt) = {
            let f = &rs.files[&p];
            (f.src.clone(), f.size, f.last_grow)
        };
        let mut fd = Found {
            path: p.clone(),
            src: src.clone(),
            size,
            modt,
        };
        if let Ok(m) = std::fs::metadata(&p) {
            fd.size = m.len() as i64;
            fd.modt = mtime(&m);
        }
        let card = live.get(&src.session_id).cloned();
        c.tail_file(rs, cancel, fd, card);
        if let Some(f) = rs.files.get_mut(&p)
            && f.parser.is_open()
            && !gotime::after(&quiet_due(c, f), &c.now())
        {
            f.quiet_at = gotime::add(&c.now(), BUSY_RECHECK);
        }
    }
    c.end_tail(rs, cancel, false);
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{AgentCard, Capture, MemBlobs, NoScrub, Options, Roster};

    struct OneCard(AgentCard);

    impl Roster for OneCard {
        fn agents(&self) -> Vec<AgentCard> {
            vec![self.0.clone()]
        }
    }

    /// Fails every `rev-list` run while `left` > 0.
    struct FlakyRevList {
        left: Mutex<usize>,
    }

    impl gitlink::Git for FlakyRevList {
        fn run(
            &self,
            cancel: &Cancel,
            timeout: Duration,
            dir: &str,
            args: &[&str],
        ) -> Result<String, gitlink::GitError> {
            {
                let mut left = self.left.lock().unwrap();
                if *left > 0 && args.contains(&"rev-list") {
                    *left -= 1;
                    return Err(gitlink::GitError::Other("transient".into()));
                }
            }
            gitlink::Exec.run(cancel, timeout, dir, args)
        }
    }

    fn git(dir: &str, args: &[&str]) -> String {
        let mut cmd = Command::new("git");
        cmd.args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir);
        for (k, _) in std::env::vars() {
            if k.to_ascii_uppercase().starts_with("GIT_") {
                cmd.env_remove(k);
            }
        }
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "git {args:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn canon(p: &std::path::Path) -> String {
        let s = std::fs::canonicalize(p)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s)
    }

    /// TestEventReflogFollowUp: a root whose reflog read failed is read
    /// again after RepoInterval; one cut short by its budget at once; and
    /// following up until nothing is pending imports the commits.
    #[test]
    fn event_reflog_follow_up() {
        let (t1, t2, t3) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let (data, claude, dir) = (canon(t1.path()), canon(t2.path()), canon(t3.path()));
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "core.autocrlf", "false"]);
        std::fs::write(gopath::join(&[&dir, "README.md"]), "hello\n").unwrap();
        git(&dir, &["add", "README.md"]);
        git(&dir, &["commit", "-q", "-m", "init"]);

        let g = Arc::new(FlakyRevList {
            left: Mutex::new(1 << 20),
        });
        let mut opts = Options::new(
            &claude,
            &data,
            "dev1",
            Arc::new(NoScrub),
            Arc::new(MemBlobs::default()),
        );
        opts.git = g.clone();
        opts.repo_interval = Duration::from_secs(3600);
        opts.repo_budget = Duration::from_secs(60);
        let st = Arc::new(
            store::Store::open(
                gopath::join(&[&data, "irori.db"]),
                "dev1",
                Arc::new(hlc::Clock::new()),
            )
            .unwrap(),
        );
        let roster = Arc::new(OneCard(AgentCard {
            key: "dev1/sess-a".into(),
            session_id: "sess-a".into(),
            cwd: dir.clone(),
            status: "idle".into(),
            ..AgentCard::default()
        }));
        let c = Capture::new(st, Some(roster), opts).unwrap();
        c.repo_pass(&Cancel::new());
        std::fs::write(gopath::join(&[&dir, "a.txt"]), "a\n").unwrap();
        git(&dir, &["add", "a.txt"]);
        git(&dir, &["commit", "-q", "-m", "a"]);
        let inner = &c.inner;
        let root = inner
            .read()
            .repo_roots
            .get(&norm_root(&dir))
            .cloned()
            .expect("root seen");
        let mut l = Loop::new(inner, Cancel::new());

        let mut rs = lock(&inner.run);
        l.read_roots(&mut rs, vec![root.clone()]);
        let at = l.follow_up().expect("failed line: a follow-up");
        assert!(
            at.saturating_duration_since(Instant::now()) > Duration::from_secs(59 * 60),
            "failed line: follow-up should wait RepoInterval"
        );

        *g.left.lock().unwrap() = 0;
        l.budget = Duration::ZERO;
        git(&dir, &["commit", "-q", "--allow-empty", "-m", "b"]);
        l.read_roots(&mut rs, vec![root]);
        let at = l.follow_up().expect("over budget: a follow-up");
        assert!(
            at.saturating_duration_since(Instant::now()) <= Duration::from_secs(1),
            "over budget: follow-up should be now"
        );
        l.budget = inner.opts.repo_budget;
        for _ in 0..5 {
            let roots = l.take_pending();
            l.read_roots(&mut rs, roots);
        }
        assert!(l.follow_up().is_none(), "still pending: {:?}", l.pending);
        drop(rs);
        assert!(
            c.commit(&git(&dir, &["rev-parse", "HEAD"])).is_some(),
            "commit b not imported"
        );
    }
}
