//! What a card waits on that isn't the person, in plain words (the React
//! app's lib/wait-name.ts, card #487, and bits.tsx's waitLabel/waitTip,
//! cards #158, #553): "a free Dolphin slot" for ww/dolphin-slot-1..4, not
//! the raw lease names.

use std::collections::HashMap;
use std::sync::LazyLock;

use irori_window::cards::CardSummary;
use regex::Regex;

use super::plain::plain_line;

/// A lease family member: a name ending in -<n>, -<lo>..<hi>, -N, -n or -*.
static MEMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([a-z0-9][a-z0-9._-]*?(?:/[a-z0-9._-]+)*?)-(\d+(?:\.\.\d+)?|[nN*])$")
        .expect("member")
});
/// The same inside free text, ending where the token ends (JS's
/// `(?![a-z0-9._/-])`, which Rust's regex has no lookahead for: the
/// third group takes the next character, or the end).
static IN_TEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b([a-z0-9][a-z0-9._-]*(?:/[a-z0-9._-]+)+)-(\d+\.\.\d+|\d+|[nN*])(?:[^a-z0-9._/-]|$)",
    )
    .expect("in text")
});
static CHECK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^check/([^/]+)/slot$").expect("check"));
static LEASE_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\blease").expect("lease"));
static PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^path:(.+):([^:]+)$").expect("path"));
static SEPS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[-_.]+").expect("seps"));

/// A lease family named in free text: its stem and its number part.
fn lease_in(text: &str) -> Option<(String, String)> {
    let m = IN_TEXT.captures(text)?;
    let n = m[2].to_string();
    if n.chars().all(|c| c.is_ascii_digit()) && !LEASE_WORD.is_match(text) {
        return None;
    }
    Some((m[1].to_string(), n))
}

fn words(seg: &str) -> String {
    let w = SEPS.replace_all(seg, " ").trim().to_string();
    let mut ch = w.chars();
    match ch.next() {
        Some(f) => f.to_uppercase().collect::<String>() + ch.as_str(),
        None => seg.to_string(),
    }
}

/// A lease family's stem in words: Irori's check slots by what they are,
/// anything else by its last path part.
fn family(stem: &str) -> String {
    if let Some(c) = CHECK.captures(stem) {
        return format!("check slot on {}", &c[1]);
    }
    words(stem.rsplit('/').next().unwrap_or(stem))
}

/// A repo key as people say it: "irori" for remote:github.com/sh/irori.
fn repo_name(key: &str) -> String {
    if let Some(r) = key.strip_prefix("remote:") {
        return r
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or(key)
            .to_string();
    }
    if let Some(r) = key.strip_prefix("root:") {
        return format!("repo {}", r.chars().take(7).collect::<String>());
    }
    key.to_string()
}

/// One numbered member of a family: "Dolphin slot 2".
fn member(stem: &str, n: &str) -> String {
    if let Some(c) = CHECK.captures(stem) {
        return format!("check slot {n} on {}", &c[1]);
    }
    format!("{} {n}", family(stem))
}

