//! The window's events (surface.md 2.10): Go's pumps, on the Core's feed.
//!
//! | Event | Payload | Emitted |
//! |---|---|---|
//! | `roster:changed` | [`RosterChange`] | every presence change |
//! | `inbox:changed` | [`InboxCounts`] | coalesced, 250 ms |
//! | `board:changed` | `{}` | coalesced, 250 ms |
//! | `capture:changed` | [`CaptureChange`] | coalesced, 300 ms |
//! | `boards:changed` | `{}` | a shared board opened, went or was renamed |
//! | `boards:invite` | `InviteStatus` | every change of the open invite |
//! | `devices:pairing` | [`PairStatus`] | pairing change (M7) |
//! | `devices:changed` | `{}` | a device renamed, here or synced (#184) |
//! | `talk:changed` | `{}` | coalesced, 250 ms: a `talk.*` row folded, or a message sent (#389) |
//! | `talk:replying` | `TalkReplying[]` | coalesced, 250 ms, only when it changed (#645) |
//! | `notifications:changed` | `{}` | a notification kept or read, here or synced (#377) |
//! | `settings:launch-at-login` | bool | after every toggle attempt |
//! | `app:open` | [`Target`] | a card's Open (M6 toast) |
//! | `update:status` | [`UpdateStatus`] | updater transitions (M8) |
//! | `voice:event` | `VoiceEvent` | a voice input session's words and end (#240, voice.rs) |
//! | `voice:download` | `VoiceDownload` | the speech model's one-time download, about every 1% (#240) |
//! | `meters:changed` | `MetersSample` | this computer's hardware meters, every 2 s, desktop only (#599, meters.rs) |
//! | `usage:changed` | `UsageLimits` | coalesced, 250 ms: a session reported Claude usage (#599, usage.rs) |
//! | `assistant:away` | `{}` | a coming-back summary is ready to take (#786) |
//! | `call:state` | `CallView` | coalesced, 100 ms: every huddle change, and its health and speaking during one (#654, call.rs) |
//!
//! `roster:changed` follows this device's presence only, as Go's
//! (`Core.Changes` is `presence.Subscribe`).

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use gocompat::sub::Queue;
use irori_core::events::{BOARD_EMIT_EVERY, CAPTURE_EMIT_EVERY, INBOX_EMIT_EVERY, coalesce};
use irori_core::{CoreEvent, Topic};
use serde::Serialize;
use specta::Type;
use sys::cancel::Cancel;

use crate::Window;
use crate::roster::RosterChange;

pub use crate::boards::EVENT_BOARDS_CHANGED;
pub use crate::call::EVENT_CALL_STATE;
pub use crate::meters::EVENT_METERS_CHANGED;
pub use crate::notifications::EVENT_NOTIFICATIONS_CHANGED;
pub use crate::settings::EVENT_LAUNCH_AT_LOGIN;
pub use crate::update::EVENT_UPDATE_STATUS;
pub use crate::usage::EVENT_USAGE_CHANGED;

/// A coming-back summary is ready: the window takes it (card #786).
pub const EVENT_ASSISTANT_AWAY: &str = "assistant:away";

pub const EVENT_ROSTER_CHANGED: &str = "roster:changed";
pub const EVENT_INBOX_CHANGED: &str = "inbox:changed";
pub const EVENT_BOARD_CHANGED: &str = "board:changed";
pub const EVENT_CAPTURE_CHANGED: &str = "capture:changed";
pub const EVENT_DEVICES_PAIRING: &str = "devices:pairing";
pub const EVENT_DEVICES_CHANGED: &str = "devices:changed";
pub const EVENT_TALK_CHANGED: &str = "talk:changed";
/// Who is replying in Talk changed (card #645); the payload is the list.
pub const EVENT_TALK_REPLYING: &str = "talk:replying";
pub const EVENT_APP_OPEN: &str = "app:open";
/// How often the window reads Wind down's switch for a change made on
/// another device (card #787).
const WIND_DOWN_EVERY: std::time::Duration = std::time::Duration::from_secs(5);
/// From the page: it listens for app:open now (card #395: a phone
/// notification tapped while Irori was closed opens then).
pub const EVENT_APP_OPEN_READY: &str = "app:open-ready";

