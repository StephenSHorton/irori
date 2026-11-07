//! A shared space's Threads panel beside its Board (threads-panel.tsx,
//! card #116), with the thread pieces it shows from talk/threads.tsx
//! (stateBadges, stateLine, ThreadCards, resolvedShown, resolvedToggle,
//! openAfterResolve): open threads on top (the one opened unfolded),
//! resolved ones folded under "Resolved".
//!
//! The open thread here is the nearest version of Talk's ThreadView: the
//! question, who was told, the newest answer, Resolve or Reopen and
//! Agents answer now. The full conversation with its reply box is the
//! thread's own page (Open the thread), until the talk package's
//! ThreadView can be called here.

use std::collections::{HashMap, HashSet};

use irori_window::cards::AgentRef;
use irori_window::threads::{ThreadCard, ThreadSummary};
use rune::{El, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use crate::logic::agent_status::{ms, now_ms};
use crate::logic::cards::Tone;
use crate::logic::names::display_name;
use crate::logic::plain::{plain_line, relative_time};
use crate::logic::spawn_line::{SpawnState, spawn_line};
use crate::ui::theme::status;
use crate::ui::{Frame, State, icons};
use crate::widgets::agents_now::agents_now_toggle;
use crate::widgets::bits::badge;
use crate::widgets::faces::{FaceOpts, face};

#[derive(Default)]
pub struct Model {
    /// Each space's threads as last read, by space id.
    pub lists: HashMap<String, Vec<ThreadSummary>>,
    pending: HashSet<String>,
    /// The thread unfolded ("" none), and whether the person hid the panel.
    pub open: String,
    pub hidden: bool,
    /// The person unfolded Resolved.
    pub show_resolved: bool,
    /// Resolve, Reopen or Agents answer now in flight, and its error.
    pub busy: bool,
    pub error: Option<String>,
}

impl Model {
    pub fn list(&self, board: &str) -> &[ThreadSummary] {
        self.lists.get(board).map_or(&[], Vec::as_slice)
    }
    pub fn loaded(&self, board: &str) -> bool {
        self.lists.contains_key(board) || self.pending.contains(board)
    }
}

/// Reads a space's threads off the UI thread.
pub fn load(s: &mut State, board: &str) {
    s.board.threads.pending.insert(board.to_string());
    let (a, b) = (board.to_string(), board.to_string());
    s.spawn(
        move |w| w.threads(&a),
        move |s, list| {
            s.board.threads.pending.remove(&b);
            s.board.threads.lists.insert(b, list);
        },
    );
}

/// Reads every space's threads again (threads are cards: a board change).
pub fn reload_all(s: &mut State) {
    let boards: Vec<String> = s.board.threads.lists.keys().cloned().collect();
    for b in boards {
        load(s, &b);
    }
}

/// Whether "Resolved" is unfolded: the person's pick, or a resolved
/// thread open (a #ref to one unfolds it, card #144).
pub fn resolved_shown(pick: bool, open: &str, threads: &[ThreadSummary]) -> bool {
    pick || threads.iter().any(|t| t.id == open && t.state == "resolved")
}

/// A click on "Resolved" (card #194): the pick it sets, and whether to
/// close the open thread (folding while a resolved one is open).
pub fn resolved_toggle(pick: bool, open: &str, threads: &[ThreadSummary]) -> (bool, bool) {
    if !resolved_shown(pick, open, threads) {
        return (true, false);
    }
    (false, threads.iter().any(|t| t.id == open && t.state == "resolved"))
}

/// The thread to keep open once `id` is resolved (cards #194, #200).
pub fn open_after_resolve(open: &str, id: &str) -> String {
    if open == id { String::new() } else { open.to_string() }
}

/// A thread's state on its title line.
pub fn state_badge(state: &str) -> Option<(&'static str, Tone)> {
    match state {
        "waiting" => Some(("Waiting", Tone::Warning)),
        "handed" => Some(("Answering", Tone::Info)),
        "answered" => Some(("Answered", Tone::Success)),
        "resolved" => Some(("Resolved", Tone::Slate)),
        _ => None,
    }
}

/// Where a thread stands, under its title.
pub fn state_line(t: &ThreadSummary, why: &str) -> String {
    if t.state == "answered" && !t.no_agent.is_empty() {
        return t.no_agent.clone();
    }
    match t.state.as_str() {
        "answered" => match &t.last_by {
            Some(by) if !t.last_text.is_empty() => format!("{}: {}", display_name(by), plain_line(&t.last_text)),
            _ => "Answered".into(),
        },
        "handed" => match t.told.last() {
            Some(w) => format!("{} is answering", display_name(w)),
            None => "An agent is answering".into(),
        },
        "resolved" => "Resolved".into(),
        _ => {
            let sp = SpawnState {
                starting: t.starting.clone(),
                at_cap: t.at_cap.clone(),
                queue_place: t.queue_place,
                usage_limit: t.usage_limit.filter(|u| !u.is_zero()),
                cant_start: t.cant_start.clone(),
                no_agent: t.no_agent.clone(),
            };
            spawn_line(&sp).map(|l| l.text).unwrap_or_else(|| if why.is_empty() { "Waiting for an agent".into() } else { why.to_string() })
        }
    }
}

fn card_tone(state: &str) -> Tone {
    match state {
        "working" => Tone::Info,
        "input_required" | "blocked" | "in_review" => Tone::Warning,
        "completed" => Tone::Success,
        _ => Tone::Slate,
    }
}

fn thread_ref(tag: &str, short: &str) -> String {
    format!("{tag}#{short}")
}

/// The cards made or changed from a thread (card #155), each one click
/// from its page.
pub fn thread_cards(f: &Frame, key: &str, cards: &[ThreadCard]) -> El {
    let k = f.k;
    let mut r = row()
        .key(key.to_string())
        .w_full()
        .gap(4.0)
        .center_y()
        .flex_wrap()
        .child(text("Cards:").font(k.ty.text_xsmall).color(k.c.text_muted));
    for c in cards {
        let id = c.id.clone();
        let label = format!("{} {}", thread_ref(&c.tag, &c.short), plain_line(&c.title));
        r = r.child(
            row()
                .key(format!("{key}/{}", c.id))
                .role("button")
                .label(label.clone())
                .max_w(320.0)
                .min_w(0.0)
                .cursor(rune::Cursor::Pointer)
                .help(format!("{} ({})", plain_line(&c.title), c.state.replace('_', " ")))
                .on_press(move |s: &mut State| s.open_card(&id))
                .child(badge(f, &format!("{key}/{}/b", c.id), &label, card_tone(&c.state), false, "")),
        );
    }
    r
}

fn set_busy(s: &mut State, what: &'static str, work: impl FnOnce(&irori_window::Window) -> Result<(), String> + Send + 'static, after: impl FnOnce(&mut State) + Send + 'static) {
    if s.board.threads.busy {
        return;
    }
    s.board.threads.busy = true;
    s.board.threads.error = None;
    let _ = what;
    s.spawn(work, move |s, r| {
        s.board.threads.busy = false;
        match r {
            Ok(()) => after(s),
            Err(e) => s.board.threads.error = Some(e),
        }
    });
}

/// The panel: its title and Hide, the open threads, then Resolved.
pub fn panel(s: &State, f: &mut Frame, board: &str, threads: &[ThreadSummary]) -> El {
    let k = f.k;
    let m = &s.board.threads;
    let shown = resolved_shown(m.show_resolved, &m.open, threads);
    let live: Vec<&ThreadSummary> = threads.iter().filter(|t| t.state != "resolved").collect();
    let resolved: Vec<&ThreadSummary> = threads.iter().filter(|t| t.state == "resolved").collect();
    // A board page passes no why (board-page.tsx): the plain waiting line.
    let why = String::new();
    let mut c = col()
        .key("board-threads")
        .role("complementary")
        .label("Threads")
        .w_full()
        .min_w(0.0)
        .gap(12.0)
        .padding(12.0)
        .radius(12.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .child(
            row()
                .key("board-threads-head")
                .w_full()
                .center_y()
                .child(
                    text(if live.is_empty() { "THREADS".to_string() } else { format!("THREADS · {} OPEN", live.len()) })
                        .font(rune::tok::TextStyle::new(11.0, 500.0).tracking(0.6))
                        .color(k.c.text_muted),
                )
                .child(el().grow(1.0))
                .child(
                    row()
                        .key("board-threads-hide")
                        .role("button")
                        .label("Hide threads")
                        .help("Hide threads")
                        .size(22.0, 22.0)
                        .center()
                        .radius(4.0)
                        .hover(k.c.hover)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|s: &mut State| s.board.threads.hidden = true)
                        .child(icon(&icons::X, 14.0).color(k.c.text_muted)),
                ),
        );
    if live.is_empty() {
        c = c.child(super::muted(
            f,
            "board-threads-none",
            "No open threads. Ask about anything in the box above: an agent on the cards you name answers, or an idle one does.",
        ));
    }
    for t in &live {
        c = c.child(thread_row(s, f, t, &why));
    }
    if !resolved.is_empty() {
        let all: Vec<ThreadSummary> = threads.to_vec();
        let mut fold = col()
            .key("board-threads-resolved")
            .w_full()
            .gap(8.0)
            .pt(12.0)
            .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_muted)
            .child(
                row()
                    .key("board-threads-resolved-toggle")
                    .role("button")
                    .label(format!("Resolved ({})", resolved.len()))
                    .checked(shown)
                    .gap(4.0)
                    .center_y()
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |s: &mut State| {
                        let (pick, close) = resolved_toggle(s.board.threads.show_resolved, &s.board.threads.open, &all);
                        s.board.threads.show_resolved = pick;
                        if close {
                            s.board.threads.open.clear();
                        }
                    })
                    .child(icon(if shown { &icons::CHEVRON_DOWN } else { &icons::CHEVRON_RIGHT }, 14.0).color(k.c.text_muted))
                    .child(text(format!("Resolved ({})", resolved.len())).font(k.ty.text_xsmall).color(k.c.text_muted)),
            );
        if shown {
            for t in &resolved {
                fold = fold.child(thread_row(s, f, t, &why));
            }
        }
        c = c.child(fold);
    }
    let _ = board;
    c
}