/// One lease by name.
fn one(name: &str) -> String {
    if name.starts_with("desk/") {
        return "the merge desk".into();
    }
    if let Some(p) = name
        .strip_prefix("port:")
        .filter(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    {
        return format!("port {p}");
    }
    if let Some(c) = PATH.captures(name) {
        return format!("{} in {}", &c[2], repo_name(&c[1]));
    }
    if let Some(m) = MEMBER.captures(name) {
        if m[2].chars().all(|c| c.is_ascii_digit()) {
            return member(&m[1], &m[2]);
        }
        return format!("a free {}", family(&m[1]));
    }
    name.to_string()
}

/// The leases a wait is on: several of one family (or a range) are "a free
/// <family>", one is itself; others are counted.
pub fn lease_names(names: &[&str]) -> String {
    let Some(first) = names.first() else {
        return String::new();
    };
    let mut stems: Vec<String> = Vec::new();
    let mut count: HashMap<String, usize> = HashMap::new();
    for n in names {
        let key = MEMBER
            .captures(n)
            .map_or_else(|| n.to_string(), |m| m[1].to_string());
        if !count.contains_key(&key) {
            stems.push(key.clone());
        }
        *count.entry(key).or_default() += 1;
    }
    let said = match MEMBER.captures(first) {
        Some(m) if count.get(&m[1]).copied().unwrap_or(0) > 1 => {
            format!("a free {}", family(&m[1]))
        }
        _ => one(first),
    };
    let others = stems.len() - 1;
    if others > 0 {
        format!("{said} and {others} more")
    } else {
        said
    }
}

/// The thing a card waits on, without "Waiting for": "" when it names
/// nothing.
pub fn wait_name(kind: &str, on: &str) -> String {
    let on = plain_line(on);
    if on.is_empty() {
        return String::new();
    }
    if kind == "lease" {
        let names: Vec<&str> = on.split_whitespace().collect();
        return lease_names(&names);
    }
    if !kind.is_empty() && kind != "external" {
        return on;
    }
    if let Some((stem, n)) = lease_in(&on) {
        let f = stem.to_lowercase();
        return if n.contains("..") || matches!(n.as_str(), "n" | "N" | "*") {
            format!("a free {}", family(&f))
        } else {
            member(&f, &n)
        };
    }
    let lower = on.to_lowercase();
    match lower.strip_prefix("external") {
        Some(rest) if rest.starts_with(char::is_whitespace) => {
            on["external".len()..].trim_start().to_string()
        }
        _ => on,
    }
}

/// The row's words: "Waiting for a free Dolphin slot", "Waiting on
/// kestrel", "Waiting on #12".
pub fn wait_line(kind: &str, on: &str) -> String {
    let n = wait_name(kind, on);
    if n.is_empty() {
        return "Waiting on something else".into();
    }
    let lease = kind == "lease"
        || (kind != "agent" && kind != "task" && lease_in(&plain_line(on)).is_some());
    if lease {
        format!("Waiting for {n}")
    } else {
        format!("Waiting on {n}")
    }
}

/// What a card's wait fields hold, for [`wait_label`] and [`wait_tip`].
#[derive(Clone, Debug, Default)]
pub struct Wait {
    pub waiting_on: i64,
    pub on: String,
    pub full: String,
    pub kind: String,
    pub question: String,
}

impl From<&CardSummary> for Wait {
    fn from(c: &CardSummary) -> Wait {
        Wait {
            waiting_on: c.waiting_on,
            on: c.wait_on.clone(),
            full: c.wait_full.clone(),
            kind: c.wait_kind.clone(),
            question: c.wait_question.clone(),
        }
    }
}

/// The state pill's words for a wait, "+N" for more (card #158).
pub fn wait_label(w: &Wait) -> String {
    let on = if w.on.is_empty() {
        "Waiting".to_string()
    } else {
        wait_line(&w.kind, &w.on)
    };
    if w.waiting_on > 1 {
        format!("{on} +{}", w.waiting_on - 1)
    } else {
        on
    }
}

/// The pill's tooltip: the wait in full (a version wait's devices, card
/// #553) and the question it was raised with.
pub fn wait_tip(w: &Wait) -> String {
    let label = wait_label(&Wait {
        on: if w.full.is_empty() {
            w.on.clone()
        } else {
            w.full.clone()
        },
        ..w.clone()
    });
    if w.question.is_empty() {
        label
    } else {
        format!("{label}: {}", plain_line(&w.question))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lease_family_is_a_free_one_of_it() {
        assert_eq!(
            lease_names(&[
                "ww/dolphin-slot-1",
                "ww/dolphin-slot-2",
                "ww/dolphin-slot-3",
                "ww/dolphin-slot-4"
            ]),
            "a free Dolphin slot"
        );
        assert_eq!(
            lease_names(&["check/macbook/slot-1", "check/macbook/slot-2"]),
            "a free check slot on macbook"
        );
    }

    #[test]
    fn one_lease_is_itself() {
        assert_eq!(lease_names(&["ww/dolphin-slot-2"]), "Dolphin slot 2");
        assert_eq!(lease_names(&["gpu/qwen"]), "gpu/qwen");
        assert_eq!(lease_names(&["port:8080"]), "port 8080");
        assert_eq!(lease_names(&["desk/irori"]), "the merge desk");
        assert_eq!(
            lease_names(&["path:remote:github.com/sh/irori:crates/coord/**"]),
            "crates/coord/** in irori"
        );
        assert_eq!(
            lease_names(&["path:root:abc1234def:src/*.rs"]),
            "src/*.rs in repo abc1234"
        );
        assert_eq!(lease_names(&["check/mac/slot-3"]), "check slot 3 on mac");
    }

    #[test]
    fn free_text_that_isnt_a_lease_keeps_its_words() {
        assert_eq!(
            wait_line("external", "external deploy to aws/us-east-1 finishing"),
            "Waiting on deploy to aws/us-east-1 finishing"
        );
        assert_eq!(
            wait_line(
                "external",
                "external CI at https://ci.example.com/job/build-42"
            ),
            "Waiting on CI at https://ci.example.com/job/build-42"
        );
        assert_eq!(
            wait_line("external", "the ww/dolphin-slot-2 lease"),
            "Waiting for Dolphin slot 2"
        );
    }

    #[test]
    fn two_families_name_the_first_and_count_the_rest() {
        assert_eq!(
            lease_names(&["ww/dolphin-slot-1", "ww/dolphin-slot-2", "port:9246"]),
            "a free Dolphin slot and 1 more"
        );
    }

    #[test]
    fn an_external_wait_naming_a_lease_family_reads_the_same() {
        let on = "external ww/dolphin-slot-1..4 leases (reed until the netem run ends)";
        assert_eq!(wait_name("external", on), "a free Dolphin slot");
        assert_eq!(wait_name("", on), "a free Dolphin slot");
        assert_eq!(wait_line("external", on), "Waiting for a free Dolphin slot");
    }

    #[test]
    fn anything_else_keeps_its_words() {
        assert_eq!(wait_line("agent", "kestrel"), "Waiting on kestrel");
        assert_eq!(wait_line("task", "#12"), "Waiting on #12");
        assert_eq!(
            wait_line("external", "external CI on main"),
            "Waiting on CI on main"
        );
        assert_eq!(
            wait_line("lease", "ww/dolphin-slot-1 ww/dolphin-slot-2"),
            "Waiting for a free Dolphin slot"
        );
        assert_eq!(wait_line("", ""), "Waiting on something else");
    }

    #[test]
    fn a_version_wait() {
        let card = Wait {
            waiting_on: 1,
            kind: "version".into(),
            on: "Irori 0.12.4".into(),
            full: "Irori 0.12.4 on Desktop and another device".into(),
            question: String::new(),
        };
        assert_eq!(wait_label(&card), "Waiting on Irori 0.12.4");
        assert_eq!(
            wait_label(&Wait {
                waiting_on: 2,
                ..card.clone()
            }),
            "Waiting on Irori 0.12.4 +1"
        );
        assert_eq!(
            wait_tip(&card),
            "Waiting on Irori 0.12.4 on Desktop and another device"
        );
        assert_eq!(
            wait_tip(&Wait {
                question: "After the update".into(),
                ..card.clone()
            }),
            "Waiting on Irori 0.12.4 on Desktop and another device: After the update"
        );
        assert_eq!(
            wait_tip(&Wait {
                waiting_on: 1,
                kind: "agent".into(),
                on: "kestrel".into(),
                question: "Soon".into(),
                ..Wait::default()
            }),
            "Waiting on kestrel: Soon"
        );
    }
}
