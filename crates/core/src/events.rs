//! The Core's change feed: what the API and the window subscribe to
//! (surface.md §2.7, §2.10).
//!
//! Every service already has its own subscription with Go's buffer rules;
//! the Core runs one pump per service that republishes into one [`Hub`], so
//! a reader takes a single [`Feed`] instead of six. A feed holds
//! [`FEED_BUFFER`] events; a full one drops the event and the reader gets
//! [`CoreEvent::Resync`] in place of the next one that fits (coord's rule),
//! so a slow window re-reads instead of blocking a service.
//!
//! Log rows are not on the hub: `GET /v1/events/stream` needs the store's
//! own subscription (256 slots, the last one reserved for resync,
//! store.md §3.9), so [`crate::Core::subscribe_rows`] hands that out as is.
//!
//! [`coalesce`] is the window's pump (Go `pumpBoard`, `InboxService.pump`,
//! `CaptureService.run`): a burst becomes one emit at most `every` after
//! the previous one, carrying everything that arrived meanwhile.

use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use gocompat::sub::{Queue, TryRecv};

/// One feed's buffer.
pub const FEED_BUFFER: usize = 64;

/// Window coalescing (board_service.go, inbox_service.go,
/// capture_service.go).
pub const BOARD_EMIT_EVERY: Duration = Duration::from_millis(250);
pub const INBOX_EMIT_EVERY: Duration = Duration::from_millis(250);
pub const CAPTURE_EMIT_EVERY: Duration = Duration::from_millis(300);

/// Which service an event came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    /// This device's presence (`roster:changed`).
    Presence,
    /// Another device's agent cards (the mesh roster).
    Roster,
    Inbox,
    Board,
    Capture,
    Asks,
    /// The person renamed one of their devices (`devices:changed`).
    Devices,
    /// Talk's channels, DMs or posts changed (`talk:changed`, card #389).
    Talk,
    /// The notification history changed (card #377).
    Notifications,
    /// The assistant's conversation changed (`assistant:changed`,
    /// card #1102).
    AssistantChat,
    /// Claude usage left changed (`usage:changed`, card #599).
    Usage,
    /// A call changed: ringing, joined, left, mute, speaking, health
    /// (card #654). `Core::call_snapshot` says what is on now.
    Call,
    /// The Office Assistant's computer changed (`assistant:home`, one-assistant
    /// card 1).
    AssistantHome,
}

/// One change.
#[derive(Debug, Clone, PartialEq)]
pub enum CoreEvent {
    Presence(Box<presence::Change>),
    Roster,
    Inbox(inbox::Change),
    Board(coord::Change),
    Capture(capture::Change),
    /// An ask's id.
    Asks(String),
    /// What a device is called changed (card #184), here or on another of
    /// the person's devices.
    Devices,
    /// A `talk.*` or `call.*` row folded, here or synced, or an agent's
    /// message showed as a DM (card #389). It names what changed
    /// (list-signals.md §3.2).
    Talk(TalkChange),
    /// A notification was kept or read (card #377), here or synced, by
    /// key.
    Notifications(NotifChange),
    /// A row of the assistant's conversation folded, here or synced
    /// (card #1102). It says only that the fold changed, never a reply's
    /// text.
    AssistantChat,
    /// A session's reading of the plan's rate-limit windows was taken
    /// (card #599): on every one, as its observed time moves even when
    /// the figures don't.
    Usage,
    /// A call changed (card #654).
    Call,
    /// The recorded assistant's computer changed, here or synced.
    AssistantHome,
    /// Events were dropped for this feed: re-read everything it shows.
    Resync,
}

impl CoreEvent {
    pub fn topic(&self) -> Option<Topic> {
        Some(match self {
            CoreEvent::Presence(_) => Topic::Presence,
            CoreEvent::Roster => Topic::Roster,
            CoreEvent::Inbox(_) => Topic::Inbox,
            CoreEvent::Board(_) => Topic::Board,
            CoreEvent::Capture(_) => Topic::Capture,
            CoreEvent::Asks(_) => Topic::Asks,
            CoreEvent::Devices => Topic::Devices,
            CoreEvent::Talk(_) => Topic::Talk,
            CoreEvent::Notifications(_) => Topic::Notifications,
            CoreEvent::AssistantChat => Topic::AssistantChat,
            CoreEvent::Usage => Topic::Usage,
            CoreEvent::Call => Topic::Call,
            CoreEvent::AssistantHome => Topic::AssistantHome,
            CoreEvent::Resync => return None,
        })
    }
}