fn told_chip(s: &State, f: &mut Frame, key: &str, w: &AgentRef) -> El {
    let k = f.k;
    let fc = face(
        s,
        f,
        &format!("{key}/face"),
        w,
        FaceOpts {
            inline: true,
            ..FaceOpts::default()
        },
    );
    row()
        .key(key.to_string())
        .h(16.0)
        .px(4.0)
        .gap(4.0)
        .center_y()
        .radius(4.0)
        .fill(k.c.layer_03)
        .child(fc)
        .child(text(display_name(w)).font(rune::tok::TextStyle::new(10.5, 500.0)).color(k.c.text_base))
}

fn thread_row(s: &State, f: &mut Frame, t: &ThreadSummary, why: &str) -> El {
    let k = f.k;
    let p = format!("board-thread-{}", t.id);
    let open = s.board.threads.open == t.id;
    let waiting = t.state == "waiting" || t.state == "handed";
    let id = t.id.clone();
    let mut head = row().key(format!("{p}/title")).w_full().gap(8.0).center_y();
    if !t.short.is_empty() {
        head = head.child(text(thread_ref(&t.tag, &t.short)).font(rune::tok::TextStyle::new(11.0, 400.0)).color(k.c.text_muted));
    }
    head = head.child(
        text(plain_line(&t.title))
            .font(rune::tok::TextStyle::new(13.0, 500.0))
            .color(k.c.text_base)
            .max_lines(1)
            .grow(1.0)
            .min_w(0.0),
    );
    if t.private {
        head = head.child(badge(
            f,
            &format!("{p}/private"),
            "Private",
            Tone::Slate,
            false,
            "Only you and your own agents see this thread; it never reaches other members' devices",
        ));
    }
    if let Some((label, tone)) = state_badge(&t.state) {
        head = head.child(badge(f, &format!("{p}/state"), label, tone, false, ""));
    }
    head = head.child(text(relative_time(ms(&t.updated_at).unwrap_or(0), now_ms())).font(rune::tok::TextStyle::new(11.0, 400.0)).color(k.c.text_muted));
    let mut btn = col()
        .key(format!("{p}/btn"))
        .role("button")
        .label(plain_line(&t.title))
        .checked(open)
        .focusable(true)
        .w_full()
        .gap(2.0)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |s: &mut State| {
            s.board.threads.open = if s.board.threads.open == id { String::new() } else { id.clone() };
            s.board.threads.error = None;
        })
        .child(head);
    if !open {
        btn = btn.child(
            text(state_line(t, why))
                .font(k.ty.text_xsmall)
                .color(if waiting { status::decide() } else { k.c.text_muted })
                .w_full()
                .max_lines(2),
        );
    }
    let mut sec = col()
        .key(p.clone())
        .w_full()
        .min_w(0.0)
        .gap(if open { 8.0 } else { 4.0 })
        .px(12.0)
        .py(10.0)
        .radius(10.0)
        .fill(k.c.bg_base.alpha(0.4))
        .border(1.0, if open { k.c.icon_accent } else { k.c.border_muted })
        .child(btn);
    if !t.cards.is_empty() {
        sec = sec.child(thread_cards(f, &format!("{p}/cards"), &t.cards));
    }
    if open {
        sec = sec.child(thread_view(s, f, &p, t));
    }
    sec
}

