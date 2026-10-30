//! `irori-invite`: inviting another person to a board (shared boards stage
//! 2; design v0.5-shared-boards §4.1, decisions B4 and 32; architecture
//! §3.3 "Invites as tickets").
//!
//! An owner opens an invite ([`Host`]): a one-use code, the invite room it
//! derives, and a link ([`Ticket`], `irori-invite:<base32>`) to paste to the
//! other person. They paste it ([`Guest`]); their device dials the
//! inviter's on `irori/invite/1` (the net layer, `irori_net::invite`,
//! card #43), both prove the code and their person keys, and the invitee's
//! screen shows five words and both person fingerprints, read out on a
//! call. The inviter types the words; the inviter's device writes
//! `board.member.added` and sends the board ([`bundle`]); the invitee's
//! opens it, writes its own `member.*` rows, and keeps the board secret.
//! The invitee's other devices get that secret over the private mesh
//! (`mesh::boardmesh::BoardSecretFrame`).
//!
//! | Module | What |
//! |---|---|
//! | [`ticket`] | the link |
//! | [`code`] | the invite room, the info, proofs, person proofs, words |
//! | [`bundle`] | the Core's boards as the invite needs them, the bundle, admit and join |
//! | [`flow`] | the conversation, [`Host`] and [`Guest`] |
//!
//! Inviting is window-only, as adding a device is: the loopback API should
//! reach only the joining side.

pub mod bundle;
pub mod code;
pub mod flow;
pub mod ticket;

pub use bundle::{Bundle, InviteBoards};
pub use flow::{Failed, Guest, Host, INVITE_TTL, Status, Timing};
pub use ticket::Ticket;
