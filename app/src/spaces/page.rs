//! Boards with All spaces picked (parts.tsx `PickSpace`, card #689),
//! Personal's page (space-boards.tsx `PersonalBoards`), and the board
//! tiles and + Add board a space's page and Personal's share
//! (board-tiles.tsx, space-boards.tsx `AddBoard`).

use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::{Button, ButtonVariant, TextInput};

use irori_window::boards::SpaceBoard;
use irori_window::cards::CardSummary;

use super::logic::{self, PERSONAL, plain_line};
use super::ui::{self as u, t12, t13};
use super::{AddBoard, parts};
use crate::ui::{Dialog, Frame, Route, State};

/// Lays tiles out as `repeat(auto-fill, minmax(min, 1fr))`: the width of
/// one tile in the row keyed `key`.
pub fn tile_width(f: &Frame, key: &str, min: f32, gap: f32) -> f32 {
    let w = f.cx.layout_size(key).map_or(0.0, |s| s[0]);
    if w < min {
        return min.max(w);
    }
    let cols = ((w + gap) / (min + gap)).floor().max(1.0);
    (w - gap * (cols - 1.0)) / cols
}

/// Boards with All spaces picked (`Route::Spaces`, card #689): it asks
/// for a space, under the page's header (Boards, and New card).
pub fn view(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let new = Button::new("+ New card")
        .help("Make a card for an agent to pick up")
        .on_press(|s: &mut State| s.open_dialog(Dialog::NewCard { board: None }))
        .build(k)
        .label("New card")
        .key("spaces-new-card");
    let body = col()
        .key("spaces-body")
        .w_full()
        .gap(16.0)
        .px(24.0)
        .pt(18.0)
        .pb(24.0)
        .child(parts::pick_space(s, f));
    col()
        .key("spaces-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(crate::ui::page_header(f, "Boards", vec![new]))
        .child(
            col()
                .key("spaces-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(body),
        )
}

/// One board in a space or in Personal: its name and tag, its cards by
/// state, and its newest few. Pressing it opens its list.
pub fn board_tile(
    f: &Frame,
    x: &SpaceBoard,
    is_default: bool,
    cards: &[&CardSummary],
    w: f32,
    open: Route,
) -> El {
    let k = f.k;
    let decide = cards.iter().filter(|c| c.open_blockers > 0).count();
    let name = plain_line(&x.name);
    let name = if name.is_empty() {
        "(unnamed)".to_string()
    } else {
        name
    };
    let key = format!("spaces-board-{}", x.board_id);
    let mut top = row().w_full().gap(8.0).center_y().child(
        text(name.clone())
            .font(rune::tok::TextStyle::new(14.0, 500.0))
            .color(k.c.text_base)
            .max_lines(1)
            .shrink()
            .min_w(0.0),
    );
    if !x.tag.is_empty() {
        top = top.child(
            text(format!("{}#", plain_line(&x.tag)))
                .font(t12(k).mono())
                .color(k.c.text_muted),
        );
    }
    if is_default {
        top = top.child(el().grow(1.0)).child(u::chip(k, "default"));
    }
    let mut t = u::panel(k)
        .key(key.clone())
        .role("button")
        .label(name)
        .focusable(true)
        .w(w)
        .padding(12.0)
        .gap(8.0)
        .on_press(move |s: &mut State| s.go(open.clone()))
        .child(top)
        .child(u::state_bar(
            k,
            &key,
            &logic::board_segs(cards, 0),
            w - 24.0,
        ));
    if let Some(r) = u::recent_cards(k, &key, &logic::recent(cards, 3)) {
        t = t.child(r);
    }
    if decide > 0 {
        t = t.child(
            text(logic::decide_line(decide))
                .font(t12(k))
                .color(u::danger(k)),
        );
    }
    t
}

/// The "+ Add board" button and its small form (on `target`: a space's
/// id or Personal).
pub fn add_board(s: &State, f: &Frame, target: &str) -> El {
    let k = f.k;
    let open = s.spaces.add.as_ref().filter(|a| a.target == target);
    let Some(a) = open else {
        let t = target.to_string();
        return Button::new("+ Add board")
            .variant(ButtonVariant::Ghost)
            .on_press(move |s: &mut State| {
                let mut a = AddBoard {
                    target: t.clone(),
                    ..AddBoard::default()
                };
                a.name.focused = true;
                s.spaces.add = Some(a);
            })
            .build(k)
            .key(format!("spaces-add-board-{target}"));
    };
    let ready = !a.name.text.trim().is_empty() && !a.tag.text.trim().is_empty();
    let submit = move |s: &mut State| add_submit(s);
    let mut form = row()
        .key("spaces-add-form")
        .gap(6.0)
        .flex_wrap()
        .center_y()
        .child(
            TextInput::new(&a.name)
                .placeholder("Board name")
                .label("New board's name")
                .width(Some(192.0))
                .on_edit(|s: &mut State| {
                    &mut s.spaces.add.get_or_insert_with(AddBoard::default).name
                })
                .on_change(|s: &mut State, _| {
                    if let Some(a) = s.spaces.add.as_mut() {
                        clamp(&mut a.name, logic::MAX_BOARD_NAME);
                    }
                })
                .on_submit(move |s: &mut State, _| submit(s))
                .build(k)
                .key("spaces-add-name"),
        )
        .child(
            TextInput::new(&a.tag)
                .placeholder("tag")
                .label("New board's tag (its card refs: tag#12)")
                .width(Some(96.0))
                .text_style(t13(k).mono())
                .on_edit(|s: &mut State| {
                    &mut s.spaces.add.get_or_insert_with(AddBoard::default).tag
                })
                .on_change(|s: &mut State, _| {
                    if let Some(a) = s.spaces.add.as_mut() {
                        lower(&mut a.tag);
                        clamp(&mut a.tag, 16);
                    }
                })
                .on_submit(move |s: &mut State, _| submit(s))
                .build(k)
                .key("spaces-add-tag"),
        )
        .child(
            Button::new("Add board")
                .disabled(!ready)
                .on_press(add_submit)
                .build(k),
        )
        .child(
            Button::new("Cancel")
                .variant(ButtonVariant::Ghost)
                .on_press(|s: &mut State| s.spaces.add = None)
                .build(k),
        );
    if let Some(e) = &a.error {
        form = form.child(u::error_line(k, plain_line(e)).role("alert"));
    }
    form
}

fn add_submit(s: &mut State) {
    let Some(a) = s.spaces.add.as_ref() else {
        return;
    };
    let (target, name, tag) = (a.target.clone(), a.name.text.clone(), a.tag.text.clone());
    if name.trim().is_empty() || tag.trim().is_empty() {
        return;
    }
    let personal = target == PERSONAL;
    s.spawn(
        move |w| {
            if personal {
                w.boards_add_to_personal(&name, &tag).map(|_| ())
            } else {
                w.boards_add_to_space(&target, &name, &tag).map(|_| ())
            }
        },
        move |s, r| match r {
            Ok(()) => {
                s.spaces.add = None;
                super::reload_personal(s);
                super::reload_spaces(s);
            }
            Err(e) => {
                if let Some(a) = s.spaces.add.as_mut() {
                    a.error = Some(logic::error_text(&e));
                }
            }
        },
    );
}

/// Keeps a field to `max` characters (an input's maxLength).
pub fn clamp(e: &mut rune::TextEdit, max: usize) {
    if e.text.chars().count() > max {
        let t: String = e.text.chars().take(max).collect();
        e.replace_text(t);
    }
}

/// Lowercases a field as it is typed.
pub fn lower(e: &mut rune::TextEdit) {
    if e.text.chars().any(|c| c.is_uppercase()) {
        let t = e.text.to_lowercase();
        e.replace_text(t);
    }
}

/// Personal's page ("board/personal", card #689; space-boards.tsx
/// `PersonalBoards`): its boards, private to you. Nobody can be invited,
/// so it has no people or settings of its own.
pub fn personal_boards(s: &State, f: &Frame) -> El {
    let k = f.k;
    let mut sec = col()
        .key("spaces-personal")
        .w_full()
        .gap(8.0)
        .label("Boards in Personal")
        .child(
            u::line(
                "Personal is you, your devices and your agents. Nobody else sees it.",
                t13(k),
                k.c.text_muted,
            )
            .max_w(672.0)
            .px(4.0),
        )
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(u::heading(k, "Boards"))
                .child(add_board(s, f, PERSONAL)),
        );
    if let Some(e) = &s.spaces.personal_error {
        sec = sec.child(u::error_line(k, plain_line(e)));
    }
    let w = tile_width(f, "spaces-personal-tiles", 260.0, 12.0);
    let mut tiles = row()
        .key("spaces-personal-tiles")
        .w_full()
        .gap(12.0)
        .flex_wrap();
    for (i, x) in s.spaces.personal.iter().enumerate() {
        let cards = logic::cards_on_personal(&s.data.cards, &x.board_id);
        tiles = tiles.child(board_tile(
            f,
            x,
            i == 0,
            &cards,
            w,
            Route::Board {
                space: PERSONAL.into(),
                board: Some(x.board_id.clone()),
            },
        ));
    }
    sec.child(tiles)
}
