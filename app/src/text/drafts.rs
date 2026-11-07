//! Drafts (the React app's lib/drafts.ts): what the person has typed in a
//! box (a comment, a blocker answer, a resolve note, a Talk post, the ask
//! box) kept outside the box, so leaving the page and coming back finds
//! the text and the mentions where they were. Keys are "<card id>/<field>"
//! or the like. Kept in memory as the person types and written to the
//! window's prefs (`text.drafts`) on the next event or send, never per
//! keystroke, so a restart finds them too.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::mentions::Mention;
use crate::ui::State;
use crate::ui::prefs::Prefs;

/// The prefs key the drafts live under.
pub const PREF: &str = "text.drafts";

/// One box's draft: its text and the mentions picked in it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<Mention>,
}

#[derive(Default)]
pub struct Drafts {
    map: BTreeMap<String, Draft>,
    loaded: bool,
    dirty: bool,
}

impl Drafts {
    fn load(&mut self, p: &Prefs) {
        if !self.loaded {
            self.map = p.get(PREF).unwrap_or_default();
            self.loaded = true;
        }
    }

    /// The kept draft for `key`, empty when none.
    pub fn get(&self, key: &str) -> Draft {
        self.map.get(key).cloned().unwrap_or_default()
    }

    /// Keeps `text` (and its mentions) for `key`; empty text drops it.
    pub fn set(&mut self, key: &str, text: &str, mentions: &[Mention]) {
        if key.is_empty() {
            return;
        }
        let d = Draft {
            text: text.to_string(),
            mentions: mentions.to_vec(),
        };
        if text.is_empty() {
            self.dirty |= self.map.remove(key).is_some();
        } else if self.map.get(key) != Some(&d) {
            self.map.insert(key.to_string(), d);
            self.dirty = true;
        }
    }
}

/// The kept draft for `key` (lib/drafts.ts `draft`).
pub fn draft(s: &mut State, key: &str) -> Draft {
    s.text.drafts.load(&s.prefs);
    s.text.drafts.get(key)
}

/// Keeps a draft (lib/drafts.ts `setDraft`): in memory now, in the prefs on
/// the next [`flush`].
pub fn set_draft(s: &mut State, key: &str, text: &str, mentions: &[Mention]) {
    s.text.drafts.load(&s.prefs);
    s.text.drafts.set(key, text, mentions);
}

/// Writes changed drafts to the prefs.
pub fn flush(s: &mut State) {
    if !s.text.drafts.dirty {
        return;
    }
    s.text.drafts.dirty = false;
    if s.text.drafts.map.is_empty() {
        s.prefs.remove(PREF);
    } else {
        let m = s.text.drafts.map.clone();
        s.prefs.set(PREF, m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draft_is_kept_until_it_is_cleared() {
        let mut d = Drafts::default();
        assert_eq!(d.get("c1/comment").text, "");
        d.set("c1/comment", "half a thought", &[]);
        assert_eq!(d.get("c1/comment").text, "half a thought");
        assert_eq!(d.get("c2/comment").text, "");
        d.set("c1/comment", "", &[]);
        assert_eq!(d.get("c1/comment").text, "");
    }

    #[test]
    fn drafts_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = State {
            prefs: Prefs::load(dir.path()),
            ..State::default()
        };
        let m = Mention {
            key: "agent:oak".into(),
            name: "oak".into(),
            gone: false,
        };
        set_draft(&mut s, "c1/comment", "@oak look", std::slice::from_ref(&m));
        flush(&mut s);
        let mut t = State {
            prefs: Prefs::load(dir.path()),
            ..State::default()
        };
        let d = draft(&mut t, "c1/comment");
        assert_eq!((d.text.as_str(), d.mentions), ("@oak look", vec![m]));
    }
}
