//! What the person said lately on the cards around one card (card #555).
//!
//! A brief holds one card, so what the person says on a sibling reaches
//! its agent only if it searches: on #416 they said what the merge desk
//! is, and #418, under the same parent, carried on with the old premise.
//! [`Fold::said_near`] is the one answer to "what did the person say near
//! this card", for a spawned agent's brief, a wake and `show`. Only the
//! person's own words count, and it is short and capped: every brief and
//! wake pays for it.

use std::collections::{HashMap, HashSet};

use gocompat::time::GoTime;

use super::fold::{Fold, card_refs};
use super::spawn::CRASHED_START;
use super::types::{BLOCKED_ON_TASK, Task};

/// How many of the person's lines one brief or wake carries.
pub const SAID_MAX: usize = 5;
/// How many of those from one card: a busy card that names this one once
/// doesn't push out a sibling's line.
pub const SAID_PER_CARD: usize = 2;
/// How much of a line is quoted; `show comment:<id>` has the rest.
pub const SAID_CHARS: usize = 300;
/// How old a line may be and still count as lately.
pub const SAID_DAYS: i64 = 7;

const DAY_NANOS: i64 = 86_400_000_000_000;

/// One thing the person said on a card near another.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Said {
    pub comment_id: String,
    pub task_id: String,
    /// The card's short ref (`#416`, `mlg#3`).
    pub card: String,
    /// The words, clipped to [`SAID_CHARS`] (`…` when cut).
    pub text: String,
    pub at: GoTime,
}

/// `s` to at most `n` characters, `…` when cut.
fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n).collect();
    out.push('…');
    out
}

/// Whether `text` names card t by its short ref.
fn names(text: &str, t: &Task) -> bool {
    !t.short.is_empty()
        && text.contains('#')
        && card_refs(text)
            .iter()
            .any(|(tag, n)| *n == t.short && tag.as_ref().is_none_or(|g| *g == t.tag))
}

/// A card's own text, where it names other cards.
fn card_text(t: &Task) -> String {
    format!("{}\n{}\n{}", t.title, t.objective, t.done_criteria)
}

impl Fold {
    /// The cards around t: its parent and the parent's other subtasks, its
    /// own subtasks, the thread it came from and the cards made from it,
    /// cards it waits on or that wait on it, and cards that name it or that
    /// it names. A comment naming t counts only when it is recent (`since`,
    /// Unix nanoseconds).
    fn near(&self, t: &Task, since: i64) -> HashSet<String> {
        let mut out: HashSet<String> = HashSet::new();
        if let Some(p) = self.tasks.get(&t.parent) {
            out.insert(p.id.clone());
            out.extend(p.subtasks.iter().cloned());
        }
        out.extend(t.subtasks.iter().cloned());
        out.extend(t.made.iter().cloned());
        if !t.from.is_empty() {
            out.insert(t.from.clone());
        }
        if let Some(b) = t.blocked_by.as_ref().filter(|b| b.kind == BLOCKED_ON_TASK) {
            out.insert(b.reference.clone());
        }
        let own = card_text(t);
        for o in self.tasks.values() {
            let waits = o
                .blocked_by
                .as_ref()
                .is_some_and(|b| b.kind == BLOCKED_ON_TASK && b.reference == t.id);
            if waits || names(&own, o) || names(&card_text(o), t) {
                out.insert(o.id.clone());
            }
        }
        for c in self.comments.values() {
            if c.at.unix_nano() >= since && names(&c.text, t) {
                out.insert(c.task_id.clone());
            }
        }
        out.remove(&t.id);
        out
    }

    /// What the person said in the last [`SAID_DAYS`] on the cards around
    /// `id`, newest first, at most [`SAID_MAX`] lines and [`SAID_PER_CARD`]
    /// from one card. The person's words only: an agent's comment, or
    /// another member's on a shared board, never counts. The card's own
    /// thread isn't here: a brief quotes it already.
    pub(crate) fn said_near(&self, id: &str) -> Vec<Said> {
        let Some(t) = self.tasks.get(id) else {
            return Vec::new();
        };
        let since = self.stamp().unix_nano() - SAID_DAYS * DAY_NANOS;
        let near = self.near(t, since);
        let mut lines: Vec<_> = self
            .comments
            .values()
            .filter(|c| near.contains(&c.task_id) && self.is_owner(&c.by))
            .filter(|c| c.at.unix_nano() >= since && !c.held)
            .filter(|c| !c.text.trim().is_empty() && c.text != CRASHED_START)
            .collect();
        lines.sort_by(|a, b| (b.at.unix_nano(), &b.id).cmp(&(a.at.unix_nano(), &a.id)));
        let mut per_card: HashMap<&str, usize> = HashMap::new();
        let mut out = Vec::new();
        for c in lines {
            let n = per_card.entry(c.task_id.as_str()).or_default();
            if *n >= SAID_PER_CARD {
                continue;
            }
            let Some(on) = self.tasks.get(&c.task_id) else {
                continue;
            };
            *n += 1;
            out.push(Said {
                comment_id: c.id.clone(),
                task_id: on.id.clone(),
                card: on.short_ref(),
                text: clip(c.text.trim(), SAID_CHARS),
                at: c.at.clone(),
            });
            if out.len() == SAID_MAX {
                break;
            }
        }
        out
    }
}
