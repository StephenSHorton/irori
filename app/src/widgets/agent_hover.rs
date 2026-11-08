//! An agent's hover card (the React app's cards/agent-hover.tsx, cards
//! #104, #144, #148, #207, #227, #453, #456, #489, #492): its name and how
//! it stands, its device, the cards it holds, is offered and watches (each
//! one click from its page), the spawned agent's actions, "Offer it
//! cards", and Stop with an optional why.
//!
//! [`trigger`] wraps a face: hovering it (or pressing it, the touch path)
//! opens the card, which [`card`] draws in the overlay layer. Rune times
//! it as main's PreviewCard does: it opens once the pointer rests on the
//! face for [`OPEN_DELAY`], and closes once the pointer has been off the
//! face and the card for [`CLOSE_DELAY`] (`El::hover_delays`).

use std::time::Duration;

use irori_window::cards::{AgentRef, CardSummary};
use rune::{El, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Placement, Switch, TextInput};

use super::faces::{device_color, face_color, identicon};
use super::spawned_actions::{Action, spawned_actions};
use crate::logic::agent_status::{Tone, Who, agent_status, ms, now_ms, through_line};
use crate::logic::cards::ref_text;
use crate::logic::names::display_name;
use crate::logic::plain::plain_line;
use crate::ui::theme::status;
use crate::ui::{Frame, State};

/// How many cards each list shows before "and N more".
const MAX_ROWS: usize = 6;

/// How long the pointer rests on a face before its card opens
/// (agent-hover.tsx `delay`).
pub const OPEN_DELAY: Duration = Duration::from_millis(250);
/// How long the card stays after the pointer leaves the face and card
/// (`closeDelay`).
pub const CLOSE_DELAY: Duration = Duration::from_millis(150);

/// The open hover card: the face it hangs from, its agent, and whether
/// the pointer is on the face or the card (it closes when on neither).
#[derive(Clone, Debug)]
pub struct Hover {
    pub anchor: String,
    pub who: AgentRef,
    pub over_face: bool,
    pub over_card: bool,
}

fn enter(s: &mut State, anchor: &str, who: &AgentRef) {
    if let Some(h) = s.widgets.hover.as_mut().filter(|h| h.anchor == anchor) {
        h.over_face = true;
        return;
    }
    s.widgets.stop_draft = rune::TextEdit::new("");
    s.widgets.stop_line = None;
    s.widgets.hover = Some(Hover {
        anchor: anchor.to_string(),
        who: who.clone(),
        over_face: true,
        over_card: false,
    });
}

/// The pointer has been off `anchor`'s face (or, with `None`, the card)
/// for the close delay.
fn leave(s: &mut State, anchor: Option<&str>) {
    let Some(h) = s.widgets.hover.as_mut() else {
        return;
    };
    match anchor {
        Some(a) if a != h.anchor => return,
        Some(_) => h.over_face = false,
        None => h.over_card = false,
    }
    if !h.over_face && !h.over_card {
        s.widgets.hover = None;
    }
}

/// Wraps a face (keyed `key`) so hovering or pressing it opens `who`'s
/// card. The person has none.
pub fn trigger(_s: &State, _f: &mut Frame, key: &str, who: &AgentRef, face: El) -> El {
    if who.status == "person" {
        return face;
    }
    let (a1, w1) = (key.to_string(), who.clone());
    let (a2, w2) = (key.to_string(), who.clone());
    face.hover_delays(OPEN_DELAY, CLOSE_DELAY)
        .on_hover_change(move |s: &mut State, inside, _: &mut Tx| {
            if inside {
                enter(s, &a1, &w1);
            } else {
                leave(s, Some(&a1));
            }
        })
        .on_press(move |s: &mut State| enter(s, &a2, &w2))
        .cursor(rune::Cursor::Pointer)
}

/// The hover card, in the overlay layer, while one is open.
pub fn card(s: &State, f: &mut Frame) {
    let Some(h) = &s.widgets.hover else { return };
    let k = f.k;
    let who = &h.who;
    let body = summary(s, f, who);
    let surface = rune_kit::popover::raised(
        k,
        col()
            .key("widgets-hover-body")
            .w(288.0)
            .padding(12.0)
            .gap(10.0)
            .children(body),
        k.radius.md,
    )
    .hover_delays(Duration::ZERO, CLOSE_DELAY)
    .on_hover_change(|s: &mut State, inside, _: &mut Tx| {
        if inside {
            if let Some(h) = s.widgets.hover.as_mut() {
                h.over_card = true;
            }
        } else {
            leave(s, None);
        }
    });
    f.layer.push(rune_kit::popover::anchored(
        k,
        surface.key("widgets-hover"),
        &h.anchor,
        Placement::BottomStart,
        6.0,
    ));
}

