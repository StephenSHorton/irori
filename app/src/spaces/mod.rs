//! Spaces (P7 Spaces and boards): the Rune port of the React app's
//! components/boards/* (board-tiles.tsx, board-page.tsx, space-boards.tsx,
//! parts.tsx), settings/space-agents.tsx, space-switcher.tsx, and the
//! non-sample halves of lib/boards.ts and lib/spaces.ts (in [`logic`]).
//!
//! Card #689 dropped the Spaces page: the sidebar's space picker
//! ([`switcher`]) picks, makes, joins and opens spaces, and the Boards tab
//! shows only the picked space's boards.
//!
//! - [`view`] is `Route::Spaces`, Boards with All spaces picked: it asks
//!   for a space (parts.tsx `PickSpace`).
//! - [`board_page`] is `Route::Board`: a space's page (its boards as
//!   tiles), one board's card list (the board package's `cards_list`),
//!   Personal's page (its boards), or one of Personal's boards, each under
//!   its header bar (pin, settings, rename, leave and forget; or rename and
//!   delete a board).
//! - [`new_space_dialog`] is New or join a space (`Dialog::NewSpace`).
//! - [`current`] is the pick; [`pick`] picks (App.tsx `pickSpace`), and
//!   the pick follows the page ([`follow`]).
//!
//! What the window reads for these pages beyond `s.data.spaces` (Personal's
//! boards, how fresh each log is, the open invite, each space's agents
//! switch) lives in [`Model`] and is re-read on the backend's events.

// Home, Talk and the board package filter by the pick with these
// (cardInSpace, convInSpace, threadsSpace, presenceLine).
mod board;
pub mod logic;
mod page;
mod parts;
mod switcher;
mod ui;

use std::collections::HashMap;

use rune::{El, TextEdit};

use irori_window::agents::AgentsSettings;
use irori_window::boards::{BoardSettings, BoardSummary, EVENT_BOARDS_CHANGED, SpaceBoard};
use irori_window::events::EVENT_BOARD_CHANGED;
use irori_window::freshness::LogFreshness;
use irori_window::invites::{EVENT_INVITE_CHANGED, InviteStatus};

use crate::ui::{Event, Frame, Route, State};

pub use board::board_page;
pub use logic::{ALL_SPACES, PERSONAL};
pub use parts::{invite_panel, new_space_dialog};
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
    DeleteBoard {
        into: String,
    },
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
    /// The space page the pick last followed (App.tsx syncedPage): the
    /// pick follows the page only when the page changes, so a pick made
    /// since isn't undone.
    pub synced_page: String,
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

/// Sets the pick (kept in the prefs) and tells every package.
fn set_pick(s: &mut State, id: &str) {
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

/// A pick in the space picker (or Profile, or Talk): App.tsx `pickSpace`.
/// Boards show only the picked space's (card #689), so on Boards the page
/// follows the pick, closing a card open over it. From the pick-a-space
/// page the space's page takes its place, so Back doesn't land there and
/// bounce.
pub fn pick(s: &mut State, id: &str) {
    if id == current(s) {
        return;
    }
    set_pick(s, id);
    if s.nav.route == Route::Spaces {
        replace(s, Route::boards_for(id));
    } else if s.nav.route.on_boards() {
        s.go(Route::boards_for(id));
    }
}

/// Moves the window in place of where it is (lib/nav.ts replace).
fn replace(s: &mut State, r: Route) {
    s.nav.replace(r);
    s.nav.save(&mut s.prefs);
    let ev = Event {
        name: "app:route-changed".into(),
        payload: serde_json::Value::Null,
    };
    crate::ui::dispatch(s, &ev);
}

/// What following the page does (App.tsx's pick/page effect).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Follow {
    Nothing,
    /// The pick-a-space page with a space picked: that space's page
    /// instead.
    Replace(Route),
    /// A space's page opened another way (a pinned space, the palette, a
    /// link): it picks that space.
    Pick(String),
}

