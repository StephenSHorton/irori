//! What a person reads where an agent or person is named (the React app's
//! lib/names.ts and cards.ts `displayName`, card #478): a key
//! (`board:<board>/<card>`, `agent:<card>`, `person:<id>`,
//! `<device>/<session>`) is never a name.

use std::sync::LazyLock;

use irori_window::cards::AgentRef;
use regex::Regex;

use super::plain::plain_line;

pub const UNKNOWN_AGENT: &str = "unknown agent";
pub const UNKNOWN_PERSON: &str = "another person";

/// An actor key, or a bare hex id from one, anywhere in a string.
static RAW_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b(?:board:[0-9A-Za-z]+/\S+|agent:[0-9A-Za-z]{6,}|person:[0-9A-Za-z]{6,}|[0-9a-f]{16,}/[0-9a-f-]{8,})",
    )
    .expect("raw key pattern")
});

/// Whether `s` is (or holds) a key rather than a name.
pub fn is_key_like(s: &str) -> bool {
    RAW_KEY.is_match(s)
}

/// `name` for a person to read, or `fallback` when it is empty or a key.
pub fn name_or(name: &str, fallback: &str) -> String {
    let n = plain_line(name);
    if !n.is_empty() && !is_key_like(&n) {
        n
    } else {
        fallback.to_string()
    }
}

/// A Talk member's name: a person nothing names is "another person", an
/// agent "unknown agent".
pub fn member_name(kind: &str, name: &str) -> String {
    name_or(
        name,
        if kind == "person" {
            UNKNOWN_PERSON
        } else {
            UNKNOWN_AGENT
        },
    )
}

/// Whether `key` is another member's agent or person on a shared board:
/// this device's roster never holds it, so missing from it isn't gone.
pub fn is_others_key(key: &str) -> bool {
    key.starts_with("board:") || key.starts_with("agent:") || key.starts_with("person:")
}

/// Who a face or line names: the roster name, else "unknown agent". Never
/// a key or a piece of one.
pub fn display_name(r: &AgentRef) -> String {
    let n = plain_line(&r.name);
    if n.is_empty() || n == r.key {
        return UNKNOWN_AGENT.into();
    }
    name_or(&n, UNKNOWN_AGENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_never_names() {
        assert!(is_key_like("board:abc123/card-1"));
        assert!(is_key_like("agent:abcdef12"));
        assert!(is_key_like("0123456789abcdef0123/0123abcd-ef"));
        assert!(!is_key_like("sample-heron"));
        assert_eq!(name_or("agent:abcdef12", UNKNOWN_AGENT), UNKNOWN_AGENT);
        assert_eq!(name_or("  heron ", UNKNOWN_AGENT), "heron");
        assert_eq!(member_name("person", ""), UNKNOWN_PERSON);
        assert_eq!(member_name("agent", ""), UNKNOWN_AGENT);
        assert!(is_others_key("person:x") && !is_others_key("dev/sess"));
    }

    #[test]
    fn display_name_falls_back() {
        let r = |key: &str, name: &str| AgentRef {
            key: key.into(),
            name: name.into(),
            ..Default::default()
        };
        assert_eq!(display_name(&r("k", "heron")), "heron");
        assert_eq!(display_name(&r("k", "")), UNKNOWN_AGENT);
        assert_eq!(display_name(&r("dev/s", "dev/s")), UNKNOWN_AGENT);
    }
}
