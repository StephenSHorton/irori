//! agents and events (I:internal/tools/observe.go).

use std::collections::HashMap;
use std::fmt::Write;

use gocompat::json::AnyValue;
use gocompat::time::GoTime;

use crate::args::Ty;
use crate::format::{MAX_FIELD_RUNES, ago, local, one_line, payload_text, q, quoted, trim};
use crate::{Caller, Inner, decode, err_text, untrusted};

const DEFAULT_EVENT_LIMIT: i64 = 20;
const MAX_EVENT_LIMIT: i64 = 100;

/// The assistant's `screen` tool (card #1009): the window's own words on
/// what it shows, as the last ask sent them. Read-only, no pixels.
pub(crate) fn screen(s: &Inner, _c: &Caller, raw: &[u8]) -> String {
    if let Err(e) = decode(raw, &[]) {
        return e;
    }
    let t = s.b.assistant_screen();
    if t.trim().is_empty() {
        return "The window has not said what is on screen.".into();
    }
    t
}

const AGENTS_ARGS: &[(&str, Ty)] = &[("status", Ty::Str), ("cwd", Ty::Str)];

pub(crate) fn agents(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, AGENTS_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let (status, cwd) = (a.str("status"), a.str("cwd"));
    let mut cards = s.b.agents();
    cards.sort_by(|x, y| {
        gocompat::text::to_lower_str(&x.name)
            .cmp(&gocompat::text::to_lower_str(&y.name))
            .then_with(|| x.key.cmp(&y.key))
    });
    let now = s.now();
    let doing = s.doing(c);
    let mut b = String::new();
    let mut shown = 0;
    for card in &cards {
        if !match_agent(card, status, cwd) {
            continue;
        }
        shown += 1;
        let d = doing.get(&card.key).cloned().unwrap_or_default();
        write_agent(&mut b, card, &c.sender.key, &now, &d, cut_off(s, card));
    }
    // Other members' agents on shared boards (card #372): what they hold
    // and whether their device is there.
    let mut others = s.b.board_agents();
    others.sort_by(|x, y| {
        x.board
            .cmp(&y.board)
            .then_with(|| {
                gocompat::text::to_lower_str(&x.card.name)
                    .cmp(&gocompat::text::to_lower_str(&y.card.name))
            })
            .then_with(|| x.card.key.cmp(&y.card.key))
    });
    let mut ob = String::new();
    let mut oshown = 0;
    for o in &others {
        if !match_agent(&o.card, status, cwd) {
            continue;
        }
        oshown += 1;
        write_board_agent(&mut ob, o, &now);
    }
    let mut head = format!(
        "{shown} of {} agents on the person's devices{}.",
        cards.len(),
        describe_agent_filter(status, cwd)
    );
    if !others.is_empty() {
        let _ = write!(
            head,
            " {oshown} of {} other people's agents in shared spaces.",
            others.len()
        );
    }
    if shown + oshown == 0 {
        return head;
    }
    if oshown > 0 {
        let _ = write!(
            b,
            "\nOther people's agents (in shared spaces; mention them on a card there):\n{ob}"
        );
    }
    format!(
        "{head} Message one with send (to = its name or key). {}\n{b}",
        untrusted()
    )
}

/// A live Claude session on this device with no attach stream (GitHub #57):
/// its process runs, so presence lists it, but its Irori tools and wakes
/// can't reach it. Only this device's Core knows, so a session on another
/// device is never called cut off.
fn cut_off(s: &Inner, c: &presence::AgentCard) -> bool {
    c.agent == "claude"
        && c.device_state.is_empty()
        && c.status != "gone"
        && s.b.attached(&c.key) == Some(false)
}

/// One device's presence for an agent's line (card #372).
pub(crate) fn device_line(c: &presence::AgentCard, now: &GoTime) -> String {
    one_line(
        &coord::roster::device_phrase(&c.device_state, &c.device_seen, &c.device_via, now),
        120,
    )
}

fn write_board_agent(b: &mut String, o: &crate::BoardAgent, now: &GoTime) {
    let c = &o.card;
    let mut name = one_line(&c.name, MAX_FIELD_RUNES);
    if name.is_empty() {
        name = "unnamed".into();
    }
    let mut whose = one_line(&o.person, MAX_FIELD_RUNES);
    if whose.is_empty() {
        whose = "another member".into();
    }
    let _ = writeln!(
        b,
        "\n- {name} [{}] {whose}'s agent in space {}, key {}",
        one_line(&c.status, 40),
        one_line(&o.board, 60),
        one_line(&c.key, 120)
    );
    let _ = writeln!(b, "  device: {}", device_line(c, now));
}

fn match_agent(c: &presence::AgentCard, status: &str, cwd: &str) -> bool {
    let st = trim(status);
    if !st.is_empty() && !gocompat::text::equal_fold(&c.status, st) {
        return false;
    }
    let cw = trim(cwd);
    if !cw.is_empty() && !norm_path(&c.cwd).contains(&norm_path(cw)) {
        return false;
    }
    true
}

