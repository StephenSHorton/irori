//! The Board's search box (search-box.tsx, card #171): it sits left of
//! the List/Canvas switch and narrows the cards shown as you type
//! (logic::search). "/" anywhere outside a text field puts the cursor in
//! it ([`super::on_key`]); Esc or the x clears it.

use rune::{El, Key, KeyEvent, Tx, icon, row, text};
use rune_kit::TextInput;

use crate::ui::{Frame, State, icons};

/// Puts the cursor in the box with its words selected.
pub fn focus(s: &mut State) {
    let q = &mut s.board.query;
    q.focused = true;
    q.anchor = 0;
    q.focus = q.text.len();
}

pub fn search_box(s: &State, f: &Frame) -> El {
    let k = f.k;
    let q = &s.board.query;
    let trail = if q.text.is_empty() {
        row()
            .key("board-search-kbd")
            .h(16.0)
            .min_w(16.0)
            .px(4.0)
            .center()
            .radius(3.0)
            .border(1.0, k.c.border_base)
            .child(text("/").font(rune::tok::TextStyle::new(10.0, 500.0)).color(k.c.text_muted))
            .a11y_hidden()
    } else {
        row()
            .key("board-search-clear")
            .role("button")
            .label("Clear the search")
            .help("Clear (Esc)")
            .size(20.0, 20.0)
            .center()
            .radius(4.0)
            .hover(k.c.hover)
            .cursor(rune::Cursor::Pointer)
            .on_press(|s: &mut State| {
                s.board.query.clear();
                s.board.query.focused = true;
            })
            .child(icon(&icons::X, 14.0).color(k.c.text_muted))
    };
    row().key("board-search").role("search").child(
        TextInput::new(q)
            .placeholder("Search cards")
            .label("Search cards")
            .leading_icon(&icons::SEARCH)
            .leading_icon_size(14.0)
            .width(Some(224.0))
            .trailing(trail)
            .on_edit(|s: &mut State| &mut s.board.query)
            .on_key(|s: &mut State, ev: &KeyEvent, tx: &mut Tx| {
                if ev.key == Key::Escape {
                    s.board.query.clear();
                    tx.blur();
                    return true;
                }
                false
            })
            .build(k),
    )
}
