//! A card's page (P3): the Rune port of the React app's
//! cards/card-detail.tsx (CardPage and its parts), cards/thread.tsx,
//! cards/share-dialog.tsx, cards/new-card-dialog.tsx and
//! cards/start-past-limit.tsx. It shows over the route's page while
//! `s.nav.card` is set.
//!
//! # API (for the other packages)
//!
//! - [`thread_view`]`(s, f, key, &ThreadOpts)`: a card's (or a Talk
//!   thread's) entries, oldest first, with "Show earlier", the live
//!   "oak is working…" line, a message opened in full, comment Modify,
//!   Release and "Start an agent with this". Talk passes `scroll: true`
//!   and its reply box as `composer`.
//! - [`reply_box`]`(s, f, card_id)`: a thread's reply box (CommentBox with
//!   `reply`); call [`open_reply`]`(s, card_id)` when the thread opens, so
//!   its draft loads.
//! - [`start_past_limit`]`(s, f, key, id, &SpawnState, small)`: "Start an
//!   agent" past a usage limit (start-past-limit.tsx).
//! - [`new_card_dialog`] and [`share_dialog`] are drawn by ui::view for
//!   `Dialog::NewCard` and `Dialog::Share`.
//!
//! The headless scene `card` opens the first card that needs you; with
//! `IRORI_CARD_SCENE` set it opens that card (an id or "#12"), or the New
//! card dialog ("new"), or the share dialog on the first private card
//! ("share").

mod blockers;
mod comment;
mod new_card;
mod page;
mod people;
mod share;
mod side;
mod thread;

use std::collections::{HashMap, HashSet};

use irori_window::boards::SpaceBoard;
use irori_window::cards::CardDetail;
use irori_window::events::{EVENT_BOARD_CHANGED, EVENT_ROSTER_CHANGED};
use irori_window::roster::GoneSession;
use rune::{El, TextEdit, col, el, row, text};

use crate::logic::card_status;
use crate::text::Composer;
use crate::ui::{Dialog, Event, Frame, State};

pub use comment::{open_reply, reply_box};
pub use new_card::new_card_dialog;
pub use page::start_past_limit;
pub use share::share_dialog;
pub use thread::{On, ThreadOpts, thread_view};

/// How many composers the page can hold at once: its comment box, and an
/// answer box and a resolve note per blocker. `tt::composer` reaches a
/// box through a fn pointer, so each slot has its own.
pub const SLOTS: usize = 24;

#[derive(Default)]
pub struct Model {
    /// The open card's page, read when it opens and on each change.
    pub detail: Option<CardDetail>,
    /// Why the card couldn't be read.
    pub error: Option<String>,
    /// Bumped per read: an older read's answer is dropped.
    read_gen: u64,
    /// A comment to scroll to (a notification's `task:<id>/comment:<id>`).
    pub focus_comment: Option<String>,
    focus_seen: Option<String>,
    focus_card: String,
    /// An element key to bring into view once it is laid out.
    pub scroll_to: Option<String>,
    /// Gone sessions Irori can revive (roster.Gone).
    pub gone: Vec<GoneSession>,
    /// Personal's boards (Move on a private card, New card's board).
    pub personal: Vec<SpaceBoard>,
    /// The page's transient state, reset when another card opens.
    pub ui: PageUi,
    /// Thread rows' state (opened in full, an edit, Release, Start an
    /// agent), by "<thread key>/<item id>": Talk's threads share it.
    pub rows: HashMap<String, thread::RowUi>,
    /// Threads showing every entry, by key.
    pub show_all: HashSet<String>,
    /// The composers: the comment box, answer boxes and resolve notes.
    pub boxes: Vec<Composer>,
    /// Which slot each box key holds.
    pub slot_of: HashMap<String, usize>,
    /// The comment boxes' own state (Wake, busy, who heard), by card id;
    /// Talk's reply box keeps its own under "reply/<id>".
    pub comments: HashMap<String, comment::CommentUi>,
    /// Talk's reply box (a thread's CommentBox with `reply`).
    pub reply: Composer,
    /// New card.
    pub new_card: new_card::NewCard,
    /// Share to a space.
    pub share: share::Share,
    /// The working line's changing word.
    pub ponder: i64,
    pub ponder_at: Option<std::time::Instant>,
    /// A thread drew a working line last frame (its word changes).
    pub working_line: std::cell::Cell<bool>,
}

