//! The card's main panel (card-detail.tsx): Ready for you (the lifted
//! done check), the objective, and the blockers: BlockerItem with its
//! OptionItem answer buttons, own-words answer box and Mark resolved,
//! WaitItem, GoneRaiser and Hand to an agent, and the ToldLine that says
//! who heard an answer.

use std::rc::Rc;

use irori_window::board::{AnswerResult, Told};
use irori_window::cards::{AgentRef, CardBlocker, CardDetail, CardOption, NOTE_ITEM};
use rune::{El, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use super::{err_line, note, panel, section, ts};
use crate::logic::agent_status::{ms, now_ms};
use crate::logic::cards::{self as lc, Tone, blocker};
use crate::logic::names::display_name;
use crate::logic::plain::{plain_line, relative_time};
use crate::logic::thread::{
    BlockerTone, answerable, blocker_chip, blocker_item_id, blocker_tone, is_wait,
};
use crate::text::{self as tt, ComposerOpts, RichOpts, Sent};
use crate::ui::{Frame, State};
use crate::widgets::{bits, confirm, done_rounds};

/// A note answer the person just sent: the note item leaves the card once
/// answered, so the panel keeps it to show who heard.
#[derive(Clone, Debug)]
pub struct Answered {
    pub question: String,
    pub answer: String,
    pub told: Told,
    pub to: AgentRef,
}

/// What an answer did: who heard, and whether the item resolved.
#[derive(Clone, Debug)]
pub struct SentLine {
    pub told: Told,
    pub resolved: bool,
    pub irori: bool,
}

/// One blocker's own state on the page.
#[derive(Clone, Debug, Default)]
pub struct BlockerUi {
    pub unfolded: bool,
    pub earlier: bool,
    pub adding: bool,
    pub resolving: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub told: Option<SentLine>,
    pub missed: String,
    /// A permission prompt's pick, as it says what happens (card #682).
    pub said: String,
}

/// An answer's start, quoted on one line (card #265).
const GIST_CHARS: usize = 200;

pub fn gist(s: &str) -> String {
    let l = plain_line(s);
    if l.chars().count() > GIST_CHARS {
        format!("{}…", l.chars().take(GIST_CHARS - 1).collect::<String>())
    } else {
        l
    }
}

/// Recommended options first, each keeping its index (the answer sends
/// the index, so the order on screen is free).
pub fn by_recommendation(options: &[CardOption]) -> Vec<(&CardOption, usize)> {
    let all: Vec<(&CardOption, usize)> = options.iter().enumerate().map(|(i, o)| (o, i)).collect();
    all.iter()
        .filter(|(o, _)| o.recommended)
        .chain(all.iter().filter(|(o, _)| !o.recommended))
        .copied()
        .collect()
}

/// Why a shared board's card takes no files from the window.
pub const SHARED_NO_FILES: &str =
    "Files don't go on a shared space's cards from the app yet; an agent can attach them for you.";
/// Why the card's question takes none: its answer is a line on the card.
const NOTE_NO_FILES: &str = "Files go with a comment: add them in the comment box below.";

/// The left column's top panel: Ready for you, the objective, the
/// blockers. None when it has nothing to show.
pub fn main_panel(
    s: &State,
    f: &mut Frame,
    d: &CardDetail,
    ready_now: Option<&CardBlocker>,
) -> Option<El> {
    let c = &d.summary;
    let open: Vec<&CardBlocker> = d.blockers.iter().filter(|b| b.resolved.is_none()).collect();
    let legacy = c.open_blockers > 0 && open.is_empty();
    let show_note = !d.note.is_empty() && c.state != lc::state::INPUT_REQUIRED;
    // The done check lifted to the top stays there once answered.
    let lifted = s.card.ui.lifted.as_deref();
    let ready = ready_now.or_else(|| lifted.and_then(|l| d.blockers.iter().find(|b| b.id == l)));
    let blockers: Vec<&CardBlocker> = d
        .blockers
        .iter()
        .filter(|b| Some(b.id.as_str()) != ready.map(|r| r.id.as_str()))
        .collect();
    let answered = s
        .card
        .ui
        .answered
        .as_ref()
        .filter(|_| s.card.ui.card == c.id);
    let has_objective = !d.goal.is_empty() || !d.done_when.is_empty() || show_note;
    // A held card (Not yet) keeps its hold item off the page: the line
    // under the title says it, so the Blockers section isn't drawn.
    let has_blockers = (!blockers.is_empty() || legacy || answered.is_some()) && c.later.is_none();
    if ready.is_none() && !has_objective && !has_blockers {
        return None;
    }
    let k = f.k;
    let no_files = d.on_board.is_some();
    let mut p = panel(f, "card/main");
    if let Some(r) = ready {
        let item = blocker_item(s, f, d, r, no_files);
        p = p.child(section(
            f,
            "card/ready",
            if ready_now.is_some() {
                "Ready for you"
            } else {
                "Done check"
            },
            !has_objective && !has_blockers,
            vec![item],
        ));
    }
    if has_objective {
        let mut body = Vec::new();
        if !d.goal.is_empty() {
            let mentions = c.ask.as_ref().map(|a| a.mentions.as_slice()).unwrap_or(&[]);
            body.push(tt::rich_with(
                s,
                f,
                "card/goal",
                &d.goal,
                &RichOpts {
                    mentions,
                    font: Some(ts(14.0, 400.0)),
                    ..RichOpts::default()
                },
            ));
        }
        if let Some(ob) = &d.on_board {
            body.push(super::labelled(
                f,
                "Visible to",
                note(
                    f,
                    ob.visible_to
                        .iter()
                        .map(|v| plain_line(v))
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            ));
        }
        if show_note {
            body.push(super::page::label_rich(s, f, "card/note", "Note", &d.note));
        }
        if !d.done_when.is_empty() {
            body.push(super::page::label_rich(
                s,
                f,
                "card/done-when",
                "Done when",
                &d.done_when,
            ));
        }
        let title = if lc::open_ask(c.ask.as_ref()) {
            "You asked"
        } else {
            "Objective"
        };
        p = p.child(section(f, "card/objective", title, !has_blockers, body));
    }
    if has_blockers {
        let mut body = Vec::new();
        if legacy {
            body.push(
                col()
                    .key("card/legacy")
                    .w_full()
                    .gap(6.0)
                    .padding(12.0)
                    .radius(8.0)
                    .border(1.0, k.c.danger.border)
                    .fill(k.c.danger.bg)
                    .child(tt::rich_line(
                        s,
                        f,
                        "card/legacy-q",
                        &c.blocker_question,
                        &RichOpts {
                            font: Some(ts(13.5, 600.0)),
                            ..RichOpts::default()
                        },
                    ))
                    .child(note(f, "Raised without recommended answers. The agent on it clears it when it's sorted.")),
            );
        }
        if let Some(a) = answered
            && !open.iter().any(|b| b.id == NOTE_ITEM)
        {
            body.push(
                col()
                    .key("card/answered")
                    .w_full()
                    .gap(8.0)
                    .padding(12.0)
                    .radius(8.0)
                    .border(1.0, k.c.border_base)
                    .fill(k.c.layer_02)
                    .opacity(0.85)
                    .child(
                        row()
                            .w_full()
                            .gap(4.0)
                            .child(
                                text("✓")
                                    .font(ts(13.5, 600.0))
                                    .color(k.c.success.fg)
                                    .label("Answered: "),
                            )
                            .child(
                                text(plain_line(&a.question))
                                    .font(ts(13.5, 600.0))
                                    .color(k.c.text_base)
                                    .shrink()
                                    .wrap(),
                            ),
                    )
                    .child(
                        text(format!("You answered: “{}”", gist(&a.answer)))
                            .font(ts(11.5, 400.0))
                            .color(k.c.text_muted)
                            .w_full()
                            .wrap(),
                    )
                    .child(told_line(
                        f,
                        "card/answered-told",
                        &a.told,
                        &a.to,
                        None,
                        false,
                        false,
                    )),
            );
        }
        for b in blockers.iter().filter(|b| !is_wait(b)) {
            body.push(blocker_item(s, f, d, b, no_files));
        }
        for b in blockers.iter().filter(|b| is_wait(b)) {
            if let Some(w) = wait_item(s, f, &c.id, b) {
                body.push(w);
            }
        }
        p = p.child(section(f, "card/blockers", "Blockers", true, body));
    }
    Some(p)
}

fn ui(s: &State, id: &str) -> BlockerUi {
    s.card.ui.blockers.get(id).cloned().unwrap_or_default()
}

pub(super) fn ui_mut<'a>(s: &'a mut State, id: &str) -> &'a mut BlockerUi {
    s.card.ui.blockers.entry(id.to_string()).or_default()
}

