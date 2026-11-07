//! The Board's card list (P2: the React app's cards/board-view.tsx,
//! card-grid.tsx and their filters): the cards in the Board's groups.
//! Pressing a card opens its page (the card package).
//!
//! A space's board page (`Route::Board`) shows the cards on that space;
//! Home shows them too, under Needs you (the home package calls
//! [`cards`]).

use rune::{El, ScrollSpec, col, el, row, text};

use irori_window::cards::CardSummary;

use crate::logic::card_status::{self, GROUPS, Group};
use crate::ui::{Event, Frame, Route, State};

/// Which cards the list shows (the filter bar, cut down).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Decide,
    Working,
}

#[derive(Default)]
pub struct Model {
    pub filter: Filter,
}

fn shown(c: &CardSummary, f: Filter) -> bool {
    match f {
        Filter::All => true,
        Filter::Decide => card_status::group(c) == Group::NeedsYou,
        Filter::Working => card_status::group(c) == Group::Working,
    }
}

/// The cards on the route's space and board, or every card.
fn in_scope<'a>(s: &'a State) -> Vec<&'a CardSummary> {
    let (space, board) = match &s.nav.route {
        Route::Board { space, board } => (Some(space.as_str()), board.as_deref()),
        _ => (None, None),
    };
    s.data
        .cards
        .iter()
        .filter(|c| space.is_none_or(|sp| c.board == sp))
        .filter(|c| board.is_none_or(|b| c.space_board == b))
        .collect()
}

pub fn view(s: &State, f: &mut Frame) -> El {
    let title = match &s.nav.route {
        Route::Board { space, .. } => s
            .data
            .spaces
            .iter()
            .find(|b| &b.board_id == space)
            .map_or_else(|| "Board".to_string(), |b| b.name.clone()),
        _ => "Board".to_string(),
    };
    let cards = in_scope(s);
    col()
        .key("board-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(crate::ui::page_header(f, &title, vec![filters(s, f, &cards)]))
        .child(cards_list(s, f, &cards).grow(1.0).min_h(0.0))
}

fn filters(s: &State, f: &Frame, cards: &[&CardSummary]) -> El {
    let k = f.k;
    let decide = cards
        .iter()
        .filter(|c| card_status::group(c) == Group::NeedsYou)
        .count();
    let working = cards
        .iter()
        .filter(|c| card_status::group(c) == Group::Working)
        .count();
    let mut tabs = row().key("filters").gap(6.0).center_y();
    for (fl, label, n) in [
        (Filter::All, "All", cards.len()),
        (Filter::Decide, "Needs a decision", decide),
        (Filter::Working, "Working", working),
    ] {
        let on = s.board.filter == fl;
        tabs = tabs.child(
            row()
                .key(format!("filter-{label}"))
                .role("tab")
                .label(label)
                .checked(on)
                .focusable(true)
                .h(26.0)
                .px(10.0)
                .gap(6.0)
                .center_y()
                .radius(13.0)
                .fill(if on { k.c.layer_02 } else { k.c.bg_base })
                .border(1.0, k.c.border_muted)
                .on_press(move |s: &mut State| s.board.filter = fl)
                .child(text(label).font(k.ty.small_medium).color(k.c.text_base))
                .child(text(n.to_string()).font(k.ty.small).color(k.c.text_muted)),
        );
    }
    tabs
}

/// The cards in their groups, scrolling.
pub fn cards_list(s: &State, f: &Frame, cards: &[&CardSummary]) -> El {
    let k = f.k;
    let mut list = col().key("groups").w_full().gap(20.0).padding(20.0);
    let mut any = false;
    for (g, label) in GROUPS {
        let in_group: Vec<&CardSummary> = cards
            .iter()
            .copied()
            .filter(|c| card_status::group(c) == g && shown(c, s.board.filter))
            .collect();
        if in_group.is_empty() {
            continue;
        }
        any = true;
        let mut grid = row()
            .key(format!("grid-{label}"))
            .w_full()
            .gap(12.0)
            .flex_wrap();
        for c in in_group.iter().copied() {
            grid = grid.child(card(c, f));
        }
        list = list.child(
            col()
                .key(format!("group-{label}"))
                .w_full()
                .gap(10.0)
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(text(label).font(k.ty.ui_medium).color(k.c.text_base))
                        .child(
                            text(in_group.len().to_string())
                                .font(k.ty.ui)
                                .color(k.c.text_muted),
                        ),
                )
                .child(grid),
        );
    }
    if !any {
        list = list.child(
            text("No cards here.")
                .key("empty")
                .font(k.ty.ui)
                .color(k.c.text_muted),
        );
    }
    list.scroll(ScrollSpec {
        stick_bottom: false,
    })
}

/// One card's tile.
pub fn card(c: &CardSummary, f: &Frame) -> El {
    let k = f.k;
    let id = c.id.clone();
    let color = card_status::color(c);
    let mut body = col()
        .key(format!("card-{}", c.id))
        .role("button")
        .label(c.title.clone())
        .focusable(true)
        .w(300.0)
        .padding(12.0)
        .gap(6.0)
        .radius(8.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .on_press(move |s: &mut State| s.open_card(&id))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    text(card_status::card_ref(c))
                        .font(k.ty.small.mono())
                        .color(k.c.text_muted),
                )
                .child(el().grow(1.0))
                .child(text(card_status::label(c)).font(k.ty.small_medium).color(color)),
        )
        .child(
            text(c.title.clone())
                .font(k.ty.ui_medium)
                .color(k.c.text_base)
                .w_full()
                .max_lines(2),
        );
    if c.open_blockers > 0 && !c.blocker_question.is_empty() {
        body = body.child(
            text(c.blocker_question.clone())
                .font(k.ty.ui)
                .color(color)
                .w_full()
                .max_lines(3),
        );
    }
    if c.progress.total > 0 {
        let frac = c.progress.done as f32 / c.progress.total as f32;
        body = body.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    el().grow(1.0)
                        .h(4.0)
                        .radius(2.0)
                        .fill(k.c.layer_03)
                        .child(el().w(276.0 * 0.78 * frac).h(4.0).radius(2.0).fill(color)),
                )
                .child(
                    text(format!("{}/{}", c.progress.done, c.progress.total))
                        .font(k.ty.small)
                        .color(k.c.text_muted),
                ),
        );
    }
    let who = &c.holder.who;
    let held = if who.name.is_empty() {
        "Nobody holds it".to_string()
    } else if c.holder.others > 0 {
        format!("{} +{}", who.name, c.holder.others)
    } else {
        who.name.clone()
    };
    body = body.child(
        text(held)
            .font(k.ty.small)
            .color(k.c.text_muted)
            .w_full()
            .max_lines(1),
    );
    if let Some(l) = &c.left_off {
        body = body.child(
            text(format!("Left off: {}", l.text))
                .font(k.ty.ui)
                .color(k.c.text_muted)
                .w_full()
                .max_lines(2),
        );
    }
    body
}

pub fn on_event(s: &mut State, ev: &Event) {
    let _ = (s, ev);
}

/// The headless scene: the first space's board.
pub fn scene(s: &mut State) {
    s.close_card();
    if let Some(b) = s.data.spaces.first() {
        let space = b.board_id.clone();
        s.go(Route::Board { space, board: None });
    }
}
