//! A card's linked pull requests as GitHub says they stand (card #208).
//!
//! The device that holds a card asks GitHub (the person's own `gh`) how
//! each PR the card links stands and writes a `pr.state` row when that
//! changes. coord never shells out: the Core's poller does, and hands what
//! it heard to [`Fold::record_pr_state`], which writes the row and acts on
//! it:
//! - merged: the holders are woken to complete the card with proof, or,
//!   while another PR the card links isn't merged yet, told without a wake;
//! - closed without a merge: the holders hear it;
//! - changes requested on GitHub: a card in review goes back to working
//!   and its holders are woken, as a reviewer's changes verdict does;
//! - GitHub couldn't be asked (no access, a PR gone) about a PR nothing is
//!   known of yet: said once on the card, and an agent's completed falls
//!   back to the rules without it;
//! - reopened after it was closed: the holders hear it, without a wake.
//!
//! What GitHub said outlasts a device that couldn't ask it (card #542): an
//! error never replaces a known state, in the fold or at the writer. A
//! device with no gh doesn't ask at all, but older phones still write
//! these rows, in any order against the rows that know better. When gh
//! fails on a PR known to be open, the row keeps OPEN and carries the
//! error beside it (card #548): the card shows it and the poller slows.
//!
//! An agent's completed is refused while a PR its card links is known to
//! be open ([`Fold::open_pr_refusal`]), and the refusal says when GitHub
//! can't be asked.
//!
//! On a shared board the notices above reach only the writer's person's
//! agents: another person's agents' keys mean nothing on that device. So
//! each device asks [`Fold::pr_news`] what its own holders may not have
//! heard, and the Core tells them through the bridge, once (card #548).

use std::collections::BTreeMap;

use gocompat::time::GoTime;
use serde::Serialize;
use store::{Event, kinds};

use super::error::{Result, invalid, not_found};
use super::fold::Fold;
use super::links::pr_link;
use super::rows::{PrStateRow, TaskStateRow, row};
use super::types::*;
use super::util::{SWEEPER, deeper};
use gocompat::quote::quote;

/// A PR notice's kind.
pub const NOTICE_PR: &str = "pr";

/// GitHub's PR states, as `gh pr view --json state` spells them.
pub const PR_OPEN: &str = "OPEN";
pub const PR_MERGED: &str = "MERGED";
pub const PR_CLOSED: &str = "CLOSED";
/// GitHub's review decision that sends a card back to work.
pub const PR_CHANGES_REQUESTED: &str = "CHANGES_REQUESTED";
/// GitHub's review decision on a repo that wants an approval before a
/// merge: it comes from another member's agents (card #471).
pub const PR_REVIEW_REQUIRED: &str = "REVIEW_REQUIRED";

/// How one linked PR stands, as last heard from GitHub.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrState {
    /// The PR's URL, as the card links it.
    pub pr: String,
    /// OPEN, MERGED or CLOSED; empty when GitHub couldn't be asked.
    pub state: String,
    /// GitHub's reviewDecision: APPROVED, CHANGES_REQUESTED,
    /// REVIEW_REQUIRED or empty.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub review: String,
    #[serde(skip_serializing_if = "GoTime::is_zero")]
    pub merged_at: GoTime,
    /// Why GitHub couldn't be asked (no gh, no access); empty when it was.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    /// When it was heard.
    pub at: GoTime,
    /// The devices that wrote what this row says, whatever their times
    /// (card #548): each told its own person's agents as it wrote.
    #[serde(skip)]
    pub writers: Vec<String>,
    /// When GitHub first said it: the earliest row saying the same.
    #[serde(skip)]
    pub since: GoTime,
    /// The state before this one, to hear a reopen by.
    #[serde(skip)]
    pub was: String,
}

/// What the poller heard about one PR.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrCheck {
    pub pr: String,
    pub state: String,
    pub review: String,
    pub merged_at: GoTime,
    pub error: String,
}

/// The PR states by card, then by PR URL.
pub(crate) type PrStates = BTreeMap<String, BTreeMap<String, PrState>>;

/// pr.state rows a board keeps for a card whose holder or wait hasn't
/// folded yet, at most (card #548).
const MAX_PR_AHEAD: usize = 256;

/// What a PR's holders are told when it comes to stand some way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrChange {
    Merged,
    Closed,
    ChangesRequested,
    Reopened,
}

/// What a device owes its own agents about one PR on a shared board
/// (card #548): see [`Fold::pr_news`].
#[derive(Debug, Clone, Default)]
pub struct PrNews {
    /// Names this news, so one agent hears it once.
    pub mark: String,
    /// The devices that wrote it.
    pub writers: Vec<String>,
    /// What to say, and to whom, in board keys.
    pub notice: Notice,
}

