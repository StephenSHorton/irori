//! The PTY half, on desktop: rsz-term panes.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use base64::Engine as _;
use parking_lot::Mutex;
use rsz_term::{Out, Pane, PaneConfig, Sink};

use crate::{NO_TERM, OpenSpec, Scope, TermId, TermInfo, TermMsg, Viewer};

/// Every terminal on this device.
pub struct Terms {
    next: AtomicU32,
    map: Arc<Mutex<HashMap<TermId, Entry>>>,
    /// Panes closed but maybe not gone yet, for [`Terms::settle`]. A pane
    /// removes its shell's bootstrap folder (rsz-zdot-*, rsz-bash-* in the
    /// temp dir) only once the shell has exited, on its own thread (#960).
    closing: Mutex<Vec<Arc<Pane>>>,
}

struct Entry {
    scope: Scope,
    cwd: String,
    /// `None` once the shell exited (or before spawn returns).
    pane: Option<Arc<Pane>>,
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
    exit: Option<Option<u32>>,
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
                pane: None,
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
            Some(e) if e.cache.exit.is_none() => e.pane = Some(pane),
            _ => self.closed(vec![pane]),
        }
        Ok(TermInfo {
            id,
            scope: spec.scope,
            cwd,
            title: String::new(),
            running: true,
        })
    }

    /// The view for a terminal: the cached state first, then a full frame.
    /// A second attach replaces the first view (one local view in S1).
    pub fn attach(&self, id: TermId, viewer: Arc<Viewer>) -> Result<(), String> {
        let (pane, replay) = {
            let mut map = self.map.lock();
            let e = map.get_mut(&id).ok_or(NO_TERM)?;
            e.viewer = Some(viewer.clone());
            (e.pane.clone(), e.cache.replay())
        };
        for m in replay {
            (viewer.msg)(m);
        }
        if let Some(p) = pane {
            p.redraw();
        }
        Ok(())
    }

    /// The view went away; the shell keeps running.
    pub fn detach(&self, id: TermId) {
        if let Some(e) = self.map.lock().get_mut(&id) {
            e.viewer = None;
        }
    }

    pub fn write(&self, id: TermId, data: Vec<u8>) -> Result<(), String> {
        self.pane(id)?.write(data);
        Ok(())
    }

    pub fn paste(&self, id: TermId, text: &str) -> Result<(), String> {
        self.pane(id)?.paste(text);
        Ok(())
    }

    pub fn resize(&self, id: TermId, cols: u16, rows: u16) -> Result<(), String> {
        self.pane(id)?.resize(cols, rows);
        Ok(())
    }

    pub fn scroll(&self, id: TermId, delta: i32) -> Result<(), String> {
        self.pane(id)?.scroll(delta);
        Ok(())
    }

    pub fn cell_px(&self, id: TermId, w: u16, h: u16) -> Result<(), String> {
        self.pane(id)?.set_cell_px(w, h);
        Ok(())
    }

    pub fn focus(&self, id: TermId, on: bool) -> Result<(), String> {
        self.pane(id)?.set_focused(on);
        Ok(())
    }

    pub fn redraw(&self, id: TermId) -> Result<(), String> {
        self.pane(id)?.redraw();
        Ok(())
    }

    /// The local view painted frames up to `seq`.
    pub fn ack(&self, id: TermId, seq: u32) -> Result<(), String> {
        // A view may ack a frame that arrived just before the shell exited.
        if let Ok(p) = self.pane(id) {
            p.ack(seq);
        }
        Ok(())
    }

    /// The visible screen as text.
    pub fn screen_text(&self, id: TermId) -> Result<String, String> {
        Ok(self.pane(id)?.screen_text())
    }

    /// Kills the shell and forgets the terminal.
    pub fn close(&self, id: TermId) -> Result<(), String> {
        let e = self.map.lock().remove(&id).ok_or(NO_TERM)?;
        self.closed(e.pane.into_iter().collect());
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
                running: e.pane.is_some(),
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
            .map(|(id, e)| (*id, e.scope.clone(), e.pane.is_some()))
            .collect();
        v.sort_by_key(|t| t.0);
        v
    }

    /// Closes every shell (Irori is quitting). It never joins: a stuck
    /// shell must not hold up quit (design section 6.3). [`Terms::settle`]
    /// waits for them, up to a deadline.
    pub fn close_all(&self) {
        let panes: Vec<_> = self
            .map
            .lock()
            .drain()
            .filter_map(|(_, e)| e.pane)
            .collect();
        self.closed(panes);
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

    /// Closes `panes` and keeps them for [`Terms::settle`], forgetting
    /// the ones whose shell has already gone.
    fn closed(&self, panes: Vec<Arc<Pane>>) {
        for p in &panes {
            p.close();
        }
        let mut closing = self.closing.lock();
        closing.retain(|p| p.state() != rsz_term::State::Closed);
        closing.extend(panes);
    }

    fn pane(&self, id: TermId) -> Result<Arc<Pane>, String> {
        let map = self.map.lock();
        let e = map.get(&id).ok_or(NO_TERM)?;
        e.pane
            .clone()
            .ok_or_else(|| "the shell has exited".to_string())
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
                let p = map.lock().get(&id).and_then(|e| e.pane.clone());
                if let Some(p) = p {
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
            Out::Exit(code) => TermMsg::Exit { code },
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
        let viewer = {
            let mut m = map.lock();
            let Some(e) = m.get_mut(&id) else { return };
            remember(e, &msg);
            e.viewer.clone()
        };
        if let Some(v) = viewer
            && !(v.msg)(msg)
        {
            forget_viewer(&map, id, &v);
        }
    })
}

/// Keeps what a later view needs, and drops the pane on Exit: this runs on
/// rsz-term's actor thread, so the last `Arc<Pane>` may go here, which is
/// safe because Pane has no joining Drop.
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
            e.pane = None;
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
