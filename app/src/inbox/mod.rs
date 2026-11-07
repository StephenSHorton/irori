//! Notifications, Held and Message (P9): the Rune port of the React app's
//! components/notifications-page.tsx, lib/notifications.ts (non-sample),
//! cards/held-dialog.tsx and cards/message-dialog.tsx (on main, under
//! frontend/src/).
//!
//! - [`view`]: the Notifications page (card #377): every notification
//!   Irori showed the person, newest first, Unread and All, each item's
//!   buttons read from the thing it names as it stands now.
//! - [`dialog`]: `Dialog::Held` (the Held list and its History) and
//!   `Dialog::Message` (one message a notification's Open names).
//!
//! The page reads its list on start and on every notifications, board or
//! inbox change; the items it shows read their card or message then too.

pub mod notify;
pub mod words;

mod held;
mod message;
mod page;

use std::collections::{HashMap, HashSet};

use irori_window::cards::CardDetail;
use irori_window::events::{EVENT_BOARD_CHANGED, EVENT_INBOX_CHANGED, EVENT_ROSTER_CHANGED};
use irori_window::notifications::{EVENT_NOTIFICATIONS_CHANGED, Notification};
use irori_window::roster::GoneSession;
use irori_window::rune_inbox::Delivery;
use rune::{El, TextEdit};

use crate::ui::{Dialog, Event, Frame, Route, State};

use notify::Filter;
use words::{Names, Repos};

/// How many items show before "Show more".
pub const PAGE_SIZE: usize = 30;
/// The History's length (lib/talk.ts HeldHistoryLimit).
pub const HELD_HISTORY: i64 = 100;

/// One row's action: running, how it ended, or why it failed (needs-you's
/// useAct and Said).
#[derive(Clone, Debug, Default)]
pub struct Act {
    pub busy: bool,
    pub error: Option<String>,
    pub done: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HeldTab {
    #[default]
    Waiting,
    History,
}

/// The Held dialog's lists.
#[derive(Default)]
pub struct Held {
    pub tab: HeldTab,
    pub list: Vec<Delivery>,
    pub history: Vec<Delivery>,
    pub loaded: bool,
    pub error: Option<String>,
    /// The delivery an action runs on ("" none).
    pub busy: String,
    pub act_error: Option<String>,
    pub said: String,
}

/// The Message dialog's message and reply.
#[derive(Default)]
pub struct Message {
    pub id: String,
    pub d: Option<Result<Delivery, String>>,
    // TODO(text::composer): the shared composer (mentions, files, voice)
    // replaces this plain box once it lands.
    pub reply: TextEdit,
    pub missed: String,
    pub error: Option<String>,
    pub busy: bool,
}

#[derive(Default)]
pub struct Model {
    // The Notifications page.
    pub list: Vec<Notification>,
    pub loaded: bool,
    pub error: Option<String>,
    pub filter: Filter,
    pub limit: usize,
    /// Items read while Unread shows: they stay until the filter changes,
    /// so one doesn't jump out from under the pointer.
    pub kept: HashSet<String>,
    /// The cards the shown items ask on, as they stand now.
    pub cards: HashMap<String, Result<CardDetail, String>>,
    /// The messages the shown items name, as they stand now.
    pub deliveries: HashMap<String, Result<Delivery, String>>,
    /// Each item's action, by its key.
    pub acts: HashMap<String, Act>,
    /// The item whose Reply box is open, and its words.
    pub replying: Option<String>,
    pub reply: TextEdit,
    /// Picks shown at once while they save (card #644), by item key.
    pub picked: HashMap<String, String>,

