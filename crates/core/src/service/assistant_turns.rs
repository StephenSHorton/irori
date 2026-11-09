//! The assistant's computer answers the conversation's questions
//! (one-assistant card 3, docs/design/one-assistant.md §5).
//!
//! A question is its `assistant.chat.asked` row. Core keeps the queue, the
//! checks and the rows; the desktop's window runs the turn through the
//! hook it installs ([`ChatTurnFn`], `Core::set_chat_turn`).
//!
//! - **The queue** is keyed by the question's store order, so the worker
//!   always takes the oldest. A turn queued or running is never queued
//!   again.
//! - **Two ways in.** Live, from the kind loop as each question's row
//!   arrives (local ones included). And a catch-up scan of the fold: when
//!   the hook is installed, when this device becomes the assistant's
//!   computer, on every resync of the kind loop (rows past its buffer are
//!   otherwise lost to the queue), and from the sweep as a backstop.
//! - **Every check runs at take time.** The worker first waits until this
//!   device is caught up with every own device it is connected to, and
//!   until the kind loop has folded every row the store holds, so it has
//!   seen any newer `assistant.home`, `said`, `stop` or `cleared` row that
//!   came in the same sync. Then: not imported, this is the assistant's
//!   computer, no reply yet, after the newest New chat, not stopped, and
//!   (for a question the scan found) one of the newest five.
//! - **A late question** (it reached this computer more than five minutes
//!   after it was asked, by the asking device's clock corrected for its
//!   skew) gets an age note and no grants.
//! - **Stop and New chat** trip the running turn's cancel at once.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gocompat::time::GoTime;
use store::{Event, Vector};

use super::Services;
use super::assistant_chat::{
    ASKED, Asked, CLEARED, ChatJob, ChatKind, ChatRow, ChatTurnFn, EARLIER_CAP, HISTORY_TURNS,
    Order, STOP, STOPPED, Said, Stop, actor_of,
};
use crate::events::CoreEvent;
use crate::logf;
use crate::timers::Kick;

/// A question that reached this computer more than this long after it was
/// asked is late: it gets an age note and no grants (§5).
pub const LATE_MS: i64 = 5 * 60_000;
/// Of the questions a catch-up scan finds, the newest this many get
/// answers; the rest a `skipped` reply (§6).
pub const SCAN_ANSWERS: usize = 5;
/// How often the worker looks again while it waits to be caught up.
const POLL: Duration = Duration::from_millis(50);
/// The longest it waits to be caught up with a connected device: a writer
/// that never lets it catch up can't hold the assistant forever.
const CAUGHT_UP_WAIT: Duration = Duration::from_secs(120);
/// The longest it waits for the kind loop to fold what the store holds,
/// before it reads the rows again itself.
const HEARD_WAIT: Duration = Duration::from_secs(5);

/// A question waiting for the worker.
#[derive(Debug, Clone)]
struct Pending {
    turn: String,
    /// Queued as its row arrived, not found by a scan.
    live: bool,
    /// Unix ms when its row reached this computer (live) or the scan found
    /// it.
    took_ms: i64,
    /// A scan found it outside the newest five.
    skip: bool,
}

