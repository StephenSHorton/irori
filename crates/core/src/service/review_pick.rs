//! Picking a PR's reviewer from the PR author's poll device (merge desk
//! §12.5 to §12.7, card #1079). After a good desk poll, each standing PR
//! card the person made gets one pass: the phase from its rows (coord's
//! [`ReviewPhase`]) plus what only this device sees (the asked agent gone,
//! OFFLINE, or its turn ended untouched); in Waiting, one ask to the best
//! candidate and a ping to each related agent not pinged before.
//!
//! Nothing is remembered but a poll count: who is left out is read back
//! from the rows (declines, claims, asks), so a restart loses nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use coord::roster::{AgentCard, OFFLINE};
use coord::{ClaimCounts, DeskPr, PickFacts, PrReview, ReviewPhase, RowPos};
use gocompat::time::GoTime;
use mesh::boardmesh::REVIEWS_PROTOCOL;
use store::Filter;
use store::kinds::{self, AGENT_ACTOR_PREFIX};

use super::Services;
use super::boards::{BoardCard, BoardLog, each_row, lock, roster_of};
use super::prcard::Found;
use super::review_choice::{AskCand, Turn, choose_ask};
use super::spaces::ReadsLevel;
use crate::logf;
use crate::prcard::{self, PrComment};

/// GitHub's approving review state.
const APPROVED: &str = "APPROVED";
/// The tier of the agent asked again after its changes requested (§12.5).
pub(crate) const SAME_AGENT: u8 = 0;
/// The tier where nobody fits: a new reviewer (§12.10, B7).
pub(crate) const NOBODY: u8 = 7;
/// Standing polls before tier 7 may go ahead without another member's
/// agent coming on the board (§12.5).
pub(crate) const TIER7_POLLS: u32 = 3;
/// The desk's line once a PR has nobody to ask.
pub(crate) const NOBODY_NOTE: &str = "nobody in the space is free to review this PR yet";

fn is_verdict(state: &str) -> bool {
    state == APPROVED || state == coord::PR_CHANGES_REQUESTED
}

fn ms(t: &GoTime) -> i64 {
    t.unix_milli_value()
}

/// One candidate: another person's agent on the PR card's board.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Cand {
    pub key: String,
    pub person: String,
    pub idle: bool,
    /// Tiers 1 to 4: same work, same files, already there, dedicated.
    pub tiers: [bool; 4],
}

/// The assistant's word (tier 5): Skip when it has none to give.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tier5 {
    Skip,
    Nobody,
    Pick(String),
}

/// Who is asked, at which tier, and who else is told.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Ranked {
    pub pick: Option<String>,
    /// [`SAME_AGENT`], 1 to 6, or [`NOBODY`].
    pub tier: u8,
    /// Candidates in tiers 1 to 3 other than the pick (§12.7).
    pub related: Vec<String>,
}

/// The order within a tier (§12.6): the person's open claims, then their
/// recent claims, then idle before busy, then a hash of the PR and key.
fn order_key<'a>(
    c: &'a Cand,
    counts: &BTreeMap<String, ClaimCounts>,
    pr: &str,
) -> (ClaimCounts, bool, u32, &'a str) {
    (
        counts.get(&c.person).copied().unwrap_or_default(),
        !c.idle,
        gocompat::fnv::fnv32a(format!("{pr}{}", c.key).as_bytes()),
        &c.key,
    )
}

fn related(cands: &[Cand], pick: Option<&str>) -> Vec<String> {
    cands
        .iter()
        .filter(|c| c.tiers[..3].iter().any(|t| *t) && Some(c.key.as_str()) != pick)
        .map(|c| c.key.clone())
        .collect()
}

/// Ranks the candidates (§12.5): the first tier with anyone in it, of 1
/// to 4, then the assistant's pick (5), then anyone idle (6); else tier 7.
pub(crate) fn rank(
    cands: &[Cand],
    counts: &BTreeMap<String, ClaimCounts>,
    pr: &str,
    tier5: &Tier5,
) -> Ranked {
    let best = |keep: &dyn Fn(&Cand) -> bool| {
        cands
            .iter()
            .filter(|c| keep(c))
            .min_by(|a, b| order_key(a, counts, pr).cmp(&order_key(b, counts, pr)))
            .map(|c| c.key.clone())
    };
    let mut pick = None;
    let mut tier = NOBODY;
    for t in 0..4 {
        if let Some(k) = best(&|c| c.tiers[t]) {
            pick = Some(k);
            tier = t as u8 + 1;
            break;
        }
    }
    if pick.is_none()
        && let Tier5::Pick(k) = tier5
        && cands.iter().any(|c| c.key == *k)
    {
        pick = Some(k.clone());
        tier = 5;
    }
    if pick.is_none()
        && let Some(k) = best(&|c| c.idle)
    {
        pick = Some(k);
        tier = 6;
    }
    Ranked {
        related: related(cands, pick.as_deref()),
        pick,
        tier,
    }
}

/// Whether tier 7 (a new reviewer) may go ahead (§12.5): another member's
/// device put an agent on the board since the card was made, or the card
/// has stood for [`TIER7_POLLS`] polls.
/// Whether another agent than the asked one took a claim after the ask.
/// The ask told the asked agent to comment then, so what its person says
/// on GitHub no longer shows it working on the ask.
pub(crate) fn claimed_by_another(f: &PickFacts, ask: &coord::ReviewRequest) -> bool {
    f.claims
        .iter()
        .any(|c| c.holder != ask.to && c.pos > ask.pos)
}

pub(crate) fn tier7_allowed(polls_standing: u32, member_card_since: bool) -> bool {
    member_card_since || polls_standing >= TIER7_POLLS
}

/// Whether an agent's turn ended (§12.5): its card statuses after the ask,
/// in log order, have an `active` followed later by an `idle`.
pub(crate) fn untouched<S: AsRef<str>>(statuses: &[S]) -> bool {
    let mut active = false;
    for s in statuses {
        match s.as_ref() {
            "active" => active = true,
            "idle" if active => return true,
            _ => {}
        }
    }
    false
}

/// What this device sees of the asked agent, beyond the rows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AskedNow {
    /// Gone, or on an OFFLINE device.
    pub gone: bool,
    /// Its turn ended after the ask ([`untouched`]).
    pub turn_ended: bool,
    /// Its person commented or reviewed on GitHub after the ask.
    pub answered: bool,
}