/// How often call:state goes at most (#654): a call's health and
/// speaking change about twice a second.
const CALL_EMIT_EVERY: Duration = Duration::from_millis(100);

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
        // Every invite change -> boards:invite.
        let ww = Arc::downgrade(&w);
        w.seams.invites.watch(Some(Arc::new(move |st| {
            if let Some(w) = ww.upgrade() {
                w.emit(crate::invites::EVENT_INVITE_CHANGED, &st);
            }
        })));
    }
    {
        let (w, stop) = (w.clone(), stop.clone());
        let feed = w.core.feed(&[Topic::Presence]);
        out.push(spawn("irori-window-roster", move || {
            let q = feed.queue().clone();
            // Wind down's switch can move on another device (card #787):
            // read it now and then, telling the window when it moved.
            let mut wind_down_at = std::time::Instant::now();
            while !stop.is_canceled() {
                if wind_down_at.elapsed() >= WIND_DOWN_EVERY {
                    wind_down_at = std::time::Instant::now();
                    w.wind_down_tick();
                }
                match q.recv_timeout(std::time::Duration::from_millis(200)) {
                    Ok(Some(CoreEvent::Presence(ch))) => {
                        let payload = RosterChange {
                            change: *ch,
                            agents: w.agents(),
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
                w.wind_down_tick();
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
            // boards:changed is coalesced too (card #233): a board synced
            // from another device moves once per board.* / member.* row.
            let moved_q: Arc<Queue<CoreEvent>> = Arc::new(Queue::new(64));
            let reg = {
                let (q, moved_q) = (q.clone(), moved_q.clone());
                stop.on_cancel(move || {
                    q.close();
                    moved_q.close();
                })
            };
            {
                let (w, stop, moved_q) = (w.clone(), stop.clone(), moved_q.clone());
                out.push(spawn("irori-window-boards-moved", move || {
                    coalesce(&moved_q, BOARD_EMIT_EVERY, &stop, |_| {
                        w.emit(EVENT_BOARDS_CHANGED, &Empty {});
                    });
                }));
            }
            out.push(spawn("irori-window-boards", move || {
                let _reg = reg;
                while let Some(ch) = q.recv() {
                    if ch.kind.is_empty() && ch.task_id.is_empty() {
                        moved_q.push(CoreEvent::Resync);
                    }
                    board_q.push(CoreEvent::Board(ch));
                }
            }));
        }
        out.push(spawn("irori-window-board", move || {
            let q = feed.queue().clone();
            coalesce(&q, BOARD_EMIT_EVERY, &stop, |batch| {
                // A board's sidebar pin from another of the person's devices
                // (card #233): the boards list holds the pins.
                if batch.iter().any(is_board_pin) {
                    w.emit(EVENT_BOARDS_CHANGED, &Empty {});
                }
                w.emit(EVENT_BOARD_CHANGED, &Empty {});
                w.board_changed();
            });
        }));
    }
    {
        // A device renamed (card #184): the header, the cards' "on <device>"
        // lines and the Devices list read the names again.
        let (w, stop) = (w.clone(), stop.clone());
        let feed = w.core.feed(&[Topic::Devices]);
        out.push(spawn("irori-window-devices", move || {
            let q = feed.queue().clone();
            coalesce(&q, BOARD_EMIT_EVERY, &stop, |_| {
                w.emit(EVENT_DEVICES_CHANGED, &Empty {});
            });
        }));
    }
    {
        // Talk (card #389): a channel, DM, post or read cursor changed,
        // here or synced, or an agent's message may show as a DM.
        let (w, stop) = (w.clone(), stop.clone());
        let feed = w.core.feed(&[Topic::Talk]);
        out.push(spawn("irori-window-talk", move || {
            let q = feed.queue().clone();
            coalesce(&q, BOARD_EMIT_EVERY, &stop, |_| {
                w.emit(EVENT_TALK_CHANGED, &Empty {});
            });
        }));
    }
    {
        // Talk's replying row (card #645): a wake, a turn starting or
        // ending, a post, or an agent going busy, gone or offline, here or
        // synced, may change who is replying. Sent only when it did.
        let (w, stop) = (w.clone(), stop.clone());
        let feed = w.core.feed(&[
            Topic::Inbox,
            Topic::Talk,
            Topic::Presence,
            Topic::Roster,
            Topic::Devices,
        ]);
        out.push(spawn("irori-window-talk-replying", move || {
            let q = feed.queue().clone();
            let mut last = w.talk_replying();
            coalesce(&q, INBOX_EMIT_EVERY, &stop, |_| {
                let now = w.talk_replying();
                if now != last {
                    w.emit(EVENT_TALK_REPLYING, &now);
                    last = now;
                }
            });
        }));
    }
    {
        // The notification history (card #377): the page and the sidebar's
        // unread count read it again.
        let (w, stop) = (w.clone(), stop.clone());
        let feed = w.core.feed(&[Topic::Notifications]);
        out.push(spawn("irori-window-notifications", move || {
            let q = feed.queue().clone();
            coalesce(&q, BOARD_EMIT_EVERY, &stop, |_| {
                w.emit(EVENT_NOTIFICATIONS_CHANGED, &Empty {});
            });
        }));
    }
    {
        // Huddles (#654): the whole picture again on every change.
        let (w, stop) = (w.clone(), stop.clone());
        let feed = w.core.feed(&[Topic::Call]);
        out.push(spawn("irori-window-call", move || {
            let q = feed.queue().clone();
            // During a call the Core publishes about twice a second; send
            // the picture only when it changed.
            let mut last: Option<crate::call::CallView> = None;
            coalesce(&q, CALL_EMIT_EVERY, &stop, |_| {
                let v = w.call_state();
                if last.as_ref() != Some(&v) {
                    w.emit(EVENT_CALL_STATE, &v);
                    last = Some(v);
                }
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
    {
        // Claude usage left (card #599): a session's mod reported the plan's
        // rate-limit windows.
        let (w, stop) = (w.clone(), stop.clone());
        let feed = w.core.feed(&[Topic::Usage]);
        out.push(spawn("irori-window-usage", move || {
            let q = feed.queue().clone();
            coalesce(&q, BOARD_EMIT_EVERY, &stop, |_| {
                w.emit(EVENT_USAGE_CHANGED, &w.usage_limits());
            });
        }));
    }
    {
        // A coming-back summary is ready (card #786).
        let (w, stop) = (w.clone(), stop.clone());
        let feed = w.core.feed(&[Topic::Away]);
        out.push(spawn("irori-window-away", move || {
            let q = feed.queue().clone();
            coalesce(&q, BOARD_EMIT_EVERY, &stop, |_| {
                w.emit(EVENT_ASSISTANT_AWAY, &Empty {});
            });
        }));
    }
    // The hardware meters (card #599): desktop only.
    out.extend(crate::meters::start(w.clone(), stop.clone()));
    out
}

/// Whether a Board event is a shared board's sidebar pin moving (card #233).
fn is_board_pin(ev: &CoreEvent) -> bool {
    matches!(ev, CoreEvent::Board(ch) if ch.kind == coord::BOARD_PIN_CHANGED)
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
