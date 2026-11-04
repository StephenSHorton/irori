//! When a card can't get an agent, stop and ask once (card #480).
//!
//! Before this, a card whose repo had no checkout on the device that hands
//! it out was asked for again on every sweep: the live log has about 57,800
//! requested/failed pairs each for #447, #451, #452 and #290, all reading
//! "no checkout of remote:github.com/StephenSHorton/irori on 05f1844a…".
//! #467 stopped the asking but left the card silent. Now:
//!
//! - No checkout: after the first failure the card gets one plain blocker,
//!   "No device has a copy of irori. Choose its folder on Desktop
//!   Computer?". The person replies with the folder's path and the next
//!   agent starts there; a reply that isn't a folder here is said so and
//!   asked again. If a copy turns up here (a Where, or a checkout Irori
//!   sees), the blocker resolves itself and the agent starts.
//! - The spawner itself fails [`SPAWN_TRIES`] times in a row: one blocker,
//!   "Irori couldn't start an agent for this card: try again?".
//!
//! Every line here names devices and repos the way the person does: the
//! device's own name and the repo's short name, never a key or an id.

use std::path::Path;

use coord::{BlockedBy, Blocker, BlockerAnswer, BlockerOption, NewBlocker, Task, TaskFilter};

use super::Services;
use crate::logf;

const SWEEPER: &str = "irori";

/// In every "no copy" question: how a sweep finds its own blocker, whatever
/// the device is called today.
pub const CHOOSE_FOLDER: &str = "Choose its folder";

/// The question when the spawner keeps failing.
pub const START_FAILED: &str = "Irori couldn't start an agent for this card: try again?";

/// Its one option.
pub const START_AGAIN: &str = "Try again";

/// How many spawner failures in a row raise [`START_FAILED`]. A missing
/// checkout is asked about after the first: nothing changes between tries.
pub const SPAWN_TRIES: u32 = 3;

/// Whether a failed request's reason is a missing checkout, in the words of
/// any version (before #480 it named the key and the device id).
pub fn no_folder_reason(reason: &str) -> bool {
    reason.starts_with("no checkout of ")
        || reason.starts_with("no copy of ")
        || reason.starts_with("this card names no repo")
}

/// Whether b is one of Irori's "no copy" blockers.
pub fn is_no_copy(b: &Blocker) -> bool {
    b.raised_by == SWEEPER && b.question.contains(CHOOSE_FOLDER)
}

fn is_start_failed(b: &Blocker) -> bool {
    b.raised_by == SWEEPER && b.question == START_FAILED
}

/// The folder a person's reply names: its first line, without the quotes
/// or backticks people put around paths.
fn as_folder(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    line.trim_matches(|c: char| matches!(c, '`' | '"' | '\'' | '“' | '”'))
        .trim()
        .to_string()
}

impl Services {
    /// What each of the person's devices is called, by id: their name for
    /// it (card #184), else its certificate's label, else this device's own
    /// name.
    pub(crate) fn device_labels(&self) -> std::collections::BTreeMap<String, String> {
        let mut out: std::collections::BTreeMap<String, String> = self
            .chain
            .devices()
            .into_iter()
            .map(|d| (d.cert.device_id, d.cert.label))
            .collect();
        let me = &self.device;
        let mine = out.entry(me.id.clone()).or_default();
        if mine.is_empty() {
            *mine = me.name.clone();
        }
        for (id, name) in self.device_names.chosen() {
            if let Some(l) = out.get_mut(&id) {
                *l = name;
            }
        }
        out
    }

    /// This device as the person calls it ("Desktop Computer").
    pub(crate) fn this_device_name(&self) -> String {
        let n = self
            .device_labels()
            .remove(&self.device.id)
            .unwrap_or_default();
        if n.trim().is_empty() {
            "this device".into()
        } else {
            n
        }
    }

    /// A checkout of t's repo that is a folder on this device. Checkouts
    /// replicate, so one seen on another device is skipped.
    pub(crate) fn checkout_here(&self, repo_key: &str) -> Option<String> {
        if repo_key.is_empty() {
            return None;
        }
        self.capture
            .repo_roots(repo_key)
            .into_iter()
            .find(|r| Path::new(r).is_dir())
    }

