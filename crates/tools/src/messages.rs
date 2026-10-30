//! send, reply, inbox, ack, defer and skip (I:internal/tools/messages.go).

use std::fmt::Write;

use gocompat::time::GoTime;
use inbox::{Address, Delivery, SendOptions, addr, hold, mode, path, st};

use crate::args::Ty;
use crate::attachments::{attached_line, describe_inbox, to_inbox};
use crate::format::{MAX_FIELD_RUNES, MAX_LIST_TEXT, ago, in_, one_line, quoted, trim, trim_opt};
use crate::notice::{card_for, held_on_card};
use crate::{Caller, Inner, MAX_RESULT, Names, decode, err_text, map_err, untrusted};

const DEFAULT_INBOX_LIMIT: i64 = 20;
const MAX_INBOX_LIMIT: i64 = 50;

const SEND_ARGS: &[(&str, Ty)] = &[
    ("to", Ty::OptStr),
    ("repo", Ty::OptStr),
    ("text", Ty::Str),
    ("reason", Ty::Str),
    ("attach", Ty::Strs),
    ("task_id", Ty::Str),
];

pub(crate) fn send(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, SEND_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let (to, repo) = (trim_opt(&a.opt_str("to")), trim_opt(&a.opt_str("repo")));
    let address = if !to.is_empty() && !repo.is_empty() {
        return err_text("give exactly one of to (an agent) or repo (a repoKey), not both.");
    } else if is_pr_target(&to) {
        Address::new(addr::PR, &to)
    } else if !to.is_empty() {
        Address::new(addr::AGENT, &to)
    } else if !repo.is_empty() {
        Address::new(addr::REPO, &repo)
    } else {
        return err_text(
            "say who it's for: to = an agent's name or key (or a PR: pr:<owner>/<name>#<number> or its URL), or repo = a repoKey (see the agents tool).",
        );
    };
    let text = a.str("text");
    if trim(text).is_empty() {
        return err_text("give the message text.");
    }
    // A message about a card names it as its Ref, and goes on its thread
    // once sent. An unknown card sends nothing.
    let mut r = String::new();
    let task_id = a.str("task_id");
    if !trim(task_id).is_empty() {
        let id = match s.task_id(task_id) {
            Ok(id) => id,
            Err(msg) => return msg,
        };
        if s.task(&id).is_none() {
            let name = if coord::parse_short(&id).is_some() {
                id.clone() // a short ref no card has, as given
            } else {
                format!("task:{id}")
            };
            return err_text(&format!(
                "no card {}; nothing was sent. The board tool shows the cards.",
                one_line(&name, 80)
            ));
        }
        r = format!("task:{id}");
    }
    let files = match s.attach_files(c, &a.strs("attach")) {
        Ok(f) => f,
        Err(msg) => return msg,
    };
    let o = SendOptions {
        reason: trim(a.str("reason")).to_string(),
        mode: mode::WAKE.into(),
        attachments: files.iter().map(to_inbox).collect(),
        ref_: r.clone(),
        notice: false,
    };
    let ib = s.b.inbox();
    let (m, d) = match ib.send_with(&c.sender, &address, text, o) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let mut out = format!(
        "Sent message {}{} {}",
        m.id,
        attached_line(&files),
        s.outcome(&d)
    );
    if !r.is_empty() {
        let to = if d.to.is_empty() {
            &address.value
        } else {
            &d.to
        };
        out += &s.recorded(&m, to, held_on_card(ib, &d));
    }
    out
}

/// `to` names a pull request ("pr:..." or a GitHub URL) rather than an
/// agent.
fn is_pr_target(to: &str) -> bool {
    let b = to.as_bytes();
    if b.len() >= 3 && b[..3].eq_ignore_ascii_case(b"pr:") {
        return true;
    }
    gocompat::text::to_lower_str(to).contains("github.com/")
}

const REPLY_ARGS: &[(&str, Ty)] = &[("delivery_id", Ty::Str), ("text", Ty::Str)];

pub(crate) fn reply(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, REPLY_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = trim(a.str("delivery_id"));
    if id.is_empty() {
        return err_text(
            "give delivery_id: the delivery=\"…\" value of the <irori-message> you are answering.",
        );
    }
    if trim(a.str("text")).is_empty() {
        return err_text("give the reply text.");
    }
    let ib = s.b.inbox();
    let (m, d) = match ib.reply(&c.sender, id, a.str("text")) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    // A reply keeps the Ref of the message it answers.
    format!(
        "Replied to delivery {} (now answered) with message {} {}{}",
        one_line(id, 80),
        m.id,
        s.outcome(&d),
        s.recorded(&m, &d.to, held_on_card(ib, &d))
    )
}