fn norm_path(p: &str) -> String {
    gocompat::text::to_lower_str(&p.replace('\\', "/"))
}

fn describe_agent_filter(status: &str, cwd: &str) -> String {
    let mut parts = vec![];
    if !cwd.is_empty() {
        parts.push(format!("cwd contains {}", q(&one_line(cwd, 120))));
    }
    if !status.is_empty() {
        parts.push(format!("status {}", one_line(status, 40)));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!(" matching {}", parts.join(", "))
}

impl Inner {
    /// Each live agent's one line from capture's overview (empty with
    /// capture off).
    pub(crate) fn doing(&self, c: &Caller) -> HashMap<String, capture::Line> {
        let Some(cp) = self.b.capture() else {
            return HashMap::new();
        };
        let open = self.b.coord().list_tasks(&coord::TaskFilter {
            open: true,
            ..coord::TaskFilter::default()
        });
        let input = capture::OverviewInput {
            current: crate::explore::current(&open),
            spawned: crate::explore::spawned_lines(
                self.b.coord(),
                self.b.agents().into_iter().map(|c| c.key),
            ),
            ..capture::OverviewInput::default()
        };
        let ov = cp.overview(&c.sender.key, &input);
        ov.agents.into_iter().map(|a| (a.key, a.doing)).collect()
    }
}

fn write_agent(
    b: &mut String,
    c: &presence::AgentCard,
    me: &str,
    now: &GoTime,
    doing: &capture::Line,
    detached: bool,
) {
    // Every field here can come from a file a peer agent wrote, so all of
    // them go through one_line, not only the free-text ones.
    let key = one_line(&c.key, 120);
    let mut name = one_line(&c.name, MAX_FIELD_RUNES);
    if name.is_empty() {
        name = "unnamed".into();
    }
    let mut agent = one_line(&c.agent, 60);
    if agent.is_empty() {
        agent = "agent".into();
    }
    let you = if !c.key.is_empty() && c.key == me {
        " (you)"
    } else {
        ""
    };
    let _ = writeln!(
        b,
        "\n- {name}{you} [{}] {agent}, key {key}",
        one_line(&c.status, 40)
    );
    let title = one_line(&c.title, MAX_FIELD_RUNES);
    if !title.is_empty() {
        let _ = writeln!(b, "  title: {title}");
    }
    if !c.cwd.is_empty() {
        let _ = writeln!(b, "  cwd: {}", one_line(&c.cwd, MAX_FIELD_RUNES));
    }
    let mut repo = one_line(&c.repo_key, MAX_FIELD_RUNES);
    if repo.is_empty() {
        repo = "none (not in a git repo)".into();
    }
    let _ = writeln!(b, "  repoKey: {repo}");
    let d = one_line(&doing.text, capture::OVERVIEW_LINE_MAX);
    if !d.is_empty() {
        let _ = writeln!(
            b,
            "  doing: {} ({})",
            quoted(&d, capture::OVERVIEW_LINE_MAX),
            one_line(&doing.r#ref, 200)
        );
    }
    let mut mode = one_line(&c.permission_mode, 60);
    if c.agent == "grok" {
        mode = "n/a".into();
    } else if mode.is_empty() {
        mode = "unknown".into();
    }
    let _ = writeln!(b, "  permissionMode: {mode}");
    if detached {
        let _ = writeln!(
            b,
            "  device: this device, but the session isn't attached to Irori: its tools and wakes can't reach it"
        );
    } else {
        let _ = writeln!(b, "  device: {}", device_line(c, now));
    }
    let _ = writeln!(b, "  last activity: {}", last_activity(c, now));
    let txt = one_line(&c.last_text, MAX_FIELD_RUNES);
    if !txt.is_empty() {
        let _ = writeln!(b, "  last text (untrusted): {}", q(&txt));
    }
}

/// An agent's last activity. Mid-turn (busy, a shell command, a prompt)
/// it says so, with how long it has been in that state: a long command
/// writes no transcript row, and a session with no transcript here (card
/// #159) has none at all, which read as "never" or hours ago.
pub(crate) fn last_activity(c: &presence::AgentCard, now: &GoTime) -> String {
    let in_turn = !matches!(
        c.status.as_str(),
        "" | "idle" | "gone" | coord::roster::NOT_ANSWERING
    );
    if in_turn {
        let mut s = "in a turn now".to_string();
        if !c.updated_at.is_zero() {
            s += &format!(
                " ({} since {})",
                one_line(&c.status, 40),
                ago(now, &c.updated_at)
            );
        }
        if !c.last_activity.is_zero() {
            s += &format!("; last transcript row {}", ago(now, &c.last_activity));
        }
        return s;
    }
    let last = if c.last_activity.is_zero() {
        c.updated_at
    } else {
        c.last_activity
    };
    ago(now, &last)
}

const EVENTS_ARGS: &[(&str, Ty)] = &[("kinds", Ty::Strs), ("limit", Ty::Int)];

pub(crate) fn events(s: &Inner, _: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, EVENTS_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let mut limit = a.int("limit");
    if limit <= 0 {
        limit = DEFAULT_EVENT_LIMIT;
    }
    let limit = limit.min(MAX_EVENT_LIMIT);
    let kinds: Vec<String> = a
        .strs("kinds")
        .iter()
        .map(|k| trim(k).to_string())
        .filter(|k| !k.is_empty())
        .collect();
    let evs = match s.b.events(&store::Filter {
        kinds: kinds.clone(),
        limit,
        desc: true,
        ..store::Filter::default()
    }) {
        Ok(v) => v,
        Err(e) => return err_text(&format!("Irori couldn't read its event log: {e}")),
    };
    let evs: Vec<_> = evs.into_iter().take(limit as usize).collect();
    let now = s.now();
    let mut head = format!("{} events on this device, newest first", evs.len());
    if !kinds.is_empty() {
        head += &format!(" (kinds: {})", one_line(&kinds.join(", "), 200));
    }
    if evs.is_empty() {
        return head + ".";
    }
    let mut b = String::new();
    for ev in &evs {
        write_event(&mut b, ev, &now);
    }
    format!(
        "{head}. Message text is left out; the inbox tool shows messages addressed to you. {}\n{b}",
        untrusted()
    )
}

fn write_event(b: &mut String, ev: &store::Event, now: &GoTime) {
    let when = if ev.hlc.0 == 0 {
        "unknown time".to_string()
    } else {
        let at = GoTime::unix_milli(ev.hlc.wall_ms());
        format!("{} ({})", local(&at, now).format_date_time(), ago(now, &at))
    };
    let mut dev = one_line(&ev.device_id, MAX_FIELD_RUNES);
    if dev.chars().count() > 8 {
        dev = dev.chars().take(8).collect();
    }
    let _ = writeln!(
        b,
        "- {when} {} dev={dev} seq={} actor={} {}",
        one_line(&ev.kind, 60),
        ev.seq,
        one_line(&ev.actor, 120),
        payload_text(&redact(&ev.kind, &ev.payload))
    );
}

const LEFT_OUT: &[u8] = br#"{"text":"[left out]"}"#;

/// A row as an agent may see it (the events tool, and the loopback API
/// without the window's credential, v0.4 §6.8): message text and reasons
/// are left out, and so is a turn's prompt when it is an Irori fence.
pub fn redact(kind: &str, raw: &[u8]) -> Vec<u8> {
    let fields: &[&str] = match kind {
        inbox::kinds::MESSAGE_SENT => &["text", "reason"],
        capture::KIND_TURN => {
            // The stored JSON may escape "<"; read the prompt itself (Go's
            // Unmarshal into struct{Prompt string}: keys fold, last wins).
            match &gocompat::json::unmarshal_any(raw) {
                Ok(AnyValue::Null) => return raw.to_vec(),
                Ok(AnyValue::Object(m)) => {
                    let mut prompt = Some(String::new());
                    for (k, v) in m {
                        if gocompat::text::equal_fold(k, "prompt") {
                            prompt = match v {
                                AnyValue::String(p) => Some(p.clone()),
                                AnyValue::Null => prompt,
                                _ => None,
                            };
                        }
                    }
                    if let Some(p) = prompt
                        && !gocompat::text::to_lower_str(&p).contains("<irori-message")
                    {
                        return raw.to_vec();
                    }
                }
                _ => {}
            }
            &["prompt"]
        }
        _ => return raw.to_vec(),
    };
    let mut v = match gocompat::json::unmarshal_any(raw) {
        Ok(v) => v,
        Err(_) => return LEFT_OUT.to_vec(),
    };
    let AnyValue::Object(m) = &mut v else {
        return LEFT_OUT.to_vec();
    };
    for k in fields {
        if m.contains_key(*k) {
            m.insert((*k).to_string(), AnyValue::String("[left out]".into()));
        }
    }
    gocompat::json::marshal(&v).unwrap_or_else(|_| LEFT_OUT.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(status: &str, updated: i64, last: i64) -> presence::AgentCard {
        let t = |s: i64| {
            if s == 0 {
                GoTime::ZERO
            } else {
                GoTime::unix(s, 0)
            }
        };
        presence::AgentCard {
            status: status.into(),
            updated_at: t(updated),
            last_activity: t(last),
            ..presence::AgentCard::default()
        }
    }

    /// Card #159: a busy agent's last activity is its turn, never "never".
    #[test]
    fn last_activity_shows_the_turn_in_progress() {
        let now = GoTime::unix(10_000, 0);
        assert_eq!(last_activity(&card("busy", 0, 0), &now), "in a turn now");
        assert_eq!(
            last_activity(&card("shell", 10_000 - 600, 10_000 - 540), &now),
            "in a turn now (shell since 10m ago); last transcript row 9m ago"
        );
        assert_eq!(
            last_activity(&card("busy", 10_000 - 3600, 0), &now),
            "in a turn now (busy since 1h ago)"
        );
        assert_eq!(last_activity(&card("idle", 0, 0), &now), "never");
        assert_eq!(last_activity(&card("idle", 9_000, 9_940), &now), "1m ago");
    }
}
