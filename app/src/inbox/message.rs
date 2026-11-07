//! One message a notification's Open names (cards/message-dialog.tsx;
//! there is no inbox in the window): a message Irori held, with Release
//! and Dismiss, or one an agent sent the person, in full, with Reply,
//! Stop and read, and Mark read. When a reply's mention wasn't delivered,
//! the dialog stays open to say so.

use irori_window::Window;
use irori_window::mentions::MentionOutcome;
use irori_window::rune_inbox::{mode, st};
use rune::{ScrollSpec, col, el, row, text};
use rune_kit::{ButtonVariant, Dialog as KitDialog, TextArea};

use super::notify::{self, Tone};
use super::page::{button, line, tone_badge};
use super::words::{self, plain_line};
use super::names;
use crate::ui::{Frame, State};

/// Runs `work`; closes, unless its answer says a mention wasn't delivered:
/// then the reply has gone, and the dialog says who missed it.
fn act(s: &mut State, work: impl FnOnce(&Window) -> Result<Vec<MentionOutcome>, String> + Send + 'static) {
    let m = &mut s.inbox.message;
    if m.busy {
        return;
    }
    m.busy = true;
    m.error = None;
    s.spawn(work, |s, r| {
        let m = &mut s.inbox.message;
        m.busy = false;
        match r {
            Ok(outcomes) => {
                let missed = words::not_delivered(&outcomes);
                if missed.is_empty() {
                    s.close_dialog();
                } else {
                    m.reply.clear();
                    m.missed = format!("Replied. {missed}");
                }
            }
            Err(e) => m.error = Some(e),
        }
    });
}

fn reply(s: &mut State, stop: bool) {
    let text = s.inbox.message.reply.text.clone();
    if text.trim().is_empty() {
        s.inbox.message.error = Some("Write a reply first.".into());
        return;
    }
    let id = s.inbox.message.id.clone();
    if stop {
        // Stop and read (card #207): the sender's running turn is cut and
        // the reply is its next prompt.
        act(s, move |w| w.inbox_reply_mode(&id, &text, &[], &[], mode::STOP));
    } else {
        act(s, move |w| w.inbox_reply(&id, &text, &[]));
    }
}