impl Inner {
    /// Where a just-sent message went and what happens next.
    pub(crate) fn outcome(&self, d: &Delivery) -> String {
        let n = self.roster();
        let mut b = String::new();
        if !d.to.is_empty() {
            b += &format!("to {}", n.who(&d.to));
        } else {
            b += &format!("to repo {}", one_line(&d.repo_key, 160));
        }
        let pr = d.message.to.kind == addr::PR;
        if pr {
            b += &format!(" about {}", one_line(&d.message.to.value, 160));
        }
        b += &format!(" (delivery {}): ", d.id);
        match d.state.as_str() {
            st::HELD => match d.hold.as_str() {
                hold::UNOWNED => {
                    b += "no agent in that repo can take it yet, so it waits for one to arrive or for the person to route it."
                }
                hold::PAUSED => {
                    b += "held because the person paused all wakes; it goes out when they unpause. Don't resend it."
                }
                h => {
                    b += &format!(
                        "held by the recipient's wake policy ({h}); it waits until the person releases it. Don't resend it."
                    )
                }
            },
            st::QUEUED => {
                if !d.to.is_empty() && !n.live(&d.to) {
                    b += &self.not_running(d, &n);
                } else {
                    b += "queued; they are woken with it, and their reply comes back to you as a message.";
                }
            }
            other => b += &format!("{other}."),
        }
        // A PR message its owner couldn't take says who it was for.
        let owner = one_line(&d.for_, MAX_FIELD_RUNES);
        if pr && !owner.is_empty() {
            b += &format!(" It went to the repo instead: its owner {owner} isn't running now.");
        }
        b
    }

    /// The outcome of a message queued for an agent that isn't running.
    fn not_running(&self, d: &Delivery, n: &Names<'_>) -> String {
        const WAITS: &str = "queued; they aren't running now, so it waits until they come back.";
        let m = &d.message;
        if m.to.kind == addr::REPO
            || m.to.kind == addr::PR
            || !d.repo_key.is_empty()
            || m.notice
            || n.owner(&d.to)
        {
            return WAITS.into();
        }
        const LEAD: &str = "queued; they aren't running now. If they don't come back within a few minutes it goes ";
        if let Some(tid) = card_for(self.b.coord(), &m.ref_) {
            return format!("{LEAD}on task:{}'s thread instead.", one_line(&tid, 80));
        }
        if n.owner(&m.from.key) {
            return WAITS.into();
        }
        format!("{LEAD}to the person, marked recipient gone.")
    }
}

const INBOX_ARGS: &[(&str, Ty)] = &[("all", Ty::Bool), ("limit", Ty::Int)];

pub(crate) fn inbox(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, INBOX_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let all = a.bool("all");
    let mut limit = a.int("limit");
    if limit <= 0 {
        limit = DEFAULT_INBOX_LIMIT;
    }
    let limit = limit.min(MAX_INBOX_LIMIT);
    let ib = s.b.inbox();
    // For is the recipient's own view: held deliveries never appear.
    let ds = ib.for_agent(&c.sender.key, all, limit as usize);
    let what = if all {
        "recent messages"
    } else {
        "open messages"
    };
    if ds.is_empty() {
        return format!("No {what} for you.");
    }
    let channel = c.path == path::CHANNEL;
    let n = s.roster();
    let now = s.now();

    // Fit whole rows into MAX_RESULT before anything is marked read, so a
    // row the result can't carry is never cut and never counted as seen.
    let head = |shown: usize| -> String {
        if shown == ds.len() {
            format!(
                "{} {what} for you, newest first. Answer with reply (delivery_id), or use defer or skip. {}\n",
                ds.len(),
                untrusted()
            )
        } else {
            format!(
                "{} {what} for you, newest first; showing the first {shown}. Answer with reply (delivery_id), or use defer or skip. {}\n",
                ds.len(),
                untrusted()
            )
        }
    };
    let foot = |more: usize| -> String {
        if more == 0 {
            return String::new();
        }
        format!(
            "\n[{more} more not shown; they stay unread. Answer, defer or skip the ones above, then call inbox again.]\n"
        )
    };
    let budget = MAX_RESULT as i64 - head(0).len() as i64 - foot(ds.len()).len() as i64;
    let mut rows: Vec<(Delivery, usize)> = vec![];
    let mut used = 0i64;
    for d in &ds {
        let mut d = d.clone();
        if channel && d.state == st::QUEUED {
            d.state = st::DELIVERED.into();
        }
        let mut cap = MAX_LIST_TEXT;
        let mut size = inbox_row(&n, &now, &d, cap).len() as i64;
        // One row with heavily escaped text can outgrow the whole budget;
        // shorten the first row's text rather than show nothing.
        while rows.is_empty() && size > budget && cap > 64 {
            cap /= 2;
            size = inbox_row(&n, &now, &d, cap).len() as i64;
        }
        if used + size > budget {
            break;
        }
        used += size;
        rows.push((d, cap));
    }
    let render = |rows: &[(Delivery, usize)]| -> String {
        let mut b = head(rows.len());
        for (d, cap) in rows {
            b += &inbox_row(&n, &now, d, *cap);
        }
        b += &foot(ds.len() - rows.len());
        b
    };
    // The budget is an estimate; check the real result and drop rows until
    // it fits, so the clip never cuts a marked row.
    let mut out = render(&rows);
    while out.len() > MAX_RESULT && !rows.is_empty() {
        rows.pop();
        out = render(&rows);
    }
    // A Grok push isn't delivered until the recipient shows it has it;
    // reading the inbox does, but only for rows shown in full.
    if channel {
        let mut changed = false;
        for (i, row) in rows.iter_mut().enumerate() {
            if ds[i].state == st::QUEUED && ib.mark_read(&c.sender.key, &ds[i].id).is_err() {
                row.0.state = st::QUEUED.into();
                changed = true;
            }
        }
        if changed {
            out = render(&rows);
        }
    }
    out
}

