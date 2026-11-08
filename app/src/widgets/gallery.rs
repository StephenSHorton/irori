//! The `widgets` scene: every shared widget on the live data, so a
//! screenshot shows them (`irori --snap w.png --scene widgets`). Not a
//! page of the app.

use std::collections::HashSet;

use irori_window::cards::{AgentRef, CardSummary};
use rune::{El, ScrollSpec, col, el, row, text};

use super::agent_strip::{on_cards, strip};
use super::agents_now::agents_now_toggle;
use super::bits::{
    ask_line, badge, holder_line, owner_chip, pr_chip, progress_bar, scope_badge, short_ref,
    side_badge, spinner, state_pill,
};
use super::choice::{Choice, choice_select};
use super::confirm::{Ask, confirm_button};
use super::done_rounds::{earlier_round, round_signs, sent_back_signs};
use super::faces::{
    FACES_MAX, FaceOpts, PersonSize, face, faces, live_dot, my_face, person_face, talk_bubble,
};
use super::loading;
use super::spawned_actions::{ALL, spawned_actions};
use crate::logic::agent_counts::{agents_of, why_waiting};
use crate::logic::agent_status::now_ms;
use crate::logic::cards::{
    Tone, Work, busy, holder_line_shows, open_ask, people_rows_of, state_pill as pill_of,
};
use crate::logic::names::display_name;
use crate::logic::needs::{group_rows, needs_of, row_state};
use crate::logic::plain::plain_line;
use crate::logic::roster_groups::roster_groups;
use crate::logic::rounds::SentBack;
use crate::ui::{Frame, State};

/// The face the scene opens the hover card on.
const HOVER_FACE: &str = "gal-hover-face";

/// The agent whose hover card the scene opens: one Irori started for a
/// card if any, else any agent on a card.
fn hover_agent(s: &State) -> Option<AgentRef> {
    let on = |f: &dyn Fn(&AgentRef) -> bool| {
        s.data
            .cards
            .iter()
            .flat_map(|c| c.working.iter())
            .find(|w| w.status != "person" && f(w))
            .cloned()
    };
    on(&|w| w.spawned && w.live)
        .or_else(|| on(&|w| w.live))
        .or_else(|| on(&|_| true))
}

/// Up to `n` cards, one per state pill first, so the gallery shows each.
fn sample_cards(s: &State, n: usize) -> Vec<&CardSummary> {
    let now = now_ms();
    let mut seen = HashSet::new();
    let mut out: Vec<&CardSummary> = s
        .data
        .cards
        .iter()
        .filter(|c| seen.insert(pill_of(c, now).label))
        .take(n)
        .collect();
    for c in &s.data.cards {
        if out.len() >= n {
            break;
        }
        if !out.iter().any(|o| o.id == c.id) {
            out.push(c);
        }
    }
    out
}

/// Sets the scene up: the hover card open, a card page read for the done
/// rounds and Who's on it.
pub fn prepare(s: &mut State) {
    if let Some(who) = hover_agent(s) {
        s.widgets.hover = Some(super::agent_hover::Hover {
            anchor: HOVER_FACE.into(),
            who,
            over_face: true,
            over_card: false,
        });
    }
    let pick = s
        .data
        .cards
        .iter()
        .find(|c| c.ready && c.open_blockers > 0)
        .or_else(|| s.data.cards.iter().find(|c| c.open_blockers > 0))
        .or_else(|| s.data.cards.first())
        .map(|c| c.id.clone());
    if let Some(id) = pick {
        s.spawn(move |w| w.card(&id).ok(), |s, d| s.widgets.sample = d);
    }
}

