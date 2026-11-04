//! Board notices through the inbox, and messages on a card's thread
//! (I:internal/tools/notice.go, cardthread.go).
//!
//! coord never depends on inbox: it returns [`coord::Notice`]s and the
//! caller routes them with [`route_notice`]. A message about a card (its
//! Ref is `task:<id>`, or `comment:<id>` on a card) is also an entry on that
//! card's thread; the tools record it after the inbox call returns, never
//! under the inbox's lock.

use coord::{Coord, MessageEntry, Notice};
use inbox::notice::NoticeResult;
use inbox::{Delivery, Error as InboxError, ErrorKind as InboxErrorKind, Inbox, Message, Sender};

use crate::format::{one_line, trim};
use crate::{Inner, Names};

/// How [`record_on_card`] writes the entry (Go `Record`).
#[derive(Debug, Clone, Default)]
pub struct Record {
    /// The recipient ended.
    pub gone: bool,
    /// The first delivery wasn't let through (no text).
    pub held: bool,
    /// The recipient's device let it through: the text for a held entry.
    pub fill: bool,
    /// `"irori"` writes the row as Irori itself, else as the sender.
    pub writer: String,
}

/// Why a message couldn't go on a card.
#[derive(Debug)]
pub enum RecordError {
    /// The Ref names no card (Go `ErrNoCard`).
    NoCard,
    Coord(coord::Error),
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordError::NoCard => f.write_str("no card"),
            RecordError::Coord(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for RecordError {}

/// `d`'s message goes on its card without its text: only when this device
/// holds it (the person's block list, allowlist or Pause all). A delivery
/// another device decides shows its text at once (card #94): since
/// decision 37 nothing holds by default, and the text otherwise stayed off
/// the card until the recipient's device reported back.
pub fn held_on_card(ib: &Inbox, d: &Delivery) -> bool {
    d.state == inbox::st::HELD && ib.owned(&d.id)
}

/// The task a message Ref names: `task:<id>`, or `comment:<id>` on a card.
pub fn card_for(co: &Coord, r: &str) -> Option<String> {
    let (kind, id) = r.split_once(':').unwrap_or((r, ""));
    let id = match kind {
        // A Ref holds the card's id, never a short ref.
        "task" => trim(id).to_string(),
        "comment" => co.get_comment(trim(id))?.task_id,
        _ => return None,
    };
    (!id.is_empty()).then_some(id)
}

/// Puts `m` (to its recipient `to`) on the card its Ref names, attachments
/// as a count. Idempotent: the entry's id is the message id.
pub fn record_on_card(
    co: &Coord,
    m: &Message,
    to: &str,
    r: &Record,
) -> Result<String, RecordError> {
    let tid = card_for(co, &m.ref_).ok_or(RecordError::NoCard)?;
    let cm = co
        .record_message(MessageEntry {
            task_id: tid,
            msg_id: m.id.clone(),
            from: m.from.key.clone(),
            to: to.into(),
            text: m.text.clone(),
            files: m.attachments.len() as i64,
            held: r.held,
            gone: r.gone,
            fill: r.fill,
            writer: r.writer.clone(),
            mentions: m
                .mentions
                .iter()
                .map(|x| coord::Mention {
                    key: x.key.clone(),
                    name: x.name.clone(),
                    gone: x.gone,
                })
                .collect(),
        })
        .map_err(RecordError::Coord)?;
    Ok(cm.task_id)
}

/// The inbox's `on_recipient_gone` over `co`: the message goes on its card
/// marked gone, written by Irori. A ref naming no card here is
/// `ErrNoCard`, so the inbox sends it to the person instead.
pub fn record_gone(co: &Coord, m: &Message, to: &str) -> inbox::Result<String> {
    let r = Record {
        gone: true,
        writer: "irori".into(),
        ..Record::default()
    };
    match record_on_card(co, m, to, &r) {
        Ok(t) => Ok(t),
        Err(RecordError::NoCard) => Err(InboxError::sentinel(InboxErrorKind::NoCard)),
        Err(RecordError::Coord(e)) if e.is(coord::ErrorKind::NotFound) => {
            Err(InboxError::sentinel(InboxErrorKind::NoCard))
        }
        Err(RecordError::Coord(e)) => Err(InboxError::other(e.to_string())),
    }
}

/// The shared board's card a row bridged from that board is closed on
/// once the agent it went to has gone (card #478), for the rows that go to
/// every holder, watcher and assignee of the card: a comment, a blocker
/// raised, answered or closed. Each is already on the shared card, where
/// the person reads it; handing every agent's copy to the person put them
/// in Talk as DMs from the board ("raised a blocker on ..." ten times
/// over). `None` for anything else, which still reaches the person: work
/// assigned, a review or release request, a handed card or an ask went to
/// that one agent and nobody else hears it.
pub fn closed_on_board(m: &Message) -> Option<String> {
    if m.from.kind != inbox::BOARD_KIND || m.board.board_id.is_empty() {
        return None;
    }
    if !matches!(
        m.board.kind.as_str(),
        inbox::row_kind::COMMENT | inbox::row_kind::BLOCKER
    ) {
        return None;
    }
    let id = m.ref_.strip_prefix("task:").map(trim).unwrap_or_default();
    (!id.is_empty()).then(|| id.to_string())
}

/// Spawned agents part 8 (card #452, spawned-agents.md §4 "Reaching an
/// agent that has gone"): `m` went to `to`, a spawned agent that has
/// finished, and names no card of its own. It goes on the card `to` was
/// spawned for, marked gone and written by Irori, and the notice returned
/// wakes that card's holders with it ("for <name>, who has finished").
/// `None` when `to` was never spawned for a card (today's path: the
/// person gets it). Nothing here ever asks for an agent: a message is no
/// authorization (§7), so a card nobody holds keeps it on its thread for
/// the next agent's brief.
pub fn record_for_finished(
    co: &Coord,
    m: &Message,
    to: &str,
    name: &str,
) -> inbox::Result<Option<(String, Notice)>> {
    let Some(tid) = co.spawned_card(to) else {
        return Ok(None);
    };
    let Some(t) = co.get_task(&tid) else {
        return Ok(None);
    };
    let mut on_card = m.clone();
    on_card.ref_ = format!("task:{tid}");
    let r = Record {
        gone: true,
        writer: "irori".into(),
        ..Record::default()
    };
    match record_on_card(co, &on_card, to, &r) {
        Ok(_) => {}
        Err(RecordError::NoCard) => return Ok(None),
        Err(RecordError::Coord(e)) => return Err(InboxError::other(e.to_string())),
    }
    let who = if name.trim().is_empty() {
        to
    } else {
        name.trim()
    };
    let mut holders: Vec<String> = t.holders.clone();
    if holders.is_empty() && !t.holder.is_empty() {
        holders.push(t.holder.clone());
    }
    holders.retain(|h| h != to && !h.is_empty());
    let n = Notice {
        kind: coord::NOTICE_COMMENT.into(),
        task_id: tid.clone(),
        reference: format!("task:{tid}"),
        title: t.title.clone(),
        to: holders,
        by: inbox::SYSTEM_KEY.into(),
        text: format!(
            "A message for {who}, who has finished, landed on {}: {}",
            t.short_ref(),
            one_line(&m.text, 300)
        ),
        wake: true,
        ..Notice::default()
    };
    Ok(Some((tid, n)))
}

/// The inbox's `on_passed` over `co`: this device let `m`'s delivery to
/// `to` through its wake policy, so a held entry gets its text.
pub fn record_passed(co: &Coord, m: &Message, to: &str) -> inbox::Result<()> {
    let r = Record {
        fill: true,
        writer: "irori".into(),
        ..Record::default()
    };
    match record_on_card(co, m, to, &r) {
        Ok(_) => Ok(()),
        Err(e) => Err(InboxError::other(e.to_string())),
    }
}

/// `coord::Notice` as the inbox takes it.
pub fn inbox_notice(n: &Notice) -> inbox::notice::Notice {
    inbox::notice::Notice {
        to: n.to.clone(),
        kind: n.kind.clone(),
        text: n.text.clone(),
        ref_: n.reference.clone(),
        wake: n.wake,
        stop: n.stop,
        attachments: n
            .attachments
            .iter()
            .map(|f| inbox::FileRef {
                sha256: f.sha256.clone(),
                name: f.name.clone(),
                size: f.size,
                mime: f.mime.clone(),
                device: f.device.clone(),
            })
            .collect(),
    }
}

/// Go `RouteNotice`: each recipient its own delivery, next_turn unless the
/// notice wakes, as a notice with its ref and kind; watch digests merged.
/// The coord row is already written, so a recipient that can't be told is
/// reported, never undone.
pub fn route_notice(ib: &Inbox, from: &Sender, n: &Notice) -> Vec<NoticeResult> {
    inbox::notice::route_notice(ib, from, &inbox_notice(n), coord::merge_watch)
}

/// Go `NoticeErr`: the failures joined, `None` when everyone heard.
pub fn notice_err(rs: &[NoticeResult]) -> Option<String> {
    inbox::notice::notice_err(rs)
}

impl Inner {
    /// Routes a coord notice from the caller, its keys made private first
    /// when it came from a board's coord.
    pub(crate) fn tell(&self, from: &Sender, n: Notice) -> Vec<NoticeResult> {
        let n = match self.b.router() {
            Some(r) => r.private_notice(n),
            None => n,
        };
        route_notice(self.b.inbox(), from, &n)
    }

    /// Who heard about it and how, for a tool result.
    pub(crate) fn told(&self, rs: &[NoticeResult], n: &Names<'_>) -> String {
        if rs.is_empty() {
            return String::new();
        }
        let (mut next, mut woke, mut held, mut failed) = (vec![], vec![], vec![], vec![]);
        for r in rs {
            let who = n.who(&r.to);
            if let Some(e) = &r.err {
                failed.push(format!("{who} ({})", one_line(&e.to_string(), 160)));
            } else if r.delivery.state == inbox::st::HELD {
                held.push(who);
            } else if r.delivery.mode == inbox::mode::WAKE {
                woke.push(who);
            } else {
                next.push(who);
            }
        }
        let mut out = String::new();
        let mut add = |list: &[String], what: &str| {
            if !list.is_empty() {
                out.push_str(&format!(" {what} {}.", list.join(", ")));
            }
        };
        add(&next, "Told at their next turn:");
        add(&woke, "Woke:");
        add(
            &held,
            "Held by the wake policy until the person releases it:",
        );
        add(&failed, "Couldn't tell:");
        out
    }

    /// The line a send or reply result ends with once `m` is on the card
    /// its Ref names; "" when it names none.
    pub(crate) fn recorded(&self, m: &Message, to: &str, held: bool) -> String {
        let co = self.b.coord();
        let Some(tid) = card_for(co, &m.ref_) else {
            if m.ref_.starts_with("comment:") {
                return format!(
                    " (sent, but not recorded on {}: no card has that comment here)",
                    one_line(&m.ref_, 80)
                );
            }
            return String::new();
        };
        let r = Record {
            held,
            ..Record::default()
        };
        if let Err(e) = record_on_card(co, m, to, &r) {
            return format!(
                " (sent, but not recorded on task:{}: {})",
                one_line(&tid, 80),
                one_line(&e.to_string(), 200)
            );
        }
        if held {
            return format!(
                " It is on task:{}'s thread; its text joins it once the wake policy lets it through.",
                one_line(&tid, 80)
            );
        }
        format!(" It is on task:{}'s thread.", one_line(&tid, 80))
    }
}