/// How far a row's state can be trusted over another's (card #542): an
/// error says nothing about the PR, and a merge can't be undone, while
/// GitHub can reopen a closed PR.
fn certainty(state: &str) -> u8 {
    match state {
        "" => 0,
        PR_MERGED => 2,
        _ => 1,
    }
}

/// Whether a row tells a PR's holders what `old` told them. A review can
/// still change after a merge, and that says nothing new.
fn same_news(state: &str, review: &str, old: &PrState) -> bool {
    state == old.state && (state == PR_MERGED || review == old.review)
}

impl Fold {
    pub(crate) fn fold_pr_state(&mut self, ev: &Event) {
        let Some(p) = row::<PrStateRow>(&ev.payload) else {
            return;
        };
        if p.task_id.is_empty() || p.pr.is_empty() {
            return;
        }
        let by_pr = self.pr_states.entry(p.task_id.clone()).or_default();
        // An error is nothing GitHub said, so it counts as nobody's word.
        let heard = p.error.is_empty() && !p.state.is_empty();
        let (mut writers, mut since, mut was) = (Vec::new(), p.at, String::new());
        if let Some(old) = by_pr.get_mut(&p.pr) {
            let same = same_news(&p.state, &p.review, old);
            if same {
                if heard && !old.writers.contains(&ev.device_id) {
                    old.writers.push(ev.device_id.clone());
                }
                if crate::gotime::before(&p.at, &old.since) {
                    old.since = p.at;
                }
            }
            // Card #542: the stronger row wins whatever their times, so every
            // device ends on the same one in whatever order they arrive.
            let (new, known) = (certainty(&p.state), certainty(&old.state));
            if new < known || new == known && crate::gotime::before(&p.at, &old.at) {
                return;
            }
            if same {
                (writers, since, was) = (old.writers.clone(), old.since, old.was.clone());
            } else {
                was = old.state.clone();
            }
        }
        if heard && !writers.contains(&ev.device_id) {
            writers.push(ev.device_id.clone());
        }
        by_pr.insert(
            p.pr.clone(),
            PrState {
                pr: p.pr,
                state: p.state,
                review: p.review,
                merged_at: p.merged_at,
                error: p.error,
                at: p.at,
                writers,
                since,
                was,
            },
        );
        // A phone hears a merge only through this row: Needs you must redraw.
        self.note(Change {
            kind: TASK_CHANGED.into(),
            task_id: p.task_id,
            ..Change::default()
        });
    }

    /// The states of the PRs `t` links, in link order; a linked PR not yet
    /// heard of is left out.
    pub(crate) fn pr_states_of(&self, t: &Task) -> Vec<PrState> {
        let Some(by_pr) = self.pr_states.get(&t.id) else {
            return Vec::new();
        };
        t.links
            .iter()
            .filter(|l| pr_link(l).is_some())
            .filter_map(|l| by_pr.get(&l.reference).cloned())
            .collect()
    }

    /// Whether PR `pr` on card `id` is known merged or closed (card #509):
    /// a wait on it no longer needs the person.
    pub(crate) fn pr_settled(&self, id: &str, pr: &str) -> bool {
        self.pr_states
            .get(id)
            .and_then(|by_pr| by_pr.get(pr))
            .is_some_and(|p| p.state == PR_MERGED || p.state == PR_CLOSED)
    }

    /// The PRs t's open, unanswered external waits ask this person to
    /// merge (card #509), not yet known merged or closed. On a shared
    /// board only their own agents' waits: another member's is theirs.
    pub(crate) fn merge_waits(&self, t: &Task) -> Vec<String> {
        self.waits_on(t, |pr| !self.pr_settled(&t.id, pr))
    }

    /// The PRs of [`Fold::merge_waits`] the poller still asks about: a
    /// closed one too, which GitHub can reopen (card #548).
    pub(crate) fn polled_waits(&self, t: &Task) -> Vec<String> {
        let by_pr = self.pr_states.get(&t.id);
        self.waits_on(t, |pr| {
            !by_pr
                .and_then(|m| m.get(pr))
                .is_some_and(|p| p.state == PR_MERGED)
        })
    }

    fn waits_on(&self, t: &Task, keep: impl Fn(&str) -> bool) -> Vec<String> {
        if t.state != BLOCKED {
            return Vec::new();
        }
        let mut out: Vec<String> = Vec::new();
        for b in &t.blockers {
            if b.status == BLOCKER_ANSWERED || self.on_board && !self.own_on_board(b) {
                continue;
            }
            if let Some(pr) = b.merge_wait().filter(|pr| keep(pr))
                && !out.contains(&pr)
            {
                out.push(pr);
            }
        }
        out
    }