fn summary(s: &State, f: &mut Frame, who: &AgentRef) -> Vec<El> {
    let k = f.k;
    let gone = who.status == "gone";
    let mut holding: Vec<&CardSummary> = vec![];
    let mut offered: Vec<&CardSummary> = vec![];
    let mut watching: Vec<&CardSummary> = vec![];
    for c in &s.data.cards {
        if let Some(w) = c.working.iter().find(|r| r.key == who.key) {
            if w.assigned {
                &mut offered
            } else {
                &mut holding
            }
            .push(c);
        } else if c.watching.iter().any(|r| r.key == who.key) {
            watching.push(c);
        }
    }
    let spawned_for = s
        .data
        .cards
        .iter()
        .find(|c| c.working.iter().any(|r| r.key == who.key && r.spawned));
    let live = s.data.agents.iter().find(|a| a.card.key == who.key);
    let spawned = spawned_for.is_some() || live.is_some_and(|l| l.spawned);
    let device = if who.device.is_empty() {
        who.where_.clone()
    } else {
        who.device.clone()
    };
    let st = agent_status(
        &Who::from(who),
        live.and_then(|l| ms(&l.card.status_since)),
        now_ms(),
    );
    let mut out = Vec::new();

    let mut info = col()
        .key("widgets-hover-info")
        .gap(2.0)
        .grow(1.0)
        .min_w(0.0)
        .child(
            row()
                .key("widgets-hover-name")
                .gap(8.0)
                .center_y()
                .min_w(0.0)
                .child(
                    text(display_name(who))
                        .font(k.ty.ui_medium)
                        .color(k.c.text_base)
                        .max_lines(1)
                        .shrink(),
                )
                .child(
                    text(st.text.clone())
                        .font(k.ty.text_xsmall)
                        .color(match st.tone {
                            Tone::Muted | Tone::Idle => k.c.text_muted,
                            Tone::Warn => status::decide(),
                            Tone::Working | Tone::Job => status::working(),
                        }),
                ),
        );
    if !device.is_empty() {
        let via = if who.device_via.is_empty() {
            String::new()
        } else {
            format!(" · {}", through_line(&who.device_via))
        };
        info = info.child(
            row()
                .key("widgets-hover-device")
                .gap(6.0)
                .center_y()
                .min_w(0.0)
                .child(
                    el().size(8.0, 8.0)
                        .fill(device_color(&who.color).unwrap_or(k.c.icon_accent)),
                )
                .child(
                    text(format!("{}{via}", plain_line(&device)))
                        .font(k.ty.text_xsmall)
                        .color(k.c.text_muted)
                        .max_lines(1)
                        .shrink(),
                ),
        );
    }
    if !who.where_.is_empty() && !who.device.is_empty() && who.where_ != who.device {
        info = info.child(small(f, plain_line(&who.where_)));
    }
    if let Some(c) = spawned_for {
        info = info.child(small(
            f,
            format!("Irori started it for {}", ref_text(&c.tag, &c.short)),
        ));
    }
    out.push(
        row()
            .key("widgets-hover-head")
            .gap(12.0)
            .center_y()
            .min_w(0.0)
            .child(identicon(
                &who.key,
                52.0,
                face_color(f, &who.status, &who.color),
            ))
            .child(info),
    );
    for (label, cards) in [
        ("Holding", &holding),
        ("Offered", &offered),
        ("Watching", &watching),
    ] {
        if let Some(l) = card_list(f, label, cards) {
            out.push(l);
        }
    }
    if holding.is_empty() && offered.is_empty() && watching.is_empty() {
        out.push(small(f, "No open cards.".into()));
    }
    if let Some(c) = spawned_for.filter(|_| !gone) {
        out.push(spawned_actions(
            s,
            f,
            "widgets-hover-spawned",
            who,
            &c.id,
            &[Action::Attach, Action::Logs, Action::End],
        ));
    }
    if !gone && !who.offline && who.live {
        if let Some(l) = live.filter(|_| !spawned) {
            let elsewhere = if l.here {
                String::new()
            } else if device.is_empty() {
                "another device".into()
            } else {
                device.clone()
            };
            out.push(offer_cards(s, f, who, l.offers_cards, &elsewhere));
        }
        out.push(stop_agent(s, f, who));
    }
    out
}

fn small(f: &Frame, t: String) -> El {
    text(t)
        .font(f.k.ty.text_xsmall)
        .color(f.k.c.text_muted)
        .max_lines(1)
        .w_full()
}

