//! Irori tidies the person's own board (card #786): what its sweeps do is
//! kept as a record the card's thread shows, with an Undo.
//!
//! A record ([`Did`]) folds from `assistant.did` rows onto its card, or
//! onto no card for a lease notice. Its id is built from the job, the card
//! and a key, so two devices that do the same thing write one record.
//! Reading it from the Board is what keeps a restart from repeating a job,
//! and a job's own writes re-fire changes, so every job checks for its
//! record first. Private board only: a shared board's log refuses the
//! kind, and no job writes there.

use std::time::Duration;

use gocompat::time::GoTime;
use serde::Serialize;
use store::Event;
use store::kinds;

use super::cards::{clean_options, count_open};
use super::error::{Result, bad_state, not_allowed, not_found};
use super::fold::{Fold, LeaseRec};
use super::later::Back;
use super::rows::{DidRow, row};
use super::types::*;
use super::util::SWEEPER;
use super::{DONE_ANSWER, MAX_OBJECTIVE};
use crate::gotime;

/// The sweep put a card back after its agent went away.
pub const JOB_HANDED: &str = "handback";
/// The sweep put an open ask back after the agent that took it went away:
/// shown as a line, but a Not yet can't stop an ask's hand-out.
pub const JOB_HANDED_ASK: &str = "handback.ask";
/// Irori asked whether a card whose PRs are all merged is done.
pub const JOB_MERGED: &str = "merged";
/// A lease held by another of the person's devices that is offline.
pub const JOB_LEASE: &str = "lease";

/// Irori linked two open cards that look like the same work. One record
/// on each card, `key` naming the other.
pub const JOB_DUPLICATE: &str = "duplicate";

/// The reason on the `TASK_CHANGED` a PR's new state makes, so a listener
/// can tell it from any other change to the card.
pub const REASON_PR_STATE: &str = "pr_state";

/// How long a device's lease holder has been away before Irori tells the
/// person, on top of the lease being past its `until`.
pub const LEASE_OFFLINE_AFTER: Duration = Duration::from_secs(30 * 60);

const MERGED_QUESTION: &str = "Its PRs are all merged and nobody is on it. Is this done?";
const HANDBACK_WHY: &str = "Irori handed it back and you asked not to offer it again";
const MERGED_WHY: &str = "Its PRs are merged and you undid Irori's question, so it isn't offered";

/// One thing Irori did, as the Board keeps it (card #786).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Did {
    pub id: String,
    pub job: String,
    pub key: String,
    pub text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub blocker_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub resource: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub holder: String,
    pub by: String,
    pub at: GoTime,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub undone: bool,
    /// The record row's place in the log, for the thread's order.
    #[serde(skip)]
    pub hlc: u64,
}

impl Did {
    fn from_row(p: &DidRow, hlc: u64) -> Did {
        Did {
            id: p.id.clone(),
            job: p.job.clone(),
            key: p.key.clone(),
            text: p.text.clone(),
            blocker_id: p.blocker_id.clone(),
            resource: p.resource.clone(),
            holder: p.holder.clone(),
            by: p.by.clone(),
            at: p.at,
            undone: p.undo,
            hlc,
        }
    }
}

/// A record's id: the same job on the same card for the same key is one
/// record, whichever device writes it.
pub fn did_id(job: &str, card: &str, key: &str) -> String {
    format!("{job}:{card}:{key}")
}

/// Whether the person can still Undo record `d` on card t: a hand-back
/// while nobody holds the card and it can be held back, a merged flag
/// while its question is open.
pub fn did_undoable(t: &Task, d: &Did) -> bool {
    if d.undone {
        return false;
    }
    match d.job.as_str() {
        JOB_HANDED => {
            t.holders.is_empty()
                && t.co_holders.is_empty()
                && !is_terminal(&t.state)
                && t.open_ask().is_none()
                && t.parked().is_none()
                && t.state != INPUT_REQUIRED
                && !t.blockers.iter().any(|b| {
                    b.open()
                        && b.later.is_empty()
                        && !b
                            .on
                            .as_ref()
                            .is_some_and(|on| super::offers::on_work(&on.kind))
                })
        }
        JOB_MERGED => t.blockers.iter().any(|b| b.id == d.blocker_id && b.open()),
        // Only a note on two cards: it holds nothing, so it can always go.
        JOB_DUPLICATE => true,
        _ => false,
    }
}

