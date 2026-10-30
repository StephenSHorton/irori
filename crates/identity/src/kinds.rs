//! Row kinds and actor prefixes this crate folds (I:internal/store/kinds.go:22-65,
//! board.go:117-118). The store's registry owns the full list; these are
//! the names identity matches on.

pub const PERSON_DEVICE_ADDED: &str = "person.device.added";
pub const PERSON_DEVICE_REVOKED: &str = "person.device.revoked";
pub const PERSON_KEY_ROTATED: &str = "person.key.rotated";
pub const PERSON_PROFILE: &str = "person.profile";

pub const BOARD_CREATED: &str = "board.created";
pub const BOARD_RENAMED: &str = "board.renamed";
pub const BOARD_MEMBER_ADDED: &str = "board.member.added";
pub const BOARD_MEMBER_ROLE: &str = "board.member.role";
pub const BOARD_MEMBER_REMOVED: &str = "board.member.removed";
pub const BOARD_MEMBER_LEFT: &str = "board.member.left";
pub const BOARD_SERVER: &str = "board.server";

pub const MEMBER_PROFILE: &str = "member.profile";
pub const MEMBER_DEVICE_ADDED: &str = "member.device.added";
pub const MEMBER_DEVICE_REVOKED: &str = "member.device.revoked";
pub const MEMBER_KEY_ROTATED: &str = "member.key.rotated";

pub const AGENT_CARD: &str = "agent.card";

/// `store.PersonActorPrefix`.
pub const PERSON_ACTOR_PREFIX: &str = "person:";
/// `store.AgentActorPrefix`.
pub const AGENT_ACTOR_PREFIX: &str = "agent:";

/// `store.IsPersonKind`: the private log's identity rows.
pub fn is_person_kind(kind: &str) -> bool {
    kind.starts_with("person.")
}

/// `identity.IsMembershipKind`: the board.* rows the membership fold reads
/// (board.server is the transport's).
pub fn is_membership_kind(kind: &str) -> bool {
    matches!(
        kind,
        BOARD_CREATED
            | BOARD_RENAMED
            | BOARD_MEMBER_ADDED
            | BOARD_MEMBER_ROLE
            | BOARD_MEMBER_REMOVED
            | BOARD_MEMBER_LEFT
    )
}
