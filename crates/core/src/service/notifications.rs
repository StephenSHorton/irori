//! The person's notification history (card #377): every notification this
//! device shows is kept as a replicated `notification.posted` row, so the
//! Notifications page (and the phone's) lists it after the popup is gone,
//! and reading one writes `notification.read`. The fold is
//! [`coord::notifications::History`].
//!
//! Once the thing an item is about is resolved, by the person or anyone
//! (a blocker answered, a done check marked, an ask closed, a message
//! replied to), Irori reads it for them (card #482): a `notification.read`
//! row with actor `irori` and why `resolved`, so the badge counts only
//! what still needs the person, on every one of their devices.

use coord::Task;
use coord::notifications::{self as history, Item, PostedRow, ReadRow};
use gocompat::time::GoTime;

use super::{Core, Services};
use crate::events::CoreEvent;

/// A level's name in the row.
fn level_name(l: notify::Level) -> &'static str {
    match l {
        notify::Level::Info => "info",
        notify::Level::Success => "success",
        notify::Level::Warn => "warn",
        notify::Level::Error => "error",
    }
}

impl Services {
    /// Keeps note in the history: once per thing it is about, however
    /// often it is shown again (a restart, another of the person's
    /// desktops).
    pub(crate) fn record_notification(&self, note: &notify::Note) {
        let Some(about) = &note.about else { return };
        let mut r = PostedRow {
            title: note.title.clone(),
            body: note.body.clone(),
            level: level_name(note.level).into(),
            at: (self.now)(),
            ..about.clone()
        };
        r.key = history::key_of(&r);
        if self.notifications.has(&r.key) {
            return;
        }
        match self.store.append("irori", history::POSTED, &r) {
            Ok(ev) => {
                if self.notifications.apply(&ev) {
                    self.hub.publish(CoreEvent::Notifications);
                }
            }
            Err(e) => crate::logf(format_args!("notifications: keep {}: {e}", note.id)),
        }
    }
}

impl Services {
    /// Reads, as Irori, every unread item whose thing is resolved now.
    /// Called at start and on every board, inbox or ask change; an item
    /// whose thing this device can't see yet stays as it is.
    pub(crate) fn settle_notifications(&self) {
        // The feed loops settle in parallel: one at a time, so two woken
        // by the same answer don't both write its read row.
        static SETTLING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _one = SETTLING.lock().unwrap_or_else(|e| e.into_inner());
        let items = self.notifications.unread_items();
        if items.is_empty() {
            return;
        }
        let now = (self.now)();
        let keys: Vec<String> = items
            .iter()
            .filter(|it| self.resolved(&it.posted, now))
            .map(|it| it.key.clone())
            .collect();
        if keys.is_empty() {
            return;
        }
        let row = ReadRow {
            keys,
            why: history::WHY_RESOLVED.into(),
            at: now,
        };
        match self.store.append("irori", history::READ, &row) {
            Ok(ev) => {
                if self.notifications.apply(&ev) {
                    self.hub.publish(CoreEvent::Notifications);
                }
            }
            Err(e) => crate::logf(format_args!("notifications: settle: {e}")),
        }
    }

    /// The card r is about, on this device's board or the one it names.
    fn notification_task(&self, r: &PostedRow) -> Option<Task> {
        self.coord.get_task(&r.task_id).or_else(|| {
            if r.board_id.is_empty() {
                return None;
            }
            self.board(&r.board_id).ok()?.coord().get_task(&r.task_id)
        })
    }

    /// Whether the thing r is about no longer needs the person. Only the
    /// things that ask something of them settle; a card handed to them, a
    /// mention or an agent's note stays until they read it.
    fn resolved(&self, r: &PostedRow, now: GoTime) -> bool {
        match r.kind.as_str() {
            history::KIND_BLOCKER | history::KIND_INPUT | history::KIND_DONE_CHECK => {
                if r.task_id.is_empty() {
                    return false;
                }
                self.notification_task(r)
                    .is_some_and(|t| card_resolved(&t, r))
            }
            history::KIND_NO_AGENT => self
                .notification_task(r)
                .is_none_or(|t| coord::is_terminal(&t.state) || !self.coord.waits_for_agent(&t)),
            history::KIND_ASK => self
                .asks
                .get(&r.ask_id)
                .is_some_and(|a| a.resolved || a.until.unix_nano() <= now.unix_nano()),
            history::KIND_MESSAGE => {
                // Before card #482 the waited summary was a message about
                // delivery "waiting": it names no message, so nothing
                // more can come of it.
                if r.delivery_id == LEGACY_WAITING {
                    return true;
                }
                self.inbox
                    .get(&r.delivery_id)
                    .is_some_and(|d| !message_waits(&d.state))
            }
            history::KIND_HELD => self
                .inbox
                .get(&r.delivery_id)
                .is_some_and(|d| d.state != inbox::st::HELD),
            history::KIND_WAITED => {
                !r.delivery_ids.is_empty()
                    && r.delivery_ids
                        .iter()
                        .all(|id| self.inbox.get(id).is_some_and(|d| !message_waits(&d.state)))
            }
            _ => false,
        }
    }
}

/// The delivery id the waited summary carried before card #482.
const LEGACY_WAITING: &str = "waiting";

/// Whether a message to the person still waits on them: not replied,
/// read, dismissed, held or moved (the page's `messageOpen`).
fn message_waits(state: &str) -> bool {
    use inbox::st;
    matches!(
        state,
        st::QUEUED | st::DELIVERED | st::ENGAGED | st::DEFERRED
    )
}

/// Whether a card's blocker, question or done check no longer needs the
/// person: resolved, answered (it's with the agent now), another member's
/// to decide, or the card finished.
fn card_resolved(t: &Task, r: &PostedRow) -> bool {
    if coord::is_terminal(&t.state) {
        return true;
    }
    if !r.blocker_id.is_empty() {
        let Some(b) = t.blockers.iter().find(|b| b.id == r.blocker_id) else {
            return false;
        };
        return b.resolved.is_some()
            || b.status == coord::BLOCKER_ANSWERED
            || b.on.as_ref().is_some_and(|on| {
                on.kind == coord::BLOCKED_ON_PERSON
                    && on
                        .reference
                        .starts_with(irori_identity::kinds::PERSON_ACTOR_PREFIX)
            });
    }
    // An older card's question without a blocker item: open while the
    // card still asks.
    if r.kind == history::KIND_DONE_CHECK {
        return !t.blockers.iter().any(|b| b.open() && b.done_check);
    }
    t.state != coord::INPUT_REQUIRED && t.state != coord::BLOCKED
}

impl Core {
    /// The person's notifications, newest first, read or not.
    pub fn notifications(&self) -> Vec<Item> {
        self.s.notifications.list()
    }

    /// How many notifications the person hasn't read.
    pub fn unread_notifications(&self) -> usize {
        self.s.notifications.unread()
    }

    /// Marks notifications read as the person; no keys reads them all.
    /// Already read ones write nothing.
    pub fn read_notifications(&self, keys: &[String]) -> Result<(), String> {
        let keys = if keys.is_empty() {
            self.s.notifications.unread_keys()
        } else {
            self.s.notifications.unread_of(keys)
        };
        if keys.is_empty() {
            return Ok(());
        }
        let ev = self
            .s
            .store
            .append(
                "user",
                history::READ,
                &ReadRow {
                    keys,
                    at: (self.s.now)(),
                    ..ReadRow::default()
                },
            )
            .map_err(|e| e.to_string())?;
        if self.s.notifications.apply(&ev) {
            self.s.hub.publish(CoreEvent::Notifications);
        }
        Ok(())
    }
}