/// Puts row p's record into `list`, or marks it undone. An undo that
/// arrives before its record is kept and filled in when the record comes.
fn merge_did(list: &mut Vec<Did>, p: &DidRow, hlc: u64) {
    match list.iter_mut().find(|d| d.id == p.id) {
        Some(d) => {
            if p.undo {
                d.undone = true;
            } else if d.text.is_empty() {
                let undone = d.undone;
                *d = Did::from_row(p, hlc);
                d.undone = undone;
            }
        }
        None => list.push(Did::from_row(p, hlc)),
    }
}

impl Fold {
    pub(crate) fn fold_did(&mut self, ev: &Event) {
        let Some(p) = row::<DidRow>(&ev.payload) else {
            return;
        };
        if self.on_board || p.id.is_empty() {
            return;
        }
        if p.task_id.is_empty() {
            merge_did(&mut self.loose_did, &p, ev.hlc.0);
            self.note(Change {
                kind: LEASE_CHANGED.into(),
                resource: p.resource,
                ..Change::default()
            });
            return;
        }
        let Some(t) = self.tasks.get_mut(&p.task_id) else {
            // Ahead of its card: it joins it when the card folds.
            merge_did(
                self.did_early.entry(p.task_id.clone()).or_default(),
                &p,
                ev.hlc.0,
            );
            return;
        };
        merge_did(&mut t.did, &p, ev.hlc.0);
        self.note(Change {
            kind: TASK_CHANGED.into(),
            task_id: p.task_id,
            ..Change::default()
        });
    }

    /// Records that waited for card `id`, now it is here.
    pub(crate) fn drain_early_did(&mut self, id: &str) {
        let Some(rows) = self.did_early.remove(id) else {
            return;
        };
        if let Some(t) = self.tasks.get_mut(id) {
            t.did = rows;
        }
    }

    fn has_did(&self, t: &Task, id: &str) -> bool {
        t.did.iter().any(|d| d.id == id)
    }

    /// Whether this device's jobs run at all, and the card's on it.
    fn tidying(&self) -> bool {
        self.tidy && !self.on_board
    }

    fn write_did(&mut self, row: DidRow) -> Result<()> {
        self.write(SWEEPER, kinds::ASSISTANT_DID, &row)
    }

    /// How a hand-back line names the agent that left, for the person:
    /// its name, never its key. An agent the roster has forgotten may
    /// still be named by the card's spawn records.
    fn gone_agent(&self, id: &str, who: &str) -> String {
        self.roster_name(who)
            .or_else(|| crate::notifications::spawned_name(&self.tasks[id], who))
            .unwrap_or_else(|| "its agent".to_string())
    }

    /// Writes "Irori handed this back" for the card the sweep just took
    /// a gone holder off. Only the device doing the hand-back calls it.
    /// A refused record never stops the hand-back.
    pub(crate) fn did_hand_back(&mut self, id: &str, who: &str, went: &str, at: &GoTime) {
        if !self.tidying() {
            return;
        }
        let Some(key) = self.tasks[id].returned.as_ref().map(|r| r.row.clone()) else {
            return;
        };
        let did = did_id(JOB_HANDED, id, &key);
        if self.has_did(&self.tasks[id], &did) {
            return;
        }
        let name = self.gone_agent(id, who);
        let _ = self.write_did(DidRow {
            id: did,
            task_id: id.into(),
            job: JOB_HANDED.into(),
            key,
            text: format!("Irori handed this back: {name} {went}"),
            holder: who.into(),
            by: SWEEPER.into(),
            at: *at,
            ..DidRow::default()
        });
    }

    /// The sweep's hand-back of an open ask whose taker went away. Keyed
    /// on the taker and the hand-back's time.
    pub(crate) fn did_hand_back_ask(&mut self, id: &str, who: &str, went: &str, at: &GoTime) {
        if !self.tidying() {
            return;
        }
        let key = format!("{who}:{}", at.unix_nano());
        let did = did_id(JOB_HANDED_ASK, id, &key);
        if self.has_did(&self.tasks[id], &did) {
            return;
        }
        let name = self.gone_agent(id, who);
        let _ = self.write_did(DidRow {
            id: did,
            task_id: id.into(),
            job: JOB_HANDED_ASK.into(),
            key,
            text: format!("Irori handed this back: {name} {went}"),
            holder: who.into(),
            by: SWEEPER.into(),
            at: *at,
            ..DidRow::default()
        });
    }

