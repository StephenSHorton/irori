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
//!   agent starts there; a reply that isn't a folder here is quoted back
//!   and asked again (card #531: a question reads as one, not as a bad
//!   path). If a copy turns up here (a Where, or a checkout Irori sees),
//!   the blocker resolves itself and the agent starts.
//! - A card that names no repo is never asked about (card #600): its
//!   agent starts where an ask's does and works the repo out itself.
//! - The spawner itself fails [`SPAWN_TRIES`] times in a row (the person's
//!   number from Settings › Agents, if they set one): one blocker,
//!   "Irori couldn't start an agent for this card: try again?".
//!
//! Every line here names devices and repos the way the person does: the
//! device's own name and the repo's short name, never a key or an id.

use std::path::Path;

use coord::{BlockedBy, Blocker, BlockerAnswer, BlockerOption, NewBlocker, Task, TaskFilter};

use super::Services;
use crate::logf;

pub(crate) const SWEEPER: &str = "irori";

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

/// A missing checkout in the card line's words (card #480): the repo's short
/// name, or that the card names none, and the device's name.
pub fn no_folder_words(repo_key: &str, device: &str) -> String {
    if repo_key.is_empty() {
        format!("this card names no repo, and {device} has no folder for it")
    } else {
        format!("no copy of {} on {device}", coord::repo_name(repo_key))
    }
}

/// Whether b is one of Irori's "no copy" blockers.
pub fn is_no_copy(b: &Blocker) -> bool {
    b.raised_by == SWEEPER && b.question.contains(CHOOSE_FOLDER)
}

fn is_start_failed(b: &Blocker) -> bool {
    b.raised_by == SWEEPER && b.question == START_FAILED
}

/// Whether Irori itself acts on an answer to b (card #531): its "no copy"
/// and "couldn't start" asks, which the sweep reads, and its crash question,
/// whose answer lets the next agent start (card #540).
pub fn irori_acts_on(b: &Blocker) -> bool {
    is_no_copy(b) || is_start_failed(b) || coord::is_crash_question(b)
}

/// The folder a person's reply names: its first line that isn't a code
/// fence, without the quotes or backticks people put around paths, with a
/// leading `~` as the home folder.
fn as_folder(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("```"))
        .map(|l| {
            l.trim_matches(|c: char| matches!(c, '`' | '"' | '\'' | '“' | '”'))
                .trim()
        })
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if line == "~" || line.starts_with("~/") || line.starts_with("~\\") {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        if let Some(h) = home {
            return format!("{}{}", h.to_string_lossy(), &line[1..]);
        }
    }
    line.to_string()
}

/// `s` cut to `n` characters, marking the cut with `…`.
fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// A reply quoted back in a line: short enough for any blocker field.
fn quoted(reply: &str) -> String {
    format!("“{}”", cut(&as_folder(reply), 120))
}

/// Whether f starts at a root, on this device or any other: a person on a
/// Mac can paste a Windows path, and the other way round.
fn rooted(f: &str) -> bool {
    let b = f.as_bytes();
    Path::new(f).is_absolute()
        || f.starts_with(['/', '\\'])
        || (b.len() > 2
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && matches!(b[2], b'/' | b'\\'))
}

/// Whether f was meant as a path, not words to Irori: "i don't understand
/// this blocker" or "where is that?" is no bad path (card #531), nor is
/// "what does folder/repo mean": a relative path with a space reads as
/// words, as most replies with both are.
fn reads_as_path(f: &str) -> bool {
    !f.ends_with('?')
        && (rooted(f) || (f.contains(['/', '\\']) && !f.contains(char::is_whitespace)))
}

/// Why a reply wasn't used as the card's folder.
fn not_a_folder_line(reply: &str, dev: &str) -> String {
    let f = as_folder(reply);
    if !reads_as_path(&f) {
        format!(
            "{} doesn't name a folder, so Irori can't start the agent yet and is asking again.",
            quoted(reply)
        )
    } else if rooted(&f) && !Path::new(&f).is_dir() {
        format!("{} isn't a folder on {dev}.", quoted(reply))
    } else {
        // Relative, or a real folder named without its drive on Windows.
        format!("{} isn't a full path on {dev}.", quoted(reply))
    }
}

impl Services {
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