/// What the page shows for the card on screen and forgets when another
/// opens (React's per-page useState).
#[derive(Default)]
pub struct PageUi {
    pub card: String,
    /// A note answer the person just sent (the note item leaves the card).
    pub answered: Option<blockers::Answered>,
    /// The done check lifted to the top stays there once answered.
    pub lifted: Option<String>,
    /// Per blocker: unfolded, Earlier options open, Add to your answer,
    /// Mark resolved open, busy, error, what the answer did.
    pub blockers: HashMap<String, blockers::BlockerUi>,
    /// "Close it by hand" and the ask's Close: the optional why.
    pub close_note: TextEdit,
    pub ask_note: TextEdit,
    pub pin_busy: bool,
    pub pause_busy: bool,
    pub pause_error: Option<String>,
    /// Start an agent (a given card's, past a limit): what it said.
    pub start_line: HashMap<String, String>,
    pub revived: HashSet<String>,
    /// Steps done before the split, unfolded.
    pub old_steps: bool,
    /// Move to another board, and Transfer ownership: open, the pick, busy,
    /// error.
    pub moving: Option<share::Pick>,
    pub transfer: Option<share::Pick>,
}

/// One accessor per slot (tt::composer takes fn pointers).
macro_rules! slots {
    ($($n:literal),*) => {
        const GETS: [crate::text::Get; SLOTS] = [$(|s: &State| &s.card.boxes[$n]),*];
        const GET_MUTS: [crate::text::GetMut; SLOTS] = [$(|s: &mut State| &mut s.card.boxes[$n]),*];
        /// Each slot's text, for its mic (call::mic_button).
        const EDITS: [crate::call::MicTarget; SLOTS] = [$(|s: &mut State| &mut s.card.boxes[$n].edit),*];
    };
}
slots!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23);

/// The slot holding box `key`, with its accessors.
pub(crate) fn slot(s: &State, key: &str) -> Option<(crate::text::Get, crate::text::GetMut)> {
    s.card.slot_of.get(key).map(|&i| (GETS[i], GET_MUTS[i]))
}

/// The text of the slot holding box `key`, for its mic.
pub(crate) fn slot_edit(s: &State, key: &str) -> Option<crate::call::MicTarget> {
    s.card.slot_of.get(key).map(|&i| EDITS[i])
}

/// The boxes the open card needs: its comment box, and per open (or
/// answerable) blocker an answer box and a resolve note.
fn box_keys(d: &CardDetail) -> Vec<String> {
    let id = &d.summary.id;
    let mut keys = vec![format!("{id}/comment")];
    for b in &d.blockers {
        keys.push(format!("{id}/blocker/{}", b.id));
        keys.push(format!("{id}/resolve/{}", b.id));
    }
    keys
}

/// Gives each box the page needs a slot (keeping the ones it has), and
/// loads its draft into a newly given one.
fn assign_slots(s: &mut State) {
    if s.card.boxes.len() < SLOTS {
        s.card.boxes.resize_with(SLOTS, Composer::default);
    }
    let Some(d) = &s.card.detail else { return };
    let want = box_keys(d);
    s.card.slot_of.retain(|k, _| want.contains(k));
    for k in want {
        if s.card.slot_of.contains_key(&k) {
            continue;
        }
        let used: HashSet<usize> = s.card.slot_of.values().copied().collect();
        let Some(free) = (0..SLOTS).find(|i| !used.contains(i)) else { break };
        s.card.slot_of.insert(k.clone(), free);
        crate::text::open_draft(s, GET_MUTS[free], &k);
    }
}

/// Reads the open card off the UI thread (its page builds from the store
/// and the inbox), and what its page needs beside it.
fn reload(s: &mut State) {
    let Some(id) = s.nav.card.clone() else {
        s.card.detail = None;
        s.card.error = None;
        return;
    };
    s.card.read_gen += 1;
    let generation = s.card.read_gen;
    s.spawn(
        move |w| {
            let d = w.card(&id);
            let gone = w.gone(50).unwrap_or_default();
            let personal = w.boards_personal().unwrap_or_default();
            (id, d, gone, personal)
        },
        move |s, (id, d, gone, personal)| {
            if s.card.read_gen != generation || s.nav.card.as_deref() != Some(id.as_str()) {
                return;
            }
            s.card.gone = gone;
            s.card.personal = personal;
            match d {
                Ok(d) => {
                    s.card.error = None;
                    s.card.detail = Some(d);
                    assign_slots(s);
                }
                Err(e) => {
                    s.card.error = Some(e);
                    s.card.detail = None;
                }
            }
        },
    );
}

