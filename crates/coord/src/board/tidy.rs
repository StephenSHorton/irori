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

use super::cards::clean_options;
use super::error::{Result, bad_state, not_allowed, not_found};
use super::fold::Fold;
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

/// The reason on the `TASK_CHANGED` a PR's new state makes, so a listener
/// can tell it from any other change to the card.
pub const REASON_PR_STATE: &str = "pr_state";

/// How long a device's lease holder has been away before Irori tells the
/// person, on top of the lease being past its `until`.
pub const LEASE_OFFLINE_AFTER: Duration = Duration::from_secs(30 * 60);

const MERGED_QUESTION: &str = "Its PRs are all merged and nobody is on it. Is this done?";
const HANDBACK_WHY: &str = "Irori handed it back and you asked not to offer it again";

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
            merge_did(self.did_early.entry(p.task_id.clone()).or_default(), &p, ev.hlc.0);
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
        let name = self.named(who);
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
        let name = self.named(who);
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
            "Merged: {prs}. Nobody is on it, so nothing closes it unless you do. Mark it done, or Undo on the thread leaves it open and stops this asking."
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
            .filter(|l| {
                !self.lease_local(l)
                    && gotime::after(now, &l.lease.until)
                    && self.live(l, now)
                    && {
                        let dev = self.lease_dev(l);
                        dev != self.dev && self.away_for(dev, LEASE_OFFLINE_AFTER)
                    }
            })
            .map(|l| l.lease.clone())
            .collect();
        for l in found {
            let key = format!("{}:{}:{}", l.resource, l.holder, l.acquired_at.unix_nano());
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

    /// The lease notices still in force: not undone, the lease as it was
    /// when told about.
    pub(crate) fn lease_notices(&self) -> Vec<Did> {
        let now = self.stamp();
        self.loose_did
            .iter()
            .filter(|d| {
                !d.undone
                    && d.job == JOB_LEASE
                    && self.held.values().any(|l| {
                        l.lease.resource == d.resource
                            && l.lease.holder == d.holder
                            && self.live(l, &now)
                    })
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
                self.close_items(
                    by,
                    id,
                    std::slice::from_ref(&d.blocker_id),
                    "the person undid Irori's question",
                )?;
            }
            _ => return Err(bad_state("this can't be undone")),
        }
        self.write_undo(by, id, &d)?;
        Ok(Some(self.tasks[id].clone()))
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

fn gocompat_cut(s: &str) -> String {
    super::util::cut(s, MAX_OBJECTIVE)
}