/// What a box holds, as a send (its text, its kept mentions, its files).
fn take(s: &State, get: crate::text::Get) -> Sent {
    let c = get(s);
    Sent {
        text: c.edit.text.trim().to_string(),
        mentions: crate::text::mentions::keep_mentions(&c.mentions, &c.edit.text),
        files: c.files.iter().map(|p| p.upload()).collect(),
    }
}

/// An item's small line: who raised a done check, when, who decides and
/// how it ended, joined by " · " with nothing left dangling when the
/// time is unknown.
pub(crate) fn meta_line(
    raiser: Option<&str>,
    raised: &str,
    decider: Option<&str>,
    by: &str,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(r) = raiser {
        parts.push(format!("Raised by {r}"));
    }
    if !raised.is_empty() {
        parts.push(raised.to_string());
    }
    if let Some(d) = decider {
        parts.push(format!("for {d} to decide"));
    }
    if !by.is_empty() {
        parts.push(by.to_string());
    }
    parts.join(" · ")
}

/// Sends an answer: `option` is a picked recommendation's index, or -1
/// for own words.
fn send(s: &mut State, card: &str, b: &CardBlocker, option: i64) {
    let bid = b.id.clone();
    let key = format!("{card}/blocker/{bid}");
    let Some((get, get_mut)) = super::slot(s, &key) else {
        return;
    };
    if ui(s, &bid).busy {
        return;
    }
    let mut sent = take(s, get);
    let replying =
        b.options_aside || matches!(b.state.as_str(), blocker::ANSWERED | blocker::REPLIED);
    if option < 0 && sent.text.is_empty() {
        ui_mut(s, &bid).error = Some(
            if replying {
                "Write a reply first."
            } else {
                "Write an answer first."
            }
            .into(),
        );
        return;
    }
    // A picked option carries no text, so no mentions.
    if option >= 0 {
        sent.mentions.clear();
        sent.text.clear();
    }
    {
        let u = ui_mut(s, &bid);
        u.busy = true;
        u.error = None;
        u.told = None;
    }
    get_mut(s).sending = true;
    let (cid, own) = (card.to_string(), sent.text.clone());
    let question = b.question.clone();
    let raiser = b.raised_by.clone();
    let ms = crate::text::mentions::to_ts(&sent.mentions);
    let files = sent.files;
    let bid2 = bid.clone();
    s.spawn(
        move |w| {
            w.answer_files(&cid, &bid2, option, &own, true, &ms, &files)
                .map(|r| (r, own))
        },
        move |s, r| {
            get_mut(s).sending = false;
            ui_mut(s, &bid).busy = false;
            match r {
                Ok((res, own)) => {
                    heard(s, &bid, &question, &raiser, &res, Some(own));
                    // A pick keeps what was typed; own words leave the box.
                    if option < 0 {
                        crate::text::sent(s, get_mut);
                        ui_mut(s, &bid).adding = false;
                    } else {
                        get_mut(s).files.clear();
                    }
                }
                Err(e) => ui_mut(s, &bid).error = Some(e),
            }
        },
    );
}