    /// The folder the person named in reply to a "no copy" blocker on t,
    /// newest first, when it is a folder here.
    pub(crate) fn chosen_folder(&self, t: &Task) -> Option<String> {
        let ids: Vec<&str> = t
            .blockers
            .iter()
            .filter(|b| is_no_copy(b))
            .map(|b| b.id.as_str())
            .collect();
        if ids.is_empty() {
            return None;
        }
        let mut said: Vec<(i64, String)> = Vec::new();
        for b in t.blockers.iter().filter(|b| is_no_copy(b)) {
            if let Some(r) = &b.resolved
                && self.chain.is_owner(&r.by)
            {
                said.push((r.at.unix_nano(), r.answer.clone()));
            }
        }
        for c in self.coord.comments(&t.id).unwrap_or_default() {
            if ids.contains(&c.blocker_id.as_str()) && self.chain.is_owner(&c.by) {
                said.push((c.at.unix_nano(), c.text.clone()));
            }
        }
        said.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        said.into_iter()
            .map(|(_, s)| as_folder(&s))
            .find(|f| !f.is_empty() && Path::new(f).is_absolute() && Path::new(f).is_dir())
    }

    /// The "no copy" question and its context for t on this device.
    fn no_copy_ask(&self, t: &Task, not_a_folder: Option<&str>) -> (String, String) {
        let dev = self.this_device_name();
        let name = coord::repo_name(&t.repo_key);
        let (question, why) = if t.repo_key.is_empty() {
            (
                format!("This card names no repo. {CHOOSE_FOLDER} on {dev}?"),
                "Irori starts a card's agent in a copy of its repo, and this card names none."
                    .to_string(),
            )
        } else if self.capture.repo_roots(&t.repo_key).is_empty() {
            (
                format!("No device has a copy of {name}. {CHOOSE_FOLDER} on {dev}?"),
                format!(
                    "Irori starts this card's agent in a copy of {name}, and none of your devices has one it knows of."
                ),
            )
        } else {
            (
                format!("{dev} has no copy of {name}. {CHOOSE_FOLDER} on {dev}?"),
                format!(
                    "Irori starts this card's agent on {dev}, where the card was made, and the only copies of {name} it knows of are on your other devices."
                ),
            )
        };
        let mut context = String::new();
        if let Some(a) = not_a_folder {
            context += &format!("“{}” isn't a folder on {dev}. ", as_folder(a));
        }
        context += &format!(
            "{why} Reply with the folder's full path on {dev} (clone it first if it isn't there) and its agent starts in it."
        );
        (question, context)
    }

    /// Card t got no folder here: ask the person once. Called after its
    /// failed request, and on each sweep that finds the card asked for
    /// again (their reply put it back on the Board).
    pub(crate) fn stop_no_copy(&self, t: &Task) {
        let Some(t) = self.coord.get_task(&t.id) else {
            return;
        };
        if is_helper(&t) || coord::is_terminal(&t.state) {
            return;
        }
        let dev = self.this_device_name();
        // The person's reply on the newest "no copy" blocker, open or closed
        // (their answer puts a holderless card back on the Board, which
        // closes its blockers). It wasn't a folder here, or it would have
        // been used: say so and ask again.
        let latest = t
            .blockers
            .iter()
            .filter(|b| is_no_copy(b))
            .max_by_key(|b| b.at.unix_nano());
        let reply = latest.and_then(|b| {
            self.coord
                .comments(&t.id)
                .unwrap_or_default()
                .into_iter()
                .filter(|c| c.blocker_id == b.id && self.chain.is_owner(&c.by))
                .filter(|c| c.at.unix_nano() >= b.at.unix_nano())
                .max_by_key(|c| c.at.unix_nano())
        });
        let not_a_folder = reply.as_ref().map(|r| r.text.clone());
        if let Some(b) = latest.filter(|b| b.open()) {
            // Still blocked and nobody replied: it is already asking.
            if reply.is_none() && t.state == coord::BLOCKED {
                return;
            }
            // Put back on the Board ("Start an agent", or a reply): still no
            // folder, so this one closes and a fresh one asks.
            let text = match &reply {
                Some(r) => format!("“{}” isn't a folder on {dev}.", as_folder(&r.text)),
                None => format!("Still no folder for it on {dev}."),
            };
            let a = BlockerAnswer {
                text,
                resolve: true,
                ..BlockerAnswer::default()
            };
            if let Err(e) = self.coord.answer(SWEEPER, &t.id, &b.id, a) {
                logf(format_args!("no copy on {}: {e}", t.short_ref()));
                return;
            }
        }
        let (question, context) = self.no_copy_ask(&t, not_a_folder.as_deref());
        let nb = NewBlocker {
            question,
            context,
            blocked_by: Some(BlockedBy {
                kind: coord::BLOCKED_ON_PERSON.into(),
                ..BlockedBy::default()
            }),
            ..NewBlocker::default()
        };
        if let Err(e) = self.coord.raise(SWEEPER, &t.id, nb) {
            logf(format_args!("no copy on {}: {e}", t.short_ref()));
        }
    }

