//! Closed (card-grid.tsx DoneGrid, DoneBody, DoneShape, PickBar and
//! lib/cards.ts useDoneCards): done, canceled and failed cards, the latest
//! first, a page at a time, narrowed by outcome and the search; Select
//! ticks cards to clear from the lists or delete for good (card #138).

use std::collections::{HashMap, HashSet};

use irori_window::cards::CardSummary;
use rune::{El, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use super::bars::{ChipOpts, Look, chip};
use super::filter::{OUTCOMES, Outcome, of_outcome};
use super::{grid, muted};
use crate::logic::agent_status::now_ms;
use crate::logic::search::matching;
use crate::ui::theme::status;
use crate::ui::{Frame, State};
use crate::widgets::confirm::{Ask, confirm_button};
use crate::widgets::loading;

/// How many closed cards a page adds (DonePageSize).
pub const PAGE: usize = 30;

/// One scope's closed cards as read.
#[derive(Clone, Debug, Default)]
pub struct Done {
    pub cards: Vec<CardSummary>,
    /// More past these (the page's cursor was not empty).
    pub more: bool,
    pub loading: bool,
    pub error: Option<String>,
    /// How many the tab shows: raised by Show more, kept by a reload.
    pub want: usize,
    /// The newest read asked (an older answer is dropped).
    req: u64,
}

#[derive(Default)]
pub struct Model {
    pub by_scope: HashMap<String, Done>,
    pub outcome: Outcome,
    /// Select mode and the ticked cards.
    pub picking: bool,
    pub picked: HashSet<String>,
    next_req: u64,
}

/// Reads a scope's closed cards: the first page, or (`more`) a page more.
pub fn load(s: &mut State, scope: &str, more: bool) {
    s.board.done.next_req += 1;
    let req = s.board.done.next_req;
    let d = s.board.done.by_scope.entry(scope.to_string()).or_default();
    if d.want == 0 {
        d.want = PAGE;
    }
    if more {
        if !d.more {
            return;
        }
        d.want = d.want.max(d.cards.len()) + PAGE;
    }
    d.loading = true;
    d.req = req;
    let (want, sc, sc2) = (d.want as i64, scope.to_string(), scope.to_string());
    s.spawn(
        move |w| w.done_cards_in(want, "", &sc),
        move |s, r| {
            let Some(d) = s.board.done.by_scope.get_mut(&sc2) else {
                return;
            };
            if d.req != req {
                return;
            }
            d.loading = false;
            match r {
                Ok(p) => {
                    d.cards = p.cards;
                    d.more = !p.next.is_empty();
                    d.error = None;
                }
                Err(e) => d.error = Some(e),
            }
        },
    );
}

/// Reads again every scope read before (a board change).
pub fn reload_all(s: &mut State) {
    let scopes: Vec<String> = s.board.done.by_scope.keys().cloned().collect();
    for sc in scopes {
        load(s, &sc, false);
    }
}

/// The skeleton while the first read is out: the outcome row, then tiles.
fn shape(f: &Frame, scope: &str) -> El {
    let mut r = row().key("board-done-shape-row").gap(4.0).center_y();
    for (i, _) in OUTCOMES.iter().enumerate() {
        r = r.child(loading::block(
            f,
            &format!("board-done-shape-{i}"),
            Some(80.0),
            24.0,
            6.0,
        ));
    }
    r = r.child(el().grow(1.0)).child(loading::block(
        f,
        "board-done-shape-sel",
        Some(56.0),
        24.0,
        6.0,
    ));
    let sc = scope.to_string();
    col()
        .key("board-done-shape")
        .role("status")
        .label("Loading the closed cards")
        .w_full()
        .gap(14.0)
        .child(r)
        .child(loading::tile_grid_shape(f, "board-done-shape-tiles", 3))
        .on_visible(move |s: &mut State, on, _tx: &mut Tx| {
            if on && !s.board.done.by_scope.contains_key(&sc) {
                load(s, &sc, false);
            }
        })
}

fn show_more(f: &Frame, scope: &str) -> El {
    let sc = scope.to_string();
    row().key("board-done-more-row").w_full().center().child(
        Button::new("Show more")
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Small)
            .on_press(move |s: &mut State| load(s, &sc, true))
            .build(f.k),
    )
}

fn error_line(key: &str, t: String) -> El {
    text(t)
        .key(key.to_string())
        .role("alert")
        .font(rune::tok::TextStyle::new(12.5, 400.0))
        .color(status::failed())
        .w_full()
        .wrap()
}