/// Shows what a saved answer's reply says. `own` is its words ("" for a
/// pick), None for Mark resolved (the check says it all).
fn heard(
    s: &mut State,
    bid: &str,
    question: &str,
    raiser: &AgentRef,
    res: &AnswerResult,
    own: Option<String>,
) {
    if bid == NOTE_ITEM
        && let Some(o) = &own
    {
        s.card.ui.answered = Some(Answered {
            question: question.to_string(),
            answer: o.trim().to_string(),
            told: res.told.clone(),
            to: raiser.clone(),
        });
    }
    let u = ui_mut(s, bid);
    u.told = own.map(|_| SentLine {
        told: res.told.clone(),
        resolved: res.resolved,
        irori: res.irori_acts,
    });
    u.missed = crate::text::mentions::not_delivered(&res.mentioned);
}

/// Mark resolved, with the note box's words.
fn mark_resolved(s: &mut State, card: &str, b: &CardBlocker) {
    let bid = b.id.clone();
    let key = format!("{card}/resolve/{bid}");
    let Some((get, get_mut)) = super::slot(s, &key) else {
        return;
    };
    if ui(s, &bid).busy {
        return;
    }
    let sent = take(s, get);
    {
        let u = ui_mut(s, &bid);
        u.busy = true;
        u.error = None;
    }
    let cid = card.to_string();
    let ms = crate::text::mentions::to_ts(&sent.mentions);
    let (question, raiser) = (b.question.clone(), b.raised_by.clone());
    let bid2 = bid.clone();
    s.spawn(
        move |w| w.resolve_blocker(&cid, &bid2, &sent.text, &ms),
        move |s, r| {
            ui_mut(s, &bid).busy = false;
            match r {
                Ok(res) => {
                    heard(s, &bid, &question, &raiser, &res, None);
                    crate::text::sent(s, get_mut);
                    let u = ui_mut(s, &bid);
                    u.resolving = false;
                    u.adding = false;
                }
                Err(e) => ui_mut(s, &bid).error = Some(e),
            }
        },
    );
}

/// How a resolved item ended, on its folded line (notifications.ts
/// blockerOutcome for a sent-back done check).
pub fn resolved_by(b: &CardBlocker) -> String {
    let Some(r) = &b.resolved else {
        return String::new();
    };
    if b.done_check && r.note == ASKED_AGAIN {
        return blocker_outcome(b);
    }
    if r.closed {
        let a = if r.answer.is_empty() {
            String::new()
        } else {
            format!(": “{}”", gist(&r.answer))
        };
        return format!("closed by {}{a}", display_name(&r.by));
    }
    if !r.note.is_empty() {
        return format!("resolved by {}: “{}”", display_name(&r.by), gist(&r.note));
    }
    format!("{} answered: “{}”", display_name(&r.by), gist(&r.answer))
}

/// How an agent's next done check resolves the one it replaces.
pub const ASKED_AGAIN: &str = "asked again: ready for the person";

/// notifications.ts blockerOutcome: how a resolved blocker or done check
/// ended.
pub fn blocker_outcome(b: &CardBlocker) -> String {
    let Some(r) = &b.resolved else {
        return if b.state == blocker::ANSWERED {
            "Answered; it's with the agent now".into()
        } else {
            String::new()
        };
    };
    let who = |n: &str| {
        let n = n.trim();
        if n.is_empty() {
            "someone".to_string()
        } else if n == "You" {
            "you".to_string()
        } else {
            n.to_string()
        }
    };
    let by = who(&r.by.name);
    let note = r.note.trim();
    if r.closed {
        return format!(
            "Closed by {by}{}",
            if note.is_empty() {
                String::new()
            } else {
                format!(": {note}")
            }
        );
    }
    let answer = r.answer.trim();
    if b.done_check && note == ASKED_AGAIN {
        return if !answer.is_empty() && answer != ASKED_AGAIN {
            format!("You sent it back: “{answer}”; {by} asked again")
        } else {
            format!("{by} asked again")
        };
    }
    if b.done_check {
        return if r.option >= 0 {
            format!("Marked done by {by}")
        } else {
            format!(
                "Sent back by {by}{}",
                if answer.is_empty() {
                    String::new()
                } else {
                    format!(": “{answer}”")
                }
            )
        };
    }
    if r.option >= 0 {
        return format!("Answered “{answer}” by {by}");
    }
    format!(
        "Answered by {by}{}",
        if answer.is_empty() {
            String::new()
        } else {
            format!(": “{answer}”")
        }
    )
}

