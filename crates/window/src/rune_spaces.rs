//! The Spaces pages' commands that lived in the Tauri command layer
//! (main:crates/window/src/commands.rs) and had logic of their own, re-homed
//! for the Rune window (card #674): the invite flow on [`crate::seams`], and
//! a member change that answers the board's settings as they are after it.
//!
//! `boards_invite`, `boards_join_invite` and `boards_confirm_invite_words`
//! reach the network: call them off the UI thread.

use crate::Window;
use crate::boards::BoardSettings;
use crate::invites::InviteStatus;

impl Window {
    /// BoardsService.Invite: opens a one-hour invite window on a board the
    /// person owns; the status carries the `irori-invite:` link to send.
    pub fn boards_invite(&self, board_id: &str) -> Result<InviteStatus, String> {
        self.seams.invites.invite(board_id)
    }

    /// BoardsService.JoinInvite: takes a pasted `irori-invite:` link.
    pub fn boards_join_invite(&self, link: &str) -> Result<InviteStatus, String> {
        self.seams.invites.join(link)
    }

    /// BoardsService.ConfirmInviteWords: the inviter typing the five words
    /// the invitee reads out.
    pub fn boards_confirm_invite_words(&self, words: &str) -> Result<InviteStatus, String> {
        self.seams.invites.confirm_words(words)
    }

    /// BoardsService.InviteStatus: the open (or last) invite.
    pub fn boards_invite_status(&self) -> InviteStatus {
        self.seams.invites.status()
    }

    /// BoardsService.CancelInvite.
    pub fn boards_cancel_invite(&self) {
        self.seams.invites.cancel();
    }

    /// RemoveMember, then the board's settings as they are now.
    pub fn boards_remove_member_settings(
        &self,
        board_id: &str,
        person_id: &str,
    ) -> Result<BoardSettings, String> {
        self.boards_remove_member(board_id, person_id)?;
        self.boards_settings(board_id)
    }

    /// SetRole (`owner` or `member`), then the board's settings as they are
    /// now.
    pub fn boards_set_role_settings(
        &self,
        board_id: &str,
        person_id: &str,
        role: &str,
    ) -> Result<BoardSettings, String> {
        self.boards_set_role(board_id, person_id, role)?;
        self.boards_settings(board_id)
    }
}
