//! The claim on a PR's review (merge desk §12.3): a `pr/<owner>/<name>/<n>`
//! lease on the PR card's board log, taken only through `review_claim`.
//! Here: finding the PR card, the claim itself, the decline comment, and
//! what ends a claim by itself (a merged or closed PR, the claimer's own
//! verdict, a claim that renews).

use std::time::Duration;

use super::MAX_COMMENT;
use super::comments::check_reason;
use super::error::{Result, held, invalid, not_found};
use super::fold::Fold;
use super::prstate::{PR_CLOSED, PR_MERGED};
use super::resource::{is_pr_claim, pr_claim_resource};
use super::rows::TaskCommentRow;
use super::types::*;
use super::util::{caller, collapse_space};
use crate::gotime;

/// How long a claim lasts: 30 minutes by the holder's clock.
pub const CLAIM_TTL: Duration = Duration::from_secs(30 * 60);
/// A claim is renewed only when its `until` is under this far away.
pub const CLAIM_RENEW_WITHIN: Duration = Duration::from_secs(10 * 60);
/// Renewal stops this long after the claim was first taken.
pub const CLAIM_MAX_AGE: Duration = Duration::from_secs(2 * 3600);

/// A claim that was written or ended this pass, for the caller to report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimStep {
    pub resource: String,
    pub holder: String,
    /// `renewed`, `verdict` (released on the holder's own review).
    pub what: &'static str,
}

/// The lowest id of `cards`, preferring one not cancelled.
fn lowest_open(cards: Vec<&Task>) -> Option<&Task> {
    cards
        .into_iter()
        .min_by(|a, b| (a.state == CANCELED, &a.id).cmp(&(b.state == CANCELED, &b.id)))
}

impl Fold {
    /// The PR card for `pr` (its URL): the standing card whose creation
    /// row names that PR (§12.2), as [`Fold::pr_card_by_resource`] picks it.
    pub(crate) fn pr_card_for(&self, pr: &str) -> Option<&Task> {
        let want = pr_claim_resource(pr)?;
        self.pr_card_by_resource(&want)
    }

    /// The standing PR card for the PR `want` names (§12.2, card #1155).
    /// Any member's person row can carry a PR card's fields and coord has
    /// no GitHub logins, so the card that stands is the one whose maker the
    /// login names, never just the lowest id:
    ///
    /// - Only cards whose maker is still an active member count.
    /// - The cards for the PR name one author login, `L`; cards naming
    ///   different logins stand for none (coord can't tell GitHub's).
    /// - The people `L` names are the active members whose profiles claim
    ///   it, or, when nobody claims it, the cards' makers. Exactly one, who
    ///   made a card, or none stands.
    /// - Among that person's cards, the lowest id, preferring one open.
    ///
    /// So a member who makes a card for someone else's PR, or claims their
    /// login, can stop its review here, visibly, but never take it from an
    /// author whose profile names the login. Off a board, the lowest id.
    pub(crate) fn pr_card_by_resource(&self, want: &str) -> Option<&Task> {
        let cards: Vec<&Task> = self
            .tasks
            .values()
            .filter(|t| {
                t.pr_card
                    .as_ref()
                    .and_then(|c| pr_claim_resource(&c.pr))
                    .is_some_and(|r| r == want)
            })
            .collect();
        if !self.on_board {
            return lowest_open(cards);
        }
        let maker = |t: &Task| self.person_of(&t.created_by);
        let active = |p: &str| self.member_active.as_ref().is_none_or(|f| f(p));
        let cards: Vec<&Task> = cards.into_iter().filter(|t| active(&maker(t))).collect();
        let login = |t: &Task| {
            t.pr_card
                .as_ref()
                .map(|c| c.author.trim().to_ascii_lowercase())
                .unwrap_or_default()
        };
        let l = login(cards.first()?);
        if l.is_empty() || cards.iter().any(|t| login(t) != l) {
            return None;
        }
        let mut named = self.login_people.as_ref()?(&l);
        if named.is_empty() {
            named = cards.iter().map(|t| maker(t)).collect();
        }
        named.sort();
        named.dedup();
        let [who] = named.as_slice() else {
            return None;
        };
        if who.is_empty() {
            return None;
        }
        lowest_open(cards.into_iter().filter(|t| maker(t) == *who).collect())
    }