/// One blocker: the question, context, who raised it, its own thread and,
/// while the person can answer, the options as buttons with a box for
/// their own words. Open and replied are amber, answered grey (the form
/// behind "Add to your answer"), resolved folds to one line.
pub fn blocker_item(
    s: &State,
    f: &mut Frame,
    d: &CardDetail,
    b: &CardBlocker,
    no_files: bool,
) -> El {
    let k = f.k;
    let cid = d.summary.id.clone();
    let u = ui(s, &b.id);
    let key = blocker_item_id(&b.id);
    let r = b.resolved.as_ref();
    let st = if b.state.is_empty() {
        blocker::OPEN
    } else {
        b.state.as_str()
    };
    let tone = blocker_tone(st);
    let answered = st == blocker::ANSWERED || st == blocker::REPLIED;
    let aside = b.options_aside;
    let replying = aside || answered;
    let box_key = format!("{cid}/blocker/{}", b.id);
    let note_key = format!("{cid}/resolve/{}", b.id);
    let typed = super::slot(s, &box_key).is_some_and(|(g, _)| !g(s).is_empty());
    let noted = super::slot(s, &note_key).is_some_and(|(g, _)| !g(s).is_empty());
    let adding = u.adding || typed;
    let resolving = u.resolving || noted;
    let show_form = answerable(st) && (st != blocker::ANSWERED || adding || aside);
    let by = resolved_by(b);

    if r.is_some() && !u.unfolded && u.told.is_none() && u.error.is_none() {
        let bid = b.id.clone();
        let line = format!("{} · {by}", plain_line(&b.question));
        let q_len = plain_line(&b.question).len();
        return row()
            .key(key)
            .w_full()
            .min_w(0.0)
            .gap(8.0)
            .px(12.0)
            .py(8.0)
            .radius(8.0)
            .border(1.0, k.c.border_base)
            .fill(k.c.layer_02)
            .opacity(0.85)
            .cursor(rune::Cursor::Pointer)
            .role("button")
            .help("Show the whole item")
            .on_press(move |s: &mut State| ui_mut(s, &bid).unfolded = true)
            .child(
                text("✓")
                    .font(ts(12.5, 600.0))
                    .color(k.c.success.fg)
                    .label("Resolved: "),
            )
            .child(
                rune::rich(
                    line,
                    vec![rune::Span::new(0, q_len).weight(600.0).color(k.c.text_base)],
                )
                .font(ts(12.5, 400.0))
                .color(k.c.text_muted)
                .max_lines(1)
                .shrink()
                .min_w(0.0),
            );
    }

    // A spawned agent's permission prompt (card #682): what waits and its
    // buttons; no answer box or thread, since words don't reach a prompt.
    if b.permission.is_some() {
        return super::permission::item(s, f, d, b, &u);
    }

    let (fill, border, dim) = match tone {
        BlockerTone::Amber if b.done_check => (k.c.success.bg.alpha(0.4), k.c.success.border, 1.0),
        BlockerTone::Amber => (k.c.danger.bg.alpha(0.4), k.c.danger.border, 1.0),
        BlockerTone::Grey => (k.c.layer_02, k.c.border_base, 1.0),
        BlockerTone::Green | BlockerTone::Closed => (k.c.layer_02, k.c.border_base, 0.85),
    };
    let mut item = col()
        .key(key)
        .w_full()
        .min_w(0.0)
        .gap(8.0)
        .padding(12.0)
        .radius(12.0)
        .border(1.0, border)
        .fill(fill)
        .opacity(dim);
    if !b.done_check {
        item = item.child(bits::asker(
            f,
            &format!("card/b/{}/asker", b.id),
            &b.raised_by,
        ));
    }
    let mut head = row().w_full().min_w(0.0).gap(6.0).items_start();
    if r.is_some() {
        head = head.child(
            text("✓")
                .font(ts(13.5, 600.0))
                .color(k.c.success.fg)
                .label("Resolved: "),
        );
    }
    head = head.child(
        tt::rich_line(
            s,
            f,
            &format!("card/b/{}/q", b.id),
            &b.question,
            &RichOpts {
                font: Some(ts(13.5, 600.0)),
                ..RichOpts::default()
            },
        )
        .grow(1.0)
        .min_w(0.0),
    );
    if let Some(chip) = blocker_chip(b) {
        head = head.child(bits::badge(
            f,
            &format!("card/b/{}/chip", b.id),
            &plain_line(&chip.label),
            chip.tone,
            false,
            "",
        ));
    }
    if r.is_some() {
        let bid = b.id.clone();
        head = head.child(
            Button::new("Fold")
                .variant(ButtonVariant::GhostMuted)
                .control_size(ControlSize::Mini)
                .on_press(move |s: &mut State| ui_mut(s, &bid).unfolded = false)
                .build(k),
        );
    }
    item = item.child(head);
    if b.done_check {
        if let Some(e) =
            done_rounds::earlier_round(s, f, &format!("card/b/{}/earlier", b.id), b, false)
        {
            item = item.child(e);
        }
        item = item.child(tt::attachment_list(
            s,
            f,
            &format!("card/b/{}/files", b.id),
            &super::files_once(&b.attachments),
            false,
        ));
        if b.previous.is_some() && !b.context.is_empty() {
            item = item.child(
                text("New proof")
                    .font(ts(11.5, 500.0))
                    .color(k.c.text_muted),
            );
        }
    }
    if !b.context.is_empty() {
        item = item.child(tt::rich_with(
            s,
            f,
            &format!("card/b/{}/ctx", b.id),
            &b.context,
            &RichOpts {
                font: Some(ts(12.5, 400.0)),
                ..RichOpts::default()
            },
        ));
    }
    if !b.links.is_empty() {
        let mut links = row()
            .w_full()
            .gap(6.0)
            .flex_wrap()
            .label(format!("Links for: {}", plain_line(&b.question)));
        for (i, l) in b.links.iter().enumerate() {
            links = links.child(tt::link_chip(
                f,
                &format!("card/b/{}/link{i}", b.id),
                &l.kind,
                &l.reference,
                &l.label,
            ));
        }
        item = item.child(links);
    }
    if !b.done_check && !b.attachments.is_empty() {
        item = item.child(tt::attachment_list(
            s,
            f,
            &format!("card/b/{}/files", b.id),
            &super::files_once(&b.attachments),
            false,
        ));
    }
    if r.is_none()
        && let Some(g) = &b.gone_agent
    {
        item = item.child(gone_raiser(s, f, &cid, &b.id, g));
    }
    // Who raised it, when, who decides, how it ended.
    let raised = ms(&b.at)
        .map(|t| relative_time(t, now_ms()))
        .unwrap_or_default();
    let decider = b.decider.as_ref().filter(|_| r.is_none()).map(display_name);
    let meta = meta_line(
        b.done_check.then(|| display_name(&b.raised_by)).as_deref(),
        &raised,
        decider.as_deref(),
        &by,
    );
    if !meta.is_empty() {
        item = item.child(
            text(meta)
                .font(ts(11.5, 400.0))
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
        );
    }
    if let Some(sb) = done_rounds::sent_back_signs(f, &format!("card/b/{}/signs", b.id), b) {
        item = item.child(sb);
    }

    let form = if show_form {
        Some(answer_form(s, f, &cid, b, &u, replying, no_files))
    } else {
        None
    };
    if !b.thread.is_empty() {
        let opts = super::ThreadOpts {
            card_id: cid.clone(),
            items: &b.thread,
            compact: true,
            in_blocker: true,
            composer: form,
            ..super::ThreadOpts::default()
        };
        item = item.child(super::thread_view(
            s,
            f,
            &format!("card/b/{}/thread", b.id),
            &opts,
        ));
    } else if let Some(fm) = form {
        item = item.child(fm);
    }
    if answered && !u.busy {
        let mut acts = row().w_full().gap(12.0).center_y().flex_wrap();
        if st == blocker::ANSWERED && !adding && !aside {
            let bid = b.id.clone();
            acts = acts.child(
                Button::new("Add to your answer")
                    .variant(ButtonVariant::GhostMuted)
                    .control_size(ControlSize::Mini)
                    .on_press(move |s: &mut State| ui_mut(s, &bid).adding = true)
                    .build(k),
            );
        }
        if !resolving {
            let bid = b.id.clone();
            acts = acts.child(
                Button::new("Mark resolved")
                    .variant(ButtonVariant::GhostMuted)
                    .control_size(ControlSize::Mini)
                    .on_press(move |s: &mut State| ui_mut(s, &bid).resolving = true)
                    .build(k),
            );
        }
        item = item.child(acts);
    }
    if resolving {
        item = item.child(resolve_form(s, f, &cid, b, &u));
    }
    if let Some(t) = &u.told {
        // Only while it still says what is so.
        let shows = if t.resolved {
            r.is_some()
        } else {
            st == blocker::ANSWERED
        };
        if shows {
            let sent = if b.id == NOTE_ITEM {
                None
            } else {
                Some(t.resolved)
            };
            item = item.child(told_line(
                f,
                &format!("card/b/{}/told", b.id),
                &t.told,
                &b.raised_by,
                sent,
                b.done_check,
                t.irori,
            ));
        }
    }
    if !u.missed.is_empty() {
        item = item.child(err_line(f, u.missed.clone()));
    }
    if let Some(e) = &u.error {
        item = item.child(err_line(f, e.clone()));
    }
    item
}

