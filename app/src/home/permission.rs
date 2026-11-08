//! "Waiting for your permission" as buttons (the React app's
//! cards/permission-prompt.tsx; cards #456, #682, docs/design/
//! spawned-agents.md §7): the agent Irori started for a card stopped on a
//! permission prompt nobody is attached to. It shows what is waiting (the
//! tool, and the command, path or URL) and buttons instead of an answer
//! box: typed words don't reach a prompt.
//!
//! - Allow once / Deny answer the live prompt: the pick goes to the device
//!   that runs the agent, where the agent's permission hook answers it.
//! - Always allow does that and also adds the rule to the board's allow
//!   list (Settings › Agents). It is offered only when a rule helps.
//! - Attach answers at the prompt itself: the only way when Irori can't
//!   answer from the card.
//!
//! [`permission_prompt`] is public so the card page can show the same.

use irori_window::cards::{AgentRef, CardBlocker, CardPermission};
use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use crate::logic::cards::blocker;
use crate::logic::names::display_name;
use crate::logic::plain::plain_line;
use crate::spaces::logic::clock_time;
use crate::ui::{Frame, State};
use crate::widgets::spawned_actions::{Action, spawned_actions};

use super::needs::answer_with;
use super::pure::answer_key;

pub const ALLOW_ONCE: &str = "Allow once";
pub const ALWAYS_ALLOW: &str = "Always allow";
pub const DENY: &str = "Deny";

/// The order the buttons come in, whatever order the item's options are.
const ORDER: [&str; 3] = [ALLOW_ONCE, ALWAYS_ALLOW, DENY];

/// The buttons a prompt offers, in order, with their option index: Always
/// allow only when the item carries a rule to add.
pub fn answers(b: &CardBlocker, p: &CardPermission) -> Vec<(&'static str, usize)> {
    ORDER
        .iter()
        .filter_map(|a| {
            b.options
                .iter()
                .position(|o| o.answer == *a)
                .map(|i| (*a, i))
        })
        .filter(|(a, _)| *a != ALWAYS_ALLOW || !p.rule.is_empty())
        .collect()
}

/// What the row says once the person picked.
pub fn picked_line(answer: &str, name: &str, rule: &str) -> String {
    match answer {
        DENY => format!("Denied: {name} goes on without it."),
        ALWAYS_ALLOW => format!(
            "Allowed, and {} is on the board's allow list.",
            plain_line(rule)
        ),
        _ => format!("Allowed once: {name} goes on."),
    }
}

/// The question line: what is stopped, on which tool.
pub fn prompt_line(name: &str, tool: &str, open: bool, has_input: bool) -> String {
    let end = if has_input { ":" } else { "." };
    if open {
        if tool.is_empty() {
            format!("{name} is stopped on a permission prompt{end}")
        } else {
            format!("{name} is stopped on a permission prompt to use {tool}{end}")
        }
    } else {
        format!(
            "{name} asked to use {}{end}",
            if tool.is_empty() { "a tool" } else { tool }
        )
    }
}

/// When Claude Code denies a safety check by itself, as the person's clock
/// reads it.
fn clock(at: &str) -> String {
    let t = clock_time(at);
    if t.is_empty() {
        "2 minutes after asking".into()
    } else {
        t
    }
}