/// The phase with this device's triggers (§12.5, D2): Asked turns Waiting
/// when the asked agent is gone or OFFLINE, or its turn ended with no word
/// on GitHub; that agent is left out of this pick.
pub(crate) fn device_phase(phase: &ReviewPhase, now: &AskedNow) -> (ReviewPhase, Option<String>) {
    if let ReviewPhase::Asked { ask } = phase
        && (now.gone || now.turn_ended && !now.answered)
    {
        return (ReviewPhase::Waiting, Some(ask.to.clone()));
    }
    (phase.clone(), None)
}

/// Whether `login` commented or reviewed on the PR after `since`.
pub(crate) fn answered_on_github(
    login: &str,
    since: &GoTime,
    comments: &[PrComment],
    reviews: &[PrReview],
) -> bool {
    let login = login.trim();
    !login.is_empty()
        && (comments
            .iter()
            .any(|c| c.login.eq_ignore_ascii_case(login) && ms(&c.at) > ms(since))
            || reviews
                .iter()
                .any(|r| r.login.eq_ignore_ascii_case(login) && ms(&r.at) > ms(since)))
}

/// Whether `login` gave a verdict on the PR after `since`.
fn verdict_after(f: &PickFacts, login: &str, since: &GoTime) -> bool {
    !login.is_empty()
        && f.pr_state.reviews.iter().any(|r| {
            is_verdict(&r.state) && r.login.eq_ignore_ascii_case(login) && ms(&r.at) > ms(since)
        })
}

/// The agents left out of picks for this PR, from rows alone (D1): those
/// that declined it; those whose claim on it ended with no verdict from
/// their person's login; and those asked when a newer pick (the person's
/// ask) went to another agent. `login_of` gives an agent's person's GitHub login.
pub(crate) fn excluded(f: &PickFacts, login_of: &dyn Fn(&str) -> String) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = f.declined.keys().cloned().collect();
    let live = f.live_claim.as_ref().map(|l| l.holder.as_str());
    for c in &f.claims {
        if Some(c.holder.as_str()) == live {
            continue;
        }
        if !verdict_after(f, login_of(&c.holder).trim(), &c.acquired_at) {
            out.insert(c.holder.clone());
        }
    }
    let mut newest: BTreeMap<&str, &RowPos> = BTreeMap::new();
    for a in &f.asks {
        let e = newest.entry(a.to.as_str()).or_insert(&a.pos);
        if a.pos > **e {
            *e = &a.pos;
        }
    }
    // Only a pick (the person's ask) replaces an ask: an agent's
    // board_review asks several at once, and none leaves another out.
    for (to, pos) in &newest {
        if f.asks
            .iter()
            .any(|a| a.to != *to && a.pos > **pos && a.from.starts_with(kinds::PERSON_ACTOR_PREFIX))
        {
            out.insert(to.to_string());
        }
    }
    out
}

/// The agent to ask again first (§12.5, D1): the newest verdict is
/// CHANGES_REQUESTED at an older head, and this agent's claim ended with
/// it. Spent once any ask is written after that claim.
pub(crate) fn changes_agent(f: &PickFacts, login_of: &dyn Fn(&str) -> String) -> Option<String> {
    let head = f.pr_state.head.as_str();
    let v = f
        .pr_state
        .reviews
        .iter()
        .filter(|r| is_verdict(&r.state))
        .max_by_key(|r| ms(&r.at))?;
    if head.is_empty() || v.state != coord::PR_CHANGES_REQUESTED || v.sha.eq_ignore_ascii_case(head)
    {
        return None;
    }
    let c = f.claims.iter().rev().find(|c| {
        ms(&c.acquired_at) < ms(&v.at) && login_of(&c.holder).trim().eq_ignore_ascii_case(&v.login)
    })?;
    if f.asks.iter().any(|a| a.pos > c.pos) {
        return None;
    }
    Some(c.holder.clone())
}

/// The pick in Waiting (§12.5): `live` are the candidates the roster
/// allows ([`live_cands`], tiers filled in), `left_out` the agent a device
/// trigger fired for. The agent whose changes were requested comes first
/// while live and not declined; then the ranking over the rest.
pub(crate) fn decide(
    f: &PickFacts,
    live: &[Cand],
    left_out: Option<&str>,
    login_of: &dyn Fn(&str) -> String,
    tier5: &Tier5,
) -> Ranked {
    let out = excluded(f, login_of);
    let live: Vec<&Cand> = live
        .iter()
        .filter(|c| Some(c.key.as_str()) != left_out)
        .collect();
    let cands: Vec<Cand> = live
        .iter()
        .filter(|c| !out.contains(&c.key))
        .map(|c| (*c).clone())
        .collect();
    if let Some(k) = changes_agent(f, login_of)
        && !f.declined.contains_key(&k)
        && live.iter().any(|c| c.key == k)
    {
        return Ranked {
            related: related(&cands, Some(&k)),
            pick: Some(k),
            tier: SAME_AGENT,
        };
    }
    rank(&cands, &f.counts, &f.pr, tier5)
}

/// The roster's agents that may be asked (§12.5): other people's (not
/// `me`, not the PR's `author`), not gone and not on an OFFLINE device.
/// Tiers are left empty. `person_of` maps a board key to its person.
pub(crate) fn live_cands(
    roster: &[AgentCard],
    person_of: &dyn Fn(&str) -> String,
    me: &str,
    author: &str,
) -> Vec<Cand> {
    roster
        .iter()
        .filter(|a| a.status != "gone" && a.device_state != OFFLINE)
        .filter_map(|a| {
            let person = person_of(&a.key);
            (!person.is_empty() && person != me && person != author).then(|| Cand {
                key: a.key.clone(),
                person,
                idle: a.status == "idle",
                tiers: [false; 4],
            })
        })
        .collect()
}

/// Whether the asked agent still holds the ask: not gone and not on an
/// OFFLINE device. A card not folded here yet holds it: absence is lag,
/// not a trigger.
pub(crate) fn asked_live(roster: &[AgentCard], key: &str) -> bool {
    !roster
        .iter()
        .any(|a| a.key == key && (a.status == "gone" || a.device_state == OFFLINE))
}

/// Whether a lease or scope in `repo` bears on `want`, the PR card's repo:
/// either unknown counts.
fn same_repo(repo: &str, want: &str) -> bool {
    repo.is_empty() || want.is_empty() || coord::same_repo_key(repo, want)
}