/// What a [`CoreEvent::Talk`] changed (list-signals.md §3.2): the
/// conversations (a channel, a DM, a thread's read cursor) and the calls it
/// touched, or `all`. There is no empty default: a change that names
/// nothing means "read everything" to its reader, so a site that forgets
/// its ids costs a full read, never a stale row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TalkChange {
    pub convs: Vec<String>,
    pub calls: Vec<String>,
    pub all: bool,
}

impl TalkChange {
    /// Everything Talk shows may have changed.
    pub fn all() -> TalkChange {
        TalkChange {
            convs: Vec::new(),
            calls: Vec::new(),
            all: true,
        }
    }

    /// These conversations changed (empty ids are left out).
    pub fn convs<S: Into<String>>(convs: impl IntoIterator<Item = S>) -> TalkChange {
        TalkChange::of(convs, std::iter::empty::<String>())
    }

    /// These calls changed (empty ids are left out).
    pub fn calls<S: Into<String>>(calls: impl IntoIterator<Item = S>) -> TalkChange {
        TalkChange::of(std::iter::empty::<String>(), calls)
    }

    /// These conversations and calls changed (empty ids are left out).
    pub fn of<A: Into<String>, B: Into<String>>(
        convs: impl IntoIterator<Item = A>,
        calls: impl IntoIterator<Item = B>,
    ) -> TalkChange {
        TalkChange {
            convs: ids(convs),
            calls: ids(calls),
            all: false,
        }
    }

    /// Whether a reader must read everything: `all`, or no ids at all.
    pub fn is_all(&self) -> bool {
        self.all || (self.convs.is_empty() && self.calls.is_empty())
    }

    /// What one folded `talk.*` or `call.*` row touched (coord's
    /// `talk::KINDS`, or a space's row after `space_talk_row`, which keeps
    /// the conv and call ids). A thread's read cursor keeps its conv id as
    /// written (`thread:<task id>` from the window). `talk.card` names only
    /// a post, so it is `all`, as is a row that names nothing.
    pub fn of_row(ev: &store::Event) -> TalkChange {
        use store::kinds as k;
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&ev.payload) else {
            return TalkChange::all();
        };
        let s = |p: &str| {
            v.pointer(p)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let t = match ev.kind.as_str() {
            k::TALK_CHANNEL_CREATED | k::TALK_CHANNEL_EDITED => {
                TalkChange::convs([s("/channelId")])
            }
            k::TALK_MEMBER | k::TALK_READ => TalkChange::convs([s("/convId")]),
            // A huddle's start or end mark, or a transcript fallback post.
            k::TALK_POST => {
                TalkChange::of([s("/convId")], [s("/call/callId"), s("/transcript/callId")])
            }
            k::CALL_SEGMENT | k::CALL_TITLE | k::CALL_ASSIST | k::CALL_ITEM | k::CALL_ITEM_SET => {
                TalkChange::of([s("/convId")], [s("/callId")])
            }
            _ => return TalkChange::all(),
        };
        if t.is_all() { TalkChange::all() } else { t }
    }
}

/// What a [`CoreEvent::Notifications`] changed: the notification keys, or
/// `all`. As [`TalkChange`], no empty default, and empty means all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifChange {
    pub keys: Vec<String>,
    pub all: bool,
}

impl NotifChange {
    /// Every notification may have changed (a store resync).
    pub fn all() -> NotifChange {
        NotifChange {
            keys: Vec::new(),
            all: true,
        }
    }

    /// These notifications changed (empty keys are left out).
    pub fn keys<S: Into<String>>(keys: impl IntoIterator<Item = S>) -> NotifChange {
        NotifChange {
            keys: ids(keys),
            all: false,
        }
    }

    /// Whether a reader must read everything: `all`, or no keys.
    pub fn is_all(&self) -> bool {
        self.all || self.keys.is_empty()
    }
}

