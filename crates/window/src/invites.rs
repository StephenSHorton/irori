//! Invites (design v0.5-shared-boards.md §4.1, §8; architecture.md 3.3
//! "Invites as tickets"): Boards → a board → Invite opens a one-hour
//! invite window and shows the `irori-invite:` link to paste; Boards →
//! Join with an invite takes that link. The invitee reads five words aloud
//! and the inviter types them; both read out each other's person
//! fingerprints (B4). The flow is #44's (`irori-invite`); the window
//! drives it through [`crate::seams::Invites`].

use serde::Serialize;
use specta::Type;

/// The boards:invite event: every change of the open invite.
pub const EVENT_INVITE_CHANGED: &str = "boards:invite";

pub const INVITE_WAITING: &str = "waiting";
pub const INVITE_WORDS: &str = "words";
pub const INVITE_JOINING: &str = "joining";
pub const INVITE_DONE: &str = "done";
pub const INVITE_FAILED: &str = "failed";

/// What either side's screen shows (the invite flow's `Status`, with the
/// side, the link and its expiry). `state` "" is no invite open.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InviteStatus {
    /// "host" (inviting), "guest" (joining) or "".
    pub role: String,
    /// waiting, words, joining, done, failed, or "".
    pub state: String,
    pub board_id: String,
    pub board_name: String,
    /// Host: the `irori-invite:` link to send.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub link: String,
    /// Host: when the invite window closes (RFC 3339).
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub expires_at: String,
    /// Guest: the five words to read out.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[specta(optional)]
    pub words: Vec<String>,
    pub peer_name: String,
    pub peer_fingerprint: String,
    pub my_fingerprint: String,
    /// Wrong words typed so far (two end the invite).
    pub misses: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub error: String,
}

impl InviteStatus {
    /// An invite still in progress.
    pub fn open(&self) -> bool {
        matches!(
            self.state.as_str(),
            INVITE_WAITING | INVITE_WORDS | INVITE_JOINING
        )
    }
}

/// The text while no invite flow is wired into this build.
pub const NO_INVITES: &str = "inviting someone to a board isn't in this build yet";

/// This build's default: no invite flow.
pub struct NoInvites;

impl crate::seams::Invites for NoInvites {
    fn invite(&self, _board_id: &str) -> Result<InviteStatus, String> {
        Err(NO_INVITES.into())
    }
    fn join(&self, _link: &str) -> Result<InviteStatus, String> {
        Err(NO_INVITES.into())
    }
    fn confirm_words(&self, _typed: &str) -> Result<InviteStatus, String> {
        Err(NO_INVITES.into())
    }
    fn status(&self) -> InviteStatus {
        InviteStatus::default()
    }
    fn cancel(&self) {}
}