/// The path part of a glob: a `path:<repo>:<glob>` lease name gives its
/// glob and repo, a bare glob itself.
fn glob_part(g: &str) -> (String, String) {
    match coord::parse_resource(g) {
        Ok(r) if r.kind == coord::RESOURCE_PATH => (r.repo_key, r.glob),
        _ => (String::new(), g.to_string()),
    }
}

/// Whether glob `g` in `repo` touches any of the PR's `paths` in `want`.
pub(crate) fn touches(repo: &str, g: &str, want: &str, paths: &[String]) -> bool {
    let (inner, glob) = glob_part(g);
    if !same_repo(repo, want) || !same_repo(&inner, want) {
        return false;
    }
    paths
        .iter()
        // A path that is no glob (a ':' in it) would overlap everything.
        .filter(|p| coord::normalize_glob(p).is_ok())
        .any(|p| coord::overlaps(&glob, p))
}

/// A candidate's tier signals (§12.5, D13). `login` is its person's
/// GitHub login.
pub(crate) fn tiers(key: &str, person: &str, login: &str, f: &PickFacts) -> [bool; 4] {
    let has = |v: &[String]| v.iter().any(|k| k == key);
    let work = f.related.iter().any(|r| {
        !coord::is_terminal(&r.state)
            && (has(&r.holders)
                || has(&r.co_holders)
                || has(&r.assigned)
                || has(&r.watchers)
                || has(&r.commenters))
    });
    let want = f.card.repo_key.as_str();
    let paths = &f.pr_state.paths;
    let files = f
        .path_leases
        .iter()
        .chain(&f.scopes)
        .any(|(h, repo, g)| h == key && touches(repo, g, want, paths));
    let login = login.trim();
    let there = !login.is_empty()
        && (f
            .pr_state
            .commenters
            .iter()
            .any(|c| c.eq_ignore_ascii_case(login))
            || f.pr_state
                .reviews
                .iter()
                .any(|r| r.login.eq_ignore_ascii_case(login)));
    let dedicated = f.reviewers.get(person).is_some_and(|k| k == key);
    [work, files, there, dedicated]
}

impl Services {
    /// One pick pass over the standing PR cards [`Services::pr_cards`]
    /// followed (§12.5): only those this person made, open, not a draft,
    /// not approved, in a space past the gate. Errors are logged.
    pub(super) fn review_pick(self: &Arc<Self>, repo_key: &str, standing: &[(Found, DeskPr)]) {
        for (f, p) in standing {
            if let Err(e) = self.pick_one(repo_key, f, p) {
                logf(format_args!("desk: PR #{} pick: {e}", p.number));
            }
        }
    }

    fn pick_one(&self, repo_key: &str, f: &Found, p: &DeskPr) -> Result<(), String> {
        let me = self.person_actor();
        let b = &f.board;
        let id = f.task.id.as_str();
        if f.task.created_by != me || coord::is_terminal(&f.task.state) || p.is_draft {
            return Ok(());
        }
        // Coord facts first, under one lock, released before anything else.
        let Some(facts) = b.coord().pick_facts(id) else {
            return Ok(());
        };
        if coord::is_terminal(&facts.card.state)
            || prcard::approved(&p.review_decision, &facts.pr_state.reviews, &p.head_ref_oid)
        {
            return Ok(());
        }
        let (old, unheard) = self
            .space_waiting_at(b, ReadsLevel::Reviews, REVIEWS_PROTOCOL)
            .map_err(|e| e.to_string())?;
        if !old.is_empty() || !unheard.is_empty() {
            return Ok(());
        }
        // The poll that made the card isn't here, so this is poll 1 at least.
        let polls = self.desk_watch.bump_pick_poll(id, &p.url);
        let roster = roster_of(self, b);
        let mut logins: BTreeMap<String, String> = BTreeMap::new();
        let mut login = |person: &str| {
            logins
                .entry(person.to_string())
                .or_insert_with(|| b.verifier.profile_github_login(person).trim().to_string())
                .clone()
        };
        let now = match &facts.phase {
            ReviewPhase::Asked { ask } => {
                let gone = !asked_live(&roster, &ask.to);
                let comments = self
                    .desk_watch
                    .activity_last(id, &p.url)
                    .map(|a| a.comments)
                    .unwrap_or_default();
                // Once another agent claimed after the ask, the asked
                // agent was told to comment instead: its person's words
                // on GitHub no longer show it working on the ask.
                let other_claimed = claimed_by_another(&facts, ask);
                AskedNow {
                    gone,
                    turn_ended: !gone && turn_ended(b, &ask.to, &ask.pos),
                    answered: !other_claimed
                        && answered_on_github(
                            &login(&key_person(b, &ask.to)),
                            &ask.at,
                            &comments,
                            &facts.pr_state.reviews,
                        ),
                }
            }
            _ => AskedNow::default(),
        };
        let (phase, left_out) = device_phase(&facts.phase, &now);
        if let Some(k) = &left_out {
            let why = if now.gone { "gone" } else { "turn ended" };
            logf(format_args!(
                "desk: PR #{}: {k} no longer holds the ask ({why})",
                p.number
            ));
        }
        if phase != ReviewPhase::Waiting {
            return Ok(());
        }
        let person_of = |k: &str| key_person(b, k);
        let mut live = live_cands(&roster, &person_of, &self.person_id(), &facts.author_person);
        for c in &mut live {
            let l = login(&c.person);
            c.tiers = tiers(&c.key, &c.person, &l, &facts);
        }
        let login_of = |k: &str| b.verifier.profile_github_login(&key_person(b, k));
        // The assistant's turn (§12.5, card #1081): once per PR, off this
        // thread. The pick waits for it and goes on at a later poll.
        let out = excluded(&facts, &login_of);
        let eligible: Vec<&Cand> = live
            .iter()
            .filter(|c| Some(c.key.as_str()) != left_out.as_deref() && !out.contains(&c.key))
            .collect();
        let (tier5, named) = if eligible.is_empty() {
            (Tier5::Skip, Vec::new())
        } else {
            let want_pick = !eligible.iter().any(|c| c.tiers.iter().any(|t| *t));
            let turn = self.choose_review(&format!("{id}\0{}", p.url), want_pick, || {
                let cands = self.ask_cands(b, &roster, &facts, &eligible);
                (
                    choose_ask(&p.title, &facts.pr_state.paths, &cands, want_pick),
                    cands,
                )
            });
            match turn {
                Turn::Skip => (Tier5::Skip, Vec::new()),
                Turn::Wait => {
                    logf(format_args!(
                        "desk: PR #{}: waiting on the assistant's turn",
                        p.number
                    ));
                    return Ok(());
                }
                Turn::Done(c) => (c.tier5, c.ping),
            }
        };
        let mut r = decide(&facts, &live, left_out.as_deref(), &login_of, &tier5);
        add_named(&mut r, &named, &eligible);
        let Some(pick) = r.pick else {
            // Tier 7 (§12.10): a new reviewer, on another person's machine.
            let member = polls < TIER7_POLLS && member_card_since(self, b, &facts);
            logf(format_args!(
                "desk: PR #{}: nobody to ask ({} live, poll {polls})",
                p.number,
                live.len()
            ));
            if tier7_allowed(polls, member) {
                let note = match self.start_reviewer(f, p, &facts)? {
                    super::review_start::Started::Nobody => Some(NOBODY_NOTE.into()),
                    _ => None,
                };
                self.desk_watch.set_note(repo_key, p.number, note);
            }
            return Ok(());
        };
        let co = b.coord();
        let after = facts.asks.last().map(|a| a.pos.clone());
        match co.pick_ask(&me, id, &pick, false, after) {
            Ok(Some(_)) => logf(format_args!(
                "desk: PR #{}: asked {pick} to review (tier {})",
                p.number, r.tier
            )),
            // An ask, claim or verdict landed since the facts were read.
            Ok(None) => return Ok(()),
            Err(e) => return Err(e.to_string()),
        }
        for k in r.related.iter().filter(|k| !facts.pinged.contains(*k)) {
            if let Err(e) = co.pick_ask(&me, id, k, true, None) {
                logf(format_args!("desk: PR #{}: ping {k}: {e}", p.number));
            }
        }
        Ok(())
    }
}

