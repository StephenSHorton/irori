//! Held messages in plain words (the React app's lib/talk.ts holdText,
//! startedLine, repoShort, allowLifts and nameLookup, card #488): why a
//! message was held, naming who is involved, never an internal reason.

use std::collections::HashMap;

use inbox::{Delivery, hold, st};
use irori_window::roster::RosterAgent;
use irori_window::ts::{RepoName, StartedAgent};

/// What a hold's sentence names: agents by key, repos by key.
pub struct Words<'a> {
    pub name: &'a dyn Fn(&str) -> String,
    pub repo: &'a dyn Fn(&str) -> String,
}

/// Why `d` was (or is) held, naming who is involved: "otter and heron have
/// messaged each other 8 times without stopping". An unknown reason reads
/// as "Irori held it." rather than its code.
pub fn hold_text(d: &Delivery, w: &Words) -> String {
    let h = if d.hold.is_empty() { d.held_for.as_str() } else { d.hold.as_str() };
    let named = |key: &str| if key.is_empty() { String::new() } else { (w.name)(key) };
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
    // An agent's name starts the sentence as it is: it's a handle.
    let lead = if from == "you" { "You".to_string() } else { from.clone() };
    let to = if d.to.starts_with("user:") {
        "you".to_string()
    } else {
        let n = named(&d.to);
        if n.is_empty() { "the other agent".into() } else { n }
    };
    let repo = if d.repo_key.is_empty() { "its repo".to_string() } else { (w.repo)(&d.repo_key) };
    match h {
        hold::BLOCKED => format!("{lead} is on your block list."),
        hold::NOT_ALLOWED => format!("{lead} isn't on your allow list."),
        hold::BYPASS_RECEIVER => "An older Irori held it, by a rule that's gone now. Release delivers it.".into(),
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
        hold::MEMBER_WAKE => {
            "Another person asked to wake your agent, and your receive setting for that space asks you first.".into()
        }
        hold::CROSS_PERSON => {
            "It comes from another person in a shared space, and your receive setting for that space asks you first."
                .into()
        }
        hold::MEMBER_LEASE => "Another person asks your agent to give up a lease.".into(),
        "member_removed" => "It comes from a person who is no longer in that space.".into(),
        "editing" => "You are editing it.".into(),
        "off_side" => format!(
            "Irori picked {to} for a card on a side {} device doesn't run. Release delivers it.",
            if to == "you" { "your".to_string() } else { format!("{to}'s") }
        ),
        "" => String::new(),
        _ => "Irori held it.".into(),
    }
}

/// What Start an agent did, in the person's words.
pub fn started_line(s: &StartedAgent) -> String {
    if s.started {
        format!("Starting an agent in {} on {}, with your message.", s.repo, s.card_ref)
    } else if s.offered {
        format!("Your message is card {} in {}, handed to a free agent.", s.card_ref, s.repo)
    } else {
        format!("Your message is card {} in {}; it waits for a free agent.", s.card_ref, s.repo)
    }
}

/// A repo key as a person reads it when no checkout here names it:
/// "example-app" for a hosted repo, "local repo 5a3c1e0" for a root key.
pub fn repo_short(key: &str) -> String {
    if let Some(sha) = key.strip_prefix("root:") {
        return format!("local repo {}", sha.chars().take(7).collect::<String>());
    }
    let k = key.trim_end_matches('/');
    let k = k.strip_suffix(".git").unwrap_or(k);
    match k.rfind(['/', ':', '\\']) {
        Some(i) if i < k.len() - 1 => k[i + 1..].to_string(),
        _ => k.to_string(),
    }
}

/// A repo's name: the checkout's, else [`repo_short`].
pub fn repo_name(names: &[RepoName], key: &str) -> String {
    names
        .iter()
        .find(|r| r.repo_key == key)
        .map(|r| r.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| repo_short(key))
}

/// Allow sender is offered: allowing them lifts the hold.
pub fn allow_lifts(d: &Delivery) -> bool {
    d.state == st::HELD && d.hold == hold::NOT_ALLOWED
}

/// Agent keys to labels: "you" for the person, the live roster's name,
/// else a name seen on a message, else "unknown agent", never a key.
pub fn name_lookup(agents: &[RosterAgent], held: &[Delivery]) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for d in held {
        if !d.message.from.name.is_empty() {
            names.insert(d.message.from.key.clone(), d.message.from.name.clone());
        }
    }
    for a in agents {
        if !a.card.name.is_empty() {
            names.insert(a.card.key.clone(), a.card.name.clone());
        }
    }
    names
}