    /// The spawner failed for t: after [`SPAWN_TRIES`] in a row, stop and
    /// ask once.
    pub(crate) fn note_start_failed(&self, t: &Task, err: &str) {
        let n = {
            let mut m = self
                .spawn
                .start_fails
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let n = m.entry(t.id.clone()).or_insert(0);
            *n += 1;
            *n
        };
        if n < SPAWN_TRIES {
            return;
        }
        self.spawn
            .start_fails
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&t.id);
        if t.blockers.iter().any(|b| b.open() && is_start_failed(b)) {
            return;
        }
        let dev = self.this_device_name();
        let nb = NewBlocker {
            question: START_FAILED.into(),
            context: format!(
                "Irori tried {n} times to start an agent for this card on {dev}, and each time it couldn't. The last error: {err}"
            ),
            options: vec![BlockerOption {
                answer: START_AGAIN.into(),
                why: "Irori tries once more; if it fails again it asks again.".into(),
                by: SWEEPER.into(),
                recommended: true,
                ..BlockerOption::default()
            }],
            blocked_by: Some(BlockedBy {
                kind: coord::BLOCKED_ON_PERSON.into(),
                ..BlockedBy::default()
            }),
            ..NewBlocker::default()
        };
        if let Err(e) = self.coord.raise(SWEEPER, &t.id, nb) {
            logf(format_args!("start failed on {}: {e}", t.short_ref()));
        }
    }

    /// An agent started for t: its failure count starts over.
    pub(crate) fn note_started(&self, id: &str) {
        self.spawn
            .start_fails
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    /// Resolves t's open "no copy" blocker, if any, now that its agent has a
    /// folder here. Whether one was resolved.
    pub(crate) fn resolve_no_copy(&self, t: &Task, folder: &str) -> bool {
        let Some(b) = t.blockers.iter().find(|b| b.open() && is_no_copy(b)) else {
            return false;
        };
        let name = coord::repo_name(&t.repo_key);
        let dev = self.this_device_name();
        let text = if name.is_empty() {
            format!("Its agent starts in {folder} on {dev}.")
        } else {
            format!("Irori found a copy of {name} on {dev} ({folder}): its agent starts there.")
        };
        let a = BlockerAnswer {
            text,
            resolve: true,
            ..BlockerAnswer::default()
        };
        match self.coord.answer(SWEEPER, &t.id, &b.id, a) {
            Ok(_) => true,
            Err(e) => {
                logf(format_args!("no copy on {}: {e}", t.short_ref()));
                false
            }
        }
    }

    /// Lifts the "no copy" blockers this device raised once a copy is here
    /// (a Where, a checkout, or a folder the person named), and asks for
    /// the agent.
    pub(crate) fn lift_no_copy(&self) {
        let me = self.device.id.clone();
        let open = self.coord.list_tasks(&TaskFilter {
            open: true,
            ..TaskFilter::default()
        });
        for t in open {
            if is_helper(&t) || !t.blockers.iter().any(|b| b.open() && is_no_copy(b)) {
                continue;
            }
            // Only the device whose request failed lifts it: another one
            // finding a copy doesn't start the agent here.
            if !t.spawn_request().is_some_and(|s| s.device == me) {
                continue;
            }
            let Some(folder) = self.spawn_folder(&t) else {
                continue;
            };
            if self.resolve_no_copy(&t, &folder) {
                self.coord.spawn_now(&t.id);
            }
        }
    }
}

/// A helper's part (spawned agents part 7): its folder is a worktree of the
/// lead's, so a missing one is the lead's matter, not a question.
pub(crate) fn is_helper(t: &Task) -> bool {
    t.spawn && !t.parent.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_name_folders() {
        assert_eq!(
            as_folder("  `C:\\Users\\me\\irori`  "),
            "C:\\Users\\me\\irori"
        );
        assert_eq!(as_folder("\"/Users/me/irori\"\nthanks"), "/Users/me/irori");
        assert_eq!(as_folder(""), "");
    }

    #[test]
    fn old_and_new_reasons_are_no_folder() {
        assert!(no_folder_reason(
            "no checkout of remote:github.com/StephenSHorton/irori on 05f1844a"
        ));
        assert!(no_folder_reason("no checkout of  on 05f1844a"));
        assert!(no_folder_reason("no copy of irori on Desktop Computer"));
        assert!(no_folder_reason(
            "this card names no repo, and Desktop Computer has no folder for it"
        ));
        assert!(!no_folder_reason("not trusted: /r on pc"));
    }
}
