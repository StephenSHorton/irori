//! The Spaces page (boards-page.tsx `BoardsPage`, `BoardTile`; space-
//! boards.tsx `PersonalBoards`, `AddBoard`, the board tiles): Personal's
//! boards, every shared space as a tile, the open invite over them, and
//! New space.

use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::{Button, ButtonVariant, TextInput};

use irori_window::boards::{BoardSummary, SpaceBoard};
use irori_window::cards::CardSummary;

use super::logic::{self, PERSONAL, plain_line};
use super::ui::{self as u, t12, t13};
use super::{AddBoard, current, parts};
use crate::ui::{Frame, Route, State};

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

pub fn view(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let space = current(s);
    let title = if space == PERSONAL { "Personal" } else { "Spaces" };
    let new = Button::new("+ New space")
        .on_press(super::open_new_space)
        .build(k)
        .label("New space")
        .key("spaces-new-button");
    let mut body = col().key("spaces-body").w_full().gap(16.0).px(24.0).pt(18.0).pb(24.0);
    if space != PERSONAL {
        body = body.child(
            u::line(
                "A space is a group of people: their boards, Talk and agents. Spaces are private: only the people you invite see them.",
                t13(k),
                k.c.text_muted,
            )
            .max_w(672.0),
        );
    }
    // An open invite sits over the list, never in its place (card #103).
    if let Some(inv) = s.spaces.invite.as_ref().filter(|i| !i.state.is_empty()) {
        body = body.child(
            u::panel(k)
                .key("spaces-invite")
                .w_full()
                .max_w(576.0)
                .padding(16.0)
                .child(parts::invite_panel(s, f, inv)),
        );
    }
    let boards = logic::boards_in_space(&s.data.spaces, &space);
    if space == logic::ALL_SPACES {
        body = body.child(personal_boards(s, f, true));
        if !boards.is_empty() {
            body = body.child(u::heading(k, "Spaces"));
        }
    }
    if space == PERSONAL {
        body = body.child(personal_boards(s, f, false));
    } else if !boards.is_empty() {
        let w = tile_width(f, "spaces-tiles", 280.0, 12.0);
        let mut tiles = row().key("spaces-tiles").w_full().gap(12.0).flex_wrap().label("Your spaces");
        for b in boards {
            tiles = tiles.child(space_tile(s, f, b, w));
        }
        body = body.child(tiles);
    } else {
        body = body.child(
            col()
                .key("spaces-empty")
                .gap(8.0)
                .items_start()
                .child(u::line(
                    "No spaces yet. Make one, then invite people to it; or join one with an invite link someone sent you.",
                    t13(k),
                    k.c.text_muted,
                ))
                .child(
                    Button::new("+ New space")
                        .on_press(super::open_new_space)
                        .build(k),
                ),
        );
    }
    parts::new_space_dialog(s, f);
    col()
        .key("spaces-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(crate::ui::page_header(f, title, vec![new]))
        .child(
            col()
                .key("spaces-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .child(body),
        )
}

/// One space on the Spaces page: name, who is on it, its boards, a bar of
/// its cards by state, its three most recently active cards, when it last
/// had activity, how fresh it is and how many cards wait on you (#126).
fn space_tile(s: &State, f: &Frame, b: &BoardSummary, w: f32) -> El {
    let k = f.k;
    let mine: Vec<&CardSummary> = s.data.cards.iter().filter(|c| c.board == b.board_id).collect();
    let decide = mine.iter().filter(|c| c.open_blockers > 0).count();
    let ok = b.ready && b.warnings.is_empty();
    let recent = logic::recent(&mine, 3);
    let last = if !b.last_at.is_zero() {
        Some(b.last_at.unix_milli_value())
    } else {
        recent.first().map(|c| c.updated_at.unix_milli_value())
    };
    let name = plain_line(&b.name);
    let name = if name.is_empty() { "(unnamed)".to_string() } else { name };
    let id = b.board_id.clone();
    let mut top = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(u::dot(k, ok))
        .child(
            text(name.clone())
                .font(rune::tok::TextStyle::new(14.0, 500.0))
                .color(k.c.text_base)
                .max_lines(1)
                .shrink()
                .min_w(0.0),
        );
    if b.owner {
        top = top.child(el().grow(1.0)).child(u::chip(k, "owner"));
    }
    let key = format!("spaces-tile-{}", b.board_id);
    let mut t = u::panel(k)
        .key(key.clone())
        .role("button")
        .label(name)
        .focusable(true)
        .w(w)
        .padding(12.0)
        .gap(8.0)
        .on_press(move |s: &mut State| {
            s.go(Route::Board {
                space: id.clone(),
                board: None,
            })
        })
        .child(top)
        .child(u::line(
            if b.ready {
                b.people.iter().map(|p| plain_line(p)).collect::<Vec<_>>().join(", ")
            } else {
                "Arriving from your other device".into()
            },
            t12(k),
            k.c.text_muted,
        ));
    if b.ready {
        t = t
            .child(
                text(
                    logic::boards_of(b)
                        .iter()
                        .map(|x| plain_line(&x.name))
                        .collect::<Vec<_>>()
                        .join(" · "),
                )
                .font(t12(k))
                .color(k.c.text_base)
                .max_lines(1),
            )
            .child(u::state_bar(
                k,
                &key,
                &logic::board_segs(&mine, b.done_week.max(0) as usize),
                w - 24.0,
            ));
    }
    if let Some(r) = u::recent_cards(k, &key, &recent) {
        t = t.child(r);
    }
    let mut foot = row().w_full().gap(8.0).flex_wrap().center_y();
    if let Some(at) = last {
        foot = foot.child(
            text(format!("Active {}", logic::relative_time(at, logic::now_ms())))
                .font(t12(k))
                .color(k.c.text_muted),
        );
    }
    if let Some(fr) = u::fresh_short(s, k, &b.board_id, t12(k)) {
        foot = foot.child(fr);
    }
    if decide > 0 {
        foot = foot.child(text(logic::decide_line(decide)).font(t12(k)).color(u::danger(k)));
    }
    t = t.child(foot);
    if let Some(w) = b.warnings.first() {
        t = t.child(u::line(plain_line(w), t12(k), u::warning(k)));
    }
    t
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
    let name = if name.is_empty() { "(unnamed)".to_string() } else { name };
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
        .child(u::state_bar(k, &key, &logic::board_segs(cards, 0), w - 24.0));
    if let Some(r) = u::recent_cards(k, &key, &logic::recent(cards, 3)) {
        t = t.child(r);
    }
    if decide > 0 {
        t = t.child(text(logic::decide_line(decide)).font(t12(k)).color(u::danger(k)));
    }
    t
}

/// + Add board and its small form (on `target`: a space's id or
/// Personal).
pub fn add_board(s: &State, f: &Frame, target: &str) -> El {
    let k = f.k;
    let open = s.spaces.add.as_ref().filter(|a| a.target == target);
    let Some(a) = open else {
        let t = target.to_string();
        return Button::new("+ Add board")
            .variant(ButtonVariant::Ghost)
            .on_press(move |s: &mut State| {
                s.spaces.add = Some(AddBoard {
                    target: t.clone(),
                    ..AddBoard::default()
                })
            })
            .build(k);
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
                .on_edit(|s: &mut State| &mut s.spaces.add.get_or_insert_with(AddBoard::default).name)
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
                .on_edit(|s: &mut State| &mut s.spaces.add.get_or_insert_with(AddBoard::default).tag)
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
    let Some(a) = s.spaces.add.as_ref() else { return };
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

/// Personal's boards, private to you. Under All spaces (titled) it sits
/// over the shared spaces, headed "Personal".
fn personal_boards(s: &State, f: &Frame, titled: bool) -> El {
    let k = f.k;
    let intro = u::line(
        "Personal is you, your devices and your agents. Nobody else sees it.",
        t13(k),
        k.c.text_muted,
    )
    .max_w(672.0)
    .px(4.0);
    let mut sec = col().key("spaces-personal").w_full().gap(8.0).label("Boards in Personal");
    if !titled {
        sec = sec.child(intro.clone());
    }
    sec = sec.child(
        row()
            .gap(8.0)
            .center_y()
            .child(u::heading(k, if titled { "Personal" } else { "Boards" }))
            .child(add_board(s, f, PERSONAL)),
    );
    if titled {
        sec = sec.child(intro);
    }
    if let Some(e) = &s.spaces.personal_error {
        sec = sec.child(u::error_line(k, plain_line(e)));
    }
    let w = tile_width(f, "spaces-personal-tiles", 260.0, 12.0);
    let mut tiles = row().key("spaces-personal-tiles").w_full().gap(12.0).flex_wrap();
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