/// The agents the assistant named join the related ones (§12.7): only
/// candidates this pick may ask, never the pick itself.
pub(crate) fn add_named(r: &mut Ranked, named: &[String], eligible: &[&Cand]) {
    for k in named {
        if Some(k) != r.pick.as_ref()
            && !r.related.contains(k)
            && eligible.iter().any(|c| c.key == *k)
        {
            r.related.push(k.clone());
        }
    }
}

impl Services {
    /// The candidates as the assistant's turn sees them: name, person and
    /// the titles of the open cards they work on that bear on this PR.
    fn ask_cands(
        &self,
        b: &BoardLog,
        roster: &[AgentCard],
        facts: &PickFacts,
        eligible: &[&Cand],
    ) -> Vec<AskCand> {
        let co = b.coord();
        eligible
            .iter()
            .map(|c| {
                let name = roster
                    .iter()
                    .find(|a| a.key == c.key)
                    .map(|a| {
                        if a.name.trim().is_empty() {
                            a.title.clone()
                        } else {
                            a.name.clone()
                        }
                    })
                    .unwrap_or_default();
                let has = |v: &[String]| v.contains(&c.key);
                let cards = facts
                    .related
                    .iter()
                    .filter(|r| {
                        !coord::is_terminal(&r.state)
                            && (has(&r.holders)
                                || has(&r.co_holders)
                                || has(&r.assigned)
                                || has(&r.watchers)
                                || has(&r.commenters))
                    })
                    .filter_map(|r| co.get_task(&r.id).map(|t| t.title))
                    .collect();
                AskCand {
                    key: c.key.clone(),
                    name: if name.trim().is_empty() {
                        c.key.clone()
                    } else {
                        name
                    },
                    person: b.verifier.profile(&c.person).unwrap_or_default(),
                    cards,
                }
            })
            .collect()
    }
}

/// The person a board key belongs to: the verifier's word, else the card
/// map's (an agent whose card-of row hasn't come yet).
fn key_person(b: &BoardLog, key: &str) -> String {
    let p = b.verifier.key_person(key);
    if !p.is_empty() {
        return p;
    }
    key.strip_prefix(AGENT_ACTOR_PREFIX)
        .and_then(|id| lock(&b.maps).cards.get(id).map(|c| c.person_id.clone()))
        .unwrap_or_default()
}

/// Whether `agent`'s turn ended after the row at `pos` (§12.5): its
/// `agent.card` rows on the board after it, from its own person's
/// devices, go active and then idle. Rows are read in HLC order.
fn turn_ended(b: &BoardLog, agent: &str, pos: &RowPos) -> bool {
    let Some(card) = agent.strip_prefix(AGENT_ACTOR_PREFIX) else {
        return false;
    };
    let Some(owner) = b.verifier.card_person(card) else {
        return false;
    };
    let mut statuses: Vec<String> = Vec::new();
    let f = Filter {
        kinds: vec![kinds::AGENT_CARD.into()],
        after_hlc: store::Timestamp(pos.hlc),
        ..Filter::default()
    };
    let read = each_row(&b.store, f, |ev| {
        let Ok(c) = serde_json::from_slice::<BoardCard>(&ev.payload) else {
            return;
        };
        if c.card_id == card && b.verifier.person_of(&ev.device_id).as_deref() == Some(&owner) {
            statuses.push(c.status);
        }
    });
    if let Err(e) = read {
        logf(format_args!("desk: board {} card rows: {e}", b.id));
        return false;
    }
    untouched(&statuses)
}

