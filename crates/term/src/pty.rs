//! The PTY half, on desktop: rsz-term panes.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use base64::Engine as _;
use parking_lot::Mutex;
use rsz_term::{Out, Pane, PaneConfig, Sink};

use crate::tail::{Emit, Tail, TailOut};
use crate::{NO_TERM, OpenSpec, Scope, TailSpec, TermId, TermInfo, TermMsg, Viewer};

/// What `write` and `paste` answer for a tail (live-tasks.md section 5.1).
const READ_ONLY: &str = "read only";

/// Every terminal on this device.
pub struct Terms {
    next: AtomicU32,
    map: Arc<Mutex<HashMap<TermId, Entry>>>,
    /// Panes closed but maybe not gone yet, for [`Terms::settle`]. A pane
    /// removes its shell's bootstrap folder (rsz-zdot-*, rsz-bash-* in the
    /// temp dir) only once the shell has exited, on its own thread (#960).
    closing: Mutex<Vec<Arc<Pane>>>,
    /// Tail threads still running, for tests ([`Terms::tail_threads`]).
    tail_threads: Arc<AtomicUsize>,
}

/// What feeds a terminal: a shell's PTY, or a run's output file.
#[derive(Clone)]
enum Src {
    Pty(Arc<Pane>),
    /// Kept after its run ends: the tab still scrolls and redraws.
    Tail(Arc<Tail>),
}

struct Entry {
    scope: Scope,
    cwd: String,
    /// `None` once the shell exited (or before spawn returns). A tail's
    /// stays until the terminal is closed.
    src: Option<Src>,
    viewer: Option<Arc<Viewer>>,
    cache: Cache,
}

/// What a newly attached view needs before its first full frame (design
/// section 6.2): rsz-term sends these only when they change.
#[derive(Default)]
struct Cache {
    title: String,
    shell: Option<TermMsg>,
    progress: Option<TermMsg>,
    /// Images' bytes by id, pruned on ImageDrop.
    images: Vec<(u32, TermMsg)>,
    placements: Option<TermMsg>,
    exit: Option<Option<i32>>,
}

impl Default for Terms {
    fn default() -> Self {
        Terms::new()
    }
}

