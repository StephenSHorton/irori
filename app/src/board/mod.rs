//! The Board's card list (P2): the Rune port of the React app's
//! cards/board-view.tsx, card-grid.tsx, order.tsx, progress-rows.tsx,
//! search-box.tsx, clear-done.tsx, threads-panel.tsx and the view switch
//! of canvas/switch.tsx. One board's cards as a page: the filter tabs and
//! the device, member and board chips, the search box, the Threads panel,
//! Pinned, the masonry grid (or Home's progress rows, or the canvas), Not
//! yet folded at the end, and Closed with its paging and clearing.
//!
//! # API (for Home and Spaces)
//!
//! - [`board_view`]`(s, f, Opts { scope, cards, home, header, lead,
//!   notice, empty, nothing_decide, threads, rows, listed, plain })`: the
//!   React BoardView. `scope` is [`SCOPE_OWN`] for Home (your private
//!   cards and the shared ones your agents are on) or a space id; `cards`
//!   are the live cards to scope (usually all of `s.data.cards`); `home`
//!   picks Home's remembered filter over the board pages' one; `threads`
//!   names the space whose Threads panel shows beside the list; `rows`
//!   draws Home's compact progress rows instead of the grid, leaving out
//!   the `listed` ids (Needs you's) on All; `plain` is a space's page (no
//!   filters, only `empty` and the threads). Returns a page that fills
//!   its parent (the list scrolls by itself).
//! - [`order::when_line`] and [`order::not_yet_button`]: a card page's
//!   "After #12, then #14" line and its Not yet popover (card #570).
//! - [`clear::clear_done`]: Clear done by age, also for Settings >
//!   Maintenance (`compact: false`).
//!
//! Home (home::view) and the `Route::Board` pages (spaces::board_page)
//! both draw [`board_view`]. List or Canvas is the canvas package's pick
//! (`canvas::wants_canvas`); its switch sits beside the search box.

pub mod bars;
pub mod clear;
pub mod done;
pub mod filter;
pub mod grid;
pub mod order;
pub mod rows;
pub mod search;
pub mod threads;

use std::collections::{HashMap, HashSet};

use irori_window::cards::CardSummary;
use irori_window::events::{EVENT_BOARD_CHANGED, EVENT_BOARDS_CHANGED};
use rune::{El, KeyEvent, ScrollSpec, TextEdit, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use crate::logic::cards::device_chips;
use crate::logic::spaces::{PERSONAL, boards_of};
use crate::ui::{Event, Frame, Route, State};

pub use filter::{Filter, SCOPE_OWN};

#[derive(Default)]
pub struct Model {
    /// The board pages' filter (not kept: React's App state).
    pub filter: Filter,
    /// Home's filter; its All, Private or board pick is kept in the prefs.
    pub home_filter: Filter,
    /// The search box's words (card #171), shared by every page.
    pub query: TextEdit,
    /// The card last opened, marked on the way back.
    pub selected: String,
    pub done: done::Model,
    pub clear: clear::Model,
    pub threads: threads::Model,
    /// Not yet folds that are open, by key.
    pub folds: HashSet<String>,
    pub order: order::Model,
}

/// What [`board_view`] shows (the React BoardView's props).
#[derive(Default)]
pub struct Opts<'a> {
    pub scope: String,
    pub cards: Vec<&'a CardSummary>,
    /// Home's remembered filter rather than the board pages' one.
    pub home: bool,
    /// The box above the board (an ask box).
    pub header: Option<El>,
    /// Leads the column, above the filters (Home: Needs you).
    pub lead: Option<El>,
    /// Under the filters.
    pub notice: Option<El>,
    /// What an empty board shows.
    pub empty: Option<El>,
    /// Needs a decision with nothing in it.
    pub nothing_decide: Option<String>,
    /// The space whose Threads panel shows beside the list.
    pub threads: Option<String>,
    /// Home's compact progress rows instead of the grid (card #261).
    pub rows: bool,
    /// The cards `lead` already lists; All leaves them out of the rows.
    pub listed: HashSet<String>,
    /// A space's page: no filters, only `empty` and the threads.
    pub plain: bool,
}

/// The live roster's status by agent key (lib/thread LiveRoster).
pub fn live_map(s: &State) -> HashMap<String, String> {
    s.data.agents.iter().map(|a| (a.card.key.clone(), a.card.status.clone())).collect()
}

/// The filter a page reads.
pub fn filter_of(s: &State, home: bool) -> Filter {
    if home { s.board.home_filter.clone() } else { s.board.filter.clone() }
}

