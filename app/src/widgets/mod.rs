//! Card and agent pieces several pages share (F0c: the React app's
//! cards/bits.tsx, faces.tsx, profile/face.tsx, agent-hover.tsx,
//! agent-strip.tsx, spawned-actions.tsx, done-rounds.tsx, check-slots.tsx
//! and common/confirm-button.tsx, choice-select.tsx,
//! agents-now-toggle.tsx, loading-shapes.tsx). The words and states they
//! show come from [`crate::logic`]; these only draw them.
//!
//! # API (for the screen packages)
//!
//! Every builder takes the [`Frame`] and a `key` that must be unique in the
//! view (prefix it with your package and the card or agent id: keys share
//! one namespace). Builders that read live data take `&State`.
//!
//! - [`bits`]: `badge(f, key, label, Tone, round, tip)` is the one chip
//!   (Tone from `logic::cards::Tone`: Info, Destructive, Success, Warning,
//!   Slate, Outline, Secondary); `tone_colors`/`tone_solid` give a tone's
//!   colours. Card pieces: `state_pill(f, key, &card)`,
//!   `progress_bar(f, key, &card) -> Option`, `holder_line(s, f, key,
//!   &card)` (by the live roster), `ask_line(f, key, &ask)`,
//!   `scope_badge`, `side_badge -> Option`, `pr_chip -> Option` (press
//!   opens the PR), `owner_chip(s, ..) -> Option`, `short_ref -> Option`,
//!   `spinner(f, key)`, `asker(f, key, &who)`. Show the holder line in
//!   place of the bar when a card has no steps, and under it while
//!   `logic::cards::holder_line_shows`; an open ask shows `ask_line`
//!   instead (`logic::cards::open_ask`).
//! - [`faces`]: `face(s, f, key, &who, FaceOpts{watching, no_dot,
//!   no_hover, inline})` (an agent's face opens its hover card),
//!   `faces(s, f, key, &[who], watching, Some(FACES_MAX))`,
//!   `talk_bubble -> Option`, `live_dot()`, `identicon(seed, size,
//!   color)`, `device_color(name)`, `face_color(f, status, device)`, and
//!   a person's face: `person_face(f, key, name, color, pixels, size)`,
//!   `my_face(s, f, key, size)` (the person's avatar from the window).
//! - [`agent_hover`]: `trigger(s, f, key, &who, el)` makes any element
//!   open `who`'s hover card (faces already do); the card itself is drawn
//!   by [`overlay`].
//! - [`agent_strip::agent_strip`]: the sidebar's Agents section (counts,
//!   Yours, On cards, check slots); `strip` and `on_cards` alone.
//! - [`spawned_actions::spawned_actions(s, f, key, &who, card_id,
//!   &[Action])`](spawned_actions::spawned_actions): Attach, Logs, Stop,
//!   End for an agent Irori started (`spawned_actions::ALL`). Attach opens
//!   the card and shows the new tab in its terminal (`term::opened`).
//! - [`done_rounds`]: `sent_back_signs(f, key, &blocker) -> Option`,
//!   `earlier_round(s, f, key, &blocker, compact) -> Option`,
//!   `round_signs`.
//! - [`confirm`]: `confirm_button(s, f, key, label, icon, disabled,
//!   Ask::new(question, confirm_label, |w| work).rich().danger().done(|s|
//!   ..))`, or `confirm::open(s, key, ask)` from a menu item. The work runs
//!   off the UI thread; its error shows under the button.
//! - [`choice::choice_select(s, f, key, label, value, &[Choice],
//!   disabled, width, |s, v| ..)`](choice::choice_select): a select over
//!   value/label choices (grouped with `Choice::in_group`).
//! - [`agents_now::agents_now_toggle(f, key, on, disabled, |s, on|
//!   ..)`](agents_now::agents_now_toggle).
//! - [`loading`]: skeletons in the shape of what loads: `block`,
//!   `needs_you_shape`, `need_row_shape`, `tile_grid_shape`,
//!   `card_tile_shape`, `list_rows_shape`, `message_rows_shape`,
//!   `post_rows_shape`.
//! - [`avatar`]: `is_avatar`, `initials`, `crop_square`, `decode`.
//! - [`check_slots`]: the check.sh runs holding this device's slots
//!   (`Window::check_slots`, crates/window/src/rune_widgets.rs).
//!
//! The widgets' own state (the open hover card, confirm and Logs dialogs,
//! Offer it cards in flight, the avatar, the check slots) is [`Model`],
//! `s.widgets`. The headless scene `widgets` shows a gallery of all of
//! them on live data.

pub mod agent_hover;
pub mod agent_strip;
pub mod agents_now;
pub mod avatar;
pub mod bits;
pub mod check_slots;
pub mod choice;
pub mod confirm;
pub mod done_rounds;
pub mod faces;
pub mod gallery;
pub mod loading;
pub mod spawned_actions;

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use irori_window::events::{EVENT_BOARD_CHANGED, EVENT_DEVICES_CHANGED, EVENT_ROSTER_CHANGED};
use irori_window::rune_widgets::CheckSlot;
use rune::{El, TextEdit};

