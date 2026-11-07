//! Not yet and order (order.tsx, card #570), kept quiet: the list's
//! shape says it. A card that follows another sits under it; one that
//! can't start yet folds away at the end. [`when_line`] and
//! [`not_yet_button`] are for the card page.

// when_line and not_yet_button are the card page's: unused
// here until it lands.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

use irori_window::later::{CardLater, CardOrder, OrderStep};
use rune::{El, TextEdit, col, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Placement, Popover, TextInput};

use super::grid::Dragging;
use crate::logic::later::{around, when_later};
use crate::logic::plain::plain_line;
use crate::ui::theme::status;
use crate::ui::{Frame, State, icons};

#[derive(Default)]
pub struct Model {
    /// A pinned tile being dragged.
    pub drag: Option<Dragging>,
    /// The card whose Not yet popover is open, its box, and its answer.
    pub hold_open: String,
    pub until: TextEdit,
    pub hold_busy: bool,
    pub hold_error: Option<String>,
    /// Start now in flight, and its errors, by card.
    pub starting: HashSet<String>,
    pub start_errors: HashMap<String, String>,
}

fn small() -> rune::tok::TextStyle {
    rune::tok::TextStyle::new(12.0, 400.0)
}

/// The end of a list: one quiet line ("3 not yet") until it is opened.
pub fn not_yet_fold(s: &State, f: &Frame, key: &str, count: usize, children: El) -> El {
    let k = f.k;
    let open = s.board.folds.contains(key);
    let kk = key.to_string();
    let chevron = icon(if open { &icons::CHEVRON_DOWN } else { &icons::CHEVRON_RIGHT }, 12.0).color(k.c.text_muted);
    let mut c = col().key(key.to_string()).w_full().gap(6.0).child(
        row()
            .key(format!("{key}/toggle"))
            .role("button")
            .label(format!("{count} not yet"))
            .checked(open)
            .focusable(true)
            .gap(4.0)
            .px(4.0)
            .center_y()
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut State| {
                if !s.board.folds.remove(&kk) {
                    s.board.folds.insert(kk.clone());
                }
            })
            .child(chevron)
            .child(text(format!("{count} not yet")).font(small()).color(k.c.text_muted)),
    );
    if open {
        c = c.child(children);
    }
    c
}

fn step_ref(st: &OrderStep) -> String {
    if st.short.is_empty() { "new".into() } else { format!("{}#{}", st.tag, st.short) }
}

/// A card in an order line: its ref and title, opening it.
fn card_link(f: &Frame, key: &str, st: &OrderStep) -> El {
    let k = f.k;
    let id = st.id.clone();
    row()
        .key(key.to_string())
        .role("link")
        .label(format!("{} {}", step_ref(st), plain_line(&st.title)))
        .gap(4.0)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |s: &mut State| s.open_card(&id))
        .child(text(step_ref(st)).font(k.ty.ui.mono()).color(k.c.text_base))
        .child(text(plain_line(&st.title)).font(k.ty.ui).color(k.c.text_base).max_lines(1).shrink())
}

/// Under a card's title, as "Part of" sits: when it starts (held, with
/// Start now), or "After #12, then #14".
pub fn when_line(s: &State, f: &Frame, key: &str, id: &str, later: Option<&CardLater>, order: Option<&CardOrder>) -> Option<El> {
    let k = f.k;
    let muted = |t: &str| text(t.to_string()).font(k.ty.ui).color(k.c.text_muted);
    if let Some(l) = later {
        return Some(held_line(s, f, key, id, &when_later(l)));
    }
    let (before, after) = around(id, order);
    if before.is_none() && after.is_none() {
        return None;
    }
    let mut r = row().key(key.to_string()).gap(4.0).center_y().flex_wrap();
    if let Some(b) = before {
        r = r.child(muted("After")).child(card_link(f, &format!("{key}/before"), b));
        if after.is_some() {
            r = r.child(muted(", then"));
        }
    }
    if let Some(a) = after {
        if before.is_none() {
            r = r.child(muted("Then"));
        }
        r = r.child(card_link(f, &format!("{key}/after"), a));
    }
    Some(r)
}

