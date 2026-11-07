//! The words the inbox's surfaces say, pure so they're tested without a
//! window: the React app's lib/irori.ts `plainLine`, `plainBlock` and
//! `relativeTime`, lib/names.ts `nameOr`, and lib/talk.ts `holdText`,
//! `repoShort`, `allowLifts`, `startedLine`, `nameLookup`, plus
//! lib/mentions.ts `notDelivered`. The palette and the desk use them too.

use std::collections::HashMap;

use irori_window::mentions::MentionOutcome;
use irori_window::rune_inbox::{Delivery, GoTime, hold, st};
use irori_window::ts::StartedAgent;

/// What a key nothing names reads as (card #478).
pub const UNKNOWN_AGENT: &str = "unknown agent";

/// C0/C1 controls, zero-width marks, bidi embeddings, overrides and
/// isolates, and the BOM.
fn hidden(c: char) -> bool {
    let c = c as u32;
    c < 0x20
        || (0x7f..=0x9f).contains(&c)
        || (0x200b..=0x200f).contains(&c)
        || (0x202a..=0x202e).contains(&c)
        || (0x2066..=0x2069).contains(&c)
        || c == 0xfeff
}

/// One line for a person to read: hidden characters and runs of white
/// space become one space, trimmed.
pub fn plain_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for ch in s.chars() {
        let ch = if hidden(ch) { ' ' } else { ch };
        if ch.is_whitespace() {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(ch);
    }
    out
}

/// A block of text: line breaks and tabs kept, other hidden characters
/// spaces, at most three blank lines in a row, trimmed.
pub fn plain_block(s: &str) -> String {
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(s.len());
    let mut newlines = 0;
    for ch in s.chars() {
        let ch = if ch == '\n' || ch == '\t' || !hidden(ch) {
            ch
        } else {
            ' '
        };
        if ch == '\n' {
            newlines += 1;
            if newlines > 3 {
                continue;
            }
        } else {
            newlines = 0;
        }
        out.push(ch);
    }
    out.trim().to_string()
}

/// Milliseconds since the Unix epoch now.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// A time as milliseconds, None for Go's zero time.
pub fn ms(t: &GoTime) -> Option<i64> {
    (!t.is_zero()).then(|| t.unix_milli_value())
}

/// "just now", "12s ago", "5m ago", "3h ago", "2d ago"; "never" for none.
pub fn relative_time(then: Option<i64>, now: i64) -> String {
    let Some(then) = then else {
        return "never".into();
    };
    let s = ((now - then) as f64 / 1000.0).round() as i64;
    if s < 5 {
        return "just now".into();
    }
    if s < 60 {
        return format!("{s}s ago");
    }
    let m = s / 60;
    if m < 60 {
        return format!("{m}m ago");
    }
    let h = m / 60;
    if h < 24 {
        return format!("{h}h ago");
    }
    format!("{}d ago", h / 24)
}

/// [`relative_time`] of a Go time.
pub fn ago(t: &GoTime, now: i64) -> String {
    relative_time(ms(t), now)
}

/// A local "14:05" clock time ("—" for none).
pub fn clock(t: &GoTime) -> String {
    if t.is_zero() {
        return "—".into();
    }
    let off = sys::tz::local_offset(t.unix_milli_value() / 1000);
    let (_, _, _, h, m, _) = t.with_offset(off).civil();
    format!("{h:02}:{m:02}")
}

