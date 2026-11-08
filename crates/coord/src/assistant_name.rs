//! The name a person gives their Office Assistant (card #1045,
//! docs/design/call-agents.md §5). A private `assistant.name` row, written
//! only by the person in the app, carries it; every space log learns it from
//! the `assistantName` field on their member.profile. With no row the name
//! is computed: "<profile name>'s assistant". Nothing is written for the
//! default, so it follows a rename of the person.

use std::sync::Mutex;

use crate::gojson::decode;

/// The private row a pick writes: `{name, by, at}`.
pub const KIND: &str = store::kinds::ASSISTANT_NAME;

/// The longest chosen name, in characters.
pub const MAX_CHARS: usize = 24;

/// Whether `n` can be a chosen name: "" (back to the default), or 1 to
/// [`MAX_CHARS`] letters and spaces, not starting or ending with a space.
pub fn valid(n: &str) -> bool {
    n.is_empty()
        || (n.chars().count() <= MAX_CHARS
            && n == n.trim()
            && n.chars().any(char::is_alphabetic)
            && n.chars().all(|c| c.is_alphabetic() || c == ' '))
}

/// The default name for a person called `person`: "Addam's assistant".
/// `person` is the profile name, else the membership name; "" reads as
/// "Your assistant".
pub fn default_name(person: &str) -> String {
    let p = person.trim();
    if p.is_empty() {
        "Your assistant".into()
    } else {
        format!("{p}'s assistant")
    }
}

/// The one label every place that shows another person's assistant uses:
/// "Juno · Addam's assistant" for a chosen name, "Addam's assistant" for the
/// default. `person` comes from the signer's member.profile, never from the
/// row's text, so a chosen name can't pose as a person. A chosen name that
/// isn't valid (a row from a build that checks less) reads as the default.
pub fn label(person: &str, chosen: &str) -> String {
    let default = default_name(person);
    let c = chosen.trim();
    if c.is_empty() || !valid(c) {
        default
    } else {
        format!("{c} · {default}")
    }
}

crate::go_struct! {
    #[derive(Debug, Clone, Default)]
    struct Row {
        name: String = "name",
    }
}

#[derive(Debug, Clone)]
struct Pick {
    name: String,
    hlc: u64,
    device: String,
    seq: i64,
}

fn newer(a: &Pick, b: &Pick) -> bool {
    (a.hlc, &a.device, a.seq) > (b.hlc, &b.device, b.seq)
}

/// Folds `assistant.name` rows: the person's newest pick. Safe for
/// concurrent use.
#[derive(Debug, Default)]
pub struct AssistantName {
    m: Mutex<Option<Pick>>,
}

impl AssistantName {
    /// An empty fold.
    pub fn new() -> AssistantName {
        AssistantName::default()
    }

    /// Folds one row; reports whether the pick changed. Only the person
    /// sets it (actor `user`).
    pub fn apply(&self, ev: &store::Event) -> bool {
        if ev.kind != KIND || ev.actor != "user" {
            return false;
        }
        let Some(r) = decode::<Row>(&ev.payload) else {
            return false;
        };
        if !valid(&r.name) {
            return false;
        }
        let n = Pick {
            name: r.name,
            hlc: ev.hlc.0,
            device: ev.device_id.clone(),
            seq: ev.seq,
        };
        let mut m = self.m.lock().unwrap_or_else(|e| e.into_inner());
        if m.as_ref().is_some_and(|cur| !newer(&n, cur)) {
            return false;
        }
        *m = Some(n);
        true
    }

    /// The chosen name, "" for none (the computed default then applies).
    pub fn chosen(&self) -> String {
        let m = self.m.lock().unwrap_or_else(|e| e.into_inner());
        m.as_ref().map(|p| p.name.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_letters_and_spaces() {
        assert!(valid(""));
        assert!(valid("Juno"));
        assert!(valid("Mary Ann"));
        assert!(valid("Zoë"));
        assert!(valid(&"a".repeat(24)));
        assert!(!valid(&"a".repeat(25)));
        assert!(!valid(" Juno"));
        assert!(!valid("Juno "));
        assert!(!valid("Juno2"));
        assert!(!valid("Juno · Bob's assistant"));
        assert!(!valid("   "));
    }

    #[test]
    fn default_and_label() {
        assert_eq!(default_name("Addam"), "Addam's assistant");
        assert_eq!(default_name(" "), "Your assistant");
        assert_eq!(label("Addam", ""), "Addam's assistant");
        assert_eq!(label("Addam", "Juno"), "Juno · Addam's assistant");
        // A chosen name can't carry a person's name of its own.
        assert_eq!(
            label("Addam", "x · Stephen's assistant"),
            "Addam's assistant"
        );
    }
}