    /// Whether card t could be flagged merged-and-done now.
    fn may_flag_merged(&self, t: &Task) -> bool {
        self.tidying()
            && matches!(t.state.as_str(), WORKING | SUBMITTED | IN_REVIEW)
            && t.holders.is_empty()
            && t.co_holders.is_empty()
            && t.assigned.is_empty()
            && !t.shared
            && !t.withdrawn
            && t.given.is_none()
            && !t.spawn_pending()
            && t.open_ask().is_none()
            && t.parked().is_none()
            // A parent with work still open isn't done.
            && !t
                .subtasks
                .iter()
                .any(|s| self.tasks.get(s).is_some_and(|k| !is_terminal(&k.state)))
            // Already asked (an agent's completed asks, card #112).
            && !t.blockers.iter().any(|b| b.open() && b.done_check)
            && self.all_prs_merged(t)
            && self.merged_since_made(t)
    }

    /// Whether some PR of t merged after t was made. One already merged
    /// when the card was made is a link for reference, not the card's work
    /// (#230); no field says so, so its time does. A merge with no known
    /// time isn't held against the card.
    fn merged_since_made(&self, t: &Task) -> bool {
        self.pr_states_of(t)
            .iter()
            .any(|p| p.merged_at.is_zero() || gotime::after(&p.merged_at, &t.created_at))
    }

    /// The open cards this device would ask about now and hasn't: what a
    /// dropped change (`RESYNC`) may have lost.
    pub(crate) fn merged_unasked(&self) -> Vec<String> {
        self.tasks
            .values()
            .filter(|t| {
                self.may_flag_merged(t)
                    && self.spawns_out(t)
                    && !self.has_did(t, &did_id(JOB_MERGED, &t.id, &Self::merged_key(t)))
            })
            .map(|t| t.id.clone())
            .collect()
    }

    /// The key that makes card t's merged record: its PRs, so a PR linked
    /// and merged later can ask again, and Undo stops this set for good.
    fn merged_key(t: &Task) -> String {
        let mut prs: Vec<&str> = t
            .links
            .iter()
            .filter(|l| super::links::pr_link(l).is_some())
            .map(|l| l.reference.as_str())
            .collect();
        prs.sort_unstable();
        prs.join(",")
    }

    /// Asks the person whether card `id`, every PR of it merged and nobody
    /// on it, is done, as Irori. A record already there means asked (or
    /// undone) before: nothing more.
    pub(crate) fn flag_merged(&mut self, id: &str) -> Result<bool> {
        let Some(t) = self.tasks.get(id).cloned() else {
            return Ok(false);
        };
        if !self.may_flag_merged(&t) {
            return Ok(false);
        }
        let key = Self::merged_key(&t);
        let did = did_id(JOB_MERGED, id, &key);
        if self.has_did(&t, &did) {
            return Ok(false);
        }
        let now = self.stamp();
        let opts = clean_options(
            &[BlockerOption {
                answer: DONE_ANSWER.into(),
                why: "Closes the card.".into(),
                recommended: true,
                ..BlockerOption::default()
            }],
            SWEEPER,
            now,
        )?;
        let prs = key.replace(',', ", ");
        let cx = format!(
            "Merged: {prs}. Nobody is on it, so nothing closes it unless you do. Mark it done, or Undo on the thread leaves it open, not offered to an agent, and stops this asking."
        );
        let (_, b) = self.raise_locked(
            SWEEPER,
            &t,
            MERGED_QUESTION,
            MERGED_QUESTION,
            &gocompat_cut(&cx),
            opts,
            Some(BlockedBy {
                kind: BLOCKED_ON_PERSON.into(),
                reference: String::new(),
            }),
            false,
            now,
            Vec::new(),
            Vec::new(),
            true,
            false,
        )?;
        self.write_did(DidRow {
            id: did,
            task_id: id.into(),
            job: JOB_MERGED.into(),
            key,
            text: "Irori asked if this is done: its PRs are all merged and nobody is on it".into(),
            blocker_id: b.id,
            by: SWEEPER.into(),
            at: now,
            ..DidRow::default()
        })?;
        Ok(true)
    }

    /// Whether l is held by another device that has been away long enough,
    /// and is past its `until`: the only lease Irori tells about, and the
    /// only one a notice's Release may take.
    fn lease_stranded(&self, l: &LeaseRec, now: &GoTime) -> bool {
        !self.lease_local(l) && gotime::after(now, &l.lease.until) && self.live(l, now) && {
            let dev = self.lease_dev(l);
            dev != self.dev && self.away_for(dev, LEASE_OFFLINE_AFTER)
        }
    }

