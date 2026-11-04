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
//! - GitHub couldn't be asked (no gh, no access): said once on the card,
//!   and an agent's completed falls back to the rules without it.
//!
//! An agent's completed is refused while a PR its card links is known to
//! be open ([`Fold::open_pr_refusal`]).

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

impl Fold {
    pub(crate) fn fold_pr_state(&mut self, ev: &Event) {
        let Some(p) = row::<PrStateRow>(&ev.payload) else {
            return;
        };
        if p.task_id.is_empty() || p.pr.is_empty() {
            return;
        }
        let by_pr = self.pr_states.entry(p.task_id.clone()).or_default();
        if let Some(old) = by_pr.get(&p.pr)
            && crate::gotime::before(&p.at, &old.at)
        {
            return;
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
            },
        );
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

    /// Refuses an agent's completed while a PR its card links is known to
    /// be open (card #208). A PR GitHub couldn't be asked about doesn't
    /// count: the rules without it apply.
    pub(crate) fn open_pr_refusal(&self, t: &Task) -> Result<()> {
        let open: Vec<String> = self
            .pr_states_of(t)
            .into_iter()
            .filter(|p| p.state == PR_OPEN)
            .map(|p| p.pr)
            .collect();
        if open.is_empty() {
            return Ok(());
        }
        Err(invalid(format!(
            "{} links a pull request that is still open ({}): set board_status in_review while it waits; Irori checks GitHub and wakes you when it merges",
            t.short_ref(),
            open.join(", ")
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
    /// changed, or when GitHub still can't be asked: that is said once.
    /// Returns the card and the notices to route.
    pub(crate) fn record_pr_state(&mut self, id: &str, c: PrCheck) -> Result<(Task, Vec<Notice>)> {
        let Some(t) = self.tasks.get(id).cloned() else {
            return Err(not_found(format!("task {}", quote(id))));
        };
        if !t
            .links
            .iter()
            .any(|l| l.reference == c.pr && pr_link(l).is_some())
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
        let unchanged = if c.error.is_empty() {
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
        let mut ns = Vec::new();
        let newly = |s: &str| c.state == s && old.state != s;
        if newly(PR_MERGED) {
            // Another linked PR still open (the card's own, with the merged
            // one linked as context, #230): say it merged, but don't ask
            // for completed and don't wake anyone over it.
            let waiting = self.unfinished_prs_besides(&t, &c.pr);
            if waiting.is_empty() {
                ns.push(self.pr_notice(
                    &t,
                    &c.pr,
                    "was merged",
                    format!(
                        "Check the work landed, then set it completed with proof: board_status task_id={} state=completed proof=\"…\".",
                        t.id
                    ),
                ));
            } else {
                let mut n = self.pr_notice(
                    &t,
                    &c.pr,
                    "was merged",
                    format!(
                        "The card also links {}, not merged yet: keep it as it is; Irori tells you when that merges.",
                        waiting.join(", ")
                    ),
                );
                n.wake = false;
                ns.push(n);
            }
        } else if newly(PR_CLOSED) {
            ns.push(self.pr_notice(
                &t,
                &c.pr,
                "was closed without merging",
                "Say on the card what happens to the work now: reopen it, link the PR that replaces it, or ask the person.".into(),
            ));
        } else if c.review == PR_CHANGES_REQUESTED && old.review != PR_CHANGES_REQUESTED {
            if t.state == IN_REVIEW {
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
            ns.push(self.pr_notice(
                &t,
                &c.pr,
                "has changes requested on GitHub",
                "Read the review with `gh pr view --comments`, make the changes, push, then set board_status in_review again.".into(),
            ));
        }
        ns.retain(|n| !n.to.is_empty());
        Ok((self.tasks[id].clone(), ns))
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
                    "its {n} is still open and nobody is on it: merge it once its checks pass, or complete the card if it merged"
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
