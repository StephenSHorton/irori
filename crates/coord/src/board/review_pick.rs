//! Picking a PR's reviewer (merge desk §12.5 to §12.7, card #1079): the
//! pick state read from rows alone, the person's ask and ping, the claim
//! tally, the declines, and the facts the author's poll device ranks
//! from. The ranking and the device triggers are the Core's.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use gocompat::time::GoTime;
use store::kinds;

use super::error::{Result, invalid, not_allowed, not_found};
use super::fold::Fold;
use super::links::pr_link;
use super::prstate::{PR_CHANGES_REQUESTED, PR_CLOSED, PR_MERGED, PrState};
use super::resource::{RESOURCE_PATH, is_pr_claim, pr_claim_resource};
use super::review::ReviewRequest;
use super::rows::ReviewRequestedRow;
use super::types::*;
use super::util::caller;
use crate::gotime;
use crate::names::trim_space;

/// How far back a person's claims count (merge desk §12.6).
pub const CLAIM_TALLY: Duration = Duration::from_secs(14 * 24 * 3600);

/// GitHub's approving review state and decision.
const APPROVED: &str = "APPROVED";

/// A `pr/` claim as first seen: its acquired_at and the row that first
/// carried it. A renewal keeps both.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaimSeen {
    pub acquired_at: GoTime,
    pub pos: RowPos,
}

/// One person's claims on this board (merge desk §12.6): live now, and
/// distinct PRs claimed in the last 14 days. Fewer sorts first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClaimCounts {
    pub open: usize,
    pub recent: usize,
}

/// Where a PR card's review stands, from its rows alone (merge desk
/// §12.5). The author's poll device adds the triggers only it sees.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ReviewPhase {
    /// Not a standing open PR card, a merged or closed PR, or approved.
    #[default]
    Off,
    /// Nobody asked, claimed or reviewed at the head.
    Waiting,
    /// The newest ask, still out.
    Asked { ask: ReviewRequest },
    /// A live claim.
    Claimed { lease: Lease },
    /// A verdict at the PR's head.
    Reviewed,
}

/// An ask a device may have missed live (merge desk §12.8): the newest
/// ask on a PR card still asked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MissedAsk {
    pub card: String,
    pub pr: String,
    pub from: String,
    pub to: String,
    pub pos: RowPos,
    pub wake: bool,
}

/// One claim ever taken on the PR.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PastClaim {
    pub holder: String,
    pub acquired_at: GoTime,
    pub pos: RowPos,
}

/// Why a card bears on the PR, for tier 1.
pub const RELATED_PR_CARD: &str = "pr";
pub const RELATED_LINKS: &str = "links";
pub const RELATED_PARENT: &str = "parent";
pub const RELATED_SIBLING: &str = "sibling";

/// A card on the same work as the PR (tier 1): the PR card, an open card
/// linking the PR, or a parent or sibling of one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelatedCard {
    pub id: String,
    /// One of the RELATED_ consts.
    pub why: &'static str,
    pub state: String,
    pub holders: Vec<String>,
    pub co_holders: Vec<String>,
    pub assigned: Vec<String>,
    pub watchers: Vec<String>,
    /// Who commented, a decline aside.
    pub commenters: Vec<String>,
    pub scope: Vec<String>,
}

/// What the author's poll device ranks a PR card's candidates from,
/// read under one lock (merge desk §12.5).
#[derive(Debug, Clone, Default)]
pub struct PickFacts {
    pub card: Task,
    pub pr: String,
    /// The card creator's person: the PR author's.
    pub author_person: String,
    /// The card's creation row.
    pub created_pos: Option<RowPos>,
    pub pr_state: PrState,
    pub phase: ReviewPhase,
    /// The asks for this PR naming an agent, by row place; pings left
    /// out. The last one's `pos` is the `after` a re-pick passes to
    /// [`super::Coord::pick_ask`], in Waiting as in Asked.
    pub asks: Vec<ReviewRequest>,
    /// The agents pinged on this card.
    pub pinged: BTreeSet<String>,
    /// Each agent's newest decline of this PR.
    pub declined: BTreeMap<String, RowPos>,
    /// Every claim taken on this PR, by row place.
    pub claims: Vec<PastClaim>,
    pub live_claim: Option<Lease>,
    /// Claim counts by person.
    pub counts: BTreeMap<String, ClaimCounts>,
    pub related: Vec<RelatedCard>,
    /// (holder or co-holder, scope glob) of every open card.
    pub scopes: Vec<(String, String)>,
    /// (holder, glob) of every live mirrored path lease.
    pub path_leases: Vec<(String, String)>,
    /// Each person's dedicated reviewer for the card's repo.
    pub reviewers: BTreeMap<String, String>,
}

