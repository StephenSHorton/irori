//! `review_claim` (merge desk §12.3, card #1078): the one claim per PR.
//! The claim is a `pr/<owner>/<name>/<n>` lease written straight to the
//! PR card's board log under the agent's board key, so any member's agent
//! can take it with no card of its own. Who may claim is checked here, on
//! rows only the author's person can write (the PR card's creation row),
//! and against GitHub with the claimer's own gh.

use std::sync::Arc;

use coord::{Task, is_terminal};
use store::kinds::PERSON_ACTOR_PREFIX;
use tools::{ClaimAnswer, ClaimOutcome};

use super::Services;
use super::boards::{BoardLog, lock};

/// An agent's name, cut and cleaned for the fixed text of a result.
pub(super) fn clean_name(n: &str) -> String {
    let s: String = n
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
        .take(40)
        .collect();
    s.trim().to_string()
}

impl Services {
    /// The member a board key is on `b`, `""` for none.
    fn key_person_on(&self, b: &BoardLog, key: &str) -> String {
        b.verifier.key_person(key)
    }

    /// The board and PR card for `pr` in a space the person belongs to.
    fn pr_card_here(&self, pr: &str) -> Option<(Arc<BoardLog>, Task)> {
        let me = self.person_id();
        let mut boards: Vec<Arc<BoardLog>> = self
            .visible_boards()
            .into_iter()
            .filter(|b| b.verifier.membership().active(&me))
            .collect();
        boards.sort_by(|a, b| a.id.cmp(&b.id));
        boards
            .into_iter()
            .find_map(|b| b.coord().pr_card_for(pr).map(|t| (b, t)))
    }

    /// Whether the person already works in the PR's repo in this space
    /// (§12.8): a card there with this `repo_key` that their person or one
    /// of their own agents created. A card someone else made and handed
    /// them doesn't count.
    fn works_in_repo(&self, b: &BoardLog, card: &Task) -> bool {
        let me = self.person_id();
        if card.repo_key.is_empty() {
            return false;
        }
        b.coord()
            .list_tasks(&coord::TaskFilter::default())
            .iter()
            .any(|t| {
                t.pr_card.is_none()
                    && coord::same_repo_key(&t.repo_key, &card.repo_key)
                    && self.key_person_on(b, &t.created_by) == me
            })
    }

    /// Whether the person themself signed off a verdict on this PR: a
    /// blocker on the PR card, raised by their agent, for them, naming the
    /// PR, resolved by a person row of theirs. Nobody else's answer counts.
    fn signed_off(&self, b: &BoardLog, card: &Task, key: &str) -> bool {
        let me = self.person_id();
        let url = card.pr_card.as_ref().map(|c| c.pr.as_str()).unwrap_or("");
        card.blockers.iter().any(|bl| {
            let Some(r) = &bl.resolved else {
                return false;
            };
            !r.closed
                && r.by.starts_with(PERSON_ACTOR_PREFIX)
                && self.key_person_on(b, &r.by) == me
                && bl.raised_by == key
                && bl.question.contains(url)
                && bl
                    .on
                    .as_ref()
                    .is_some_and(|on| self.key_person_on(b, &on.reference) == me)
        })
    }

    /// The holder's name on `b`: its card's, cleaned; `an agent` if none.
    fn holder_name(&self, b: &BoardLog, holder: &str) -> String {
        let name = holder
            .strip_prefix(store::kinds::AGENT_ACTOR_PREFIX)
            .and_then(|id| lock(&b.maps).cards.get(id).map(|c| c.name.clone()))
            .map(|n| clean_name(&n))
            .unwrap_or_default();
        if name.is_empty() {
            "an agent".into()
        } else {
            name
        }
    }

    /// This device's part as a claimer on `b` (§12.3): renew each claim
    /// whose agent still answers in the local roster, and let go of one
    /// whose person posted a verdict on GitHub.
    pub(crate) fn review_claim_duties(&self, b: &BoardLog) {
        let agents: std::collections::HashSet<String> =
            self.roster.agents().into_iter().map(|a| a.key).collect();
        let alive = |holder: &str| {
            let m = lock(&b.maps);
            holder
                .strip_prefix(store::kinds::AGENT_ACTOR_PREFIX)
                .and_then(|id| m.by_id.get(id))
                .is_some_and(|k| agents.contains(k))
        };
        let me = self.person_id();
        let login = {
            let own = self.own_github_login();
            if own.is_empty() {
                b.verifier.profile_github_login(&me)
            } else {
                own
            }
        };
        let login_of = |_: &str| login.trim().to_string();
        for st in b.coord().claim_duties(&alive, &login_of) {
            if st.what == "verdict" {
                crate::logf(format_args!(
                    "board {}: claim {} ended with a verdict",
                    b.id, st.resource
                ));
            }
        }
    }