fn card_list(f: &Frame, label: &str, cards: &[&CardSummary]) -> Option<El> {
    if cards.is_empty() {
        return None;
    }
    let k = f.k;
    let mut c = col()
        .key(format!("widgets-hover-{label}"))
        .gap(2.0)
        .w_full()
        .child(
            text(format!("{} · {}", label.to_uppercase(), cards.len()))
                .font(rune::tok::TextStyle::new(11.0, 500.0).tracking(0.4))
                .color(k.c.text_muted),
        );
    for card in cards.iter().take(MAX_ROWS) {
        let id = card.id.clone();
        c = c.child(
            row()
                .key(format!("widgets-hover-{label}-{}", card.id))
                .role("button")
                .focusable(true)
                .w_full()
                .min_w(0.0)
                .gap(6.0)
                .px(4.0)
                .py(2.0)
                .radius(4.0)
                .center_y()
                .cursor(rune::Cursor::Pointer)
                .hover(k.c.layer_02)
                .on_press(move |s: &mut State| {
                    s.widgets.hover = None;
                    s.open_card(&id);
                })
                .child(
                    text(ref_text(&card.tag, &card.short))
                        .font(k.ty.text_xsmall.mono())
                        .color(k.c.text_muted),
                )
                .child(
                    text(plain_line(&card.title))
                        .font(k.ty.text_small)
                        .color(k.c.text_base)
                        .max_lines(1)
                        .shrink(),
                ),
        );
    }
    if cards.len() > MAX_ROWS {
        c = c.child(small(f, format!("and {} more", cards.len() - MAX_ROWS)));
    }
    Some(c)
}

/// "Offer it cards" on a terminal agent (card #453): shows the change at
/// once, roster:changed confirms it, an error puts it back. Elsewhere it
/// only shows what that device chose (card #492).
fn offer_cards(s: &State, f: &Frame, who: &AgentRef, on: bool, elsewhere: &str) -> El {
    let k = f.k;
    let shown = s.widgets.offer_want.get(&who.key).copied().unwrap_or(on);
    let key = who.key.clone();
    let hint = if elsewhere.is_empty() {
        "Irori hands it unheld cards, asks and threads when it's idle. Off: it works only on what you give it.".to_string()
    } else {
        format!(
            "It runs on {}: turn this on or off there.",
            plain_line(elsewhere)
        )
    };
    let mut c = col().key("widgets-hover-offer").w_full().gap(4.0).child(
        Switch::new(shown)
            .label("Offer it cards")
            .description(hint)
            .control_size(ControlSize::Small)
            .disabled(!elsewhere.is_empty())
            .on_toggle(move |s: &mut State| {
                let v = !shown;
                s.widgets.offer_want.insert(key.clone(), v);
                s.widgets.offer_error.remove(&key);
                let (k1, k2) = (key.clone(), key.clone());
                s.spawn(
                    move |w| w.agents_offer_set(&k1, v),
                    move |s, r| {
                        if let Err(e) = r {
                            s.widgets.offer_want.remove(&k2);
                            s.widgets.offer_error.insert(k2, e);
                        }
                    },
                );
            })
            .build(k),
    );
    if let Some(e) = s.widgets.offer_error.get(&who.key) {
        c = c.child(
            text(e.clone())
                .font(k.ty.text_xsmall)
                .color(k.c.danger.fg)
                .w_full()
                .wrap(),
        );
    }
    c
}

fn stop(s: &mut State) {
    let Some(h) = &s.widgets.hover else { return };
    if s.widgets.stop_busy {
        return;
    }
    let who = h.who.clone();
    let why = s.widgets.stop_draft.text.clone();
    s.widgets.stop_busy = true;
    s.widgets.stop_line = None;
    let key = who.key.clone();
    s.spawn(
        move |w| w.agent_stop(&key, &why),
        move |s, r| {
            s.widgets.stop_busy = false;
            s.widgets.stop_line = Some(match r {
                Ok(()) => {
                    s.widgets.stop_draft = rune::TextEdit::new("");
                    (
                        true,
                        format!("{} is stopped and reads this first.", display_name(&who)),
                    )
                }
                Err(e) => (false, e),
            });
        },
    );
}

/// Stop on an agent (card #207): its running turn is cut and what the
/// person writes (or "Stop and wait for me.") is its next prompt.
fn stop_agent(s: &State, f: &Frame, who: &AgentRef) -> El {
    let k = f.k;
    let busy = s.widgets.stop_busy;
    let input = TextInput::new(&s.widgets.stop_draft)
        .placeholder("Why (optional)")
        .label(format!("Why you stop {}", display_name(who)))
        .height(28.0)
        .disabled(busy)
        .on_edit(|s: &mut State| &mut s.widgets.stop_draft)
        .on_submit(|s: &mut State, _: &mut Tx| stop(s))
        .build(k);
    let mut c = col().key("widgets-hover-stop").w_full().gap(4.0).child(
        row()
            .key("widgets-hover-stop-row")
            .w_full()
            .gap(4.0)
            .center_y()
            .child(el().key("widgets-hover-stop-input").grow(1.0).min_w(0.0).child(input))
            .child(
                Button::new("Stop")
                    .variant(ButtonVariant::Danger)
                    .control_size(ControlSize::Small)
                    .disabled(busy)
                    .help("Cut its running turn now: it reads this first and answers you before anything else")
                    .on_press(stop)
                    .build(k),
            ),
    );
    if let Some((ok, t)) = &s.widgets.stop_line {
        c = c.child(
            text(t.clone())
                .font(k.ty.text_xsmall)
                .color(if *ok { k.c.text_muted } else { k.c.danger.fg })
                .w_full()
                .wrap(),
        );
    }
    c
}
