//! Share to a space, Move, Transfer ownership (cards/share-dialog.tsx).
//! Share is `Dialog::Share` (ui::view draws [`share_dialog`]); Move and
//! Transfer belong to the card page, which draws their dialogs.

use irori_window::boards::{BoardSummary, SharePreview, SpaceBoard};
use irori_window::cards::CardDetail;
use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Dialog};

use super::{err_line, spacer, ts};
use crate::logic::owners::card_owner;
use crate::logic::plain::plain_line;
use crate::ui::{Dialog as Dlg, Frame, State, icons};
use crate::widgets::choice::{Choice, choice_select};

/// Share's state: the space picked, its preview (kept with the board it
/// is for, so a new pick shows nothing stale), a refusal, in flight.
#[derive(Default)]
pub struct Share {
    pub board: String,
    pub got: Option<(String, Result<SharePreview, String>)>,
    pub failed: Option<String>,
    pub busy: bool,
}

/// An open Move or Transfer dialog.
#[derive(Clone, Debug, Default)]
pub struct Pick {
    pub card: String,
    /// The value picked, and the choices (value, label).
    pub to: String,
    pub choices: Vec<(String, String)>,
    /// Move: a parent, whose subtasks move with it.
    pub family: bool,
    /// Transfer: the owner offers it (else asks the owner for it).
    pub offering: bool,
    pub owner_name: String,
    pub busy: bool,
    pub error: Option<String>,
}

fn ready(s: &State) -> Vec<&BoardSummary> {
    s.data.spaces.iter().filter(|b| b.ready).collect()
}

fn share_card(s: &State) -> Option<String> {
    match &s.dialog {
        Some(Dlg::Share { card }) => Some(card.clone()),
        _ => None,
    }
}

/// The share dialog opened: start on the first space, read its preview.
pub fn opened(s: &mut State) {
    let first = ready(s).first().map(|b| b.board_id.clone()).unwrap_or_default();
    s.card.share = Share {
        board: first,
        ..Share::default()
    };
    load_preview(s);
}

/// The dialog opened before any space was ready (the spaces list still
/// loading): once one is, pick it and read its preview, as React's
/// `pick = board || ready[0]` effect does. True when it started one.
pub fn watch(s: &mut State) -> bool {
    if share_card(s).is_none() || !s.card.share.board.is_empty() {
        return false;
    }
    let Some(first) = ready(s).first().map(|b| b.board_id.clone()) else { return false };
    s.card.share.board = first;
    load_preview(s);
    true
}

fn load_preview(s: &mut State) {
    let Some(card) = share_card(s) else { return };
    let board = s.card.share.board.clone();
    if board.is_empty() {
        return;
    }
    let b2 = board.clone();
    s.spawn(
        move |w| w.boards_share_preview(&card, &b2),
        move |s, r| {
            if s.card.share.board == board {
                s.card.share.got = Some((board, r));
            }
        },
    );
}

fn share(s: &mut State) {
    let Some(card) = share_card(s) else { return };
    let sh = &s.card.share;
    let Some((b, Ok(p))) = &sh.got else { return };
    if sh.busy || *b != sh.board {
        return;
    }
    let (board, hash) = (b.clone(), p.hash.clone());
    s.card.share.busy = true;
    s.card.share.failed = None;
    s.spawn(
        move |w| w.boards_share(&card, &board, &hash),
        |s, r| {
            s.card.share.busy = false;
            match r {
                Ok(id) => {
                    s.close_dialog();
                    s.open_card(&id);
                }
                Err(e) => {
                    // Made afresh, so the person confirms what it says now.
                    s.card.share.failed = Some(e);
                    load_preview(s);
                }
            }
        },
    );
}