fn is_alnum(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

fn is_hex(c: char) -> bool {
    c.is_ascii_digit() || ('a'..='f').contains(&c)
}

/// Whether `s` is (or holds) an actor key rather than a name: board:<b>/<x>,
/// agent:<id>, person:<id>, or <hex device>/<hex session>.
pub fn is_key_like(s: &str) -> bool {
    let b: Vec<char> = s.chars().collect();
    let boundary = |i: usize| i == 0 || !(b[i - 1].is_alphanumeric() || b[i - 1] == '_');
    let run =
        |from: usize, ok: &dyn Fn(char) -> bool| b[from..].iter().take_while(|c| ok(**c)).count();
    for i in 0..b.len() {
        if !boundary(i) {
            continue;
        }
        let rest: String = b[i..].iter().take(7).collect();
        if rest.starts_with("board:") {
            let n = run(i + 6, &is_alnum);
            if n > 0
                && b.get(i + 6 + n) == Some(&'/')
                && b.get(i + 7 + n).is_some_and(|c| !c.is_whitespace())
            {
                return true;
            }
        }
        if rest.starts_with("agent:") && run(i + 6, &is_alnum) >= 6 {
            return true;
        }
        if rest.starts_with("person:") && run(i + 7, &is_alnum) >= 6 {
            return true;
        }
        let n = run(i, &is_hex);
        if n >= 16 && b.get(i + n) == Some(&'/') && run(i + n + 1, &|c| is_hex(c) || c == '-') >= 8
        {
            return true;
        }
    }
    false
}

/// `name` for a person to read, or `fallback` when it is empty or a key.
pub fn name_or(name: &str, fallback: &str) -> String {
    let n = plain_line(name);
    if n.is_empty() || is_key_like(&n) {
        fallback.to_string()
    } else {
        n
    }
}

/// Names actor keys: the first name any source gives a key; "you" for the
/// person; [`UNKNOWN_AGENT`] for one nothing names.
#[derive(Clone, Debug, Default)]
pub struct Names {
    names: HashMap<String, String>,
}

impl Names {
    pub fn add(&mut self, key: &str, name: &str) {
        if !key.is_empty() && !name.is_empty() && !self.names.contains_key(key) {
            self.names.insert(key.to_string(), name.to_string());
        }
    }

    pub fn name(&self, key: &str) -> String {
        if key.is_empty() {
            return String::new();
        }
        if key.starts_with("user:") {
            return "you".into();
        }
        name_or(
            self.names.get(key).map_or("", String::as_str),
            UNKNOWN_AGENT,
        )
    }
}

/// A repo key as a person reads it: "local repo 5a3c1e0" for a local one,
/// else its last part without .git.
pub fn repo_short(key: &str) -> String {
    if let Some(rest) = key.strip_prefix("root:") {
        return format!("local repo {}", rest.chars().take(7).collect::<String>());
    }
    let k = key.trim_end_matches('/');
    let k = k.strip_suffix(".git").unwrap_or(k);
    match k.rfind(['/', ':', '\\']) {
        Some(i) if i < k.len() - 1 => k[i + 1..].to_string(),
        _ => k.to_string(),
    }
}

/// Repo keys named by this device's checkouts (inbox_repo_names), else
/// [`repo_short`].
#[derive(Clone, Debug, Default)]
pub struct Repos(pub HashMap<String, String>);

impl Repos {
    pub fn name(&self, key: &str) -> String {
        match self.0.get(key) {
            Some(n) if !n.is_empty() => n.clone(),
            _ => repo_short(key),
        }
    }
}

/// Why Irori holds a message, in plain words (card #488).
pub fn hold_text(d: &Delivery, names: &Names, repos: &Repos) -> String {
    let h = if d.hold.is_empty() {
        d.held_for.as_str()
    } else {
        d.hold.as_str()
    };
    let named = |key: &str| {
        if key.is_empty() {
            String::new()
        } else {
            let n = names.name(key);
            if n == UNKNOWN_AGENT { String::new() } else { n }
        }
    };
    let from = if d.message.from.key.starts_with("user:") {
        "you".to_string()
    } else {
        let n = named(&d.message.from.key);
        if !n.is_empty() {
            n
        } else if !d.message.from.name.is_empty() {
            d.message.from.name.clone()
        } else {
            "an agent".into()
        }
    };
    let lead = if from == "you" {
        "You".to_string()
    } else {
        from.clone()
    };
    let to = if d.to.starts_with("user:") {
        "you".to_string()
    } else {
        let n = named(&d.to);
        if n.is_empty() {
            "the other agent".into()
        } else {
            n
        }
    };
    let repo = if d.repo_key.is_empty() {
        "its repo".to_string()
    } else {
        repos.name(&d.repo_key)
    };
    match h {
        hold::BLOCKED => format!("{lead} is on your block list."),
        hold::NOT_ALLOWED => format!("{lead} isn't on your allow list."),
        hold::BYPASS_RECEIVER => {
            "An older Irori held it, by a rule that's gone now. Release delivers it.".into()
        }
        hold::HOP_LIMIT => {
            let n = d.message.hop;
            if n > 1 {
                format!("{lead} and {to} have messaged each other {n} times without stopping.")
            } else {
                format!("{lead} and {to} kept messaging each other without stopping.")
            }
        }
        hold::UNOWNED => {
            if from == "you" {
                format!("No agent is working in {repo} to take your message.")
            } else {
                format!("No agent is working in {repo} to take it yet.")
            }
        }
        hold::PAUSED => "You paused wakes.".into(),
        hold::MEMBER_WAKE => "Another person asked to wake your agent, and your receive setting for that space asks you first.".into(),
        hold::CROSS_PERSON => "It comes from another person in a shared space, and your receive setting for that space asks you first.".into(),
        hold::MEMBER_LEASE => "Another person asks your agent to give up a lease.".into(),
        hold::MEMBER_REMOVED => "It comes from a person who is no longer in that space.".into(),
        hold::EDITING => "You are editing it.".into(),
        hold::OFF_SIDE => {
            let whose = if to == "you" { "your".to_string() } else { format!("{to}'s") };
            format!("Irori picked {to} for a card on a side {whose} device doesn't run. Release delivers it.")
        }
        "" => String::new(),
        _ => "Irori held it.".into(),
    }
}

/// Allow sender lifts this hold: the sender isn't on the allow list.
pub fn allow_lifts(d: &Delivery) -> bool {
    d.state == st::HELD && d.hold == hold::NOT_ALLOWED
}

/// What Start an agent did with the person's held message (card #488).
pub fn started_line(s: &StartedAgent) -> String {
    if s.started {
        format!(
            "Starting an agent in {} on {}, with your message.",
            s.repo, s.card_ref
        )
    } else if s.offered {
        format!(
            "Your message is card {} in {}, handed to a free agent.",
            s.card_ref, s.repo
        )
    } else {
        format!(
            "Your message is card {} in {}; it waits for a free agent.",
            s.card_ref, s.repo
        )
    }
}

/// Who a reply's mentions didn't reach, "" when every one woke.
pub fn not_delivered(outcomes: &[MentionOutcome]) -> String {
    let parts: Vec<String> = outcomes
        .iter()
        .filter(|o| o.state != "woke")
        .map(|o| match o.state.as_str() {
            "gone" => format!("{} is gone", o.name),
            "unknown" => format!("no agent named {}", o.name),
            "held" if !o.note.is_empty() => format!("{} is held ({})", o.name, o.note),
            "held" => format!("{} is held", o.name),
            _ if !o.note.is_empty() => format!("{}: {}", o.name, o.note),
            _ => format!("{} failed", o.name),
        })
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("Not delivered: {}.", parts.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_line_and_block() {
        assert_eq!(plain_line("  a\u{200b}b \n c\t"), "a b c");
        assert_eq!(plain_block("a\r\n\n\n\n\nb\u{7}"), "a\n\n\nb");
    }

    #[test]
    fn relative_times() {
        let now = 1_000_000_000;
        assert_eq!(relative_time(None, now), "never");
        assert_eq!(relative_time(Some(now - 2_000), now), "just now");
        assert_eq!(relative_time(Some(now - 30_000), now), "30s ago");
        assert_eq!(relative_time(Some(now - 5 * 60_000), now), "5m ago");
        assert_eq!(relative_time(Some(now - 3 * 3_600_000), now), "3h ago");
        assert_eq!(relative_time(Some(now - 50 * 3_600_000), now), "2d ago");
    }

    #[test]
    fn keys_are_never_names() {
        assert!(is_key_like("board:abc123/task-1"));
        assert!(is_key_like("agent:abcdef12"));
        assert!(is_key_like("0123456789abcdef/0123abcd-ef"));
        assert!(!is_key_like("otter"));
        assert!(!is_key_like("agent:abc"));
        assert_eq!(name_or("agent:abcdef12", UNKNOWN_AGENT), UNKNOWN_AGENT);
        let mut n = Names::default();
        n.add("dev/s1", "otter");
        n.add("dev/s1", "badger");
        assert_eq!(n.name("dev/s1"), "otter");
        assert_eq!(n.name("user:pc"), "you");
        assert_eq!(n.name("dev/s2"), UNKNOWN_AGENT);
    }

    #[test]
    fn repo_short_names() {
        assert_eq!(
            repo_short("remote:github.com/o/ww-multiplayer.git"),
            "ww-multiplayer"
        );
        assert_eq!(repo_short("root:5a3c1e0b9d7f"), "local repo 5a3c1e0");
        assert_eq!(repo_short("plain"), "plain");
    }

    fn held(hold: &str, from: &str) -> Delivery {
        let mut d = Delivery {
            state: st::HELD.into(),
            hold: hold.into(),
            to: "dev/s2".into(),
            repo_key: "remote:github.com/o/irori".into(),
            ..Delivery::default()
        };
        d.message.from.key = from.into();
        d.message.from.name = "otter".into();
        d
    }

    #[test]
    fn hold_words() {
        let names = Names::default();
        let repos = Repos::default();
        assert_eq!(
            hold_text(&held(hold::BLOCKED, "dev/s1"), &names, &repos),
            "otter is on your block list."
        );
        assert_eq!(
            hold_text(&held(hold::UNOWNED, "user:pc"), &names, &repos),
            "No agent is working in irori to take your message."
        );
        assert_eq!(
            hold_text(&held(hold::HOP_LIMIT, "dev/s1"), &names, &repos),
            "otter and the other agent kept messaging each other without stopping."
        );
        assert_eq!(hold_text(&held("", "dev/s1"), &names, &repos), "");
        assert_eq!(
            hold_text(&held("weird", "dev/s1"), &names, &repos),
            "Irori held it."
        );
        assert!(allow_lifts(&held(hold::NOT_ALLOWED, "dev/s1")));
        assert!(!allow_lifts(&held(hold::BLOCKED, "dev/s1")));
    }

    #[test]
    fn started_and_missed_lines() {
        let s = StartedAgent {
            card_ref: "#12".into(),
            repo: "irori".into(),
            started: true,
            offered: false,
        };
        assert_eq!(
            started_line(&s),
            "Starting an agent in irori on #12, with your message."
        );
        let o = |name: &str, state: &str, note: &str| MentionOutcome {
            key: String::new(),
            name: name.into(),
            state: state.into(),
            note: note.into(),
        };
        assert_eq!(not_delivered(&[o("otter", "woke", "")]), "");
        assert_eq!(
            not_delivered(&[o("otter", "gone", ""), o("heron", "held", "paused")]),
            "Not delivered: otter is gone; heron is held (paused)."
        );
    }
}