    /// The people who made a PR card for the PR `res` names on this board,
    /// standing or not (card #1155). Only an author's own client makes one,
    /// so these are the author and anyone posing as them; a row's fold
    /// reads this rather than which card stands, which turns on profiles.
    pub(crate) fn pr_card_makers(&self, res: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .tasks
            .values()
            .filter(|t| {
                t.pr_card
                    .as_ref()
                    .and_then(|c| pr_claim_resource(&c.pr))
                    .is_some_and(|r| r == res)
            })
            .map(|t| self.person_of(&t.created_by))
            .filter(|p| !p.is_empty())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Whether the PR a `pr/` claim is on has ended: its card's `pr.state`
    /// reads MERGED or CLOSED. The fold then counts the claim as not live,
    /// on every device, with no row needed.
    pub(crate) fn pr_claim_over(&self, name: &str) -> bool {
        if !self.on_board || !is_pr_claim(name) {
            return false;
        }
        let Some(t) = self.pr_card_by_resource(name) else {
            return false;
        };
        let Some(c) = &t.pr_card else {
            return false;
        };
        self.pr_states
            .get(&t.id)
            .and_then(|m| m.get(&c.pr))
            .is_some_and(|s| s.state == PR_MERGED || s.state == PR_CLOSED)
    }

    /// Takes the claim on `pr` for `by`, or says who holds it. It reads the
    /// live lease first, so a refused claim writes no `lease.refused` row.
    /// The caller has already checked who may claim (§12.3 refusals).
    pub(crate) fn claim_pr(&mut self, by: &str, pr: &str) -> Result<Lease> {
        let by = caller(by)?;
        let Some(name) = pr_claim_resource(pr) else {
            return Err(invalid(format!("{pr} is not a GitHub pull request URL")));
        };
        if self.pr_card_for(pr).is_none() {
            return Err(not_found(format!("no PR card for {pr} on this board")));
        }
        let now = self.stamp();
        self.prune(&now);
        if let Some(l) = self.held.get(&name).filter(|l| self.live(l, &now))
            && l.lease.holder != by
        {
            return Err(held(format!(
                "{name} is held by {} until {}",
                l.lease.holder,
                l.lease.until.format_rfc3339()
            )));
        }
        self.acquire(&by, &name, CLAIM_TTL, false)
    }

    /// The live claim on `pr`, if any.
    pub(crate) fn claim_of(&mut self, pr: &str) -> Option<Lease> {
        let name = pr_claim_resource(pr)?;
        let now = self.stamp();
        self.prune(&now);
        self.held
            .get(&name)
            .filter(|l| self.live(l, &now))
            .map(|l| l.lease.clone())
    }

    /// "Not me": a comment on the PR card from `by`, with the PR's URL in
    /// `declines` and no `to` (a comment with `to` is a release request).
    pub(crate) fn decline_review(&mut self, by: &str, pr: &str, reason: &str) -> Result<Comment> {
        let by = caller(by)?;
        let Some(card) = self.pr_card_for(pr).map(|t| t.id.clone()) else {
            return Err(not_found(format!("no PR card for {pr} on this board")));
        };
        let url = self.tasks[&card]
            .pr_card
            .as_ref()
            .map(|c| c.pr.clone())
            .unwrap_or_default();
        let why = collapse_space(reason);
        check_reason(&why, false)?;
        let text = if why.is_empty() {
            format!("Declined the review of {url}.")
        } else {
            format!("Declined the review of {url}: {why}")
        };
        if text.chars().count() > MAX_COMMENT {
            return Err(invalid("reason too long"));
        }
        let at = self.stamp();
        let actor = self.actor_for(&by);
        self.write_comment(
            &actor,
            TaskCommentRow {
                task_id: card,
                comment_id: super::util::new_id(&at),
                by,
                text,
                wake: false,
                at,
                declines: url,
                ..TaskCommentRow::default()
            },
        )
    }

    /// This device's part as a claimer: renews each of its claims that is
    /// under [`CLAIM_RENEW_WITHIN`] from `until`, while `alive` says the
    /// holder's agent answers in the local roster and the claim is under
    /// [`CLAIM_MAX_AGE`] old; and releases a claim whose holder's person
    /// (`login_of` names the GitHub login of a holder key) posted an
    /// APPROVED or CHANGES_REQUESTED review since `acquired_at`.
    pub(crate) fn claim_duties(
        &mut self,
        alive: &dyn Fn(&str) -> bool,
        login_of: &dyn Fn(&str) -> String,
    ) -> Vec<ClaimStep> {
        if !self.on_board {
            return Vec::new();
        }
        let now = self.stamp();
        self.prune(&now);
        let mine: Vec<Lease> = self
            .held
            .values()
            .filter(|l| is_pr_claim(&l.lease.resource) && self.lease_local(l) && self.live(l, &now))
            .map(|l| l.lease.clone())
            .collect();
        let mut out = Vec::new();
        for l in mine {
            let login = login_of(&l.holder);
            if !login.is_empty() && self.claim_reviewed(&l, &login) {
                if self.release(&l.holder, &l.resource).is_ok() {
                    out.push(ClaimStep {
                        resource: l.resource,
                        holder: l.holder,
                        what: "verdict",
                    });
                }
                continue;
            }
            let near = !gotime::after(&l.until, &gotime::add(&now, CLAIM_RENEW_WITHIN));
            let young = gotime::after(&gotime::add(&l.acquired_at, CLAIM_MAX_AGE), &now);
            if near && young && alive(&l.holder) {
                let renewed = self.acquire(&l.holder, &l.resource, CLAIM_TTL, false);
                if renewed.is_ok() {
                    out.push(ClaimStep {
                        resource: l.resource,
                        holder: l.holder,
                        what: "renewed",
                    });
                }
            }
        }
        out
    }

    /// Whether `login` posted a verdict (APPROVED or CHANGES_REQUESTED, not
    /// a plain comment) on the claim's PR after the claim was taken.
    fn claim_reviewed(&self, l: &Lease, login: &str) -> bool {
        let Some(t) = self.pr_card_by_resource(&l.resource) else {
            return false;
        };
        let Some(c) = &t.pr_card else {
            return false;
        };
        let Some(s) = self.pr_states.get(&t.id).and_then(|m| m.get(&c.pr)) else {
            return false;
        };
        s.reviews.iter().any(|r| {
            r.login.eq_ignore_ascii_case(login)
                && (r.state == "APPROVED" || r.state == "CHANGES_REQUESTED")
                && gotime::after(&r.at, &l.acquired_at)
        })
    }
}
