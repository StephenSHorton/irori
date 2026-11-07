//! A card's page (P3: the React app's cards/card-detail.tsx, thread.tsx,
//! new-card-dialog.tsx and share-dialog.tsx). It shows over the route's
//! page while `s.nav.card` is set.
//!
//! This is the spike's page, carried over: goal, done-when, steps, open
//! blockers, the thread and a comment box. P3 ports the rest.

use rune::{El, ScrollSpec, TextEdit, Tx, col, el, row, text, text_input};
use rune_kit::{Button, ButtonVariant};

use irori_window::cards::CardDetail;
use irori_window::events::{EVENT_BOARD_CHANGED, EVENT_ROSTER_CHANGED};

use crate::logic::card_status;
use crate::ui::theme::status;
use crate::ui::{Event, Frame, State};

#[derive(Default)]
pub struct Model {
    /// The open card's page, read when it opens and on each change.
    pub detail: Option<CardDetail>,
    pub draft: TextEdit,
    /// A comment to scroll to (a notification's `task:<id>/comment:<id>`).
    pub focus_comment: Option<String>,
}

fn reload(s: &mut State) {
    let Some(id) = s.nav.card.clone() else {
        s.card.detail = None;
        return;
    };
    s.card.detail = s.w.as_ref().and_then(|w| w.card(&id).ok());
}

pub fn on_event(s: &mut State, ev: &Event) {
    match ev.name.as_str() {
        "app:card-opened" => {
            if s.card.detail.as_ref().map(|d| d.summary.id.as_str()) != s.nav.card.as_deref() {
                s.card.draft.clear();
            }
            reload(s);
        }
        EVENT_BOARD_CHANGED | EVENT_ROSTER_CHANGED if s.nav.card.is_some() => reload(s),
        _ => {}
    }
}

/// The headless scene: the first card that needs you, else the newest.
pub fn scene(s: &mut State) {
    let pick = s
        .data
        .cards
        .iter()
        .find(|c| card_status::group(c) == card_status::Group::NeedsYou)
        .or_else(|| s.data.cards.first())
        .map(|c| c.id.clone());
    if let Some(id) = pick {
        s.open_card(&id);
    }
}

fn send_comment(s: &mut State) {
    let Some(id) = s.nav.card.clone() else { return };
    let body = s.card.draft.text.trim().to_string();
    if body.is_empty() {
        return;
    }
    s.act(
        "Comment",
        move |w| w.comment(&id, &body, "", &[], &[]).map(|_| ()),
        |s, ()| {
            s.card.draft.clear();
            reload(s);
        },
    );
}