/// The open thread: who opened it, the question, who was told, the newest
/// answer, then Resolve or Reopen and Agents answer now.
fn thread_view(s: &State, f: &mut Frame, p: &str, t: &ThreadSummary) -> El {
    let k = f.k;
    let resolved = t.state == "resolved";
    let busy = s.board.threads.busy;
    let mut c = col().key(format!("{p}/view")).w_full().min_w(0.0).gap(8.0);
    if let Some(by) = &t.opened_by {
        c = c.child(
            text(format!("{} opened this thread", display_name(by)))
                .font(k.ty.text_xsmall)
                .color(k.c.text_muted),
        );
    }
    c = c.child(
        col()
            .key(format!("{p}/q"))
            .w_full()
            .max_h(200.0)
            .min_h(0.0)
            .scroll(rune::ScrollSpec { stick_bottom: false })
            .child(crate::text::rich_text(s, f, &format!("{p}/qt"), &t.text)),
    );
    if !t.told.is_empty() {
        let mut r = row()
            .key(format!("{p}/told"))
            .w_full()
            .gap(4.0)
            .center_y()
            .flex_wrap()
            .child(text(if t.state == "waiting" { "Asked first:" } else { "Told:" }).font(k.ty.text_xsmall).color(k.c.text_muted));
        for (i, w) in t.told.iter().enumerate() {
            r = r.child(told_chip(s, f, &format!("{p}/told/{i}"), w));
        }
        c = c.child(r);
    }
    match &t.last_by {
        Some(by) if !t.last_text.is_empty() => {
            c = c.child(
                col()
                    .key(format!("{p}/last"))
                    .w_full()
                    .gap(4.0)
                    .padding(8.0)
                    .radius(8.0)
                    .fill(k.c.layer_02)
                    .child(
                        text(format!("{} · {} {}", display_name(by), t.replies, if t.replies == 1 { "reply" } else { "replies" }))
                            .font(k.ty.text_xsmall_medium)
                            .color(k.c.text_muted),
                    )
                    .child(crate::text::rich_text(s, f, &format!("{p}/lastt"), &t.last_text)),
            );
        }
        _ => {
            c = c.child(
                text(if resolved { "No answer." } else { "No answer yet." })
                    .font(k.ty.text_xsmall)
                    .color(k.c.text_muted),
            );
        }
    }
    let id = t.id.clone();
    let mut acts = row().key(format!("{p}/acts")).w_full().gap(8.0).center_y().flex_wrap();
    let open_id = t.id.clone();
    acts = acts.child(
        Button::new("Open the thread")
            .variant(ButtonVariant::Outline)
            .control_size(ControlSize::Small)
            .help("The whole conversation, with the reply box")
            .on_press(move |s: &mut State| s.open_card(&open_id))
            .build(k),
    );
    if resolved {
        acts = acts.child(
            Button::new("Reopen")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .disabled(busy)
                .help("Bring it back: your next reply goes to the agents that answered")
                .on_press(move |s: &mut State| {
                    let id = id.clone();
                    set_busy(s, "Reopen", move |w| w.reopen_thread(&id), |_| {});
                })
                .build(k),
        );
    } else {
        let id2 = id.clone();
        acts = acts.child(
            Button::new("Resolve")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .disabled(busy)
                .help("Done with it: the thread closes")
                .on_press(move |s: &mut State| {
                    let id = id.clone();
                    let gone = id.clone();
                    set_busy(s, "Resolve", move |w| w.resolve_thread(&id), move |s| {
                        s.board.threads.open = open_after_resolve(&s.board.threads.open, &gone);
                    });
                })
                .build(k),
        );
        acts = acts.child(agents_now_toggle(f, &format!("{p}/now"), t.agents_now, busy, move |s: &mut State, on| {
            let id = id2.clone();
            set_busy(s, "Agents answer now", move |w| w.set_agents_now(&id, on), |_| {});
        }));
    }
    if let Some(e) = &s.board.threads.error {
        acts = acts.child(text(e.clone()).font(k.ty.text_xsmall).color(status::failed()));
    }
    c.child(acts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str, state: &str) -> ThreadSummary {
        ThreadSummary {
            id: id.into(),
            state: state.into(),
            ..ThreadSummary::default()
        }
    }

    #[test]
    fn resolved_unfolds_for_the_pick_or_an_open_resolved_thread() {
        let ts = [t("a", "answered"), t("r", "resolved")];
        assert!(!resolved_shown(false, "", &ts));
        assert!(resolved_shown(true, "", &ts));
        assert!(resolved_shown(false, "r", &ts));
        assert!(!resolved_shown(false, "a", &ts));
        assert_eq!(resolved_toggle(false, "", &ts), (true, false));
        assert_eq!(resolved_toggle(true, "a", &ts), (false, false));
        assert_eq!(resolved_toggle(false, "r", &ts), (false, true));
        assert_eq!(open_after_resolve("a", "a"), "");
        assert_eq!(open_after_resolve("b", "a"), "b");
    }

    #[test]
    fn state_line_says_where_a_thread_stands() {
        let mut a = t("a", "answered");
        assert_eq!(state_line(&a, ""), "Answered");
        a.last_by = Some(AgentRef {
            name: "oak".into(),
            ..AgentRef::default()
        });
        a.last_text = "done it".into();
        assert_eq!(state_line(&a, ""), "oak: done it");
        a.no_agent = "No computer can start one".into();
        assert_eq!(state_line(&a, ""), "No computer can start one");
        assert_eq!(state_line(&t("h", "handed"), ""), "An agent is answering");
        assert_eq!(state_line(&t("r", "resolved"), ""), "Resolved");
        assert_eq!(state_line(&t("w", "waiting"), ""), "Waiting for an agent");
        assert_eq!(state_line(&t("w", "waiting"), "all busy"), "all busy");
    }
}
