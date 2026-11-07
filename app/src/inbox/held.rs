//! The Held list (cards/held-dialog.tsx): every message Irori is holding
//! back until the person decides, each saying why (card #488), with
//! Release, Dismiss and Allow sender & deliver, or Start an agent for
//! their own message to a repo nobody is in; and its History, the ones
//! already decided, where a dismissed one can still be released. `from`
//! shows only that sender's (a held notification's Open).

use irori_window::Window;
use irori_window::rune_inbox::{Delivery, SYSTEM_KEY, hold, st};
use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::{Button, ButtonVariant, Dialog as KitDialog};

use super::notify::Tone;
use super::page::{buttons, line, small_button, tone_badge};
use super::words::{self, Names, Repos, plain_line};
use super::{HeldTab, names};
use crate::ui::{Dialog, Frame, State};

fn sender(d: &Delivery, names: &Names) -> String {
    let n = plain_line(&d.message.from.name);
    if n.is_empty() { names.name(&d.message.from.key) } else { n }
}

/// Runs an action on one held message, then reads the list again.
fn act(s: &mut State, id: &str, work: impl FnOnce(&Window) -> Result<String, String> + Send + 'static) {
    let h = &mut s.inbox.held;
    if !h.busy.is_empty() {
        return;
    }
    h.busy = id.to_string();
    h.act_error = None;
    h.said.clear();
    s.spawn(work, |s, r| {
        s.inbox.held.busy.clear();
        match r {
            Ok(said) => s.inbox.held.said = said,
            Err(e) => s.inbox.held.act_error = Some(e),
        }
        super::load_held(s);
    });
}

/// "#28" for the card a held message is about, else "a card".
fn card_label(s: &State, r: &str) -> String {
    let id = r.trim_start_matches("task:");
    match s.data.card(id) {
        Some(c) if !c.short.is_empty() => format!("{}#{}", c.tag, c.short),
        _ => "a card".into(),
    }
}

pub fn dialog(s: &State, f: &mut Frame, from: Option<&str>) {
    let k = f.k;
    let h = &s.inbox.held;
    let names = names(s);
    let repos = &s.inbox.repos;
    let from = from.unwrap_or("");
    let shown: Vec<&Delivery> = match h.tab {
        HeldTab::Waiting => &h.list,
        HeldTab::History => &h.history,
    }
    .iter()
    .filter(|d| from.is_empty() || d.message.from.key == from)
    .collect();
    let from_name = if from.is_empty() {
        String::new()
    } else {
        h.list
            .iter()
            .chain(&h.history)
            .find(|d| d.message.from.key == from)
            .map_or_else(|| names.name(from), |d| sender(d, &names))
    };

    let tab = |t: HeldTab, label: &str, count: Option<usize>| {
        let on = h.tab == t;
        let label = match count {
            Some(n) if n > 0 => format!("{label} {n}"),
            _ => label.to_string(),
        };
        Button::new(label.clone())
            .variant(if on { ButtonVariant::Submit } else { ButtonVariant::Ghost })
            .on_press(move |s: &mut State| s.inbox.held.tab = t)
            .build(k)
            .key(format!("inbox-held-tab-{}", if t == HeldTab::Waiting { "waiting" } else { "history" }))
            .checked(on)
    };
    let mut bar = row()
        .key("inbox-held-bar")
        .w_full()
        .gap(4.0)
        .center_y()
        .child(
            row()
                .role("group")
                .label("Held list")
                .gap(4.0)
                .child(tab(HeldTab::Waiting, "Waiting", Some(h.list.len())))
                .child(tab(HeldTab::History, "History", None)),
        )
        .child(el().grow(1.0));
    if !from.is_empty() {
        bar = bar.child(
            Button::new(format!("From {from_name} ×"))
                .variant(ButtonVariant::Outline)
                .help("Show every sender's")
                .on_press(|s: &mut State| s.dialog = Some(Dialog::Held { from: None }))
                .build(k)
                .key("inbox-held-from"),
        );
    }

    let mut body = col().key("inbox-held-body").w_full().px(16.0).pb(16.0).gap(10.0).child(bar);
    if !h.loaded {
        body = body.child(line(k, "inbox-held-loading", "Loading…", false));
    } else if shown.is_empty() {
        let empty = match (h.tab, from.is_empty()) {
            (HeldTab::Waiting, true) => "Nothing is held now.".to_string(),
            (HeldTab::Waiting, false) => format!("Nothing from {from_name} is held now."),
            (HeldTab::History, true) => "No past held messages.".to_string(),
            (HeldTab::History, false) => format!("No past held messages from {from_name}."),
        };
        body = body.child(line(k, "inbox-held-empty", &empty, false));
    } else {
        let now = words::now_ms();
        let mut list = col()
            .key("inbox-held-list")
            .role("list")
            .label(if h.tab == HeldTab::Waiting { "Waiting on you" } else { "Decided" })
            .w_full();
        let last = shown.len() - 1;
        for (i, d) in shown.iter().enumerate() {
            list = list.child(held_row(s, f, d, &names, repos, now, i == last));
        }
        body = body.child(
            col()
                .key("inbox-held-scroll")
                .w_full()
                .max_h(440.0)
                .min_h(0.0)
                .child(list)
                .scroll(ScrollSpec { stick_bottom: false }),
        );
    }
    if !h.said.is_empty() && h.act_error.is_none() {
        body = body.child(line(k, "inbox-held-said", &h.said, false));
    }
    if let Some(e) = h.act_error.as_ref().or(h.error.as_ref()) {
        body = body.child(line(k, "inbox-held-error", e, true));
    }
    KitDialog::new("inbox-held", "Held messages")
        .open(true)
        .description(
            "Messages Irori is holding back until you say, and why. Release delivers one; Dismiss drops it, and History can still release it.",
        )
        .width(672.0)
        .fit()
        .child(body)
        .on_close(|s: &mut State| s.close_dialog())
        .build(k, f.layer);
}