/// Picks a filter: Home keeps its All, Private or board pick in the
/// prefs. Closed loads its first page.
pub fn set_filter(s: &mut State, home: bool, f: Filter, scope: &str) {
    if home {
        if filter::remembered_filter(&f) {
            s.prefs.set(filter::HOME_FILTER_PREF, f.id());
        }
        s.board.home_filter = f.clone();
    } else {
        s.board.filter = f.clone();
    }
    if f == Filter::Done {
        done::load(s, scope, false);
    }
}

fn members_of<'a>(s: &'a State, scope: &str) -> Option<&'a [irori_window::boards::SummaryMember]> {
    if scope == SCOPE_OWN {
        return None;
    }
    s.data.spaces.iter().find(|b| b.board_id == scope).map(|b| b.members.as_slice())
}

pub(crate) fn muted(f: &Frame, key: &str, t: &str) -> El {
    text(t.to_string())
        .key(key.to_string())
        .font(rune::tok::TextStyle::new(12.5, 400.0))
        .color(f.k.c.text_muted)
        .w_full()
        .wrap()
}

pub(crate) fn heading(f: &Frame, key: &str, t: &str, n: Option<usize>) -> El {
    let k = f.k;
    let mut r = row()
        .key(key.to_string())
        .gap(8.0)
        .center_y()
        .px(4.0)
        .child(text(t.to_uppercase()).font(rune::tok::TextStyle::new(11.0, 500.0).tracking(0.6)).color(k.c.text_muted));
    if let Some(n) = n {
        r = r.child(text(n.to_string()).font(rune::tok::TextStyle::new(11.0, 500.0)).color(k.c.text_muted));
    }
    r
}