use crate::ui::{Event, Frame, State};

pub use agent_strip::agent_strip;

#[derive(Default)]
pub struct Model {
    /// The headless gallery (`--scene widgets`) shows instead of the page.
    pub gallery: bool,
    /// The open hover card, and when it closes (the pointer left).
    pub hover: Option<agent_hover::Hover>,
    pub hover_close_at: Option<Instant>,
    /// The hover card's Stop: its why, in flight, its answer.
    pub stop_draft: TextEdit,
    pub stop_busy: bool,
    pub stop_line: Option<(bool, String)>,
    /// "Offer it cards" the person set, until the roster says so; its
    /// error by agent key.
    pub offer_want: HashMap<String, bool>,
    pub offer_error: HashMap<String, String>,
    /// The open confirm dialog; errors and work in flight by button key.
    pub confirm: Option<confirm::Open>,
    pub confirm_errors: HashMap<String, String>,
    pub busy: HashSet<String>,
    /// Spawned actions' answers by agent key, and agents asked to end.
    pub lines: HashMap<String, (bool, String)>,
    pub ended: HashSet<String>,
    pub logs: Option<spawned_actions::Logs>,
    /// The open choice select (its key).
    pub select_open: Option<String>,
    /// Earlier rounds whose proof is shown (by key).
    pub round_proof: HashSet<String>,
    /// The person's avatar and colour.
    pub avatar: avatar::Avatar,
    pub person_color: String,
    /// This device's check slots, and when they were last read.
    pub check_slots: Vec<CheckSlot>,
    pub slots_at: Option<Instant>,
    /// The gallery's card page read (done rounds, people rows).
    pub sample: Option<irori_window::cards::CardDetail>,
}

/// Reads the person's avatar and colour off the UI thread.
fn refresh_person(s: &mut State) {
    s.spawn(
        |w| {
            let url = w.person_avatar();
            let rgba = avatar::decode(&url);
            (url, rgba, w.person_color())
        },
        |s, (url, rgba, color)| {
            if s.widgets.avatar.url != url || s.widgets.avatar.pixels.is_none() {
                s.widgets.avatar = avatar::Avatar::from_rgba(url, rgba);
            }
            s.widgets.person_color = color;
        },
    );
}

pub fn on_event(s: &mut State, ev: &Event) {
    match ev.name.as_str() {
        "app:start" => {
            refresh_person(s);
            check_slots::refresh(s);
        }
        EVENT_DEVICES_CHANGED => refresh_person(s),
        EVENT_BOARD_CHANGED => check_slots::refresh(s),
        EVENT_ROSTER_CHANGED => {
            // The roster caught up with "Offer it cards": show it again.
            let agents = &s.data.agents;
            s.widgets.offer_want.retain(|k, want| {
                !agents
                    .iter()
                    .any(|a| a.card.key == *k && a.offers_cards == *want)
            });
            // An agent that left closes its hover card.
            if let Some(h) = &s.widgets.hover
                && !s.data.agents.iter().any(|a| a.card.key == h.who.key)
                && !s.data.cards.iter().any(|c| {
                    c.working
                        .iter()
                        .chain(&c.watching)
                        .any(|w| w.key == h.who.key)
                })
            {
                s.widgets.hover = None;
            }
            check_slots::refresh(s);
        }
        _ => {}
    }
}

/// The widgets' overlays: the hover card, the confirm and Logs dialogs.
pub fn overlay(s: &State, f: &mut Frame) {
    agent_hover::card(s, f);
    confirm::dialog(s, f);
    spawned_actions::logs_dialog(s, f);
}

/// Closes the hover card once its grace is up; re-reads the check slots
/// every [`check_slots::POLL`]. True when something visible changed.
pub fn poll(s: &mut State, tx: &mut rune::Tx) -> bool {
    let now = Instant::now();
    let mut changed = false;
    if let Some(at) = s.widgets.hover_close_at {
        if at <= now {
            s.widgets.hover = None;
            s.widgets.hover_close_at = None;
            changed = true;
        } else {
            tx.wake_after(at - now);
        }
    }
    if !s.headless && s.w.is_some() {
        match s.widgets.slots_at {
            Some(t) if now.duration_since(t) < check_slots::POLL => {
                tx.wake_after(check_slots::POLL - now.duration_since(t));
            }
            _ => {
                check_slots::refresh(s);
                tx.wake_after(check_slots::POLL);
            }
        }
    }
    changed
}

/// The gallery page (the `widgets` scene), when it is on.
pub fn page(s: &State, f: &mut Frame) -> Option<El> {
    s.widgets.gallery.then(|| gallery::view(s, f))
}

/// The headless scene: a gallery of the widgets on live data, with an
/// agent's hover card open.
pub fn scene(s: &mut State) {
    s.close_card();
    s.widgets.gallery = true;
    gallery::prepare(s);
}
