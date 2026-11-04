//! Peer review across people on a shared board (v0.5 §10, decision 29):
//! an agent asks another person's agents to review a PR its card links,
//! and a person dedicates one of their agents as the reviewer for the
//! board or a repo. The review itself is posted by the reviewer's own
//! `gh`; Irori never posts reviews or merges, and its state (reviewing,
//! approved, changes requested) comes from the PR ledger, not from here.
//!
//! On the person's own board (#88) an agent asks another of their agents
//! to review a card's work, with or without a PR. The reviewer is woken
//! once and answers with a verdict comment (approved, or changes); the
//! card shows each reviewer's state, and the holders are woken only when
//! changes are asked.

use std::collections::BTreeMap;

use gocompat::time::GoTime;
use serde::Serialize;
use store::{Event, kinds};

use super::comments::check_comment;
use super::error::{Result, invalid, not_allowed, not_found};
use super::fold::Fold;
use super::fold::is_holder;
use super::links::pr_link;
use super::rows::{ReviewRequestedRow, ReviewReviewerRow, TaskCommentRow, TaskStateRow, row};
use super::types::*;
use super::util::{caller, deeper, new_id};
use crate::names::trim_space;
use gocompat::quote::quote;

/// A review request's notice kind.
pub const NOTICE_REVIEW: &str = "review";

/// A review's states on a private card (#88): asked and not yet answered,
/// approved, or changes asked.
pub const REVIEW_REQUESTED: &str = "requested";
pub const VERDICT_APPROVED: &str = "approved";
pub const VERDICT_CHANGES: &str = "changes";

/// One reviewer's review of a card as it stands: their newest request
/// and, when they answered it, their verdict.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub reviewer: String,
    /// Who asked (the newest request's asker).
    pub from: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub pr: String,
    /// requested, approved or changes.
    pub state: String,
    /// When it was asked, or answered.
    pub at: GoTime,
    /// The verdict comment, once there is one.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub comment_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
}

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

/// Why an agent on the person's own board can't be asked to review a card
/// that links a PR (card #471).
pub const OWN_PR_REVIEW: &str = "a PR of your person's is never reviewed by their own agents: merge it yourself once its checks pass (or let the merge desk), and where the repo needs an approval it comes from another member's agents on a shared board";

