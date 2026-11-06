//! What the person said lately on the cards around one card (card #555).
//!
//! A brief holds one card, so what the person says on a sibling reaches
//! its agent only if it searches: on #416 they said what the merge desk
//! is, and #418, under the same parent, carried on with the old premise.
//! [`Fold::said_near`] is the one answer to "what did the person say near
//! this card", for a spawned agent's brief, a wake and `show`. Only the
//! person's own words count, and it is short and capped: every brief and
//! wake pays for it.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use gocompat::time::GoTime;

use super::asks::ASK_FILES_NOTE;
use super::fold::{Fold, card_refs};
use super::spawn::CRASHED_START;
use super::types::{BLOCKED_ON_TASK, Comment, Task, WORKING};

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
    /// from one card. When there is more than fits, a line on a subtask or
    /// one naming the card goes first, then its parent's and siblings',
    /// then the rest. The card's own thread isn't here: a brief quotes it.
    pub(crate) fn said_near(&self, id: &str) -> Vec<Said> {
        let Some(t) = self.tasks.get(id) else {
            return Vec::new();
        };
        let since = self.said_since();
        let family = |task: &str| {
            !t.parent.is_empty()
                && self
                    .tasks
                    .get(task)
                    .is_some_and(|o| o.id == t.parent || o.parent == t.parent)
        };
        self.said_lines(&self.near(t, since), since, |c| {
            if t.subtasks.contains(&c.task_id) || names(&c.text, t) {
                2
            } else {
                u8::from(family(&c.task_id))
            }
        })
    }

    /// [`Self::said_near`] for `id`'s subtasks alone: `show` on a parent,
    /// where nothing else nearby may crowd them out.
    pub(crate) fn said_on_subtasks(&self, id: &str) -> Vec<Said> {
        let Some(t) = self.tasks.get(id) else {
            return Vec::new();
        };
        let subs: HashSet<String> = t.subtasks.iter().cloned().collect();
        self.said_lines(&subs, self.said_since(), |_| 0)
    }

    /// The card to tell `key` about and what the person said around it,
    /// for a wake: the first of `refs` (`task:<id>`, `comment:<id>`) that
    /// is `key`'s card, or with `working` the card it works now. `key` is
    /// this log's form of the agent's key. None when it has no such card.
    pub(crate) fn said_for(
        &self,
        key: &str,
        refs: &[String],
        working: bool,
    ) -> Option<(String, Vec<Said>)> {
        let has = |l: &[String]| l.iter().any(|k| k == key);
        let held = |t: &Task| has(&t.holders) || has(&t.co_holders);
        let mine =
            |t: &Task| held(t) || has(&t.assigned) || t.offer.as_ref().is_some_and(|o| o.to == key);
        let named = refs.iter().find_map(|r| {
            let id = match r.strip_prefix("comment:") {
                Some(c) => self.comments.get(c)?.task_id.as_str(),
                None => r.strip_prefix("task:").unwrap_or(r),
            };
            // A bridged notice's ref: task:<id>/comment:<id>.
            let id = id.split('/').next().unwrap_or(id);
            self.tasks.get(id).filter(|t| mine(t))
        });
        let t = named.or_else(|| {
            self.tasks
                .values()
                .filter(|t| working && t.state == WORKING && held(t))
                .max_by_key(|t| t.updated_at.unix_nano())
        })?;
        Some((t.short_ref(), self.said_near(&t.id)))
    }

    fn said_since(&self) -> i64 {
        self.stamp().unix_nano() - SAID_DAYS * DAY_NANOS
    }

    /// The person's own lines on `cards` since `since`, highest `rank`
    /// first and then newest, within the caps, returned newest first.
    /// Their own: the row's `by` is only what its writer claimed, so a line
    /// must also have folded as theirs (`person`; a mirrored message is
    /// checked when it is recorded). A picked blocker option is left out:
    /// the agent wrote that text, the person only clicked it.
    fn said_lines(
        &self,
        cards: &HashSet<String>,
        since: i64,
        rank: impl Fn(&Comment) -> u8,
    ) -> Vec<Said> {
        let mut lines: Vec<_> = self
            .comments
            .values()
            .filter(|c| cards.contains(&c.task_id) && self.is_owner(&c.by))
            .filter(|c| c.person || !c.msg_id.is_empty())
            .filter(|c| c.at.unix_nano() >= since && !c.held && c.pick == 0)
            .filter(|c| !c.text.trim().is_empty())
            .filter(|c| c.text != CRASHED_START && c.text != ASK_FILES_NOTE)
            .collect();
        lines.sort_by_key(|c| Reverse((rank(c), c.at.unix_nano(), c.hlc, c.seq)));
        let mut per_card: HashMap<&str, usize> = HashMap::new();
        let mut kept: Vec<&Comment> = Vec::new();
        for c in lines {
            let n = per_card.entry(c.task_id.as_str()).or_default();
            if *n < SAID_PER_CARD && self.tasks.contains_key(&c.task_id) {
                *n += 1;
                kept.push(c);
                if kept.len() == SAID_MAX {
                    break;
                }
            }
        }
        kept.sort_by_key(|c| Reverse((c.at.unix_nano(), c.hlc, c.seq)));
        kept.into_iter()
            .map(|c| Said {
                comment_id: c.id.clone(),
                task_id: c.task_id.clone(),
                card: self.tasks[&c.task_id].short_ref(),
                text: clip(c.text.trim(), SAID_CHARS),
                at: c.at,
            })
            .collect()
    }
}
