//! A shared board's members (v0.5-shared-boards §4.2-4.4): an owner
//! removes a member or changes a role, and a person leaves a board other
//! people are on.
//!
//! - **Remove** writes board.member.removed with a cut over each of the
//!   member's certified devices at what this device holds. Every member's
//!   fold refuses the same rows past it, the board secret rotates (the
//!   follow thread's rotate_secret) and their connections close. The
//!   remover's cleanup: the member's agents come off every open card
//!   (task.left `member_removed`). Their leases end in every fold at the
//!   removal already (the cut makes their devices revoked, coord's
//!   `revoked`). Bridged deliveries from them still waiting are held on
//!   every device of every member as the removal folds there
//!   ([`Services::hold_from_gone`]).
//! - **Role** writes board.member.role; the last owner can't stop being
//!   one (refused here, and ignored by the fold as a conflict).
//! - **Leave** writes board.member.left with a cut over the person's own
//!   devices. The board shows "Leaving…" until another member's device
//!   says (its board vector) that it holds the `left` row, and only then
//!   forgets the board on every device of the person (board.forgot); or
//!   the person picks Forget now.

use std::collections::BTreeMap;
use std::sync::Arc;

use coord::ErrorKind as CK;
use irori_identity::membership::{BOARD_MEMBER, BOARD_OWNER, MEMBER_LEFT};
use store::Filter;
use store::kinds::{self, PERSON_ACTOR_PREFIX};

use super::Services;
use super::boards::{BoardLog, Error, Result};
use super::boards_own::BoardForgotRow;
use crate::logf;