/// One permission prompt: the line, what waits, and its buttons. `who` is
/// the agent on the prompt as the card lists it (for Attach and Logs);
/// the item's own agent when the card doesn't list it.
pub fn permission_prompt(
    s: &State,
    f: &mut Frame,
    key: &str,
    card_id: &str,
    b: &CardBlocker,
    who: Option<&AgentRef>,
    compact: bool,
) -> Option<El> {
    let p = b.permission.as_ref()?;
    let k = f.k;
    let agent = who.or((!p.agent.key.is_empty()).then_some(&p.agent));
    let name = agent.map_or_else(|| "Its agent".to_string(), display_name);
    let open = b.state != blocker::RESOLVED && b.state != blocker::CLOSED;
    let tool = plain_line(&p.tool);
    let akey = answer_key(card_id, &b.id);
    let said = s.home.said.get(&akey).cloned().unwrap_or_default();
    let small = |t: String, color| {
        text(t)
            .font(rune::tok::TextStyle::new(13.0, 400.0))
            .color(color)
            .w_full()
            .wrap()
    };

    let mut c = col()
        .key(format!("{key}/perm-{}", b.id))
        .w_full()
        .min_w(0.0)
        .gap(6.0);
    // The tool in medium weight, as React's <span className="font-medium">.
    let line = prompt_line(&name, &tool, open, !p.input.is_empty());
    let lead = format!("{name} is stopped on a permission prompt to use ");
    let spans = if open && !tool.is_empty() && line.starts_with(&lead) {
        vec![rune::Span::new(lead.len(), lead.len() + tool.len()).weight(500.0)]
    } else {
        Vec::new()
    };
    c = c.child(
        rune::rich(line, spans)
            .font(k.ty.ui)
            .color(k.c.text_base)
            .w_full()
            .wrap_anywhere(),
    );
    if !p.input.is_empty() {
        c = c.child(
            col()
                .key(format!("{key}/perm-{}/input", b.id))
                .w_full()
                .min_w(0.0)
                // React's max-h-24 / max-h-48 with overflow-auto.
                .max_h(if compact { 96.0 } else { 192.0 })
                .min_h(0.0)
                .px(8.0)
                .py(6.0)
                .radius(k.radius.md)
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
                        .w_full()
                        .wrap_anywhere()
                        .selectable(true),
                ),
        );
    }
    if open && p.answerable && !p.expires.is_empty() {
        c = c.child(small(
            format!(
                "A safety check Claude Code asks even in bypass mode: it denies the call by itself at {} unless you answer.",
                clock(&p.expires)
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
    let picks = answers(b, p);
    if !said.is_empty() {
        c = c.child(
            small(said.clone(), k.c.text_muted)
                .key(format!("{key}/perm-{}/said", b.id))
                .role("status"),
        );
    } else if open {
        let mut bs = row()
            .key(format!("{key}/perm-{}/acts", b.id))
            .w_full()
            .gap(6.0)
            .flex_wrap()
            .items_start()
            .role("group")
            .label(format!("Answers to {name}'s permission prompt"));
        if p.answerable {
            for (a, i) in &picks {
                let v = match *a {
                    ALLOW_ONCE => ButtonVariant::Contrast,
                    DENY => ButtonVariant::Danger,
                    _ => ButtonVariant::Outline,
                };
                let help = if *a == ALWAYS_ALLOW {
                    format!("Also adds {} to the board's allow list", p.rule)
                } else {
                    b.options
                        .get(*i)
                        .map(|o| plain_line(&o.why))
                        .unwrap_or_default()
                };
                let (cid, bb, i, line) = (
                    card_id.to_string(),
                    b.clone(),
                    *i as i64,
                    picked_line(a, &name, &p.rule),
                );
                let mut bt = Button::new(*a)
                    .variant(v)
                    .control_size(ControlSize::Small)
                    .on_press(move |s: &mut State| {
                        answer_with(
                            s,
                            &cid,
                            &bb,
                            i,
                            String::new(),
                            line.clone(),
                            "Your answer to the permission prompt wasn't saved",
                        )
                    });
                if !help.is_empty() {
                    bt = bt.help(help);
                }
                bs = bs.child(bt.build(k).key(format!("{key}/perm-{}/{a}", b.id)));
            }
        }
        if let Some(w) = agent {
            bs = bs.child(spawned_actions(
                s,
                f,
                &format!("{key}/perm-{}/spawned", b.id),
                w,
                card_id,
                &[Action::Attach, Action::Logs],
            ));
        }
        c = c.child(bs);
    }
    if open
        && said.is_empty()
        && p.answerable
        && !p.rule.is_empty()
        && picks.iter().any(|(a, _)| *a == ALWAYS_ALLOW)
    {
        // The rule in mono, as React's <code>.
        let rule = plain_line(&p.rule);
        let head = "Always allow also adds ";
        c = c.child(
            rune::rich(
                format!("{head}{rule} to the board's allow list (Settings › Agents)."),
                vec![rune::Span::new(head.len(), head.len() + rule.len()).mono()],
            )
            .font(k.ty.text_xsmall)
            .color(k.c.text_muted)
            .w_full()
            .wrap_anywhere(),
        );
    }
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use irori_window::cards::CardOption;

    fn item(opts: &[&str]) -> CardBlocker {
        CardBlocker {
            options: opts
                .iter()
                .map(|a| CardOption {
                    answer: a.to_string(),
                    ..CardOption::default()
                })
                .collect(),
            ..CardBlocker::default()
        }
    }

    #[test]
    fn buttons_in_order_always_allow_only_with_a_rule() {
        let b = item(&[DENY, ALWAYS_ALLOW, ALLOW_ONCE]);
        let mut p = CardPermission {
            rule: "Bash(cargo test:*)".into(),
            ..CardPermission::default()
        };
        assert_eq!(
            answers(&b, &p),
            vec![(ALLOW_ONCE, 2), (ALWAYS_ALLOW, 1), (DENY, 0)]
        );
        p.rule.clear();
        assert_eq!(answers(&b, &p), vec![(ALLOW_ONCE, 2), (DENY, 0)]);
        assert!(answers(&item(&["Something else"]), &p).is_empty());
    }

    #[test]
    fn lines() {
        assert_eq!(
            prompt_line("thyme", "Bash", true, true),
            "thyme is stopped on a permission prompt to use Bash:"
        );
        assert_eq!(
            prompt_line("thyme", "", true, false),
            "thyme is stopped on a permission prompt."
        );
        assert_eq!(
            prompt_line("thyme", "", false, false),
            "thyme asked to use a tool."
        );
        assert_eq!(
            picked_line(DENY, "thyme", ""),
            "Denied: thyme goes on without it."
        );
        assert_eq!(
            picked_line(ALLOW_ONCE, "thyme", ""),
            "Allowed once: thyme goes on."
        );
        assert_eq!(
            picked_line(ALWAYS_ALLOW, "thyme", "Bash(ls:*)"),
            "Allowed, and Bash(ls:*) is on the board's allow list."
        );
        assert_eq!(clock("not a time"), "2 minutes after asking");
    }
}