/// One board's cards as a page (BoardView, card #161).
pub fn board_view(s: &State, f: &mut Frame, o: Opts) -> El {
    let k = f.k;
    let mine = filter::board_order(&o.cards.iter().copied().filter(|c| filter::in_scope(c, &o.scope)).collect::<Vec<_>>());
    let chips = device_chips(mine.iter().copied());
    let row_boards = (o.scope == SCOPE_OWN).then(|| filter::home_boards(&mine, &s.data.spaces));
    let members = members_of(s, &o.scope);
    let picked = if o.plain { Filter::All } else { filter_of(s, o.home) };
    let chip_ids: Vec<String> = chips.iter().map(|d| d.id.clone()).collect();
    let active = filter::active_filter(&picked, &chip_ids, row_boards.as_deref(), members);
    let query = s.board.query.text.clone();
    let searching = !query.trim().is_empty();
    let (all, context) = filter::shown_cards(&mine, &active, &query);
    let shown: Vec<&CardSummary> = if o.rows && active == Filter::All && !searching {
        all.iter().copied().filter(|c| !o.listed.contains(&c.id)).collect()
    } else {
        all.clone()
    };
    let hidden = filter::hidden_by_filter(&mine, &active, &query);
    let pinned: Vec<&CardSummary> = shown
        .iter()
        .copied()
        .filter(|c| c.pinned || c.parent.as_ref().is_some_and(|p| shown.iter().any(|x| x.id == p.id && x.pinned)))
        .collect();
    let in_pinned: HashSet<&str> = pinned.iter().map(|c| c.id.as_str()).collect();
    let not_yet: Vec<&CardSummary> = shown.iter().copied().filter(|c| !in_pinned.contains(c.id.as_str()) && c.later.is_some()).collect();
    let decide = mine.iter().filter(|c| c.open_blockers > 0).count();
    let review = mine.iter().filter(|c| c.state == crate::logic::cards::state::IN_REVIEW).count();

    let th = o.threads.as_deref().map(|b| (b, s.board.threads.list(b)));
    let open_threads = th.map_or(0, |(_, l)| l.iter().filter(|t| t.state != "resolved").count());
    let show_threads = th.is_some_and(|(_, l)| !s.board.threads.hidden && (!l.is_empty() || !s.board.threads.open.is_empty()));
    let threads_btn = th.is_some_and(|(_, l)| !show_threads && !l.is_empty());
    let canvas = crate::canvas::wants_canvas(s);
    let fill = canvas && active != Filter::Done && !mine.is_empty();
    let home = o.home;
    let scope = o.scope.clone();

    let btn = threads_btn.then(|| {
        let mut b = row()
            .key("board-threads-btn")
            .role("button")
            .label("Show the Threads panel")
            .help("Show the Threads panel")
            .focusable(true)
            .h(28.0)
            .px(10.0)
            .gap(6.0)
            .center_y()
            .radius(k.radius.md)
            .cursor(rune::Cursor::Pointer)
            .on_press(|s: &mut State| s.board.threads.hidden = false)
            .child(text("Threads").font(k.ty.small_medium).color(k.c.text_base));
        if open_threads > 0 {
            b = b.child(crate::widgets::bits::badge(
                f,
                "board-threads-btn/n",
                &open_threads.to_string(),
                crate::logic::cards::Tone::Slate,
                false,
                "",
            ));
        }
        b
    });
    // The threads are read when the page first shows them.
    let watch = o.threads.clone().filter(|b| !s.board.threads.loaded(b));

    let mut column: Vec<El> = Vec::new();
    if mine.is_empty() && picked != Filter::Done {
        if !o.plain {
            column.push(bars::filter_bar(f, home, &picked, 0, 0, &[Filter::All, Filter::Done], &scope));
        }
        if let Some(b) = btn {
            column.push(row().key("board-threads-btn-row").w_full().child(el().grow(1.0)).child(b));
        }
        column.extend(o.lead);
        column.extend(o.notice);
        column.extend(o.empty);
    } else {
        if let Some(lead) = o.lead {
            column.push(if fill { col().key("board-lead-box").w_full().max_h(260.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(lead) } else { lead });
        }
        if o.rows {
            column.push(
                text("Agent progress")
                    .key("board-progress-h")
                    .font(rune::tok::TextStyle::new(15.0, 600.0))
                    .color(k.c.text_base)
                    .px(4.0)
                    .pt(8.0),
            );
        }
        let mut left = row().key("board-filter-left").gap(12.0).center_y().flex_wrap().min_w(0.0).shrink();
        left = left.child(bars::filter_bar(f, home, &active, decide, review, &[Filter::All, Filter::Decide, Filter::Review, Filter::Done], &scope));
        if let Some(d) = bars::device_bar(f, home, &active, &chips, &scope) {
            left = left.child(d);
        }
        if let Some(ms) = members
            && let Some(m) = bars::member_bar(f, home, &active, ms, &scope)
        {
            left = left.child(m);
        }
        if let Some(rb) = &row_boards
            && let Some(b) = bars::board_bar(f, home, &active, rb, &scope)
        {
            left = left.child(b);
        }
        let mut right = row().key("board-filter-right").gap(8.0).center_y();
        if let Some(b) = btn {
            right = right.child(b);
        }
        right = right.child(search::search_box(s, f)).child(crate::canvas::view_switch(s, f));
        column.push(
            row()
                .key("board-filters")
                .w_full()
                .gap(12.0)
                .center_y()
                .flex_wrap()
                .child(left)
                .child(el().grow(1.0))
                .child(right),
        );
        column.extend(o.notice);
        if let Some((n, d)) = hidden {
            column.push(hidden_line(f, home, searching, n, d, &scope));
        }
        if active == Filter::Done {
            column.push(done::grid(s, f, &scope));
        } else if canvas {
            // The cards the filter and search show (the query narrows
            // List, Canvas and Closed alike, card #171), with the ones
            // Needs you lists kept on the map (card #587).
            column.push(crate::canvas::view_of(s, f, &all).w_full().grow(1.0).min_h(if fill { 0.0 } else { 480.0 }));
        } else if !shown.is_empty() && o.rows {
            if !pinned.is_empty() {
                column.push(heading(f, "board-pinned-h", "Pinned", None));
                column.push(rows::progress_rows(s, f, "board-rows-pinned", "Pinned cards", &pinned, false));
            }
            let rest: Vec<&CardSummary> = shown.iter().copied().filter(|c| !in_pinned.contains(c.id.as_str())).collect();
            column.push(rows::progress_rows(s, f, "board-rows", if pinned.is_empty() { "Cards" } else { "Everything else" }, &rest, true));
        } else if !shown.is_empty() {
            if !pinned.is_empty() {
                column.push(heading(f, "board-pinned-h", "Pinned", None));
                column.push(grid::card_grid(s, f, "pinned", &pinned, &context, true, None));
                column.push(heading(f, "board-else-h", "Everything else", None).pt(8.0));
            }
            let rest: Vec<&CardSummary> = shown
                .iter()
                .copied()
                .filter(|c| !in_pinned.contains(c.id.as_str()) && c.later.is_none())
                .collect();
            column.push(grid::card_grid(s, f, "main", &rest, &context, false, None));
            if !not_yet.is_empty() {
                let g = grid::card_grid(s, f, "notyet", &not_yet, &context, false, None);
                column.push(order::not_yet_fold(s, f, "board-notyet", not_yet.len(), g));
            }
        } else {
            let t = if searching {
                "No cards match.".to_string()
            } else if o.rows && !all.is_empty() {
                "Nothing else in motion.".to_string()
            } else if active == Filter::Decide && o.nothing_decide.is_some() {
                o.nothing_decide.clone().unwrap_or_default()
            } else {
                "Nothing here with this filter.".to_string()
            };
            column.push(muted(f, "board-nothing", &t));
        }
    }

    let mut main = col().key("board-column").w_full().min_w(0.0).min_h(0.0).grow(1.0).gap(14.0).px(4.0);
    for c in column {
        main = main.child(c);
    }
    let main = if fill {
        main.pb(16.0).pt(4.0)
    } else {
        col()
            .key("board-scroll")
            .grow(1.0)
            .min_w(0.0)
            .min_h(0.0)
            .h_full()
            .scroll(ScrollSpec { stick_bottom: false })
            .child(main.grow(0.0).pb(40.0))
    };
    let mut body = row().key("board-body").w_full().grow(1.0).min_h(0.0).gap(16.0).child(main);
    if show_threads && let Some((b, list)) = th {
        body = body.child(
            col()
                .key("board-threads-col")
                .w(380.0)
                .h_full()
                .min_h(0.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .child(threads::panel(s, f, b, list).mb(16.0)),
        );
    }
    let mut page = col().key("board-view").w_full().h_full().min_h(0.0).gap(14.0);
    if let Some(h) = o.header {
        page = page.child(h);
    }
    page = page.child(body);
    if let Some(b) = watch {
        page = page.on_visible(move |s: &mut State, on, _tx: &mut Tx| {
            if on {
                threads::load(s, &b);
            }
        });
    }
    page
}

/// "A filter is hiding 3 cards, 1 needs a decision  Show all" (card #237).
fn hidden_line(f: &Frame, home: bool, search: bool, n: usize, decide: usize, scope: &str) -> El {
    let k = f.k;
    let t = rune::tok::TextStyle::new(12.5, 400.0);
    let warn = crate::ui::theme::status::decide();
    let mut r = row()
        .key("board-hidden")
        .role("status")
        .w_full()
        .gap(8.0)
        .center_y()
        .flex_wrap()
        .px(10.0)
        .py(6.0)
        .radius(k.radius.md)
        .fill(warn.alpha(0.1))
        .border(1.0, warn.alpha(0.4))
        .child(
            text(format!(
                "{} is hiding {n} {}",
                if search { "A search" } else { "A filter" },
                if n == 1 { "card" } else { "cards" }
            ))
            .font(t)
            .color(k.c.text_base),
        );
    if decide > 0 {
        r = r.child(
            text(format!(", {decide} {} a decision", if decide == 1 { "needs" } else { "need" }))
                .font(rune::tok::TextStyle::new(12.5, 600.0))
                .color(warn),
        );
    }
    let scope = scope.to_string();
    r.child(
        Button::new("Show all")
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Small)
            .on_press(move |s: &mut State| {
                set_filter(s, home, Filter::All, &scope);
                s.board.query = TextEdit::default();
            })
            .build(k),
    )
}

pub fn on_event(s: &mut State, ev: &Event) {
    match ev.name.as_str() {
        "app:start" => {
            s.board.home_filter = filter::saved_home_filter(s.prefs.get::<String>(filter::HOME_FILTER_PREF).as_deref());
        }
        "app:card-opened" => {
            if let Some(id) = ev.payload.as_str() {
                s.board.selected = id.to_string();
            }
        }
        EVENT_BOARD_CHANGED => {
            done::reload_all(s);
            clear::recount(s);
            threads::reload_all(s);
        }
        EVENT_BOARDS_CHANGED => threads::reload_all(s),
        _ => {}
    }
}

/// "/" outside a text field puts the cursor in the search box (card
/// #171), on Home and the board pages, with no dialog open.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let slash = matches!(&ev.key, rune::Key::Text(t) if t == "/");
    if !slash || ev.mods.cmd || ev.mods.ctrl || ev.mods.alt || tx.focused().is_some() {
        return false;
    }
    if s.dialog.is_some() || s.nav.card.is_some() || !matches!(s.nav.route, Route::Home | Route::Board { .. }) {
        return false;
    }
    search::focus(s);
    true
}

/// The headless scene: a space's board with the most cards, its Threads
/// panel read (`--scene board`).
pub fn scene(s: &mut State) {
    s.close_card();
    // The Core opens the shared spaces a few seconds in.
    crate::spaces::wait_for_spaces(s);
    crate::canvas::set_view(s, "list");
    let mut best: Option<(String, String, usize)> = None;
    for b in &s.data.spaces {
        for x in boards_of(b) {
            let n = filter::cards_on_board(b, &s.data.cards, &x.board_id).len();
            if best.as_ref().is_none_or(|(_, _, m)| n > *m) {
                best = Some((b.board_id.clone(), x.board_id.clone(), n));
            }
        }
    }
    let own = s.data.cards.iter().filter(|c| c.board.is_empty()).count();
    match best {
        Some((space, board, n)) if n >= own.min(1) && n > 0 => {
            threads::load(s, &space);
            s.go(Route::Board { space, board: Some(board) });
        }
        _ => s.go(Route::Board {
            space: PERSONAL.into(),
            board: None,
        }),
    }
}
