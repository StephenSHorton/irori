//! Board notices through the inbox, and the inbox's hooks into the Board
//! (I:internal/core/notice.go, board.go; I:internal/tools/notice.go,
//! cardthread.go).
//!
//! coord never depends on inbox: it returns [`coord::Notice`]s and the
//! caller routes them. The sweep, the window and the tools all route
//! through [`crate::Core::route_notice`] or [`route`], so every notice goes
//! the one way.
//!
//! The card-thread helpers ([`record_gone`], [`record_passed`],
//! [`record_on_card`]) and the routing are [`tools::notice`]'s, the one
//! port of `notice.go`/`cardthread.go`; they are re-exported here for the
//! inbox's `on_recipient_gone` / `on_passed`. Only the inbox's `Stale`
//! hook is the Core's own.

use coord::{Coord, Notice};
use inbox::notice::NoticeResult;
use inbox::{Delivery, Inbox, Sender};

pub use tools::notice::{
    Record, RecordError, card_for, held_on_card, inbox_notice, notice_err, record_gone,
    record_on_card, record_passed,
};

/// The inbox's `Stale` hook: a notice about an ask someone else took (or
/// that closed) closes as skipped before it reaches an agent. Only what was
/// said to the room goes stale.
pub fn stale_notice(co: &Coord, d: &Delivery) -> String {
    match d.message.reason.as_str() {
        coord::NOTICE_ASK | coord::NOTICE_ASK_COMMENT => co.stale_notice(&d.message.ref_, &d.to),
        _ => String::new(),
    }
}

/// Go `tools.RouteNotice` ([`tools::notice::route_notice`]): each
/// recipient its own delivery, next_turn unless the notice wakes, as a
/// notice with its ref and kind; watch digests merged.
pub fn route(ib: &Inbox, from: &Sender, n: &Notice) -> Vec<NoticeResult> {
    tools::notice::route_notice(ib, from, n)
}
