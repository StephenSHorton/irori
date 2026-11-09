//! What the desk and the `prs` tool show of a PR's review (merge desk
//! §12.9, card #1080): the claimer, the related agents, GitHub's review
//! state, the pick state and why nothing is picked, split by who looks.
//! All of it is read from the boards this device shows, per call; nothing
//! is written, and it needs no capture, so `prs` can show it with capture
//! off.

use coord::{DeskPr, ReviewPhase};

use super::Services;
use super::boards::BoardLog;
use super::review_pick::tiers;
use crate::deskmap::{ClaimCount, DeskClaimer, DeskRelated, DeskReview, NoPick, why_none};

/// Why an agent is related, from its tier signals (§12.5); "told to look"
/// when none holds now (a lease lapsed since the ping).
pub(crate) fn why_related(t: [bool; 4]) -> &'static str {
    if t[0] {
        "same card"
    } else if t[1] {
        "same files"
    } else if t[2] {
        "commented"
    } else if t[3] {
        "dedicated reviewer"
    } else {
        "told to look"
    }
}

impl Services {
    /// A member's name on `b`, "" when unknown.
    fn person_name_on(&self, b: &BoardLog, person: &str) -> String {
        if person.is_empty() {
            return String::new();
        }
        let n = b
            .verifier
            .profile(person)
            .or_else(|| {
                b.verifier
                    .membership()
                    .person(person)
                    .map(|p| p.name.clone())
            })
            .unwrap_or_default();
        super::review_claim::clean_name(&n)
    }

    /// The review of PR `p` in `repo_key` as this viewer sees it.
    pub(crate) fn pr_review(&self, repo_key: &str, p: &DeskPr) -> DeskReview {
        let desk = self.coord.desk(repo_key);
        let reviews_off = desk.as_ref().is_some_and(|d| d.settings.reviews_off);
        let polled_elsewhere = desk.as_ref().is_some_and(|d| {
            !d.settings.poll_device.is_empty() && d.settings.poll_device != self.device.id
        });
        let own = self.own_github_login();
        let viewer = desk
            .as_ref()
            .and_then(|d| d.prs.as_ref())
            .map(|s| s.viewer.clone())
            .unwrap_or_default();
        let opened_by_me = !p.author.is_empty()
            && (p.author.eq_ignore_ascii_case(&own) || p.author.eq_ignore_ascii_case(&viewer));
        let note = self.desk_watch.note(repo_key, p.number);
        let Some((b, card)) = self.pr_card_here(&p.url) else {
            let boards = self.visible_boards();
            let no_space =
                crate::prcard::choose_space(&self.space_facts(&boards, repo_key)).is_none();
            let author_in_space = boards.iter().any(|b| {
                let m = b.verifier.membership();
                m.members
                    .iter()
                    .filter(|x| !x.person_id.is_empty() && m.active(&x.person_id))
                    .any(|x| {
                        b.verifier
                            .profile_github_login(&x.person_id)
                            .eq_ignore_ascii_case(&p.author)
                    })
            });
            return DeskReview {
                pick: "none".into(),
                review: p.review_decision.clone(),
                why_none: why_none(&NoPick {
                    author: opened_by_me,
                    has_card: false,
                    note,
                    reviews_off,
                    no_space,
                    polled_elsewhere,
                    author_in_space: author_in_space || opened_by_me,
                }),
                ..DeskReview::default()
            };
        };
        let Some(f) = b.coord().pick_facts(&card.id) else {
            return DeskReview {
                pick: "none".into(),
                review: p.review_decision.clone(),
                ..DeskReview::default()
            };
        };
        let me = self.person_id();
        let mut r = DeskReview {
            review: if f.pr_state.review.is_empty() {
                p.review_decision.clone()
            } else {
                f.pr_state.review.clone()
            },
            ..DeskReview::default()
        };
        r.pick = match &f.phase {
            ReviewPhase::Off => "off",
            ReviewPhase::Waiting => "waiting",
            ReviewPhase::Asked { .. } => "asked",
            ReviewPhase::Claimed { .. } => "claimed",
            ReviewPhase::Reviewed => "reviewed",
        }
        .into();
        let name = |key: &str| {
            let n = self.holder_name(&b, key);
            if n == "an agent" { String::new() } else { n }
        };
        let person_of = |key: &str| self.person_name_on(&b, &b.verifier.key_person(key));
        if let ReviewPhase::Asked { ask } = &f.phase {
            r.asked = name(&ask.to);
        }
        if let Some(l) = &f.live_claim {
            r.claimer = Some(DeskClaimer {
                agent: name(&l.holder),
                person: person_of(&l.holder),
                until: l.until.format_rfc3339(),
            });
        }
        for k in &f.pinged {
            let person = b.verifier.key_person(k);
            let login = b.verifier.profile_github_login(&person);
            r.related.push(DeskRelated {
                agent: name(k),
                person: person_of(k),
                why: why_related(tiers(k, &person, login.trim(), &f)).into(),
            });
        }
        if r.pick == "waiting" {
            let author = f.author_person == me;
            r.why_none = why_none(&NoPick {
                author,
                has_card: true,
                note: if author { note } else { String::new() },
                ..NoPick::default()
            });
        }
        r
    }