/// The pick and the page follow each other. Over a card nothing moves
/// (a pick made there sticks). On the pick-a-space page a known pick goes
/// to its space's page, once the list has loaded (no redirect on a stale
/// pick). A space's page picks its space, once per page (`synced`).
pub(crate) fn follow(
    route: &Route,
    card_open: bool,
    pick: &str,
    spaces: &[BoardSummary],
    loaded: bool,
    synced: &mut String,
) -> Follow {
    if card_open {
        return Follow::Nothing;
    }
    let known = |id: &str| id == PERSONAL || spaces.iter().any(|b| b.board_id == id);
    if *route == Route::Spaces && pick != ALL_SPACES && loaded && known(pick) {
        return Follow::Replace(Route::boards_for(pick));
    }
    let Route::Board { space, .. } = route else {
        synced.clear();
        return Follow::Nothing;
    };
    if synced == space {
        return Follow::Nothing;
    }
    let out = if space != pick && known(space) {
        Follow::Pick(space.clone())
    } else {
        Follow::Nothing
    };
    if space == PERSONAL || loaded {
        *synced = space.clone();
    }
    out
}

/// Runs [`follow`] on the window as it is now.
fn follow_page(s: &mut State) {
    for _ in 0..2 {
        let mut synced = std::mem::take(&mut s.spaces.synced_page);
        let step = follow(
            &s.nav.route,
            s.nav.card.is_some(),
            &current(s),
            &s.data.spaces,
            !s.data.spaces.is_empty(),
            &mut synced,
        );
        s.spaces.synced_page = synced;
        match step {
            Follow::Nothing => return,
            Follow::Replace(r) => {
                s.nav.replace(r);
                s.nav.save(&mut s.prefs);
            }
            Follow::Pick(id) => set_pick(s, &id),
        }
    }
}

pub use board::start_space_rename;

