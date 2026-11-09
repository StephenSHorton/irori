//! The window's events (surface.md 2.10): Go's pumps, on the Core's feed.
//!
//! | Event | Payload | Emitted |
//! |---|---|---|
//! | `roster:changed` | [`RosterChange`] | coalesced, 250 ms, only when the roster changed (#1135) |
//! | `inbox:changed` | [`InboxSignal`]: [`InboxCounts`] + `seq, all, deliveries, cards` | coalesced, 250 ms |
//! | `board:changed` | [`CardsChange`] `{seq, all, cards, notes, desk}` | coalesced, 250 ms: a card, note or desk change, a presence, roster or lease change that dirties a row, a queue move, a clock time passing. `cards` holds every row that reads a changed card (the grid's summary cache, card #1147) |
//! | `capture:changed` | [`CaptureChange`] | coalesced, 300 ms |
//! | `boards:changed` | [`BoardsSignal`] `{seq}` | a shared board opened, went or was renamed, or its pin moved |
//! | `boards:invite` | `InviteStatus` | every change of the open invite |
//! | `devices:pairing` | [`PairStatus`] | pairing change (M7) |
//! | `devices:changed` | [`DevicesSignal`] `{seq, all: true}` | a device renamed, here or synced (#184) |
//! | `talk:changed` | [`TalkSignal`] `{seq, all, convs, calls}` | coalesced, 250 ms: a `talk.*` row folded, or a message sent (#389) |
//! | `talk:replying` | `TalkReplying[]` | coalesced, 250 ms, only when it changed (#645) |
//! | `assistant:home` | `AssistantHomeStatus` | coalesced, 250 ms: the recorded assistant's computer changed (one-assistant card 1) |
//! | `notifications:changed` | [`NotifSignal`] `{seq, all, keys, asks}` | coalesced, 250 ms: a notification kept or read, here or synced (#377), or an ask changed |
//! | `assistant:changed` | [`AssistantSignal`] `{seq, all}` | coalesced, 250 ms: a row of the assistant's conversation folded, here or synced (#1102) |
//! | `settings:launch-at-login` | bool | after every toggle attempt |
//! | `app:open` | [`Target`] | a card's Open (M6 toast) |
//! | `update:status` | [`UpdateStatus`] | updater transitions (M8) |
//! | `voice:event` | `VoiceEvent` | a voice input session's words and end (#240, voice.rs) |
//! | `voice:download` | `VoiceDownload` | the speech model's one-time download, about every 1% (#240) |
//! | `meters:changed` | `MetersSample` | this computer's hardware meters, every 2 s, desktop only (#599, meters.rs) |
//! | `runs:changed` | `RunLoad[]` | every 2 s while a run of this device goes, the full set each time; once `[]` after the last (#1249, runs.rs) |
//! | `usage:changed` | `UsageLimits` | coalesced, 250 ms: a session reported Claude usage (#599, usage.rs) |
//! | `sync:changed` | `bool` | on change: this device is exchanging rows or blobs with a peer, held 1 s (#1089, sync_badge.rs) |
//! | `call:state` | `CallView` | coalesced, 100 ms: every huddle change, and its health and speaking during one (#654, call.rs) |
//!
//! The six list signals (`board`, `talk`, `notifications`, `inbox`,
//! `devices`, `boards`, `assistant`) are minted by [`Window::emit_change`]:
//! each carries a `seq`, counted per name under one mutex with the emit,
//! and `all: true` means "read everything". They say which ids changed
//! (list-signals.md §3.3, §4); [`crate::signals`] has the envelopes, the
//! kind table and the expansion.
//!
//! `roster:changed` follows this device's presence only, as Go's
//! (`Core.Changes` is `presence.Subscribe`).

use std::sync::atomic::{AtomicBool, Ordering};
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
use crate::signals::{
    self, AssistantSignal, BoardsSignal, CardsChange, DevicesSignal, Expander, InboxSignal,
    MAX_IDS, NotifSignal, TalkSignal,
};

