//! The comment box under a card's thread (card-detail.tsx CommentBox):
//! the person's comment goes to the agents on the card, Wake now (on by
//! default) asks it now, Stop and read cuts their running turns (card
//! #207), files go with it (card #124), and the line under it says who
//! heard. As a thread's reply box (`reply`, Talk) a reply always wakes.

use std::rc::Rc;

use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Switch};

use super::blockers::SHARED_NO_FILES;
use super::{err_line, spacer};
use crate::logic::thread::heard_line;
use crate::text::{self as tt, Composer, ComposerOpts, Get, GetMut, Sent};
use crate::ui::{Frame, State};

/// A comment box's own state.
#[derive(Clone, Debug, Default)]
pub struct CommentUi {
    /// Wake now is on unless the person switched it off.
    pub no_wake: bool,
    pub busy: bool,
    pub error: Option<String>,
    /// Who heard the last comment, and whether telling them failed.
    pub heard: Option<(String, String)>,
    pub missed: String,
}

fn reply_get(s: &State) -> &Composer {
    &s.card.reply
}
fn reply_mut(s: &mut State) -> &mut Composer {
    &mut s.card.reply
}

/// Loads a thread's reply draft when Talk opens it.
#[allow(dead_code)] // Talk's (P5) API
pub fn open_reply(s: &mut State, card_id: &str) {
    let key = format!("{card_id}/comment");
    if s.card.reply.key != key {
        tt::open_draft(s, reply_mut, &key);
    }
}

/// A Talk thread's reply box.
#[allow(dead_code)] // Talk's (P5) API
pub fn reply_box(s: &State, f: &mut Frame, card_id: &str) -> El {
    comment_box(s, f, card_id, true, false)
}

fn ui_key(card: &str, reply: bool) -> String {
    if reply { format!("reply/{card}") } else { card.to_string() }
}

fn accessors(s: &State, card: &str, reply: bool) -> Option<(Get, GetMut)> {
    if reply {
        return Some((reply_get, reply_mut));
    }
    super::slot(s, &format!("{card}/comment"))
}

fn submit(s: &mut State, card: &str, reply: bool, stop: bool) {
    let Some((get, get_mut)) = accessors(s, card, reply) else { return };
    let uk = ui_key(card, reply);
    let ui = s.card.comments.entry(uk.clone()).or_default();
    if ui.busy {
        return;
    }
    let c = get(s);
    let body = c.edit.text.trim().to_string();
    let mentions = tt::mentions::keep_mentions(&c.mentions, &c.edit.text);
    let files: Vec<_> = c.files.iter().map(|p| p.upload()).collect();
    let ui = s.card.comments.entry(uk.clone()).or_default();
    if body.is_empty() {
        ui.error = Some("Write a comment first.".into());
        return;
    }
    let tell = if stop {
        "stop"
    } else if !ui.no_wake || reply {
        "wake"
    } else {
        "next_turn"
    };
    ui.busy = true;
    ui.error = None;
    ui.heard = None;
    get_mut(s).sending = true;
    let id = card.to_string();
    let ms = tt::mentions::to_ts(&mentions);
    s.spawn(
        move |w| w.comment(&id, &body, tell, &ms, &files),
        move |s, r| {
            get_mut(s).sending = false;
            let ok = r.is_ok();
            {
                let ui = s.card.comments.entry(uk.clone()).or_default();
                ui.busy = false;
                match r {
                    Ok(res) => {
                        ui.heard = Some((heard_line(&res.answers, &res.watching, &res.told.mode), res.told.error.clone()));
                        ui.missed = tt::mentions::not_delivered(&res.mentioned);
                    }
                    Err(e) => ui.error = Some(e),
                }
            }
            if ok {
                tt::sent(s, get_mut);
            }
        },
    );
}

/// The comment box for card `card` (`reply`: a thread's reply box).
pub fn comment_box(s: &State, f: &mut Frame, card: &str, reply: bool, no_files: bool) -> El {
    let k = f.k;
    let Some((get, get_mut)) = accessors(s, card, reply) else {
        return col();
    };
    let empty = CommentUi::default();
    let ui = s.card.comments.get(&ui_key(card, reply)).unwrap_or(&empty);
    let c1 = card.to_string();
    let opts = ComposerOpts {
        placeholder: if reply { "Reply…" } else { "Comment on the card…" }.into(),
        label: if reply { "Your reply" } else { "Your comment on the card" }.into(),
        on_send: Some(Rc::new(move |s: &mut State, _m: Sent| submit(s, &c1, reply, false))),
        card: card.to_string(),
        board: s.card.detail.as_ref().filter(|d| d.summary.id == card).and_then(|d| d.on_board.as_ref()).map(|b| b.id.clone()).unwrap_or_default(),
        no_files: no_files.then(|| SHARED_NO_FILES.to_string()),
        disabled: ui.busy,
        rows: Some((2, 10)),
        ..ComposerOpts::default()
    };
    let key = if reply { format!("card/reply-{card}") } else { "card/comment".to_string() };
    let field = tt::composer(s, f, &key, get, get_mut, &opts);
    let mut controls = row().w_full().gap(12.0).center_y();
    if !reply {
        let c2 = card.to_string();
        controls = controls.child(
            Switch::new(!ui.no_wake)
                .label("Wake now")
                .disabled(ui.busy)
                .control_size(ControlSize::Small)
                .on_toggle(move |s: &mut State| {
                    let u = s.card.comments.entry(c2.clone()).or_default();
                    u.no_wake = !u.no_wake;
                })
                .build(k),
        );
    }
    let (c3, c4) = (card.to_string(), card.to_string());
    controls = controls
        .child(spacer())
        .child(
            Button::new("Stop and read")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .disabled(ui.busy)
                .help("Cut the agents' running turns: they read this first and answer you before anything else")
                .on_press(move |s: &mut State| submit(s, &c3, reply, true))
                .build(k),
        )
        .child(
            Button::new(if ui.busy {
                "Sending…"
            } else if reply {
                "Reply"
            } else {
                "Comment"
            })
            .variant(ButtonVariant::Submit)
            .control_size(ControlSize::Small)
            .disabled(ui.busy)
            .on_press(move |s: &mut State| submit(s, &c4, reply, false))
            .build(k),
        );
    let mut out = col().key(format!("{key}/form")).w_full().gap(6.0).mt(4.0).child(field).child(controls);
    if let Some((line, err)) = &ui.heard {
        out = out.child(if err.is_empty() {
            text(line.clone()).font(k.ty.text_xsmall).color(k.c.text_muted).w_full().wrap()
        } else {
            err_line(f, format!("On the card, but not everyone could be told: {err}"))
        });
    }
    if !ui.missed.is_empty() {
        out = out.child(err_line(f, ui.missed.clone()));
    }
    if let Some(e) = &ui.error {
        out = out.child(err_line(f, e.clone()));
    }
    out
}