/// Share to a space (v0.5 §5.2): pick a space, see exactly what goes
/// there after the scrub and who will see it, then Share.
pub fn share_dialog(s: &State, f: &mut Frame) {
    if share_card(s).is_none() {
        return;
    }
    let k = f.k;
    let sh = &s.card.share;
    let spaces = ready(s);
    let pick = if sh.board.is_empty() { spaces.first().map(|b| b.board_id.clone()).unwrap_or_default() } else { sh.board.clone() };
    let (preview, got_err) = match &sh.got {
        Some((b, Ok(p))) if *b == pick => (Some(p), None),
        Some((b, Err(e))) if *b == pick => (None, Some(e.clone())),
        _ => (None, None),
    };
    let error = sh.failed.clone().or(got_err);
    let closes = if preview.is_some_and(|p| !p.subtasks.is_empty()) { " and its subtasks close" } else { " closes" };
    let mut body = col()
        .key("card/share-body")
        .w_full()
        .gap(12.0)
        .px(16.0)
        .pb(16.0)
        .child(
            text(format!(
                "This is exactly what goes to the space. The card's comments, notes and blockers stay private, and this private card{closes}."
            ))
            .font(k.ty.ui)
            .color(k.c.text_muted)
            .w_full()
            .wrap(),
        );
    if spaces.len() > 1 {
        let choices: Vec<Choice> = spaces.iter().map(|b| Choice::new(b.board_id.clone(), plain_line(&b.name))).collect();
        let sel = choice_select(s, f, "card/share-space", "Space", &pick, &choices, sh.busy, 260.0, |s, v| {
            s.card.share.board = v.to_string();
            s.card.share.failed = None;
            load_preview(s);
        });
        body = body.child(labelled_select(f, "Space", sel));
    }
    match preview {
        Some(p) => body = body.child(preview_box(f, p)),
        None if error.is_none() => body = body.child(text("Preparing the preview…").font(k.ty.text_xsmall).color(k.c.text_muted)),
        None => {}
    }
    if let Some(e) = error {
        body = body.child(err_line(f, e));
    }
    let name = preview.map(|p| plain_line(&p.board_name)).unwrap_or_default();
    body = body.child(
        row()
            .w_full()
            .gap(8.0)
            .child(spacer())
            .child(
                Button::new("Cancel")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .disabled(sh.busy)
                    .on_press(|s: &mut State| s.close_dialog())
                    .build(k),
            )
            .child(
                Button::new(if sh.busy { "Sharing…".to_string() } else { format!("Share to {name}") })
                    .variant(ButtonVariant::Submit)
                    .control_size(ControlSize::Small)
                    .disabled(sh.busy || preview.is_none())
                    .on_press(share)
                    .build(k),
            ),
    );
    Dialog::new("card/share", "Share to a space")
        .open(true)
        .width(520.0)
        .fit()
        .child(body)
        .on_close(|s: &mut State| {
            if !s.card.share.busy {
                s.close_dialog();
            }
        })
        .build(k, f.layer);
}

fn labelled_select(f: &Frame, label: &str, sel: El) -> El {
    col()
        .w_full()
        .gap(6.0)
        .child(text(label.to_string()).font(f.k.ty.small_medium).color(f.k.c.text_base))
        .child(sel)
}

/// The scrubbed card as the board will hold it.
fn preview_box(f: &Frame, p: &SharePreview) -> El {
    let k = f.k;
    let small = |t: String| text(t).font(k.ty.text_xsmall).color(k.c.text_muted).w_full().wrap();
    let mut b = col()
        .key("card/share-preview")
        .w_full()
        .gap(8.0)
        .padding(12.0)
        .radius(8.0)
        .border(1.0, k.c.border_base)
        .fill(k.c.layer_02)
        .child(text(plain_line(&p.title)).font(ts(13.0, 600.0)).color(k.c.text_base).w_full().wrap());
    if !p.goal.is_empty() {
        b = b.child(text(p.goal.clone()).font(ts(13.0, 400.0)).color(k.c.text_base).w_full().wrap());
    }
    if !p.done_when.is_empty() {
        b = b.child(small(format!("Done when: {}", plain_line(&p.done_when))));
    }
    if !p.steps.is_empty() {
        let mut l = col().w_full().gap(2.0).pl(8.0);
        for (i, st) in p.steps.iter().enumerate() {
            l = l.child(text(format!("{}. {}", i + 1, plain_line(st))).font(k.ty.text_xsmall).color(k.c.text_base).w_full().wrap());
        }
        b = b.child(l);
    }
    if !p.links.is_empty() {
        b = b.child(
            text(if p.links.len() == 1 { plain_line(&p.links[0]) } else { format!("{} links", p.links.len()) })
                .font(k.ty.text_xsmall.mono())
                .color(k.c.text_muted)
                .help(p.links.join("\n"))
                .w_full()
                .max_lines(1),
        );
    }
    if !p.subtasks.is_empty() {
        let mut sub = col()
            .w_full()
            .gap(4.0)
            .pt(8.0)
            .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_base)
            .child(
                text(if p.subtasks.len() == 1 {
                    "Its subtask goes with it".to_string()
                } else {
                    format!("Its {} subtasks go with it", p.subtasks.len())
                })
                .font(ts(12.0, 500.0))
                .color(k.c.text_base),
            );
        for st in &p.subtasks {
            let r = plain_line(&st.reference);
            let line = format!("{r}  {}", plain_line(&st.title));
            let mut item = col()
                .w_full()
                .gap(2.0)
                .ml(12.0)
                .pl(8.0)
                .border_sides([0.0, 0.0, 0.0, 2.0], k.c.border_base)
                .child(
                    rune::rich(line, vec![rune::Span::new(0, r.len()).mono().size(12.0).color(k.c.text_muted)])
                        .font(ts(13.0, 400.0))
                        .color(k.c.text_base)
                        .w_full()
                        .wrap(),
                );
            if !st.steps.is_empty() {
                item = item.child(small(if st.steps.len() == 1 { "1 step".into() } else { format!("{} steps", st.steps.len()) }));
            }
            sub = sub.child(item);
        }
        b = b.child(sub);
    }
    if !p.kept_private.is_empty() {
        b = b.child(small(format!(
            "Not shared (ended): {}",
            p.kept_private.iter().map(|x| plain_line(x)).collect::<Vec<_>>().join("; ")
        )));
    }
    b = b.child(
        text(format!("Visible to: {}", p.visible_to.iter().map(|x| plain_line(x)).collect::<Vec<_>>().join(", ")))
            .font(k.ty.text_xsmall)
            .color(k.c.text_base)
            .w_full()
            .wrap()
            .pt(8.0)
            .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_base),
    );
    if !p.dropped.is_empty() {
        b = b.child(
            text(format!("Taken out: {}.", p.dropped.iter().map(|x| plain_line(x)).collect::<Vec<_>>().join("; ")))
                .font(k.ty.text_xsmall)
                .color(k.c.warning.fg)
                .w_full()
                .wrap(),
        );
    }
    b
}

