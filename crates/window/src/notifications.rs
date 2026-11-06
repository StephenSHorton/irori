//! The Notifications page (card #377): every notification Irori showed the
//! person, newest first, from the Core's history. An item names what it
//! is about (a card's blocker, an ask, a message); the page reads that
//! thing's live state for its buttons, so they go once it's resolved.
//! An agent's ask is the one thing the window has no other view of, so
//! its state comes with the item.

use gocompat::time::GoTime;
use serde::Serialize;
use specta::Type;

use crate::Window;
use crate::cards::AgentRef;
use crate::ts;

/// `notifications:changed`: a notification was kept or read, here or on
/// another of the person's devices.
pub const EVENT_NOTIFICATIONS_CHANGED: &str = "notifications:changed";

/// The card an item is about.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NotificationCard {
    pub id: String,
    pub short: String,
    /// Its board's tag: numbers are per board (card #558).
    pub tag: String,
    pub title: String,
}

/// An agent's ask (its notify call with buttons): open until answered or
/// its time runs out.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NotificationAsk {
    pub open: bool,
    pub actions: Vec<String>,
    /// The answer once resolved: a button's label, or "" for a dismiss or
    /// no answer in time.
    pub choice: String,
    pub answered: bool,
    #[specta(type = ts::Time)]
    pub until: GoTime,
}

/// One notification.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    pub key: String,
    /// blocker, done_check, input, handed, returned, unclaimed, mention,
    /// ask, message, held or agent.
    pub kind: String,
    pub title: String,
    pub body: String,
    pub level: String,
    #[specta(type = ts::Time)]
    pub at: GoTime,
    pub read: bool,
    pub task_id: String,
    pub blocker_id: String,
    pub comment_id: String,
    pub delivery_id: String,
    pub board_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub from: Option<AgentRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub card: Option<NotificationCard>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub ask: Option<NotificationAsk>,
}

/// The most items the page lists.
pub const MAX_LISTED: usize = 500;

impl Window {
    /// The person's notifications, newest first.
    pub fn notifications(&self) -> Vec<Notification> {
        let items = self.core.notifications();
        if items.is_empty() {
            return Vec::new();
        }
        let p = self.people();
        let co = self.core.coord();
        let asks = self.core.asks();
        let now = GoTime::from_system(std::time::SystemTime::now());
        items
            .into_iter()
            .take(MAX_LISTED)
            .map(|it| {
                let mut r = it.posted;
                let task = if r.task_id.is_empty() {
                    None
                } else {
                    co.get_task(&r.task_id).or_else(|| {
                        (!r.board_id.is_empty())
                            .then(|| self.core.board_coord(&r.board_id).ok())
                            .flatten()
                            .and_then(|c| c.get_task(&r.task_id))
                    })
                };
                let card = task.as_ref().map(|t| NotificationCard {
                    id: t.id.clone(),
                    short: t.short.clone(),
                    tag: t.tag.clone(),
                    title: t.title.clone(),
                });
                let mut from = (!r.from.is_empty()).then(|| p.ref_(&r.from));
                if r.kind == coord::notifications::KIND_RETURNED {
                    // A row kept before card #483 names the agent by its
                    // key: say it again by name, never the key.
                    let name = returned_name(from.as_ref(), task.as_ref(), &r.from)
                        .or_else(|| coord::notifications::returned_name_in(&r.body, &r.from));
                    r.body = coord::notifications::returned_body(name.as_deref());
                    match (&mut from, name) {
                        (Some(f), Some(n)) => f.name = n,
                        (f, None) => *f = None,
                        _ => {}
                    }
                }
                let ask = if r.kind == coord::notifications::KIND_ASK && !r.ask_id.is_empty() {
                    asks.get(&r.ask_id).map(|a| NotificationAsk {
                        open: !a.resolved && a.until.unix_nano() > now.unix_nano(),
                        actions: a.actions.clone(),
                        choice: a.choice.clone(),
                        answered: a.answered,
                        until: a.until,
                    })
                } else {
                    None
                };
                Notification {
                    key: it.key,
                    kind: r.kind,
                    title: r.title,
                    body: r.body,
                    level: r.level,
                    at: r.at,
                    read: it.read,
                    task_id: r.task_id,
                    blocker_id: r.blocker_id,
                    comment_id: r.comment_id,
                    delivery_id: r.delivery_id,
                    board_id: r.board_id,
                    from,
                    card,
                    ask,
                }
            })
            .collect()
    }

    /// How many notifications the person hasn't read.
    pub fn unread_notifications(&self) -> u32 {
        u32::try_from(self.core.unread_notifications()).unwrap_or(u32::MAX)
    }

    /// Marks notifications read; none named reads them all.
    pub fn read_notifications(&self, keys: &[String]) -> Result<(), String> {
        self.core.read_notifications(keys)
    }

    /// Answers an agent's ask from its notification: refused once it was
    /// answered (on any device) or its time ran out, so a stale page never
    /// answers twice.
    pub fn answer_notification_ask(&self, ask_id: &str, choice: &str) -> Result<(), String> {
        let asks = self.core.asks();
        let Some(a) = asks.get(ask_id) else {
            return Err("that ask is gone".into());
        };
        if a.resolved {
            return Err(if a.answered && !a.choice.is_empty() {
                format!("Already answered: {}", a.choice)
            } else {
                "Already closed: it was dismissed or ran out of time".into()
            });
        }
        let now = GoTime::from_system(std::time::SystemTime::now());
        if a.until.unix_nano() <= now.unix_nano() {
            return Err("It ran out of time: the agent heard no answer".into());
        }
        if !choice.is_empty() && !a.actions.iter().any(|l| l == choice) {
            return Err(format!("“{choice}” isn't one of its buttons"));
        }
        asks.answer(ask_id, choice, &self.core.user_sender().key)
            .map_err(|e| e.to_string())
    }
}

/// The name a returned card's gone agent went by (card #483): its name as
/// the window knows it (live, or the last it went by), then the card's
/// spawn record; `None` when only its key is known. "unknown agent" is
/// what the window calls a key nothing names (card #478), not a name.
fn returned_name(from: Option<&AgentRef>, task: Option<&coord::Task>, key: &str) -> Option<String> {
    from.filter(|f| f.name != "unknown agent")
        .and_then(|f| coord::notifications::agent_name_of(key, &f.name))
        .or_else(|| task.and_then(|t| coord::notifications::spawned_name(t, key)))
}