    /// Refuses an agent's completed while a PR its card links is known to
    /// be open (card #208). A PR GitHub was never heard on doesn't count:
    /// the rules without it apply. One GitHub can't be asked about now
    /// still counts, open being the last it said (card #548): a gh that
    /// failed once must not let open work through. The refusal says so.
    pub(crate) fn open_pr_refusal(&self, t: &Task) -> Result<()> {
        let open: Vec<PrState> = self
            .pr_states_of(t)
            .into_iter()
            .filter(|p| p.state == PR_OPEN)
            .collect();
        if open.is_empty() {
            return Ok(());
        }
        let prs = open
            .iter()
            .map(|p| p.pr.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        if let Some(p) = open.iter().find(|p| !p.error.is_empty()) {
            return Err(invalid(format!(
                "{} links a pull request last heard open ({prs}), and GitHub can't be asked now: {}. If it has merged, fix gh on this device, or ask the person with board_block to complete the card",
                t.short_ref(),
                p.error
            )));
        }
        Err(invalid(format!(
            "{} links a pull request that is still open ({prs}): ask the person “Can I merge…?” with board_block and merge it yourself on their yes; Irori checks GitHub and tells you when it merges",
            t.short_ref()
        )))
    }

    /// Whether every PR `t` links is known to be merged (and it links one).
    pub(crate) fn all_prs_merged(&self, t: &Task) -> bool {
        let linked = t.links.iter().filter(|l| pr_link(l).is_some()).count();
        let states = self.pr_states_of(t);
        linked > 0 && states.len() == linked && states.iter().all(|p| p.state == PR_MERGED)
    }

    /// Records what the poller heard about PR `c.pr` on card `id` and acts
    /// on what changed (module doc). Nothing is written when nothing
    /// changed, when GitHub still can't be asked (that is said once), or
    /// when it can't be asked about a PR known merged or closed. Failing
    /// on a PR known open, the row keeps OPEN and adds the error.
    /// Returns the card and the notices to route.
    pub(crate) fn record_pr_state(&mut self, id: &str, c: PrCheck) -> Result<(Task, Vec<Notice>)> {
        let Some(t) = self.tasks.get(id).cloned() else {
            return Err(not_found(format!("task {}", quote(id))));
        };
        // Card #509: or a PR an open external wait is on.
        if !t
            .links
            .iter()
            .any(|l| l.reference == c.pr && pr_link(l).is_some())
            && !t
                .blockers
                .iter()
                .any(|b| b.merge_wait().is_some_and(|w| w == c.pr))
        {
            return Err(invalid(format!(
                "{} links no pull request {}",
                t.short_ref(),
                quote(&c.pr)
            )));
        }
        let old = self
            .pr_states
            .get(id)
            .and_then(|m| m.get(&c.pr))
            .cloned()
            .unwrap_or_default();
        // Card #548: gh failing on a PR known open keeps what it last said.
        let c = if !c.error.is_empty() && old.state == PR_OPEN {
            PrCheck {
                state: old.state.clone(),
                review: old.review.clone(),
                ..c
            }
        } else {
            c
        };
        // What the fold wouldn't take isn't written: it would only repeat.
        let unchanged = certainty(&c.state) < certainty(&old.state)
            || if c.error.is_empty() {
                old.error.is_empty()
                    && old.state == c.state
                    && old.review == c.review
                    && !old.at.is_zero()
            } else {
                // Said once: a later failure stays quiet until it is heard again.
                !old.error.is_empty()
            };
        if unchanged {
            return Ok((t, Vec::new()));
        }
        let at = self.stamp();
        self.write_as(
            SWEEPER,
            kinds::PR_STATE,
            &PrStateRow {
                task_id: id.into(),
                pr: c.pr.clone(),
                state: c.state.clone(),
                review: c.review.clone(),
                merged_at: c.merged_at,
                error: c.error.clone(),
                by: SWEEPER.into(),
                at,
            },
        )?;
        // A row this log didn't take (a board's writer rules) changes
        // nothing here: telling the holders would repeat at every poll.
        let took = self
            .pr_states
            .get(id)
            .and_then(|m| m.get(&c.pr))
            .is_some_and(|p| p.at == at);
        if !took {
            return Err(invalid(format!(
                "{}: the pr.state row for {} wasn't taken here",
                t.short_ref(),
                quote(&c.pr)
            )));
        }
        if !c.error.is_empty() || is_terminal(&t.state) {
            return Ok((self.tasks[id].clone(), Vec::new()));
        }
        let newly = |s: &str| c.state == s && old.state != s;
        let change = if newly(PR_MERGED) {
            Some(PrChange::Merged)
        } else if newly(PR_CLOSED) {
            Some(PrChange::Closed)
        } else if c.state == PR_OPEN
            && c.review == PR_CHANGES_REQUESTED
            && old.review != PR_CHANGES_REQUESTED
        {
            Some(PrChange::ChangesRequested)
        } else if c.state == PR_OPEN && old.state == PR_CLOSED {
            Some(PrChange::Reopened)
        } else {
            None
        };
        if change == Some(PrChange::ChangesRequested) && t.state == IN_REVIEW {
            let at = self.stamp();
            self.write_as(
                SWEEPER,
                kinds::TASK_STATUS,
                &TaskStateRow {
                    task_id: id.into(),
                    state: WORKING.into(),
                    note: "changes requested on GitHub".into(),
                    by: SWEEPER.into(),
                    at,
                    ..TaskStateRow::default()
                },
            )?;
        }
        let ns = change
            .map(|ch| self.change_notice(&t, &c.pr, ch))
            .filter(|n| !n.to.is_empty());
        Ok((self.tasks[id].clone(), ns.into_iter().collect()))
    }

    /// The notice for `change` to PR `pr` on `t`.
    fn change_notice(&self, t: &Task, pr: &str, change: PrChange) -> Notice {
        match change {
            PrChange::Merged => {
                // Another linked PR still open (the card's own, with the
                // merged one linked as context, #230): say it merged, but
                // don't ask for completed and don't wake anyone over it.
                let waiting = self.unfinished_prs_besides(t, pr);
                if waiting.is_empty() {
                    return self.pr_notice(
                        t,
                        pr,
                        "was merged",
                        format!(
                            "Check the work landed, then set it completed with proof: board_status task_id={} state=completed proof=\"…\".",
                            t.id
                        ),
                    );
                }
                let mut n = self.pr_notice(
                    t,
                    pr,
                    "was merged",
                    format!(
                        "The card also links {}, not merged yet: keep it as it is; Irori tells you when that merges.",
                        waiting.join(", ")
                    ),
                );
                n.wake = false;
                n
            }
            PrChange::Closed => self.pr_notice(
                t,
                pr,
                "was closed without merging",
                "Say on the card what happens to the work now: reopen it, link the PR that replaces it, or ask the person.".into(),
            ),
            PrChange::ChangesRequested => self.pr_notice(
                t,
                pr,
                "has changes requested on GitHub",
                "Read the review with `gh pr view --comments`, make the changes, push, then set board_status in_review again.".into(),
            ),
            PrChange::Reopened => {
                let mut n = self.pr_notice(
                    t,
                    pr,
                    "was reopened on GitHub",
                    "It counts as open again: the card can't be completed until it merges or closes.".into(),
                );
                n.wake = false;
                n
            }
        }
    }

    /// What this board's PR rows say now, for each device to pass to the
    /// agents it hosts (card #548): the writer's device could only tell
    /// its own person's. Read from the fold, not from rows as they come,
    /// so a row that came ahead of the card's holder still counts;
    /// [`PrNews::mark`] keeps it to once. Rows kept for a holder that
    /// hadn't folded get another look first.
    pub(crate) fn pr_news(&mut self) -> Vec<PrNews> {
        self.retry_pr_ahead(None);
        let mut out = Vec::new();
        for (id, by_pr) in &self.pr_states {
            let Some(t) = self.tasks.get(id).filter(|t| !is_terminal(&t.state)) else {
                continue;
            };
            for (pr, p) in by_pr {
                let change = match (p.state.as_str(), p.review.as_str()) {
                    (PR_MERGED, _) => PrChange::Merged,
                    (PR_CLOSED, _) => PrChange::Closed,
                    (PR_OPEN, PR_CHANGES_REQUESTED) => PrChange::ChangesRequested,
                    (PR_OPEN, _) if p.was == PR_CLOSED => PrChange::Reopened,
                    _ => continue,
                };
                let notice = self.change_notice(t, pr, change);
                if notice.to.is_empty() || p.writers.is_empty() {
                    continue;
                }
                // A merge is final; the others can come round again.
                let mark = if change == PrChange::Merged {
                    format!("pr:{id}/{pr}/{PR_MERGED}")
                } else {
                    format!(
                        "pr:{id}/{pr}/{}/{}@{}",
                        p.state,
                        p.review,
                        p.since.unix_nano()
                    )
                };
                out.push(PrNews {
                    mark,
                    writers: p.writers.clone(),
                    notice,
                });
            }
        }
        out
    }

    /// Keeps a pr.state row a board refused (card #548): its holder's or
    /// wait's row may be on its way from another device, and nothing
    /// writes a settled PR's row again. The newest per PR and device.
    pub(crate) fn hold_pr_row(&mut self, ev: &Event) {
        if !self.on_board || ev.kind != kinds::PR_STATE {
            return;
        }
        let Some(p) = row::<PrStateRow>(&ev.payload) else {
            return;
        };
        self.pr_ahead.retain(|o| {
            o.device_id != ev.device_id
                || !row::<PrStateRow>(&o.payload)
                    .is_some_and(|r| r.task_id == p.task_id && r.pr == p.pr)
        });
        if self.pr_ahead.len() >= MAX_PR_AHEAD {
            self.pr_ahead.remove(0);
        }
        self.pr_ahead.push(ev.clone());
    }

    /// Folds the kept pr.state rows that count now: card `id`'s, or all.
    pub(crate) fn retry_pr_ahead(&mut self, id: Option<&str>) {
        if self.pr_ahead.is_empty() {
            return;
        }
        for ev in std::mem::take(&mut self.pr_ahead) {
            let named = id
                .is_none_or(|id| row::<PrStateRow>(&ev.payload).is_some_and(|r| r.task_id == id));
            if named && self.sweeper_ok(&ev) {
                self.fold(&ev);
            } else {
                self.pr_ahead.push(ev);
            }
        }
    }

    /// The PRs `t` links, other than `pr`, that aren't known to be merged
    /// or closed (open, or not heard of yet), in link order.
    fn unfinished_prs_besides(&self, t: &Task, pr: &str) -> Vec<String> {
        let heard = self.pr_states.get(&t.id);
        t.links
            .iter()
            .filter(|l| pr_link(l).is_some() && l.reference != pr)
            .filter(|l| {
                !heard
                    .and_then(|m| m.get(&l.reference))
                    .is_some_and(|p| p.state == PR_MERGED || p.state == PR_CLOSED)
            })
            .map(|l| l.reference.clone())
            .collect()
    }

    /// Puts back on the Board each in-review card nobody holds while a PR
    /// it links is open or not heard of yet (card #471): its agent went,
    /// so nobody merges it and nobody hears it merge. Not a card waiting
    /// on someone else: one whose PR GitHub says needs an approval (that
    /// comes from another member's agents) or that another member's agent
    /// was asked to review on a shared board.
    pub(crate) fn return_unheld_prs(&mut self) -> Result<()> {
        let ids: Vec<(String, String)> = self
            .tasks
            .values()
            .filter(|t| {
                t.state == IN_REVIEW
                    && t.holders.is_empty()
                    && t.co_holders.is_empty()
                    && t.assigned.is_empty()
                    && t.subtasks.is_empty()
                    && t.open_ask().is_none()
                    && self.hands_out(t)
                    && !(self.on_board && self.reviews.get(&t.id).is_some_and(|r| !r.is_empty()))
                    && !self
                        .pr_states_of(t)
                        .iter()
                        .any(|p| p.state == PR_OPEN && p.review == PR_REVIEW_REQUIRED)
            })
            .filter_map(|t| {
                let pr = self.unfinished_prs_besides(t, "").into_iter().next()?;
                let n = t
                    .links
                    .iter()
                    .find(|l| l.reference == pr)
                    .and_then(pr_link)
                    .map(|(_, n)| format!("PR #{n}"))
                    .unwrap_or(pr);
                Some((t.id.clone(), n))
            })
            .collect();
        for (id, n) in ids {
            self.back_on_board(
                SWEEPER,
                &id,
                &format!(
                    "its {n} is still open and nobody is on it: ask the person “Can I merge…?” and merge it on their yes, or complete the card if it merged"
                ),
            )?;
        }
        Ok(())
    }

    fn pr_notice(&self, t: &Task, pr: &str, what: &str, next: String) -> Notice {
        let nt = self.tasks.get(&t.id).unwrap_or(t);
        Notice {
            kind: NOTICE_PR.into(),
            task_id: t.id.clone(),
            reference: format!("task:{}", t.id),
            title: t.title.clone(),
            by: SWEEPER.into(),
            to: self.recipients(nt, SWEEPER),
            wake: true,
            text: format!(
                "The pull request on {} ({}) {what}: {pr}\n{next}{}",
                quote(&t.title),
                t.refs(),
                deeper(&t.id)
            ),
            ..Notice::default()
        }
    }
}
