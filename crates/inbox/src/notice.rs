//! Routing a Board notice through the inbox, with watch-digest dedupe
//! (I:internal/tools/notice.go:32-75).
//!
//! The digest text merge is coord's (`coord.MergeWatch`); it is passed in
//! as a function so the inbox doesn't depend on coord. Merge: call
//! `route_notice(ib, from, &n, coord::merge_watch)` from the tools crate,
//! with [`Notice`] built from `coord::Notice`.

use crate::{Address, Delivery, Error, FileRef, Inbox, SendOptions, Sender, addr, mode, st};

/// The kind of a watch digest (`coord.NoticeWatched`).
pub const NOTICE_WATCHED: &str = "watched";

/// The skip note on a digest a newer one replaced.
pub const MERGED_NOTE: &str = "merged into a newer update on the card";

/// What the inbox needs of a `coord.Notice`.
#[derive(Debug, Clone, Default)]
pub struct Notice {
    pub to: Vec<String>,
    pub kind: String,
    pub text: String,
    pub ref_: String,
    /// Wake the recipients instead of telling them at their next turn.
    pub wake: bool,
    pub attachments: Vec<FileRef>,
}

/// What became of one recipient.
#[derive(Debug, Clone)]
pub struct NoticeResult {
    pub to: String,
    /// Set when `err` is `None`.
    pub delivery: Delivery,
    pub err: Option<Error>,
}

/// Sends n to each of its recipients (not empty ones, not the sender): mode
/// next_turn unless n asked to wake, the ref and notice flag along, the
/// notice kind as the reason. A watch digest still waiting unread for the
/// recipient's next turn (same card, same sender) is merged into the new one
/// and skipped.
pub fn route_notice(
    ib: &Inbox,
    from: &Sender,
    n: &Notice,
    merge: impl Fn(&str, &str) -> Option<String>,
) -> Vec<NoticeResult> {
    if n.to.is_empty() {
        return vec![];
    }
    let md = if n.wake { mode::WAKE } else { mode::NEXT_TURN };
    let mut out = Vec::with_capacity(n.to.len());
    for to in &n.to {
        if to.is_empty() || *to == from.key {
            continue;
        }
        let (mut text, mut earlier) = (n.text.clone(), String::new());
        if n.kind == NOTICE_WATCHED && md == mode::NEXT_TURN {
            (text, earlier) = merge_watched(ib, from, to, n, &merge);
        }
        let o = SendOptions {
            mode: md.into(),
            ref_: n.ref_.clone(),
            notice: true,
            reason: n.kind.clone(),
            attachments: n.attachments.clone(),
        };
        let res = ib.send_with(from, &Address::new(addr::AGENT, to), &text, o);
        let (delivery, mut err) = match res {
            Ok((_, d)) => (d, None),
            Err(e) => (Delivery::default(), Some(e)),
        };
        if err.is_none() && !earlier.is_empty() {
            // The new digest carries the earlier one's changes.
            err = ib.skip(to, &earlier, MERGED_NOTE).err();
        }
        out.push(NoticeResult {
            to: to.clone(),
            delivery,
            err,
        });
    }
    out
}

/// The text to send and the waiting delivery it replaces (`""` for none).
fn merge_watched(
    ib: &Inbox,
    from: &Sender,
    to: &str,
    n: &Notice,
    merge: &impl Fn(&str, &str) -> Option<String>,
) -> (String, String) {
    for d in ib.for_agent(to, false, 0) {
        let m = &d.message;
        if d.state != st::QUEUED
            || d.mode != mode::NEXT_TURN
            || !m.notice
            || m.ref_ != n.ref_
            || m.from.key != from.key
        {
            continue;
        }
        if let Some(text) = merge(&m.text, &n.text) {
            return (text, d.id.clone());
        }
    }
    (n.text.clone(), String::new())
}

/// Joins the failures (None when every recipient was told).
pub fn notice_err(rs: &[NoticeResult]) -> Option<String> {
    let errs: Vec<String> = rs
        .iter()
        .filter_map(|r| r.err.as_ref().map(|e| format!("notice to {}: {e}", r.to)))
        .collect();
    if errs.is_empty() {
        None
    } else {
        Some(errs.join("\n"))
    }
}