pub fn dialog(s: &State, f: &mut Frame, id: &str) {
    let k = f.k;
    let m = &s.inbox.message;
    let names = names(s);
    let repos = &s.inbox.repos;
    let d = match &m.d {
        Some(Ok(d)) if m.id == id => Some(d),
        _ => None,
    };
    let load_error = match &m.d {
        Some(Err(e)) if m.id == id => Some(e.clone()),
        _ => None,
    };
    let now = words::now_ms();
    let Some(d) = d else {
        let mut body = col().key("inbox-msg-body").w_full().px(16.0).pb(16.0);
        if let Some(e) = &load_error {
            body = body.child(line(k, "inbox-msg-error", e, true));
        }
        KitDialog::new("inbox-msg", if load_error.is_some() { "Couldn't open the message" } else { "Opening the message…" })
            .open(true)
            .width(512.0)
            .fit()
            .child(body)
            .on_close(|s: &mut State| s.close_dialog())
            .build(k, f.layer);
        return;
    };
    let from = {
        let p = plain_line(&d.message.from.name);
        if p.is_empty() { names.name(&d.message.from.key) } else { p }
    };
    let to = if d.to.starts_with("user:") {
        "you".to_string()
    } else if !d.to.is_empty() {
        names.name(&d.to)
    } else {
        let r = plain_line(&repos.name(&d.repo_key));
        if d.repo_key.is_empty() || r.is_empty() { "a repo".into() } else { r }
    };
    let held = d.state == st::HELD;
    let for_you = d.to.starts_with("user:") && notify::message_open(d);
    let title = if held {
        format!("{from} sent {to} a message Irori held")
    } else {
        format!("{from} sent {to} a message")
    };
    let mut desc = String::new();
    if held {
        desc.push_str(&plain_line(&words::hold_text(d, &names, repos)));
        desc.push(' ');
    }
    desc.push_str(&words::ago(&d.message.sent_at, now));
    if !held && !for_you {
        desc.push_str(&format!(" · {}", d.state));
    }

    // The kit Dialog's description doesn't wrap: a hold's reason can be long.
    let mut body = col()
        .key("inbox-msg-body")
        .w_full()
        .px(16.0)
        .pb(16.0)
        .gap(10.0)
        .mt(-8.0)
        .child(text(desc).key("inbox-msg-desc").font(k.ty.ui).color(k.c.text_muted).w_full().wrap());
    // An agent's message whose recipient had ended came to the person
    // instead; a reply goes to its sender.
    if d.for_.starts_with("recipient gone") {
        body = body.child(
            row()
                .key("inbox-msg-gone")
                .w_full()
                .gap(6.0)
                .center_y()
                .child(tone_badge(k, "inbox-msg-gone-badge", "Recipient gone", Tone::Warning))
                .child(text(plain_line(&d.for_)).font(k.ty.small).color(k.c.text_muted).shrink().wrap()),
        );
    }
    body = body.child(
        col()
            .key("inbox-msg-text")
            .w_full()
            .max_h(360.0)
            .min_h(0.0)
            .radius(8.0)
            .border(1.0, k.c.border_muted)
            .fill(k.c.bg_base)
            .child(
                text(words::plain_block(&d.message.text))
                    .font(k.ty.body)
                    .color(k.c.text_base)
                    .w_full()
                    .wrap()
                    .selectable(true)
                    .px(12.0)
                    .py(10.0),
            )
            .scroll(ScrollSpec { stick_bottom: false }),
    );
    if for_you && m.missed.is_empty() {
        // TODO(text::composer): the shared composer (mentions, files,
        // voice) replaces this plain box once it lands.
        body = body.child(
            TextArea::new(&m.reply)
                .placeholder(format!("Reply to {from}"))
                .label(format!("Your reply to {from}"))
                .auto_grow(2, 8)
                .disabled(m.busy)
                .on_edit(|s: &mut State| &mut s.inbox.message.reply)
                .on_submit(|s: &mut State, _| reply(s, false))
                .build(k)
                .key("inbox-msg-reply"),
        );
    }
    if !m.missed.is_empty() {
        body = body.child(line(k, "inbox-msg-missed", &m.missed, true));
    }
    if let Some(e) = &m.error {
        body = body.child(line(k, "inbox-msg-error", e, true));
    }
    if (held || for_you) && m.missed.is_empty() {
        let mut foot = row().key("inbox-msg-foot").w_full().gap(8.0).center_y().child(el().grow(1.0));
        if held {
            let (i1, i2) = (m.id.clone(), m.id.clone());
            foot = foot
                .child(button(k, "inbox-msg-dismiss", "Dismiss", ButtonVariant::Outline, m.busy, move |s: &mut State| {
                    let i = i1.clone();
                    act(s, move |w| w.inbox_dismiss(&i).map(|_| Vec::new()));
                }))
                .child(button(k, "inbox-msg-release", "Release", ButtonVariant::Submit, m.busy, move |s: &mut State| {
                    let i = i2.clone();
                    act(s, move |w| w.inbox_release(&i).map(|_| Vec::new()));
                }));
        } else {
            let i1 = m.id.clone();
            foot = foot.child(button(k, "inbox-msg-read", "Mark read", ButtonVariant::Outline, m.busy, move |s: &mut State| {
                let i = i1.clone();
                act(s, move |w| w.inbox_mark_read(&i).map(|_| Vec::new()));
            }));
            if !d.message.from.key.starts_with("user:") {
                foot = foot.child(
                    button(k, "inbox-msg-stop", "Stop and read", ButtonVariant::Outline, m.busy, |s: &mut State| reply(s, true))
                        .help("Cut the sender's running turn: it reads this first and answers you before anything else"),
                );
            }
            foot = foot.child(button(
                k,
                "inbox-msg-send",
                if m.busy { "Sending…" } else { "Reply" },
                ButtonVariant::Submit,
                m.busy,
                |s: &mut State| reply(s, false),
            ));
        }
        body = body.child(foot);
    }
    KitDialog::new("inbox-msg", title)
        .open(true)
        .width(512.0)
        .fit()
        .child(body)
        .on_close(|s: &mut State| s.close_dialog())
        .build(k, f.layer);
}
