//! "Waiting for your permission" (permission-prompt.tsx, cards #456 and
//! #682): the agent Irori started for a card stopped on a permission
//! prompt nobody is attached to. The item shows what waits (the tool, and
//! the command, path or URL) and buttons instead of an answer box: typed
//! words don't reach a prompt.
//!
//! - Allow once and Deny answer the live prompt: the pick goes to the
//!   device that runs the agent, where its permission hook answers it.
//! - Always allow does that and also adds the rule to the board's allow
//!   list (Settings › Agents). It is offered only when a rule helps.
//! - Attach answers at the prompt itself, the only way when Irori can't
//!   answer from the card.

use coord::{
    PERMISSION_ALLOW_ONCE as ALLOW_ONCE, PERMISSION_ALWAYS as ALWAYS_ALLOW, PERMISSION_DENY as DENY,
};
use irori_window::cards::{AgentRef, CardBlocker, CardDetail, CardPermission};
use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use super::blockers::{BlockerUi, resolved_by, ui_mut};
use super::{err_line, ts};
use crate::logic::agent_status::{ms, now_ms};
use crate::logic::names::display_name;
use crate::logic::plain::{plain_line, relative_time};
use crate::logic::thread::blocker_item_id;
use crate::text::{self as tt, RichOpts};
use crate::ui::{Frame, State};
use crate::widgets::{bits, spawned_actions};

/// The order the buttons come in, whatever order the item's options are.
const ORDER: [&str; 3] = [ALLOW_ONCE, ALWAYS_ALLOW, DENY];

/// The answers to offer, in [`ORDER`], each with its option's index:
/// Always allow only when there is a rule to add.
pub fn answers(b: &CardBlocker, p: &CardPermission) -> Vec<(&'static str, usize)> {
    ORDER
        .iter()
        .filter(|a| **a != ALWAYS_ALLOW || !p.rule.is_empty())
        .filter_map(|a| {
            b.options
                .iter()
                .position(|o| o.answer == *a)
                .map(|i| (*a, i))
        })
        .collect()
}

/// The agent on the prompt: the one the card lists, else none when the
/// permission names no agent.
fn agent_of(p: &CardPermission) -> Option<&AgentRef> {
    (!p.agent.key.is_empty()).then_some(&p.agent)
}

/// The prompt's agent by name, "Its agent" when unknown.
pub fn agent_name(p: &CardPermission) -> String {
    agent_of(p)
        .map(display_name)
        .unwrap_or_else(|| "Its agent".into())
}

/// What waits, as a sentence: with its tool in bold (the returned range)
/// and a colon when the input follows.
pub fn headline(p: &CardPermission, open: bool) -> (String, Option<std::ops::Range<usize>>) {
    let name = agent_name(p);
    let tool = plain_line(&p.tool);
    let end = if p.input.is_empty() { "." } else { ":" };
    if open {
        if tool.is_empty() {
            return (
                format!("{name} is stopped on a permission prompt{end}"),
                None,
            );
        }
        let head = format!("{name} is stopped on a permission prompt to use ");
        let at = head.len();
        (format!("{head}{tool}{end}"), Some(at..at + tool.len()))
    } else {
        let tool = if tool.is_empty() {
            "a tool".to_string()
        } else {
            tool
        };
        (format!("{name} asked to use {tool}{end}"), None)
    }
}

/// The line that replaces the buttons once the person picked.
pub fn said(answer: &str, p: &CardPermission) -> String {
    let name = agent_name(p);
    match answer {
        DENY => format!("Denied: {name} goes on without it."),
        ALWAYS_ALLOW => format!(
            "Allowed, and {} is on the board's allow list.",
            plain_line(&p.rule)
        ),
        _ => format!("Allowed once: {name} goes on."),
    }
}

/// When Claude Code denies a safety check by itself: the person's clock
/// time, or "2 minutes after asking" when the time doesn't read.
pub fn expires_at(expires: &str) -> String {
    let t = crate::spaces::logic::clock_time(expires);
    if t.is_empty() {
        "2 minutes after asking".into()
    } else {
        t
    }
}

