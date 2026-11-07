//! The Notifications page (components/notifications-page.tsx): the list,
//! each item's frame, and what it can still do or how it ended. Its
//! answer rows are a local port of needs-you's BlockerAnswer, OwnWords and
//! Said (the home package owns those rows; these can use them once they
//! land).

use irori_window::cards::{CardBlocker, CardDetail, CardPermission};
use irori_window::notifications::{Notification, NotificationCard};
use irori_window::rune_inbox::{Delivery, GIVEN_QUESTION};
use std::rc::Rc;

use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit};

use super::notify::{self, Filter, Tone, kind};
use super::words::{self, plain_line};
use super::{Act, PAGE_SIZE};
use crate::text::{self as txt, Composer, ComposerOpts, SendFn, Sent};
use crate::ui::{Dialog, Frame, Route, State};

// ---- shared bits (the dialogs use them too) --------------------------------

/// A button: `primary` is shadcn's default, else outline, ghost or link.
pub(super) fn button(
    k: &Kit,
    key: impl Into<String>,
    label: impl Into<String>,
    variant: ButtonVariant,
    disabled: bool,
    on: impl Fn(&mut State) + 'static,
) -> El {
    Button::new(label)
        .variant(variant)
        .disabled(disabled)
        .on_press(on)
        .build(k)
        .key(key.into())
}

/// A small button (shadcn's xs).
pub(super) fn small_button(
    k: &Kit,
    key: impl Into<String>,
    label: impl Into<String>,
    variant: ButtonVariant,
    disabled: bool,
    on: impl Fn(&mut State) + 'static,
) -> El {
    Button::new(label)
        .variant(variant)
        .control_size(ControlSize::Small)
        .disabled(disabled)
        .on_press(on)
        .build(k)
        .key(key.into())
}

/// A tinted badge in a tone (the React Badge's success, destructive,
/// warning, info and secondary; rune-kit's Badge has only neutral,
/// accent and subtle).
pub fn tone_badge(k: &Kit, key: impl Into<String>, label: &str, tone: Tone) -> El {
    let (bg, fg) = match tone {
        Tone::Success => (k.c.success.bg, k.c.success.fg),
        Tone::Destructive => (k.c.danger.bg, k.c.danger.fg),
        Tone::Warning => (k.c.warning.bg, k.c.warning.fg),
        Tone::Info => (k.c.info.bg, k.c.info.fg),
        Tone::Secondary => (k.c.layer_03, k.c.text_muted),
    };
    row()
        .key(key.into())
        .role("status")
        .label(label.to_string())
        .h(18.0)
        .px(6.0)
        .radius(4.0)
        .center()
        .fill(bg)
        .child(text(label.to_string()).font(k.ty.small_medium).color(fg))
}

/// A row of buttons that wraps.
pub(super) fn buttons(key: impl Into<String>, list: Vec<El>) -> El {
    row().key(key.into()).w_full().gap(6.0).flex_wrap().children(list)
}

/// An outcome line, or an error in red.
pub(super) fn line(k: &Kit, key: impl Into<String>, s: &str, error: bool) -> El {
    text(s.to_string())
        .key(key.into())
        .font(k.ty.small)
        .color(if error { k.c.danger.fg } else { k.c.text_muted })
        .w_full()
        .wrap()
}

/// Said: a row's outcome while the list catches up, or its error.
fn said(k: &Kit, key: &str, a: Option<&Act>) -> Option<El> {
    let a = a?;
    if let Some(e) = &a.error {
        return Some(line(k, format!("inbox-said-{key}"), e, true));
    }
    (!a.done.is_empty()).then(|| line(k, format!("inbox-said-{key}"), &a.done, false))
}

/// The card's ref as the page shows it: "wind#12", "#12".
fn card_ref(c: &NotificationCard) -> String {
    format!("{}#{}", c.tag, c.short)
}

// ---- actions ------------------------------------------------------------------

/// Opening an item (any click inside it) marks it read; Unread keeps it.
fn opened(s: &mut State, key: &str) {
    let Some(n) = s.inbox.list.iter_mut().find(|n| n.key == key) else { return };
    if n.read {
        return;
    }
    n.read = true;
    s.inbox.kept.insert(key.to_string());
    let keys = vec![key.to_string()];
    s.spawn(move |w| w.read_notifications(&keys), |_, _| {});
}