/// The options to pick, and the own-words box with its button.
fn answer_form(
    s: &State,
    f: &mut Frame,
    cid: &str,
    b: &CardBlocker,
    u: &BlockerUi,
    replying: bool,
    no_files: bool,
) -> El {
    let k = f.k;
    let mut form = col()
        .key(format!("card/b/{}/form", b.id))
        .w_full()
        .min_w(0.0)
        .gap(6.0);
    let aside = b.options_aside;
    if aside && !b.options.is_empty() {
        let bid = b.id.clone();
        let mut e = col().w_full().gap(4.0).child(
            row().child(
                Button::new(format!(
                    "{} Earlier options ({})",
                    if u.earlier { "▾" } else { "▸" },
                    b.options.len()
                ))
                .variant(ButtonVariant::GhostMuted)
                .control_size(ControlSize::Mini)
                .expanded(u.earlier)
                .on_press(move |s: &mut State| {
                    let u = ui_mut(s, &bid);
                    u.earlier = !u.earlier;
                })
                .build(k),
            ),
        );
        if u.earlier {
            let mut list = col()
                .w_full()
                .gap(4.0)
                .pl(12.0)
                .border_sides([0.0, 0.0, 0.0, 1.0], k.c.border_base)
                .label(format!("Earlier options for: {}", plain_line(&b.question)));
            for (i, o) in b.options.iter().enumerate() {
                let mut line = plain_line(&o.answer);
                if o.recommended {
                    line.push_str(&format!(" (recommended by {})", display_name(&o.by)));
                }
                list = list.child(
                    text(line)
                        .font(k.ty.text_xsmall)
                        .color(k.c.text_muted)
                        .w_full()
                        .wrap()
                        .key(format!("card/b/{}/old{i}", b.id)),
                );
            }
            e = e.child(list);
        }
        form = form.child(e);
    }
    if !aside && !b.options.is_empty() {
        let mut grid = row()
            .w_full()
            .gap(6.0)
            .flex_wrap()
            .role("group")
            .label(format!("Answers to: {}", plain_line(&b.question)));
        for (o, i) in by_recommendation(&b.options) {
            grid = grid.child(option_item(s, f, cid, b, o, i, u.busy));
        }
        form = form.child(grid);
    }
    let files_off = if no_files {
        Some(SHARED_NO_FILES.to_string())
    } else if b.id == NOTE_ITEM {
        Some(NOTE_NO_FILES.to_string())
    } else {
        None
    };
    let placeholder = if replying {
        "Say more, or describe another option"
    } else if b.done_check {
        if b.gone_agent.is_some() {
            "Or say why it isn't done; the next agent on it gets it"
        } else {
            "Or say why it isn't done; it goes back to the agent"
        }
    } else if !b.options.is_empty() {
        "Or answer in your own words"
    } else {
        "Answer in your own words"
    };
    let label = if u.busy {
        "Sending…"
    } else if replying {
        "Reply"
    } else if b.done_check {
        "Not done"
    } else {
        "Answer"
    };
    let key = format!("{cid}/blocker/{}", b.id);
    let Some((get, get_mut)) = super::slot(s, &key) else {
        return form.child(note(
            f,
            "Too many open items on this card to answer them all here; answer one first.",
        ));
    };
    let bb = b.clone();
    let cid2 = cid.to_string();
    let send_btn = Button::new(label)
        .variant(ButtonVariant::Submit)
        .control_size(ControlSize::Small)
        .disabled(u.busy)
        .on_press(move |s: &mut State| send(s, &cid2, &bb, -1))
        .build(k);
    let bb = b.clone();
    let cid3 = cid.to_string();
    let opts = ComposerOpts {
        placeholder: placeholder.into(),
        label: format!(
            "{}: {}",
            if replying {
                "Your reply on"
            } else {
                "Your answer to"
            },
            plain_line(&b.question)
        ),
        on_send: Some(Rc::new(move |s: &mut State, _m: Sent| {
            send(s, &cid3, &bb, -1)
        })),
        extra: vec![send_btn],
        card: cid.to_string(),
        no_files: files_off,
        disabled: u.busy,
        rows: Some((1, 8)),
        popup_below: true,
        ..ComposerOpts::default()
    };
    let _ = get_mut;
    form.child(tt::composer(
        s,
        f,
        &format!("card/b/{}/box", b.id),
        get,
        get_mut,
        &opts,
    ))
}

