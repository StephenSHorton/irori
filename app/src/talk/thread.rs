//! An open thread (talk-page.tsx `Conversation` and threads.tsx
//! `ThreadCards`, `ThreadView`, `OpenedBy`): what was asked, the answers
//! (the card's own thread, events left out), the reply box and Resolve;
//! and "Start a thread" (the ask box) when nothing is open.
//!
//! The answers and the reply box are the card package's thread view and
//! CommentBox (`crate::card::thread_view`, `reply_box`), as threads.tsx
//! used cards/thread.tsx; StartPastLimit is the card package's too.

use std::rc::Rc;

use irori_window::cards::{AgentRef, CardThreadItem};
use irori_window::threads::{THREAD_RESOLVED, THREAD_WAITING, ThreadSummary};
use rune::{El, ScrollSpec, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use super::list::ts;
use super::logic;
use super::{ask_box, ask_box_mut, icons};
use crate::card::{On, ThreadOpts};
use crate::logic::agent_status::now_ms;
use crate::logic::names::display_name;
use crate::logic::plain::{plain_line, relative_time};
use crate::logic::spawn_line::SpawnState;
use crate::text::{ComposerOpts, RichOpts, Sent};
use crate::ui::theme::status;
use crate::ui::{Frame, State};
use crate::widgets::bits::badge;
use crate::widgets::faces::{FaceOpts, face};

/// The pane for thread `t`.
pub fn view(s: &State, f: &mut Frame, t: &ThreadSummary) -> El {
    let k = f.k;
    let now = now_ms();
    let mut header = row()
        .key("talk-thread-header")
        .w_full()
        .gap(8.0)
        .center_y()
        .child(icon(&icons::MESSAGE_TEXT, 16.0).color(k.c.text_muted));
    let r = logic::thread_ref(&t.tag, &t.short);
    if !r.is_empty() {
        header = header.child(text(r).font(ts(12.0, 400.0)).color(k.c.text_muted));
    }
    header = header.child(
        text(plain_line(&t.title))
            .font(ts(15.0, 600.0))
            .color(k.c.text_base)
            .max_lines(1)
            .grow(1.0)
            .min_w(0.0),
    );
    if t.private {
        header = header.child(badge(
            f,
            "talk-thread-private",
            "Private",
            crate::logic::cards::Tone::Slate,
            false,
            "Only you and your own agents see this thread; it never reaches other members' devices",
        ));
    }
    if let Some((label, tone)) = logic::state_badge(&t.state) {
        header = header.child(badge(f, "talk-thread-state", label, tone, false, ""));
    }
    let at = logic::at_ms(&t.updated_at);
    header = header.child(text(relative_time(at, now)).font(ts(11.0, 400.0)).color(k.c.text_muted));

    let mut pane = col()
        .key("talk-thread")
        .w_full()
        .h_full()
        .min_h(0.0)
        .padding(16.0)
        .gap(12.0)
        .child(header);
    if !t.cards.is_empty() {
        pane = pane.child(thread_cards(f, t));
    }
    pane.child(thread_view(s, f, t).grow(1.0).min_h(0.0))
}

/// The cards made or changed from a thread (card #155), each one click
/// from its page.
fn thread_cards(f: &Frame, t: &ThreadSummary) -> El {
    let k = f.k;
    let mut r = row()
        .key("talk-thread-cards")
        .w_full()
        .gap(4.0)
        .center_y()
        .flex_wrap()
        .child(text("Cards:").font(ts(11.0, 400.0)).color(k.c.text_muted));
    for c in &t.cards {
        let (bg, fg, border) = crate::widgets::bits::tone_colors(f, logic::card_state_tone(&c.state));
        let id = c.id.clone();
        r = r.child(
            row()
                .key(format!("talk-tc-{}", c.id))
                .role("button")
                .focusable(true)
                .h(18.0)
                .px(4.0)
                .gap(4.0)
                .center_y()
                .radius(5.0)
                .min_w(0.0)
                .max_w(360.0)
                .fill(bg)
                .border(1.0, border)
                .help(format!("{} ({})", plain_line(&c.title), c.state.replacen('_', " ", 1)))
                .on_press(move |s: &mut State| s.open_card(&id))
                .child(text(logic::thread_ref(&c.tag, &c.short)).font(ts(10.5, 500.0).mono()).color(fg))
                .child(text(plain_line(&c.title)).font(ts(10.5, 500.0)).color(fg).max_lines(1).shrink()),
        );
    }
    r
}

/// Whether a thread entry shows: events stay out, but a card made from the
/// thread is part of the answer.
fn shown(it: &CardThreadItem) -> bool {
    it.kind != "event" || it.made
}

/// The open thread: what the person asked, the answers, the reply box and
/// Resolve. Only its answers scroll (card #564).
fn thread_view(s: &State, f: &mut Frame, t: &ThreadSummary) -> El {
    let k = f.k;
    let resolved = t.state == THREAD_RESOLVED;
    let detail = s.talk.thread.as_ref().filter(|d| d.summary.id == t.id);
    let reading = detail.is_none() && s.talk.thread_error.is_none();
    // A made card's line names it by its ref, never by its id.
    let items: Vec<CardThreadItem> = detail
        .map(|d| {
            d.thread
                .iter()
                .filter(|it| shown(it))
                .map(|it| {
                    let mut it = it.clone();
                    if it.kind == "event" {
                        it.text = event_text(s, &it.text);
                    }
                    it
                })
                .collect()
        })
        .unwrap_or_default();

    let mut top = col().key("talk-thread-top").w_full().gap(4.0);
    if let Some(o) = &t.opened_by {
        top = top.child(
            row()
                .key("talk-thread-opened")
                .gap(4.0)
                .center_y()
                .child(face(s, f, "talk-thread-opened-f", o, FaceOpts { inline: true, ..FaceOpts::default() }))
                .child(text(logic::opened_line(o)).font(ts(11.0, 400.0)).color(k.c.text_muted).max_lines(1)),
        );
    }
    top = top.child(
        col()
            .key("talk-thread-question")
            .w_full()
            .max_h(220.0)
            .min_h(0.0)
            .scroll(ScrollSpec { stick_bottom: false })
            .child(crate::text::rich_with(s, f, "talk-thread-q", &t.text, &RichOpts {
                font: Some(ts(12.5, 400.0)),
                ..RichOpts::default()
            })),
    );
    if !t.told.is_empty() {
        let mut told = row()
            .key("talk-thread-told")
            .w_full()
            .gap(4.0)
            .center_y()
            .flex_wrap()
            .child(
                text(if t.state == THREAD_WAITING { "Asked first:" } else { "Told:" })
                    .font(ts(11.0, 400.0))
                    .color(k.c.text_muted),
            );
        for w in &t.told {
            let key = format!("talk-told-{}", w.key);
            told = told.child(
                row()
                    .key(key.clone())
                    .h(18.0)
                    .px(4.0)
                    .gap(4.0)
                    .center_y()
                    .radius(5.0)
                    .fill(k.c.layer_02)
                    .border(1.0, k.c.border_muted)
                    .child(face(s, f, &format!("{key}-f"), w, FaceOpts { inline: true, ..FaceOpts::default() }))
                    .child(text(display_name(w)).font(ts(10.5, 500.0)).color(k.c.text_muted)),
            );
        }
        top = top.child(told);
    }

    // The answers, newest in view, with the reply box under them: the
    // card package's thread (cards/thread.tsx) in its compact, scrolling
    // form.
    let mut agents: Vec<AgentRef> = Vec::new();
    agents.extend(t.opened_by.iter().cloned());
    agents.extend(t.told.iter().cloned());
    agents.extend(t.answered_by.iter().cloned());
    let empty = if reading {
        Some(crate::widgets::loading::message_rows_shape(f, "talk-thread-loading", 2))
    } else {
        None
    };
    let composer = (!resolved).then(|| crate::card::reply_box(s, f, &t.id));
    let opts = ThreadOpts {
        card_id: t.id.clone(),
        items: &items,
        empty_text: Some(logic::empty_line(t)),
        empty,
        compact: true,
        composer,
        scroll: true,
        on: Some(On {
            agents: &agents,
            over: resolved,
            working: None,
            jobs: None,
        }),
        ..ThreadOpts::default()
    };
    let answers = crate::card::thread_view(s, f, "talk/thread", &opts);

    let mut out = col()
        .key("talk-thread-view")
        .w_full()
        .min_h(0.0)
        .gap(8.0)
        .child(top)
        .child(answers.grow(1.0).min_h(0.0));
    if let Some(e) = &s.talk.thread_error {
        out = out.child(text(format!("Could not read the thread: {e}")).font(ts(12.0, 400.0)).color(status::failed()).w_full().wrap());
    }
    // A waiting thread past its device's usage limit: start one anyway
    // (cards #450, #576).
    if t.state == THREAD_WAITING
        && let Some(b) = crate::card::start_past_limit(s, f, "talk-thread-past-limit", &t.id, &spawn_state(t), true)
    {
        out = out.child(b);
    }
    let mut actions = row().key("talk-thread-actions").w_full().gap(8.0).center_y();
    let id = t.id.clone();
    actions = actions.child(if resolved {
        Button::new("Reopen")
            .variant(ButtonVariant::Outline)
            .control_size(ControlSize::Small)
            .disabled(s.talk.thread_busy)
            .help("Bring it back: your next reply goes to the agents that answered")
            .on_press(move |s: &mut State| {
                let id = id.clone();
                act(s, move |w| w.reopen_thread(&id), |_| {});
            })
            .build(k)
    } else {
        Button::new("Resolve")
            .variant(ButtonVariant::Outline)
            .control_size(ControlSize::Small)
            .disabled(s.talk.thread_busy)
            .help("Done with it: the thread closes")
            .on_press(move |s: &mut State| {
                let id = id.clone();
                let done = id.clone();
                act(s, move |w| w.resolve_thread(&id), move |s| {
                    let open = super::open_id(s).to_string();
                    super::open(s, &logic::open_after_resolve(&open, &done));
                });
            })
            .build(k)
    });
    if !resolved {
        let id = t.id.clone();
        actions = actions.child(crate::widgets::agents_now::agents_now_toggle(
            f,
            "talk-thread-now",
            t.agents_now,
            s.talk.thread_busy,
            move |s: &mut State, on| {
                let id = id.clone();
                act(s, move |w| w.set_agents_now(&id, on), |_| {});
            },
        ));
    }
    if !s.talk.thread_act_error.is_empty() {
        actions = actions.child(text(s.talk.thread_act_error.clone()).font(ts(12.0, 400.0)).color(status::failed()).max_lines(2).shrink());
    }
    out.child(actions)
}

/// Resolve, Reopen or the toggle: one at a time, its error beside them.
fn act(
    s: &mut State,
    work: impl FnOnce(&irori_window::Window) -> Result<(), String> + Send + 'static,
    done: impl FnOnce(&mut State) + Send + 'static,
) {
    if s.talk.thread_busy {
        return;
    }
    s.talk.thread_busy = true;
    s.talk.thread_act_error.clear();
    s.spawn(work, move |s, r| {
        s.talk.thread_busy = false;
        match r {
            Ok(()) => {
                // The list's state comes back on board:changed; read it now
                // so the pane doesn't wait for it.
                s.spawn(|w| w.threads_all(), |s, ts| s.talk.threads = ts);
                done(s);
            }
            Err(e) => s.talk.thread_act_error = e,
        }
    });
}

/// An event's words with each `task:<id>` as the card's ref ("#12"), so
/// it reads as a ref chip and never as an id.
fn event_text(s: &State, t: &str) -> String {
    t.split(' ')
        .map(|w| match w.strip_prefix("task:") {
            Some(id) => {
                let id = id.trim_end_matches(|c: char| !c.is_ascii_alphanumeric());
                let found = s.data.card(id).map(crate::logic::card_status::card_ref).or_else(|| {
                    s.talk.threads.iter().flat_map(|t| &t.cards).find(|c| c.id == id).map(|c| logic::thread_ref(&c.tag, &c.short))
                });
                found.filter(|r| r != "new" && !r.is_empty()).unwrap_or_else(|| "a card".into())
            }
            None => w.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a thread says about starting its agent, as the card package's
/// StartPastLimit reads it.
fn spawn_state(t: &ThreadSummary) -> SpawnState {
    SpawnState {
        starting: t.starting.clone(),
        at_cap: t.at_cap.clone(),
        queue_place: t.queue_place,
        usage_limit: t.usage_limit.filter(|u| !u.is_zero()),
        cant_start: t.cant_start.clone(),
        no_agent: t.no_agent.clone(),
    }
}

/// "Start a thread": ask the room about anything (the React app's
/// AskBox on the Talk page).
pub fn start_view(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let board = crate::logic::spaces::threads_space(&super::picked_space(s)).to_string();
    let c = ask_box(s);
    let kinds = logic::mention_kinds(c.mentions.iter().map(|m| m.key.as_str()));
    let now_on = s.talk.ask_now.unwrap_or(logic::agents_now_for(&kinds));
    let b = board.clone();
    let on_send: crate::text::SendFn = Rc::new(move |s: &mut State, m: Sent| {
        if m.text.trim().is_empty() {
            return;
        }
        let kinds = logic::mention_kinds(m.mentions.iter().map(|x| x.key.as_str()));
        let now = s.talk.ask_now.unwrap_or(logic::agents_now_for(&kinds));
        let ms = crate::text::mentions::to_ts(&m.mentions);
        let board = b.clone();
        ask_box_mut(s).sending = true;
        s.talk.ask_error.clear();
        s.spawn(
            move |w| w.start_thread_with(&m.text, &ms, &m.files, &board, Some(now)),
            |s, r| match r {
                Ok(r) => {
                    crate::text::sent(s, ask_box_mut);
                    ask_box_mut(s).missed(&r.mentioned);
                    s.talk.ask_now = None;
                    let id = r.task.id.clone();
                    s.spawn(|w| w.threads_all(), move |s, ts| {
                        s.talk.threads = ts;
                        super::open(s, &id);
                    });
                }
                Err(e) => {
                    ask_box_mut(s).sending = false;
                    s.talk.ask_error = e;
                }
            },
        );
    });
    let no_mentions = (!board.is_empty())
        .then(|| "A shared space's thread takes no mentions: name a card instead".to_string());
    let mut extra = vec![crate::widgets::agents_now::agents_now_toggle(
        f,
        "talk-ask-now",
        now_on,
        c.sending,
        |s: &mut State, on| s.talk.ask_now = Some(on),
    )];
    extra.push(super::conv::send_button(f, "talk-ask-send", c, &board, no_mentions.clone(), on_send.clone(), ask_box_mut));
    let opts = ComposerOpts {
        placeholder: "Ask your agents…".into(),
        label: "Ask the room".into(),
        asking: true,
        board: board.clone(),
        popup_below: true,
        rows: Some((2, 10)),
        mic: Some(crate::call::mic_button_in(
            s,
            f,
            "talk-ask",
            |s: &mut State| &mut s.talk.ask_box.edit,
            c.sending,
        )),
        no_mentions,
        no_files: (!board.is_empty()).then(|| "Files on a shared space's thread aren't supported yet".to_string()),
        extra,
        on_send: Some(on_send),
        ..ComposerOpts::default()
    };
    let mut out = col()
        .key("talk-start")
        .w_full()
        .h_full()
        .padding(16.0)
        .gap(12.0)
        .child(text("Start a thread").font(ts(13.0, 500.0)).color(k.c.text_base))
        .child(
            text("Ask the room about anything. An agent on the cards you name answers, or an idle one does. It stays in Threads until you resolve it.")
                .font(ts(12.5, 400.0))
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
        )
        .child(crate::text::composer(s, f, "talk-ask", ask_box, ask_box_mut, &opts));
    if !s.talk.ask_error.is_empty() {
        out = out.child(text(s.talk.ask_error.clone()).font(ts(12.0, 400.0)).color(status::failed()).w_full().wrap());
    }
    out.child(el().grow(1.0))
}