impl Fold {
    /// Notes a `pr/` claim row: a newer acquired_at replaces the one
    /// known; the same one keeps the earliest row.
    pub(crate) fn see_claim(&mut self, holder: &str, name: &str, at: GoTime, pos: RowPos) {
        let key = (holder.to_string(), name.to_string());
        match self.pr_claims.get_mut(&key) {
            Some(w) if gotime::cmp(&w.acquired_at, &at).is_eq() => {
                if pos < w.pos {
                    w.pos = pos;
                }
            }
            Some(w) if gotime::after(&w.acquired_at, &at) => {}
            _ => {
                self.pr_claims.insert(
                    key,
                    ClaimSeen {
                        acquired_at: at,
                        pos,
                    },
                );
            }
        }
    }

    /// Claim counts by person (merge desk §12.6): live `pr/` claims, and
    /// distinct PRs claimed within [`CLAIM_TALLY`] of now.
    pub(crate) fn claim_counts(&self, now: &GoTime) -> BTreeMap<String, ClaimCounts> {
        let mut out: BTreeMap<String, ClaimCounts> = BTreeMap::new();
        for l in self.held.values() {
            if !is_pr_claim(&l.lease.resource) || !self.live(l, now) {
                continue;
            }
            let p = self.person_of(&l.lease.holder);
            if !p.is_empty() {
                out.entry(p).or_default().open += 1;
            }
        }
        let mut recent: BTreeSet<(String, &str)> = BTreeSet::new();
        for ((holder, res), c) in &self.pr_claims {
            if gotime::after(&gotime::add(&c.acquired_at, CLAIM_TALLY), now) {
                let p = self.person_of(holder);
                if !p.is_empty() {
                    recent.insert((p, res));
                }
            }
        }
        for (p, _) in recent {
            out.entry(p).or_default().recent += 1;
        }
        out
    }

    /// Each agent's newest decline of the PR card's PR (merge desk §12.3).
    pub(crate) fn declined(&self, card: &str) -> BTreeMap<String, RowPos> {
        let mut out: BTreeMap<String, RowPos> = BTreeMap::new();
        let Some(want) = self
            .tasks
            .get(card)
            .and_then(|t| t.pr_card.as_ref())
            .and_then(|c| pr_claim_resource(&c.pr))
        else {
            return out;
        };
        for c in self.comments.values() {
            if c.declines.is_empty() || pr_claim_resource(&c.declines).as_deref() != Some(&want) {
                continue;
            }
            let pos = c.pos();
            match out.get_mut(&c.by) {
                Some(w) if *w >= pos => {}
                Some(w) => *w = pos,
                None => {
                    out.insert(c.by.clone(), pos);
                }
            }
        }
        out
    }

    /// The card and its PR when it is the standing PR card for that PR.
    fn standing_pr_card(&self, id: &str) -> Option<(&Task, &PrCard)> {
        let t = self.tasks.get(id)?;
        let c = t.pr_card.as_ref()?;
        self.pr_card_for(&c.pr)
            .is_some_and(|s| s.id == t.id)
            .then_some((t, c))
    }

    /// The newest ask on the card for the PR `res` names, by row place,
    /// so every device agrees.
    fn newest_ask(&self, id: &str, res: &str) -> Option<&ReviewRequest> {
        self.review_asks(id)
            .filter(|r| !r.to.is_empty() && pr_claim_resource(&r.pr).as_deref() == Some(res))
            .max_by(|a, b| a.pos.cmp(&b.pos))
    }

    /// Where the PR card's review stands, from rows alone (merge desk
    /// §12.5): see [`ReviewPhase`].
    pub(crate) fn review_phase(&self, id: &str) -> ReviewPhase {
        if !self.on_board {
            return ReviewPhase::Off;
        }
        let Some((t, c)) = self.standing_pr_card(id) else {
            return ReviewPhase::Off;
        };
        let Some(res) = pr_claim_resource(&c.pr) else {
            return ReviewPhase::Off;
        };
        if is_terminal(&t.state) {
            return ReviewPhase::Off;
        }
        let st = self
            .pr_states
            .get(id)
            .and_then(|m| m.get(&c.pr))
            .cloned()
            .unwrap_or_default();
        if st.state == PR_MERGED || st.state == PR_CLOSED {
            return ReviewPhase::Off;
        }
        let at_head = |states: &[&str]| {
            !st.head.is_empty()
                && st
                    .reviews
                    .iter()
                    .any(|r| r.sha == st.head && states.contains(&r.state.as_str()))
        };
        if st.review == APPROVED || st.review.is_empty() && at_head(&[APPROVED]) {
            return ReviewPhase::Off;
        }
        let now = self.stamp();
        if let Some(l) = self.held.get(&res).filter(|l| self.live(l, &now)) {
            return ReviewPhase::Claimed {
                lease: l.lease.clone(),
            };
        }
        if at_head(&[APPROVED, PR_CHANGES_REQUESTED]) {
            return ReviewPhase::Reviewed;
        }
        if let Some(a) = self.newest_ask(id, &res) {
            let declined = self.declined(id).get(&a.to).is_some_and(|p| *p > a.pos);
            let claimed = self
                .pr_claims
                .iter()
                .any(|((_, r), s)| *r == res && s.pos > a.pos);
            if !declined && !claimed {
                return ReviewPhase::Asked { ask: a.clone() };
            }
        }
        ReviewPhase::Waiting
    }