pub fn on_event(s: &mut State, ev: &Event) {
    match ev.name.as_str() {
        "app:card-opened" => {
            let id = s.nav.card.clone().unwrap_or_default();
            if s.card.ui.card != id {
                s.card.ui = PageUi {
                    card: id.clone(),
                    ..PageUi::default()
                };
                s.card.ui.close_note = TextEdit::new(&crate::text::drafts::draft(s, &format!("{id}/close")).text);
                s.card.ui.ask_note = TextEdit::new(&crate::text::drafts::draft(s, &format!("{id}/close-ask")).text);
                if s.card.detail.as_ref().is_some_and(|d| d.summary.id != id) {
                    s.card.detail = None;
                }
                s.card.error = None;
            }
            // A notification's comment belongs to the card it opened: one
            // already shown is dropped when another card opens.
            if s.card.focus_comment.is_some() && s.card.focus_seen == s.card.focus_comment {
                if s.card.focus_card != id {
                    s.card.focus_comment = None;
                }
            } else if let Some(c) = s.card.focus_comment.clone() {
                s.card.scroll_to = Some(crate::logic::thread::thread_entry_id(&c));
                s.card.focus_seen = Some(c);
                s.card.focus_card = id.clone();
            }
            reload(s);
        }
        EVENT_BOARD_CHANGED | EVENT_ROSTER_CHANGED if s.nav.card.is_some() => reload(s),
        irori_window::boards::EVENT_BOARDS_CHANGED if s.nav.card.is_some() => reload(s),
        _ => {}
    }
    if let Some(Dialog::NewCard { .. }) = &s.dialog {
        new_card::on_event(s, ev);
    }
}

/// Moves the working line's word on (card #317) while a card is open, and
/// notices New card opening.
pub fn poll(s: &mut State, tx: &mut rune::Tx) -> bool {
    use std::time::{Duration, Instant};
    let opened = new_card::watch(s);
    if s.nav.card.is_none() || s.headless || !s.card.working_line.get() {
        return opened;
    }
    let every = Duration::from_millis(crate::logic::thread::PONDER_EVERY as u64);
    let now = Instant::now();
    match s.card.ponder_at {
        Some(at) if now.duration_since(at) < every => {
            tx.wake_after(every - now.duration_since(at));
            opened
        }
        _ => {
            s.card.ponder_at = Some(now);
            s.card.ponder += 1;
            tx.wake_after(every);
            true
        }
    }
}

/// The headless scene: the first card that needs you, else the newest;
/// `IRORI_CARD_SCENE` picks a card ("#12" or an id; "blocker", "ready",
/// "working" or "shared" for the first such), "new" or "share", or
/// "demo-blocker" (the default card with two made-up open items).
pub fn scene(s: &mut State) {
    let want = std::env::var("IRORI_CARD_SCENE").unwrap_or_default();
    match want.as_str() {
        "new" => {
            s.open_dialog(Dialog::NewCard { board: None });
            new_card::opened(s);
            return;
        }
        "share" => {
            if let Some(c) = s
                .data
                .cards
                .iter()
                .find(|c| c.scope != "board" && c.board.is_empty() && !crate::logic::cards::over(&c.state))
                .map(|c| c.id.clone())
            {
                s.open_card(&c);
                s.open_dialog(Dialog::Share { card: c });
                share::opened(s);
            }
            return;
        }
        _ => {}
    }
    let pick = if want == "blocker" || want == "ready" || want == "working" || want == "shared" {
        s.data
            .cards
            .iter()
            .find(|c| match want.as_str() {
                "blocker" => c.open_blockers > 0 && !c.ready,
                "ready" => c.ready,
                "working" => !c.working.is_empty() && !crate::logic::cards::over(&c.state),
                _ => c.scope == "board" && !c.board.is_empty(),
            })
            .map(|c| c.id.clone())
    } else if !want.is_empty() && !want.starts_with("demo") {
        let short = want.trim_start_matches('#').to_string();
        let found = s
            .data
            .cards
            .iter()
            .find(|c| c.id == want || (want.starts_with('#') && c.short == short))
            .map(|c| c.id.clone());
        if found.is_none() && want.starts_with('#') {
            // A closed card isn't in the open list: look in Closed.
            s.spawn(
                move |w| w.done_cards(500, "").ok().and_then(|p| p.cards.into_iter().find(|c| c.short == short)).map(|c| c.id),
                |s, id| {
                    if let Some(id) = id {
                        s.open_card(&id);
                    }
                },
            );
            return;
        }
        found.or(Some(want.clone()))
    } else {
        s.data
            .cards
            .iter()
            .find(|c| card_status::group(c) == card_status::Group::NeedsYou)
            .or_else(|| s.data.cards.first())
            .map(|c| c.id.clone())
    };
    if let Some(id) = pick {
        s.open_card(&id);
    }
    if want == "demo-blocker" {
        demo_blockers(s);
    }
}