/// One answer to pick: the button (the answer, who recommends it); its why
/// and link beside it, so their links can be clicked without picking.
fn option_item(
    s: &State,
    f: &mut Frame,
    cid: &str,
    b: &CardBlocker,
    o: &CardOption,
    i: usize,
    busy: bool,
) -> El {
    let k = f.k;
    let key = format!("card/b/{}/opt{i}", b.id);
    let bb = b.clone();
    let cid = cid.to_string();
    let mut head = row().w_full().min_w(0.0).gap(6.0).items_start().child(
        tt::rich_line(
            s,
            f,
            &format!("{key}/a"),
            &o.answer,
            &RichOpts {
                font: Some(ts(13.0, 600.0)),
                interactive: false,
                selectable: false,
                ..RichOpts::default()
            },
        )
        .grow(1.0)
        .min_w(0.0),
    );
    if o.recommended {
        head = head.child(bits::badge(
            f,
            &format!("{key}/rec"),
            "Recommended",
            Tone::Info,
            true,
            "",
        ));
    }
    let who = if o.recommended {
        text(format!("{} recommends this.", display_name(&o.by)))
            .font(ts(12.0, 500.0))
            .color(k.c.text_accent)
    } else {
        text(format!("Option from {}.", display_name(&o.by)))
            .font(ts(12.0, 400.0))
            .color(k.c.text_muted)
    };
    let pick = col()
        .key(format!("{key}/pick"))
        .w_full()
        .gap(2.0)
        .px(10.0)
        .pt(8.0)
        .pb(if o.why.is_empty() && o.link.is_none() {
            8.0
        } else {
            2.0
        })
        .role("button")
        .label(plain_line(&o.answer))
        .focusable(!busy)
        .cursor(if busy {
            rune::Cursor::Default
        } else {
            rune::Cursor::Pointer
        })
        .opacity(if busy { 0.6 } else { 1.0 })
        .on_press(move |s: &mut State| send(s, &cid, &bb, i as i64))
        .child(head)
        .child(who);
    let mut c = col()
        .key(key.clone())
        .grow(1.0)
        .min_w(240.0)
        .w(280.0)
        .gap(4.0)
        .radius(8.0)
        .border(
            1.0,
            if o.recommended {
                k.c.text_accent.alpha(0.6)
            } else {
                k.c.border_base
            },
        )
        .fill(if o.recommended {
            k.c.bg_accent.alpha(0.12)
        } else {
            k.c.layer_01
        })
        .child(pick);
    if !o.why.is_empty() || o.link.is_some() {
        let mut extra = col().w_full().gap(4.0).px(10.0).pb(8.0);
        if !o.why.is_empty() {
            extra = extra.child(tt::rich_with(
                s,
                f,
                &format!("{key}/why"),
                &o.why,
                &RichOpts {
                    font: Some(ts(12.0, 400.0)),
                    color: Some(k.c.text_muted),
                    ..RichOpts::default()
                },
            ));
        }
        if let Some(l) = &o.link {
            extra = extra.child(row().child(tt::link_chip(
                f,
                &format!("{key}/link"),
                &l.kind,
                &l.reference,
                &l.label,
            )));
        }
        c = c.child(extra);
    }
    c
}

