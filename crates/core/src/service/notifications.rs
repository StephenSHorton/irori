//! The person's notification history (card #377): every notification this
//! device shows is kept as a replicated `notification.posted` row, so the
//! Notifications page (and the phone's) lists it after the popup is gone,
//! and reading one writes `notification.read`. The fold is
//! [`coord::notifications::History`].

use coord::notifications::{self as history, Item, PostedRow, ReadRow};

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
                },
            )
            .map_err(|e| e.to_string())?;
        if self.s.notifications.apply(&ev) {
            self.s.hub.publish(CoreEvent::Notifications);
        }
        Ok(())
    }
}