/// Sends the pick: the item resolves as the person, and the agent's
/// device answers the hook.
fn send(s: &mut State, card: &str, b: &CardBlocker, option: usize, answer: &'static str) {
    let bid = b.id.clone();
    let Some(p) = b.permission.clone() else {
        return;
    };
    {
        let u = ui_mut(s, &bid);
        if u.busy || !u.said.is_empty() {
            return;
        }
        u.busy = true;
        u.error = None;
        // As React's optimistic answer: the line shows at once, and goes
        // again if the answer isn't saved.
        u.said = said(answer, &p);
    }
    let cid = card.to_string();
    let bid2 = bid.clone();
    s.spawn(
        move |w| w.answer(&cid, &bid2, option as i64, "", true, &[]),
        move |s, r| {
            let u = ui_mut(s, &bid);
            u.busy = false;
            if let Err(e) = r {
                u.said.clear();
                u.error = Some(format!(
                    "Your answer to the permission prompt wasn't saved: {}",
                    plain_line(&e)
                ));
            }
        },
    );
}

/// The prompt's body under the question (PermissionPrompt): what waits,
/// the warnings, and the buttons or what the pick said.
pub fn prompt(
    s: &State,
    f: &mut Frame,
    cid: &str,
    b: &CardBlocker,
    u: &BlockerUi,
    compact: bool,
) -> Option<El> {
    let k = f.k;
    let p = b.permission.as_ref()?;
    let key = format!("card/b/{}/perm", b.id);
    let open = b.state != "resolved" && b.state != "closed" && b.resolved.is_none();
    let name = agent_name(p);
    let (head, bold) = headline(p, open);
    let mut spans = Vec::new();
    if let Some(r) = bold {
        spans.push(rune::Span::new(r.start, r.end).weight(500.0));
    }
    let mut c = col().key(key.clone()).w_full().min_w(0.0).gap(6.0).child(
        rune::rich(head, spans)
            .font(ts(13.5, 400.0))
            .color(k.c.text_base)
            .w_full()
            .wrap_anywhere(),
    );
    if !p.input.is_empty() {
        let tool = plain_line(&p.tool);
        c = c.child(
            col()
                .key(format!("{key}/input"))
                .w_full()
                .min_w(0.0)
                .max_h(if compact { 96.0 } else { 192.0 })
                .min_h(0.0)
                .px(8.0)
                .py(6.0)
                .radius(6.0)
                .fill(k.c.layer_02)
                .border(1.0, k.c.border_base)
                .label(if tool.is_empty() {
                    "What is waiting".to_string()
                } else {
                    format!("The {tool} input waiting")
                })
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    text(p.input.clone())
                        .font(k.ty.mono)
                        .color(k.c.text_base)
                        .selectable(true)
                        .w_full()
                        .wrap_anywhere(),
                ),
        );
    }
    let small =
        |t: String, color: rune::Color| text(t).font(ts(13.0, 400.0)).color(color).w_full().wrap();
    if open && p.answerable && !p.expires.is_empty() {
        c = c.child(small(
            format!(
                "A safety check Claude Code asks even in bypass mode: it denies the call by itself at {} unless you answer.",
                expires_at(&p.expires)
            ),
            k.c.warning.fg,
        ));
    }
    if open && !p.answerable {
        c = c.child(small(
            format!("Irori can't answer this prompt from the card: attach to {name} to answer it."),
            k.c.text_muted,
        ));
    }
    let offered = answers(b, p);
    if !u.said.is_empty() {
        c = c.child(small(u.said.clone(), k.c.text_muted));
    } else if open {
        let mut buttons = row()
            .key(format!("{key}/answers"))
            .w_full()
            .min_w(0.0)
            .gap(6.0)
            .items_start()
            .flex_wrap()
            .label(format!("Answers to {name}'s permission prompt"));
        if p.answerable {
            for (a, i) in &offered {
                let (a, i) = (*a, *i);
                let variant = match a {
                    ALLOW_ONCE => ButtonVariant::Submit,
                    DENY => ButtonVariant::Danger,
                    _ => ButtonVariant::Outline,
                };
                let why = b
                    .options
                    .get(i)
                    .map(|o| plain_line(&o.why))
                    .unwrap_or_default();
                let tip = if a == ALWAYS_ALLOW {
                    format!("Also adds {} to the board's allow list", p.rule)
                } else {
                    why
                };
                let (cid2, b2) = (cid.to_string(), b.clone());
                let mut bt = Button::new(a)
                    .variant(variant)
                    .control_size(ControlSize::Small)
                    .disabled(u.busy)
                    .on_press(move |s: &mut State| send(s, &cid2, &b2, i, a));
                if !tip.is_empty() {
                    bt = bt.help(tip);
                }
                buttons = buttons.child(bt.build(k));
            }
        }
        if let Some(a) = agent_of(p) {
            buttons = buttons.child(spawned_actions::spawned_actions(
                s,
                f,
                &format!("{key}/spawned"),
                a,
                cid,
                &[
                    spawned_actions::Action::Attach,
                    spawned_actions::Action::Logs,
                ],
            ));
        }
        c = c.child(buttons);
    }
    if open
        && u.said.is_empty()
        && p.answerable
        && !p.rule.is_empty()
        && offered.iter().any(|(a, _)| *a == ALWAYS_ALLOW)
    {
        let rule = plain_line(&p.rule);
        let head = "Always allow also adds ";
        let line = format!("{head}{rule} to the board's allow list (Settings › Agents).");
        c = c.child(
            rune::rich(
                line,
                vec![rune::Span::new(head.len(), head.len() + rule.len()).mono()],
            )
            .font(ts(12.0, 400.0))
            .color(k.c.text_muted)
            .w_full()
            .wrap_anywhere(),
        );
    }
    Some(c)
}

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
    if let Some(p) = prompt(s, f, &cid, b, u, false) {
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

#[cfg(test)]
mod tests {
    use irori_window::cards::CardOption;

    use super::*;

    fn opt(a: &str) -> CardOption {
        CardOption {
            answer: a.into(),
            ..CardOption::default()
        }
    }

    fn perm(rule: &str) -> CardPermission {
        CardPermission {
            agent: AgentRef {
                key: "d/thyme".into(),
                name: "thyme".into(),
                ..AgentRef::default()
            },
            tool: "Bash".into(),
            input: "cargo test".into(),
            rule: rule.into(),
            answerable: true,
            ..CardPermission::default()
        }
    }

    #[test]
    fn buttons_come_in_order_whatever_the_options_order() {
        let b = CardBlocker {
            options: vec![opt(DENY), opt(ALWAYS_ALLOW), opt(ALLOW_ONCE)],
            ..CardBlocker::default()
        };
        assert_eq!(
            answers(&b, &perm("Bash(cargo test)")),
            vec![(ALLOW_ONCE, 2), (ALWAYS_ALLOW, 1), (DENY, 0)]
        );
    }

    #[test]
    fn always_allow_needs_a_rule() {
        let b = CardBlocker {
            options: vec![opt(ALLOW_ONCE), opt(ALWAYS_ALLOW), opt(DENY)],
            ..CardBlocker::default()
        };
        assert_eq!(answers(&b, &perm("")), vec![(ALLOW_ONCE, 0), (DENY, 2)]);
    }

    #[test]
    fn the_headline_says_what_waits() {
        let p = perm("");
        let (t, bold) = headline(&p, true);
        assert_eq!(t, "thyme is stopped on a permission prompt to use Bash:");
        assert_eq!(&t[bold.unwrap()], "Bash");
        assert_eq!(headline(&p, false).0, "thyme asked to use Bash:");
        let none = CardPermission::default();
        assert_eq!(
            headline(&none, true),
            ("Its agent is stopped on a permission prompt.".into(), None)
        );
        assert_eq!(headline(&none, false).0, "Its agent asked to use a tool.");
    }

    #[test]
    fn the_pick_says_what_happens() {
        let p = perm("Bash(cargo test)");
        assert_eq!(said(ALLOW_ONCE, &p), "Allowed once: thyme goes on.");
        assert_eq!(said(DENY, &p), "Denied: thyme goes on without it.");
        assert_eq!(
            said(ALWAYS_ALLOW, &p),
            "Allowed, and Bash(cargo test) is on the board's allow list."
        );
    }

    #[test]
    fn a_bad_expiry_reads_as_two_minutes() {
        assert_eq!(expires_at("soon"), "2 minutes after asking");
        assert!(expires_at("2026-11-07T18:47:00Z").ends_with('M'));
    }
}