/// The turn the worker is on.
struct Running {
    turn: String,
    /// Its question's place.
    at: Order,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct Queue {
    pending: BTreeMap<Order, Pending>,
    /// Turns queued or running.
    busy: HashSet<String>,
    running: Option<Running>,
}

/// The chat worker's state.
pub(crate) struct ChatQueue {
    hook: Mutex<Option<ChatTurnFn>>,
    q: Mutex<Queue>,
    kick: Arc<Kick>,
    /// The newest seq of each writer the kind loop has folded.
    heard: Mutex<Vector>,
    /// The kind loop's resyncs (tests).
    resyncs: AtomicU64,
}

impl ChatQueue {
    /// `heard`: the store's vector read just before the folds first loaded.
    pub(crate) fn new(heard: Vector) -> ChatQueue {
        ChatQueue {
            hook: Mutex::new(None),
            q: Mutex::new(Queue::default()),
            kick: Kick::new(),
            heard: Mutex::new(heard),
            resyncs: AtomicU64::new(0),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether `have` holds every row `want` names.
fn covers(have: &Vector, want: &Vector) -> bool {
    want.iter()
        .all(|(w, seq)| have.get(w).copied().unwrap_or(0) >= *seq)
}

/// The prompt for one question from rows, the port of the window's
/// `render_prompt` (rows carry no files): the last [`HISTORY_TURNS`] turns
/// as a plain transcript, then the question, then the age note of a late
/// one.
pub(crate) fn chat_prompt(history: &[(bool, String)], text: &str, note: Option<&str>) -> String {
    let skip = history.len().saturating_sub(HISTORY_TURNS);
    let turns: Vec<&(bool, String)> = history
        .iter()
        .skip(skip)
        .filter(|(_, t)| !t.trim().is_empty())
        .collect();
    let mut b = String::new();
    if !turns.is_empty() {
        b.push_str("The conversation so far:\n\n");
        for (mine, t) in turns {
            let who = if *mine { "You" } else { "The person" };
            b.push_str(&format!("{who}: {}\n\n", t.trim()));
        }
        b.push_str("Now answer the person's new message.\n\n");
    }
    b.push_str(&format!("The person: {}\n", text.trim()));
    if let Some(n) = note {
        b.push_str(n);
        b.push('\n');
    }
    b
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `ms` as this computer's clock shows it: "14:02", with the day when it
/// isn't `today`'s ("14:02 on Nov 7").
fn clock(ms: i64, today: i64) -> String {
    let civil = |ms: i64| {
        let t = GoTime::unix_milli(ms);
        t.with_offset(sys::tz::local_offset(t.secs)).civil()
    };
    let (_, mo, d, h, mi, _) = civil(ms);
    let (_, tmo, td, ..) = civil(today);
    let hm = format!("{h:02}:{mi:02}");
    if (mo, d) == (tmo, td) {
        return hm;
    }
    let month = MONTHS
        .get((mo as usize).wrapping_sub(1))
        .copied()
        .unwrap_or("");
    format!("{hm} on {month} {d}")
}

impl Services {
    /// Installs (or takes away) the desktop's chat turn. Installing it
    /// answers the questions that came before it, and says so on
    /// `spawn.host` (`assistantChat`).
    pub(crate) fn set_chat_turn(&self, f: Option<ChatTurnFn>) {
        let some = f.is_some();
        *lock(&self.chat.hook) = f;
        self.republish_host();
        if some {
            self.chat_scan();
        }
    }

    /// Whether this run has the chat turn: then this computer can answer
    /// questions asked on the person's other devices.
    pub(crate) fn has_chat_turn(&self) -> bool {
        lock(&self.chat.hook).is_some()
    }

    /// The kind loop heard `ev`, after folding it. Notes how far it has
    /// folded, queues a question, and stops the running turn on its `stop`
    /// row or a New chat newer than its question. A row written here comes
    /// through too, already folded.
    pub(crate) fn chat_heard(&self, ev: &Event) {
        {
            let mut h = lock(&self.chat.heard);
            let seq = h.entry(ev.device_id.clone()).or_insert(0);
            if ev.seq > *seq {
                *seq = ev.seq;
            }
        }
        if actor_of(&ev.kind) != Some(ev.actor.as_str()) {
            return;
        }
        match ev.kind.as_str() {
            ASKED => {
                if let Ok(a) = serde_json::from_slice::<Asked>(&ev.payload) {
                    self.chat_live(&a.turn);
                }
            }
            STOP => {
                if let Ok(s) = serde_json::from_slice::<Stop>(&ev.payload) {
                    self.chat_trip(|r| r.turn == s.turn);
                }
            }
            CLEARED => {
                let at = Order::of(ev);
                self.chat_trip(|r| r.at < at);
            }
            _ => {}
        }
    }

    /// Trips the running turn's cancel when `hit` says so.
    fn chat_trip(&self, hit: impl Fn(&Running) -> bool) {
        let q = lock(&self.chat.q);
        if let Some(r) = q.running.as_ref().filter(|r| hit(r)) {
            r.cancel.store(true, Ordering::SeqCst);
        }
    }

    /// The kind loop resynced: it reloaded the folds from the store, whose
    /// vector just before was `before`. Rows it skipped are lost to the
    /// queue, so the scan finds them.
    pub(crate) fn chat_resynced(&self, before: &Vector) {
        {
            let mut h = lock(&self.chat.heard);
            for (w, seq) in before {
                let cur = h.entry(w.clone()).or_insert(0);
                if *seq > *cur {
                    *cur = *seq;
                }
            }
        }
        self.chat.resyncs.fetch_add(1, Ordering::SeqCst);
        self.chat_scan();
    }

    /// How many times the kind loop has resynced (tests).
    pub(crate) fn chat_resyncs(&self) -> u64 {
        self.chat.resyncs.load(Ordering::SeqCst)
    }

    fn now_ms(&self) -> i64 {
        (self.now)().unix_milli_value()
    }

    /// Queues question `turn` as its row arrives. A question the scan
    /// already queued becomes a live one: never skipped.
    fn chat_live(&self, turn: &str) {
        if !self.has_chat_turn() {
            return;
        }
        let Some(t) = self.assistant_chat.turn_at(turn) else {
            return;
        };
        if t.turn.asked.imported {
            return;
        }
        let took = self.now_ms();
        {
            let mut q = lock(&self.chat.q);
            if q.busy.contains(turn) {
                if let Some(p) = q.pending.get_mut(&t.at).filter(|p| p.turn == turn) {
                    p.live = true;
                    p.skip = false;
                    p.took_ms = p.took_ms.min(took);
                }
                return;
            }
            q.busy.insert(turn.to_string());
            q.pending.insert(
                t.at,
                Pending {
                    turn: turn.to_string(),
                    live: true,
                    took_ms: took,
                    skip: false,
                },
            );
        }
        self.chat.kick.kick();
    }

    /// The catch-up scan: queues every unanswered question New chat
    /// doesn't hide that isn't queued yet. Only on the assistant's computer
    /// with the hook. Of what it finds, the newest five the person didn't
    /// stop are answered; the older ones get a `skipped` reply when the
    /// worker reaches them, oldest first, before the five.
    pub(crate) fn chat_scan(&self) {
        if !self.has_chat_turn() || !self.is_assistant_home() {
            return;
        }
        let found = self.assistant_chat.unanswered();
        if found.is_empty() {
            return;
        }
        let answer: HashSet<&str> = found
            .iter()
            .rev()
            .filter(|(_, _, stopped)| !stopped)
            .take(SCAN_ANSWERS)
            .map(|(_, t, _)| t.as_str())
            .collect();
        let took = self.now_ms();
        let mut added = false;
        {
            let mut q = lock(&self.chat.q);
            for (at, turn, _) in &found {
                if !q.busy.insert(turn.clone()) {
                    continue;
                }
                q.pending.insert(
                    at.clone(),
                    Pending {
                        turn: turn.clone(),
                        live: false,
                        took_ms: took,
                        skip: !answer.contains(turn.as_str()),
                    },
                );
                added = true;
            }
        }
        if added {
            self.chat.kick.kick();
        }
    }

    /// The Core stops: the worker ends, and a running turn with it.
    pub(crate) fn chat_close(&self) {
        self.chat.kick.close();
        self.chat_trip(|_| true);
    }

    /// The worker: the oldest question first, one at a time, until the
    /// Core stops. One bad turn never ends it.
    pub(crate) fn chat_loop(&self) {
        let kick = self.chat.kick.clone();
        let mut seen = kick.generation();
        loop {
            let next = {
                let mut q = lock(&self.chat.q);
                q.pending.pop_first().map(|(at, p)| {
                    let cancel = Arc::new(AtomicBool::new(false));
                    q.running = Some(Running {
                        turn: p.turn.clone(),
                        at,
                        cancel: cancel.clone(),
                    });
                    (p, cancel)
                })
            };
            let Some((p, cancel)) = next else {
                if !kick.wait(&mut seen) {
                    return;
                }
                continue;
            };
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.chat_take(&p, &cancel)
            }));
            if run.is_err() {
                logf(format_args!("assistant chat: turn {} failed", p.turn));
            }
            let mut q = lock(&self.chat.q);
            q.running = None;
            q.busy.remove(&p.turn);
        }
    }

    /// Waits until this device is caught up with every own device it is
    /// connected to, and the kind loop has folded every row the store
    /// holds. False once the Core stops.
    fn chat_ready(&self) -> bool {
        let closed = || self.chat.kick.is_closed();
        let deadline = Instant::now() + CAUGHT_UP_WAIT;
        while !self.mesh.node.caught_up() {
            if closed() {
                return false;
            }
            if Instant::now() >= deadline {
                logf(format_args!(
                    "assistant chat: not caught up after {}s; answering anyway",
                    CAUGHT_UP_WAIT.as_secs()
                ));
                break;
            }
            std::thread::sleep(POLL);
        }
        let Ok(want) = self.store.vector() else {
            return !closed();
        };
        let deadline = Instant::now() + HEARD_WAIT;
        loop {
            if closed() {
                return false;
            }
            if covers(&lock(&self.chat.heard), &want) {
                return true;
            }
            if Instant::now() >= deadline {
                // The kind loop is behind (a flood): read the rows the
                // checks need from the store, not the caches.
                logf(format_args!(
                    "assistant chat: the folds are behind; reading the rows again"
                ));
                self.follow_assistant_home();
                if super::assistant_chat::load(&self.store, &self.assistant_chat) {
                    self.hub.publish(CoreEvent::AssistantChat);
                }
                return true;
            }
            std::thread::sleep(POLL);
        }
    }

    /// When question `asked`, written by `writer`, was asked by this
    /// computer's clock: its `at`, corrected by the asking device's skew
    /// where the mesh knows it.
    fn chat_asked_ms(&self, asked: &Asked, writer: &str) -> i64 {
        let skew_ns = if writer == self.device.id {
            0
        } else {
            self.mesh.node.skew(writer).unwrap_or(0)
        };
        asked.at.saturating_sub(skew_ns / 1_000_000)
    }

    /// The age note of a late question.
    fn chat_late_note(&self, writer: &str, asked_ms: i64, took_ms: i64) -> String {
        let labels = self.device_labels();
        let on = labels
            .get(writer)
            .filter(|n| !n.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| "another device".into());
        format!(
            "(Asked on {on} at {}; it reached your computer at {}. Changes asked for this long ago aren't made: if the person still wants one, ask them to say it again.)",
            clock(asked_ms, took_ms),
            clock(took_ms, took_ms)
        )
    }

    /// Writes this computer's reply to `turn`.
    fn chat_said(&self, turn: &str, said: Said) {
        let row = Said {
            turn: turn.to_string(),
            device: self.device.id.clone(),
            at: self.now_ms(),
            ..said
        };
        if let Err(e) = self.write_chat(ChatRow::Said(row)) {
            logf(format_args!("assistant chat: reply to {turn}: {e}"));
        }
    }

    /// Takes one question: waits until caught up, checks it against the
    /// fold as it is now, and runs it.
    fn chat_take(&self, p: &Pending, cancel: &Arc<AtomicBool>) {
        if !self.chat_ready() {
            return;
        }
        let Some(t) = self.assistant_chat.turn_at(&p.turn) else {
            return;
        };
        let turn = &t.turn;
        // The first check that fails drops it.
        if turn.asked.imported
            || !self.is_assistant_home()
            || turn.said.is_some()
            || t.hidden
        {
            return;
        }
        if turn.stopped || cancel.load(Ordering::SeqCst) {
            self.chat_said(
                &p.turn,
                Said {
                    error: STOPPED.into(),
                    ..Said::default()
                },
            );
            return;
        }
        if !p.live && p.skip {
            self.chat_said(
                &p.turn,
                Said {
                    skipped: true,
                    ..Said::default()
                },
            );
            return;
        }
        let Some(hook) = lock(&self.chat.hook).clone() else {
            return;
        };
        let asked_ms = self.chat_asked_ms(&turn.asked, &turn.asked_on);
        let late = p.took_ms.saturating_sub(asked_ms) > LATE_MS;
        // Only the person's own words allow a change (card #987), and not
        // words that waited this long.
        let grants = if late {
            Vec::new()
        } else {
            let earlier = self.assistant_chat.typed_before(&t.at, EARLIER_CAP);
            let earlier: Vec<&str> = earlier.iter().map(String::as_str).collect();
            tools::grants_for(&turn.asked.text, &earlier)
        };
        let note = late.then(|| self.chat_late_note(&turn.asked_on, asked_ms, p.took_ms));
        let history = self.assistant_chat.history_before(&t.at);
        let job = ChatJob {
            turn: p.turn.clone(),
            prompt: chat_prompt(&history, &turn.asked.text, note.as_deref()),
            screen: turn.asked.screen.clone(),
            grants,
            cancel: cancel.clone(),
            kind: ChatKind::Chat,
        };
        let c = cancel.clone();
        let done = hook(&job, &move |_| !c.load(Ordering::SeqCst));
        let stopped = cancel.load(Ordering::SeqCst);
        if done.later && !stopped {
            // Irori is closing: the next start's scan answers it.
            return;
        }
        self.chat_said(
            &p.turn,
            Said {
                text: done.text,
                changed: done.changed,
                posted: done.posted,
                looked: done.looked.unwrap_or_default(),
                error: if stopped { STOPPED.into() } else { done.error },
                ..Said::default()
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_is_the_last_twelve_turns_then_the_question() {
        assert_eq!(chat_prompt(&[], "hi", None), "The person: hi\n");
        let history: Vec<(bool, String)> =
            (0..20).map(|i| (i % 2 == 1, format!("turn {i}"))).collect();
        let p = chat_prompt(&history, "what's blocked?", None);
        assert!(p.starts_with("The conversation so far:\n\n"));
        assert!(!p.contains("turn 7\n"));
        assert!(p.contains("The person: turn 8\n"));
        assert!(p.contains("You: turn 19\n"));
        assert!(p.ends_with("Now answer the person's new message.\n\nThe person: what's blocked?\n"));
    }

    #[test]
    fn a_late_question_carries_its_note_after_it() {
        let p = chat_prompt(&[], "close #12", Some("(Asked on Phone at 14:02; …)"));
        assert_eq!(p, "The person: close #12\n(Asked on Phone at 14:02; …)\n");
    }

    #[test]
    fn covers_needs_every_writer() {
        let have: Vector = [("a".to_string(), 3), ("b".to_string(), 1)].into();
        assert!(covers(&have, &[("a".to_string(), 3)].into()));
        assert!(!covers(&have, &[("a".to_string(), 4)].into()));
        assert!(!covers(&have, &[("c".to_string(), 1)].into()));
    }
}