/// Non-empty ids, each once, in first-seen order.
fn ids<S: Into<String>>(it: impl IntoIterator<Item = S>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in it {
        let s = s.into();
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

struct Sub {
    q: Weak<Queue<CoreEvent>>,
    topics: Vec<Topic>,
    lost: bool,
}

/// Fans the services' changes out to every [`Feed`].
#[derive(Default)]
pub struct Hub {
    subs: Mutex<Vec<Sub>>,
}

impl Hub {
    pub fn new() -> Hub {
        Hub::default()
    }

    /// A feed of `topics` (empty: every topic). Dropping it unsubscribes.
    pub fn subscribe(&self, topics: &[Topic]) -> Feed {
        let q = Arc::new(Queue::new(FEED_BUFFER));
        self.lock().push(Sub {
            q: Arc::downgrade(&q),
            topics: topics.to_vec(),
            lost: false,
        });
        Feed { q }
    }

    /// Hands `ev` to every feed that wants it, never blocking.
    pub fn publish(&self, ev: CoreEvent) {
        let topic = ev.topic();
        let mut subs = self.lock();
        subs.retain(|s| s.q.strong_count() > 0);
        for s in subs.iter_mut() {
            if !(s.topics.is_empty() || topic.is_some_and(|t| s.topics.contains(&t))) {
                continue;
            }
            let Some(q) = s.q.upgrade() else { continue };
            let msg = if s.lost {
                CoreEvent::Resync
            } else {
                ev.clone()
            };
            s.lost = !q.push(msg);
        }
    }

    /// Open feeds.
    pub fn len(&self) -> usize {
        let mut subs = self.lock();
        subs.retain(|s| s.q.strong_count() > 0);
        subs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Closes every feed (the Core stopped): readers drain and see the end.
    pub fn close(&self) {
        for s in self.lock().drain(..) {
            if let Some(q) = s.q.upgrade() {
                q.close();
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Sub>> {
        self.subs.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// One reader's events.
pub struct Feed {
    q: Arc<Queue<CoreEvent>>,
}

impl Feed {
    /// The next event; `None` once the Core stopped and the feed drained.
    pub fn recv(&self) -> Option<CoreEvent> {
        self.q.recv()
    }

    /// The next event within `d`; `Err` on timeout.
    #[allow(clippy::result_unit_err)]
    pub fn recv_timeout(&self, d: Duration) -> Result<Option<CoreEvent>, ()> {
        self.q.recv_timeout(d)
    }

    /// The next event if one is buffered.
    pub fn try_recv(&self) -> Option<CoreEvent> {
        match self.q.try_recv() {
            TryRecv::Item(e) => Some(e),
            _ => None,
        }
    }

    /// For async readers (`recv_async`).
    pub fn queue(&self) -> &Arc<Queue<CoreEvent>> {
        &self.q
    }

    pub fn close(&self) {
        self.q.close();
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        self.q.close();
    }
}

/// Go's window pumps: waits for changes on `q`; the first one of a burst
/// schedules an emit `every` after the previous emit (at once if that long
/// has passed), and the emit carries every change that arrived by then.
/// Returns when `q` closes or `stop` is canceled.
pub fn coalesce<T>(
    q: &Queue<T>,
    every: Duration,
    stop: &sys::cancel::Cancel,
    mut emit: impl FnMut(Vec<T>),
) {
    let mut last: Option<Instant> = None;
    let mut due: Option<Instant> = None;
    let mut batch = Vec::new();
    let tick = Duration::from_millis(50).min(every.max(Duration::from_millis(1)));
    loop {
        if stop.is_canceled() {
            return;
        }
        let wait = match due {
            Some(at) => at.saturating_duration_since(Instant::now()),
            None => tick,
        };
        match q.recv_timeout(wait.min(tick)) {
            Ok(Some(x)) => {
                batch.push(x);
                if due.is_none() {
                    let since = last.map(|l| l.elapsed()).unwrap_or(every);
                    due = Some(Instant::now() + every.saturating_sub(since));
                }
            }
            Ok(None) => return,
            Err(()) => {}
        }
        if let Some(at) = due
            && Instant::now() >= at
        {
            // What already arrived rides along.
            while let TryRecv::Item(x) = q.try_recv() {
                batch.push(x);
            }
            due = None;
            last = Some(Instant::now());
            emit(std::mem::take(&mut batch));
        }
    }
}