/// Mark resolved's note box with its Save and Cancel.
fn resolve_form(s: &State, f: &mut Frame, cid: &str, b: &CardBlocker, u: &BlockerUi) -> El {
    let k = f.k;
    let key = format!("{cid}/resolve/{}", b.id);
    let Some((get, get_mut)) = super::slot(s, &key) else {
        return el();
    };
    let (bb, cid2) = (b.clone(), cid.to_string());
    let (bb2, cid3) = (b.clone(), cid.to_string());
    let bid = b.id.clone();
    let opts = ComposerOpts {
        placeholder: "A note beside the check (optional)".into(),
        label: format!("Note on resolving: {}", plain_line(&b.question)),
        on_send: Some(Rc::new(move |s: &mut State, _m: Sent| {
            mark_resolved(s, &cid2, &bb)
        })),
        extra: vec![
            Button::new(if u.busy { "Saving…" } else { "Mark resolved" })
                .variant(ButtonVariant::Submit)
                .control_size(ControlSize::Small)
                .disabled(u.busy)
                .on_press(move |s: &mut State| mark_resolved(s, &cid3, &bb2))
                .build(k),
            Button::new("Cancel")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .disabled(u.busy)
                .on_press(move |s: &mut State| {
                    ui_mut(s, &bid).resolving = false;
                    crate::text::sent(s, get_mut);
                })
                .build(k),
        ],
        card: cid.to_string(),
        hide_files: true,
        disabled: u.busy,
        rows: Some((1, 6)),
        popup_below: true,
        ..ComposerOpts::default()
    };
    tt::composer(s, f, &format!("card/b/{}/note", b.id), get, get_mut, &opts)
}

/// Says who heard an answer and how (ToldLine).
pub fn told_line(
    f: &Frame,
    key: &str,
    told: &Told,
    raiser: &AgentRef,
    sent: Option<bool>,
    done: bool,
    irori: bool,
) -> El {
    let k = f.k;
    let name = display_name(raiser);
    let (words, color) = if !told.error.is_empty() {
        (
            format!("Answered, but {name} couldn't be told: {}", told.error),
            k.c.danger.fg,
        )
    } else if sent == Some(true) {
        (
            if done {
                "Done: the card is closed."
            } else if irori {
                "Sent to Irori: it starts the agent, or asks again."
            } else if !told.to.is_empty() {
                "Resolved: the agent on the card gets your answer."
            } else {
                "Resolved: nobody is running to act on it."
            }
            .to_string(),
            k.c.success.fg,
        )
    } else if sent == Some(false) {
        (
            format!("Sent to {name}; it stays open until {name} resolves it."),
            k.c.text_muted,
        )
    } else if told.to.is_empty() {
        (
            "Answered. Nobody needed telling.".to_string(),
            k.c.text_muted,
        )
    } else {
        let wake = told.mode == "wake";
        (
            format!(
                "{name} {} your answer{} and carries on.",
                if wake { "is woken with" } else { "gets" },
                if wake { "" } else { " at its next turn" }
            ),
            k.c.success.fg,
        )
    };
    text(words)
        .key(key.to_string())
        .font(k.ty.text_xsmall)
        .color(color)
        .w_full()
        .wrap()
}