    /// What the person said on blockers `on` of t, newest first: their
    /// comments, and the answer a blocker was resolved with (the window
    /// resolves an Irori ask with the answer, so it is on no comment, card
    /// #531). Irori's own resolve lines and a close aren't the person's
    /// words, even a close by them ("back on the Board: …").
    fn said_on(&self, t: &Task, on: &[&Blocker]) -> Vec<String> {
        let mut said: Vec<(i64, String)> = Vec::new();
        for b in on {
            if let Some(r) = &b.resolved
                && !r.closed
                && self.chain.is_owner(&r.by)
            {
                said.push((r.at.unix_nano(), r.answer.clone()));
            }
        }
        for c in self.coord.comments(&t.id).unwrap_or_default() {
            let on_it = on
                .iter()
                .any(|b| b.id == c.blocker_id && c.at.unix_nano() >= b.at.unix_nano());
            if on_it && self.chain.is_owner(&c.by) {
                said.push((c.at.unix_nano(), c.text));
            }
        }
        said.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        said.into_iter().map(|(_, s)| s).collect()
    }

    /// The folder the person named in reply to a "no copy" blocker on t,
    /// newest first, when it is a folder here.
    pub(crate) fn chosen_folder(&self, t: &Task) -> Option<String> {
        let on: Vec<&Blocker> = t.blockers.iter().filter(|b| is_no_copy(b)).collect();
        if on.is_empty() {
            return None;
        }
        self.said_on(t, &on)
            .into_iter()
            .map(|s| as_folder(&s))
            .find(|f| !f.is_empty() && Path::new(f).is_absolute() && Path::new(f).is_dir())
    }