/// The scene's open items, for data that has none: a question with two
/// options (one recommended, with its why and link) and one whose agent
/// is gone.
fn demo_blockers(s: &mut State) {
    use irori_window::cards::{AgentRef, CardBlocker, CardOption};
    let Some(d) = s.card.detail.as_mut() else { return };
    let oak = AgentRef {
        key: "demo/oak".into(),
        name: "oak".into(),
        status: "idle".into(),
        live: true,
        where_: "Macbook Pro".into(),
        ..AgentRef::default()
    };
    let opt = |a: &str, why: &str, rec: bool| CardOption {
        answer: a.into(),
        why: why.into(),
        by: oak.clone(),
        recommended: rec,
        ..CardOption::default()
    };
    let mut first = CardBlocker {
        id: "demo-b1".into(),
        question: "Merge the Android branch into dev now, or wait for #679?".into(),
        context: "The branch builds and the emulator checks pass. `scripts/android-apk.sh` is the only new script.".into(),
        raised_by: oak.clone(),
        state: "open".into(),
        options: vec![
            opt("Merge it now", "Nothing else waits on it, and #679 builds on it either way.", true),
            opt("Wait for #679", "", false),
        ],
        ..CardBlocker::default()
    };
    first.options[0].link = Some(Default::default());
    if let Some(l) = first.options[0].link.as_mut() {
        l.kind = "url".into();
        l.reference = "https://github.com/StephenSHorton/rune/pull/1".into();
        l.label = "GitHub #1".into();
    }
    let gone = CardBlocker {
        id: "demo-b2".into(),
        question: "Which folder should the APK go in?".into(),
        raised_by: oak.clone(),
        state: "open".into(),
        gone_agent: Some(AgentRef {
            status: "gone".into(),
            live: false,
            ..oak.clone()
        }),
        ..CardBlocker::default()
    };
    d.blockers.insert(0, gone);
    d.blockers.insert(0, first);
    d.summary.open_blockers += 2;
    assign_slots(s);
}

pub fn view(s: &State, f: &mut Frame) -> El {
    page::view(s, f)
}

// ---- shared bits ----------------------------------------------------------

pub(crate) fn ts(size: f32, weight: f32) -> rune::TextStyle {
    rune::tok::TextStyle::new(size, weight)
}

/// A titled block of a card panel (card-detail.tsx Section).
pub(crate) fn section(f: &Frame, key: &str, title: &str, last: bool, body: Vec<El>) -> El {
    let k = f.k;
    let mut c = col()
        .key(key.to_string())
        .w_full()
        .min_w(0.0)
        .gap(10.0)
        .px(20.0)
        .py(14.0)
        .child(
            text(title.to_uppercase())
                .font(ts(11.0, 500.0).tracking(0.4))
                .color(k.c.text_muted),
        )
        .children(body);
    if !last {
        c = c.border_sides([0.0, 0.0, 1.0, 0.0], k.c.border_base);
    }
    c
}

/// A card-like panel (the shadcn Card).
pub(crate) fn panel(f: &Frame, key: &str) -> El {
    col()
        .key(key.to_string())
        .w_full()
        .min_w(0.0)
        .radius(12.0)
        .border(1.0, f.k.c.border_base)
        .fill(f.k.c.layer_01)
}

/// A bar under the header (the React app's glass-panel).
pub(crate) fn bar(f: &Frame, key: &str) -> El {
    row()
        .key(key.to_string())
        .w_full()
        .min_w(0.0)
        .gap(12.0)
        .px(16.0)
        .py(10.0)
        .center_y()
        .radius(12.0)
        .border(1.0, f.k.c.border_base)
        .fill(f.k.c.layer_01)
}

/// A small muted line that wraps.
pub(crate) fn note(f: &Frame, t: impl Into<String>) -> El {
    text(t.into())
        .font(ts(12.5, 400.0))
        .color(f.k.c.text_muted)
        .w_full()
        .wrap()
}

/// A red line under a control.
pub(crate) fn err_line(f: &Frame, t: impl Into<String>) -> El {
    text(t.into())
        .font(f.k.ty.text_xsmall)
        .color(f.k.c.danger.fg)
        .w_full()
        .wrap()
}

/// A muted "Label:" before a value, both wrapping as one line would.
pub(crate) fn labelled(f: &Frame, label: &str, value: El) -> El {
    row()
        .w_full()
        .min_w(0.0)
        .gap(4.0)
        .items_start()
        .child(
            text(format!("{label}:"))
                .font(ts(12.5, 500.0))
                .color(f.k.c.text_base),
        )
        .child(value.grow(1.0).min_w(0.0))
}

pub(crate) fn spacer() -> El {
    el().grow(1.0)
}

#[cfg(test)]
mod tests;