fn held_row(s: &State, f: &Frame, d: &Delivery, names: &Names, repos: &Repos, now: i64, last: bool) -> El {
    let k = f.k;
    let id = d.id.clone();
    let h = &s.inbox.held;
    let busy = h.busy == d.id;
    let disabled = !h.busy.is_empty();
    let held = d.state == st::HELD;
    let from = sender(d, names);
    let to = if d.to.starts_with("user:") {
        "you".to_string()
    } else if !d.to.is_empty() {
        names.name(&d.to)
    } else {
        let r = plain_line(&repos.name(&d.repo_key));
        if d.repo_key.is_empty() || r.is_empty() { "a repo".into() } else { r }
    };
    let unowned = d.hold == hold::UNOWNED && d.to.is_empty();
    let mine = d.message.from.key.starts_with("user:");
    let where_ = if d.repo_key.is_empty() { "its repo".to_string() } else { plain_line(&repos.name(&d.repo_key)) };
    let r = d.message.ref_.clone();

    let mut head = row()
        .w_full()
        .gap(6.0)
        .center_y()
        .flex_wrap()
        .child(text(from.clone()).font(k.ty.ui_medium).color(k.c.text_base))
        .child(text("->").font(k.ty.ui).color(k.c.text_muted).a11y_hidden())
        .child(text(to).font(k.ty.ui_medium).color(k.c.text_base).wrap_anywhere().shrink());
    if r.starts_with("task:") {
        let card = r.trim_start_matches("task:").to_string();
        head = head.child(
            Button::new(card_label(s, &r))
                .variant(ButtonVariant::GhostMuted)
                .control_size(rune_kit::ControlSize::Small)
                .help("Open the card")
                .on_press(move |s: &mut State| {
                    s.close_dialog();
                    s.open_card(&card);
                })
                .build(k)
                .key(format!("inbox-held-card-{id}")),
        );
    }
    head = head.child(el().grow(1.0)).child(
        text(words::ago(&d.message.sent_at, now))
            .font(k.ty.small)
            .color(k.c.text_muted)
            .help(d.message.sent_at.format_date_time()),
    );

    let mut row_ = col()
        .key(format!("inbox-held-row-{id}"))
        .role("listitem")
        .w_full()
        .gap(4.0)
        .py(10.0)
        .child(head)
        .child(text(plain_line(&words::hold_text(d, names, repos))).font(k.ty.small).color(k.c.text_muted).w_full().wrap())
        .child(
            text(words::plain_block(&d.message.text))
                .font(k.ty.ui)
                .color(k.c.text_base)
                .w_full()
                .wrap()
                .max_lines(4),
        );
    if held {
        let mut list = Vec::new();
        if unowned {
            // Card #488: Release can't deliver it (nobody is there); the
            // person's own message can start an agent there instead.
            if mine {
                let i = id.clone();
                list.push(small_button(
                    k,
                    format!("inbox-held-start-{id}"),
                    if busy { "Working…".to_string() } else { format!("Start an agent in {where_}") },
                    ButtonVariant::Submit,
                    disabled,
                    move |s: &mut State| {
                        let i2 = i.clone();
                        act(s, &i, move |w| w.inbox_start_agent(&i2).map(|r| words::started_line(&r)));
                    },
                ));
            }
        } else {
            let i = id.clone();
            list.push(small_button(
                k,
                format!("inbox-held-release-{id}"),
                if busy { "Working…" } else { "Release" },
                ButtonVariant::Submit,
                disabled,
                move |s: &mut State| {
                    let i2 = i.clone();
                    act(s, &i, move |w| w.inbox_release(&i2).map(|_| String::new()));
                },
            ));
        }
        let i = id.clone();
        list.push(small_button(k, format!("inbox-held-dismiss-{id}"), "Dismiss", ButtonVariant::Outline, disabled, move |s: &mut State| {
            let i2 = i.clone();
            act(s, &i, move |w| w.inbox_dismiss(&i2).map(|_| String::new()));
        }));
        if words::allow_lifts(d) {
            let i = id.clone();
            list.push(
                small_button(k, format!("inbox-held-allow-{id}"), "Allow sender & deliver", ButtonVariant::Ghost, disabled, move |s: &mut State| {
                    let i2 = i.clone();
                    act(s, &i, move |w| w.inbox_allow_and_release(&i2).map(|_| String::new()));
                })
                .help(format!("Adds {from} to your allow list, then delivers this")),
            );
        }
        row_ = row_.child(buttons(format!("inbox-held-acts-{id}"), list).pt(2.0));
    } else {
        let at = if d.decided_at.is_zero() { &d.updated_at } else { &d.decided_at };
        // Moved straight out of Held (to another agent): its last row is
        // the decision, so nothing was released.
        let moved_held = d.state == st::MOVED && at.unix_milli_value() == d.updated_at.unix_milli_value();
        let by = if d.decided_by == SYSTEM_KEY {
            "Irori".to_string()
        } else if !d.decided_by.is_empty() {
            names.name(&d.decided_by)
        } else {
            String::new()
        };
        let dismissed = d.state == st::DISMISSED;
        let badge = if dismissed {
            "Dismissed"
        } else if moved_held {
            "Moved"
        } else if d.decided_by == SYSTEM_KEY {
            "Let through"
        } else {
            "Released"
        };
        let mut said = String::new();
        if !by.is_empty() {
            said.push_str(&format!("by {by} "));
        }
        said.push_str(&words::ago(at, now));
        if moved_held {
            if !d.moved_to.is_empty() {
                said.push_str(&format!(" · to {}", names.name(&d.moved_to)));
            }
        } else if !dismissed {
            said.push_str(&format!(" · now {}", d.state));
        }
        let mut r = row()
            .key(format!("inbox-held-past-{id}"))
            .w_full()
            .gap(6.0)
            .center_y()
            .pt(2.0)
            .child(tone_badge(k, format!("inbox-held-badge-{id}"), badge, if dismissed { Tone::Secondary } else { Tone::Success }))
            .child(text(said).font(k.ty.small).color(k.c.text_muted).shrink());
        if dismissed {
            let i = id.clone();
            r = r.child(el().grow(1.0)).child(small_button(
                k,
                format!("inbox-held-rerelease-{id}"),
                if busy { "Working…" } else { "Release" },
                ButtonVariant::Outline,
                disabled,
                move |s: &mut State| {
                    let i2 = i.clone();
                    act(s, &i, move |w| w.inbox_release(&i2).map(|_| String::new()));
                },
            ));
        }
        row_ = row_.child(r);
    }
    if last {
        row_
    } else {
        col().w_full().child(row_).child(el().w_full().h(1.0).fill(k.c.border_muted))
    }
}
