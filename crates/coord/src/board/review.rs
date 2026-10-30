//! Peer review across people on a shared board (v0.5 §10, decision 29):
//! an agent asks another person's agents to review a PR its card links,
//! and a person dedicates one of their agents as the reviewer for the
//! board or a repo. The review itself is posted by the reviewer's own
//! `gh`; Irori never posts reviews or merges, and its state (reviewing,
//! approved, changes requested) comes from the PR ledger, not from here.

use std::collections::BTreeMap;

use gocompat::time::GoTime;
use serde::Serialize;
use store::{Event, kinds};

use super::error::{Result, invalid, not_allowed, not_found};
use super::fold::Fold;
use super::links::pr_link;
use super::rows::{ReviewRequestedRow, ReviewReviewerRow, row};
use super::types::*;
use super::util::caller;
use crate::names::trim_space;

/// A review request's notice kind.
pub const NOTICE_REVIEW: &str = "review";

/// One review request as folded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRequest {
    pub task_id: String,
    pub pr: String,
    pub from: String,
    /// The agent asked: named, or a dedicated reviewer the request was
    /// resolved to when written. Empty only on rows older than that.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub to: String,
    pub at: GoTime,
}

/// The reviewers people dedicated: `(person, repoKey or "")` to the
/// newest row's place and agent.
pub(crate) type Reviewers = BTreeMap<(String, String), (RowPos, String)>;

impl Fold {
    pub(crate) fn fold_review(&mut self, ev: &Event, at: &dyn Fn(GoTime) -> GoTime) {
        match ev.kind.as_str() {
            kinds::REVIEW_REQUESTED => {
                let Some(p) = row::<ReviewRequestedRow>(&ev.payload) else {
                    return;
                };
                if !self.on_board || !self.tasks.contains_key(&p.task_id) || p.pr.is_empty() {
                    return;
                }
                // An agent asks from a device of its person, and never asks
                // its own person's agents to approve.
                let theirs = p.to.is_empty() || self.person_of(&p.to) != self.person_of(&p.from);
                if !p.from.starts_with(kinds::AGENT_ACTOR_PREFIX)
                    || !self.may_write(&p.from, &ev.device_id)
                    || !theirs
                {
                    self.ignore(&ev.device_id);
                    return;
                }
                let r = ReviewRequest {
                    task_id: p.task_id.clone(),
                    pr: p.pr,
                    from: p.from,
                    to: p.to,
                    at: at(p.at),
                };
                let rs = self.reviews.entry(p.task_id.clone()).or_default();
                if !rs.contains(&r) {
                    rs.push(r);
                }
                self.note(Change {
                    kind: TASK_CHANGED.into(),
                    task_id: p.task_id,
                    ..Change::default()
                });
            }
            kinds::REVIEW_REVIEWER => {
                let Some(p) = row::<ReviewReviewerRow>(&ev.payload) else {
                    return;
                };
                let person = self.person_of(&p.by);
                if !self.on_board
                    || !self.person_row(ev, &p.by)
                    || !p.reviewer.is_empty() && self.person_of(&p.reviewer) != person
                {
                    self.ignore(&ev.device_id);
                    return;
                }
                let pos = RowPos::of(ev);
                let key = (person, p.repo_key);
                if self
                    .reviewers
                    .get(&key)
                    .is_some_and(|(w, _)| !w.before(&pos))
                {
                    return;
                }
                self.reviewers.insert(key, (pos, p.reviewer));
            }
            _ => {}
        }
    }

    /// A person's dedicated reviewer for repo: the repo's, else the
    /// board's.
    pub(crate) fn reviewer_of(&self, person: &str, repo: &str) -> String {
        let get = |r: &str| {
            self.reviewers
                .get(&(person.to_string(), r.to_string()))
                .map(|(_, a)| a.clone())
                .unwrap_or_default()
        };
        let a = if repo.is_empty() {
            String::new()
        } else {
            get(repo)
        };
        if a.is_empty() { get("") } else { a }
    }

