//! Go `run.go`: scan on file events and process exits, fall back to an
//! interval while a watcher is down, and rescan everything every
//! SafetyInterval regardless.
//!
//! Watchers start on a thread of their own (card #601): on a busy Mac
//! `FSEventStreamStart` can take a minute, and the roster must not wait on
//! it. Until they are up the loop polls every interval, as it does while a
//! watcher is down.

use std::collections::{HashMap, HashSet};
use std::future::pending;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};
use tokio_util::sync::CancellationToken;

use crate::session::is_session_file;
use crate::transcript::{base_name, transcript_id};
use crate::watcher::{Inner, Watcher};

/// How often an unlisted session's transcript activity may force a full
/// scan (card #433).
const UNKNOWN_EVERY: Duration = Duration::from_secs(10);

/// How long Run, ending, waits for a watcher start still running.
const START_WAIT: Duration = Duration::from_secs(1);

/// One file event (Go `fswatch.Event`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FsEvent {
    /// The path that changed, as the watcher spells it.
    pub path: String,
    /// Events were lost: rescan everything.
    pub overflow: bool,
}

/// A running file watcher (Go's `eventSource`): its events, and how to
/// stop it. A closed channel is a watcher that died.
pub struct FsSource {
    pub events: mpsc::Receiver<FsEvent>,
    pub stop: Option<Box<dyn FnOnce() + Send>>,
}

impl FsSource {
    /// A source fed by `events`, stopped by `stop`.
    pub fn new(events: mpsc::Receiver<FsEvent>, stop: impl FnOnce() + Send + 'static) -> FsSource {
        FsSource {
            events,
            stop: Some(Box::new(stop)),
        }
    }

    /// Go `Close`.
    pub fn close(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}

impl Drop for FsSource {
    fn drop(&mut self) {
        self.close();
    }
}

impl std::fmt::Debug for FsSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FsSource").finish_non_exhaustive()
    }
}

/// Why a watch could not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchError {
    /// Go `fswatch.ErrUnsupported`: no backend here; don't retry.
    Unsupported,
    /// Anything else (a dir that doesn't exist yet): retried on the next
    /// tick.
    Other(String),
}

/// What a watcher start on its own thread hands back: one result for each
/// watcher it was asked to start.
#[derive(Default)]
struct Started {
    sessions: Option<Result<FsSource, WatchError>>,
    projects: Option<Result<FsSource, WatchError>>,
}

/// Run's state (Go `loop`). It lives on the run task only.
struct Loop {
    x: Arc<Inner>,
    sessions: Option<FsSource>,
    projects: Option<FsSource>,
    unsupported: bool,
    /// A watcher start is running; its result comes on `started_tx`'s
    /// channel. At most one runs at a time.
    starting: bool,
    started_tx: mpsc::Sender<Started>,
    exits: HashMap<i64, Box<dyn FnOnce() + Send>>,
    exited_tx: mpsc::Sender<()>,
    full: bool,
    dirty: HashSet<String>,
    /// When a transcript of a session that isn't listed last asked for a
    /// full scan (card #433), so a busy unlisted transcript asks at most
    /// once per [`UNKNOWN_EVERY`] (subagent transcripts are never listed).
    unknown_at: Option<Instant>,
}

async fn next_event(src: &mut Option<FsSource>) -> Option<FsEvent> {
    match src {
        Some(s) => s.events.recv().await,
        None => pending().await,
    }
}