    /// Each person's open and 14-day claim counts across the spaces this
    /// device shows (§12.6, §12.9), people with none left out.
    pub(crate) fn claim_counts_all(&self) -> Vec<ClaimCount> {
        let mut by: std::collections::BTreeMap<String, ClaimCount> = Default::default();
        for b in self.visible_boards() {
            for (person, c) in b.coord().claim_counts() {
                if c.open == 0 && c.recent == 0 {
                    continue;
                }
                let name = self.person_name_on(&b, &person);
                let name = if name.is_empty() {
                    person.clone()
                } else {
                    name
                };
                let e = by.entry(name.clone()).or_insert_with(|| ClaimCount {
                    person: name,
                    ..ClaimCount::default()
                });
                e.open += c.open;
                e.recent += c.recent;
            }
        }
        by.into_values().collect()
    }

    /// [`Self::pr_review`] for each PR of `repo` (`owner/name`) the desk
    /// lists, worded for the `prs` tool.
    pub(crate) fn pr_review_lines(&self, repo: &str) -> Vec<tools::PrReviewLine> {
        let key = coord::canonical_repo_key(&format!("remote:github.com/{repo}"));
        let Some(desk) = self.coord.desk(&key) else {
            return Vec::new();
        };
        let Some(snap) = desk.prs.as_ref() else {
            return Vec::new();
        };
        let reviews: std::collections::HashMap<i64, DeskReview> = self
            .desk_verdicts(&key)
            .into_iter()
            .map(|a| (a.pr, a.review))
            .collect();
        snap.prs
            .iter()
            .filter_map(|p| reviews.get(&p.number).map(|r| (p, r.clone())))
            .map(|(p, r)| {
                let who = |a: &str, p: &str| match (a.is_empty(), p.is_empty()) {
                    (_, true) => a.to_string(),
                    (true, false) => format!("an agent ({p})"),
                    (false, false) => format!("{a} ({p})"),
                };
                tools::PrReviewLine {
                    number: p.number,
                    url: p.url.clone(),
                    pick: r.pick,
                    review: r.review,
                    asked: r.asked,
                    claimer: r
                        .claimer
                        .as_ref()
                        .map(|c| who(&c.agent, &c.person))
                        .unwrap_or_default(),
                    until: r.claimer.map(|c| c.until).unwrap_or_default(),
                    related: r
                        .related
                        .iter()
                        .map(|x| format!("{}: {}", who(&x.agent, &x.person), x.why))
                        .collect(),
                    why_none: r.why_none,
                }
            })
            .collect()
    }
}