    /// The person's own ask or ping on their PR card (merge desk §12.5,
    /// §12.7), from the author's poll device. A ping goes once per agent
    /// on the card. An ask is written only while the newest ask is still
    /// `after`, the one the caller decided on, so one decision asks once.
    /// Returns the request as folded, or None when nothing was written.
    pub(crate) fn pick_ask(
        &mut self,
        by: &str,
        card: &str,
        to: &str,
        look: bool,
        after: Option<RowPos>,
    ) -> Result<Option<ReviewRequest>> {
        if !self.on_board {
            return Err(invalid("a reviewer is picked on a shared board"));
        }
        let by = caller(by)?;
        if !by.starts_with(kinds::PERSON_ACTOR_PREFIX) || !self.is_owner(&by) {
            return Err(not_allowed("only the person's Irori picks a reviewer"));
        }
        let Some(t) = self.tasks.get(card) else {
            return Err(not_found(format!("task {card}")));
        };
        let Some((t, c)) = self.standing_pr_card(card) else {
            return Err(invalid(format!(
                "{} isn't the standing PR card for a pull request",
                t.short_ref()
            )));
        };
        if is_terminal(&t.state) {
            return Err(invalid(format!("{} is closed", t.short_ref())));
        }
        if t.created_by != by {
            return Err(not_allowed(format!(
                "{} is another person's PR card",
                t.short_ref()
            )));
        }
        let pr = c.pr.clone();
        let Some(res) = pr_claim_resource(&pr) else {
            return Err(invalid(format!("{pr} is not a GitHub pull request URL")));
        };
        let to = trim_space(to).to_string();
        let theirs = self.person_of(&to);
        if !to.starts_with(kinds::AGENT_ACTOR_PREFIX)
            || theirs.is_empty()
            || theirs == self.person_of(&by)
        {
            return Err(not_allowed(format!(
                "{to} isn't another member's agent in this space"
            )));
        }
        if look {
            let pinged = self
                .reviews
                .get(card)
                .is_some_and(|rs| rs.iter().any(|r| r.look && r.to == to));
            if pinged {
                return Ok(None);
            }
        } else if self.newest_ask(card, &res).map(|a| &a.pos) != after.as_ref() {
            return Ok(None);
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::REVIEW_REQUESTED,
            &ReviewRequestedRow {
                task_id: card.into(),
                pr: pr.clone(),
                from: by.clone(),
                to: to.clone(),
                at,
                wake: !look,
                look,
            },
        )?;
        let want = ReviewRequest {
            task_id: card.into(),
            pr,
            from: by,
            to,
            at,
            look,
            wake: !look,
            ..ReviewRequest::default()
        };
        match self.read_back(card, &want) {
            Some(r) => Ok(Some(r)),
            None => Err(invalid("the review ask wasn't taken here")),
        }
    }

    /// The standing open PR cards in Waiting or Asked, with their author's
    /// person: where another member's idle agents are wanted.
    pub(crate) fn wanting_review(&self) -> Vec<(String, String)> {
        self.tasks
            .values()
            .filter(|t| t.pr_card.is_some())
            .filter(|t| {
                matches!(
                    self.review_phase(&t.id),
                    ReviewPhase::Waiting | ReviewPhase::Asked { .. }
                )
            })
            .map(|t| (t.id.clone(), self.person_of(&t.created_by)))
            .collect()
    }

    /// The newest ask on each standing open PR card still in Asked, when
    /// the card links its PR: what a device may have missed live (merge
    /// desk §12.8). Never a ping.
    pub(crate) fn missed_review_asks(&self) -> Vec<MissedAsk> {
        let mut out = Vec::new();
        for t in self.tasks.values().filter(|t| t.pr_card.is_some()) {
            let ReviewPhase::Asked { ask } = self.review_phase(&t.id) else {
                continue;
            };
            let links = t
                .links
                .iter()
                .any(|l| pr_link(l).is_some() && trim_space(&l.reference) == ask.pr);
            if links {
                out.push(MissedAsk {
                    card: t.id.clone(),
                    pr: ask.pr,
                    from: ask.from,
                    to: ask.to,
                    pos: ask.pos,
                    wake: ask.wake,
                });
            }
        }
        out
    }