pub use crate::assistant_chat::EVENT_ASSISTANT_CHANGED;
pub use crate::assistant_home::EVENT_ASSISTANT_HOME;
pub use crate::boards::EVENT_BOARDS_CHANGED;
pub use crate::call::EVENT_CALL_STATE;
pub use crate::meters::EVENT_METERS_CHANGED;
pub use crate::notifications::EVENT_NOTIFICATIONS_CHANGED;
pub use crate::runs::EVENT_RUNS_CHANGED;
pub use crate::settings::EVENT_LAUNCH_AT_LOGIN;
pub use crate::sync_badge::EVENT_SYNC_CHANGED;
pub use crate::update::EVENT_UPDATE_STATUS;
pub use crate::usage::EVENT_USAGE_CHANGED;

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

/// How often roster:changed goes at most (card #1135).
const ROSTER_EMIT_EVERY: Duration = Duration::from_millis(250);

/// How often call:state goes at most (#654): a call's health and
/// speaking change about twice a second.
const CALL_EMIT_EVERY: Duration = Duration::from_millis(100);

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
/// feed, which wakes the pump, and sets a flag the pump takes each batch,
/// so a kick still counts when the feed was full (list-signals.md §3.1).
#[derive(Default)]
pub(crate) struct Kick {
    q: Mutex<Option<Arc<Queue<CoreEvent>>>>,
    kicked: AtomicBool,
}

impl Kick {
    pub(crate) fn set(&self, q: Arc<Queue<CoreEvent>>) {
        *self.q.lock().unwrap_or_else(|e| e.into_inner()) = Some(q);
    }