/// A held card's line with its Start now.
fn held_line(s: &State, f: &Frame, key: &str, id: &str, line: &str) -> El {
    let k = f.k;
    let busy = s.board.order.starting.contains(id);
    let cid = id.to_string();
    let mut c = col().key(key.to_string()).gap(2.0).child(
        row()
            .gap(6.0)
            .center_y()
            .flex_wrap()
            .child(text(plain_line(line)).font(k.ty.ui).color(k.c.text_muted))
            .child(
                text("Start now")
                    .key(format!("{key}/start"))
                    .role("button")
                    .label("Start now")
                    .font(k.ty.ui)
                    .color(k.c.text_base)
                    .opacity(if busy { 0.6 } else { 1.0 })
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |s: &mut State| start_now(s, &cid)),
            ),
    );
    if let Some(e) = s.board.order.start_errors.get(id) {
        c = c.child(text(e.clone()).font(small()).color(status::failed()));
    }
    c
}

fn start_now(s: &mut State, id: &str) {
    if !s.board.order.starting.insert(id.to_string()) {
        return;
    }
    s.board.order.start_errors.remove(id);
    let (a, b) = (id.to_string(), id.to_string());
    s.spawn(
        move |w| w.start_now(&a),
        move |s, r| {
            s.board.order.starting.remove(&b);
            if let Err(e) = r {
                s.board.order.start_errors.insert(b, e);
            }
        },
    );
}

fn hold(s: &mut State) {
    if s.board.order.hold_busy {
        return;
    }
    let id = s.board.order.hold_open.clone();
    let until = s.board.order.until.text.clone();
    s.board.order.hold_busy = true;
    s.board.order.hold_error = None;
    s.spawn(
        move |w| w.not_yet(&id, &until, ""),
        |s, r| {
            s.board.order.hold_busy = false;
            match r {
                Ok(()) => {
                    s.board.order.until = TextEdit::default();
                    s.board.order.hold_open.clear();
                }
                Err(e) => s.board.order.hold_error = Some(e),
            }
        },
    );
}

/// Holds a card back from the agents: one box takes a card, a date or a
/// version, and left empty it waits for the person's word. None for a
/// card already held, over, or an ask.
pub fn not_yet_button(s: &State, f: &mut Frame, key: &str, id: &str, held: bool, over: bool, ask: bool) -> Option<El> {
    if held || over || ask {
        return None;
    }
    let k = f.k;
    let o = &s.board.order;
    let open = o.hold_open == id;
    let cid = id.to_string();
    let trigger = Button::new("Not yet")
        .variant(ButtonVariant::Ghost)
        .control_size(ControlSize::Small)
        .on_press(move |s: &mut State| {
            s.board.order.hold_open = if s.board.order.hold_open == cid { String::new() } else { cid.clone() };
            s.board.order.hold_error = None;
            s.board.order.until.focused = true;
        })
        .build(k);
    let mut body = col()
        .key(format!("{key}/body"))
        .w(260.0)
        .gap(8.0)
        .child(text("Hold it back until").font(rune::tok::TextStyle::new(13.0, 400.0)).color(k.c.text_base))
        .child(
            TextInput::new(&o.until)
                .placeholder("#83, 1 Dec or v0.13.0")
                .label("Until")
                .width(None)
                .disabled(o.hold_busy)
                .on_edit(|s: &mut State| &mut s.board.order.until)
                .on_submit(|s: &mut State, _tx: &mut rune::Tx| hold(s))
                .build(k),
        )
        .child(
            text("Leave it empty to hold it until you say. No agent gets it before then.")
                .font(small())
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
        );
    if let Some(e) = &o.hold_error {
        body = body.child(text(e.clone()).font(small()).color(status::failed()).w_full().wrap());
    }
    body = body.child(
        row().w_full().child(rune::el().grow(1.0)).child(
            Button::new("Hold back")
                .control_size(ControlSize::Small)
                .disabled(o.hold_busy)
                .on_press(hold)
                .build(k),
        ),
    );
    Some(
        Popover::new(key.to_string())
            .trigger(trigger)
            .open(open)
            .placement(Placement::BottomEnd)
            .on_close(|s: &mut State| {
                s.board.order.hold_open.clear();
                s.board.order.hold_error = None;
            })
            .child(body)
            .build(k, f.layer),
    )
}