impl Fold {
    pub(crate) fn fold_review(&mut self, ev: &Event, at: &dyn Fn(GoTime) -> GoTime) {
        match ev.kind.as_str() {
            kinds::REVIEW_REQUESTED => {
                let Some(p) = row::<ReviewRequestedRow>(&ev.payload) else {
                    return;
                };
                if !self.tasks.contains_key(&p.task_id) || self.on_board && p.pr.is_empty() {
                    return;
                }
                // An agent asks from a device of its person. On a shared
                // board it never asks its own person's agents to approve;
                // on its own board it names one of them (#88).
                let theirs = if self.on_board {
                    p.to.is_empty() || {
                        let to = self.person_of(&p.to);
                        !to.is_empty() && to != self.person_of(&p.from)
                    }
                } else {
                    self.is_agent_key(&p.to) && p.to != p.from
                };
                if !self.is_agent_key(&p.from) || !self.may_write(&p.from, &ev.device_id) || !theirs
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

    /// With nobody named and no dedicated reviewer, a shared board's
    /// review goes to one free agent of another person (card #191,
    /// Stephen): live, idle, in the card's repo when both have one, holding
    /// no working card; the one idle longest. Never one of the asker's own
    /// person's agents, so never across one person's own devices.
    pub(crate) fn open_reviewer(&self, t: &Task, from: &str) -> Option<String> {
        let roster = self.roster.as_ref()?.agents();
        let mine = self.person_of(from);
        if mine.is_empty() {
            return None;
        }
        let others: Vec<_> = roster
            .into_iter()
            .filter(|a| {
                let p = self.person_of(&a.key);
                !p.is_empty() && p != mine
            })
            .collect();
        self.free_for(t, &others, &[]).into_iter().next()
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
        let by = caller(by)?;
        if !self.is_agent_key(&by) {
            return Err(invalid("an agent asks for a review"));
        }
        let (pr, to) = (trim_space(pr).to_string(), trim_space(to).to_string());
        let Some(t) = self.tasks.get(id).cloned() else {
            return Err(not_found(format!("task {id}")));
        };
        if self.on_board && pr.is_empty() {
            return Err(invalid("give pr: the pull request the card links"));
        }
        if !pr.is_empty()
            && !t
                .links
                .iter()
                .any(|l| pr_link(l).is_some() && trim_space(&l.reference) == pr)
        {
            return Err(invalid(format!(
                "{} doesn't link {pr}; board_link the PR first",
                t.short_ref()
            )));
        }
        if !self.on_board {
            return self.request_own_review(&by, &t, &pr, &to);
        }
        if !to.is_empty() && self.person_of(&to) == self.person_of(&by) {
            return Err(not_allowed(format!(
                "{to} is one of your own person's agents: they can comment, but a review comes from another person's agent"
            )));
        }
        // Card #471: an agent nobody on the board knows is no other
        // person's.
        if !to.is_empty() && self.person_of(&to).is_empty() {
            return Err(not_allowed(format!(
                "{to} isn't a member's agent on this board: a review comes from another person's agent"
            )));
        }
        let mut rcpt = self.review_recipients(&by, &to, &t.repo_key);
        if rcpt.is_empty() {
            rcpt = self.open_reviewer(&t, &by).into_iter().collect();
        }
        if rcpt.is_empty() {
            return Err(not_found(
                "nobody to review it: no other person's agent is free now, and none is dedicated; name one in to, or ask them to dedicate a reviewer",
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
        self.enter_review(&by, id)?;
        let n = Notice {
            kind: NOTICE_REVIEW.into(),
            task_id: id.into(),
            reference: format!("task:{id}"),
            title: t.title.clone(),
            to: rcpt,
            by: by.clone(),
            text: format!(
                "{} asks you to review {pr} for {}. Review it with your own gh; Irori never posts reviews or merges.",
                self.named(&by),
                t.short_ref()
            ),
            ..Notice::default()
        };
        Ok((out, n))
    }

    /// [`Fold::request_review`] on the person's own board (#88): one of
    /// their agents, named in `to`, reviews the card's work; the PR is
    /// optional. The reviewer is woken once.
    fn request_own_review(
        &mut self,
        by: &str,
        t: &Task,
        pr: &str,
        to: &str,
    ) -> Result<(Vec<ReviewRequest>, Notice)> {
        // Card #471: every agent on the person's own board is theirs, and
        // a PR of the person's is never reviewed or approved by their own
        // agents. It merges once its checks pass; where the repo wants an
        // approval, another member's agents give it.
        if !pr.is_empty() || t.links.iter().any(|l| pr_link(l).is_some()) {
            return Err(not_allowed(OWN_PR_REVIEW));
        }
        if to.is_empty() {
            return Err(invalid(
                "name the agent to review it in to: on your own board nobody is the reviewer by default",
            ));
        }
        if !self.is_agent_key(to) {
            return Err(invalid("an agent reviews: name one of your agents in to"));
        }
        if to == by {
            return Err(invalid(
                "ask another agent: a review is a second pair of eyes",
            ));
        }
        let at = self.stamp();
        let row = ReviewRequestedRow {
            task_id: t.id.clone(),
            pr: pr.into(),
            from: by.into(),
            to: to.into(),
            at,
        };
        self.write(by, kinds::REVIEW_REQUESTED, &row)?;
        self.enter_review(by, &t.id)?;
        let short = t.short_ref();
        let what = if pr.is_empty() {
            "its work".to_string()
        } else {
            pr.to_string()
        };
        let n = Notice {
            kind: NOTICE_REVIEW.into(),
            task_id: t.id.clone(),
            reference: format!("task:{}", t.id),
            title: t.title.clone(),
            to: vec![to.into()],
            by: by.into(),
            wake: true,
            text: format!(
                "{} asks you to review {} ({short}): {what}. Look at the work, then answer on the card: board_comment task_id={short} verdict=approved, or verdict=changes saying what needs changing.{}",
                self.named(by),
                quote(&t.title),
                deeper(&t.id)
            ),
            ..Notice::default()
        };
        let r = ReviewRequest {
            task_id: t.id.clone(),
            pr: pr.into(),
            from: by.into(),
            to: to.into(),
            at,
        };
        Ok((vec![r], n))
    }

    /// Refuses putting t in review (card #191) unless there is something to
    /// review: a PR it links, or a review already asked. A parent's state
    /// follows its subtasks, so it is never put in review by hand.
    pub(crate) fn review_refusal(&self, t: &Task) -> Result<()> {
        if !t.subtasks.is_empty() {
            return Err(invalid(format!(
                "{} is a parent: it shows in_review when its open subtasks are",
                t.short_ref()
            )));
        }
        let has_pr = t.links.iter().any(|l| pr_link(l).is_some());
        let asked = self.reviews.get(&t.id).is_some_and(|rs| !rs.is_empty());
        if !has_pr && !asked {
            return Err(invalid(format!(
                "{} has nothing to review yet: board_link its pull request, or ask for a review with board_review",
                t.short_ref()
            )));
        }
        Ok(())
    }

    /// Refuses an agent's completed on a card in review whose newest review
    /// isn't approved yet (card #191): the person shouldn't be asked "is
    /// this done?" while the work still waits on its reviewer. A card put in
    /// review with no review asked (a PR waiting on a merge desk) completes
    /// as usual once the agent says it is merged.
    pub(crate) fn unreviewed_refusal(&self, t: &Task) -> Result<()> {
        let open: Vec<String> = self
            .reviews_of(&t.id)
            .into_iter()
            .filter(|r| r.state != VERDICT_APPROVED)
            .map(|r| self.named(&r.reviewer))
            .collect();
        if open.is_empty() {
            return Ok(());
        }
        Err(invalid(format!(
            "{} is in review and {} hasn't approved it yet: wait for the approval (or the merge) before completed",
            t.short_ref(),
            open.join(", ")
        )))
    }

    /// Puts t in review after `by` asked for one (card #191): only a holder's
    /// request on a working card moves it; a watcher's leaves it as it is.
    fn enter_review(&mut self, by: &str, id: &str) -> Result<()> {
        let Some(t) = self.tasks.get(id) else {
            return Ok(());
        };
        if t.state != WORKING || !is_holder(t, by) || !t.subtasks.is_empty() {
            return Ok(());
        }
        let at = self.stamp();
        self.write(
            by,
            kinds::TASK_STATUS,
            &TaskStateRow {
                task_id: id.into(),
                state: IN_REVIEW.into(),
                note: "waiting on its review".into(),
                by: by.into(),
                at,
                ..TaskStateRow::default()
            },
        )?;
        Ok(())
    }

    /// An agent's key: `agent:<cardId>` on a shared board; on the private
    /// board any session key that isn't the person's.
    pub(crate) fn is_agent_key(&self, k: &str) -> bool {
        if self.on_board {
            return k.starts_with(kinds::AGENT_ACTOR_PREFIX);
        }
        !k.is_empty()
            && !self.is_owner(k)
            && !k.starts_with(kinds::PERSON_ACTOR_PREFIX)
            && k != super::util::SWEEPER
    }

    /// Whether a comment row is a verdict that counts: on the person's own
    /// board, from an agent asked to review that card, approved or changes.
    pub(crate) fn verdict_counts(&self, p: &TaskCommentRow) -> bool {
        !self.on_board
            && (p.verdict == VERDICT_APPROVED || p.verdict == VERDICT_CHANGES)
            && p.msg_id.is_empty()
            && p.blocker_id.is_empty()
            && self
                .reviews
                .get(&p.task_id)
                .is_some_and(|rs| rs.iter().any(|r| r.to == p.by))
    }

    /// A reviewer's verdict (#88): a comment on the card, approved or
    /// changes. The holders and whoever asked hear it; they are woken only
    /// when changes are asked.
    pub(crate) fn review_verdict(
        &mut self,
        by: &str,
        id: &str,
        input: CommentInput,
    ) -> Result<(Comment, Notice)> {
        let verdict = trim_space(&input.verdict).to_ascii_lowercase();
        if verdict != VERDICT_APPROVED && verdict != VERDICT_CHANGES {
            return Err(invalid("verdict is approved or changes"));
        }
        if self.on_board {
            return Err(invalid(
                "a verdict answers a review on your own board; on a shared card the review is posted on the PR with gh",
            ));
        }
        if !trim_space(&input.blocker_id).is_empty() {
            return Err(invalid(
                "a verdict is its own comment, not a reply on a blocker",
            ));
        }
        let (by, text) =
            check_comment(by, &input.text, &input.files, self.is_owner(trim_space(by)))?;
        let t = self.any_task(id)?.clone();
        let askers: Vec<String> = self
            .reviews
            .get(id)
            .map(|rs| {
                rs.iter()
                    .filter(|r| r.to == by)
                    .map(|r| r.from.clone())
                    .collect()
            })
            .unwrap_or_default();
        if askers.is_empty() {
            return Err(not_allowed(format!(
                "nobody asked you to review {}; give your view as a plain comment",
                t.short_ref()
            )));
        }
        let changes = verdict == VERDICT_CHANGES;
        let at = self.stamp();
        let cid = new_id(&at);
        let actor = self.actor_for(&by);
        let cm = self.write_comment(
            &actor,
            TaskCommentRow {
                task_id: id.into(),
                comment_id: cid.clone(),
                by: by.clone(),
                text: text.clone(),
                wake: changes || input.wake,
                at,
                attachments: input.files.clone(),
                verdict: verdict.clone(),
                ..TaskCommentRow::default()
            },
        )?;
        if changes && self.tasks[id].state == IN_REVIEW {
            let at = self.stamp();
            let note = format!("{} asked for changes", self.named(&by));
            let actor = self.actor_for(&by);
            self.write(
                &actor,
                kinds::TASK_STATUS,
                &TaskStateRow {
                    task_id: id.into(),
                    state: WORKING.into(),
                    note,
                    by: by.clone(),
                    at,
                    ..TaskStateRow::default()
                },
            )?;
        }
        let nt = self.tasks[id].clone();
        let mut to = self.recipients(&nt, &by);
        for a in askers {
            if a != by && !to.contains(&a) {
                to.push(a);
            }
        }
        let head = if changes {
            format!(
                "{} asks for changes on {} ({})",
                self.named(&by),
                quote(&t.title),
                t.refs()
            )
        } else {
            format!(
                "{} approved {} ({})",
                self.named(&by),
                quote(&t.title),
                t.refs()
            )
        };
        let tail = if changes {
            "\nMake the changes, then board_review again to ask for another look."
        } else {
            ""
        };
        let n = Notice {
            kind: NOTICE_COMMENT.into(),
            task_id: id.into(),
            reference: format!("comment:{cid}"),
            title: t.title.clone(),
            by: by.clone(),
            to,
            wake: changes || input.wake,
            attachments: input.files,
            text: format!("{head}:\n{text}{tail}{}", deeper(id)),
            ..Notice::default()
        };
        Ok((cm, n))
    }

    /// A card's reviews, one per reviewer in the order first asked: the
    /// newest request, answered by a verdict at or after it.
    pub(crate) fn reviews_of(&self, id: &str) -> Vec<Review> {
        let Some(rs) = self.reviews.get(id) else {
            return Vec::new();
        };
        let mut out: Vec<Review> = Vec::new();
        for r in rs {
            if r.to.is_empty() {
                continue;
            }
            match out.iter_mut().find(|v| v.reviewer == r.to) {
                Some(v) if v.at.unix_nano() <= r.at.unix_nano() => {
                    v.from = r.from.clone();
                    v.pr = r.pr.clone();
                    v.at = r.at;
                }
                Some(_) => {}
                None => out.push(Review {
                    reviewer: r.to.clone(),
                    from: r.from.clone(),
                    pr: r.pr.clone(),
                    state: REVIEW_REQUESTED.into(),
                    at: r.at,
                    ..Review::default()
                }),
            }
        }
        if self.on_board {
            return out;
        }
        let ids = self.by_task.get(id).cloned().unwrap_or_default();
        for v in &mut out {
            let asked = v.at.unix_nano();
            let newest = ids
                .iter()
                .filter_map(|c| self.comments.get(c))
                .filter(|c| {
                    c.by == v.reviewer && !c.verdict.is_empty() && c.at.unix_nano() >= asked
                })
                .max_by_key(|c| c.at.unix_nano());
            if let Some(c) = newest {
                v.state = c.verdict.clone();
                v.at = c.at;
                v.comment_id = c.id.clone();
                v.text = c.text.clone();
            }
        }
        out
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
