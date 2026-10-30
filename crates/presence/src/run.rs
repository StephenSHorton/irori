//! Go `run.go`: scan on file events and process exits, fall back to an
//! interval while a watcher is down, and rescan everything every
//! SafetyInterval regardless.

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

/// Run's state (Go `loop`). It lives on the run task only.
struct Loop {
    x: Arc<Inner>,
    sessions: Option<FsSource>,
    projects: Option<FsSource>,
    unsupported: bool,
    exits: HashMap<i64, Box<dyn FnOnce() + Send>>,
    exited_tx: mpsc::Sender<()>,
    full: bool,
    dirty: HashSet<String>,
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
    /// rescan every safety interval. Returns once every watcher and exit
    /// wait has stopped.
    pub async fn run(&self, cancel: CancellationToken) {
        let x = self.inner.clone();
        let (exited_tx, mut exited_rx) = mpsc::channel::<()>(1);
        let mut l = Loop {
            x: x.clone(),
            sessions: None,
            projects: None,
            unsupported: false,
            exits: HashMap::new(),
            exited_tx,
            full: false,
            dirty: HashSet::new(),
        };
        l.start_watchers();
        x.scan();
        l.track_exits();

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
                        l.dirty.insert(id);
                        due = true;
                    },
                },
                Some(()) = exited_rx.recv() => {
                    l.full = true;
                    due = true;
                }
                _ = &mut settle, if armed => armed = false,
                _ = &mut safety, if safety_due => {
                    safety_due = false;
                    l.start_watchers();
                    l.full = true;
                }
            }
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
        l.close();
    }
}

impl Loop {
    /// SafetyInterval while both watchers run, else Interval.
    fn period(&self) -> Duration {
        if self.sessions.is_some() && self.projects.is_some() {
            self.x.safety_interval
        } else {
            self.x.interval
        }
    }

    /// Starts whichever watcher is not running. A dir that doesn't exist
    /// yet fails and is retried on the next tick.
    fn start_watchers(&mut self) {
        if self.unsupported {
            return;
        }
        let Some(watch) = self.x.opts.watch.clone() else {
            self.unsupported = true;
            return;
        };
        let root = |dir: &str| gocompat::filepath::join(&[self.x.opts.claude_dir.as_str(), dir]);
        let start = |dir: &str, recursive: bool, unsupported: &mut bool| match watch(
            Path::new(&root(dir)),
            recursive,
        ) {
            Ok(s) => Some(s),
            Err(WatchError::Unsupported) => {
                *unsupported = true;
                None
            }
            Err(WatchError::Other(_)) => None,
        };
        let mut unsupported = false;
        if self.sessions.is_none() {
            self.sessions = start("sessions", false, &mut unsupported);
        }
        if self.projects.is_none() && !unsupported {
            self.projects = start("projects", true, &mut unsupported);
        }
        if unsupported {
            self.unsupported = true;
            self.close_watchers();
        }
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