impl Watcher {
    /// Go `Run`: scans until `cancel` fires: on file events under
    /// claude_dir/sessions and claude_dir/projects and on session process
    /// exit where the OS reports them, else every interval, with a full
    /// rescan every safety interval. The first scan doesn't wait for the
    /// watchers: they start on their own thread, and a full scan follows
    /// once one is up, so nothing written meanwhile is missed. Returns once
    /// every watcher and exit wait has stopped, except a watcher whose
    /// start outlasts [`START_WAIT`]: it stops itself when its start ends.
    pub async fn run(&self, cancel: CancellationToken) {
        let x = self.inner.clone();
        let (exited_tx, mut exited_rx) = mpsc::channel::<()>(1);
        let (started_tx, mut started_rx) = mpsc::channel::<Started>(1);
        let mut l = Loop {
            x: x.clone(),
            sessions: None,
            projects: None,
            unsupported: false,
            starting: false,
            started_tx,
            exits: HashMap::new(),
            exited_tx,
            full: false,
            dirty: HashSet::new(),
            unknown_at: None,
        };
        x.scan();
        l.track_exits();
        l.start_watchers();

        let far = || Instant::now() + Duration::from_secs(86_400 * 365);
        let safety = sleep_until(Instant::now() + l.period());
        let settle = sleep_until(far());
        tokio::pin!(safety, settle);
        let mut safety_due = true;
        let mut armed = false;

        loop {
            let mut due = false;
            tokio::select! {
                _ = cancel.cancelled() => break,
                e = next_event(&mut l.sessions) => match e {
                    None => {
                        if let Some(mut s) = l.sessions.take() {
                            s.close();
                        }
                        l.full = true;
                        due = true;
                    }
                    // Any session file (not the .key secrets) added,
                    // rewritten or removed: reread them all now.
                    Some(e) => if e.overflow || is_session_file(base_name(&e.path)) {
                        l.full = true;
                        due = true;
                    },
                },
                e = next_event(&mut l.projects) => match e {
                    None => {
                        if let Some(mut s) = l.projects.take() {
                            s.close();
                        }
                        l.full = true;
                        due = true;
                    }
                    Some(e) => if e.overflow {
                        l.full = true;
                        due = true;
                    } else if let Some(id) = transcript_id(&e.path) {
                        // A session that writes its transcript but isn't
                        // listed (its session file was unreadable when last
                        // scanned) is read again in full (card #433).
                        // Subagent transcripts (deeper down) never are.
                        if top_transcript(&e.path) && !x.has_session(&id) && l.unknown_due() {
                            l.full = true;
                        }
                        l.dirty.insert(id);
                        due = true;
                    },
                },
                Some(()) = exited_rx.recv() => {
                    l.full = true;
                    due = true;
                }
                Some(st) = started_rx.recv() => {
                    // A watcher that just came up missed what changed while
                    // it started: read everything again.
                    if l.install(st) {
                        l.full = true;
                        due = true;
                    }
                }
                _ = &mut settle, if armed => armed = false,
                _ = &mut safety, if safety_due => {
                    safety_due = false;
                    l.start_watchers();
                    l.full = true;
                }
            }
            x.watching.store(
                l.sessions.is_some() && l.projects.is_some(),
                std::sync::atomic::Ordering::SeqCst,
            );
            if due && !armed {
                armed = true;
                settle.as_mut().reset(Instant::now() + x.settle);
            }
            if armed {
                continue; // wait for the burst to settle
            }
            if l.full {
                x.scan();
                l.track_exits();
                l.full = false;
                l.dirty.clear();
                safety.as_mut().reset(Instant::now() + l.period());
                safety_due = true;
            } else if !l.dirty.is_empty() {
                x.rescan_tails(&l.dirty);
                l.dirty.clear();
            }
        }
        // A start still running hands its watchers back to be stopped. Close
        // must stay prompt, so a slow one (FSEvents on a busy Mac) is left
        // to stop its own when it ends: its send then fails.
        if l.starting {
            let _ = tokio::time::timeout(START_WAIT, started_rx.recv()).await;
        }
        l.close();
        x.watching.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Whether `path` is a session's own transcript, `projects/<slug>/<id>.jsonl`
/// (not a subagent's, further down).
fn top_transcript(path: &str) -> bool {
    Path::new(path)
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .is_some_and(|n| n == "projects")
}

impl Loop {
    /// Whether an unlisted session's transcript may ask for a full scan
    /// now (at most once per [`UNKNOWN_EVERY`]).
    fn unknown_due(&mut self) -> bool {
        let now = Instant::now();
        if self
            .unknown_at
            .is_some_and(|t| now.duration_since(t) < UNKNOWN_EVERY)
        {
            return false;
        }
        self.unknown_at = Some(now);
        true
    }

    /// SafetyInterval while both watchers run, else Interval.
    fn period(&self) -> Duration {
        if self.sessions.is_some() && self.projects.is_some() {
            self.x.safety_interval
        } else {
            self.x.interval
        }
    }

    /// Starts whichever watcher is not running, on a thread of its own;
    /// [`Loop::install`] takes what it hands back. A dir that doesn't exist
    /// yet fails and is retried on the next tick.
    fn start_watchers(&mut self) {
        if self.unsupported || self.starting {
            return;
        }
        let Some(watch) = self.x.opts.watch.clone() else {
            self.unsupported = true;
            return;
        };
        let (want_sessions, want_projects) = (self.sessions.is_none(), self.projects.is_none());
        if !want_sessions && !want_projects {
            return;
        }
        let root = |dir: &str| gocompat::filepath::join(&[self.x.opts.claude_dir.as_str(), dir]);
        let (sessions_dir, projects_dir) = (root("sessions"), root("projects"));
        let tx = self.started_tx.clone();
        let spawned = std::thread::Builder::new()
            .name("presence-watch".into())
            .spawn(move || {
                let mut st = Started::default();
                if want_sessions {
                    st.sessions = Some(watch(Path::new(&sessions_dir), false));
                }
                let unsupported = matches!(st.sessions, Some(Err(WatchError::Unsupported)));
                if want_projects && !unsupported {
                    st.projects = Some(watch(Path::new(&projects_dir), true));
                }
                // The loop may have ended: its watchers then stop here.
                let _ = tx.blocking_send(st);
            });
        // No thread now: the next tick tries again.
        self.starting = spawned.is_ok();
    }

    /// Takes the watchers a start handed back; true when one came up.
    fn install(&mut self, st: Started) -> bool {
        self.starting = false;
        let mut unsupported = false;
        let mut up = false;
        for (got, slot) in [
            (st.sessions, &mut self.sessions),
            (st.projects, &mut self.projects),
        ] {
            match got {
                Some(Ok(src)) => {
                    *slot = Some(src);
                    up = true;
                }
                Some(Err(WatchError::Unsupported)) => unsupported = true,
                Some(Err(WatchError::Other(_))) | None => {}
            }
        }
        if unsupported {
            self.unsupported = true;
            self.close_watchers();
            return false;
        }
        up
    }

    /// Waits on the process of every live session, so an agent that dies
    /// without removing its session file leaves at once.
    fn track_exits(&mut self) {
        let pids = self.x.live_pids();
        let gone: Vec<i64> = self
            .exits
            .keys()
            .filter(|p| !pids.contains(p))
            .copied()
            .collect();
        for pid in gone {
            if let Some(stop) = self.exits.remove(&pid) {
                stop();
            }
        }
        let Some(watch_exit) = self.x.watch_exit.clone() else {
            return;
        };
        for pid in pids {
            if self.exits.contains_key(&pid) {
                continue;
            }
            let tx = self.exited_tx.clone();
            if let Some(stop) = watch_exit(
                pid,
                Box::new(move || {
                    let _ = tx.try_send(());
                }),
            ) {
                self.exits.insert(pid, stop);
            }
        }
    }

    fn close_watchers(&mut self) {
        if let Some(mut s) = self.sessions.take() {
            s.close();
        }
        if let Some(mut s) = self.projects.take() {
            s.close();
        }
    }

    /// Stops every watcher and exit wait and waits for them.
    fn close(&mut self) {
        self.close_watchers();
        for (_, stop) in self.exits.drain() {
            stop();
        }
    }
}