/// Runs one row action at a time, keeping its outcome or its error.
fn run(
    s: &mut State,
    key: &str,
    done: &'static str,
    work: impl FnOnce(&irori_window::Window) -> Result<(), String> + Send + 'static,
) {
    let a = s.inbox.acts.entry(key.to_string()).or_default();
    if a.busy {
        return;
    }
    *a = Act {
        busy: true,
        ..Act::default()
    };
    let key = key.to_string();
    s.spawn(work, move |s, r| {
        let a = s.inbox.acts.entry(key).or_default();
        a.busy = false;
        match r {
            Ok(()) => a.done = done.to_string(),
            Err(e) => a.error = Some(e),
        }
    });
}

fn set_filter(s: &mut State, f: Filter) {
    s.inbox.filter = f;
    s.inbox.kept.clear();
    s.inbox.limit = PAGE_SIZE;
    super::load_live(s);
}

// ---- the page ------------------------------------------------------------------

pub fn view(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let m = &s.inbox;
    let unread = notify::unread_count(&m.list);
    let unread_total = s.data.unread_notifications as usize;
    let title = if unread > 0 {
        format!("Notifications · {unread} unread")
    } else {
        "Notifications".to_string()
    };
    let tab = |fl: Filter, label: &'static str| {
        let on = m.filter == fl;
        Button::new(label)
            .variant(if on { ButtonVariant::Neutral } else { ButtonVariant::Ghost })
            .on_press(move |s: &mut State| set_filter(s, fl))
            .build(k)
            .key(format!("inbox-tab-{label}"))
            .role("tab")
            .checked(on)
    };
    let mark_all = button(
        k,
        "inbox-mark-all",
        "Mark all read",
        ButtonVariant::Ghost,
        unread == 0 && unread_total == 0,
        |s: &mut State| {
            for n in &mut s.inbox.list {
                if !n.read {
                    n.read = true;
                    s.inbox.kept.insert(n.key.clone());
                }
            }
            s.spawn(|w| w.read_notifications(&[]), |s, r| {
                if let Err(e) = r {
                    s.error("Mark all read", &e);
                }
            });
        },
    );
    let header = crate::ui::page_header(
        f,
        &title,
        vec![row()
            .key("inbox-tabs")
            .role("tablist")
            .label("Which notifications")
            .gap(4.0)
            .center_y()
            .child(tab(Filter::Unread, "Unread"))
            .child(tab(Filter::All, "All"))
            .child(mark_all)],
    );

    let now = words::now_ms();
    let items = super::items(m);
    let mut list = col().key("inbox-list").w_full().max_w(760.0).gap(10.0);
    if let Some(e) = &m.error {
        list = list.child(line(k, "inbox-error", &format!("Could not load the notifications: {e}"), true));
    }
    if !m.loaded {
        list = list.child(
            text("Loading the notifications…")
                .key("inbox-loading")
                .font(k.ty.ui)
                .color(k.c.text_muted),
        );
    } else if items.is_empty() {
        let empty = if m.filter == Filter::Unread && !m.list.is_empty() {
            "You've read them all. All shows every one."
        } else {
            "Nothing yet. Every notification Irori shows you is kept here."
        };
        list = list.child(text(empty).key("inbox-empty").font(k.ty.ui).color(k.c.text_muted).w_full().wrap());
    } else {
        for n in items.iter().take(m.limit) {
            list = list.child(item(s, f, n, now));
        }
        if items.len() > m.limit {
            let more = PAGE_SIZE.min(items.len() - m.limit);
            list = list.child(
                row().w_full().child(button(
                    k,
                    "inbox-more",
                    format!("Show {more} more"),
                    ButtonVariant::Ghost,
                    false,
                    |s: &mut State| {
                        s.inbox.limit += PAGE_SIZE;
                        super::load_live(s);
                    },
                )),
            );
        }
    }
    col()
        .key("inbox-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(header)
        .child(
            col()
                .key("inbox-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .px(20.0)
                .pb(24.0)
                .child(list)
                .scroll(ScrollSpec { stick_bottom: false }),
        )
}

/// One notification: its frame, then what it can still do or how it ended.
fn item(s: &State, f: &mut Frame, n: &Notification, now: i64) -> El {
    let k = f.k;
    let key = n.key.clone();
    let mut head = row().w_full().gap(8.0).center_y().flex_wrap();
    if !n.read {
        head = head.child(el().key(format!("inbox-dot-{key}")).size(8.0, 8.0).radius(4.0).fill(k.c.bg_accent).help("Unread"));
    }
    head = head.child(tone_badge(k, format!("inbox-kind-{key}"), notify::kind_label(&n.kind), notify::kind_tone(&n.kind)));
    if let Some(c) = &n.card {
        let label = if c.short.is_empty() { plain_line(&c.title) } else { card_ref(c) };
        let (id, nk) = (c.id.clone(), key.clone());
        head = head.child(
            text(label)
                .key(format!("inbox-ref-{key}"))
                .role("button")
                .label(format!("Open {}", plain_line(&c.title)))
                .focusable(true)
                .font(k.ty.small.mono())
                .color(k.c.text_muted)
                .help(plain_line(&c.title))
                .max_lines(1)
                .shrink()
                .on_press(move |s: &mut State| {
                    opened(s, &nk);
                    s.open_card(&id);
                }),
        );
    }
    head = head.child(el().grow(1.0)).child(
        text(words::ago(&n.at, now))
            .key(format!("inbox-at-{key}"))
            .font(k.ty.small)
            .color(k.c.text_muted)
            .help(n.at.format_date_time()),
    );
    let title_font = rune::tok::TextStyle::new(15.0, if n.read { 500.0 } else { 600.0 });
    let mut body = col()
        .key(format!("inbox-item-{key}"))
        .role("article")
        .label(format!("{}{}", if n.read { "" } else { "Unread: " }, plain_line(&n.title)))
        .w_full()
        .gap(8.0)
        .px(16.0)
        .py(12.0)
        .radius(10.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .opacity(if n.read { 0.8 } else { 1.0 })
        .on_press({
            let key = key.clone();
            move |s: &mut State| opened(s, &key)
        })
        .child(head)
        .child(text(plain_line(&n.title)).font(title_font).color(k.c.text_base).w_full().wrap());
    if !n.body.is_empty() && n.kind != kind::DONE_CHECK && !notify::asks_on_card(n) {
        body = body.child(
            text(words::plain_block(&n.body))
                .font(k.ty.ui)
                .color(k.c.text_muted)
                .w_full()
                .wrap()
                .max_lines(3),
        );
    }
    for e in live(s, f, n, now) {
        body = body.child(e);
    }
    body
}

/// Outcome: how the thing ended, and when.
fn outcome(k: &Kit, key: &str, line_: &str, when: Option<&irori_window::rune_inbox::GoTime>, now: i64) -> El {
    let mut t = line_.to_string();
    if let Some(w) = when.filter(|w| !w.is_zero()) {
        t.push_str(&format!(" · {}", words::ago(w, now)));
    }
    text(t)
        .key(format!("inbox-outcome-{key}"))
        .role("status")
        .font(k.ty.ui)
        .color(k.c.text_muted)
        .w_full()
        .wrap()
}

fn open_card_button(k: &Kit, key: &str, card: &str) -> El {
    let (nk, id) = (key.to_string(), card.to_string());
    buttons(
        format!("inbox-open-{key}"),
        vec![button(k, format!("inbox-open-card-{key}"), "Open card", ButtonVariant::Outline, false, move |s: &mut State| {
            opened(s, &nk);
            s.open_card(&id);
        })],
    )
}

/// What the item can still do, or how it ended (Live).
fn live(s: &State, f: &mut Frame, n: &Notification, now: i64) -> Vec<El> {
    let k = f.k;
    let key = n.key.as_str();
    let card_id = n.card.as_ref().map_or(n.task_id.as_str(), |c| c.id.as_str());
    if notify::asks_on_card(n) {
        return card_live(s, f, n, now);
    }
    if n.kind == kind::ASK
        && let Some(a) = &n.ask
    {
        let ask = n.key.strip_prefix("ask:").unwrap_or("").to_string();
        if let Some(choice) = s.inbox.picked.get(key) {
            return vec![outcome(k, key, &format!("Answered “{choice}”"), None, now)];
        }
        if !a.open {
            return vec![outcome(k, key, &notify::ask_outcome(a), None, now)];
        }
        let mut list = Vec::new();
        for (i, l) in a.actions.iter().enumerate() {
            let (nk, ask, l2) = (key.to_string(), ask.clone(), l.clone());
            list.push(button(
                k,
                format!("inbox-ask-{key}-{i}"),
                l.clone(),
                if i == 0 { ButtonVariant::Submit } else { ButtonVariant::Outline },
                ask.is_empty(),
                move |s: &mut State| pick_ask(s, &nk, &ask, &l2),
            ));
        }
        return vec![buttons(format!("inbox-asks-{key}"), list).role("group").label("Answers")];
    }
    // Several messages that waited: they're DMs, so Talk's list (card #482).
    if notify::waited_summary(n) {
        let nk = key.to_string();
        return vec![buttons(
            format!("inbox-waited-{key}"),
            vec![button(k, format!("inbox-msgs-{key}"), "Open messages", ButtonVariant::Outline, false, move |s: &mut State| {
                opened(s, &nk);
                s.go(Route::Talk(None));
            })],
        )];
    }
    if (n.kind == kind::MESSAGE || n.kind == kind::HELD) && !n.delivery_id.is_empty() {
        return delivery_live(s, f, n, now);
    }
    if !n.task_id.is_empty() {
        return vec![if n.card.is_some() {
            open_card_button(k, key, card_id)
        } else {
            outcome(k, key, "The card is gone", None, now)
        }];
    }
    Vec::new()
}

fn pick_ask(s: &mut State, key: &str, ask: &str, choice: &str) {
    opened(s, key);
    s.inbox.picked.insert(key.to_string(), choice.to_string());
    let (a, c, k2) = (ask.to_string(), choice.to_string(), key.to_string());
    s.spawn(
        move |w| w.answer_notification_ask(&a, &c),
        move |s, r| {
            if let Err(e) = r {
                s.inbox.picked.remove(&k2);
                s.error("Your answer wasn't sent", &e);
            }
        },
    );
}

/// A card's blocker, done check or question as it stands now (CardLive).
fn card_live(s: &State, f: &mut Frame, n: &Notification, now: i64) -> Vec<El> {
    let k = f.k;
    let key = n.key.as_str();
    let Some(card) = &n.card else {
        return vec![outcome(k, key, "The card is gone", None, now)];
    };
    let detail: &CardDetail = match s.inbox.cards.get(&card.id) {
        None => {
            return vec![line(k, format!("inbox-loading-{key}"), "Loading the answers…", false)];
        }
        Some(Err(_)) => return vec![open_card_button(k, key, &card.id)],
        Some(Ok(d)) => d,
    };
    let b = (!n.blocker_id.is_empty())
        .then(|| detail.blockers.iter().find(|b| b.id == n.blocker_id))
        .flatten();
    if let Some(b) = b {
        // A permission prompt is its buttons (card #682).
        if notify::blocker_open(b)
            && !s.inbox.picked.contains_key(key)
            && let Some(p) = &b.permission
        {
            return vec![
                text(plain_line(&b.question)).font(k.ty.ui).color(k.c.text_base).w_full().wrap(),
                permission_prompt(s, f, key, &card.id, b, p),
            ];
        }
        if notify::blocker_open(b) && !s.inbox.picked.contains_key(key) {
            return blocker_answer(s, f, key, &card.id, b);
        }
        let mut out = Vec::new();
        if let Some(said) = s.inbox.picked.get(key) {
            out.push(line(k, format!("inbox-picked-{key}"), said, false));
            out.push(open_card_button(k, key, &card.id));
            return out;
        }
        if !b.done_check {
            out.push(text(plain_line(&b.question)).font(k.ty.ui).color(k.c.text_base).w_full().wrap());
        }
        out.push(outcome(k, key, &notify::blocker_outcome(b), b.resolved.as_ref().map(|r| &r.at), now));
        out.push(open_card_button(k, key, &card.id));
        return out;
    }
    // An older card's question without an item: open while the card still
    // asks the person.
    let asks = detail.summary.state == "input_required" || detail.summary.state == "blocked";
    let mut out = Vec::new();
    if !n.body.is_empty() {
        out.push(text(plain_line(&n.body)).font(k.ty.ui).color(k.c.text_base).w_full().wrap());
    }
    if !asks {
        out.push(outcome(k, key, "Answered; the card has moved on", Some(&detail.summary.updated_at), now));
    }
    out.push(open_card_button(k, key, &card.id));
    out
}

/// BlockerAnswer with answerOpens: the options answer here, "Answer…"
/// opens the card to answer there (card #644).
fn blocker_answer(s: &State, f: &Frame, key: &str, card: &str, b: &CardBlocker) -> Vec<El> {
    let k = f.k;
    let _ = s;
    let question = plain_line(&b.question);
    let mut out = Vec::new();
    if !b.done_check {
        let mut q = col().w_full().gap(2.0);
        if b.question != GIVEN_QUESTION && !b.raised_by.name.is_empty() {
            q = q.child(
                text(format!("{} asks", plain_line(&b.raised_by.name)))
                    .font(k.ty.small)
                    .color(k.c.text_muted),
            );
        }
        out.push(q.child(text(question.clone()).font(k.ty.ui).color(k.c.text_base).w_full().wrap()));
    }
    if !b.context.is_empty() {
        out.push(text(plain_line(&b.context)).font(k.ty.ui).color(k.c.text_muted).w_full().wrap().max_lines(3));
    }
    let replied = b.state == "replied";
    if replied && !b.missing.is_empty() {
        let need: Vec<String> = b.missing.iter().map(|m| plain_line(m)).collect();
        out.push(text(format!("They still need: {}", need.join("; "))).font(k.ty.ui).color(k.c.text_base).w_full().wrap());
    }
    let mut list = Vec::new();
    if !b.options_aside {
        // Recommended first, each keeping its index.
        let mut order: Vec<(usize, &irori_window::cards::CardOption)> = b.options.iter().enumerate().collect();
        order.sort_by_key(|(_, o)| !o.recommended);
        for (n_, (i, o)) in order.into_iter().enumerate() {
            let mut label = plain_line(&o.answer);
            if o.recommended && !b.done_check {
                label.push_str(" (recommended)");
            }
            let primary = n_ == 0 && (o.recommended || b.done_check);
            let (nk, c, bid, done_check) = (key.to_string(), card.to_string(), b.id.clone(), b.done_check);
            // isDone (lib/optimistic.ts): a done check's "Done" option.
            let is_done = done_check && o.answer == "Done";
            let mut btn = button(
                k,
                format!("inbox-opt-{key}-{i}"),
                label,
                if primary { ButtonVariant::Submit } else { ButtonVariant::Outline },
                false,
                move |s: &mut State| answer(s, &nk, &c, &bid, i as i64, done_check, is_done),
            );
            if !o.why.is_empty() {
                btn = btn.help(plain_line(&o.why));
            }
            list.push(btn);
        }
    }
    let (nk, c) = (key.to_string(), card.to_string());
    if b.question != GIVEN_QUESTION {
        let label = if b.done_check {
            "Not done…"
        } else if replied {
            "Reply…"
        } else {
            "Answer…"
        };
        let (nk, c) = (nk.clone(), c.clone());
        list.push(button(k, format!("inbox-own-{key}"), label, ButtonVariant::Outline, false, move |s: &mut State| {
            opened(s, &nk);
            s.open_card(&c);
        }));
    }
    list.push(button(k, format!("inbox-blocker-open-{key}"), "Open card", ButtonVariant::Ghost, false, move |s: &mut State| {
        opened(s, &nk);
        s.open_card(&c);
    }));
    out.push(buttons(format!("inbox-answers-{key}"), list).role("group").label(format!("Answers to: {question}")));
    out
}

/// A pick shows at once and saves in the background (card #644); a
/// refusal brings the buttons back and a note says why.
fn answer(s: &mut State, key: &str, card: &str, blocker: &str, option: i64, done_check: bool, is_done: bool) {
    opened(s, key);
    let said = if done_check {
        if is_done { "Marked done." } else { "Sent back to the agent." }
    } else {
        "Answer sent."
    };
    s.inbox.picked.insert(key.to_string(), said.to_string());
    let (c, b, k2) = (card.to_string(), blocker.to_string(), key.to_string());
    s.spawn(
        move |w| w.answer(&c, &b, option, "", true, &[]).map(|_| ()),
        move |s, r| {
            if let Err(e) = r {
                s.inbox.picked.remove(&k2);
                let what = if done_check {
                    "Your answer to the done check wasn't saved"
                } else {
                    "Your answer wasn't saved"
                };
                s.error(what, &e);
            }
        },
    );
}

/// "Waiting for your permission" (cards #456, #682; cards/permission-
/// prompt.tsx, compact): the agent Irori started for the card stopped on
/// a permission prompt nobody is attached to. It shows what is waiting
/// (the tool, and the command, path or URL) and buttons instead of an
/// answer box: Allow once and Deny answer the live prompt, Always allow
/// also adds the rule to the board's allow list, and Attach answers at
/// the prompt itself (the only way when Irori can't answer from the card).
fn permission_prompt(s: &State, f: &mut Frame, key: &str, card: &str, b: &CardBlocker, p: &CardPermission) -> El {
    let k = f.k;
    let agent = (!p.agent.key.is_empty()).then_some(&p.agent);
    let name = agent.map_or_else(|| "Its agent".to_string(), crate::logic::names::display_name);
    let tool = plain_line(&p.tool);
    let mut said = if tool.is_empty() {
        format!("{name} is stopped on a permission prompt")
    } else {
        format!("{name} is stopped on a permission prompt to use {tool}")
    };
    said.push_str(if p.input.is_empty() { "." } else { ":" });
    let mut c = col()
        .key(format!("inbox-permission-{key}"))
        .w_full()
        .min_w(0.0)
        .gap(6.0)
        .child(text(said).font(k.ty.ui).color(k.c.text_base).w_full().wrap());
    if !p.input.is_empty() {
        c = c.child(
            col()
                .key(format!("inbox-permission-input-{key}"))
                .label(if tool.is_empty() { "What is waiting".to_string() } else { format!("The {tool} input waiting") })
                .w_full()
                .max_h(96.0)
                .min_h(0.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .px(8.0)
                .py(6.0)
                .radius(6.0)
                .border(1.0, k.c.border_muted)
                .fill(k.c.layer_02)
                .child(
                    text(p.input.clone())
                        .font(k.ty.small.mono())
                        .color(k.c.text_base)
                        .w_full()
                        .wrap()
                        .selectable(true),
                ),
        );
    }
    if p.answerable && !p.expires.is_empty() {
        let at = crate::spaces::logic::clock_time(&p.expires);
        let at = if at.is_empty() { "2 minutes after asking".to_string() } else { at };
        c = c.child(
            text(format!(
                "A safety check Claude Code asks even in bypass mode: it denies the call by itself at {at} unless you answer."
            ))
            .font(k.ty.small)
            .color(k.c.warning.fg)
            .w_full()
            .wrap(),
        );
    }
    if !p.answerable {
        c = c.child(line(
            k,
            format!("inbox-permission-attach-{key}"),
            &format!("Irori can't answer this prompt from the card: attach to {name} to answer it."),
            false,
        ));
    }
    let answers = if p.answerable { notify::permission_answers(b, &p.rule) } else { Vec::new() };
    let mut list = Vec::new();
    for (a, i) in &answers {
        let (a, i) = (*a, *i);
        let variant = match a {
            notify::ALLOW_ONCE => ButtonVariant::Submit,
            notify::DENY => ButtonVariant::Danger,
            _ => ButtonVariant::Outline,
        };
        let said = notify::permission_said(a, &name, &plain_line(&p.rule));
        let (nk, c2, bid) = (key.to_string(), card.to_string(), b.id.clone());
        let mut btn = small_button(k, format!("inbox-permission-{key}-{i}"), a, variant, false, move |s: &mut State| {
            answer_said(s, &nk, &c2, &bid, i as i64, said.clone(), "Your answer to the permission prompt wasn't saved")
        });
        if a == notify::ALWAYS_ALLOW {
            btn = btn.help(format!("Also adds {} to the board's allow list", p.rule));
        } else if let Some(o) = b.options.get(i).filter(|o| !o.why.is_empty()) {
            btn = btn.help(plain_line(&o.why));
        }
        list.push(btn);
    }
    if let Some(w) = agent {
        list.push(crate::widgets::spawned_actions::spawned_actions(
            s,
            f,
            &format!("inbox-permission-acts-{key}"),
            w,
            card,
            &[crate::widgets::spawned_actions::Action::Attach, crate::widgets::spawned_actions::Action::Logs],
        ));
    }
    if !list.is_empty() {
        c = c.child(buttons(format!("inbox-permission-buttons-{key}"), list).role("group").label(format!("Answers to {name}'s permission prompt")));
    }
    if !p.rule.is_empty() && answers.iter().any(|(a, _)| *a == notify::ALWAYS_ALLOW) {
        let rule = plain_line(&p.rule);
        let head = "Always allow also adds ";
        let all = format!("{head}{rule} to the board's allow list (Settings › Agents).");
        c = c.child(
            rune::rich(all, vec![rune::Span::new(head.len(), head.len() + rule.len()).mono()])
                .font(k.ty.small)
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
        );
    }
    c
}

/// A pick that shows `said` at once and saves in the background; a
/// refusal brings the buttons back and a note says `failed`.
fn answer_said(s: &mut State, key: &str, card: &str, blocker: &str, option: i64, said: String, failed: &'static str) {
    opened(s, key);
    s.inbox.picked.insert(key.to_string(), said);
    let (c, b, k2) = (card.to_string(), blocker.to_string(), key.to_string());
    s.spawn(
        move |w| w.answer(&c, &b, option, "", true, &[]).map(|_| ()),
        move |s, r| {
            if let Err(e) = r {
                s.inbox.picked.remove(&k2);
                s.error(failed, &e);
            }
        },
    );
}

/// A message to the person, or one Irori held, as it stands now.
fn delivery_live(s: &State, f: &mut Frame, n: &Notification, now: i64) -> Vec<El> {
    let k = f.k;
    let key = n.key.as_str();
    let held = n.kind == kind::HELD;
    let d: &Delivery = match s.inbox.deliveries.get(&n.delivery_id) {
        None => return Vec::new(),
        Some(Err(_)) => return vec![outcome(k, key, "The message is gone", None, now)],
        Some(Ok(d)) => d,
    };
    let ended = notify::delivery_outcome(d, held);
    if !ended.is_empty() {
        let at = if d.decided_at.is_zero() { &d.updated_at } else { &d.decided_at };
        return vec![outcome(k, key, ended, Some(at), now)];
    }
    let act = s.inbox.acts.get(key);
    if let Some(a) = act
        && !a.done.is_empty()
    {
        return vec![line(k, format!("inbox-said-{key}"), &a.done, false)];
    }
    let busy = act.is_some_and(|a| a.busy);
    let id = d.id.clone();
    let mut out = Vec::new();
    if held {
        let (k1, i1, k2, i2) = (key.to_string(), id.clone(), key.to_string(), id.clone());
        out.push(buttons(
            format!("inbox-held-{key}"),
            vec![
                button(k, format!("inbox-release-{key}"), "Release", ButtonVariant::Submit, busy, move |s: &mut State| {
                    opened(s, &k1);
                    let i = i1.clone();
                    run(s, &k1, "Released.", move |w| w.inbox_release(&i));
                }),
                button(k, format!("inbox-dismiss-{key}"), "Dismiss", ButtonVariant::Outline, busy, move |s: &mut State| {
                    opened(s, &k2);
                    let i = i2.clone();
                    run(s, &k2, "Dismissed.", move |w| w.inbox_dismiss(&i));
                }),
            ],
        ));
    } else {
        let from = {
            let p = plain_line(&d.message.from.name);
            if p.is_empty() { "the agent".to_string() } else { p }
        };
        if s.inbox.replying.as_deref() == Some(key) {
            out.push(own_words(s, f, key, &id, &from, busy));
        } else {
            let (k1, k2, i2, k3, i3) = (key.to_string(), key.to_string(), id.clone(), key.to_string(), id.clone());
            out.push(buttons(
                format!("inbox-msg-{key}"),
                vec![
                    button(k, format!("inbox-reply-{key}"), "Reply…", ButtonVariant::Submit, busy, move |s: &mut State| {
                        opened(s, &k1);
                        s.inbox.replying = Some(k1.clone());
                        s.inbox.reply = super::focused_box();
                    }),
                    button(k, format!("inbox-read-{key}"), "Mark read", ButtonVariant::Outline, busy, move |s: &mut State| {
                        opened(s, &k2);
                        let i = i2.clone();
                        run(s, &k2, "Marked read.", move |w| w.inbox_mark_read(&i));
                    }),
                    button(k, format!("inbox-open-msg-{key}"), "Open", ButtonVariant::Ghost, false, move |s: &mut State| {
                        opened(s, &k3);
                        s.open_dialog(Dialog::Message { id: i3.clone() });
                    }),
                ],
            ));
        }
    }
    if let Some(e) = said(k, key, act) {
        out.push(e);
    }
    out
}

/// OwnWords: the fold-out reply box (the shared composer: mentions,
/// files and the mic), with Reply and Cancel.
fn own_words(s: &State, f: &mut Frame, key: &str, id: &str, from: &str, busy: bool) -> El {
    let k = f.k;
    let on_send: SendFn = {
        let (key, id) = (key.to_string(), id.to_string());
        Rc::new(move |s: &mut State, m: Sent| {
            if m.text.trim().is_empty() && m.files.is_empty() {
                return;
            }
            let i = id.clone();
            let ms: Vec<irori_window::ts::Mention> = m.mentions.iter().map(|x| x.to_ts()).collect();
            // The box stays open, its words kept, while the reply goes
            // ("Sending…"); a failure leaves them there with the error.
            let a = s.inbox.acts.entry(key.clone()).or_default();
            if a.busy {
                return;
            }
            *a = Act {
                busy: true,
                ..Act::default()
            };
            s.inbox.reply.sending = true;
            let k2 = key.clone();
            s.spawn(
                move |w| w.inbox_reply_files(&i, &m.text, &ms, &m.files).map(|_| ()),
                move |s, r| reply_done(s, &k2, r),
            );
        })
    };
    let o = ComposerOpts {
        placeholder: format!("Reply to {from}"),
        label: format!("Your reply to {from}"),
        on_send: Some(on_send),
        rows: Some((1, 6)),
        popup_below: true,
        disabled: busy,
        // Esc cancels the box, as OwnWords' onKeyDown did (an open picker
        // takes it first).
        on_escape: Some(Rc::new(cancel_reply)),
        mic: Some(crate::call::mic_button(s, f, &format!("inbox-reply-{key}"), |s: &mut State| {
            &mut s.inbox.reply.edit
        })),
        ..ComposerOpts::default()
    };
    let field = txt::composer(s, f, &format!("inbox-reply-box-{key}"), reply_box, reply_box_mut, &o);
    let send = {
        let o = o.clone();
        move |s: &mut State| txt::submit(s, reply_box_mut, &o)
    };
    row()
        .key(format!("inbox-own-words-{key}"))
        .w_full()
        .gap(6.0)
        .items_start()
        .flex_wrap()
        .child(col().grow(1.0).min_w(256.0).child(field))
        .child(button(
            k,
            format!("inbox-send-{key}"),
            if busy { "Sending…" } else { "Reply" },
            ButtonVariant::Submit,
            busy || s.inbox.reply.is_empty(),
            send,
        ))
        .child(button(k, format!("inbox-cancel-{key}"), "Cancel", ButtonVariant::Ghost, busy, cancel_reply))
}

/// A reply's answer: sent closes the box and says so; a failure keeps the
/// box open with what was typed, and says why under it.
fn reply_done(s: &mut State, key: &str, r: Result<(), String>) {
    let a = s.inbox.acts.entry(key.to_string()).or_default();
    a.busy = false;
    s.inbox.reply.sending = false;
    match r {
        Ok(()) => {
            a.done = "Replied.".to_string();
            if s.inbox.replying.as_deref() == Some(key) {
                s.inbox.replying = None;
            }
            s.inbox.reply.sent();
        }
        Err(e) => a.error = Some(e),
    }
}

/// Closes the open Reply box (its Cancel, and Esc in it).
fn cancel_reply(s: &mut State) {
    s.inbox.replying = None;
}

fn reply_box(s: &State) -> &Composer {
    &s.inbox.reply
}

fn reply_box_mut(s: &mut State) -> &mut Composer {
    &mut s.inbox.reply
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_reply_keeps_the_box_and_its_words() {
        let mut s = State::default();
        s.inbox.replying = Some("msg:1".into());
        s.inbox.reply.edit = rune::TextEdit::new("on my way");
        s.inbox.reply.sending = true;
        s.inbox.acts.entry("msg:1".into()).or_default().busy = true;
        reply_done(&mut s, "msg:1", Err("inbox: no such delivery or message".into()));
        assert_eq!(s.inbox.replying.as_deref(), Some("msg:1"), "the box stays open");
        assert_eq!(s.inbox.reply.edit.text, "on my way", "with what was typed");
        assert!(!s.inbox.reply.sending);
        let a = &s.inbox.acts["msg:1"];
        assert!(!a.busy);
        assert_eq!(a.error.as_deref(), Some("inbox: no such delivery or message"));
        // A second try that goes through closes it and says so.
        reply_done(&mut s, "msg:1", Ok(()));
        assert!(s.inbox.replying.is_none());
        assert!(s.inbox.reply.edit.text.is_empty());
        assert_eq!(s.inbox.acts["msg:1"].done, "Replied.");
    }

    #[test]
    fn cancel_closes_the_reply_box() {
        let mut s = State::default();
        s.inbox.replying = Some("msg:1".into());
        cancel_reply(&mut s);
        assert!(s.inbox.replying.is_none());
    }
}
