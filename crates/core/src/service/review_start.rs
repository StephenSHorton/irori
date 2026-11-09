//! Starting a new reviewer when nobody fits (merge desk §12.10, card
//! #1082): tier 7 of the pick. The PR author's device makes a review card on
//! the PR card's board as the author's person and gives it to the member
//! with the fewest claims who already works in the PR's repo in that space.
//! Their Irori starts its agent without the "OK to start an agent on this?"
//! question while their switch is on (`start_given`), and asks as for any
//! given card while it is off. A start that fails ("couldn't start": no
//! checkout, not trusted) is cancelled here and the card goes to the
//! next-fewest.
//!
//! Nothing is remembered: who was tried is read back from the review cards
//! on the board.

use std::collections::{BTreeMap, BTreeSet};

use coord::{ClaimCounts, DeskPr, NewTask, PickFacts, Task, TaskFilter};
use store::kinds::PERSON_ACTOR_PREFIX;

use super::Services;
use super::boards::BoardLog;
use super::prcard::Found;
use crate::logf;

/// What a pass of tier 7 came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Started {
    /// A review card went to this person.
    Given(String),
    /// A review card is already out and may still start.
    Pending,
    /// Nobody qualifies, or everyone who does has been tried.
    Nobody,
}

/// The people to try, fewest claims first (§12.6): open claims, then 14-day
/// claims, then a hash of the PR and the person so the same one doesn't
/// always win, then the person.
pub(crate) fn order(
    people: &BTreeSet<String>,
    counts: &BTreeMap<String, ClaimCounts>,
    pr: &str,
) -> Vec<String> {
    let mut v: Vec<&String> = people.iter().collect();
    v.sort_by_key(|p| {
        (
            counts.get(*p).copied().unwrap_or_default(),
            gocompat::fnv::fnv32a(format!("{pr}{p}").as_bytes()),
            (*p).clone(),
        )
    });
    v.into_iter().cloned().collect()
}

/// What the review card says it is for.
fn objective(p: &DeskPr, repo: &str) -> String {
    format!(
        "Review pull request #{} in {repo}: {}. Run review_claim pr={} first and do what it says.",
        p.number, p.title, p.url
    )
}

impl Services {
    /// The members who may be given a review card on `b` (§12.8, §12.10):
    /// active, not the PR author, and with a card of their own in this repo
    /// in this space. A card someone handed them, or the author made, never
    /// counts.
    fn reviewer_candidates(&self, b: &BoardLog, card: &Task) -> BTreeSet<String> {
        let me = self.person_id();
        if card.repo_key.is_empty() {
            return BTreeSet::new();
        }
        let ms = b.verifier.membership();
        let mut works: BTreeSet<String> = BTreeSet::new();
        for t in b.coord().list_tasks(&TaskFilter::default()) {
            if t.pr_card.is_some()
                || !t.review_of.is_empty()
                || !coord::same_repo_key(&t.repo_key, &card.repo_key)
            {
                continue;
            }
            let p = b.verifier.key_person(&t.created_by);
            if !p.is_empty() && p != me && ms.active(&p) {
                works.insert(p);
            }
        }
        works
    }

    /// Tier 7: makes a review card for the PR card `f` and gives it to the
    /// qualifying person with the fewest claims who has not had one for
    /// this PR (§12.10). The author's person acts, from their own device.
    pub(super) fn start_reviewer(
        &self,
        f: &Found,
        p: &DeskPr,
        facts: &PickFacts,
    ) -> Result<Started, String> {
        let b = &f.board;
        let co = b.coord();
        let me = self.person_actor();
        let mut tried: BTreeSet<String> = BTreeSet::new();
        let mut pending = false;
        for t in co
            .list_tasks(&TaskFilter::default())
            .into_iter()
            .filter(|t| t.review_of == f.task.id)
        {
            let Some(g) = t.given.as_ref() else {
                // Made and not given yet (the give is the next row): it is
                // out.
                if !coord::is_terminal(&t.state) {
                    pending = true;
                }
                continue;
            };
            tried.insert(g.person.clone());
            if coord::is_terminal(&t.state) {
                continue;
            }
            if let Some(why) = coord::Coord::given_failed(&t) {
                // "Couldn't start" there: it goes to the next-fewest.
                logf(format_args!(
                    "desk: PR #{}: review card {} couldn't start: {why}",
                    p.number,
                    t.short_ref()
                ));
                if let Err(e) = co.set_task_state(
                    &me,
                    &t.id,
                    coord::CANCELED,
                    "couldn't start on their computer; trying the next person",
                ) {
                    logf(format_args!(
                        "desk: PR #{}: cancel review card: {e}",
                        p.number
                    ));
                    pending = true;
                }
            } else {
                pending = true;
            }
        }
        if pending {
            return Ok(Started::Pending);
        }
        let people: BTreeSet<String> = self
            .reviewer_candidates(b, &f.task)
            .difference(&tried)
            .cloned()
            .collect();
        let Some(person) = order(&people, &facts.counts, &facts.pr).into_iter().next() else {
            return Ok(Started::Nobody);
        };
        let nt = NewTask {
            title: format!("Review PR #{}: {}", p.number, p.title),
            objective: objective(p, &f.task.repo_key),
            repo_key: f.task.repo_key.clone(),
            links: vec![p.url.clone()],
            board: f.task.board.clone(),
            board_named: true,
            review_of: f.task.id.clone(),
            ..NewTask::default()
        };
        let card = co.create_task(&me, nt).map_err(|e| e.to_string())?;
        if let Err(e) = co.assign_task(&me, &card.id, &format!("{PERSON_ACTOR_PREFIX}{person}")) {
            // Not given: it must not stay as the author's own card.
            let _ = co.set_task_state(&me, &card.id, coord::CANCELED, "couldn't be given");
            return Err(e.to_string());
        }
        logf(format_args!(
            "desk: PR #{}: nobody fits; gave a review card to {person}",
            p.number
        ));
        Ok(Started::Given(person))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(v: &[(&str, usize, usize)]) -> BTreeMap<String, ClaimCounts> {
        v.iter()
            .map(|(p, open, recent)| {
                (
                    p.to_string(),
                    ClaimCounts {
                        open: *open,
                        recent: *recent,
                    },
                )
            })
            .collect()
    }

    fn set(v: &[&str]) -> BTreeSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn fewest_open_claims_first_then_fewest_recent() {
        let c = counts(&[("a", 2, 0), ("b", 0, 5), ("c", 0, 1)]);
        assert_eq!(order(&set(&["a", "b", "c"]), &c, "pr"), ["c", "b", "a"]);
    }

    #[test]
    fn a_person_with_no_claims_beats_one_with_some() {
        let c = counts(&[("a", 1, 1)]);
        assert_eq!(order(&set(&["a", "b"]), &c, "pr")[0], "b");
    }

    #[test]
    fn a_tie_is_broken_by_a_hash_not_by_name() {
        let people = set(&["p1", "p2", "p3", "p4"]);
        let c = BTreeMap::new();
        let firsts: BTreeSet<String> = (0..16)
            .map(|i| order(&people, &c, &format!("pr/{i}"))[0].clone())
            .collect();
        assert!(firsts.len() > 1, "{firsts:?}");
    }
}