// ---- Move and Transfer ---------------------------------------------------------

/// Move: to another board in its space, or another of Personal's (card
/// #563). It takes the next number there; a parent takes its subtasks.
pub fn move_button(f: &Frame, d: &CardDetail, boards: Vec<SpaceBoard>, current: String) -> El {
    let id = d.summary.id.clone();
    let family = d.summary.subtasks > 0;
    Button::new("Move")
        .variant(ButtonVariant::Ghost)
        .control_size(ControlSize::Small)
        .on_press(move |s: &mut State| {
            // The boards may have changed since it last opened.
            let choices: Vec<(String, String)> = boards
                .iter()
                .filter(|b| b.board_id != current)
                .map(|b| (b.board_id.clone(), plain_line(&b.name)))
                .collect();
            s.card.ui.moving = Some(Pick {
                card: id.clone(),
                to: choices.first().map(|c| c.0.clone()).unwrap_or_default(),
                choices,
                family,
                ..Pick::default()
            });
        })
        .build(f.k)
        .key("card/move")
}

/// Transfer ownership (card #432): the owner offers it to another member;
/// anyone else asks the owner for it.
pub fn transfer_button(s: &State, f: &Frame, d: &CardDetail, board: &BoardSummary) -> Option<El> {
    let owner = card_owner(&d.summary, &s.data.spaces)?;
    let others: Vec<(String, String)> = board
        .members
        .iter()
        .filter(|m| m.person_id != owner.person_id)
        .map(|m| (m.person_id.clone(), plain_line(&m.name)))
        .collect();
    if others.is_empty() {
        return None;
    }
    let offering = owner.me;
    let owner_name = plain_line(&owner.name);
    let id = d.summary.id.clone();
    let help = if offering {
        "Transfer ownership: hand this card to another member".to_string()
    } else {
        format!("Ask {owner_name} to hand this card over to you")
    };
    Some(
        Button::new(if offering { "Transfer ownership…" } else { "Ask to own it…" })
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Small)
            .icon(&icons::ARROW_RIGHT_LEFT)
            .help(help)
            .on_press(move |s: &mut State| {
                s.card.ui.transfer = Some(Pick {
                    card: id.clone(),
                    to: others[0].0.clone(),
                    choices: others.clone(),
                    offering,
                    owner_name: owner_name.clone(),
                    ..Pick::default()
                });
            })
            .build(f.k),
    )
}

fn do_move(s: &mut State) {
    let Some(p) = s.card.ui.moving.as_mut() else { return };
    if p.busy || p.to.is_empty() {
        return;
    }
    p.busy = true;
    p.error = None;
    let (id, to) = (p.card.clone(), p.to.clone());
    s.spawn(
        move |w| w.move_card(&id, &to).map(|_| ()),
        |s, r| match r {
            Ok(()) => s.card.ui.moving = None,
            Err(e) => {
                if let Some(p) = s.card.ui.moving.as_mut() {
                    p.busy = false;
                    p.error = Some(e);
                }
            }
        },
    );
}

