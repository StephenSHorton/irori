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
        write_agent(&mut b, card, &c.sender.key, &now, &d);
    }
    let head = format!(
        "{shown} of {} agents on this device{}.",
        cards.len(),
        describe_agent_filter(status, cwd)
    );
    if shown == 0 {
        return head;
    }
    format!(
        "{head} Message one with send (to = its name or key). {}\n{b}",
        untrusted()
    )
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
    let last = if c.last_activity.is_zero() {
        c.updated_at
    } else {
        c.last_activity
    };
    let _ = writeln!(b, "  last activity: {}", ago(now, &last));
    let txt = one_line(&c.last_text, MAX_FIELD_RUNES);
    if !txt.is_empty() {
        let _ = writeln!(b, "  last text (untrusted): {}", q(&txt));
    }
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
