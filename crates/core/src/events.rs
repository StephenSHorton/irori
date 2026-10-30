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
            CoreEvent::Resync => return None,
        })
    }
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
