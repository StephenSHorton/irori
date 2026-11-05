//! The person's notification history (card #377): every notification
//! Irori showed them, kept so they can look back at it, read or not.
//!
//! The device that shows a notification writes a replicated
//! `notification.posted` row (actor `irori`); reading one writes
//! `notification.read` (actor `user`). Both stay in the private log and
//! sync to the person's other devices, so the phone, which shows no
//! desktop notifications, has the same history.
//!
//! A row's key names the thing it is about (a card's blocker, an ask, a
//! message), so the same thing posted again, after a restart or by
//! another of the person's desktops, stays one item: the first post wins.
//! A post with no key (an agent's notify call) is its own item.
//!
//! What the item can still do is not kept here: the window reads it from
//! the thing itself (the card's blocker, the delivery, the ask), so its
//! buttons follow that and never a copy.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use gocompat::time::GoTime;

use crate::gojson::{decode, omit};

pub const POSTED: &str = store::kinds::NOTIFICATION_POSTED;
pub const READ: &str = store::kinds::NOTIFICATION_READ;

/// What a notification is about (`kind`).
pub const KIND_BLOCKER: &str = "blocker";
pub const KIND_DONE_CHECK: &str = "done_check";
pub const KIND_INPUT: &str = "input";
pub const KIND_HANDED: &str = "handed";
pub const KIND_RETURNED: &str = "returned";
pub const KIND_UNCLAIMED: &str = "unclaimed";
pub const KIND_MENTION: &str = "mention";
pub const KIND_ASK: &str = "ask";
pub const KIND_MESSAGE: &str = "message";
pub const KIND_HELD: &str = "held";
pub const KIND_AGENT: &str = "agent";
/// The one quiet card for several messages that waited while Irori was
/// off (#364): `delivery_ids` names them, `delivery_id` the newest.
pub const KIND_WAITED: &str = "waited";

/// A read row's `why` when Irori wrote it because the thing the item is
/// about was resolved, by the person or anyone (card #482).
pub const WHY_RESOLVED: &str = "resolved";

crate::go_struct! {
    /// The `notification.posted` payload.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct PostedRow {
        pub key: String = "key" "omit",
        pub kind: String = "kind",
        pub title: String = "title",
        pub body: String = "body" "omit",
        pub level: String = "level" "omit",
        pub task_id: String = "taskId" "omit",
        pub blocker_id: String = "blockerId" "omit",
        pub ask_id: String = "askId" "omit",
        pub delivery_id: String = "deliveryId" "omit",
        /// Every delivery a `waited` item stands for.
        pub delivery_ids: Vec<String> = "deliveryIds" "omit",
        pub board_id: String = "boardId" "omit",
        pub comment_id: String = "commentId" "omit",
        /// Which time it happened, when the same thing can happen again
        /// (a card returned twice): part of the key.
        pub instance: String = "instance" "omit",
        /// Who it is from: an agent key, when there is one.
        pub from: String = "from" "omit",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// The `notification.read` payload.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct ReadRow {
        pub keys: Vec<String> = "keys",
        /// Why it was read when the person didn't read it: `resolved`
        /// (Irori, actor `irori`) once the thing it is about was settled.
        pub why: String = "why" "omit",
        pub at: GoTime = "at",
    }
}

/// One notification in the history.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Item {
    /// The row's key, or `row:<device>:<seq>` for a post without one.
    pub key: String,
    pub posted: PostedRow,
    /// Which device showed it first.
    pub device: String,
    pub read: bool,
    hlc: u64,
    seq: i64,
}

#[derive(Debug, Default)]
struct State {
    items: HashMap<String, Item>,
    read: HashSet<String>,
}

/// Folds `notification.posted` and `notification.read` rows. Safe for
/// concurrent use.
#[derive(Debug, Default)]
pub struct History {
    m: Mutex<State>,
}