/// One delivery for the inbox list, text clipped to `cap` runes.
fn inbox_row(n: &Names<'_>, now: &GoTime, d: &Delivery, cap: usize) -> String {
    let mut b = String::new();
    let m = &d.message;
    let _ = writeln!(
        b,
        "\n- delivery {} [{}] from {}, {}, trust {}",
        d.id,
        d.state,
        n.sender(&m.from),
        ago(now, &m.sent_at),
        one_line(&m.trust, 20)
    );
    match m.to.kind.as_str() {
        addr::REPO => {
            let _ = writeln!(b, "  to repo: {}", one_line(&m.to.value, 160));
        }
        addr::PR => {
            let _ = writeln!(b, "  about PR: {}", one_line(&m.to.value, 160));
            let owner = one_line(&d.for_, MAX_FIELD_RUNES);
            if !owner.is_empty() {
                let _ = writeln!(b, "  for: {owner}, the PR's owner, who isn't running now");
            }
        }
        _ => {}
    }
    if !m.reply_to.is_empty() {
        let _ = writeln!(b, "  in reply to message: {}", one_line(&m.reply_to, 80));
    }
    if d.state == st::DEFERRED && !d.until.is_zero() {
        let _ = writeln!(b, "  deferred until: {}", in_(now, &d.until));
    }
    let note = one_line(&d.note, MAX_FIELD_RUNES);
    if !note.is_empty() {
        let _ = writeln!(b, "  note: {note}");
    }
    let r = one_line(&m.reason, MAX_FIELD_RUNES);
    if !r.is_empty() {
        let _ = writeln!(b, "  reason (untrusted): {r}");
    }
    let _ = writeln!(b, "  text (untrusted): {}", quoted(&m.text, cap));
    for f in &m.attachments {
        let _ = writeln!(
            b,
            "  attachment (untrusted name): {}",
            one_line(&describe_inbox(f), 300)
        );
    }
    b
}

const ACK_ARGS: &[(&str, Ty)] = &[("delivery_id", Ty::Str)];

pub(crate) fn ack(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, ACK_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    if c.path == path::MOD {
        return err_text(
            "the Irori mod acknowledges deliveries for you; reply, defer or skip instead.",
        );
    }
    let id = trim(a.str("delivery_id"));
    if id.is_empty() {
        return err_text("give delivery_id.");
    }
    if let Err(e) = s.b.inbox().ack(&c.sender.key, id) {
        return map_err(e);
    }
    format!(
        "Acknowledged delivery {}. Reply when you have an answer.",
        one_line(id, 80)
    )
}

const DEFER_ARGS: &[(&str, Ty)] = &[
    ("delivery_id", Ty::Str),
    ("minutes", Ty::OptInt),
    ("note", Ty::Str),
];

pub(crate) fn defer(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, DEFER_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = trim(a.str("delivery_id"));
    if id.is_empty() {
        return err_text("give delivery_id.");
    }
    let Some(m) = a.opt_int("minutes") else {
        return err_text("give minutes (1 to 720).");
    };
    if !(1..=720).contains(&m) {
        return err_text(&format!("minutes must be 1 to 720 (12 hours), not {m}."));
    }
    if let Err(e) = s.b.inbox().defer(&c.sender.key, id, m, trim(a.str("note"))) {
        return map_err(e);
    }
    format!(
        "Deferred delivery {} for {m} minutes; it comes back to you then.",
        one_line(id, 80)
    )
}

const SKIP_ARGS: &[(&str, Ty)] = &[("delivery_id", Ty::Str), ("note", Ty::Str)];

pub(crate) fn skip(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, SKIP_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = trim(a.str("delivery_id"));
    if id.is_empty() {
        return err_text("give delivery_id.");
    }
    if let Err(e) = s.b.inbox().skip(&c.sender.key, id, trim(a.str("note"))) {
        return map_err(e);
    }
    format!(
        "Skipped delivery {}; it is closed without a reply.",
        one_line(id, 80)
    )
}