/// The Closed tab of `scope` ("own" for Home's, else a space id).
pub fn grid(s: &State, f: &mut Frame, scope: &str) -> El {
    let Some(all) = s
        .board
        .done
        .by_scope
        .get(scope)
        .filter(|d| !(d.loading && d.cards.is_empty()) || d.error.is_some())
    else {
        return shape(f, scope);
    };
    let k = f.k;
    let query = s.board.query.text.clone();
    let refs: Vec<&CardSummary> = all.cards.iter().collect();
    let cards = matching(&refs, &query);
    let mut c = col().key("board-done").w_full().min_w(0.0).gap(14.0);
    if let Some(e) = &all.error
        && cards.is_empty()
    {
        return c.child(error_line(
            "board-done-error",
            format!("Could not load the closed cards: {e}"),
        ));
    }
    if cards.is_empty() && all.cards.is_empty() {
        return c.child(muted(f, "board-done-none", "Nothing closed yet."));
    }
    if cards.is_empty() {
        c = c.child(muted(
            f,
            "board-done-nomatch",
            if all.more {
                "No cards match among these; Show more looks further back."
            } else {
                "No cards match."
            },
        ));
        if all.more {
            c = c.child(show_more(f, scope));
        }
        return c;
    }
    let d = &s.board.done;
    let shown = of_outcome(&cards, d.outcome);
    let mut tabs = row()
        .key("board-outcomes")
        .role("group")
        .label("Filter closed cards by outcome")
        .w_full()
        .gap(4.0)
        .center_y()
        .flex_wrap();
    for (o, label) in OUTCOMES {
        tabs = tabs.child(chip(
            f,
            &format!("board-outcome-{}", o.state()),
            label,
            d.outcome == o,
            ChipOpts {
                look: Look::Secondary,
                ..ChipOpts::default()
            },
            move |s: &mut State| s.board.done.outcome = o,
        ));
    }
    let mut right = row().key("board-done-tools").gap(8.0).center_y();
    if !d.picking {
        right = right.child(super::clear::clear_done(s, f, "board-clear", scope, true));
    }
    right = right.child(
        Button::new(if d.picking {
            "Done selecting"
        } else {
            "Select"
        })
        .variant(if d.picking {
            ButtonVariant::Neutral
        } else {
            ButtonVariant::Outline
        })
        .control_size(ControlSize::Small)
        .on_press(|s: &mut State| {
            if s.board.done.picking {
                s.board.done.picking = false;
                s.board.done.picked.clear();
            } else {
                s.board.done.picking = true;
            }
        })
        .build(k),
    );
    c = c.child(tabs.child(el().grow(1.0)).child(right));
    if d.picking {
        let picked: Vec<&CardSummary> = cards
            .iter()
            .copied()
            .filter(|c| d.picked.contains(&c.id))
            .collect();
        let all_ids: Vec<String> = shown.iter().map(|c| c.id.clone()).collect();
        c = c.child(pick_bar(s, f, scope, &picked, all_ids));
    }
    if let Some(e) = &all.error {
        c = c.child(error_line(
            "board-done-reload-error",
            format!("Could not reload the closed cards: {e}"),
        ));
    }
    if shown.is_empty() {
        let what = match d.outcome {
            Outcome::Completed => "done",
            o => o.state(),
        };
        c = c.child(muted(
            f,
            "board-done-none-of",
            &format!(
                "None {what} among these{}",
                if all.more {
                    "; Show more looks further back."
                } else {
                    "."
                }
            ),
        ));
    }
    let none = HashSet::new();
    c = c.child(grid::card_grid(
        s,
        f,
        "done",
        &shown,
        &none,
        false,
        Some(now_ms()),
    ));
    if all.more {
        c = c.child(show_more(f, scope));
    }
    c
}

/// Acts on the ticked closed cards: clear them from the lists, or delete
/// them for good (your own cards only: a shared space's card is withdrawn
/// by its space, not deleted).
fn pick_bar(s: &State, f: &Frame, scope: &str, picked: &[&CardSummary], all: Vec<String>) -> El {
    let k = f.k;
    let n = picked.len();
    let shared = picked.iter().any(|c| c.scope == "board");
    let what = format!("{n} {}", if n == 1 { "card" } else { "cards" });
    let ids: Vec<String> = picked.iter().map(|c| c.id.clone()).collect();
    let mut r = row()
        .key("board-pick")
        .w_full()
        .gap(8.0)
        .center_y()
        .flex_wrap()
        .px(12.0)
        .py(8.0)
        .radius(10.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .child(
            text(if n > 0 {
                format!("{what} selected")
            } else {
                "Click cards to select them".into()
            })
            .font(rune::tok::TextStyle::new(12.5, 400.0))
            .color(k.c.text_base),
        )
        .child(
            Button::new("Select all shown")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut State| s.board.done.picked = all.iter().cloned().collect())
                .build(k),
        );
    if n > 0 {
        r = r.child(
            Button::new("Clear selection")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut State| s.board.done.picked.clear())
                .build(k),
        );
    }
    let after = {
        let sc = scope.to_string();
        move |s: &mut State| {
            s.board.done.picked.clear();
            load(s, &sc, false);
        }
    };
    let clear_ids = ids.clone();
    let clear = Ask::new(
        format!("Clear {what}? They leave the app's lists. Agents still find them with search and show. Nothing is deleted."),
        format!("Clear {what}"),
        move |w| w.clear_done(0, &clear_ids, "").map(|_| ()),
    )
    .rich()
    .done(after.clone());
    let delete = Ask::new(
        format!(
            "Delete {what} permanently? They leave Irori on every device, and agents no longer find them with search or show. This can't be undone.{}",
            if shared { " Cards in a shared space can't be deleted here." } else { "" }
        ),
        format!("Delete {what}"),
        move |w| {
            for id in &ids {
                w.delete_card(id)?;
            }
            Ok(())
        },
    )
    .rich()
    .danger()
    .done(after);
    r.child(el().grow(1.0)).child(
        row()
            .key("board-pick-acts")
            .gap(8.0)
            .items_start()
            .child(confirm_button(
                s,
                f,
                "board-pick-clear",
                &if n > 0 {
                    format!("Clear {n}…")
                } else {
                    "Clear…".into()
                },
                None,
                n == 0,
                clear,
            ))
            .child(confirm_button(
                s,
                f,
                "board-pick-delete",
                "Delete permanently…",
                None,
                n == 0 || shared,
                delete,
            )),
    )
}