/// Whether another member's device wrote an `agent.card` row on the board
/// since the PR card was made (§12.5): their agents had a chance to come.
fn member_card_since(s: &Services, b: &BoardLog, f: &PickFacts) -> bool {
    let Some(pos) = &f.created_pos else {
        return false;
    };
    let me = s.person_id();
    let mut seen = false;
    let flt = Filter {
        kinds: vec![kinds::AGENT_CARD.into()],
        after_hlc: store::Timestamp(pos.hlc),
        ..Filter::default()
    };
    let read = each_row(&b.store, flt, |ev| {
        if !seen {
            seen = b.verifier.person_of(&ev.device_id).is_some_and(|p| p != me);
        }
    });
    if let Err(e) = read {
        logf(format_args!("desk: board {} card rows: {e}", b.id));
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;
    use coord::{Lease, PastClaim, PrState, RelatedCard, ReviewRequest, Task};

    const PR: &str = "https://github.com/o/r/pull/7";
    const REPO: &str = "remote:github.com/o/r";

    fn cand(key: &str, person: &str, idle: bool, tiers: [bool; 4]) -> Cand {
        Cand {
            key: key.into(),
            person: person.into(),
            idle,
            tiers,
        }
    }

    const NONE: [bool; 4] = [false; 4];
    const T1: [bool; 4] = [true, false, false, false];
    const T2: [bool; 4] = [false, true, false, false];
    const T3: [bool; 4] = [false, false, true, false];
    const T4: [bool; 4] = [false, false, false, true];

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

    fn pick(cands: &[Cand], c: &BTreeMap<String, ClaimCounts>) -> (Option<String>, u8) {
        let r = rank(cands, c, PR, &Tier5::Skip);
        (r.pick, r.tier)
    }

    fn at(n: i64) -> GoTime {
        GoTime::unix_milli(n)
    }

    fn pos(hlc: u64) -> RowPos {
        RowPos {
            hlc,
            dev: "dev-a".into(),
            seq: hlc as i64,
        }
    }

    fn ask(to: &str, p: u64) -> ReviewRequest {
        ReviewRequest {
            task_id: "card".into(),
            pr: PR.into(),
            from: "person:pa".into(),
            to: to.into(),
            at: at(p as i64),
            wake: true,
            pos: pos(p),
            ..ReviewRequest::default()
        }
    }

    fn review(login: &str, state: &str, sha: &str, t: i64) -> PrReview {
        PrReview {
            login: login.into(),
            state: state.into(),
            sha: sha.into(),
            at: at(t),
        }
    }

    fn claim(holder: &str, t: i64, p: u64) -> PastClaim {
        PastClaim {
            holder: holder.into(),
            acquired_at: at(t),
            pos: pos(p),
        }
    }

    fn facts() -> PickFacts {
        PickFacts {
            card: Task {
                id: "card".into(),
                repo_key: REPO.into(),
                ..Task::default()
            },
            pr: PR.into(),
            author_person: "pa".into(),
            pr_state: PrState {
                pr: PR.into(),
                head: "h2".into(),
                ..PrState::default()
            },
            phase: ReviewPhase::Waiting,
            ..PickFacts::default()
        }
    }

    /// Agent `agent:bN` is person pb's, login "bob"; `agent:cN` pc's, "carol".
    fn login_of(k: &str) -> String {
        if k.starts_with("agent:b") {
            "bob".into()
        } else if k.starts_with("agent:c") {
            "carol".into()
        } else {
            String::new()
        }
    }

    #[test]
    fn each_tier_beats_the_next() {
        let c = BTreeMap::new();
        let all = [
            cand("agent:6", "p6", true, NONE),
            cand("agent:4", "p4", false, T4),
            cand("agent:3", "p3", false, T3),
            cand("agent:2", "p2", false, T2),
            cand("agent:1", "p1", false, T1),
        ];
        for (n, want) in [(5, 1u8), (4, 2), (3, 3), (2, 4), (1, 6)] {
            let (p, tier) = pick(&all[..n], &c);
            assert_eq!(tier, want, "{n} candidates");
            let key = if want == 6 {
                "agent:6".to_string()
            } else {
                format!("agent:{want}")
            };
            assert_eq!(p.as_deref(), Some(key.as_str()));
        }
    }

    #[test]
    fn fewer_open_claims_win_then_fewer_recent() {
        let cands = [
            cand("agent:a", "pa2", true, T1),
            cand("agent:b", "pb", true, T1),
        ];
        let c = counts(&[("pa2", 1, 0), ("pb", 0, 9)]);
        assert_eq!(pick(&cands, &c).0.as_deref(), Some("agent:b"), "open first");
        let c = counts(&[("pa2", 1, 3), ("pb", 1, 2)]);
        assert_eq!(
            pick(&cands, &c).0.as_deref(),
            Some("agent:b"),
            "then recent"
        );
        let c = counts(&[("pa2", 0, 0), ("pb", 1, 0)]);
        assert_eq!(pick(&cands, &c).0.as_deref(), Some("agent:a"));
    }

    #[test]
    fn counts_come_before_idle() {
        // A busy agent of a person with fewer claims beats an idle one of a
        // person with more.
        let cands = [
            cand("agent:idle", "pmany", true, T2),
            cand("agent:busy", "pfew", false, T2),
        ];
        let c = counts(&[("pmany", 0, 3), ("pfew", 0, 1)]);
        assert_eq!(pick(&cands, &c).0.as_deref(), Some("agent:busy"));
    }

    #[test]
    fn relevance_beats_a_lower_count() {
        let cands = [
            cand("agent:none", "p1", true, T2),
            cand("agent:many", "p2", false, T1),
        ];
        let c = counts(&[("p2", 5, 20)]);
        assert_eq!(pick(&cands, &c), (Some("agent:many".into()), 1));
    }

    #[test]
    fn idle_beats_busy_at_equal_counts() {
        let cands = [
            cand("agent:busy", "p1", false, T3),
            cand("agent:idle", "p2", true, T3),
        ];
        let c = counts(&[("p1", 1, 1), ("p2", 1, 1)]);
        assert_eq!(pick(&cands, &c).0.as_deref(), Some("agent:idle"));
    }

    #[test]
    fn the_hash_decides_a_full_tie() {
        let keys = ["agent:x1", "agent:x2", "agent:x3", "agent:x4"];
        let cands: Vec<Cand> = keys.iter().map(|k| cand(k, "p", true, NONE)).collect();
        let c = BTreeMap::new();
        let h = |k: &str| gocompat::fnv::fnv32a(format!("{PR}{k}").as_bytes());
        let want = keys.iter().min_by_key(|k| h(k)).unwrap();
        assert_eq!(pick(&cands, &c), (Some(want.to_string()), 6));
        // Deterministic, whatever the order the roster gives.
        let mut rev = cands.clone();
        rev.reverse();
        assert_eq!(pick(&rev, &c).0.as_deref(), Some(*want));
        // Another PR may pick another: the hash takes the PR too.
        let other = rank(&cands, &c, "https://github.com/o/r/pull/8", &Tier5::Skip);
        let h8 =
            |k: &str| gocompat::fnv::fnv32a(format!("https://github.com/o/r/pull/8{k}").as_bytes());
        assert_eq!(
            other.pick.as_deref(),
            Some(*keys.iter().min_by_key(|k| h8(k)).unwrap())
        );
    }

    #[test]
    fn busy_candidates_with_no_signal_reach_tier_7() {
        let cands = [
            cand("agent:a", "p1", false, NONE),
            cand("agent:b", "p2", false, NONE),
        ];
        let r = rank(&cands, &BTreeMap::new(), PR, &Tier5::Skip);
        assert_eq!((r.pick, r.tier), (None, NOBODY));
        assert!(r.related.is_empty());
        let r = rank(&[], &BTreeMap::new(), PR, &Tier5::Skip);
        assert_eq!((r.pick, r.tier), (None, NOBODY));
    }

    #[test]
    fn tier_5_skip_and_nobody_go_on_to_tier_6() {
        let cands = [
            cand("agent:busy", "p1", false, NONE),
            cand("agent:idle", "p2", true, NONE),
        ];
        let c = BTreeMap::new();
        for t5 in [
            Tier5::Skip,
            Tier5::Nobody,
            Tier5::Pick("agent:stranger".into()),
        ] {
            let r = rank(&cands, &c, PR, &t5);
            assert_eq!(
                (r.pick.as_deref(), r.tier),
                (Some("agent:idle"), 6),
                "{t5:?}"
            );
        }
        let r = rank(&cands, &c, PR, &Tier5::Pick("agent:busy".into()));
        assert_eq!((r.pick.as_deref(), r.tier), (Some("agent:busy"), 5));
        // Tiers 1 to 4 come before the assistant.
        let mut with = cands.to_vec();
        with.push(cand("agent:t4", "p3", false, T4));
        let r = rank(&with, &c, PR, &Tier5::Pick("agent:busy".into()));
        assert_eq!(r.pick.as_deref(), Some("agent:t4"));
    }

    #[test]
    fn related_are_tiers_1_to_3_other_than_the_pick() {
        let cands = [
            cand("agent:1", "p1", true, T1),
            cand("agent:2", "p2", true, T2),
            cand("agent:3", "p3", true, T3),
            cand("agent:4", "p4", true, T4),
            cand("agent:6", "p6", true, NONE),
        ];
        let r = rank(&cands, &BTreeMap::new(), PR, &Tier5::Skip);
        assert_eq!(r.pick.as_deref(), Some("agent:1"));
        assert_eq!(r.related, ["agent:2", "agent:3"]);
    }

    #[test]
    fn tier_7_waits_for_three_polls_or_a_member_row() {
        assert!(!tier7_allowed(1, false));
        assert!(!tier7_allowed(2, false));
        assert!(tier7_allowed(3, false));
        assert!(tier7_allowed(1, true));
    }

    #[test]
    fn a_turn_ends_on_active_then_idle() {
        assert!(untouched(&["active", "idle"]));
        assert!(untouched(&["idle", "active", "active", "idle"]));
        assert!(!untouched(&["idle", "active"]));
        assert!(!untouched(&["idle"]));
        assert!(!untouched::<&str>(&[]));
        assert!(!untouched(&["active", "gone"]));
    }

    fn card(key: &str, status: &str, state: &str) -> AgentCard {
        AgentCard {
            key: key.into(),
            status: status.into(),
            device_state: state.into(),
            ..AgentCard::default()
        }
    }

    #[test]
    fn offline_gone_mine_and_the_authors_are_never_candidates() {
        let roster = [
            card("agent:b1", "idle", "connected"),
            card("agent:b2", "active", ""),
            card("agent:b3", "idle", OFFLINE),
            card("agent:b4", "gone", "connected"),
            card("agent:m1", "idle", "connected"),
            card("agent:a1", "idle", "connected"),
            card("agent:x1", "idle", "connected"),
        ];
        let person = |k: &str| match &k[..7] {
            "agent:b" => "pb".to_string(),
            "agent:m" => "pme".to_string(),
            "agent:a" => "pa".to_string(),
            _ => String::new(),
        };
        let got = live_cands(&roster, &person, "pme", "pa");
        let keys: Vec<_> = got.iter().map(|c| (c.key.as_str(), c.idle)).collect();
        assert_eq!(keys, [("agent:b1", true), ("agent:b2", false)]);
        assert!(got.iter().all(|c| c.person == "pb"));
        assert!(asked_live(&roster, "agent:b2"));
        assert!(!asked_live(&roster, "agent:b3"), "offline");
        assert!(!asked_live(&roster, "agent:b4"), "gone");
        assert!(asked_live(&roster, "agent:zz"), "not folded yet holds");
    }

    fn asked(to: &str) -> ReviewPhase {
        ReviewPhase::Asked { ask: ask(to, 10) }
    }

    #[test]
    fn an_asked_agent_offline_gone_or_untouched_is_left_out() {
        let p = asked("agent:b1");
        let quiet = AskedNow::default();
        assert_eq!(device_phase(&p, &quiet), (p.clone(), None));
        for now in [
            AskedNow {
                gone: true,
                ..quiet
            },
            AskedNow {
                turn_ended: true,
                ..quiet
            },
            AskedNow {
                gone: true,
                answered: true,
                ..quiet
            },
        ] {
            assert_eq!(
                device_phase(&p, &now),
                (ReviewPhase::Waiting, Some("agent:b1".into())),
                "{now:?}"
            );
        }
        // A turn that ended with a word on GitHub was not untouched.
        let spoke = AskedNow {
            turn_ended: true,
            answered: true,
            ..quiet
        };
        assert_eq!(device_phase(&p, &spoke), (p.clone(), None));
        // Only Asked turns: Claimed, Reviewed and Off stay as they are.
        for ph in [ReviewPhase::Reviewed, ReviewPhase::Off] {
            let now = AskedNow {
                gone: true,
                ..quiet
            };
            assert_eq!(device_phase(&ph, &now), (ph.clone(), None));
        }
        // And the one left out is not picked: the other is.
        let live = [
            cand("agent:b1", "pb", true, T1),
            cand("agent:c1", "pc", false, NONE),
        ];
        let mut f = facts();
        f.asks = vec![ask("agent:b1", 10)];
        let r = decide(&f, &live, Some("agent:b1"), &login_of, &Tier5::Skip);
        assert_eq!(r.pick, None, "busy c1 has no signal");
        let live = [
            cand("agent:b1", "pb", true, T1),
            cand("agent:c1", "pc", true, NONE),
        ];
        let r = decide(&f, &live, Some("agent:b1"), &login_of, &Tier5::Skip);
        assert_eq!(r.pick.as_deref(), Some("agent:c1"));
        assert!(r.related.is_empty(), "the one left out isn't pinged");
    }

    /// The chain pick_one runs, from a roster: an OFFLINE agent is never a
    /// candidate, nor pinged, however it ranks; the asked agent's device
    /// going OFFLINE turns Asked into Waiting and the other is asked.
    #[test]
    fn an_offline_agent_is_never_asked_and_an_asked_one_going_offline_re_picks() {
        let person = |k: &str| if k.starts_with("agent:b") { "pb" } else { "pc" }.to_string();
        let mut f = facts();
        f.related = vec![RelatedCard {
            id: "card".into(),
            why: coord::RELATED_PR_CARD,
            state: "open".into(),
            watchers: vec!["agent:b1".into(), "agent:c1".into()],
            ..RelatedCard::default()
        }];
        let pass = |f: &PickFacts, roster: &[AgentCard]| {
            let now = match &f.phase {
                ReviewPhase::Asked { ask } => AskedNow {
                    gone: !asked_live(roster, &ask.to),
                    ..AskedNow::default()
                },
                _ => AskedNow::default(),
            };
            let (phase, left_out) = device_phase(&f.phase, &now);
            if phase != ReviewPhase::Waiting {
                return None;
            }
            let mut live = live_cands(roster, &person, "pme", &f.author_person);
            for c in &mut live {
                c.tiers = tiers(&c.key, &c.person, "", f);
            }
            Some(decide(
                f,
                &live,
                left_out.as_deref(),
                &login_of,
                &Tier5::Skip,
            ))
        };
        // b1 ranks first (idle, fewer claims), but its device is OFFLINE.
        f.counts = counts(&[("pc", 3, 9)]);
        let roster = [
            card("agent:b1", "idle", OFFLINE),
            card("agent:c1", "active", "connected"),
        ];
        let r = pass(&f, &roster).unwrap();
        assert_eq!(r.pick.as_deref(), Some("agent:c1"));
        assert!(r.related.is_empty(), "the OFFLINE agent pinged");
        // Alone and OFFLINE: nobody.
        let r = pass(&f, &roster[..1]).unwrap();
        assert_eq!((r.pick, r.tier), (None, NOBODY));

        // c1 is asked; while its device answers, nothing changes.
        f.asks = vec![ask("agent:c1", 10)];
        f.phase = asked("agent:c1");
        let roster = [
            card("agent:b1", "idle", "connected"),
            card("agent:c1", "active", "connected"),
        ];
        assert_eq!(pass(&f, &roster), None);
        // Its device goes OFFLINE: b1 is asked, and c1 isn't pinged.
        let roster = [
            card("agent:b1", "idle", "connected"),
            card("agent:c1", "active", OFFLINE),
        ];
        let r = pass(&f, &roster).unwrap();
        assert_eq!(r.pick.as_deref(), Some("agent:b1"));
        assert!(r.related.is_empty(), "{:?}", r.related);
        // Once that ask is written, c1 stays out though its device is back.
        f.asks.push(ask("agent:b1", 20));
        f.phase = ReviewPhase::Waiting;
        let roster = [card("agent:c1", "idle", "connected")];
        let r = pass(&f, &roster).unwrap();
        assert_eq!(r.pick, None);
    }

    #[test]
    fn answers_on_github_count_after_the_ask_only() {
        let c = PrComment {
            login: "Bob".into(),
            at: at(20),
            ..PrComment::default()
        };
        assert!(answered_on_github(
            "bob",
            &at(10),
            std::slice::from_ref(&c),
            &[]
        ));
        assert!(!answered_on_github("bob", &at(30), &[c], &[]));
        let r = review("BOB", "COMMENTED", "h", 20);
        assert!(answered_on_github(
            "bob",
            &at(10),
            &[],
            std::slice::from_ref(&r)
        ));
        assert!(!answered_on_github("", &at(10), &[], &[r]));
    }

    #[test]
    fn declined_lapsed_and_moved_on_agents_are_excluded() {
        let mut f = facts();
        f.declined.insert("agent:b1".into(), pos(5));
        // b2 claimed and let it lapse with no verdict; c1 claimed and
        // gave one; c2 holds it now.
        f.claims = vec![
            claim("agent:b2", 100, 1),
            claim("agent:c1", 200, 2),
            claim("agent:c2", 300, 3),
        ];
        f.pr_state.reviews = vec![review("carol", "CHANGES_REQUESTED", "h1", 250)];
        f.live_claim = Some(Lease {
            holder: "agent:c2".into(),
            ..Lease::default()
        });
        // b3 was asked, then the ask moved on to b4.
        f.asks = vec![ask("agent:b3", 4), ask("agent:b4", 6)];
        let out = excluded(&f, &login_of);
        let want: BTreeSet<String> = ["agent:b1", "agent:b2", "agent:b3"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(out, want);
        // A verdict before the claim doesn't save it.
        f.pr_state.reviews = vec![review("bob", "APPROVED", "h1", 50)];
        assert!(excluded(&f, &login_of).contains("agent:b2"));
        // Asked again later, b3 is not left out.
        f.asks.push(ask("agent:b3", 8));
        let out = excluded(&f, &login_of);
        assert!(!out.contains("agent:b3") && out.contains("agent:b4"));
    }

    #[test]
    fn an_agents_ask_to_several_leaves_none_out() {
        let mut f = facts();
        // One board_review with no `to` before the PR card stood: a row
        // for each other person's dedicated reviewer.
        let mut b1 = ask("agent:b1", 4);
        let mut c1 = ask("agent:c1", 5);
        for a in [&mut b1, &mut c1] {
            a.from = "agent:x1".into();
        }
        f.asks = vec![b1, c1];
        assert!(excluded(&f, &login_of).is_empty());
        // c1 goes OFFLINE: the re-pick may still ask b1, a tier 4 match.
        let live = [cand("agent:b1", "pb", true, T4)];
        let r = decide(&f, &live, Some("agent:c1"), &login_of, &Tier5::Skip);
        assert_eq!((r.pick.as_deref(), r.tier), (Some("agent:b1"), 4));
        // The pick that replaced them leaves both out.
        f.asks.push(ask("agent:b2", 6));
        let out = excluded(&f, &login_of);
        assert!(out.contains("agent:b1") && out.contains("agent:c1"));
    }

    #[test]
    fn a_declined_agent_is_never_picked() {
        let mut f = facts();
        f.declined.insert("agent:b1".into(), pos(5));
        let live = [cand("agent:b1", "pb", true, T1)];
        let r = decide(&f, &live, None, &login_of, &Tier5::Skip);
        assert_eq!((r.pick, r.tier), (None, NOBODY));
    }

    #[test]
    fn after_changes_requested_the_same_agent_is_asked_at_a_new_head() {
        let mut f = facts();
        f.asks = vec![ask("agent:b1", 1)];
        f.claims = vec![claim("agent:b1", 100, 2)];
        f.pr_state.reviews = vec![review("bob", "CHANGES_REQUESTED", "h1", 200)];
        // At the head the verdict was given on, the phase is Reviewed: no
        // pick runs.
        f.pr_state.head = "h1".into();
        f.phase = ReviewPhase::Reviewed;
        let (ph, _) = device_phase(&f.phase, &AskedNow::default());
        assert_eq!(ph, ReviewPhase::Reviewed);
        assert_eq!(changes_agent(&f, &login_of), None);
        // A new head: Waiting, and b1 comes first, over a tier-1 agent.
        f.pr_state.head = "h2".into();
        f.phase = ReviewPhase::Waiting;
        let live = [
            cand("agent:b1", "pb", false, NONE),
            cand("agent:c1", "pc", true, T1),
        ];
        let r = decide(&f, &live, None, &login_of, &Tier5::Skip);
        assert_eq!((r.pick.as_deref(), r.tier), (Some("agent:b1"), SAME_AGENT));
        assert_eq!(r.related, ["agent:c1"]);
        // Not while it is gone (not live) or declined.
        let r = decide(&f, &live[1..], None, &login_of, &Tier5::Skip);
        assert_eq!(r.pick.as_deref(), Some("agent:c1"));
        let mut d = f.clone();
        d.declined.insert("agent:b1".into(), pos(3));
        let r = decide(&d, &live, None, &login_of, &Tier5::Skip);
        assert_eq!(r.pick.as_deref(), Some("agent:c1"));
        // Spent once an ask follows the claim.
        f.asks.push(ask("agent:b1", 5));
        assert_eq!(changes_agent(&f, &login_of), None);
        // An approval newer than the changes isn't a re-ask.
        let mut g = facts();
        g.claims = vec![claim("agent:b1", 100, 2)];
        g.pr_state.reviews = vec![
            review("bob", "CHANGES_REQUESTED", "h1", 200),
            review("carol", "APPROVED", "h1", 300),
        ];
        assert_eq!(changes_agent(&g, &login_of), None);
    }

    #[test]
    fn tier_signals_come_from_the_facts() {
        let mut f = facts();
        f.related = vec![
            RelatedCard {
                id: "card".into(),
                why: coord::RELATED_PR_CARD,
                state: "open".into(),
                watchers: vec!["agent:w".into()],
                ..RelatedCard::default()
            },
            RelatedCard {
                id: "work".into(),
                why: coord::RELATED_LINKS,
                state: "in_progress".into(),
                holders: vec!["agent:h".into()],
                commenters: vec!["agent:said".into()],
                ..RelatedCard::default()
            },
            RelatedCard {
                id: "old".into(),
                why: coord::RELATED_SIBLING,
                state: coord::COMPLETED.into(),
                holders: vec!["agent:done".into()],
                ..RelatedCard::default()
            },
        ];
        f.pr_state.paths = vec!["src/loader.rs".into(), "docs/a.md".into()];
        f.path_leases = vec![
            ("agent:lease".into(), REPO.into(), "src/**".into()),
            (
                "agent:elsewhere".into(),
                "remote:github.com/o/other".into(),
                "src/**".into(),
            ),
        ];
        f.scopes = vec![("agent:scope".into(), String::new(), "docs/*.md".into())];
        f.pr_state.commenters = vec!["Carol".into()];
        f.pr_state.reviews = vec![review("dave", "COMMENTED", "h", 1)];
        f.reviewers.insert("pr".into(), "agent:rev".into());
        let t = |k: &str, p: &str, l: &str| tiers(k, p, l, &f);
        assert_eq!(t("agent:w", "p", ""), T1);
        assert_eq!(t("agent:h", "p", ""), T1);
        assert_eq!(t("agent:said", "p", ""), T1);
        assert_eq!(t("agent:done", "p", ""), NONE, "a done card isn't work");
        assert_eq!(t("agent:lease", "p", ""), T2);
        assert_eq!(t("agent:elsewhere", "p", ""), NONE, "another repo's lease");
        assert_eq!(t("agent:scope", "p", ""), T2);
        assert_eq!(t("agent:x", "p", "carol"), T3);
        assert_eq!(t("agent:x", "p", "DAVE"), T3);
        assert_eq!(t("agent:x", "p", ""), NONE);
        assert_eq!(t("agent:rev", "pr", ""), T4);
        assert_eq!(t("agent:rev", "other", ""), NONE);
    }

    #[test]
    fn a_glob_with_a_repo_prefix_compares_its_path_part() {
        let paths = vec!["src/loader.rs".to_string()];
        assert!(touches("", "src/**", REPO, &paths));
        assert!(touches("", &format!("path:{REPO}:src/*.rs"), REPO, &paths));
        assert!(!touches(
            "",
            "path:remote:github.com/o/other:src/**",
            REPO,
            &paths
        ));
        assert!(!touches("", "docs/**", REPO, &paths));
        assert!(!touches(REPO, "src/**", REPO, &[]));
        // A path that is no glob doesn't match everything.
        assert!(!touches("", "docs/**", REPO, &["a:b".to_string()]));
    }

    #[test]
    fn another_agents_claim_after_the_ask_stops_github_words_counting() {
        let mut f = facts();
        let a = ask("agent:x", 10);
        f.claims = vec![claim("agent:x", 1, 11)];
        assert!(!claimed_by_another(&f, &a), "its own claim");
        f.claims = vec![claim("agent:z", 1, 9)];
        assert!(!claimed_by_another(&f, &a), "a claim before the ask");
        f.claims.push(claim("agent:z", 2, 12));
        assert!(claimed_by_another(&f, &a));
    }
}