    /// The "no copy" question, its context and its options for t on this
    /// device. `reply` is what the person said to the last one, if it
    /// wasn't a folder: the new question quotes it, so it can't read as the
    /// same one coming back.
    fn no_copy_ask(&self, t: &Task, reply: Option<&str>) -> (String, String, Vec<BlockerOption>) {
        let dev = self.this_device_name();
        let name = coord::repo_name(&t.repo_key);
        let (question, why) = if self.capture.repo_roots(&t.repo_key).is_empty() {
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
                    "Irori starts this card's agent on {dev}, where the card was made, and none of the folders it has seen for {name} is on {dev} now."
                ),
            )
        };
        let question = match reply {
            Some(r) => format!(
                "Irori still needs a folder after your reply, “{}”. {CHOOSE_FOLDER} on {dev}?",
                cut(&as_folder(r), 60)
            ),
            None => question,
        };
        let mut context = String::new();
        if let Some(r) = reply {
            context += &not_a_folder_line(r, &dev);
            context.push(' ');
        }
        context += &why;
        context.push(' ');
        context += &format!(
            "Reply with the folder's full path on {dev} (clone it first if it isn't there) and its agent starts in it."
        );
        let options = Vec::new();
        (
            cut(&question, 200),
            cut(&context, coord::MAX_CONTEXT),
            options,
        )
    }

    /// Card t got no folder here: ask the person once. Called after its
    /// failed request, and on each sweep that finds the card asked for
    /// again (their reply put it back on the Board).
    pub(crate) fn stop_no_copy(&self, t: &Task) {
        let Some(t) = self.coord.get_task(&t.id) else {
            return;
        };
        // A card that names no repo isn't asked about (card #600): its
        // agent works the repo out. It has no folder here only when this
        // device has no checkout of any repo; its failed request says so.
        if is_helper(&t) || coord::is_terminal(&t.state) || t.repo_key.is_empty() {
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
        let reply = latest.and_then(|b| self.said_on(&t, &[b]).into_iter().next());
        if let Some(b) = latest.filter(|b| b.open()) {
            // Still blocked and nobody replied: it is already asking.
            if reply.is_none() && t.state == coord::BLOCKED {
                return;
            }
            // Put back on the Board ("Start an agent", or a reply): still no
            // folder, so this one closes and a fresh one asks.
            let text = match &reply {
                Some(r) => not_a_folder_line(r, &dev),
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
        let (question, context, options) = self.no_copy_ask(&t, reply.as_deref());
        let nb = NewBlocker {
            question,
            context,
            options,
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

    /// The spawner failed for t: after the setting's tries in a row, else
    /// [`SPAWN_TRIES`], stop and ask once.
    pub(crate) fn note_start_failed(&self, t: &Task, err: &str) {
        let tries = match self.spawn_setting("").start_tries {
            n if n >= 1 => n as u32,
            _ => SPAWN_TRIES,
        };
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
        if n < tries {
            return;
        }
        // One short of the limit: after Try again, the next failure asks
        // straight away. A start clears it (note_started).
        self.spawn
            .start_fails
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(t.id.clone(), tries - 1);
        if t.blockers.iter().any(|b| b.open() && is_start_failed(b)) {
            return;
        }
        let dev = self.this_device_name();
        let lead = format!("Irori couldn't start an agent for this card on {dev}.");
        let nb = NewBlocker {
            question: START_FAILED.into(),
            context: cut(
                &format!("{lead} The last error: {}", err.trim()),
                coord::MAX_CONTEXT,
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
    pub(crate) fn resolve_no_copy(&self, t: &Task, folder: &str, trusted: bool) -> bool {
        let Some(b) = t.blockers.iter().find(|b| b.open() && is_no_copy(b)) else {
            return false;
        };
        let name = coord::repo_name(&t.repo_key);
        let dev = self.this_device_name();
        let folder = cut(folder, 200);
        let found = if name.is_empty() {
            format!("Irori starts its agent in {folder} on {dev}, and it works out the card's repo")
        } else {
            format!("Irori found a copy of {name} on {dev} ({folder})")
        };
        let text = if trusted {
            format!("{found}: its agent starts there.")
        } else {
            format!(
                "{found}, but it isn't trusted for agents yet: Trust and start it in Needs you."
            )
        };
        let a = BlockerAnswer {
            text: cut(&text, 400),
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
    /// (a Where, a checkout, or a folder the person named): the card then
    /// starts as any other does.
    pub(crate) fn lift_no_copy(&self, sp: &super::spawn::Spawner) {
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
            // Not trusted: the request fails as such and Needs you offers
            // Trust and start. Resolving moves the card on past its failed
            // request, so the sweep asks again without waiting out the
            // retry; it isn't the person's ask, so the queue order still
            // holds (card #576).
            let trusted = (sp.trusted)(Path::new(&folder));
            self.resolve_no_copy(&t, &folder, trusted);
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
        assert_eq!(as_folder("```\n/Users/me/irori\n```"), "/Users/me/irori");
        assert_eq!(
            as_folder("```sh\n`/Users/me/irori`\n```"),
            "/Users/me/irori"
        );
        if let Some(h) = std::env::var_os("HOME") {
            assert_eq!(
                as_folder("~/irori"),
                format!("{}/irori", h.to_string_lossy())
            );
        }
        let long = "x".repeat(5000);
        assert!(quoted(&long).chars().count() <= 122);
        assert_eq!(
            not_a_folder_line("work/irori", "Desktop Computer"),
            "“work/irori” isn't a full path on Desktop Computer."
        );
        assert_eq!(
            not_a_folder_line("C:\\nope\\irori", "Macbook Pro"),
            "“C:\\nope\\irori” isn't a folder on Macbook Pro."
        );
        assert_eq!(
            not_a_folder_line("/nope/irori", "Desktop Computer"),
            "“/nope/irori” isn't a folder on Desktop Computer."
        );
    }

    /// A real folder named without its drive is there, but not a full path.
    #[cfg(windows)]
    #[test]
    fn a_folder_without_its_drive_is_not_a_full_path() {
        let cwd = std::env::current_dir().unwrap();
        let s = cwd.to_string_lossy();
        let line = not_a_folder_line(&s[2..], "Desktop Computer");
        assert!(
            line.ends_with("isn't a full path on Desktop Computer."),
            "{line}"
        );
    }

    /// Card #531: words to Irori aren't called a bad path.
    #[test]
    fn a_question_is_not_a_bad_path() {
        for r in [
            "i don't understand this blocker",
            "I'll clone it later",
            "what folder?",
            "is it /Users/me/irori?",
            "what does folder/repo mean",
            "huh",
        ] {
            let line = not_a_folder_line(r, "Macbook Pro");
            assert!(line.starts_with(&format!("“{r}”")), "{line}");
            assert!(line.contains("asking again"), "{line}");
            assert!(!line.contains("isn't a"), "{line}");
        }
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