    // Shared by the page and the dialogs.
    pub repos: Repos,
    pub gone: Vec<GoneSession>,
    pub held: Held,
    pub message: Message,
}

/// Names actor keys: the roster's, the held messages' senders and the
/// sessions that went (lib/talk.ts nameLookup in App).
pub fn names(s: &State) -> Names {
    let mut n = Names::default();
    for a in &s.data.agents {
        n.add(&a.card.key, &a.card.name);
    }
    let m = &s.inbox;
    for d in m.held.list.iter().chain(&m.held.history) {
        n.add(&d.message.from.key, &d.message.from.name);
    }
    if let Some(Ok(d)) = &m.message.d {
        n.add(&d.message.from.key, &d.message.from.name);
    }
    for d in m.deliveries.values().flatten() {
        n.add(&d.message.from.key, &d.message.from.name);
    }
    for g in &m.gone {
        n.add(&g.key, &g.name);
    }
    n
}

/// Re-reads the notifications, then what the shown items name.
pub fn load(s: &mut State) {
    s.spawn(
        |w| w.notifications(),
        |s, list| {
            s.inbox.list = list;
            s.inbox.loaded = true;
            s.inbox.error = None;
            load_live(s);
        },
    );
}

/// The items the page shows now (the filter, the kept ones).
pub fn items(m: &Model) -> Vec<&Notification> {
    match m.filter {
        Filter::Unread => m.list.iter().filter(|n| !n.read || m.kept.contains(&n.key)).collect(),
        Filter::All => notify::shown(&m.list, Filter::All),
    }
}

/// Reads the cards and messages the shown items name.
pub fn load_live(s: &mut State) {
    let m = &s.inbox;
    let mut cards: Vec<String> = Vec::new();
    let mut deliveries: Vec<String> = Vec::new();
    for n in items(m).into_iter().take(m.limit.max(PAGE_SIZE)) {
        if notify::asks_on_card(n)
            && let Some(c) = &n.card
            && !cards.contains(&c.id)
        {
            cards.push(c.id.clone());
        }
        if matches!(n.kind.as_str(), notify::kind::MESSAGE | notify::kind::HELD)
            && !n.delivery_id.is_empty()
            && !notify::waited_summary(n)
            && !deliveries.contains(&n.delivery_id)
        {
            deliveries.push(n.delivery_id.clone());
        }
    }
    if cards.is_empty() && deliveries.is_empty() {
        return;
    }
    s.spawn(
        move |w| {
            let c: Vec<_> = cards.into_iter().map(|id| { let r = w.card(&id); (id, r) }).collect();
            let d: Vec<_> = deliveries.into_iter().map(|id| { let r = w.inbox_get(&id); (id, r) }).collect();
            (c, d)
        },
        |s, (c, d)| {
            s.inbox.cards.extend(c);
            s.inbox.deliveries.extend(d);
        },
    );
}

/// Re-reads the Held list and its History, and the repo names.
pub fn load_held(s: &mut State) {
    s.spawn(
        |w| (w.inbox_held(), w.inbox_held_history(HELD_HISTORY), w.inbox_repo_names()),
        |s, (mut list, history, repos)| {
            list.sort_by_key(|d| std::cmp::Reverse(d.created_at.unix_milli_value()));
            let h = &mut s.inbox.held;
            h.list = list;
            h.history = history;
            h.loaded = true;
            h.error = None;
            s.inbox.repos = Repos(repos.into_iter().map(|r| (r.repo_key, r.name)).collect());
        },
    );
}

/// Reads the message the open Message dialog names.
pub fn load_message(s: &mut State) {
    let id = s.inbox.message.id.clone();
    if id.is_empty() {
        return;
    }
    s.spawn(
        {
            let id = id.clone();
            move |w| (w.inbox_get(&id), w.inbox_repo_names())
        },
        move |s, (d, repos)| {
            if s.inbox.message.id != id {
                return;
            }
            s.inbox.message.d = Some(d);
            s.inbox.repos = Repos(repos.into_iter().map(|r| (r.repo_key, r.name)).collect());
        },
    );
}

fn load_gone(s: &mut State) {
    s.spawn(|w| w.gone(20).unwrap_or_default(), |s, g| s.inbox.gone = g);
}

pub fn on_event(s: &mut State, ev: &Event) {
    match ev.name.as_str() {
        "app:start" => {
            s.inbox.limit = PAGE_SIZE;
            load(s);
            load_gone(s);
        }
        EVENT_NOTIFICATIONS_CHANGED | EVENT_BOARD_CHANGED => load(s),
        EVENT_INBOX_CHANGED => {
            load(s);
            if matches!(s.dialog, Some(Dialog::Held { .. })) {
                load_held(s);
            }
            if matches!(s.dialog, Some(Dialog::Message { .. })) {
                load_message(s);
            }
        }
        EVENT_ROSTER_CHANGED => load_gone(s),
        "app:dialog-opened" => match s.dialog.clone() {
            Some(Dialog::Held { .. }) => {
                s.inbox.held.tab = HeldTab::Waiting;
                s.inbox.held.said.clear();
                s.inbox.held.act_error = None;
                load_held(s);
            }
            Some(Dialog::Message { id }) => {
                s.inbox.message = Message {
                    id,
                    reply: focused_edit(),
                    ..Message::default()
                };
                load_message(s);
            }
            _ => {}
        },
        _ => {}
    }
}

/// The Notifications page.
pub fn view(s: &State, f: &mut Frame) -> El {
    page::view(s, f)
}

/// The headless scene that shows it (`irori --snap x.png --scene notifications`):
/// All, so read items show too.
pub fn scene(s: &mut State) {
    s.close_card();
    s.go(Route::Notifications);
    s.inbox.filter = Filter::All;
    load(s);
}

/// The Held list over the Notifications page (`--scene held`).
pub fn held_scene(s: &mut State) {
    s.close_card();
    s.go(Route::Notifications);
    s.open_dialog(Dialog::Held { from: None });
}

/// Held or Message, whichever `s.dialog` names (mounted in `f.layer`).
pub fn dialog(s: &State, f: &mut Frame) {
    match &s.dialog {
        Some(Dialog::Held { from }) => held::dialog(s, f, from.as_deref()),
        Some(Dialog::Message { id }) => message::dialog(s, f, id),
        _ => {}
    }
}

/// An empty field that asks for focus when it mounts.
pub fn focused_edit() -> TextEdit {
    let mut e = TextEdit::default();
    e.focused = true;
    e
}
