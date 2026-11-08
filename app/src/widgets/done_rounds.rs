//! A done check the person sent back (the React app's
//! cards/done-rounds.tsx, card #476): while the agent works on it, the
//! check says whether it heard and answered them; the next check opens
//! with that exchange above its new proof, so a re-raised check never
//! reads like a first ask.
//!
//! The person's and the agent's words are drawn as plain wrapped text
//! (`plain_block`); swap in the text package's rich text when it lands.

use irori_window::cards::CardBlocker;
use rune::{El, col, icon, row, text};

use super::bits::badge;
use crate::logic::cards::Tone;
use crate::logic::names::display_name;
use crate::logic::plain::{plain_block, plain_line};
use crate::logic::rounds::{SentBack, sent_back};
use crate::ui::theme::status;
use crate::ui::{Frame, State, icons};

/// The heard / answered pair under the person's Not done.
pub fn round_signs(f: &Frame, key: &str, r: &SentBack, agent: &str) -> El {
    let k = f.k;
    let sign = |i: usize, ok: bool, yes: String, no: &str| {
        let c = if ok {
            status::working()
        } else {
            k.c.text_muted
        };
        row()
            .key(format!("{key}/sign{i}"))
            .gap(4.0)
            .center_y()
            .child(if ok {
                icon(&icons::CHECK, 12.0).color(c)
            } else {
                text("○")
                    .font(rune::tok::TextStyle::new(11.5, 400.0))
                    .color(c)
            })
            .child(
                text(if ok { yes } else { no.to_string() })
                    .font(rune::tok::TextStyle::new(11.5, 400.0))
                    .color(c),
            )
    };
    row()
        .key(key.to_string())
        .role("status")
        .label("What the agent did with your reason")
        .gap(12.0)
        .flex_wrap()
        .child(sign(0, r.heard, format!("{agent} read it"), "Not read yet"))
        .child(sign(
            1,
            r.answered,
            format!("{agent} answered"),
            "No answer yet",
        ))
}

fn agent_of(r: &SentBack, fallback: String) -> String {
    r.replies
        .last()
        .map(|it| display_name(&it.who))
        .unwrap_or(fallback)
}

/// On a done check the person sent back (open, waiting on the agent):
/// what the agent did with their reason.
pub fn sent_back_signs(f: &Frame, key: &str, b: &CardBlocker) -> Option<El> {
    if !b.done_check || b.resolved.is_some() {
        return None;
    }
    let r = sent_back(&b.thread, "")?;
    let fallback = display_name(b.sent_to.as_ref().unwrap_or(&b.raised_by));
    let agent = agent_of(&r, fallback);
    Some(round_signs(f, key, &r, &agent))
}

fn said(f: &Frame, key: &str, who: String, t: &str, compact: bool) -> El {
    let k = f.k;
    let body = if compact {
        text(plain_line(t))
            .font(k.ty.text_small)
            .color(k.c.text_base)
            .w_full()
            .max_lines(2)
    } else {
        text(plain_block(t))
            .font(k.ty.text_small)
            .color(k.c.text_base)
            .w_full()
            .wrap()
            .selectable(true)
    };
    col()
        .key(key.to_string())
        .w_full()
        .gap(2.0)
        .child(
            text(who)
                .font(rune::tok::TextStyle::new(11.5, 500.0))
                .color(k.c.text_muted),
        )
        .child(body)
}

/// Opens a re-raised done check: the round the person sent back.
/// `compact` (Needs you) keeps it to a few lines.
pub fn earlier_round(
    s: &State,
    f: &Frame,
    key: &str,
    b: &CardBlocker,
    compact: bool,
) -> Option<El> {
    let k = f.k;
    let p = b.previous.as_ref()?;
    let r = sent_back(&p.thread, &p.answer)?;
    let raiser = display_name(&b.raised_by);
    let who = agent_of(&r, raiser);
    let mut sec = col()
        .key(key.to_string())
        .role("group")
        .label(format!(
            "Round {}: what you said and the agent's answer",
            p.round
        ))
        .w_full()
        .gap(6.0)
        .px(12.0)
        .py(8.0)
        .radius(8.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .child(
            row()
                .key(format!("{key}/head"))
                .gap(6.0)
                .center_y()
                .child(badge(
                    f,
                    &format!("{key}/round"),
                    &format!("Round {}", p.round),
                    Tone::Slate,
                    false,
                    "",
                ))
                .child(
                    text("You sent the last check back")
                        .font(rune::tok::TextStyle::new(11.5, 400.0))
                        .color(k.c.text_muted),
                ),
        )
        .child(said(
            f,
            &format!("{key}/reason"),
            "You said".into(),
            &r.reason,
            compact,
        ));
    if r.replies.is_empty() {
        sec = sec.child(
            text(format!(
                "{who} didn't answer your reason before asking again."
            ))
            .font(k.ty.text_small)
            .color(status::decide())
            .w_full()
            .wrap(),
        );
    } else {
        let shown = if compact {
            &r.replies[r.replies.len() - 1..]
        } else {
            &r.replies[..]
        };
        for it in shown {
            sec = sec.child(said(
                f,
                &format!("{key}/reply-{}", it.id),
                format!("{} answered", display_name(&it.who)),
                &it.text,
                compact,
            ));
        }
    }
    if !compact {
        sec = sec.child(round_signs(f, &format!("{key}/signs"), &r, &who));
    }
    if p.same_proof {
        sec = sec.child(
            text("The proof below is the same as the one you sent back.")
                .font(k.ty.text_small_medium)
                .color(status::decide())
                .w_full()
                .wrap(),
        );
    } else if !compact && !p.proof.is_empty() {
        let open = s.widgets.round_proof.contains(key);
        let kk = key.to_string();
        sec = sec.child(
            text(if open {
                "Hide the proof you sent back"
            } else {
                "The proof you sent back"
            })
            .font(rune::tok::TextStyle::new(11.5, 400.0))
            .color(k.c.text_muted)
            .role("button")
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut State| {
                if !s.widgets.round_proof.remove(&kk) {
                    s.widgets.round_proof.insert(kk.clone());
                }
            }),
        );
        if open {
            sec = sec.child(
                text(plain_block(&p.proof))
                    .font(rune::tok::TextStyle::new(12.0, 400.0))
                    .color(k.c.text_muted)
                    .w_full()
                    .wrap()
                    .selectable(true),
            );
        }
    }
    Some(sec)
}