impl Services {
    /// This person's newest key epoch when they own board `b` (§4.2), for
    /// a row signed with it.
    fn owner_epoch(&self, b: &BoardLog, what: &str) -> Result<i64> {
        let me = self.person_id();
        if !b.verifier.membership().owner(&me) {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!("only an owner {what} on board {}", b.id),
            ));
        }
        self.newest_epoch(b)
    }

    fn newest_epoch(&self, b: &BoardLog) -> Result<i64> {
        let epoch = b.verifier.epoch(&self.person_id());
        if epoch != self.ident.current().epoch {
            return Err(Error::coord(
                CK::NotAllowed,
                "this device's person key isn't the board's newest yet",
            ));
        }
        Ok(epoch)
    }

    /// The cut over `person_id`'s certified devices in board `b` (§4.3,
    /// §4.4): the highest seq of each one's stream this device holds, 0
    /// for one with no rows here. One board write at a time per device
    /// (the store's), so a leave's own next seq is this plus one.
    fn member_cut(&self, b: &BoardLog, person_id: &str) -> Result<BTreeMap<String, i64>> {
        let held = b.store.vector()?;
        Ok(b.verifier
            .devices(person_id)
            .into_iter()
            .map(|d| {
                let seq = held.get(&d.cert.device_id).copied().unwrap_or(0);
                (d.cert.device_id, seq)
            })
            .collect())
    }

    /// Closes the connections board `board_id` no longer admits, hands a
    /// rotated secret on and pulls (a local membership write).
    pub(crate) fn recheck_board_mesh(&self, board_id: &str) {
        if let Some(m) = self.mesh.board(board_id) {
            let _g = self.pairing.handle.enter();
            m.recheck();
            m.kick();
        }
    }

    /// An owner removes member `person_id` from board `board_id` (§4.3),
    /// with the remover's cleanup.
    pub(crate) fn remove_member(
        self: &Arc<Self>,
        board_id: &str,
        person_id: &str,
        reason: &str,
    ) -> Result<()> {
        let b = self.board(board_id)?;
        let epoch = self.owner_epoch(&b, "removes people")?;
        if person_id == self.person_id() {
            return Err(Error::coord(
                CK::Invalid,
                "you can't remove yourself: leave the board instead",
            ));
        }
        if !b.verifier.membership().active(person_id) {
            return Err(Error::coord(
                CK::NotFound,
                format_args!("{person_id} isn't on board {board_id}"),
            ));
        }
        let cut = self.member_cut(&b, person_id)?;
        let row = self.ident.board_member_removed_row(
            epoch,
            person_id,
            reason.trim(),
            Some(cut),
            (self.now)(),
        )?;
        b.store.append(
            &format!("{PERSON_ACTOR_PREFIX}{}", self.person_id()),
            kinds::BOARD_MEMBER_REMOVED,
            &row,
        )?;
        match b.coord().take_off_person(&self.person_actor(), person_id) {
            Ok(n) if n > 0 => logf(format_args!(
                "board {board_id}: took {n} of {person_id}'s places off its cards"
            )),
            Ok(_) => {}
            Err(e) => logf(format_args!(
                "board {board_id}: taking {person_id}'s agents off: {e}"
            )),
        }
        self.hold_from_gone(&b);
        self.recheck_board_mesh(board_id);
        self.boards.moved();
        Ok(())
    }

    /// An owner makes member `person_id` an owner or a member (§4.2).
    pub(crate) fn set_member_role(
        &self,
        board_id: &str,
        person_id: &str,
        role: &str,
    ) -> Result<()> {
        if role != BOARD_OWNER && role != BOARD_MEMBER {
            return Err(Error::coord(
                CK::Invalid,
                format_args!("a role is {BOARD_OWNER} or {BOARD_MEMBER}"),
            ));
        }
        let b = self.board(board_id)?;
        let epoch = self.owner_epoch(&b, "changes roles")?;
        let m = b.verifier.membership();
        let Some(p) = m.person(person_id).filter(|p| m.active(&p.person_id)) else {
            return Err(Error::coord(
                CK::NotFound,
                format_args!("{person_id} isn't on board {board_id}"),
            ));
        };
        if p.role == role {
            return Ok(());
        }
        if role == BOARD_MEMBER && m.members.iter().filter(|p| m.owner(&p.person_id)).count() <= 1 {
            return Err(Error::coord(
                CK::NotAllowed,
                "the last owner can't stop being one: make someone else an owner first",
            ));
        }
        let row = self
            .ident
            .board_member_role_row(epoch, person_id, role, (self.now)())?;
        b.store.append(
            &format!("{PERSON_ACTOR_PREFIX}{}", self.person_id()),
            kinds::BOARD_MEMBER_ROLE,
            &row,
        )?;
        self.boards.moved();
        Ok(())
    }

    /// Takes the person off a board on every one of their devices (§4.4,
    /// §4.6). Alone on it, a private board.forgot does it now; with other
    /// people on it, board.member.left goes first and the board shows
    /// "Leaving…" until another member holds that row
    /// ([`Services::sweep_leaving`]).
    pub(crate) fn leave_board(self: &Arc<Self>, board_id: &str) -> Result<()> {
        let b = self.board(board_id)?;
        let me = self.person_id();
        let m = b.verifier.membership();
        let others = m
            .members
            .iter()
            .any(|p| p.person_id != me && m.active(&p.person_id));
        if !others || !m.active(&me) {
            return self.forget_everywhere(board_id);
        }
        if m.owner(&me) && m.members.iter().filter(|p| m.owner(&p.person_id)).count() <= 1 {
            return Err(Error::coord(
                CK::NotAllowed,
                "you're this board's only owner: make someone else an owner before you leave",
            ));
        }
        let epoch = self.newest_epoch(&b)?;
        // The cut takes in the `left` row itself (this device's next seq):
        // board vectors stop at a device's cut, and another member's
        // vector covering that row is what ends "Leaving…".
        let mut cut = self.member_cut(&b, &me)?;
        if let Some(c) = cut.get_mut(&self.device.id) {
            *c += 1;
        }
        let row = self
            .ident
            .board_member_left_row(epoch, Some(cut), (self.now)())?;
        b.store.append(
            &format!("{PERSON_ACTOR_PREFIX}{me}"),
            kinds::BOARD_MEMBER_LEFT,
            &row,
        )?;
        self.recheck_board_mesh(board_id);
        self.boards.moved();
        Ok(())
    }

    /// Forgets board `board_id` on every one of the person's devices now,
    /// through a private board.forgot row (§4.6): leave alone on a board,
    /// a leave another member holds, or Forget now while "Leaving…".
    pub(crate) fn forget_everywhere(self: &Arc<Self>, board_id: &str) -> Result<()> {
        self.board(board_id)?;
        self.store.append(
            "irori",
            kinds::BOARD_FORGOT,
            &BoardForgotRow {
                board_id: board_id.into(),
                at: (self.now)(),
            },
        )?;
        self.reconcile_boards();
        Ok(())
    }

    /// The person left board `b` and it is still here: "Leaving…" (§4.4).
    pub(crate) fn board_leaving(&self, b: &BoardLog) -> bool {
        b.verifier
            .membership()
            .person(&self.person_id())
            .is_some_and(|p| p.state == MEMBER_LEFT)
    }

    /// Whether a device of another member said it holds every
    /// board.member.left row this person wrote on board `b`.
    fn leave_heard(&self, b: &BoardLog) -> bool {
        let me = self.person_id();
        let (mut left, actor) = (Vec::new(), format!("{PERSON_ACTOR_PREFIX}{me}"));
        let _ = super::boards::each_row(
            &b.store,
            Filter {
                kinds: vec![kinds::BOARD_MEMBER_LEFT.into()],
                ..Default::default()
            },
            |ev| {
                if ev.actor == actor {
                    left.push((ev.device_id.clone(), ev.seq));
                }
            },
        );
        if left.is_empty() {
            return false;
        }
        let Some(m) = self.mesh.board(&b.id) else {
            return false;
        };
        m.heard().iter().any(|h| {
            !h.person_id.is_empty()
                && h.person_id != me
                && left
                    .iter()
                    .all(|(dev, seq)| h.vector.get(dev).copied().unwrap_or(0) >= *seq)
        })
    }

    /// Forgets every board the person left once another member holds the
    /// `left` row (§4.4): the Core's sweep.
    pub(crate) fn sweep_leaving(self: &Arc<Self>) {
        for b in self.boards.list() {
            if self.board_leaving(&b)
                && self.leave_heard(&b)
                && let Err(e) = self.forget_everywhere(&b.id)
            {
                logf(format_args!("board {}: forget after leaving: {e}", b.id));
            }
        }
    }

    /// Holds the bridged deliveries still waiting from every person who
    /// was removed from board `b` or left it (§4.3), on this device.
    pub(crate) fn hold_from_gone(&self, b: &BoardLog) {
        let me = self.person_id();
        for p in b.verifier.membership().members {
            if p.person_id == me || p.state == irori_identity::membership::MEMBER_ACTIVE {
                continue;
            }
            match self.inbox.hold_from_member(&b.id, &p.person_id) {
                Ok(n) if n > 0 => logf(format_args!(
                    "board {}: held {n} deliveries from {}, no longer on it",
                    b.id, p.person_id
                )),
                Ok(_) => {}
                Err(e) => logf(format_args!("board {}: hold: {e}", b.id)),
            }
        }
    }
}
