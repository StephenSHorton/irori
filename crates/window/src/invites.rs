//! Invites (design v0.5-shared-boards.md §4.1, §8; architecture.md 3.3
//! "Invites as tickets"): Boards → a board → Invite opens a one-hour
//! invite window and shows the `irori-invite:` link to paste; Boards →
//! Join with an invite takes that link. The invitee reads five words aloud
//! and the inviter types them; both read out each other's person
//! fingerprints (B4). The flow is #44's (`irori-invite`, driven by
//! `Core::invite` and the rest); the window reaches it through
//! [`crate::seams::Invites`], [`CoreInvites`] in this build.

use std::sync::Arc;

use irori_core::Core;
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

/// The window's shape of the Core's invite status (the same JSON).
pub fn invite_status(s: irori_core::InviteStatus) -> InviteStatus {
    InviteStatus {
        role: s.role,
        state: s.status.state,
        board_id: s.status.board_id,
        board_name: s.status.board_name,
        link: s.link,
        expires_at: s.expires_at,
        words: s.status.words,
        peer_name: s.status.peer_name,
        peer_fingerprint: s.status.peer_fingerprint,
        my_fingerprint: s.status.my_fingerprint,
        misses: s.status.misses.into(),
        error: s.status.error,
    }
}

/// The seam on the Core's invite driver (#44, `Core::invite` and the rest).
pub struct CoreInvites {
    core: Arc<Core>,
}

impl CoreInvites {
    pub fn new(core: Arc<Core>) -> CoreInvites {
        CoreInvites { core }
    }

    fn run<T>(&self, f: impl std::future::Future<Output = T>) -> T {
        self.core.pairing_runtime().block_on(f)
    }
}

impl crate::seams::Invites for CoreInvites {
    fn invite(&self, board_id: &str) -> Result<InviteStatus, String> {
        self.run(self.core.invite(board_id)).map(invite_status)
    }
    fn join(&self, link: &str) -> Result<InviteStatus, String> {
        self.run(self.core.join_invite(link.trim()))
            .map(invite_status)
    }
    fn confirm_words(&self, typed: &str) -> Result<InviteStatus, String> {
        self.core.confirm_invite_words(typed).map(invite_status)
    }
    fn status(&self) -> InviteStatus {
        invite_status(self.core.invite_status())
    }
    fn cancel(&self) {
        self.core.cancel_invite();
    }
    fn watch(&self, f: Option<Arc<dyn Fn(InviteStatus) + Send + Sync>>) {
        self.core.watch_invites(
            f.map(|f| -> irori_core::InviteWatchFn { Arc::new(move |s| f(invite_status(s))) }),
        );
    }
}
