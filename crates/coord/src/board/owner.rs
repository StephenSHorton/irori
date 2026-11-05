//! A shared card's owner changing hands (card #432). A card's owner is
//! the person who decides Done and answers its questions (#429): its
//! creator's person until a transfer. The owner offers the card to another
//! member, or a member asks the owner for it; either raises an
//! Accept/Decline item for the other side, and the card moves only when
//! that side accepts. Every member's fold applies the same rule to the
//! `task.owner` rows, so they agree on who owns it.

use store::{Event, kinds};

use super::error::{Result, bad_state, invalid, not_allowed};
use super::fold::Fold;
use super::rows::*;
use super::types::*;
use crate::names::trim_space;

/// The two answers on an ownership item.
pub const OWNER_ACCEPT: &str = "Accept";
pub const OWNER_DECLINE: &str = "Decline";

const OP_OFFER: &str = "offer";
const OP_REQUEST: &str = "request";
const OP_ACCEPT: &str = "accept";
const OP_DECLINE: &str = "decline";

impl Fold {
    /// The person id of the card's owner: the last accepted transfer, else
    /// its creator's person ("" off a board or when unknown).
    pub(crate) fn owner_person(&self, t: &Task) -> String {
        if !t.owner.is_empty() {
            return t.owner.clone();
        }
        self.person_of(&t.created_by)
    }

    /// The card's transfer still waiting on an answer: its item is open.
    /// One closed some other way (an agent's unblock of every item) is
    /// over, and the card keeps its owner.
    pub(crate) fn pending_transfer(&self, t: &Task) -> Option<OwnerTransfer> {
        t.owner_transfer
            .clone()
            .filter(|tr| t.blockers.iter().any(|b| b.id == tr.blocker_id && b.open()))
    }

    /// The person whose device wrote a row, on a board.
    pub(crate) fn writer_person(&self, device: &str) -> String {
        match (self.on_board, &self.dev_person) {
            (true, Some(dp)) => dp(device),
            _ => String::new(),
        }
    }

    /// Who answers a pending transfer, and who asked for it.
    fn transfer_sides(&self, t: &Task, tr: &OwnerTransfer) -> (String, String) {
        let owner = self.owner_person(t);
        if tr.op == OP_OFFER {
            (tr.person.clone(), owner)
        } else {
            (owner, tr.person.clone())
        }
    }

    /// Folds a `task.owner` row. It counts only from a device of the right
    /// person: an offer from the owner's, a request from the asker's, an
    /// accept from the other side's, a decline from either side's.
    pub(crate) fn fold_owner(&mut self, ev: &Event) {
        let Some(p) = row::<TaskOwnerRow>(&ev.payload) else {
            return;
        };
        if !self.on_board || p.person.is_empty() {
            return;
        }
        let Some(t) = self.tasks.get(&p.task_id) else {
            return;
        };
        let w = self.writer_person(&ev.device_id);
        let owner = self.owner_person(t);
        let ok = !w.is_empty()
            && match p.op.as_str() {
                OP_OFFER => w == owner && p.person != owner,
                OP_REQUEST => w == p.person && p.person != owner,
                OP_ACCEPT | OP_DECLINE => match &self.pending_transfer(t) {
                    Some(tr) if tr.person == p.person => {
                        let (decides, asked) = self.transfer_sides(t, tr);
                        w == decides || (p.op == OP_DECLINE && w == asked)
                    }
                    _ => false,
                },
                _ => false,
            };
        if !ok {
            self.ignore(&ev.device_id);
            return;
        }
        let t = self.tasks.get_mut(&p.task_id).expect("checked");
        match p.op.as_str() {
            OP_OFFER | OP_REQUEST => {
                t.owner_transfer = Some(OwnerTransfer {
                    op: p.op.clone(),
                    person: p.person.clone(),
                    blocker_id: p.blocker_id.clone(),
                });
            }
            OP_ACCEPT => {
                t.owner = p.person.clone();
                t.owner_transfer = None;
            }
            _ => t.owner_transfer = None,
        }
        self.note(Change {
            kind: TASK_CHANGED.into(),
            task_id: p.task_id,
            ..Change::default()
        });
    }

