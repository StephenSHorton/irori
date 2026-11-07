//! The Irori bits each canvas node is drawn with (the React app's
//! components/canvas/node.tsx): a card is the grid card's own state pill,
//! ref, title, faces and progress bar with its markers on top; a blocker
//! is its question under the card; an agent is a pill in its device
//! colour; an area is a dashed box with its label.

use rune::{Color, Cycle, El, col, el, row, text};

use crate::logic::agent_status::Tone as StatusTone;
use crate::logic::cards::{Tone, blocker};
use crate::ui::theme::status;
use crate::ui::{Frame, State};
use crate::widgets::{bits, faces};

use super::data::{AGENT, AgentNode, BLOCKER, BlockerNode, By, CARD, CardNode, GroupNode, Node};

/// "2/5 steps", or "" for a card without steps.
pub fn steps_text(done: i64, total: i64) -> String {
    if total > 0 {
        format!("{done}/{total} steps")
    } else {
        String::new()
    }
}

/// One of a card's at-a-glance flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    /// It waits on the person.
    Needs,
    /// Nobody live is on it.
    Nobody,
    /// Nothing happened on it for a while.
    Stale,
}

impl Marker {
    pub fn text(self) -> &'static str {
        match self {
            Marker::Needs => "Waiting on you",
            Marker::Nobody => "Nobody on it",
            Marker::Stale => "Stale",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Marker::Needs => "It needs your answer or your Done",
            Marker::Nobody => "No agent that is running holds it",
            Marker::Stale => "Nothing has happened on it for a while, and nobody is busy on it",
        }
    }
}

/// A card's at-a-glance flags (card #587), most urgent first: it waits on
/// you, nobody live is on it, nothing happened on it a while.
pub fn markers_of(n: &CardNode) -> Vec<Marker> {
    if n.done {
        return Vec::new();
    }
    let mut out = Vec::new();
    if n.needs {
        out.push(Marker::Needs);
    }
    if n.nobody {
        out.push(Marker::Nobody);
    }
    if n.stale {
        out.push(Marker::Stale);
    }
    out
}

/// The card a click on a node opens: a card itself, a blocker's card,
/// nothing for an agent or an area.
pub fn open_target(n: &Node) -> Option<String> {
    match n {
        Node::Card(c) => Some(c.id.clone()),
        Node::Blocker(b) => Some(b.card.clone()),
        _ => None,
    }
}

/// A status tone as a colour (card edges, the minimap).
pub fn tone_color(f: &Frame, t: Tone) -> Color {
    match t {
        Tone::Destructive => status::failed(),
        Tone::Success => status::working(),
        Tone::Info => status::review(),
        _ => f.k.c.text_muted,
    }
}

/// An agent status tone as a dot's colour.
pub fn status_dot(f: &Frame, t: StatusTone) -> Color {
    match t {
        StatusTone::Working | StatusTone::Job => status::working(),
        StatusTone::Warn => status::decide(),
        StatusTone::Idle => f.k.c.text_muted,
        StatusTone::Muted => f.k.c.text_muted.alpha(0.5),
    }
}

fn small(size: f32, weight: f32) -> rune::tok::TextStyle {
    rune::tok::TextStyle::new(size, weight)
}