/// A blocker that waits on other work (card #481): a grey line naming it,
/// linked when it's a card, with Stop waiting; amber with Hand to an agent
/// when its agent is gone and nobody holds the card (card #485).
fn wait_item(s: &State, f: &mut Frame, cid: &str, b: &CardBlocker) -> Option<El> {
    let w = b.waits_on.as_ref()?;
    let k = f.k;
    let gone = b.gone_agent.is_some();
    let label = plain_line(&w.label);
    let mut what = row().w_full().gap(4.0).min_w(0.0).items_start().child(
        text("Waiting on")
            .font(ts(13.0, 400.0))
            .color(k.c.text_muted),
    );
    if !w.task_id.is_empty() {
        what = what.child(super::page::card_link(
            f,
            &format!("card/w/{}/card", b.id),
            &w.task_id,
            &w.label,
            &w.title,
        ));
        if gone {
            what = what.child(text("(gone)").font(ts(13.0, 500.0)).color(k.c.warning.fg));
        }
    } else if gone {
        // The name and "(gone)" read as one line, as React's inline spans.
        let words = format!("{label} (gone)");
        let spans = vec![
            rune::Span::new(0, label.len()).color(k.c.text_base),
            rune::Span::new(label.len() + 1, words.len())
                .color(k.c.warning.fg)
                .weight(500.0),
        ];
        what = what.child(
            rune::rich(words, spans)
                .font(ts(13.0, 400.0))
                .color(k.c.text_muted)
                .shrink()
                .min_w(0.0)
                .wrap(),
        );
    } else {
        what = what.child(
            tt::rich_line(
                s,
                f,
                &format!("card/w/{}/label", b.id),
                &w.label,
                &RichOpts {
                    font: Some(ts(13.0, 400.0)),
                    ..RichOpts::default()
                },
            )
            .grow(1.0)
            .min_w(0.0),
        );
    }
    let mut left = col().grow(1.0).min_w(0.0).gap(2.0).child(what);
    if !b.question.is_empty() {
        left = left.child(
            text(plain_line(&b.question))
                .font(k.ty.text_xsmall)
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
        );
    }
    let mut buttons = row().gap(6.0).items_start();
    if let Some(g) = &b.gone_agent {
        buttons = buttons.child(hand_on_button(
            s,
            f,
            &format!("card/w/{}/hand", b.id),
            cid,
            g,
        ));
    }
    let (cid2, bid) = (cid.to_string(), b.id.clone());
    let lbl = label.clone();
    buttons = buttons.child(confirm::confirm_button(
        s,
        f,
        &format!("card/w/{}/stop", b.id),
        "Stop waiting",
        None,
        false,
        confirm::Ask::new(
            format!("Stop waiting on {label}? The card no longer waits on it, and whoever is on it hears."),
            "Stop waiting",
            move |w| w.resolve_blocker(&cid2, &bid, &format!("Stopped waiting on {lbl}"), &[]).map(|_| ()),
        )
        .rich(),
    ));
    let mut c = col()
        .key(blocker_item_id(&b.id))
        .w_full()
        .min_w(0.0)
        .gap(4.0)
        .px(12.0)
        .py(8.0)
        .radius(8.0)
        .border(
            1.0,
            if gone {
                k.c.warning.border
            } else {
                k.c.border_base
            },
        )
        .fill(if gone { k.c.warning.bg } else { k.c.layer_02 })
        .label(format!(
            "Waiting on {}{}",
            w.label,
            if gone { " (gone)" } else { "" }
        ))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .child(left)
                .child(buttons),
        );
    if !b.context.is_empty() {
        c = c.child(tt::rich_line(
            s,
            f,
            &format!("card/w/{}/ctx", b.id),
            &b.context,
            &RichOpts {
                font: Some(k.ty.text_xsmall),
                color: Some(k.c.text_muted),
                ..RichOpts::default()
            },
        ));
    }
    if gone {
        c = c.child(
            text("Nobody holds this card, so nothing will move it.")
                .font(k.ty.text_xsmall)
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
        );
    }
    Some(c)
}

/// "Hand to an agent" (card #485): the card goes back on the Board with
/// what was open said on it, and Irori starts an agent for it.
fn hand_on_button(s: &State, f: &Frame, key: &str, cid: &str, who: &AgentRef) -> El {
    let cid = cid.to_string();
    confirm::confirm_button(
        s,
        f,
        key,
        "Hand to an agent",
        None,
        false,
        confirm::Ask::new(
            format!(
                "Hand this card to a new agent? {} is gone. Its open items close and are said on the card, and Irori starts an agent for it, or the next free agent takes it.",
                display_name(who)
            ),
            "Hand to an agent",
            move |w| w.hand_on(&cid).map(|_| ()),
        )
        .rich(),
    )
}

/// The amber line under an item whose agent is gone on a card nobody
/// holds (card #485).
fn gone_raiser(s: &State, f: &Frame, cid: &str, bid: &str, who: &AgentRef) -> El {
    let k = f.k;
    let lead = format!("Waiting on {} (gone)", display_name(who));
    let words =
        format!("{lead}. Nobody holds this card: your answer goes to the agent it goes to next.");
    row()
        .key(format!("card/b/{bid}/gone"))
        .w_full()
        .min_w(0.0)
        .gap(8.0)
        .px(10.0)
        .py(6.0)
        .center_y()
        .flex_wrap()
        .radius(6.0)
        .border(1.0, k.c.warning.border)
        .fill(k.c.warning.bg)
        .role("status")
        .child(
            rune::rich(
                words,
                vec![
                    rune::Span::new(0, lead.len())
                        .weight(500.0)
                        .color(k.c.warning.fg),
                ],
            )
            .font(ts(12.5, 400.0))
            .color(k.c.text_base)
            .grow(1.0)
            .min_w(0.0)
            .wrap(),
        )
        .child(hand_on_button(
            s,
            f,
            &format!("card/b/{bid}/hand"),
            cid,
            who,
        ))
}