    /// Tells the person about leases held by another of their devices
    /// that has been offline past [`LEASE_OFFLINE_AFTER`] and past the
    /// lease's `until`. Never releases one (v0.4-devices: nothing is
    /// force-released automatically).
    pub(crate) fn note_offline_leases(&mut self, now: &GoTime) {
        if !self.tidying() {
            return;
        }
        let found: Vec<Lease> = self
            .held
            .values()
            .filter(|l| self.lease_stranded(l, now))
            .map(|l| l.lease.clone())
            .collect();
        for l in found {
            let key = lease_key(&l);
            let did = did_id(JOB_LEASE, "", &key);
            if self.loose_did.iter().any(|d| d.id == did) {
                continue;
            }
            let name = self.named(&l.holder);
            let _ = self.write_did(DidRow {
                id: did,
                job: JOB_LEASE.into(),
                key,
                text: format!(
                    "Lease on {} is held by {name} on a device that's offline, and its time is up",
                    l.resource
                ),
                resource: l.resource.clone(),
                holder: l.holder.clone(),
                by: SWEEPER.into(),
                at: *now,
                ..DidRow::default()
            });
        }
    }

    /// The lease notices still in force: not undone, the same lease as
    /// when told about, and still stranded. One whose device came back or
    /// whose lease was renewed goes, and returns if the lease strands again.
    pub(crate) fn lease_notices(&self) -> Vec<Did> {
        let now = self.stamp();
        self.loose_did
            .iter()
            .filter(|d| {
                !d.undone
                    && d.job == JOB_LEASE
                    && self
                        .held
                        .values()
                        .any(|l| lease_key(&l.lease) == d.key && self.lease_stranded(l, &now))
            })
            .cloned()
            .collect()
    }

    pub(crate) fn did_can_undo(&self, t: &Task, d: &Did) -> bool {
        did_undoable(t, d)
    }

    /// The person's Undo on record `did`: a hand-back is held back from
    /// being offered, a merged flag is taken back, a lease notice is
    /// dropped. The record then keeps the job from doing it again.
    pub(crate) fn undo_did(&mut self, by: &str, id: &str, did: &str) -> Result<Option<Task>> {
        if !self.is_person(by) {
            return Err(not_allowed("only the person undoes what Irori did"));
        }
        if self.on_board {
            return Err(not_allowed("Irori tidies the person's own board only"));
        }
        if id.is_empty() {
            return self.undo_lease_notice(by, did);
        }
        let t = self.any_task(id)?.clone();
        let Some(d) = t.did.iter().find(|d| d.id == did).cloned() else {
            return Err(not_found(format!("{} has no record {did}", t.short_ref())));
        };
        if !self.did_can_undo(&t, &d) {
            return Err(bad_state(format!(
                "{}: this can't be undone any more",
                t.short_ref()
            )));
        }
        match d.job.as_str() {
            JOB_HANDED => {
                self.park(by, id, HANDBACK_WHY, Back::Word)?;
            }
            JOB_MERGED => {
                self.take_back_merged(by, &t, &d.blocker_id)?;
            }
            JOB_DUPLICATE => self.undo_twin(by, id, &d)?,
            _ => return Err(bad_state("this can't be undone")),
        }
        self.write_undo(by, id, &d)?;
        Ok(Some(self.tasks[id].clone()))
    }

    /// The other card's half of a duplicate link goes with this one. A
    /// card deleted since has none left to undo.
    fn undo_twin(&mut self, by: &str, id: &str, d: &Did) -> Result<()> {
        let twin = did_id(JOB_DUPLICATE, &d.key, id);
        let found = self
            .tasks
            .get(&d.key)
            .and_then(|t| t.did.iter().find(|x| x.id == twin && !x.undone))
            .cloned();
        match found {
            Some(other) => self.write_undo(by, &d.key, &other),
            None => Ok(()),
        }
    }

    /// Links cards `a` and `b` as looking like the same work: one line on
    /// each, naming the other. Advisory: it writes only the two records,
    /// so neither card is closed, held or delayed. False when the pair
    /// can't be linked (any more), or was linked or undone before.
    pub(crate) fn link_duplicate(&mut self, a: &str, b: &str) -> Result<bool> {
        if self.on_board {
            return Ok(false);
        }
        let (Some(ta), Some(tb)) = (self.tasks.get(a).cloned(), self.tasks.get(b).cloned()) else {
            return Ok(false);
        };
        if !may_link_duplicate(&ta, &tb) {
            return Ok(false);
        }
        self.write_link(&ta, &tb)?;
        self.write_link(&tb, &ta)?;
        Ok(true)
    }