/// Opens New or join a space (card #689: the space picker's, the
/// palette's and Profile's), over whatever page is showing. It reads the
/// invite afresh: an open one shows inside it.
pub fn open_new_space(s: &mut State) {
    // The name field takes the keyboard (autoFocus).
    let mut n = NewSpace::default();
    n.name.focused = true;
    s.spaces.new_space = Some(n);
    s.spaces.switcher_open = false;
    s.spaces.invite = None;
    reload_invite(s);
    s.open_dialog(crate::ui::Dialog::NewSpace);
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
                    if let Route::Board {
                        space,
                        board: Some(b),
                    } = &s.nav.route
                    {
                        let gone = space == PERSONAL
                            && !s.spaces.personal.is_empty()
                            && !s.spaces.personal.iter().any(|x| x.board_id == *b);
                        if gone {
                            s.nav.route = Route::Board {
                                space: PERSONAL.into(),
                                board: Some(PERSONAL.into()),
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
    s.spawn(
        |w| w.boards_list(),
        |s, v| {
            s.data.spaces = v;
            follow_page(s);
        },
    );
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
                // The host's words box takes the keyboard as it shows
                // (autoFocus).
                let was = s
                    .spaces
                    .invite
                    .as_ref()
                    .map(|i| i.state.clone())
                    .unwrap_or_default();
                if logic::words_box_shows(&was, &st) {
                    s.spaces.words.focused = true;
                }
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
            // The Core opens the shared spaces a few seconds after launch;
            // a list read before then is empty. For the first minute an
            // empty list is read again every 2 s, so the sidebar and the
            // Spaces page don't wait for the next event or the 20 s tick.
            let started = std::time::Instant::now();
            let mut last = std::time::Instant::now();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                let early = started.elapsed().as_secs() < 60;
                let tick = last.elapsed().as_secs() >= logic::PRESENCE_RELOAD_SECS;
                if tick {
                    last = std::time::Instant::now();
                } else if !early {
                    continue;
                }
                h.update(move |s: &mut State, _| {
                    let shown = s.nav.card.is_none()
                        && matches!(s.nav.route, Route::Spaces | Route::Board { .. });
                    if (shown && tick) || (early && s.data.spaces.is_empty()) {
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
            follow_page(s);
            reload_personal(s);
            reload_fresh(s);
            reload_agents(s);
            reload_invite(s);
            reload_reach(s);
        }
        "app:route-changed" => follow_page(s),
        EVENT_BOARDS_CHANGED => {
            follow_page(s);
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

/// Boards with All spaces picked: pick a space.
pub fn view(s: &State, f: &mut Frame) -> El {
    page::view(s, f)
}

/// The headless scene that shows it (`irori --snap x.png --scene spaces`).
/// The Core opens the shared spaces a few seconds after launch, and a
/// headless clock doesn't wait for it: give it up to 12 s here so the
/// scene shows them.
///
/// With no other setting it is Boards with All spaces picked (pick a
/// space). IRORI_SCENE_SPACE picks a space and shows its page: a space's
/// id, `personal` (Personal's page), `personal/<board>` (one of its
/// boards), or `first` (the first shared space). IRORI_SCENE_NEW=1 opens
/// New or join a space over it.
///
/// IRORI_SCENE_INVITE shows a stand-in invite (no network needed), inside
/// New or join a space: `waiting` (the host's link), `words-host` (the
/// host types the five words, one miss), `words-guest` (the guest reads
/// them out), `joining`, `done` or `failed`.
pub fn scene(s: &mut State) {
    s.close_card();
    wait_for_spaces(s);
    let want = std::env::var("IRORI_SCENE_SPACE").unwrap_or_default();
    let (space, board) = match want.split_once('/') {
        Some((a, b)) => (a.to_string(), Some(b.to_string())),
        None => (want, None),
    };
    let space = if space == "first" {
        s.data
            .spaces
            .first()
            .map(|b| b.board_id.clone())
            .unwrap_or_default()
    } else {
        space
    };
    set_pick(s, &space);
    s.spaces.synced_page.clear();
    match board {
        Some(b) => s.go(Route::Board {
            space,
            board: Some(b),
        }),
        None => s.go(Route::boards_for(&space)),
    }
    let invite = std::env::var("IRORI_SCENE_INVITE")
        .ok()
        .and_then(|step| scene_invite(&step));
    if std::env::var("IRORI_SCENE_NEW").is_ok() || invite.is_some() {
        open_new_space(s);
    }
    if let Some(i) = invite {
        if logic::words_box_shows("", &i) {
            s.spaces.words.focused = true;
            if let Some(n) = s.spaces.new_space.as_mut() {
                n.name.focused = false;
            }
        }
        s.spaces.invite = Some(i);
    }
}

/// A stand-in invite at `step` (see [`scene`]).
fn scene_invite(step: &str) -> Option<InviteStatus> {
    use irori_window::invites::{
        INVITE_DONE, INVITE_FAILED, INVITE_JOINING, INVITE_WAITING, INVITE_WORDS,
    };
    let mut v = InviteStatus {
        role: "host".into(),
        board_name: "Netcode Lab".into(),
        peer_name: "Casey".into(),
        link: "irori-invite:v1:3q2hx7k0cz9m4t1wq8n6p5r2s4u7y3e0a9d8f6g5h4j2k1l0".into(),
        peer_fingerprint: "7c1e 90ab 44d2 e8f3".into(),
        my_fingerprint: "eda0 0926 ae6b 1e18".into(),
        words: ["harbour", "lantern", "otter", "maple", "drift"]
            .map(String::from)
            .to_vec(),
        ..InviteStatus::default()
    };
    v.state = match step {
        "waiting" => INVITE_WAITING,
        "words-host" => {
            v.misses = 1;
            INVITE_WORDS
        }
        "words-guest" => {
            v.role = "guest".into();
            INVITE_WORDS
        }
        "joining" => INVITE_JOINING,
        "done" => INVITE_DONE,
        "failed" => {
            v.error = "Those words didn't match twice, so the invite ended.".into();
            INVITE_FAILED
        }
        _ => return None,
    }
    .into();
    Some(v)
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
    s.spaces.fresh = w
        .freshness()
        .into_iter()
        .map(|f| (f.log.clone(), f))
        .collect();
    let ids: Vec<String> = s.data.spaces.iter().map(|b| b.board_id.clone()).collect();
    s.spaces.agents = ids
        .iter()
        .map(|id| (id.clone(), w.agents_settings(id)))
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn space(id: &str) -> BoardSummary {
        BoardSummary {
            board_id: id.into(),
            name: id.into(),
            ready: true,
            ..BoardSummary::default()
        }
    }

    fn board(id: &str) -> Route {
        Route::Board {
            space: id.into(),
            board: None,
        }
    }

    #[test]
    fn the_pick_a_space_page_follows_a_known_pick() {
        let spaces = vec![space("b1")];
        let mut synced = String::new();
        assert_eq!(
            follow(&Route::Spaces, false, "b1", &spaces, true, &mut synced),
            Follow::Replace(board("b1"))
        );
        assert_eq!(
            follow(&Route::Spaces, false, PERSONAL, &spaces, true, &mut synced),
            Follow::Replace(board(PERSONAL))
        );
        // All spaces stays to ask; a stale pick doesn't redirect, nor one
        // made before the list has loaded.
        assert_eq!(
            follow(
                &Route::Spaces,
                false,
                ALL_SPACES,
                &spaces,
                true,
                &mut synced
            ),
            Follow::Nothing
        );
        assert_eq!(
            follow(&Route::Spaces, false, "gone", &spaces, true, &mut synced),
            Follow::Nothing
        );
        assert_eq!(
            follow(&Route::Spaces, false, "b1", &[], false, &mut synced),
            Follow::Nothing
        );
    }

    #[test]
    fn a_spaces_page_picks_its_space_once() {
        let spaces = vec![space("b1"), space("b2")];
        let mut synced = String::new();
        assert_eq!(
            follow(&board("b1"), false, ALL_SPACES, &spaces, true, &mut synced),
            Follow::Pick("b1".into())
        );
        assert_eq!(synced, "b1");
        // A pick made since on the same page isn't undone.
        assert_eq!(
            follow(&board("b1"), false, "b2", &spaces, true, &mut synced),
            Follow::Nothing
        );
        // One of its boards is the same page.
        let list = Route::Board {
            space: "b1".into(),
            board: Some("x".into()),
        };
        assert_eq!(
            follow(&list, false, "b2", &spaces, true, &mut synced),
            Follow::Nothing
        );
        // Away and back picks it again.
        assert_eq!(
            follow(&Route::Home, false, "b2", &spaces, true, &mut synced),
            Follow::Nothing
        );
        assert_eq!(synced, "");
        assert_eq!(
            follow(&board("b1"), false, "b2", &spaces, true, &mut synced),
            Follow::Pick("b1".into())
        );
    }

    #[test]
    fn a_pick_over_a_card_sticks() {
        let spaces = vec![space("b1")];
        let mut synced = String::new();
        assert_eq!(
            follow(&board("b1"), true, ALL_SPACES, &spaces, true, &mut synced),
            Follow::Nothing
        );
        assert_eq!(
            follow(&Route::Spaces, true, "b1", &spaces, true, &mut synced),
            Follow::Nothing
        );
    }

    #[test]
    fn an_unknown_page_waits_for_the_list() {
        let mut synced = String::new();
        // Not loaded: it isn't marked synced, so it picks once it loads.
        assert_eq!(
            follow(&board("b1"), false, ALL_SPACES, &[], false, &mut synced),
            Follow::Nothing
        );
        assert_eq!(synced, "");
        let spaces = vec![space("b1")];
        assert_eq!(
            follow(&board("b1"), false, ALL_SPACES, &spaces, true, &mut synced),
            Follow::Pick("b1".into())
        );
        // Personal is always known.
        let mut synced = String::new();
        assert_eq!(
            follow(&board(PERSONAL), false, ALL_SPACES, &[], false, &mut synced),
            Follow::Pick(PERSONAL.into())
        );
    }

    #[test]
    fn a_pick_on_boards_moves_the_page_and_back_doesnt_bounce() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = State {
            prefs: crate::ui::prefs::Prefs::load(dir.path()),
            ..State::default()
        };
        s.data.spaces = vec![space("b1"), space("b2")];
        s.go(Route::Notes);
        s.go(Route::Spaces);
        pick(&mut s, "b1");
        assert_eq!(current(&s), "b1");
        assert_eq!(s.nav.route, board("b1"));
        // The pick-a-space page was replaced: Back goes to Notes.
        s.back();
        assert_eq!(s.nav.route, Route::Notes);
        // On a space's page a pick goes to the other's, closing a card.
        s.go(board("b1"));
        s.open_card("c1");
        pick(&mut s, "b2");
        assert_eq!(s.nav.route, board("b2"));
        assert_eq!(s.nav.card, None);
        // Elsewhere the page stays.
        s.go(Route::Talk(None));
        pick(&mut s, PERSONAL);
        assert_eq!(s.nav.route, Route::Talk(None));
        assert_eq!(current(&s), PERSONAL);
    }

    #[test]
    fn new_or_join_a_space_opens_over_the_page() {
        let mut s = State::default();
        s.go(Route::Home);
        s.spaces.invite = Some(InviteStatus {
            state: "failed".into(),
            ..InviteStatus::default()
        });
        open_new_space(&mut s);
        assert_eq!(s.nav.route, Route::Home);
        assert_eq!(s.dialog, Some(crate::ui::Dialog::NewSpace));
        assert!(s.spaces.new_space.as_ref().is_some_and(|n| n.name.focused));
        // It reads the invite afresh, not the last one's state.
        assert!(s.spaces.invite.is_none());
        parts::close_new_space(&mut s);
        assert!(s.dialog.is_none());
        assert!(s.spaces.new_space.is_none());
    }
}