    /// The person offers the card to `to` (a person id) when they own it,
    /// or asks its owner for it when they don't (`to` is then ignored).
    /// Raises an Accept/Decline item for the other side.
    pub(crate) fn transfer_owner(
        &mut self,
        by: &str,
        id: &str,
        to: &str,
    ) -> Result<(Task, Blocker)> {
        let by = trim_space(by).to_string();
        if !self.on_board {
            return Err(invalid("only a card on a shared board changes owner"));
        }
        if !self.is_owner(&by) {
            return Err(not_allowed("only a person offers a card or asks for it"));
        }
        let t = self.open_task(id)?.clone();
        if self.pending_transfer(&t).is_some() {
            return Err(bad_state(format!(
                "{} already has an ownership offer or request waiting",
                t.short_ref()
            )));
        }
        let me = self.person_of(&by);
        let owner = self.owner_person(&t);
        if me.is_empty() || owner.is_empty() {
            return Err(invalid("this card's owner isn't known on this device yet"));
        }
        let to = trim_space(to);
        let to = to.strip_prefix(kinds::PERSON_ACTOR_PREFIX).unwrap_or(to);
        let (op, person, decides, q, cx) = if me == owner {
            if to.is_empty() || to == owner {
                return Err(invalid("offer the card to another member of the space"));
            }
            (
                OP_OFFER,
                to.to_string(),
                to.to_string(),
                "Take over this card?",
                "Its owner offers it to you. As its owner you decide Done and answer the questions its agents raise.",
            )
        } else {
            (
                OP_REQUEST,
                me.clone(),
                owner,
                "Hand this card over?",
                "Another member asks to own it. They would decide Done and answer the questions its agents raise; you would no longer.",
            )
        };
        let now = self.stamp();
        let opt = |answer: &str, recommended: bool| BlockerOption {
            answer: answer.into(),
            by: by.clone(),
            at: now,
            recommended,
            ..BlockerOption::default()
        };
        let (_, b) = self.raise_locked(
            &by,
            &t,
            q,
            q,
            cx,
            vec![opt(OWNER_ACCEPT, false), opt(OWNER_DECLINE, false)],
            Some(BlockedBy {
                kind: BLOCKED_ON_PERSON.into(),
                reference: format!("{}{decides}", kinds::PERSON_ACTOR_PREFIX),
            }),
            false,
            now,
            Vec::new(),
            Vec::new(),
            false,
            true,
        )?;
        self.write(
            &by,
            kinds::TASK_OWNER,
            &TaskOwnerRow {
                task_id: t.id.clone(),
                op: op.into(),
                person,
                blocker_id: b.id.clone(),
                by: by.clone(),
                at: now,
            },
        )?;
        Ok((self.tasks[&t.id].clone(), b))
    }

    /// The other side's Accept or Decline on an ownership item (or the
    /// asker taking it back with Decline). Writes the owner row, then
    /// resolves the item with that answer. `Ok(None)` when bid isn't the
    /// pending transfer's item.
    pub(crate) fn answer_owner(
        &mut self,
        by: &str,
        id: &str,
        bid: &str,
        accept: bool,
    ) -> Result<Option<String>> {
        let t = self.open_task(id)?.clone();
        let Some(tr) = self.pending_transfer(&t).filter(|tr| tr.blocker_id == bid) else {
            return Ok(None);
        };
        if !self.is_owner(by) {
            return Err(not_allowed(
                "only the person it asks accepts or declines an ownership change",
            ));
        }
        let me = self.person_of(by);
        let (decides, asked) = self.transfer_sides(&t, &tr);
        if me != decides && !(me == asked && !accept) {
            return Err(not_allowed(
                "only the person it asks accepts or declines an ownership change",
            ));
        }
        let at = self.stamp();
        self.write(
            by,
            kinds::TASK_OWNER,
            &TaskOwnerRow {
                task_id: t.id.clone(),
                op: if accept { OP_ACCEPT } else { OP_DECLINE }.into(),
                person: tr.person.clone(),
                blocker_id: bid.into(),
                by: by.into(),
                at,
            },
        )?;
        Ok(Some(tr.person))
    }
}