fn section(f: &Frame, key: &str, title: &str, body: El) -> El {
    let k = f.k;
    col()
        .key(key.to_string())
        .w_full()
        .gap(8.0)
        .padding(14.0)
        .radius(10.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .child(
            text(title.to_uppercase())
                .font(rune::tok::TextStyle::new(11.0, 600.0).tracking(0.5))
                .color(k.c.text_muted),
        )
        .child(body)
}

fn note(f: &Frame, t: String) -> El {
    text(t)
        .font(f.k.ty.text_xsmall)
        .color(f.k.c.text_muted)
        .w_full()
        .wrap()
}

fn card_row(s: &State, f: &mut Frame, c: &CardSummary) -> El {
    let k = f.k;
    let p = format!("gal-card-{}", c.id);
    let mut chips = row()
        .key(format!("{p}/chips"))
        .gap(6.0)
        .center_y()
        .flex_wrap()
        .min_w(0.0);
    if let Some(r) = short_ref(f, &format!("{p}/ref"), c) {
        chips = chips.child(r);
    }
    chips = chips
        .child(state_pill(f, &format!("{p}/pill"), c))
        .child(scope_badge(f, &format!("{p}/scope"), c));
    for e in [
        side_badge(f, &format!("{p}/side"), c),
        owner_chip(s, f, &format!("{p}/owner"), c),
        pr_chip(f, &format!("{p}/pr"), c),
    ]
    .into_iter()
    .flatten()
    {
        chips = chips.child(e);
    }
    if busy(&Work::from(c)) {
        chips = chips.child(spinner(f, &format!("{p}/spin")));
    }
    let mut r = col()
        .key(p.clone())
        .w_full()
        .gap(5.0)
        .py(8.0)
        .border_sides([0.0, 0.0, 1.0, 0.0], k.c.border_muted)
        .child(chips)
        .child(
            text(plain_line(&c.title))
                .font(k.ty.ui_medium)
                .color(k.c.text_base)
                .w_full()
                .max_lines(1),
        );
    if let Some(b) = progress_bar(f, &format!("{p}/bar"), c) {
        r = r.child(b);
    }
    if let Some(a) = c.ask.as_ref().filter(|_| open_ask(c.ask.as_ref())) {
        r = r.child(ask_line(f, &format!("{p}/ask"), a));
    } else if c.progress.total == 0 || holder_line_shows(&c.holder) {
        r = r.child(holder_line(s, f, &format!("{p}/holder"), c));
    }
    let rl = row_state(c, now_ms());
    let mut tail = row()
        .key(format!("{p}/tail"))
        .gap(8.0)
        .center_y()
        .min_w(0.0)
        .child(
            text(format!("{}: {}", rl.state.label(), rl.line))
                .font(k.ty.text_xsmall)
                .color(k.c.text_muted)
                .max_lines(1)
                .shrink(),
        );
    if !c.working.is_empty() {
        tail = tail.child(el().grow(1.0)).child(faces(
            s,
            f,
            &format!("{p}/faces"),
            &c.working,
            false,
            Some(FACES_MAX),
        ));
    }
    if !c.watching.is_empty() {
        tail = tail.child(faces(
            s,
            f,
            &format!("{p}/watch"),
            &c.watching,
            true,
            Some(3),
        ));
    }
    r.child(tail)
}

fn faces_section(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let mut r = row().key("gal-faces-row").gap(10.0).center_y().flex_wrap();
    if let Some(who) = hover_agent(s) {
        r = r.child(face(s, f, HOVER_FACE, &who, FaceOpts::default()));
    }
    let agents: Vec<AgentRef> = s
        .data
        .cards
        .iter()
        .flat_map(|c| c.working.iter().chain(&c.watching))
        .filter(|w| w.status != "person")
        .fold(Vec::<AgentRef>::new(), |mut v, w| {
            if !v.iter().any(|x| x.key == w.key) {
                v.push(w.clone());
            }
            v
        });
    if !agents.is_empty() {
        r = r.child(faces(
            s,
            f,
            "gal-faces-many",
            &agents,
            false,
            Some(FACES_MAX),
        ));
        r = r.child(faces(
            s,
            f,
            "gal-faces-watch",
            &agents[..agents.len().min(3)],
            true,
            None,
        ));
        r = r.child(
            row()
                .key("gal-inline")
                .gap(4.0)
                .center_y()
                .child(face(
                    s,
                    f,
                    "gal-inline-face",
                    &agents[0],
                    FaceOpts {
                        inline: true,
                        ..FaceOpts::default()
                    },
                ))
                .child(
                    text(display_name(&agents[0]))
                        .font(k.ty.text_small)
                        .color(k.c.text_base),
                ),
        );
    }
    let you = AgentRef {
        key: "user:you".into(),
        name: "You".into(),
        status: "person".into(),
        live: true,
        ..AgentRef::default()
    };
    r = r.child(face(s, f, "gal-person", &you, FaceOpts::default()));
    if agents.len() >= 2
        && let Some(t) = talk_bubble(s, f, "gal-talk", &agents[..2])
    {
        r = r.child(t);
    }
    r = r.child(live_dot());
    let mut people = row().key("gal-people").gap(10.0).center_y();
    for (i, sz) in [
        PersonSize::Sm,
        PersonSize::Md,
        PersonSize::Lg,
        PersonSize::Xl,
    ]
    .into_iter()
    .enumerate()
    {
        people = people.child(my_face(s, f, &format!("gal-me-{i}"), sz));
    }
    for (i, (n, c)) in [
        ("Blake", "teal"),
        ("Casey", "orange"),
        ("Addam", "indigo"),
        ("", ""),
    ]
    .into_iter()
    .enumerate()
    {
        people = people.child(person_face(
            f,
            &format!("gal-p-{i}"),
            n,
            c,
            None,
            PersonSize::Md,
        ));
    }
    col()
        .key("gal-faces")
        .w_full()
        .gap(10.0)
        .child(r)
        .child(people)
}

fn roster_section(s: &State, f: &mut Frame) -> El {
    let groups = roster_groups(&s.data.agents, &s.data.cards);
    let all: Vec<_> = s.data.agents.iter().collect();
    let mut c = col().key("gal-roster").w_full().gap(4.0);
    if let Some(st) = strip(s, f, "gal-strip", &all) {
        c = c.child(st);
    } else {
        c = c.child(note(f, "No agents are around.".into()));
    }
    if !groups.on_cards.is_empty() {
        c = c.child(on_cards(s, f, &groups.on_cards));
    }
    c.child(note(f, {
        let w = why_waiting(&agents_of(&s.data.agents), "");
        if w.is_empty() {
            "Someone is free to take a thread.".into()
        } else {
            w
        }
    }))
}

fn needs_section(s: &State, f: &Frame) -> El {
    let ns = needs_of(&s.data.cards, &[]);
    let now = now_ms();
    let rows: Vec<&CardSummary> = s.data.cards.iter().collect();
    let groups = group_rows(&rows, |c| row_state(c, now).state);
    let c = col().key("gal-needs").w_full().gap(4.0).child(note(
        f,
        format!(
            "Needs you: {} ({})",
            ns.len(),
            ns.iter()
                .map(|n| n.kind_name())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ));
    let mut g = row().key("gal-groups").gap(6.0).flex_wrap();
    for (i, (st, rs)) in groups.iter().enumerate() {
        g = g.child(badge(
            f,
            &format!("gal-group-{i}"),
            &format!("{} {}", st.label(), rs.len()),
            Tone::Slate,
            true,
            "",
        ));
    }
    c.child(g)
}

fn spawned_section(s: &State, f: &mut Frame) -> El {
    let spawned = s
        .data
        .cards
        .iter()
        .find_map(|c| {
            c.working
                .iter()
                .find(|w| w.spawned && w.status != "gone")
                .map(|w| (c, w))
        })
        .or_else(|| {
            s.data.cards.iter().find_map(|c| {
                c.working
                    .iter()
                    .find(|w| w.status != "person")
                    .map(|w| (c, w))
            })
        });
    match spawned {
        Some((c, w)) => col()
            .key("gal-spawned")
            .w_full()
            .gap(6.0)
            .child(note(
                f,
                format!(
                    "{} on {}",
                    display_name(w),
                    crate::logic::cards::ref_text(&c.tag, &c.short)
                ),
            ))
            .child(spawned_actions(s, f, "gal-spawned-actions", w, &c.id, &ALL)),
        None => note(f, "No agent is on a card.".into()),
    }
}

fn rounds_section(s: &State, f: &Frame) -> El {
    let mut c = col().key("gal-rounds").w_full().gap(8.0);
    let mut any = false;
    if let Some(d) = &s.widgets.sample {
        c = c.child(note(
            f,
            format!(
                "From {}",
                crate::logic::cards::ref_text(&d.summary.tag, &d.summary.short)
            ),
        ));
        for b in &d.blockers {
            if let Some(e) = sent_back_signs(f, &format!("gal-signs-{}", b.id), b) {
                c = c.child(e);
                any = true;
            }
            if let Some(e) = earlier_round(s, f, &format!("gal-round-{}", b.id), b, false) {
                c = c.child(e);
                any = true;
            }
        }
        let rows = people_rows_of(d);
        if !rows.is_empty() {
            c = c.child(note(
                f,
                format!(
                    "Who's on it: {}",
                    rows.iter()
                        .map(|r| format!("{} ({:?})", display_name(&r.who), r.role).to_lowercase())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
    }
    if !any {
        let r = SentBack {
            reason: "the others haven't been told the next steps".into(),
            reason_at: None,
            replies: vec![],
            heard: true,
            answered: false,
            pending: vec![],
        };
        c = c.child(round_signs(f, "gal-signs-sample", &r, "card-81-1"));
    }
    c
}

fn controls_section(s: &State, f: &mut Frame) -> El {
    let ask = Ask::new(
        "Stop the gallery's sample? Nothing happens.",
        "Stop",
        |_| Ok(()),
    );
    let end = Ask::new(
        "End the sample agent? This is the gallery: it does nothing, then says so.",
        "End",
        |_| Err("The gallery's End does nothing.".into()),
    )
    .rich()
    .danger();
    let choices = [
        Choice::new("", "All repos"),
        Choice::new("irori", "irori").in_group("Repos"),
        Choice::new("suzuri", "suzuri").in_group("Repos"),
    ];
    col()
        .key("gal-controls")
        .w_full()
        .gap(10.0)
        .child(
            row()
                .key("gal-controls-row")
                .gap(8.0)
                .center_y()
                .flex_wrap()
                .child(confirm_button(
                    s,
                    f,
                    "gal-confirm",
                    "Stop",
                    Some(&crate::ui::icons::SQUARE),
                    false,
                    ask,
                ))
                .child(confirm_button(
                    s,
                    f,
                    "gal-confirm-end",
                    "End",
                    Some(&crate::ui::icons::X),
                    false,
                    end,
                )),
        )
        .child(choice_select(
            s,
            f,
            "gal-choice",
            "Repo",
            "irori",
            &choices,
            false,
            200.0,
            |_, _| {},
        ))
        .child(agents_now_toggle(f, "gal-now", true, false, |_, _| {}))
        .child(agents_now_toggle(f, "gal-later", false, false, |_, _| {}))
}

fn loading_section(f: &Frame) -> El {
    col()
        .key("gal-loading")
        .w_full()
        .gap(10.0)
        .child(loading::need_row_shape(f, "gal-ld-need"))
        .child(loading::list_rows_shape(f, "gal-ld-list", 2, true))
        .child(loading::card_tile_shape(f, "gal-ld-tile"))
}

pub fn view(s: &State, f: &mut Frame) -> El {
    let cards = sample_cards(s, 9);
    let mut list = col().key("gal-cards").w_full();
    if cards.is_empty() {
        list = list.child(note(f, "No cards on this device.".into()));
    }
    for c in cards {
        list = list.child(card_row(s, f, c));
    }
    let faces_b = faces_section(s, f);
    let needs_b = needs_section(s, f);
    let left = col()
        .key("gal-left")
        .grow(1.0)
        .min_w(0.0)
        .gap(12.0)
        .child(section(f, "gal-s-faces", "Faces", faces_b))
        .child(section(f, "gal-s-cards", "Card bits", list))
        .child(section(f, "gal-s-needs", "Needs you and rows", needs_b));
    let roster_b = roster_section(s, f);
    let spawned_b = spawned_section(s, f);
    let rounds_b = rounds_section(s, f);
    let controls_b = controls_section(s, f);
    let loading_b = loading_section(f);
    let right = col()
        .key("gal-right")
        .grow(1.0)
        .min_w(0.0)
        .gap(12.0)
        .child(section(f, "gal-s-roster", "Agent strip", roster_b))
        .child(section(f, "gal-s-spawned", "Spawned actions", spawned_b))
        .child(section(f, "gal-s-rounds", "Done rounds", rounds_b))
        .child(section(f, "gal-s-controls", "Controls", controls_b))
        .child(section(f, "gal-s-loading", "Loading", loading_b));
    col()
        .key("gal-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(crate::ui::page_header(f, "Widgets", vec![]))
        .child(
            col()
                .key("gal-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    row()
                        .key("gal-cols")
                        .w_full()
                        .gap(12.0)
                        .px(20.0)
                        .pb(20.0)
                        .child(left)
                        .child(right),
                ),
        )
}
