//! A permission item on the card page (card-detail.tsx's BlockerItem
//! `b.permission` branch, cards #456 and #682): the agent Irori started
//! for a card stopped on a permission prompt nobody is attached to. The
//! item wraps the same prompt Needs you shows
//! ([`crate::home::permission::permission_prompt`], full size): what waits
//! and Allow once, Always allow, Deny, Attach and Logs instead of an
//! answer box, since typed words don't reach a prompt.

use irori_window::cards::{CardBlocker, CardDetail};
use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use super::blockers::{BlockerUi, resolved_by, ui_mut};
use super::{err_line, ts};
use crate::home::permission::permission_prompt;
use crate::logic::agent_status::{ms, now_ms};
use crate::logic::plain::relative_time;
use crate::logic::thread::blocker_item_id;
use crate::text::{self as tt, RichOpts};
use crate::ui::{Frame, State};
use crate::widgets::bits;

/// A permission item on the card (card-detail.tsx BlockerItem's
/// `b.permission` branch): who raised it, the question, the prompt and
/// when. No answer box or thread, since words don't reach a prompt.
pub fn item(s: &State, f: &mut Frame, d: &CardDetail, b: &CardBlocker, u: &BlockerUi) -> El {
    let k = f.k;
    let cid = d.summary.id.clone();
    let r = b.resolved.is_some();
    let (fill, border, dim) = if r {
        (k.c.layer_02, k.c.border_base, 0.85)
    } else {
        (k.c.danger.bg.alpha(0.4), k.c.danger.border, 1.0)
    };
    let mut c = col()
        .key(blocker_item_id(&b.id))
        .w_full()
        .min_w(0.0)
        .gap(8.0)
        .padding(12.0)
        .radius(12.0)
        .border(1.0, border)
        .fill(fill)
        .opacity(dim)
        .child(bits::asker(
            f,
            &format!("card/b/{}/asker", b.id),
            &b.raised_by,
        ));
    let mut head = row().w_full().min_w(0.0).gap(6.0).items_start();
    if r {
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
    if r {
        let bid = b.id.clone();
        head = head.child(
            Button::new("Fold")
                .variant(ButtonVariant::GhostMuted)
                .control_size(ControlSize::Mini)
                .on_press(move |s: &mut State| ui_mut(s, &bid).unfolded = false)
                .build(k),
        );
    }
    c = c.child(head);
    // The prompt itself is Needs you's (home::permission), in full.
    if let Some(p) = permission_prompt(s, f, "card/b", &cid, b, None, false) {
        c = c.child(p);
    }
    let raised = ms(&b.at)
        .map(|t| relative_time(t, now_ms()))
        .unwrap_or_default();
    let by = resolved_by(b);
    let meta = [raised, by]
        .into_iter()
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    if !meta.is_empty() {
        c = c.child(
            text(meta)
                .font(ts(11.5, 400.0))
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
        );
    }
    if let Some(e) = &u.error {
        c = c.child(err_line(f, e.clone()));
    }
    c
}