impl History {
    pub fn new() -> History {
        History::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.m.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Forgets every row (a resync reloads them).
    pub fn reset(&self) {
        *self.lock() = State::default();
    }

    /// Folds one row; reports whether the history changed.
    pub fn apply(&self, ev: &store::Event) -> bool {
        match ev.kind.as_str() {
            POSTED if ev.actor == "irori" => {
                let Some(r) = decode::<PostedRow>(&ev.payload) else {
                    return false;
                };
                if r.title.trim().is_empty() && r.body.trim().is_empty() {
                    return false;
                }
                let key = if r.key.is_empty() {
                    format!("row:{}:{}", ev.device_id, ev.seq)
                } else {
                    r.key.clone()
                };
                let mut st = self.lock();
                if let Some(cur) = st.items.get(&key)
                    && !older(ev, cur)
                {
                    return false;
                }
                let read = st.read.contains(&key);
                st.items.insert(
                    key.clone(),
                    Item {
                        key,
                        posted: r,
                        device: ev.device_id.clone(),
                        read,
                        hlc: ev.hlc.0,
                        seq: ev.seq,
                    },
                );
                true
            }
            READ if ev.actor == "user" || ev.actor == "irori" => {
                let Some(r) = decode::<ReadRow>(&ev.payload) else {
                    return false;
                };
                // Irori reads only what it settled, and says so.
                if ev.actor == "irori" && r.why != WHY_RESOLVED {
                    return false;
                }
                let mut st = self.lock();
                let mut changed = false;
                for k in r.keys {
                    if let Some(it) = st.items.get_mut(&k)
                        && !it.read
                    {
                        it.read = true;
                        changed = true;
                    }
                    st.read.insert(k);
                }
                changed
            }
            _ => false,
        }
    }

    /// Every item, newest first.
    pub fn list(&self) -> Vec<Item> {
        let mut out: Vec<Item> = self.lock().items.values().cloned().collect();
        out.sort_by(|a, b| {
            b.posted
                .at
                .unix_nano()
                .cmp(&a.posted.at.unix_nano())
                .then(b.hlc.cmp(&a.hlc))
                .then(b.key.cmp(&a.key))
        });
        out
    }

    /// How many items are unread.
    pub fn unread(&self) -> usize {
        self.lock().items.values().filter(|i| !i.read).count()
    }

    /// Whether key is an item already (a post of it would change nothing).
    pub fn has(&self, key: &str) -> bool {
        !key.is_empty() && self.lock().items.contains_key(key)
    }

    /// Of keys, the items not read yet: what a read row needs to name.
    pub fn unread_of(&self, keys: &[String]) -> Vec<String> {
        let st = self.lock();
        keys.iter()
            .filter(|k| st.items.get(*k).is_some_and(|i| !i.read))
            .cloned()
            .collect()
    }

    /// Every unread item, in no order.
    pub fn unread_items(&self) -> Vec<Item> {
        self.lock()
            .items
            .values()
            .filter(|i| !i.read)
            .cloned()
            .collect()
    }

    /// Every unread item's key.
    pub fn unread_keys(&self) -> Vec<String> {
        let st = self.lock();
        let mut out: Vec<String> = st
            .items
            .values()
            .filter(|i| !i.read)
            .map(|i| i.key.clone())
            .collect();
        out.sort();
        out
    }
}

/// Whether ev was written before the item's row (the first post wins).
fn older(ev: &store::Event, cur: &Item) -> bool {
    if ev.hlc.0 != cur.hlc {
        return ev.hlc.0 < cur.hlc;
    }
    if ev.device_id != cur.device {
        return ev.device_id < cur.device;
    }
    ev.seq < cur.seq
}

/// The body of a returned card's notification (card #483): the agent that
/// went away by its name, or "Its agent" when no name is known. Never an
/// agent key: the watcher posts it and the Notifications page rebuilds a
/// stored row's body with it.
pub fn returned_body(name: Option<&str>) -> String {
    let who = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(RETURNED_NOBODY);
    format!("{who}{RETURNED_TAIL}")
}

const RETURNED_TAIL: &str = " went away, so it's open again. Assign it to an agent, or close it.";
const RETURNED_NOBODY: &str = "Its agent";

/// The name a stored returned row's body gave agent `key`, when it gave
/// one and not the key (rows before card #483 named it by key): what the
/// page falls back to once nothing else knows the name.
pub fn returned_name_in(body: &str, key: &str) -> Option<String> {
    let who = body.strip_suffix(RETURNED_TAIL)?.trim();
    if who == RETURNED_NOBODY || who.contains('/') {
        return None;
    }
    agent_name_of(key, who)
}

/// `name` when it is a name for agent `key` and not the key itself (a
/// resolver's fallback): `None` for empty or the key (card #483).
pub fn agent_name_of(key: &str, name: &str) -> Option<String> {
    let n = name.trim();
    (!n.is_empty() && n != key.trim()).then(|| n.to_string())
}

/// The name of agent `key` from task `t`'s spawn records: the name the
/// card's agent was started with (card #483).
pub fn spawned_name(t: &crate::Task, key: &str) -> Option<String> {
    if key.is_empty() {
        return None;
    }
    t.spawns
        .iter()
        .rev()
        .filter(|s| s.key == key)
        .find_map(|s| agent_name_of(key, &s.name))
}

/// The key for what a notification is about: the same thing gets the
/// same key on every device. Empty when it is only itself.
pub fn key_of(r: &PostedRow) -> String {
    let parts: Vec<&str> = [
        r.board_id.as_str(),
        r.task_id.as_str(),
        r.blocker_id.as_str(),
        r.comment_id.as_str(),
        r.ask_id.as_str(),
        r.delivery_id.as_str(),
        r.instance.as_str(),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect();
    if parts.is_empty() || r.kind == KIND_AGENT {
        return String::new();
    }
    format!("{}:{}", r.kind, parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        device: &str,
        seq: i64,
        hlc: u64,
        actor: &str,
        kind: &str,
        payload: String,
    ) -> store::Event {
        store::Event {
            device_id: device.into(),
            seq,
            hlc: store::Timestamp(hlc),
            actor: actor.into(),
            kind: kind.into(),
            payload: payload.into_bytes(),
            ..store::Event::default()
        }
    }

    fn posted(device: &str, seq: i64, hlc: u64, title: &str, at: &str) -> store::Event {
        let mut r = PostedRow {
            kind: KIND_BLOCKER.into(),
            task_id: "T1".into(),
            blocker_id: "b1".into(),
            title: title.into(),
            at: GoTime::parse_rfc3339(at).unwrap(),
            ..PostedRow::default()
        };
        r.key = key_of(&r);
        let p = gocompat::json::marshal(&r).unwrap();
        row(
            device,
            seq,
            hlc,
            "irori",
            POSTED,
            String::from_utf8(p).unwrap(),
        )
    }

    #[test]
    fn one_item_per_thing_the_first_post_wins_and_reads_sync() {
        let h = History::new();
        // The PC's post arrives after the Mac's later one: the first wins.
        assert!(h.apply(&posted("mac", 4, 20, "second", "2026-11-03T10:00:02Z")));
        assert!(h.apply(&posted("pc", 9, 10, "first", "2026-11-03T10:00:01Z")));
        assert!(!h.apply(&posted("mac", 5, 30, "third", "2026-11-03T10:00:03Z")));
        let l = h.list();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].posted.title, "first");
        assert_eq!(l[0].key, "blocker:T1/b1");

        // A read from the phone, before the post it names reaches here.
        let read = ReadRow {
            keys: vec!["blocker:T2/b1".into()],
            ..ReadRow::default()
        };
        let p = String::from_utf8(gocompat::json::marshal(&read).unwrap()).unwrap();
        assert!(!h.apply(&row("phone", 1, 40, "user", READ, p.clone())));
        // Only the person reads, or Irori once the thing was resolved.
        assert!(!h.apply(&row("phone", 2, 41, "irori", READ, p)));
        let mut r2 = posted("mac", 6, 50, "other", "2026-11-03T10:00:05Z");
        r2.payload = String::from_utf8(r2.payload.clone())
            .unwrap()
            .replace("T1", "T2")
            .into_bytes();
        assert!(h.apply(&r2));
        let l = h.list();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].posted.task_id, "T2", "newest first");
        assert!(l[0].read, "the early read counts");
        assert_eq!(h.unread(), 1);
        assert_eq!(h.unread_keys(), vec!["blocker:T1/b1".to_string()]);

        // Irori reads what it settled.
        let settled = ReadRow {
            keys: vec!["blocker:T1/b1".into()],
            why: WHY_RESOLVED.into(),
            ..ReadRow::default()
        };
        let sp = String::from_utf8(gocompat::json::marshal(&settled).unwrap()).unwrap();
        assert!(h.apply(&row("pc", 3, 52, "irori", READ, sp)));
        assert_eq!(h.unread(), 0, "a resolved item is read");
        assert!(h.unread_items().is_empty());

        // A post without a key is its own item.
        let agent = PostedRow {
            kind: KIND_AGENT.into(),
            title: "alpha".into(),
            body: "green".into(),
            ..PostedRow::default()
        };
        assert_eq!(key_of(&agent), "");
        let p = String::from_utf8(gocompat::json::marshal(&agent).unwrap()).unwrap();
        assert!(h.apply(&row("mac", 7, 60, "irori", POSTED, p.clone())));
        assert!(h.apply(&row("mac", 8, 61, "irori", POSTED, p)));
        assert_eq!(h.list().len(), 4);
    }
}