    fn write_link(&mut self, t: &Task, other: &Task) -> Result<()> {
        let at = self.stamp();
        self.write_did(DidRow {
            id: did_id(JOB_DUPLICATE, &t.id, &other.id),
            task_id: t.id.clone(),
            job: JOB_DUPLICATE.into(),
            key: other.id.clone(),
            text: format!(
                "Irori linked this with {} {}: they look like the same work",
                other.short_ref(),
                super::util::cut(&other.title, LINK_TITLE_MAX)
            ),
            by: SWEEPER.into(),
            at,
            ..DidRow::default()
        })
    }

    /// Closes Irori's merged question and leaves the card as open as it
    /// was, never offered: closing alone would put a card nobody holds
    /// back to submitted, and start an agent on finished work. In review
    /// goes back to in review; anything else is held on the person's word,
    /// raised before the close so no device sees the card free. A holder
    /// who joined since keeps the ordinary way back.
    fn take_back_merged(&mut self, by: &str, t: &Task, bid: &str) -> Result<()> {
        const ANSWER: &str = "the person undid Irori's question";
        let bids = [bid.to_string()];
        if !t.holders.is_empty() || !t.co_holders.is_empty() {
            return self.close_items(by, &t.id, &bids, ANSWER);
        }
        if self.review_before_block.contains(&t.id) && count_open(t) == 1 {
            return self.close_items_to(by, &t.id, &bids, ANSWER, Some(IN_REVIEW));
        }
        let item = self.park_item(MERGED_WHY, &Back::Word);
        self.write_park(by, &t.id, &t.note, &item)?;
        self.close_items(by, &t.id, &bids, ANSWER)
    }

    fn undo_lease_notice(&mut self, by: &str, did: &str) -> Result<Option<Task>> {
        let Some(d) = self.loose_did.iter().find(|d| d.id == did).cloned() else {
            return Err(not_found(format!("no notice {did}")));
        };
        if !d.undone {
            self.write_undo(by, "", &d)?;
        }
        Ok(None)
    }

    fn write_undo(&mut self, by: &str, id: &str, d: &Did) -> Result<()> {
        let at = self.stamp();
        self.write(
            by,
            kinds::ASSISTANT_DID,
            &DidRow {
                id: d.id.clone(),
                task_id: id.into(),
                undo: true,
                by: by.into(),
                at,
                ..DidRow::default()
            },
        )
    }
}

/// The most of the other card's title a link's line shows.
const LINK_TITLE_MAX: usize = 60;

/// Whether card t can take part in a duplicate link at all: open, the
/// person's own board's, and not a thread an agent opened or one about a
/// shared board.
pub fn may_duplicate(t: &Task) -> bool {
    !t.shared
        && !t.withdrawn
        && !is_terminal(&t.state)
        && !is_terminal(effective_state(t))
        && !t
            .ask
            .as_ref()
            .is_some_and(|a| !a.opener.is_empty() || !a.board.is_empty())
}

/// Whether a and b are already tied by the Board: one is the other's
/// parent, subtask or sibling, they were made from one thread or one from
/// the other, or one comes after the other. Cards like that look alike
/// and aren't duplicates.
fn already_related(a: &Task, b: &Task) -> bool {
    let tied = |x: &Task, y: &Task| {
        x.parent == y.id
            || x.subtasks.contains(&y.id)
            || x.from == y.id
            || x.made.contains(&y.id)
            || x.after_cards().contains(&y.id)
    };
    tied(a, b)
        || tied(b, a)
        || (!a.parent.is_empty() && a.parent == b.parent)
        || (!a.from.is_empty() && a.from == b.from)
}

/// Whether a and b hold a duplicate record naming each other, in force or
/// undone: an undone one is what stops Irori linking them again.
fn linked_before(a: &Task, b: &Task) -> bool {
    let names = |x: &Task, y: &Task| {
        x.did
            .iter()
            .any(|d| d.job == JOB_DUPLICATE && d.key == y.id)
    };
    names(a, b) || names(b, a)
}

/// Whether Irori may link a and b as duplicates: two different open cards
/// of the person's own in the same repo, not already tied, never linked.
pub fn may_link_duplicate(a: &Task, b: &Task) -> bool {
    a.id != b.id
        && may_duplicate(a)
        && may_duplicate(b)
        && ledger::same_repo_key(&a.repo_key, &b.repo_key)
        && !already_related(a, b)
        && !linked_before(a, b)
}

/// A lease notice's key: one acquisition by one holder of one resource.
fn lease_key(l: &Lease) -> String {
    format!("{}:{}:{}", l.resource, l.holder, l.acquired_at.unix_nano())
}

fn gocompat_cut(s: &str) -> String {
    super::util::cut(s, MAX_OBJECTIVE)
}
