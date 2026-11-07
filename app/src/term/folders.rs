//! Where a card's shell starts (the React app's lib/term.ts:
//! `rememberedFolder`, `rememberFolder`, `pickFolder`) and a tab's name
//! (terminal-section.tsx `tabName`). The remembered folder stays on this
//! device, in the window's prefs: a path means nothing on another one.

use irori_window::rune_term::TermInfo;
use irori_window::terms::Folder;

use crate::ui::prefs::Prefs;

fn key(card: &str) -> String {
    format!("term.folder.{card}")
}

pub fn remembered(p: &Prefs, card: &str) -> Option<String> {
    p.get::<String>(&key(card))
}

pub fn remember(p: &mut Prefs, card: &str, path: &str) {
    p.set(&key(card), path);
}

/// The folder to open in with no question, when it can: the remembered
/// one if it is still offered, else the only one offered. None: the
/// person has to choose (several, or none).
pub fn pick(kept: Option<&str>, offered: &[Folder]) -> Option<String> {
    if let Some(k) = kept
        && offered.iter().any(|f| f.path == k)
    {
        return Some(k.to_string());
    }
    if offered.len() == 1 {
        return Some(offered[0].path.clone());
    }
    None
}

/// A tab's name: its folder's last part, numbered after the first.
pub fn tab_name(t: &TermInfo, i: usize) -> String {
    let base = t
        .cwd
        .split(['/', '\\'])
        .rfind(|s| !s.is_empty())
        .unwrap_or("shell");
    if i == 0 {
        base.to_string()
    } else {
        format!("{base} {}", i + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use irori_window::rune_term::Scope;

    fn f(p: &str) -> Folder {
        Folder {
            path: p.into(),
            why: "repo".into(),
        }
    }

    #[test]
    fn pick_prefers_the_remembered_folder_while_offered() {
        let offered = [f("/a"), f("/b")];
        assert_eq!(pick(Some("/b"), &offered).as_deref(), Some("/b"));
        // Gone from the offer: ask.
        assert_eq!(pick(Some("/c"), &offered), None);
        assert_eq!(pick(None, &offered), None);
        assert_eq!(pick(Some("/c"), &[f("/a")]).as_deref(), Some("/a"));
        assert_eq!(pick(None, &[]), None);
    }

    #[test]
    fn remember_round_trips_per_card() {
        let d = tempfile::tempdir().unwrap();
        let mut p = Prefs::load(d.path());
        remember(&mut p, "abc", "/x");
        assert_eq!(remembered(&p, "abc").as_deref(), Some("/x"));
        assert_eq!(remembered(&p, "def"), None);
    }

    #[test]
    fn tab_names() {
        let t = |cwd: &str| TermInfo {
            id: 1,
            scope: Scope::Card("c".into()),
            cwd: cwd.into(),
            title: String::new(),
            running: true,
        };
        assert_eq!(tab_name(&t("/Users/me/irori"), 0), "irori");
        assert_eq!(tab_name(&t("/Users/me/irori/"), 1), "irori 2");
        assert_eq!(tab_name(&t("C:\\src\\app"), 2), "app 3");
        assert_eq!(tab_name(&t(""), 0), "shell");
    }
}