/// A card node: the grid card's own state pill, ref, title, faces and
/// progress bar, so the canvas card reads like the list card.
pub fn card_body(s: &State, f: &mut Frame, key: &str, n: &CardNode) -> El {
    let k = f.k;
    let c = &n.card;
    let mut head = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .min_w(0.0)
        .child(bits::state_pill(f, &format!("{key}/pill"), c));
    if !n.place.is_empty() {
        head = head.child(
            text(n.place.clone())
                .font(small(10.5, 400.0))
                .color(k.c.text_muted)
                .max_lines(1)
                .shrink()
                .help(n.place.clone()),
        );
    }
    head = head.child(el().grow(1.0));
    if let Some(r) = bits::short_ref(f, &format!("{key}/ref"), c) {
        head = head.child(r);
    }

    let mut foot = row().w_full().gap(8.0).center_y().min_w(0.0);
    if !c.working.is_empty() {
        foot = foot.child(faces::faces(
            s,
            f,
            &format!("{key}/working"),
            &c.working,
            false,
            Some(faces::FACES_MAX),
        ));
    }
    if !c.watching.is_empty() {
        foot = foot.child(faces::faces(
            s,
            f,
            &format!("{key}/watching"),
            &c.watching,
            true,
            Some(faces::FACES_MAX),
        ));
    }
    if n.subtasks > 0 {
        foot = foot.child(
            text(format!(
                "{} subtask{}",
                n.subtasks,
                if n.subtasks == 1 { "" } else { "s" }
            ))
            .font(k.ty.text_xsmall)
            .color(k.c.text_muted),
        );
    }
    foot = foot.child(el().grow(1.0)).child(
        text(steps_text(n.done_steps, n.total_steps))
            .font(k.ty.text_xsmall)
            .color(k.c.text_muted),
    );

    let mut body = col()
        .w(CARD.w)
        .h(CARD.h)
        .padding(12.0)
        .gap(8.0)
        .min_w(0.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .clip_rounded(0.0)
        // The status colour down the left edge.
        .child(
            el().pin(Some(0.0), Some(0.0), None, Some(0.0))
                .w(2.0)
                .fill(tone_color(f, n.tone))
                .a11y_hidden(),
        )
        .child(head)
        .child(
            text(n.label.clone())
                .font(small(14.0, 500.0))
                .color(k.c.text_base)
                .w_full()
                .max_lines(2),
        )
        .child(el().grow(1.0))
        .child(foot);
    if let Some(bar) = bits::progress_bar(f, &format!("{key}/bar"), c) {
        body = body.child(bar);
    }
    body
}

/// The markers on a card's top edge, outside its clipped body, so the
/// flags read even zoomed out. `None` when it has none.
pub fn card_markers(f: &Frame, key: &str, n: &CardNode) -> Option<El> {
    let ms = markers_of(n);
    if ms.is_empty() {
        return None;
    }
    let k = f.k;
    let mut r =
        row()
            .key(format!("{key}/markers"))
            .gap(4.0)
            .pin(None, Some(-10.0), Some(8.0), None);
    for m in ms {
        let bg = match m {
            Marker::Needs => status::failed(),
            Marker::Nobody => status::decide(),
            Marker::Stale => k.c.text_muted,
        };
        r = r.child(
            el().px(6.0)
                .h(17.0)
                .center_y()
                .radius(3.0)
                .fill(bg)
                .help(m.title())
                .label(m.text())
                .child(
                    text(m.text().to_uppercase())
                        .font(small(10.0, 650.0))
                        .color(k.c.bg_base),
                ),
        );
    }
    Some(r)
}

/// An open blocker under its card: the question, red when it waits on the
/// person, amber when an answer waits on an agent.
pub fn blocker_body(f: &Frame, n: &BlockerNode) -> El {
    let k = f.k;
    let person = n.state == blocker::OPEN;
    let c = if person {
        status::failed()
    } else {
        status::decide()
    };
    col()
        .w(BLOCKER.w)
        .h(BLOCKER.h)
        .px(12.0)
        .py(8.0)
        .gap(4.0)
        .center_y()
        .border(1.0, c.alpha(0.6))
        .fill(k.c.bg_base.mix(c, 0.1))
        .child(
            text(if person {
                "NEEDS YOU"
            } else {
                "ANSWERED, WAITING ON AGENT"
            })
            .font(small(10.0, 650.0))
            .color(c),
        )
        .child(
            text(n.label.clone())
                .font(small(12.0, 400.0))
                .color(k.c.text_base)
                .w_full()
                .max_lines(2),
        )
}

/// The person's tint, or an agent's device colour, as an agent pill wears
/// it.
pub fn agent_tint(s: &State, f: &Frame, n: &AgentNode) -> Color {
    let person = || {
        rune::color::hex(&s.widgets.person_color)
            .or_else(|| faces::device_color(&s.widgets.person_color))
            .unwrap_or(f.k.c.icon_accent)
    };
    // You stay in the person tint; another person on a shared board wears
    // their own colour (card #385), as their agents do.
    if n.person && !n.agent.device_id.is_empty() {
        return person();
    }
    match faces::device_color(&n.agent.color) {
        Some(c) => c,
        None if n.person => person(),
        None => f.k.c.icon_accent,
    }
}

const PULSE: &[(f32, f32)] = &[(0.0, 0.55), (0.7, 0.0), (1.0, 0.0)];

/// An agent (or the person) as a pill in its device colour, its status as
/// a dot (card #587): green working (it pulses), amber when it needs
/// something, grey idle.
pub fn agent_body(s: &State, f: &Frame, key: &str, n: &AgentNode) -> El {
    let k = f.k;
    let tint = agent_tint(s, f, n);
    let dot = status_dot(f, n.tone);
    let working = matches!(n.tone, StatusTone::Working | StatusTone::Job);
    let tip = [
        if n.person {
            "you".to_string()
        } else {
            n.label.clone()
        },
        n.status.clone(),
        n.agent.where_.clone(),
    ]
    .into_iter()
    .filter(|x| !x.is_empty())
    .collect::<Vec<_>>()
    .join(" · ");
    let mut pill = row()
        .w(AGENT.w)
        .h(AGENT.h)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .radius(AGENT.h / 2.0)
        .border(1.0, tint.alpha(0.6))
        .fill(k.c.bg_base.mix(tint, 0.14))
        .help(tip);
    if working {
        // The pulse: a ring that swells out of the pill and fades.
        pill = pill.child(
            el().key(format!("{key}/pulse"))
                .pin(Some(-5.0), Some(-5.0), Some(-5.0), Some(-5.0))
                .radius(AGENT.h / 2.0 + 5.0)
                .border(3.0, dot)
                .a11y_hidden()
                .cycle(Cycle {
                    period: 2.4,
                    phase: 0.0,
                    stops: PULSE,
                    still: 0.0,
                    fps: 30.0,
                }),
        );
    }
    if n.person {
        pill = pill
            .child(
                el().size(8.0, 8.0)
                    .radius(4.0)
                    .fill(status::working())
                    .a11y_hidden(),
            )
            .child(
                text("you")
                    .font(k.ty.text_xsmall_medium)
                    .color(tint)
                    .max_lines(1)
                    .shrink(),
            );
    } else {
        pill = pill
            .child(faces::identicon(&n.agent.key, 16.0, tint))
            .child(
                text(n.label.clone())
                    .font(k.ty.text_xsmall_medium)
                    .color(tint)
                    .max_lines(1)
                    .shrink(),
            )
            .child(el().grow(1.0))
            .child(
                el().size(8.0, 8.0)
                    .radius(4.0)
                    .fill(dot)
                    .label(n.status.clone()),
            );
    }
    if !n.live && !n.person {
        pill = pill.opacity(0.55);
    }
    pill
}

/// An area's box: an agent's hub plain, the cards nobody is on in amber
/// (what nobody is working shows from across the canvas), done faint.
pub fn group_body(f: &Frame, n: &GroupNode, w: f32, h: f32) -> El {
    let k = f.k;
    let (border, fill, label) = match n.by {
        By::Agent => (
            k.c.border_base.alpha(0.7),
            k.c.text_base.alpha(0.02),
            k.c.text_muted,
        ),
        By::Unheld => (
            status::decide().alpha(0.6),
            status::decide().alpha(0.04),
            status::decide(),
        ),
        By::Others => (k.c.border_base.alpha(0.4), Color::default(), k.c.text_muted),
        By::Done => (
            k.c.border_base.alpha(0.3),
            Color::default(),
            k.c.text_muted.alpha(0.7),
        ),
    };
    el().w(w)
        .h(h)
        .border(1.0, border)
        .border_dash(5.0, 4.0)
        .fill(fill)
        .child(
            text(n.label.to_uppercase())
                .font(small(12.0, 500.0))
                .color(label)
                .max_lines(1)
                .pin(Some(12.0), Some(8.0), None, None),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::data::{Node, to_graph};
    use crate::logic::cards::state;
    use irori_window::cards::{AgentRef, CardHolder, CardSummary, Holding};
    use std::collections::HashMap;

    fn agent(key: &str, status: &str, live: bool) -> AgentRef {
        AgentRef {
            key: key.into(),
            name: key.into(),
            status: status.into(),
            live,
            ..AgentRef::default()
        }
    }

    fn card(id: &str, edit: impl FnOnce(&mut CardSummary)) -> CardSummary {
        let mut c = CardSummary {
            id: id.into(),
            short: id.into(),
            title: id.into(),
            state: state::WORKING.into(),
            holder: CardHolder {
                state: Holding::Active,
                ..CardHolder::default()
            },
            ..CardSummary::default()
        };
        edit(&mut c);
        c
    }

    #[test]
    fn waiting_on_you_nobody_on_it_and_stale_are_marked_done_cards_arent() {
        let cards = [
            card("m1", |c| {
                c.working = vec![agent("a", "busy", true)];
                c.open_blockers = 1;
            }),
            card("m2", |c| {
                c.working = vec![agent("g", "gone", false)];
                c.holder.state = Holding::Gone;
            }),
            card("m3", |c| {
                c.working = vec![agent("a", "idle", true)];
                c.stale = true;
                c.holder.state = Holding::Idle;
            }),
            card("m4", |c| {
                c.state = state::COMPLETED.into();
                c.stale = true;
            }),
        ];
        let refs: Vec<&CardSummary> = cards.iter().collect();
        let g = to_graph(&refs, &HashMap::new());
        let marks: Vec<Vec<Marker>> = g.cards().map(markers_of).collect();
        assert_eq!(
            marks,
            vec![
                vec![Marker::Needs],
                vec![Marker::Nobody],
                vec![Marker::Stale],
                vec![]
            ]
        );
    }

    #[test]
    fn steps_and_open_targets() {
        assert_eq!(steps_text(2, 5), "2/5 steps");
        assert_eq!(steps_text(0, 0), "");
        let c = card("x", |_| {});
        let refs = [&c];
        let g = to_graph(&refs, &HashMap::new());
        let n = g.node("x").unwrap();
        assert_eq!(open_target(n).as_deref(), Some("x"));
        let b = Node::Blocker(BlockerNode {
            id: "blocker:x:open".into(),
            label: String::new(),
            card: "x".into(),
            state: "open".into(),
            group: String::new(),
        });
        assert_eq!(open_target(&b).as_deref(), Some("x"));
    }
}