/// One key's label by [`name_lookup`]'s map.
pub fn name_of(names: &HashMap<String, String>, key: &str) -> String {
    if key.starts_with("user:") {
        return "you".into();
    }
    names.get(key).cloned().unwrap_or_else(|| "unknown agent".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(f: impl FnOnce(&mut Delivery)) -> Delivery {
        let mut d = Delivery {
            state: st::HELD.into(),
            ..Delivery::default()
        };
        d.message.from.key = "dev/otter".into();
        d.message.from.name = "otter".into();
        d.to = "dev/heron".into();
        f(&mut d);
        d
    }

    fn words<'a>(name: &'a dyn Fn(&str) -> String, repo: &'a dyn Fn(&str) -> String) -> Words<'a> {
        Words { name, repo }
    }

    #[test]
    fn holds_name_who_is_involved() {
        let name = |k: &str| if k == "dev/heron" { "heron".to_string() } else { String::new() };
        let repo = |k: &str| repo_short(k);
        let w = words(&name, &repo);
        let loop_ = d(|d| {
            d.hold = hold::HOP_LIMIT.into();
            d.message.hop = 8;
        });
        assert_eq!(hold_text(&loop_, &w), "otter and heron have messaged each other 8 times without stopping.");
        let once = d(|d| d.hold = hold::HOP_LIMIT.into());
        assert_eq!(hold_text(&once, &w), "otter and heron kept messaging each other without stopping.");
        let blocked = d(|d| d.hold = hold::BLOCKED.into());
        assert_eq!(hold_text(&blocked, &w), "otter is on your block list.");
        let mine = d(|d| {
            d.hold = hold::UNOWNED.into();
            d.message.from.key = "user:me".into();
            d.repo_key = "github.com/acme/example-app.git".into();
            d.to = String::new();
        });
        assert_eq!(hold_text(&mine, &w), "No agent is working in example-app to take your message.");
        let theirs = d(|d| d.hold = hold::UNOWNED.into());
        assert_eq!(hold_text(&theirs, &w), "No agent is working in its repo to take it yet.");
        let side = d(|d| {
            d.held_for = "off_side".into();
        });
        assert_eq!(
            hold_text(&side, &w),
            "Irori picked heron for a card on a side heron's device doesn't run. Release delivers it."
        );
        assert_eq!(hold_text(&d(|d| d.hold = "weird".into()), &w), "Irori held it.");
        assert_eq!(hold_text(&d(|_| {}), &w), "");
    }

    #[test]
    fn repo_short_names() {
        assert_eq!(repo_short("github.com/acme/example-app"), "example-app");
        assert_eq!(repo_short("git@github.com:acme/app.git"), "app");
        assert_eq!(repo_short("root:5a3c1e0ffff"), "local repo 5a3c1e0");
        assert_eq!(repo_short("plain"), "plain");
        let names = vec![RepoName {
            repo_key: "root:abc".into(),
            name: "irori".into(),
        }];
        assert_eq!(repo_name(&names, "root:abc"), "irori");
        assert_eq!(repo_name(&names, "root:abcdef12"), "local repo abcdef1");
    }

    #[test]
    fn started_lines() {
        let s = |started, offered| StartedAgent {
            card_ref: "#12".into(),
            repo: "irori".into(),
            started,
            offered,
        };
        assert_eq!(started_line(&s(true, false)), "Starting an agent in irori on #12, with your message.");
        assert_eq!(started_line(&s(false, true)), "Your message is card #12 in irori, handed to a free agent.");
        assert_eq!(started_line(&s(false, false)), "Your message is card #12 in irori; it waits for a free agent.");
    }

    #[test]
    fn allow_lifts_only_not_allowed() {
        assert!(allow_lifts(&d(|d| d.hold = hold::NOT_ALLOWED.into())));
        assert!(!allow_lifts(&d(|d| d.hold = hold::BYPASS_RECEIVER.into())));
        assert!(!allow_lifts(&d(|d| {
            d.hold = hold::NOT_ALLOWED.into();
            d.state = st::DISMISSED.into();
        })));
    }
}