impl Terms {
    pub fn new() -> Terms {
        Terms {
            next: AtomicU32::new(0),
            map: Arc::new(Mutex::new(HashMap::new())),
            closing: Mutex::new(Vec::new()),
            tail_threads: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Whether this device can run terminals.
    pub fn supported(&self) -> bool {
        true
    }

    /// Starts a new shell for a scope. A scope may have several; nothing
    /// starts one but this (the person, or later an agent, asking).
    pub fn open(&self, spec: OpenSpec) -> Result<TermInfo, String> {
        let cwd = spec
            .cwd
            .filter(|d| std::path::Path::new(d).is_dir())
            .or_else(|| {
                std::env::var_os("HOME")
                    .or_else(|| std::env::var_os("USERPROFILE"))
                    .map(|h| h.to_string_lossy().into_owned())
            })
            .unwrap_or_default();
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.map.lock().insert(
            id,
            Entry {
                scope: spec.scope.clone(),
                cwd: cwd.clone(),
                src: None,
                viewer: None,
                cache: Cache::default(),
            },
        );
        let cfg = PaneConfig {
            cols: spec.cols.max(2),
            rows: spec.rows.max(1),
            command: spec.command.clone().filter(|c| !c.is_empty()),
            cwd: (!cwd.is_empty()).then(|| cwd.clone()),
            // Irori has no input bar, so the shell keeps its own prompt,
            // with suzuri's marks still on (S-9, route a).
            keep_prompt: true,
            term_program: Some("irori".into()),
            env: vec![("IRORI_TERM_ID".into(), id.to_string())],
            ..Default::default()
        };
        let pane = match Pane::spawn(cfg, sink(Arc::downgrade(&self.map), id)) {
            Ok(p) => Arc::new(p),
            Err(e) => {
                self.map.lock().remove(&id);
                return Err(format!("could not start a shell: {e}"));
            }
        };
        let mut map = self.map.lock();
        match map.get_mut(&id) {
            // The shell may already have exited; then the entry keeps no pane.
            Some(e) if e.cache.exit.is_none() => e.src = Some(Src::Pty(pane)),
            _ => {
                drop(map);
                self.closed(vec![Src::Pty(pane)]);
            }
        }
        Ok(TermInfo {
            id,
            scope: spec.scope,
            cwd,
            title: String::new(),
            running: true,
            tail: None,
        })
    }

    /// Opens a read-only terminal on a run's output file (live-tasks.md
    /// section 5.1): no PTY, the file fed into the engine as it grows,
    /// frames as a shell's. It shows the run's end as an `Exit` and stays
    /// until closed; closing it never touches the run.
    pub fn open_tail(&self, spec: TailSpec) -> Result<TermInfo, String> {
        let cwd = spec
            .path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.map.lock().insert(
            id,
            Entry {
                scope: spec.scope.clone(),
                cwd: cwd.clone(),
                src: None,
                viewer: None,
                cache: Cache::default(),
            },
        );
        let tail = Tail::start(
            spec.path,
            spec.label,
            spec.run,
            emit(Arc::downgrade(&self.map), id),
            &self.tail_threads,
        );
        let tail = match tail {
            Ok(t) => Arc::new(t),
            Err(e) => {
                self.map.lock().remove(&id);
                return Err(e);
            }
        };
        let info = {
            let mut map = self.map.lock();
            map.get_mut(&id).map(|e| {
                e.src = Some(Src::Tail(tail.clone()));
                TermInfo {
                    id,
                    scope: spec.scope,
                    cwd,
                    title: e.cache.title.clone(),
                    running: e.cache.exit.is_none(),
                    tail: Some(tail.info(e.cache.exit)),
                }
            })
        };
        match info {
            Some(i) => Ok(i),
            // Closed before it was even returned.
            None => {
                tail.join();
                Err(NO_TERM.into())
            }
        }
    }

    /// The person pressed Stop on a tail's run: its view says "Stopped"
    /// whatever end follows (live-tasks.md section 5.1).
    pub fn mark_stopped(&self, id: TermId) -> Result<(), String> {
        match self.src(id)? {
            Src::Tail(t) => {
                t.mark_stopped();
                Ok(())
            }
            Src::Pty(_) => Err("not a tail".into()),
        }
    }

    /// The view for a terminal: the cached state first, then a full frame.
    /// A second attach replaces the first view (one local view in S1).
    pub fn attach(&self, id: TermId, viewer: Arc<Viewer>) -> Result<(), String> {
        let (src, replay) = {
            let mut map = self.map.lock();
            let e = map.get_mut(&id).ok_or(NO_TERM)?;
            e.viewer = Some(viewer.clone());
            (e.src.clone(), e.cache.replay())
        };
        for m in replay {
            (viewer.msg)(m);
        }
        match src {
            Some(Src::Pty(p)) => p.redraw(),
            Some(Src::Tail(t)) => t.redraw(),
            None => {}
        }
        Ok(())
    }

    /// The view went away; the shell keeps running.
    pub fn detach(&self, id: TermId) {
        if let Some(e) = self.map.lock().get_mut(&id) {
            e.viewer = None;
        }
    }

    /// Typing; a tail refuses it.
    pub fn write(&self, id: TermId, data: Vec<u8>) -> Result<(), String> {
        match self.src(id)? {
            Src::Pty(p) => p.write(data),
            Src::Tail(_) => return Err(READ_ONLY.into()),
        }
        Ok(())
    }

    pub fn paste(&self, id: TermId, text: &str) -> Result<(), String> {
        match self.src(id)? {
            Src::Pty(p) => p.paste(text),
            Src::Tail(_) => return Err(READ_ONLY.into()),
        }
        Ok(())
    }

    pub fn resize(&self, id: TermId, cols: u16, rows: u16) -> Result<(), String> {
        match self.src(id)? {
            Src::Pty(p) => p.resize(cols, rows),
            Src::Tail(t) => t.resize(cols, rows),
        }
        Ok(())
    }

    pub fn scroll(&self, id: TermId, delta: i32) -> Result<(), String> {
        match self.src(id)? {
            Src::Pty(p) => p.scroll(delta),
            Src::Tail(t) => t.scroll(delta),
        }
        Ok(())
    }

    /// A tail shows no images, so it has no use for the cell size.
    pub fn cell_px(&self, id: TermId, w: u16, h: u16) -> Result<(), String> {
        if let Src::Pty(p) = self.src(id)? {
            p.set_cell_px(w, h);
        }
        Ok(())
    }

    /// A tail takes no input, so focus changes nothing.
    pub fn focus(&self, id: TermId, on: bool) -> Result<(), String> {
        if let Src::Pty(p) = self.src(id)? {
            p.set_focused(on);
        }
        Ok(())
    }

    pub fn redraw(&self, id: TermId) -> Result<(), String> {
        match self.src(id)? {
            Src::Pty(p) => p.redraw(),
            Src::Tail(t) => t.redraw(),
        }
        Ok(())
    }

    /// The local view painted frames up to `seq`.
    pub fn ack(&self, id: TermId, seq: u32) -> Result<(), String> {
        // A view may ack a frame that arrived just before the shell exited.
        match self.src(id) {
            Ok(Src::Pty(p)) => p.ack(seq),
            Ok(Src::Tail(t)) => t.ack(seq),
            Err(_) => {}
        }
        Ok(())
    }

    /// The visible screen as text.
    pub fn screen_text(&self, id: TermId) -> Result<String, String> {
        Ok(match self.src(id)? {
            Src::Pty(p) => p.screen_text(),
            Src::Tail(t) => t.screen_text(),
        })
    }

    /// Kills the shell and forgets the terminal. A tail's threads stop;
    /// its run goes on.
    pub fn close(&self, id: TermId) -> Result<(), String> {
        let e = self.map.lock().remove(&id).ok_or(NO_TERM)?;
        self.closed(e.src.into_iter().collect());
        Ok(())
    }

    /// A scope's terminals, oldest first: running ones, and ones whose
    /// shell exited but that nobody closed yet.
    pub fn in_scope(&self, scope: &Scope) -> Vec<TermInfo> {
        let mut v: Vec<TermInfo> = self
            .map
            .lock()
            .iter()
            .filter(|(_, e)| &e.scope == scope)
            .map(|(id, e)| TermInfo {
                id: *id,
                scope: e.scope.clone(),
                cwd: e.cwd.clone(),
                title: e.cache.title.clone(),
                running: e.running(),
                tail: match &e.src {
                    Some(Src::Tail(t)) => Some(t.info(e.cache.exit)),
                    _ => None,
                },
            })
            .collect();
        v.sort_by_key(|t| t.id);
        v
    }

    /// Every terminal's id, scope and whether its shell still runs.
    pub fn list(&self) -> Vec<(TermId, Scope, bool)> {
        let mut v: Vec<_> = self
            .map
            .lock()
            .iter()
            .map(|(id, e)| (*id, e.scope.clone(), e.running()))
            .collect();
        v.sort_by_key(|t| t.0);
        v
    }

    /// Closes every shell (Irori is quitting). It never joins a shell: a
    /// stuck shell must not hold up quit (design section 6.3).
    /// [`Terms::settle`] waits for them, up to a deadline. Tails' threads
    /// are stopped and joined here (within one 100 ms poll).
    pub fn close_all(&self) {
        let srcs: Vec<_> = self.map.lock().drain().filter_map(|(_, e)| e.src).collect();
        self.closed(srcs);
    }

    /// Tail threads still running on this device's terminals. For tests.
    #[doc(hidden)]
    pub fn tail_threads(&self) -> usize {
        self.tail_threads.load(Ordering::SeqCst)
    }

    /// Waits, at most `within`, for every shell closed so far to be gone:
    /// its threads ended and its bootstrap folder removed from the temp
    /// dir. Without it a process that exits right after closing leaves
    /// rsz-zdot-* behind (#960). Panes still running past the deadline are
    /// abandoned (design section 6.3); returns whether all were gone.
    pub fn settle(&self, within: Duration) -> bool {
        let panes = std::mem::take(&mut *self.closing.lock());
        if panes.is_empty() {
            return true;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let joined = std::thread::Builder::new()
            .name("irori-term-settle".into())
            .spawn(move || {
                for p in &panes {
                    p.join();
                }
                let _ = tx.send(());
            });
        joined.is_ok() && rx.recv_timeout(within).is_ok()
    }

    /// Closes `srcs`: panes are kept for [`Terms::settle`], forgetting the
    /// ones whose shell has already gone; tails are stopped and joined
    /// (they have no bootstrap folder to wait on). Never called with the
    /// map locked: a tail's threads take it to send.
    fn closed(&self, srcs: Vec<Src>) {
        let mut panes = Vec::new();
        for s in srcs {
            match s {
                Src::Pty(p) => {
                    p.close();
                    panes.push(p);
                }
                Src::Tail(t) => t.join(),
            }
        }
        let mut closing = self.closing.lock();
        closing.retain(|p| p.state() != rsz_term::State::Closed);
        closing.extend(panes);
    }

    fn src(&self, id: TermId) -> Result<Src, String> {
        let map = self.map.lock();
        let e = map.get(&id).ok_or(NO_TERM)?;
        e.src
            .clone()
            .ok_or_else(|| "the shell has exited".to_string())
    }
}

impl Entry {
    /// A shell runs, or a tail's run has not ended.
    fn running(&self) -> bool {
        match &self.src {
            Some(Src::Pty(_)) => true,
            Some(Src::Tail(_)) => self.cache.exit.is_none(),
            None => false,
        }
    }
}

impl Cache {
    fn replay(&self) -> Vec<TermMsg> {
        let mut v = Vec::new();
        if !self.title.is_empty() {
            v.push(TermMsg::Title {
                title: self.title.clone(),
            });
        }
        v.extend(self.shell.clone());
        v.extend(self.progress.clone());
        v.extend(self.images.iter().map(|(_, m)| m.clone()));
        v.extend(self.placements.clone());
        if let Some(code) = self.exit {
            v.push(TermMsg::Exit { code });
        }
        v
    }
}

/// The pane's sink. rsz-term calls it from four threads, so it only takes
/// the map's lock for a moment and never blocks on a view. It holds the map
/// weakly: the map holds the pane, and the pane holds the sink.
fn sink(map: Weak<Mutex<HashMap<TermId, Entry>>>, id: TermId) -> Sink {
    Arc::new(move |o: Out| {
        let Some(map) = map.upgrade() else { return };
        let msg = match o {
            Out::Frame(b) => {
                let v = map.lock().get(&id).and_then(|e| e.viewer.clone());
                if let Some(v) = v
                    && !(v.frame)(b)
                {
                    forget_viewer(&map, id, &v);
                }
                return;
            }
            Out::NoticeAlive(nid) => {
                // Always answered, so OSC 99 queries never hang (design
                // section 3). Irori shows no notices yet: none is live.
                let p = map.lock().get(&id).and_then(|e| e.src.clone());
                if let Some(Src::Pty(p)) = p {
                    p.reply(rsz_term::protocols::alive_reply(&nid, &[]));
                }
                return;
            }
            // Notices become toasts in a later slice.
            Out::Notice(_) | Out::NoticeClose(_) => return,
            Out::Title(title) => TermMsg::Title { title },
            Out::Bell => TermMsg::Bell,
            Out::Clipboard(text) => TermMsg::Clipboard { text },
            Out::Shell(st) => TermMsg::Shell {
                integrated: st.integrated,
                cwd: st.cwd,
                program: st.program,
                prompts: st.prompts,
            },
            Out::CommandDone { exit, command } => TermMsg::CommandDone { exit, command },
            // suzuri's codes are u32; Windows' big ones wrap as they do
            // in claude's own trailer.
            Out::Exit(code) => TermMsg::Exit {
                code: code.map(|c| c as i32),
            },
            Out::Progress { state, percent } => TermMsg::Progress { state, percent },
            Out::ImageData(d) => TermMsg::Image {
                id: d.id,
                name: d.name,
                path: d.path,
                format: d.format,
                w: d.width,
                h: d.height,
                z: d.compressed,
                data: base64::engine::general_purpose::STANDARD.encode(&d.data),
            },
            Out::ImageDrop(ids) => TermMsg::ImageDrop { ids },
            Out::Placements(list) => TermMsg::Placements {
                list: list
                    .iter()
                    .map(|p| {
                        let (cx, cy, cw, ch) = p.crop;
                        [
                            p.image as i64,
                            p.row as i64,
                            p.col as i64,
                            p.cols as i64,
                            p.rows as i64,
                            p.z as i64,
                            p.inline as i64,
                            cx as i64,
                            cy as i64,
                            cw as i64,
                            ch as i64,
                        ]
                    })
                    .collect(),
            },
        };
        deliver(&map, id, msg);
    })
}

/// A tail's sends: the same frames and messages a pane's sink passes on.
/// Answers false once the terminals are gone or the terminal was closed.
fn emit(map: Weak<Mutex<HashMap<TermId, Entry>>>, id: TermId) -> Emit {
    Arc::new(move |o: TailOut| {
        let Some(map) = map.upgrade() else {
            return false;
        };
        match o {
            TailOut::Frame(b) => {
                let v = {
                    let m = map.lock();
                    let Some(e) = m.get(&id) else { return false };
                    e.viewer.clone()
                };
                if let Some(v) = v
                    && !(v.frame)(b)
                {
                    forget_viewer(&map, id, &v);
                }
                true
            }
            TailOut::Msg(msg) => deliver(&map, id, msg),
        }
    })
}

/// Keeps what a later view needs and passes `msg` to the view. False when
/// the terminal is gone.
fn deliver(map: &Mutex<HashMap<TermId, Entry>>, id: TermId, msg: TermMsg) -> bool {
    let viewer = {
        let mut m = map.lock();
        let Some(e) = m.get_mut(&id) else {
            return false;
        };
        remember(e, &msg);
        e.viewer.clone()
    };
    if let Some(v) = viewer
        && !(v.msg)(msg)
    {
        forget_viewer(map, id, &v);
    }
    true
}

/// Keeps what a later view needs, and drops a shell's pane on Exit: this
/// runs on rsz-term's actor thread, so the last `Arc<Pane>` may go here,
/// which is safe because Pane has no joining Drop. A tail keeps its engine
/// after its run ends, so it still scrolls and redraws.
fn remember(e: &mut Entry, msg: &TermMsg) {
    let c = &mut e.cache;
    match msg {
        TermMsg::Title { title } => c.title = title.clone(),
        TermMsg::Shell { .. } => c.shell = Some(msg.clone()),
        TermMsg::Progress { .. } => c.progress = Some(msg.clone()),
        TermMsg::Image { id, .. } => {
            c.images.retain(|(i, _)| i != id);
            c.images.push((*id, msg.clone()));
        }
        TermMsg::ImageDrop { ids } => c.images.retain(|(i, _)| !ids.contains(i)),
        TermMsg::Placements { .. } => c.placements = Some(msg.clone()),
        TermMsg::Exit { code } => {
            c.exit = Some(*code);
            if matches!(e.src, Some(Src::Pty(_))) {
                e.src = None;
            }
        }
        TermMsg::Bell | TermMsg::Clipboard { .. } | TermMsg::CommandDone { .. } => {}
    }
}

/// The view's channel closed: stop sending to it, unless a new view has
/// already replaced it.
fn forget_viewer(map: &Mutex<HashMap<TermId, Entry>>, id: TermId, v: &Arc<Viewer>) {
    if let Some(e) = map.lock().get_mut(&id)
        && e.viewer.as_ref().is_some_and(|cur| Arc::ptr_eq(cur, v))
    {
        e.viewer = None;
    }
}
