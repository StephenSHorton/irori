//! Spaces (P7 Spaces and boards): the Rune port of the React app's
//! components/boards/* (boards-page.tsx, board-page.tsx, space-boards.tsx,
//! parts.tsx), settings/space-agents.tsx, space-switcher.tsx, and the
//! non-sample halves of lib/boards.ts and lib/spaces.ts (in [`logic`]).
//!
//! - [`view`] is the Spaces page (`Route::Spaces`): Personal's boards, then
//!   every shared space as a tile, the open invite over them, and New space.
//! - [`board_page`] is `Route::Board`: a space's page (its boards as
//!   tiles), one board's card list (the board package's `cards_list`), or
//!   one of Personal's boards, each under its header bar (pin, settings,
//!   rename, leave and forget; or rename and delete a board).
//! - [`switcher`] is the sidebar's space picker; [`current`] is the pick.
//!
//! What the window reads for these pages beyond `s.data.spaces` (Personal's
//! boards, how fresh each log is, the open invite, each space's agents
//! switch) lives in [`Model`] and is re-read on the backend's events.

// Home, Talk and the board package filter by the pick with these
// (cardInSpace, convInSpace, threadsSpace, presenceLine).
pub mod logic;
mod board;
mod page;
mod parts;
mod switcher;
mod ui;

use std::collections::HashMap;

use rune::{El, TextEdit};

use irori_window::agents::AgentsSettings;
use irori_window::boards::{BoardSettings, EVENT_BOARDS_CHANGED, SpaceBoard};
use irori_window::events::EVENT_BOARD_CHANGED;
use irori_window::freshness::LogFreshness;
use irori_window::invites::{EVENT_INVITE_CHANGED, InviteStatus};

use crate::ui::{Event, Frame, Route, State};

pub use board::board_page;
pub use parts::invite_panel;
pub use logic::{ALL_SPACES, PERSONAL};
pub use switcher::{switcher, switcher_folded};

/// The prefs key of the space pick (the React app's localStorage
/// `irori.space`).
pub const SPACE_PREF: &str = "spaces.pick";

/// Sent (to every package's `on_event`) when the switcher picks a space.
pub const EVENT_SPACE_PICKED: &str = "app:space-picked";

/// New space's two tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NewTab {
    #[default]
    Make,
    Join,
}

/// The New space dialog (make a space, or join one with an invite).
#[derive(Default)]
pub struct NewSpace {
    pub tab: NewTab,
    pub name: TextEdit,
    pub tag: TextEdit,
    /// The tag was typed, so it no longer follows the name.
    pub typed_tag: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub link: TextEdit,
}

/// + Add board's small form, on a space (its id) or Personal.
#[derive(Default)]
pub struct AddBoard {
    pub target: String,
    pub name: TextEdit,
    pub tag: TextEdit,
    pub error: Option<String>,
}

/// A step that asks before it acts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    Leave,
    ForgetHere,
    /// Delete the open board; its cards go to `into`.
    DeleteBoard { into: String },
}

/// The header bar's own state, for the page it was opened on.
#[derive(Default)]
pub struct Header {
    /// The route this state belongs to: a different page starts clean.
    pub at: Option<Route>,
    pub renaming: bool,
    pub name: TextEdit,
    pub error: Option<String>,
    pub menu: bool,
    pub asking: Option<Ask>,
}

/// A member action in Space settings that asks first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemberAsk {
    StepDown(String),
    Remove { person: String, name: String },
}

/// The Space settings dialog.
#[derive(Default)]
pub struct Settings {
    pub board_id: String,
    pub settings: Option<BoardSettings>,
    pub error: Option<String>,
    pub agents: Option<AgentsSettings>,
    pub agents_error: Option<String>,
    /// Reviews: the agent and repo picked to always use.
    pub agent: String,
    pub repo: String,
    pub agent_open: bool,
    pub repo_open: bool,
    pub asking: Option<MemberAsk>,
    /// A failed confirm step's error, under its member's row.
    pub member_error: Option<(String, String)>,
    pub invite_error: Option<String>,
}

#[derive(Default)]
pub struct Model {
    /// Personal's boards, its default first.
    pub personal: Vec<SpaceBoard>,
    pub personal_error: Option<String>,
    pub personal_loaded: bool,
    /// How fresh each log is, by log ("personal" or a space's id).
    pub fresh: HashMap<String, LogFreshness>,
    /// The invite shown (open, or ended and not yet closed).
    pub invite: Option<InviteStatus>,
    /// The inviter's typed words, and their error.
    pub words: TextEdit,
    pub words_error: Option<String>,
    pub copied: bool,
    /// Why an invite link may not reach far ("" when it should).
    pub reach: String,
    pub new_space: Option<NewSpace>,
    pub add: Option<AddBoard>,
    pub header: Header,
    pub list_header: Header,
    pub settings: Option<Settings>,
    /// Each shared space's agents switch (the "agents are off" line).
    pub agents: HashMap<String, AgentsSettings>,
    pub agents_error: Option<String>,
    pub switcher_open: bool,
    ticker: bool,
}

/// The picked space: All spaces, Personal or a shared space's id. Until
/// the spaces load, a shared pick can't be checked, so it is kept.
pub fn current(s: &State) -> String {
    let raw = s.prefs.get::<String>(SPACE_PREF).unwrap_or_default();
    if s.data.spaces.is_empty() && raw != PERSONAL {
        return raw;
    }
    logic::known(&raw, &s.data.spaces)
}

