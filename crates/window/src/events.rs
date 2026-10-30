//! The window's events (surface.md 2.10): Go's pumps, on the Core's feed.
//!
//! | Event | Payload | Emitted |
//! |---|---|---|
//! | `roster:changed` | [`RosterChange`] | every presence change |
//! | `inbox:changed` | [`InboxCounts`] | coalesced, 250 ms |
//! | `board:changed` | `{}` | coalesced, 250 ms |
//! | `capture:changed` | [`CaptureChange`] | coalesced, 300 ms |
//! | `boards:changed` | `{}` | a shared board opened, went or was renamed |
//! | `devices:pairing` | [`PairStatus`] | pairing change (M7) |
//! | `settings:launch-at-login` | bool | after every toggle attempt |
//! | `app:open` | [`Target`] | a card's Open (M6 toast) |
//! | `update:status` | [`UpdateStatus`] | updater transitions (M8) |
//!
//! `roster:changed` follows this device's presence only, as Go's
//! (`Core.Changes` is `presence.Subscribe`).

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use gocompat::sub::Queue;
use irori_core::events::{BOARD_EMIT_EVERY, CAPTURE_EMIT_EVERY, INBOX_EMIT_EVERY, coalesce};
use irori_core::{CoreEvent, Topic};
use serde::Serialize;
use specta::Type;
use sys::cancel::Cancel;

use crate::Window;
use crate::roster::RosterChange;

pub use crate::boards::EVENT_BOARDS_CHANGED;
pub use crate::settings::EVENT_LAUNCH_AT_LOGIN;
pub use crate::update::EVENT_UPDATE_STATUS;

pub const EVENT_ROSTER_CHANGED: &str = "roster:changed";
pub const EVENT_INBOX_CHANGED: &str = "inbox:changed";
pub const EVENT_BOARD_CHANGED: &str = "board:changed";
pub const EVENT_CAPTURE_CHANGED: &str = "capture:changed";
pub const EVENT_DEVICES_PAIRING: &str = "devices:pairing";
pub const EVENT_APP_OPEN: &str = "app:open";

/// The board:changed and boards:changed payload.
#[derive(Debug, Clone, Default, Serialize, Type)]
pub struct Empty {}

/// The capture:changed payload: what changed since the last one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CaptureChange {
    pub kinds: Vec<String>,
    pub keys: Vec<String>,
    pub repo_keys: Vec<String>,
}

fn add_once(list: &mut Vec<String>, v: &str) {
    if !v.is_empty() && !list.iter().any(|x| x == v) {
        list.push(v.to_string());
    }
}

/// The nudge Go's inbox pump takes on its `kick` channel: a policy write
/// that re-queues nothing still emits. It pushes a resync into the pump's
/// feed.
#[derive(Default)]
pub(crate) struct Kick(Mutex<Option<Arc<Queue<CoreEvent>>>>);

impl Kick {
    pub(crate) fn set(&self, q: Arc<Queue<CoreEvent>>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(q);
    }

    pub(crate) fn notify(&self) {
        if let Some(q) = self.0.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            q.push(CoreEvent::Resync);
        }
    }
}

/// Go `CaptureService.run`'s fold: a burst of capture changes as one
/// capture:changed, each kind, key and repo once in first-seen order.
/// `None` when the burst carries nothing to say.
pub fn capture_change(batch: &[CoreEvent]) -> Option<CaptureChange> {
    let mut c = CaptureChange::default();
    for ev in batch {
        if let CoreEvent::Capture(ch) = ev {
            if let Ok(serde_json::Value::String(k)) = serde_json::to_value(ch.kind) {
                add_once(&mut c.kinds, &k);
            }
            add_once(&mut c.keys, &ch.key);
            add_once(&mut c.repo_keys, &ch.repo_key);
        }
    }
    (!(c.kinds.is_empty() && c.keys.is_empty() && c.repo_keys.is_empty())).then_some(c)
}

