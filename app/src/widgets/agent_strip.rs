//! The sidebar's Agents section (the React app's cards/agent-strip.tsx
//! and app-sidebar.tsx's Agents block, cards #407, #456, #450, #512): the
//! counts as chips, then Yours (a strip of faces, busy first) and On cards
//! (the agents Irori started, each with its card's ref, together under the
//! card when it has several), then the check slots.

use irori_window::roster::RosterAgent;
use rune::{El, col, row, text};

use super::bits::badge;
use super::check_slots::check_slots;
use super::faces::{face_color, identicon};
use crate::logic::agent_counts::{Agent, agent_counts};
use crate::logic::agent_status::{Who, agent_status, ms, now_ms, through_line};
use crate::logic::cards::{Tone, ref_text};
use crate::logic::roster_groups::{OnCard, roster_groups};
use crate::ui::{Frame, State};

/// How many faces the strip shows before "+N".
const STRIP_MAX: usize = 8;

/// A roster agent's device colour, once there is more than one device.
fn color_of(s: &State, device_id: &str) -> String {
    let live = s.data.devices.iter().filter(|d| !d.revoked).count();
    if live < 2 {
        return String::new();
    }
    s.data
        .devices
        .iter()
        .find(|d| d.device_id == device_id)
        .map(|d| d.color.clone())
        .unwrap_or_default()
}

fn status_text(a: &RosterAgent) -> String {
    agent_status(&Who::from(a), ms(&a.card.status_since), now_ms()).text
}

fn label(f: &Frame, t: &str) -> El {
    text(t.to_uppercase())
        .font(rune::tok::TextStyle::new(11.0, 500.0).tracking(0.4))
        .color(f.k.c.text_muted)
        .px(10.0)
        .pt(6.0)
        .pb(2.0)
}

/// A strip of faces: who is around, in their device colours. Gone agents
/// and agents on an offline device aren't shown.
pub fn strip(s: &State, f: &Frame, key: &str, agents: &[&RosterAgent]) -> Option<El> {
    let mut here: Vec<&RosterAgent> = agents
        .iter()
        .copied()
        .filter(|a| a.card.status != "gone" && a.card.device_state != "offline")
        .collect();
    if here.is_empty() {
        return None;
    }
    here.sort_by(|a, b| {
        (b.card.status == "busy")
            .cmp(&(a.card.status == "busy"))
            .then_with(|| crate::logic::spaces::name_order(&a.card.name, &b.card.name))
    });
    let mut r = row()
        .key(key.to_string())
        .w_full()
        .px(10.0)
        .pt(6.0)
        .gap(4.0)
        .flex_wrap()
        .center_y();
    for a in here.iter().take(STRIP_MAX) {
        let mut tip = format!("{}: {}", a.card.name, status_text(a));
        if !a.card.device_via.is_empty() {
            tip = format!("{tip} · {}", through_line(&a.card.device_via));
        }
        let color = face_color(f, &a.card.status, &color_of(s, &a.card.device_id));
        r = r.child(
            row()
                .key(format!("{key}/{}", a.card.key))
                .size(28.0, 28.0)
                .center()
                .help(tip.clone())
                .label(tip)
                .role("img")
                .child(identicon(&a.card.key, 28.0, color)),
        );
    }
    if here.len() > STRIP_MAX {
        r = r.child(
            text(format!("+{}", here.len() - STRIP_MAX))
                .font(f.k.ty.text_xsmall)
                .color(f.k.c.text_muted),
        );
    }
    Some(r)
}

/// What a spawned agent's row says under its name: where it runs, how
/// it stands, and the device its rows come through.
fn on_card_line(s: &State, card_id: &str, a: &RosterAgent) -> String {
    let w = s
        .data
        .cards
        .iter()
        .find(|c| c.id == card_id)
        .and_then(|c| c.working.iter().find(|r| r.key == a.card.key))
        .map(|r| r.where_.clone())
        .unwrap_or_default();
    let via = if a.card.device_via.is_empty() {
        String::new()
    } else {
        through_line(&a.card.device_via)
    };
    [w, status_text(a), via]
        .into_iter()
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

fn on_card_row(s: &State, f: &Frame, card_id: &str, a: &RosterAgent, tag: &str) -> El {
    let k = f.k;
    let id = card_id.to_string();
    let color = face_color(f, &a.card.status, &color_of(s, &a.card.device_id));
    let mut r = row()
        .key(format!("widgets-oncard-{card_id}-{}", a.card.key))
        .role("button")
        .focusable(true)
        .w_full()
        .min_w(0.0)
        .px(6.0)
        .py(4.0)
        .gap(8.0)
        .center_y()
        .radius(6.0)
        .cursor(rune::Cursor::Pointer)
        .hover(k.c.layer_02)
        .on_press(move |s: &mut State| s.open_card(&id))
        .child(identicon(&a.card.key, 22.0, color))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(
                    text(a.card.name.clone())
                        .font(k.ty.text_xsmall_medium)
                        .color(k.c.text_base)
                        .max_lines(1),
                )
                .child(
                    text(on_card_line(s, card_id, a))
                        .font(rune::tok::TextStyle::new(11.0, 400.0))
                        .color(k.c.text_muted)
                        .max_lines(1),
                ),
        );
    if !tag.is_empty() {
        r = r.child(
            text(tag.to_string())
                .font(rune::tok::TextStyle::new(10.0, 400.0).mono())
                .color(k.c.text_muted)
                .px(4.0)
                .radius(3.0)
                .fill(k.c.layer_02),
        );
    }
    r
}