/// Picks the space Home, Talk and Spaces show (kept in the prefs).
pub fn pick(s: &mut State, id: &str) {
    if id == ALL_SPACES {
        s.prefs.remove(SPACE_PREF);
    } else {
        s.prefs.set(SPACE_PREF, id);
    }
    let ev = Event {
        name: EVENT_SPACE_PICKED.into(),
        payload: serde_json::Value::String(id.to_string()),
    };
    crate::ui::dispatch(s, &ev);
}

/// Opens New space over the Spaces page (the palette's "New space").
pub fn open_new_space(s: &mut State) {
    s.close_card();
    s.go(Route::Spaces);
    s.spaces.new_space = Some(NewSpace::default());
}

/// Re-reads Personal's boards.
pub(crate) fn reload_personal(s: &mut State) {
    s.spawn(
        |w| w.boards_personal(),
        |s, r| {
            s.spaces.personal_loaded = true;
            match r {
                Ok(v) => {
                    s.spaces.personal = v;
                    s.spaces.personal_error = None;
                    // Deleted on another device (card #563): its cards are
                    // on the default board now, so that is where it goes.
                    if let Route::Board { space, board: Some(b) } = &s.nav.route {
                        let gone = space == PERSONAL
                            && !s.spaces.personal.is_empty()
                            && !s.spaces.personal.iter().any(|x| x.board_id == *b);
                        if gone {
                            s.nav.route = Route::Board {
                                space: PERSONAL.into(),
                                board: None,
                            };
                        }
                    }
                }
                Err(e) => s.spaces.personal_error = Some(logic::error_text(&e)),
            }
        },
    );
}

/// Re-reads the spaces list (the shared one every page reads).
pub(crate) fn reload_spaces(s: &mut State) {
    s.spawn(|w| w.boards_list(), |s, v| s.data.spaces = v);
}

fn reload_fresh(s: &mut State) {
    s.spawn(
        |w| w.freshness(),
        |s, v| s.spaces.fresh = v.into_iter().map(|f| (f.log.clone(), f)).collect(),
    );
}

fn reload_agents(s: &mut State) {
    let ids: Vec<String> = s.data.spaces.iter().map(|b| b.board_id.clone()).collect();
    s.spawn(
        move |w| {
            ids.iter()
                .map(|id| (id.clone(), w.agents_settings(id)))
                .collect::<HashMap<_, _>>()
        },
        |s, v| s.spaces.agents = v,
    );
}

/// The invite as it is now: shown while open, or while an ended one is
/// still on screen.
fn reload_invite(s: &mut State) {
    s.spawn(
        |w| w.boards_invite_status(),
        |s, st| {
            if st.open() || (s.spaces.invite.is_some() && !st.state.is_empty()) {
                s.spaces.invite = Some(st);
            }
        },
    );
}

pub(crate) fn reload_invite_now(s: &mut State) {
    reload_invite(s);
}

pub(crate) fn reload_reach(s: &mut State) {
    s.spawn(
        |w| w.home_server(),
        |s, h| s.spaces.reach = logic::invite_reach_line(&h),
    );
}

/// The open board page's presence and the freshness lines move without an
/// event: re-read them every 20 s while a Spaces page shows (React's
/// PresenceReload and the freshness poll).
fn start_ticker(s: &mut State) {
    if s.spaces.ticker || s.headless {
        return;
    }
    let Some(h) = s.handle.clone() else { return };
    s.spaces.ticker = true;
    std::thread::Builder::new()
        .name("irori-spaces-tick".into())
        .spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(logic::PRESENCE_RELOAD_SECS));
                h.update(|s: &mut State, _| {
                    let shown = s.nav.card.is_none()
                        && matches!(s.nav.route, Route::Spaces | Route::Board { .. });
                    if shown {
                        reload_spaces(s);
                        reload_fresh(s);
                    }
                });
            }
        })
        .ok();
}

pub fn on_event(s: &mut State, ev: &Event) {
    start_ticker(s);
    match ev.name.as_str() {
        "app:start" => {
            reload_personal(s);
            reload_fresh(s);
            reload_agents(s);
            reload_invite(s);
            reload_reach(s);
        }
        EVENT_BOARDS_CHANGED => {
            reload_personal(s);
            reload_fresh(s);
            reload_agents(s);
        }
        EVENT_INVITE_CHANGED => reload_invite(s),
        // A space's agents switch set elsewhere (Settings, Agents, or
        // another device) lands as a board change.
        EVENT_BOARD_CHANGED if matches!(s.nav.route, Route::Board { .. }) => reload_agents(s),
        _ => {}
    }
}

/// The Spaces page.
pub fn view(s: &State, f: &mut Frame) -> El {
    page::view(s, f)
}

/// The headless scene that shows it (`irori --snap x.png --scene spaces`).
/// The Core opens the shared spaces a few seconds after launch, and a
/// headless clock doesn't wait for it: give it up to 12 s here so the
/// scene shows them.
pub fn scene(s: &mut State) {
    s.close_card();
    s.go(Route::Spaces);
    wait_for_spaces(s);
}

/// (Headless scenes only.) Waits for the Core's spaces to open.
pub fn wait_for_spaces(s: &mut State) {
    let Some(w) = s.w.clone() else { return };
    for _ in 0..60 {
        let list = w.boards_list();
        if !list.is_empty() {
            s.data.spaces = list;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    if let Ok(v) = w.boards_personal() {
        s.spaces.personal = v;
    }
    s.spaces.fresh = w.freshness().into_iter().map(|f| (f.log.clone(), f)).collect();
    let ids: Vec<String> = s.data.spaces.iter().map(|b| b.board_id.clone()).collect();
    s.spaces.agents = ids.iter().map(|id| (id.clone(), w.agents_settings(id))).collect();
}