fn do_transfer(s: &mut State) {
    let Some(p) = s.card.ui.transfer.as_mut() else { return };
    if p.busy {
        return;
    }
    p.busy = true;
    p.error = None;
    let id = p.card.clone();
    let to = if p.offering { p.to.clone() } else { String::new() };
    s.spawn(
        move |w| w.boards_transfer_owner(&id, &to).map(|_| ()),
        |s, r| match r {
            Ok(()) => s.card.ui.transfer = None,
            Err(e) => {
                if let Some(p) = s.card.ui.transfer.as_mut() {
                    p.busy = false;
                    p.error = Some(e);
                }
            }
        },
    );
}

/// The page's Move and Transfer dialogs, when open.
pub fn dialogs(s: &State, f: &mut Frame) {
    let k = f.k;
    if let Some(p) = &s.card.ui.moving {
        let choices: Vec<Choice> = p.choices.iter().map(|(v, l)| Choice::new(v.clone(), l.clone())).collect();
        let mut body = col()
            .key("card/move-body")
            .w_full()
            .gap(12.0)
            .px(16.0)
            .pb(16.0)
            .child(
                text(format!(
                    "It takes the next number on that board. Links to its old number still find it.{}",
                    if p.family { " Its subtasks move with it." } else { "" }
                ))
                .font(k.ty.ui)
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
            )
            .child({
                let sel = choice_select(s, f, "card/move-board", "Board", &p.to, &choices, p.busy, 260.0, |s, v| {
                    if let Some(p) = s.card.ui.moving.as_mut() {
                        p.to = v.to_string();
                    }
                });
                labelled_select(f, "Board", sel)
            });
        if let Some(e) = &p.error {
            body = body.child(err_line(f, plain_line(e)));
        }
        body = body.child(
            row()
                .w_full()
                .gap(8.0)
                .child(spacer())
                .child(
                    Button::new("Cancel")
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Small)
                        .on_press(|s: &mut State| s.card.ui.moving = None)
                        .build(k),
                )
                .child(
                    Button::new("Move")
                        .variant(ButtonVariant::Submit)
                        .control_size(ControlSize::Small)
                        .disabled(p.busy || p.to.is_empty())
                        .on_press(do_move)
                        .build(k),
                ),
        );
        Dialog::new("card/move", "Move to another board")
            .open(true)
            .width(440.0)
            .fit()
            .child(body)
            .on_close(|s: &mut State| s.card.ui.moving = None)
            .build(k, f.layer);
    }
    if let Some(p) = &s.card.ui.transfer {
        let pick_name = p.choices.iter().find(|c| c.0 == p.to).map(|c| c.1.clone()).unwrap_or_default();
        let mut body = col()
            .key("card/transfer-body")
            .w_full()
            .gap(12.0)
            .px(16.0)
            .pb(16.0)
            .child(
                text(if p.offering {
                    "The card's owner decides Done and answers the questions its agents raise. They get the question, and the card is theirs only once they accept.".to_string()
                } else {
                    format!(
                        "{} owns it: they decide Done and answer its questions. They get the question, and the card is yours only once they accept.",
                        p.owner_name
                    )
                })
                .font(k.ty.ui)
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
            );
        if p.offering && p.choices.len() > 1 {
            let choices: Vec<Choice> = p.choices.iter().map(|(v, l)| Choice::new(v.clone(), l.clone())).collect();
            let sel = choice_select(s, f, "card/transfer-to", "Hand it to", &p.to, &choices, p.busy, 260.0, |s, v| {
                if let Some(p) = s.card.ui.transfer.as_mut() {
                    p.to = v.to_string();
                }
            });
            body = body.child(labelled_select(f, "Hand it to", sel));
        }
        if let Some(e) = &p.error {
            body = body.child(err_line(f, e.clone()));
        }
        body = body.child(
            row()
                .w_full()
                .gap(8.0)
                .child(spacer())
                .child(
                    Button::new("Cancel")
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Small)
                        .disabled(p.busy)
                        .on_press(|s: &mut State| s.card.ui.transfer = None)
                        .build(k),
                )
                .child(
                    Button::new(if p.busy {
                        "Asking…".to_string()
                    } else if p.offering {
                        format!("Offer it to {pick_name}")
                    } else {
                        format!("Ask {}", p.owner_name)
                    })
                    .variant(ButtonVariant::Submit)
                    .control_size(ControlSize::Small)
                    .disabled(p.busy || (p.offering && p.to.is_empty()))
                    .on_press(do_transfer)
                    .build(k),
                ),
        );
        Dialog::new("card/transfer", if p.offering { "Transfer ownership" } else { "Ask to own this card" })
            .open(true)
            .width(440.0)
            .fit()
            .child(body)
            .on_close(|s: &mut State| {
                if !s.card.ui.transfer.as_ref().is_some_and(|p| p.busy) {
                    s.card.ui.transfer = None;
                }
            })
            .build(k, f.layer);
    }
}