/// The agents Irori started for cards: each with its "#N", or together
/// under the card when one card has several. A row opens its card.
pub fn on_cards(s: &State, f: &Frame, groups: &[OnCard]) -> El {
    let k = f.k;
    let mut c = col()
        .key("widgets-oncards")
        .w_full()
        .px(4.0)
        .gap(2.0)
        .max_h(224.0)
        .min_h(0.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        });
    for g in groups {
        let r = ref_text(&g.tag, &g.short);
        if g.agents.len() == 1 {
            c = c.child(on_card_row(s, f, &g.card_id, g.agents[0], &r));
            continue;
        }
        let id = g.card_id.clone();
        let mut grp = col()
            .key(format!("widgets-oncard-group-{}", g.card_id))
            .role("group")
            .label(format!("{r} {}", g.title))
            .w_full()
            .py(2.0)
            .radius(6.0)
            .border(1.0, k.c.border_muted)
            .child(
                row()
                    .key(format!("widgets-oncard-head-{}", g.card_id))
                    .role("button")
                    .w_full()
                    .min_w(0.0)
                    .px(6.0)
                    .py(2.0)
                    .gap(6.0)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |s: &mut State| s.open_card(&id))
                    .child(
                        text(r.clone())
                            .font(rune::tok::TextStyle::new(11.0, 400.0).mono())
                            .color(k.c.text_muted),
                    )
                    .child(
                        text(format!("{} · {} agents", g.title, g.agents.len()))
                            .font(rune::tok::TextStyle::new(11.0, 400.0))
                            .color(k.c.text_muted)
                            .max_lines(1)
                            .shrink(),
                    ),
            );
        for a in &g.agents {
            grp = grp.child(on_card_row(s, f, &g.card_id, a, ""));
        }
        c = c.child(grp);
    }
    c
}

/// The sidebar's Agents section: the counts, Yours and On cards, the
/// check slots. Nothing while there are no agents, cards on them or runs.
pub fn agent_strip(s: &State, f: &mut Frame) -> El {
    let groups = roster_groups(&s.data.agents, &s.data.cards);
    let yours: Vec<Agent> = groups.yours.iter().map(|a| Agent::from(*a)).collect();
    let n = agent_counts(&yours, &groups.blocked);
    let blocked_on_cards = groups
        .on_cards
        .iter()
        .flat_map(|g| g.agents.iter())
        .filter(|a| groups.blocked.contains(&a.card.key))
        .count();
    let on_cards_now = groups.spawned - blocked_on_cards;
    let blocked_now = n.blocked + blocked_on_cards;
    let mut sec = col()
        .key("agent-strip")
        .w_full()
        .pb(8.0)
        .gap(2.0)
        .child(label(f, "Agents"));
    let tip = [
        (on_cards_now > 0).then(|| {
            format!("{on_cards_now} started by Irori for a card, each ends when its card is Done")
        }),
        (groups.starting > 0).then(|| format!("{} cards starting an agent", groups.starting)),
        (groups.queued > 0).then(|| {
            format!(
                "{} cards queued for an agent: a device's free slot or a usage limit's reset",
                groups.queued
            )
        }),
        Some(format!("{} of yours", n.total)),
        Some(format!(
            "{} working{}",
            n.working,
            if n.running > 0 {
                format!(" ({} running a job)", n.running)
            } else {
                String::new()
            }
        )),
        (blocked_now > 0).then(|| {
            format!("{blocked_now} blocked: on a permission prompt, or on a card waiting for you")
        }),
        (n.free + n.own_work > 0).then(|| format!("{} idle", n.free + n.own_work)),
        (n.silent > 0).then(|| format!("{} idle but not answering", n.silent)),
        (n.offline > 0).then(|| format!("{} on an offline device", n.offline)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n");
    let chips: Vec<(usize, &str, Tone)> = vec![
        (on_cards_now, "on cards", Tone::Info),
        (groups.starting, "starting", Tone::Slate),
        (groups.queued, "queued", Tone::Warning),
        (n.working, "working", Tone::Info),
        (blocked_now, "blocked", Tone::Warning),
        (n.silent, "not answering", Tone::Warning),
    ];
    let mut counts = row()
        .key("widgets-agent-counts")
        .w_full()
        .px(10.0)
        .gap(4.0)
        .flex_wrap()
        .help(tip);
    for (i, (n, w, t)) in chips.into_iter().enumerate() {
        if n > 0 {
            counts = counts.child(badge(
                f,
                &format!("widgets-agent-count-{i}"),
                &format!("{n} {w}"),
                t,
                false,
                "",
            ));
        }
    }
    sec = sec.child(counts);
    if groups.on_cards.is_empty() {
        let all: Vec<&RosterAgent> = s.data.agents.iter().collect();
        if let Some(st) = strip(s, f, "widgets-strip-all", &all) {
            sec = sec.child(st);
        }
    } else {
        sec = sec.child(label(f, "Yours"));
        if let Some(st) = strip(s, f, "widgets-strip-yours", &groups.yours) {
            sec = sec.child(st);
        }
        sec = sec
            .child(label(f, "On cards"))
            .child(on_cards(s, f, &groups.on_cards));
    }
    if let Some(cs) = check_slots(s, f) {
        sec = sec.child(cs);
    }
    sec
}
