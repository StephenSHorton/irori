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
use super::resource::pr_claim_resource;
use super::rows::{ReviewRequestedRow, ReviewReviewerRow, TaskCommentRow, TaskStateRow, row};
use super::types::*;
use super::util::{caller, deeper, new_id};
use crate::names::trim_space;
use gocompat::fnv::fnv32a;
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
    /// A ping (merge desk §12.7): look at the PR; no review is asked.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub look: bool,
    /// Whether the row wakes an idle recipient.
    #[serde(skip)]
    pub wake: bool,
    /// The row's place in the log: the bridge's key is its device and seq.
    #[serde(skip)]
    pub pos: RowPos,
}

/// The reviewers people dedicated: `(person, repoKey or "")` to the
/// newest row's place and agent.
pub(crate) type Reviewers = BTreeMap<(String, String), (RowPos, String)>;

/// Why an agent on the person's own board can't be asked to review a card
/// that links a PR (card #471).
pub const OWN_PR_REVIEW: &str = "a PR of your person's is never reviewed by their own agents: ask them “Can I merge…?” with board_block and merge it yourself on their yes, and where the repo needs an approval it comes from another member's agents in a shared space";

impl Fold {
    pub(crate) fn fold_review(&mut self, ev: &Event, at: &dyn Fn(GoTime) -> GoTime) {
        match ev.kind.as_str() {
            kinds::REVIEW_REQUESTED => {
                let Some(p) = row::<ReviewRequestedRow>(&ev.payload) else {
                    return;
                };
                let Some(t) = self.tasks.get(&p.task_id) else {
                    return;
                };
                if self.on_board && p.pr.is_empty() {
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
                // Merge desk §12.7: on a board the PR card's creator asks
                // as their person, from a device of theirs.
                let from_ok = if self.on_board && p.from.starts_with(kinds::PERSON_ACTOR_PREFIX) {
                    self.person_row(ev, &p.from) && pr_card(t) && t.created_by == p.from
                } else {
                    self.is_agent_key(&p.from) && self.may_write(&p.from, &ev.device_id)
                };
                // §12.3: never the PR author's own agents, on every device.
                // The author is whoever made a PR card for the row's PR,
                // standing or not (card #1155): which card stands turns on
                // profiles, and a row's fold must turn on rows alone.
                let author_ok = !self.on_board
                    || p.to.is_empty()
                    || pr_claim_resource(&p.pr).is_none_or(|res| {
                        !self.pr_card_makers(&res).contains(&self.person_of(&p.to))
                    });
                // On a PR card, an ask is for that card's PR; a ping
                // (§12.7) only ever from its creator as their person.
                let pr_ok = !self.on_board
                    || t.pr_card
                        .as_ref()
                        .is_none_or(|c| pr_claim_resource(&c.pr) == pr_claim_resource(&p.pr));
                let look_ok =
                    !p.look || pr_card(t) && p.from.starts_with(kinds::PERSON_ACTOR_PREFIX);
                if !from_ok || !theirs || !author_ok || !pr_ok || !look_ok {
                    self.ignore(&ev.device_id);
                    return;
                }
                let r = ReviewRequest {
                    task_id: p.task_id.clone(),
                    pr: p.pr,
                    from: p.from,
                    to: p.to,
                    at: at(p.at),
                    look: p.look,
                    wake: p.wake,
                    pos: RowPos::of(ev),
                };
                let rs = self.reviews.entry(p.task_id.clone()).or_default();
                if !rs.iter().any(|x| x.pos == r.pos) {
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
                let key = (person, ledger::canonical_repo_key(&p.repo_key));
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

    /// A card's review asks, oldest folded first, without pings: a ping
    /// never counts as an ask (merge desk §12.7).
    pub(crate) fn review_asks<'a>(&'a self, id: &str) -> impl Iterator<Item = &'a ReviewRequest> {
        self.reviews
            .get(id)
            .into_iter()
            .flatten()
            .filter(|r| !r.look)
    }

    /// The request this device just wrote, as folded: the newest of its
    /// own rows that says the same.
    pub(crate) fn read_back(&self, id: &str, want: &ReviewRequest) -> Option<ReviewRequest> {
        self.reviews
            .get(id)?
            .iter()
            .filter(|r| {
                r.pos.dev == self.dev
                    && (&r.from, &r.to, &r.pr, r.at, r.look)
                        == (&want.from, &want.to, &want.pr, want.at, want.look)
            })
            .max_by(|a, b| a.pos.cmp(&b.pos))
            .cloned()
    }

    /// With nobody named and no dedicated reviewer, a shared board's
    /// review goes to one free agent of another person (card #191,
    /// Stephen): live, idle, in the card's repo when both have one, holding
    /// no working card. Never one of the asker's own person's agents, so
    /// never across one person's own devices. The person with the fewest
    /// open claims, then the fewest in 14 days, goes first; then a hash of
    /// the PR and the agent's key (merge desk §12.6).
    pub(crate) fn open_reviewer(&self, t: &Task, from: &str, pr: &str) -> Option<String> {
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
        let counts = self.claim_counts(&self.stamp());
        let mut free = self.free_for(t, &others, &[]);
        free.sort_by_cached_key(|k| {
            let c = counts.get(&self.person_of(k)).copied().unwrap_or_default();
            (c, fnv32a(format!("{pr}{k}").as_bytes()), k.clone())
        });
        free.into_iter().next()
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
        // Merge desk §12.5: a PR with a PR card here gets its reviewer
        // from the author's poll device, so with no to nothing is asked.
        let pc = if pr_card(&t) {
            Some(t.clone())
        } else {
            self.pr_card_for(&pr).cloned()
        };
        if to.is_empty() && pc.is_some() {
            self.enter_review(&by, id)?;
            return Ok((Vec::new(), Notice::default()));
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
                "{to} isn't a member's agent in this space: a review comes from another person's agent"
            )));
        }
        // §12.3, advisory: never the PR author's own agents.
        if let Some(pc) = &pc
            && !to.is_empty()
            && self.person_of(&to) == self.person_of(&pc.created_by)
        {
            return Err(not_allowed(format!(
                "{to} is the PR author's own agent: a review comes from another person's agent"
            )));
        }
        let mut rcpt = self.review_recipients(&by, &to, &t.repo_key);
        if rcpt.is_empty() {
            rcpt = self.open_reviewer(&t, &by, &pr).into_iter().collect();
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
                    wake: true,
                    look: false,
                },
            )?;
            let want = ReviewRequest {
                task_id: id.into(),
                pr: pr.clone(),
                from: by.clone(),
                to: to.clone(),
                at,
                wake: true,
                ..ReviewRequest::default()
            };
            out.push(self.read_back(id, &want).unwrap_or(want));
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
                "name the agent to review it in to: on a private card nobody is the reviewer by default",
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
            // Private: the notice wakes the reviewer once.
            wake: false,
            look: false,
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
        let want = ReviewRequest {
            task_id: t.id.clone(),
            pr: pr.into(),
            from: by.into(),
            to: to.into(),
            at,
            ..ReviewRequest::default()
        };
        let r = self.read_back(&t.id, &want).unwrap_or(want);
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
        let asked = self.review_asks(&t.id).next().is_some();
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
    /// review with no review asked (a PR waiting on the person's merge yes) completes
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
            && self.review_asks(&p.task_id).any(|r| r.to == p.by)
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
                "a verdict answers a review on a private card; on a shared card the review is posted on the PR with gh",
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
            .review_asks(id)
            .filter(|r| r.to == by)
            .map(|r| r.from.clone())
            .collect();
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
        let mut out: Vec<Review> = Vec::new();
        for r in self.review_asks(id) {
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
            return Err(invalid("a dedicated reviewer is set in a shared space"));
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
                "{reviewer} isn't one of your agents in this space"
            )));
        }
        let repo = ledger::canonical_repo_key(repo);
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