    /// `review_claim {pr, action}` for `agent` (§12.3).
    pub(crate) fn review_claim(
        &self,
        agent: &str,
        pr: &str,
        action: &str,
        reason: &str,
    ) -> Result<ClaimAnswer, String> {
        if coord::pr_claim_resource(pr).is_none() {
            return Err(
                "give the PR's full URL, like https://github.com/owner/name/pull/7.".into(),
            );
        }
        let Some((b, card)) = self.pr_card_here(pr) else {
            return Err("no card for this PR is on a shared space you belong to, so there is nothing to claim. Comment on the PR on GitHub if you have something to say.".into());
        };
        let key = self.board_key(&b.id, agent).map_err(|e| e.to_string())?;
        let url = card
            .pr_card
            .as_ref()
            .map(|c| c.pr.clone())
            .unwrap_or_default();
        let ans = |outcome, holder: String, until: String| ClaimAnswer {
            outcome,
            pr: url.clone(),
            card: card.short_ref(),
            holder,
            until,
            go_ahead: false,
            signed_off: false,
        };
        let me = self.person_id();
        match action {
            "claim" => {
                if is_terminal(&card.state) {
                    return Err("this PR is merged or closed; there is nothing to review.".into());
                }
                let author = card
                    .pr_card
                    .as_ref()
                    .map(|c| c.author.clone())
                    .unwrap_or_default();
                let m = b.verifier.membership();
                let is_author = self.key_person_on(&b, &card.created_by) == me
                    || self.own_github_login().eq_ignore_ascii_case(&author)
                    || m.members
                        .iter()
                        .filter(|p| !p.person_id.is_empty() && m.active(&p.person_id))
                        .any(|p| {
                            p.person_id == me
                                && b.verifier
                                    .profile_github_login(&p.person_id)
                                    .eq_ignore_ascii_case(&author)
                        });
                if is_author {
                    return Err("this is your person's own PR; you can't claim its review. You can still comment on it.".into());
                }
                if self.own_github_login().is_empty()
                    && b.verifier.profile_github_login(&me).trim().is_empty()
                {
                    return Err("Irori doesn't know your GitHub login yet, so it can't tell your review from the author's. Ask your person to sign in to gh (`gh auth login`); then claim again.".into());
                }
                let Some(checker) = self.pr_checker.clone() else {
                    return Err("this device can't ask GitHub who wrote the PR, so it can't give you the claim.".into());
                };
                match checker.author(&url) {
                    Ok(a) if a.eq_ignore_ascii_case(&author) => {}
                    Ok(_) => {
                        return Err("GitHub says someone else wrote this PR than its card names, so Irori won't give you the claim.".into());
                    }
                    Err(_) => {
                        return Err("your gh couldn't confirm who wrote this PR, so Irori can't give you the claim. Check `gh auth status`, then claim again.".into());
                    }
                }
                match b.coord().claim_pr(&key, &url) {
                    Ok(l) => {
                        let mut a = ans(
                            ClaimOutcome::Claimed,
                            String::new(),
                            l.until.format_rfc3339(),
                        );
                        a.go_ahead = self.inbox.reviews_unasked() && self.works_in_repo(&b, &card);
                        Ok(a)
                    }
                    Err(e) if e.is(coord::ErrorKind::Held) => {
                        let held = b.coord().claim_of(&url);
                        let (name, until) = held
                            .map(|l| (self.holder_name(&b, &l.holder), l.until.format_rfc3339()))
                            .unwrap_or_default();
                        Ok(ans(ClaimOutcome::Held, name, until))
                    }
                    Err(_) => Err(
                        "Irori couldn't write the claim to the space's log; try again in a moment."
                            .into(),
                    ),
                }
            }
            "confirm" => {
                let held = b.coord().claim_of(&url);
                let mut a = match held {
                    Some(l) if l.holder == key => {
                        ans(ClaimOutcome::Yours, String::new(), l.until.format_rfc3339())
                    }
                    Some(l) => ans(
                        ClaimOutcome::NotYours,
                        self.holder_name(&b, &l.holder),
                        l.until.format_rfc3339(),
                    ),
                    None => ans(ClaimOutcome::NotYours, String::new(), String::new()),
                };
                a.go_ahead = self.inbox.reviews_unasked() && self.works_in_repo(&b, &card);
                a.signed_off = self.signed_off(&b, &card, &key);
                Ok(a)
            }
            "release" => {
                let done = b
                    .coord()
                    .release_claim(&key, &url)
                    .map_err(|e| e.to_string())?;
                let o = if done {
                    ClaimOutcome::Released
                } else {
                    ClaimOutcome::NotHeld
                };
                Ok(ans(o, String::new(), String::new()))
            }
            "decline" => {
                b.coord()
                    .decline_review(&key, &url, reason)
                    .map_err(|e| e.to_string())?;
                let _ = b.coord().release_claim(&key, &url);
                Ok(ans(ClaimOutcome::Declined, String::new(), String::new()))
            }
            _ => Err("action is claim, confirm, release or decline.".into()),
        }
    }
}