/// Starts the pumps and forwards pairing changes; the pumps stop when
/// `stop` is canceled or the Core closes its feeds.
pub fn start(w: Arc<Window>, stop: Cancel) -> Vec<JoinHandle<()>> {
    let mut out = Vec::new();
    {
        // Go DevicesService.attach: Pairing().SetOnChange -> devices:pairing.
        let ww = Arc::downgrade(&w);
        w.seams.pairing.watch(Some(Arc::new(move |st| {
            if let Some(w) = ww.upgrade() {
                w.emit(EVENT_DEVICES_PAIRING, &st);
            }
        })));
    }
    {
        let (w, stop) = (w.clone(), stop.clone());
        let feed = w.core.feed(&[Topic::Presence]);
        out.push(spawn("irori-window-roster", move || {
            let q = feed.queue().clone();
            while !stop.is_canceled() {
                match q.recv_timeout(std::time::Duration::from_millis(200)) {
                    Ok(Some(CoreEvent::Presence(ch))) => {
                        let payload = RosterChange {
                            change: *ch,
                            agents: w.core.mesh_agents(),
                        };
                        w.emit(EVENT_ROSTER_CHANGED, &payload);
                    }
                    Ok(Some(_)) | Err(()) => {}
                    Ok(None) => return,
                }
            }
        }));
    }
    {
        let (w, stop) = (w.clone(), stop.clone());
        let feed = w.core.feed(&[Topic::Inbox]);
        w.kick_inbox(feed.queue());
        out.push(spawn("irori-window-inbox", move || {
            let q = feed.queue().clone();
            coalesce(&q, INBOX_EMIT_EVERY, &stop, |_| {
                let c = w.refresh_inbox().counts;
                w.emit(EVENT_INBOX_CHANGED, &c);
                w.counts_changed(c);
            });
        }));
    }
    {
        let (w, stop) = (w.clone(), stop.clone());
        let feed = w.core.feed(&[Topic::Board]);
        // Every shared board's changes go through the same coalescing, and
        // a board opening, going or renamed is boards:changed (Go
        // pumpBoards).
        {
            let (w, board_q) = (w.clone(), feed.queue().clone());
            let q = w.core.subscribe_boards();
            let reg = {
                let q = q.clone();
                stop.on_cancel(move || q.close())
            };
            out.push(spawn("irori-window-boards", move || {
                let _reg = reg;
                while let Some(ch) = q.recv() {
                    if ch.kind.is_empty() && ch.task_id.is_empty() {
                        w.emit(EVENT_BOARDS_CHANGED, &Empty {});
                    }
                    board_q.push(CoreEvent::Board(ch));
                }
            }));
        }
        out.push(spawn("irori-window-board", move || {
            let q = feed.queue().clone();
            coalesce(&q, BOARD_EMIT_EVERY, &stop, |_| {
                w.emit(EVENT_BOARD_CHANGED, &Empty {});
                w.board_changed();
            });
        }));
    }
    {
        let (w, stop) = (w.clone(), stop.clone());
        let feed = w.core.feed(&[Topic::Capture]);
        out.push(spawn("irori-window-capture", move || {
            let q = feed.queue().clone();
            coalesce(&q, CAPTURE_EMIT_EVERY, &stop, |batch| {
                if let Some(c) = capture_change(&batch) {
                    w.emit(EVENT_CAPTURE_CHANGED, &c);
                }
            });
        }));
    }
    out
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(f)
        .unwrap_or_else(|e| panic!("spawn {name}: {e}"))
}

#[cfg(test)]
mod tests {
    use capture::{Change, ChangeKind};
    use irori_core::CoreEvent;

    use super::capture_change;

    fn ch(kind: ChangeKind, key: &str, repo_key: &str) -> CoreEvent {
        CoreEvent::Capture(Change {
            kind,
            key: key.into(),
            repo_key: repo_key.into(),
        })
    }

    /// Go TestCapturePumpCoalesces: a burst becomes one capture:changed
    /// with each kind, key and repo once, in first-seen order.
    #[test]
    fn capture_burst_folds_into_one_change() {
        let c = capture_change(&[
            ch(ChangeKind::Open, "dev1/a", ""),
            ch(ChangeKind::Open, "dev1/a", ""),
            ch(ChangeKind::Commit, "", "root:abc"),
            ch(ChangeKind::Edit, "dev1/b", "root:abc"),
            CoreEvent::Resync,
        ])
        .unwrap();
        assert_eq!(c.kinds, ["open", "commit", "edit"]);
        assert_eq!(c.keys, ["dev1/a", "dev1/b"]);
        assert_eq!(c.repo_keys, ["root:abc"]);
        assert!(capture_change(&[CoreEvent::Resync]).is_none());
    }
}