    /// What the author's poll device ranks the PR card's candidates from
    /// (merge desk §12.5); None for a card that is no PR card.
    pub(crate) fn pick_facts(&self, card: &str) -> Option<PickFacts> {
        let t = self.tasks.get(card)?;
        let pr = t.pr_card.as_ref()?.pr.clone();
        let res = pr_claim_resource(&pr)?;
        let now = self.stamp();
        let mut asks: Vec<ReviewRequest> = self
            .review_asks(card)
            .filter(|r| {
                !r.to.is_empty() && pr_claim_resource(&r.pr).as_deref() == Some(res.as_str())
            })
            .cloned()
            .collect();
        asks.sort_by(|a, b| a.pos.cmp(&b.pos));
        let pinged = self
            .reviews
            .get(card)
            .into_iter()
            .flatten()
            .filter(|r| r.look)
            .map(|r| r.to.clone())
            .collect();
        let mut claims: Vec<PastClaim> = self
            .pr_claims
            .iter()
            .filter(|((_, r), _)| *r == res)
            .map(|((h, _), c)| PastClaim {
                holder: h.clone(),
                acquired_at: c.acquired_at,
                pos: c.pos.clone(),
            })
            .collect();
        claims.sort_by(|a, b| a.pos.cmp(&b.pos));
        let live_claim = self
            .held
            .get(&res)
            .filter(|l| self.live(l, &now))
            .map(|l| l.lease.clone());
        let scopes = self
            .tasks
            .values()
            .filter(|x| !is_terminal(&x.state))
            .flat_map(|x| {
                x.holders
                    .iter()
                    .chain(&x.co_holders)
                    .flat_map(move |h| x.scope.iter().map(move |g| (h.clone(), g.clone())))
            })
            .collect();
        let path_leases = self
            .held
            .values()
            .filter(|l| l.res.kind == RESOURCE_PATH && self.live(l, &now))
            .map(|l| (l.lease.holder.clone(), l.res.glob.clone()))
            .collect();
        let mut reviewers = BTreeMap::new();
        for (p, _) in self.reviewers.keys() {
            let a = self.reviewer_of(p, &t.repo_key);
            if !a.is_empty() {
                reviewers.insert(p.clone(), a);
            }
        }
        Some(PickFacts {
            card: self.shown(t),
            author_person: self.person_of(&t.created_by),
            created_pos: self.created_pos.get(card).cloned(),
            pr_state: self
                .pr_states
                .get(card)
                .and_then(|m| m.get(&pr))
                .cloned()
                .unwrap_or_default(),
            phase: self.review_phase(card),
            asks,
            pinged,
            declined: self.declined(card),
            claims,
            live_claim,
            counts: self.claim_counts(&now),
            related: self.related_cards(card, &res),
            scopes,
            path_leases,
            reviewers,
            pr,
        })
    }

    /// The cards on the same work as the PR (tier 1): the PR card, every
    /// open card linking the PR, and their parents and siblings.
    fn related_cards(&self, card: &str, res: &str) -> Vec<RelatedCard> {
        let mut base: Vec<(&str, &'static str)> = vec![(card, RELATED_PR_CARD)];
        for t in self.tasks.values() {
            let links = t.links.iter().any(|l| {
                pr_link(l).is_some() && pr_claim_resource(&l.reference).as_deref() == Some(res)
            });
            if t.id != card && links && !is_terminal(&t.state) {
                base.push((&t.id, RELATED_LINKS));
            }
        }
        let mut ids = base.clone();
        for (id, _) in &base {
            let Some(parent) = self.tasks.get(*id).map(|t| t.parent.as_str()) else {
                continue;
            };
            if parent.is_empty() || !self.tasks.contains_key(parent) {
                continue;
            }
            ids.push((parent, RELATED_PARENT));
            for s in self.tasks.values().filter(|s| s.parent == parent) {
                ids.push((&s.id, RELATED_SIBLING));
            }
        }
        let mut out: Vec<RelatedCard> = Vec::new();
        for (id, why) in ids {
            if out.iter().any(|r| r.id == id) {
                continue;
            }
            let Some(t) = self.tasks.get(id) else {
                continue;
            };
            let mut commenters: Vec<String> = Vec::new();
            for c in self
                .by_task
                .get(id)
                .into_iter()
                .flatten()
                .filter_map(|c| self.comments.get(c))
            {
                if c.declines.is_empty() && !c.by.is_empty() && !commenters.contains(&c.by) {
                    commenters.push(c.by.clone());
                }
            }
            out.push(RelatedCard {
                id: id.to_string(),
                why,
                state: t.state.clone(),
                holders: t.holders.clone(),
                co_holders: t.co_holders.clone(),
                assigned: t.assigned.clone(),
                watchers: t.watchers.clone(),
                commenters,
                scope: t.scope.clone(),
            });
        }
        out
    }
}