    pub(crate) fn notify(&self) {
        self.kicked.store(true, Ordering::SeqCst);
        if let Some(q) = self.q.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            q.push(CoreEvent::Resync);
        }
    }

    /// Whether a kick came since the last take: the pump's batch reads
    /// everything then.
    pub(crate) fn take(&self) -> bool {
        self.kicked.swap(false, Ordering::SeqCst)
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
            // Coalesced like the others (card #1135): a busy agent's every
            // line is a presence change. The last change of a burst rides
            // with the roster after it, and an unchanged roster is dropped.
            let mut last: Option<serde_json::Value> = None;
            coalesce(&q, ROSTER_EMIT_EVERY, &stop, |batch| {
                let Some(ch) = batch.into_iter().rev().find_map(|ev| match ev {
                    CoreEvent::Presence(ch) => Some(ch),
                    _ => None,
                }) else {
                    return;
                };
                let payload = RosterChange {
                    change: *ch,
                    agents: w.agents(),
                };
                // The window draws from the roster alone; the change is
                // context. Compare the roster, not the change.
                let Ok(agents) = serde_json::to_value(&payload.agents) else {
                    w.emit(EVENT_ROSTER_CHANGED, &payload);
                    return;
                };
                if last.as_ref() == Some(&agents) {
                    return;
                }
                last = Some(agents);
                w.emit(EVENT_ROSTER_CHANGED, &payload);
            });
        }));
    }
    {
        // Wind down's switch can move on another device (card #787): read
        // it now and then, telling the window when it moved. Its own
        // thread, because the coalesced roster pump only wakes on events.
        let (w, stop) = (w.clone(), stop.clone());
        out.push(spawn("irori-window-wind-down", move || {
            let mut at = std::time::Instant::now();
            while !stop.is_canceled() {
                std::thread::sleep(Duration::from_millis(200));
                if at.elapsed() >= WIND_DOWN_EVERY {
                    at = std::time::Instant::now();
                    w.wind_down_tick();
                }
            }
        }));
    }
    {
        let (w, stop) = (w.clone(), stop.clone());
        let feed = feed_of(&w, &[Topic::Inbox]);
        w.kick_inbox(feed.queue());
        out.push(spawn("irori-window-inbox", move || {
            let q = feed.queue().clone();
            coalesce(&q, INBOX_EMIT_EVERY, &stop, |batch| {
                // A kick means everything (list-signals.md §3.1).
                let (mut all, mut deliveries) = signals::fold_inbox(&batch, w.inbox_kicked());
                let mut cards = w.deliveries_cards(&deliveries);
                if cards.len() > MAX_IDS || deliveries.len() > MAX_IDS {
                    all = true;
                }
                if all {
                    deliveries.clear();
                    cards.clear();
                }
                let c = w.refresh_inbox().counts;
                w.emit_change(EVENT_INBOX_CHANGED, |seq| InboxSignal {
                    counts: c,
                    seq,
                    all,
                    deliveries,
                    cards,
                });
                w.counts_changed(c);
                w.wind_down_tick();
            });
        }));
    }
    {
        let (w, stop) = (w.clone(), stop.clone());
        // Presence and the roster ride along (they move the cards' agent
        // lines) and so does the inbox (a queued assignment notice shows
        // on its card as waiting): list-signals.md §3.3.
        let feed = feed_of(
            &w,
            &[
                Topic::Board,
                Topic::Presence,
                Topic::Roster,
                Topic::Inbox,
                Topic::Runs,
            ],
        );
        // From here every change reaches this pump, which keeps the grid's
        // summary cache (card #1147): reads may use it until it stops.
        w.cache_live(true);
        let live_reg = {
            let w = Arc::downgrade(&w);
            stop.on_cancel(move || {
                if let Some(w) = w.upgrade() {
                    w.cache_live(false);
                }
            })
        };
        // Every board's card changes, shared or private, come on the Board
        // feed, and so does a board opening, going or renamed, as a resync
        // (list-signals.md §3.1). That is also boards:changed (Go
        // pumpBoards).
        {
            let w = w.clone();
            let q = w.core.subscribe_boards();
            // boards:changed is coalesced too (card #233): a board synced
            // from another device moves once per board.* / member.* row.
            let moved_q: Arc<Queue<CoreEvent>> = Arc::new(Queue::new(feed_cap_or(64)));
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
                        w.emit_change(EVENT_BOARDS_CHANGED, |seq| BoardsSignal { seq });
                    });
                }));
            }
            out.push(spawn("irori-window-boards", move || {
                let _reg = reg;
                // Only zero Changes come here now; a lost one is a zero
                // Change too (Boards::tell), and boards:changed always
                // reads the whole list, so a full moved_q loses nothing.
                while q.recv().is_some() {
                    moved_q.push(CoreEvent::Resync);
                }
            }));
        }
        let ex = Arc::new(Mutex::new(Expander::default()));
        {
            // Rows that change by the clock with no event: a talking line
            // ending, a card going stale, a spawn's grace (§3.3 step 5).
            let (w, stop, ex) = (w.clone(), stop.clone(), ex.clone());
            out.push(spawn("irori-window-due", move || {
                let mut at = std::time::Instant::now();
                while !stop.is_canceled() {
                    std::thread::sleep(Duration::from_millis(200));
                    if at.elapsed() < DUE_EVERY {
                        continue;
                    }
                    at = std::time::Instant::now();
                    let ids = w.due_cards(&mut ex.lock().unwrap_or_else(|e| e.into_inner()));
                    if !ids.is_empty() {
                        w.emit_change(EVENT_BOARD_CHANGED, |seq| CardsChange {
                            seq,
                            cards: ids,
                            ..CardsChange::default()
                        });
                    }
                }
            }));
        }
        out.push(spawn("irori-window-board", move || {
            let _live = live_reg;
            let q = feed.queue().clone();
            coalesce(&q, BOARD_EMIT_EVERY, &stop, |batch| {
                let mut f = signals::fold_board(&batch);
                // A board's sidebar pin from another of the person's devices
                // (card #233): the boards list holds the pins.
                if f.pin {
                    w.emit_change(EVENT_BOARDS_CHANGED, |seq| BoardsSignal { seq });
                }
                let change = w.expand(&mut ex.lock().unwrap_or_else(|e| e.into_inner()), &mut f);
                let sent = change.is_some();
                if let Some(c) = change {
                    w.emit_change(EVENT_BOARD_CHANGED, |seq| CardsChange { seq, ..c });
                }
                // The tray's count follows every board or inbox change and
                // an agent going away (a wait on it is the person's), not
                // each line an agent prints.
                if sent || f.board_or_inbox || f.gone {
                    w.board_changed();
                }
            });
        }));
    }
    {
        // A device renamed (card #184): the header, the cards' "on <device>"
        // lines and the Devices list read the names again.
        let (w, stop) = (w.clone(), stop.clone());
        let feed = feed_of(&w, &[Topic::Devices]);
        out.push(spawn("irori-window-devices", move || {
            let q = feed.queue().clone();
            coalesce(&q, BOARD_EMIT_EVERY, &stop, |_| {
                // A device's sides change what its cards wait on with no
                // card change: the grid's cache starts again (card #1147).
                w.cache_clear();
                w.emit_change(EVENT_DEVICES_CHANGED, |seq| DevicesSignal {
                    seq,
                    all: true,
                });
            });
        }));
    }
    {
        // Talk (card #389): a channel, DM, post or read cursor changed,
        // here or synced, or an agent's message may show as a DM.
        let (w, stop) = (w.clone(), stop.clone());
        let feed = feed_of(&w, &[Topic::Talk]);
        out.push(spawn("irori-window-talk", move || {
            let q = feed.queue().clone();
            coalesce(&q, BOARD_EMIT_EVERY, &stop, |batch| {
                let (all, convs, calls) = signals::fold_talk(&batch);
                w.emit_change(EVENT_TALK_CHANGED, |seq| TalkSignal {
                    seq,
                    all,
                    convs,
                    calls,
                });
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
        // The asks ride along: a row shows its ask's state.
        let feed = feed_of(&w, &[Topic::Notifications, Topic::Asks]);
        out.push(spawn("irori-window-notifications", move || {
            let q = feed.queue().clone();
            coalesce(&q, BOARD_EMIT_EVERY, &stop, |batch| {
                let (all, keys, asks) = signals::fold_notifications(&batch);
                w.emit_change(EVENT_NOTIFICATIONS_CHANGED, |seq| NotifSignal {
                    seq,
                    all,
                    keys,
                    asks,
                });
            });
        }));
    }
    {
        // The assistant's conversation (card #1102): the chat reads its
        // list again. Only that it changed, never a reply's text.
        let (w, stop) = (w.clone(), stop.clone());
        let feed = feed_of(&w, &[Topic::AssistantChat]);
        out.push(spawn("irori-window-assistant-chat", move || {
            let q = feed.queue().clone();
            coalesce(&q, BOARD_EMIT_EVERY, &stop, |batch| {
                let all = batch.iter().any(|ev| matches!(ev, CoreEvent::Resync));
                w.emit_change(EVENT_ASSISTANT_CHANGED, |seq| AssistantSignal { seq, all });
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
        // The recorded assistant's computer changed (one-assistant card 1).
        let (w, stop) = (w.clone(), stop.clone());
        let feed = w.core.feed(&[Topic::AssistantHome]);
        out.push(spawn("irori-window-assistant-home", move || {
            let q = feed.queue().clone();
            coalesce(&q, BOARD_EMIT_EVERY, &stop, |_| {
                w.emit(EVENT_ASSISTANT_HOME, &w.assistant_home_status());
            });
        }));
    }
    // The hardware meters (card #599): desktop only.
    out.extend(crate::meters::start(w.clone(), stop.clone()));
    out.extend(crate::runs::start(w.clone(), stop.clone()));
    // The header's sync badge (card #1089).
    out.extend(crate::sync_badge::start(w.clone(), stop.clone()));
    out
}

/// A Core feed of `topics`. A debug build can shrink its queue
/// (`IRORI_DEV_FEED_QUEUE=N`) to make it overflow on purpose (§8).
fn feed_of(w: &Window, topics: &[Topic]) -> irori_core::Feed {
    match signals::dev_switch(signals::DEV_FEED_QUEUE_ENV) {
        0 => w.core.feed(topics),
        n => w.core.feed_sized(topics, n),
    }
}

/// The slots a window-local queue gets: `default`, or the debug switch's.
fn feed_cap_or(default: usize) -> usize {
    match signals::dev_switch(signals::DEV_FEED_QUEUE_ENV) {
        0 => default,
        n => n,
    }
}

/// How often the due timer looks at the clock.
const DUE_EVERY: Duration = Duration::from_secs(1);

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

    /// A kick still counts when the inbox pump's feed is full (card #1144,
    /// list-signals.md §3.1): the second push is dropped, but the flag the
    /// pump takes each batch says a kick came, once.
    #[test]
    fn kick_counts_when_the_feed_is_full() {
        use std::sync::Arc;

        use gocompat::sub::Queue;

        use super::Kick;

        let k = Kick::default();
        assert!(!k.take(), "no kick yet");
        let q = Arc::new(Queue::new(1));
        k.set(q.clone());
        k.notify();
        k.notify();
        assert_eq!(q.len(), 1, "the second push is dropped");
        assert!(k.take(), "the kick is taken");
        assert!(!k.take(), "taken once");
    }
}