    /// Who a request reaches: its `to`, else every other person's
    /// dedicated reviewer for the card's repo.
    pub(crate) fn review_recipients(&self, from: &str, to: &str, repo: &str) -> Vec<String> {
        if !to.is_empty() {
            return vec![to.to_string()];
        }
        let mine = self.person_of(from);
        let mut people: Vec<&String> = self.reviewers.keys().map(|(p, _)| p).collect();
        people.dedup();
        let mut out: Vec<String> = people
            .into_iter()
            .filter(|p| **p != mine)
            .map(|p| self.reviewer_of(p, repo))
            .filter(|a| !a.is_empty())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// An agent asking another person's agents to review a PR its card
    /// links: `to`, or each other person's dedicated reviewer for the
    /// card's repo, one request each.
    pub(crate) fn request_review(
        &mut self,
        by: &str,
        id: &str,
        pr: &str,
        to: &str,
    ) -> Result<(Vec<ReviewRequest>, Notice)> {
        if !self.on_board {
            return Err(invalid(
                "review requests go to other people's agents on a shared board",
            ));
        }
        let by = caller(by)?;
        if !by.starts_with(kinds::AGENT_ACTOR_PREFIX) {
            return Err(invalid("an agent asks for a review"));
        }
        let (pr, to) = (trim_space(pr).to_string(), trim_space(to).to_string());
        let Some(t) = self.tasks.get(id).cloned() else {
            return Err(not_found(format!("task {id}")));
        };
        if !t
            .links
            .iter()
            .any(|l| pr_link(l).is_some() && trim_space(&l.reference) == pr)
        {
            return Err(invalid(format!(
                "{} doesn't link {pr}; board_link the PR first",
                t.short_ref()
            )));
        }
        if !to.is_empty() && self.person_of(&to) == self.person_of(&by) {
            return Err(not_allowed(format!(
                "{to} is one of your own person's agents: they can comment, but a review comes from another person's agent"
            )));
        }
        let rcpt = self.review_recipients(&by, &to, &t.repo_key);
        if rcpt.is_empty() {
            return Err(not_found(
                "nobody to review it: name another person's agent in to, or ask them to dedicate a reviewer",
            ));
        }
        // One row per recipient, each naming its agent: the bridge
        // delivers a request only to the agent its row names, so a bare
        // "the dedicated reviewer" row would reach nobody.
        let at = self.stamp();
        let mut out = Vec::with_capacity(rcpt.len());
        for to in &rcpt {
            self.write(
                &by,
                kinds::REVIEW_REQUESTED,
                &ReviewRequestedRow {
                    task_id: id.into(),
                    pr: pr.clone(),
                    from: by.clone(),
                    to: to.clone(),
                    at,
                },
            )?;
            out.push(ReviewRequest {
                task_id: id.into(),
                pr: pr.clone(),
                from: by.clone(),
                to: to.clone(),
                at,
            });
        }
        let n = Notice {
            kind: NOTICE_REVIEW.into(),
            task_id: id.into(),
            reference: format!("task:{id}"),
            title: t.title.clone(),
            to: rcpt,
            by: by.clone(),
            text: format!(
                "{by} asks you to review {pr} for {}. Review it with your own gh; Irori never posts reviews or merges.",
                t.short_ref()
            ),
            ..Notice::default()
        };
        Ok((out, n))
    }

    /// A person dedicating one of their own agents as their reviewer for
    /// the board, or for repo; an empty reviewer clears it.
    pub(crate) fn set_reviewer(&mut self, by: &str, reviewer: &str, repo: &str) -> Result<()> {
        if !self.on_board {
            return Err(invalid("a dedicated reviewer is set on a shared board"));
        }
        let by = caller(by)?;
        if !by.starts_with(kinds::PERSON_ACTOR_PREFIX) || !self.is_owner(&by) {
            return Err(not_allowed(
                "only the person dedicates a reviewer, in the app",
            ));
        }
        let reviewer = trim_space(reviewer).to_string();
        if !reviewer.is_empty()
            && (!reviewer.starts_with(kinds::AGENT_ACTOR_PREFIX)
                || self.person_of(&reviewer) != self.person_of(&by))
        {
            return Err(not_allowed(format!(
                "{reviewer} isn't one of your agents on this board"
            )));
        }
        let repo = trim_space(repo).to_string();
        if !repo.is_empty() {
            super::resource::check_repo_key(&repo)?;
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::REVIEW_REVIEWER,
            &ReviewReviewerRow {
                reviewer,
                repo_key: repo,
                by: by.clone(),
                at,
            },
        )
    }
}