pub fn view(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let Some(d) = &s.card.detail else {
        return crate::ui::not_yet(f, "Card", "This card isn't on this device.");
    };
    let c = &d.summary;
    let color = card_status::color(c);
    let r = format!("{}, task:{}", card_status::card_ref(c), c.id);
    let top = row()
        .key("card-top")
        .w_full()
        .h(48.0)
        .px(20.0)
        .gap(8.0)
        .center_y()
        .child(
            Button::new("Back")
                .variant(ButtonVariant::Ghost)
                .icon(&crate::ui::icons::ARROW_LEFT)
                .on_press(|s: &mut State| s.back())
                .build(k),
        )
        .child(
            text(card_status::card_ref(c))
                .font(k.ty.small.mono())
                .color(k.c.text_muted),
        )
        .child(text(card_status::label(c)).font(k.ty.small_medium).color(color))
        .child(el().grow(1.0))
        .child(
            Button::new("Copy ref")
                .variant(ButtonVariant::Ghost)
                .on_press_tx(move |_: &mut State, tx: &mut Tx| tx.copy(r.clone()))
                .build(k),
        );
    let mut page = col()
        .key("detail")
        .w_full()
        .max_w(820.0)
        .gap(14.0)
        .padding(20.0)
        .child(
            text(c.title.clone())
                .font(rune::tok::TextStyle::new(18.0, 600.0))
                .color(k.c.text_base)
                .w_full()
                .wrap(),
        );
    if !d.goal.is_empty() {
        page = page.child(section(f, "Objective", &d.goal));
    }
    if !d.done_when.is_empty() {
        page = page.child(section(f, "Done when", &d.done_when));
    }
    if !d.steps.is_empty() {
        let done = d.steps.iter().filter(|s| s.done).count();
        let mut steps = col()
            .key("steps")
            .w_full()
            .gap(6.0)
            .child(label(f, &format!("Steps {done}/{}", d.steps.len())));
        for st in &d.steps {
            let (mark, mc) = if st.done {
                ("✓", status::working())
            } else if st.doing {
                ("▸", status::decide())
            } else {
                ("○", k.c.text_muted)
            };
            steps = steps.child(
                row()
                    .key(format!("step-{}", st.id))
                    .w_full()
                    .gap(8.0)
                    .child(text(mark).font(k.ty.ui).color(mc).w(14.0))
                    .child(
                        text(st.text.clone())
                            .font(k.ty.ui)
                            .color(if st.done { k.c.text_muted } else { k.c.text_base })
                            .grow(1.0)
                            .min_w(0.0)
                            .wrap(),
                    ),
            );
        }
        page = page.child(steps);
    }
    for b in d.blockers.iter().filter(|b| b.resolved.is_none()) {
        let mut bl = col()
            .key(format!("blocker-{}", b.id))
            .w_full()
            .gap(6.0)
            .padding(12.0)
            .radius(8.0)
            .border(1.0, status::decide())
            .child(label(f, "Needs a decision"))
            .child(
                text(b.question.clone())
                    .font(k.ty.ui_medium)
                    .color(k.c.text_base)
                    .w_full()
                    .wrap(),
            );
        if !b.context.is_empty() {
            bl = bl.child(
                text(b.context.clone())
                    .font(k.ty.ui)
                    .color(k.c.text_muted)
                    .w_full()
                    .wrap(),
            );
        }
        for (i, o) in b.options.iter().enumerate() {
            let rec = if o.recommended { "  (recommended)" } else { "" };
            bl = bl.child(
                text(format!("{}. {}{rec}", i + 1, o.answer))
                    .font(k.ty.ui)
                    .color(k.c.text_base)
                    .w_full()
                    .wrap(),
            );
        }
        page = page.child(bl);
    }
    if !d.thread.is_empty() {
        let mut th = col()
            .key("thread")
            .w_full()
            .gap(10.0)
            .child(label(f, "Thread"));
        for it in &d.thread {
            th = th.child(
                col()
                    .key(format!("item-{}", it.id))
                    .w_full()
                    .gap(2.0)
                    .child(
                        text(if it.who.name.is_empty() {
                            it.kind.clone()
                        } else {
                            it.who.name.clone()
                        })
                        .font(k.ty.small_medium)
                        .color(k.c.text_muted),
                    )
                    .child(
                        text(it.text.clone())
                            .font(k.ty.ui)
                            .color(k.c.text_base)
                            .w_full()
                            .wrap()
                            .selectable(true),
                    ),
            );
        }
        page = page.child(th);
    }
    page = page.child(comment_box(s, f));
    col()
        .key("card-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(top)
        .child(
            col()
                .key("card-scroll")
                .w_full()
                .items_center()
                .child(page)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .grow(1.0)
                .min_h(0.0),
        )
}

fn comment_box(s: &State, f: &Frame) -> El {
    let k = f.k;
    let field = text_input(&s.card.draft, k.ty.ui)
        .key("comment")
        .placeholder("Comment on this card…", k.c.text_faint)
        .w_full()
        .min_h(60.0)
        .padding(8.0)
        .radius(6.0)
        .border(1.0, k.c.border_base)
        .fill(k.c.layer_01)
        .on_edit(|s: &mut State| &mut s.card.draft)
        .on_submit(|s: &mut State, _: &mut Tx| send_comment(s));
    col()
        .key("comment-box")
        .w_full()
        .gap(8.0)
        .child(field)
        .child(
            row().w_full().child(el().grow(1.0)).child(
                Button::new("Comment")
                    .variant(ButtonVariant::Submit)
                    .on_press(|s: &mut State| send_comment(s))
                    .build(k),
            ),
        )
}

fn label(f: &Frame, s: &str) -> El {
    text(s.to_string())
        .font(f.k.ty.small_medium)
        .color(f.k.c.text_muted)
}

fn section(f: &Frame, title: &str, body: &str) -> El {
    col().w_full().gap(4.0).child(label(f, title)).child(
        text(body.to_string())
            .font(f.k.ty.ui)
            .color(f.k.c.text_base)
            .w_full()
            .wrap()
            .selectable(true),
    )
}

/// New card (P3: new-card-dialog.tsx).
pub fn new_card_dialog(s: &State, f: &mut Frame) {
    let _ = s;
    rune_kit::Dialog::new("dlg-new-card", "New card")
        .open(true)
        .description("Not ported to Rune yet (P3).")
        .on_close(|s: &mut State| s.close_dialog())
        .build(f.k, f.layer);
}

/// Share a card to a space (P3: share-dialog.tsx).
pub fn share_dialog(s: &State, f: &mut Frame) {
    let _ = s;
    rune_kit::Dialog::new("dlg-share", "Share card")
        .open(true)
        .description("Not ported to Rune yet (P3).")
        .on_close(|s: &mut State| s.close_dialog())
        .build(f.k, f.layer);
}
