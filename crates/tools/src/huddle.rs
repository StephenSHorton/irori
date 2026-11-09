//! Huddle tools for ordinary agents (card #1055; docs/design/call-agents.md
//! §11): `huddle_status`, `huddle_canvas` and `huddle_note`.
//!
//! Nothing is pushed to an agent. It reaches for these when it needs them.
//! The two reads end with [`LIVE_LINE`] while the huddle is live, as
//! `talk_read conv=call:<id>` does. `huddle_note` adds one `open` item
//! signed by the agent: it can't settle, move or card anything.

use std::sync::Arc;

use coord::talk::{self as ct, CanvasItem, NewItem, Talk};

use crate::args::Ty;
use crate::format::{one_line, trim};
use crate::talk::{call_info, call_parent, call_place, me};
use crate::{Backend, Caller, Inner, decode, err_text, untrusted};

/// What a read of a live huddle ends with (§11).
pub(crate) const LIVE_LINE: &str =
    "This huddle is still going. What's said may change: don't start work from it until it ends.";

/// How much of the canvas one `huddle_canvas` call shows at most, so the
/// result is never clipped (the clip is [`crate::MAX_RESULT`]): the page
/// stops here and `since` points at the last item shown.
const CANVAS_BUDGET: usize = 6 << 10;
/// The most items on one page.
const CANVAS_PAGE: usize = 40;
/// How much of each item's text `huddle_canvas` shows.
const ITEM_TEXT: usize = 600;

/// `call` as the huddle's id: bare, or `call:<id>` as talk_list names it.
fn call_id(v: &str) -> &str {
    let v = trim(v);
    v.strip_prefix(ct::CALL_PREFIX).unwrap_or(v).trim()
}

/// Whether huddle `call` is live: the Core's word, else its end mark.
/// `None` for a huddle this device doesn't know.
pub(crate) fn is_live(b: &dyn Backend, talk: &Talk, call: &str) -> Option<bool> {
    talk.call_conv(call)?;
    if let Some(h) = b.huddle_now(call) {
        return Some(!h.over);
    }
    let info = call_info(talk, &format!("{}{call}", ct::CALL_PREFIX))?;
    Some(info.end.is_none())
}

/// The line a read of huddle `call` ends with while it is live, else "".
pub(crate) fn live_tail(b: &dyn Backend, talk: &Talk, call: &str) -> String {
    if is_live(b, talk, call) == Some(true) {
        format!("{LIVE_LINE}\n")
    } else {
        String::new()
    }
}

/// The huddle named by the `call` argument, when `reader` may read it:
/// its id, the conversation it was in and the talk.
fn open<'a>(
    b: &'a dyn Backend,
    reader: &str,
    raw: &str,
) -> Result<(&'a Arc<Talk>, String, String), String> {
    let talk = b.talk().ok_or("Talk is off on this Core")?;
    let id = call_id(raw);
    if id.is_empty() {
        return Err("call is the huddle's id, as talk_list names it (call:<id>)".into());
    }
    let parent = call_parent(b, talk, reader, id)?;
    Ok((talk, id.to_string(), parent))
}

const STATUS_ARGS: &[(&str, Ty)] = &[("call", Ty::Str)];

/// huddle_status: title, live or over, people, whose assistant is
/// connected, and how many lines and items it has.
pub(crate) fn huddle_status(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, STATUS_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let b = s.b.as_ref();
    let (talk, id, parent) = match open(b, me(c), a.str("call")) {
        Ok(v) => v,
        Err(e) => return err_text(&e),
    };
    let n = s.roster();
    let info = call_info(talk, &format!("{}{id}", ct::CALL_PREFIX));
    let now = b.huddle_now(&id);
    let live = is_live(b, talk, &id) == Some(true);
    let title = info
        .as_ref()
        .map(|i| one_line(&i.title, 80))
        .unwrap_or_default();
    let mut out = format!(
        "Huddle {id}{} in {}: {}.\n",
        if title.is_empty() {
            String::new()
        } else {
            format!(" \"{title}\"")
        },
        call_place(talk, &n, &parent),
        if live { "live" } else { "over" },
    );
    // Who is in it: who the Core hears now, else who took part.
    let mut people: Vec<String> = now.as_ref().map(|h| h.people.clone()).unwrap_or_default();
    if people.is_empty() {
        people = info.as_ref().map(|i| i.people.clone()).unwrap_or_default();
    }
    if people.is_empty() {
        out += "People: none known here.\n";
    } else {
        // `who` names another person without a "gone" tag; this
        // device's person prints as itself.
        let names: Vec<String> = people.iter().map(|p| n.who(p)).collect();
        out += &format!("People: {}.\n", names.join(", "));
    }
    match now.as_ref().map(|h| &h.assistants) {
        Some(rows) if !rows.is_empty() => {
            let rows: Vec<String> = rows
                .iter()
                .map(|(p, st)| format!("{}'s assistant {st}", n.who(p)))
                .collect();
            out += &format!("Assistants: {}.\n", rows.join("; "));
        }
        _ if live => out += "Assistants: none connected.\n",
        _ => {}
    }
    let items = talk.call_items(&id);
    let open_items = items.iter().filter(|i| i.status == "open").count();
    let lines = crate::talk::visible_lines(talk, me(c), &id);
    out += &format!(
        "{lines} line{} said; {} canvas item{} ({open_items} open). huddle_canvas call={id} shows them; talk_read conv={}{id} the transcript.\n",
        if lines == 1 { "" } else { "s" },
        items.len(),
        if items.len() == 1 { "" } else { "s" },
        ct::CALL_PREFIX,
    );
    out + &live_tail(b, talk, &id) + untrusted()
}

const CANVAS_ARGS: &[(&str, Ty)] = &[("call", Ty::Str), ("since", Ty::Str)];

/// One item as a line: id, kind and status, who, text.
fn item_line(n: &crate::Names<'_>, i: &CanvasItem) -> String {
    let status = if i.status.is_empty() {
        String::new()
    } else {
        format!(" {}", i.status)
    };
    let who = if i.assistant {
        format!("{}'s assistant", n.who(&i.by))
    } else {
        n.who(&i.by)
    };
    let mut out = format!(
        "- [{}] {}{status} ({who}): {}",
        i.item_id,
        i.kind,
        one_line(&i.text, ITEM_TEXT)
    );
    if let Some(r) = &i.reply {
        out += &format!(" (reply to {r})");
    }
    if !i.refs.is_empty() {
        out += &format!(" (about {})", i.refs.join(", "));
    }
    if !i.card.is_empty() {
        out += &format!(" (carded: {})", i.card);
    }
    out + "\n"
}

/// huddle_canvas: the items, paged. The page is budgeted before it is
/// shown and `since` points at the last item on it, so nothing a clip would
/// cut is skipped over.
pub(crate) fn huddle_canvas(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, CANVAS_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let b = s.b.as_ref();
    let (talk, id, _) = match open(b, me(c), a.str("call")) {
        Ok(v) => v,
        Err(e) => return err_text(&e),
    };
    let items = talk.call_items(&id);
    if items.is_empty() {
        return format!(
            "Nothing on the canvas of huddle {id} yet.\n{}{}",
            live_tail(b, talk, &id),
            untrusted()
        );
    }
    let since = trim(a.str("since"));
    let start = if since.is_empty() {
        0
    } else {
        match items.iter().position(|i| i.item_id == since) {
            Some(p) => p + 1,
            None => return err_text(&format!("no item {} on this canvas", one_line(since, 80))),
        }
    };
    let n = s.roster();
    let mut body = String::new();
    let mut shown = 0;
    for i in items[start..].iter().take(CANVAS_PAGE) {
        let l = item_line(&n, i);
        // Always one item, so a page makes progress.
        if shown > 0 && body.len() + l.len() > CANVAS_BUDGET {
            break;
        }
        body += &l;
        shown += 1;
    }
    let left = items.len() - start - shown;
    let mut out = format!(
        "The canvas of huddle {id}, oldest first ({} item{}):\n{body}",
        items.len(),
        if items.len() == 1 { "" } else { "s" },
    );
    if left > 0 {
        let last = &items[start + shown - 1].item_id;
        out += &format!("{left} more: huddle_canvas call={id} since={last}\n");
    }
    out + &live_tail(b, talk, &id) + untrusted()
}

const NOTE_ARGS: &[(&str, Ty)] = &[
    ("call", Ty::Str),
    ("kind", Ty::Str),
    ("text", Ty::Str),
    ("reply", Ty::Str),
];

/// huddle_note: adds one `open` item to the canvas, signed by the agent.
/// Refused in a person DM, and under Ask me autonomy. It can't make a card,
/// settle or move anything: nothing is pushed to agents, so they write only
/// when something asked them to.
pub(crate) fn huddle_note(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, NOTE_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let b = s.b.as_ref();
    let by = me(c);
    if ct::is_person_key(by) {
        return err_text("huddle_note is for agents: a person writes on the canvas themselves");
    }
    let (talk, id, parent) = match open(b, by, a.str("call")) {
        Ok(v) => v,
        Err(e) => return err_text(&e),
    };
    if talk.pair_other(&parent).is_some() {
        return err_text("a huddle between two people is theirs: agents don't write on its canvas");
    }
    if let Some(ch) = talk.channel(&parent)
        && b.inbox().autonomy(&ch.space_id) == inbox::autonomy::ASK
    {
        return err_text(
            "your autonomy here is Ask me, so you can't add to a huddle's canvas yourself: ask your person to",
        );
    }
    let reply = trim(a.str("reply"));
    if !reply.is_empty() && !talk.call_items(&id).iter().any(|i| i.item_id == reply) {
        return err_text(&format!(
            "no item {} on this canvas to reply to: huddle_canvas call={id} lists them",
            one_line(reply, 80)
        ));
    }
    let item = NewItem {
        kind: trim(a.str("kind")).to_ascii_lowercase(),
        text: a.str("text").to_string(),
        reply: (!reply.is_empty()).then(|| reply.to_string()),
        // Never a status: the row is open, whatever is asked.
        ..NewItem::default()
    };
    match talk.write_item(&parent, &id, item, by) {
        Ok(i) => format!(
            "Added {} {} to the canvas of huddle {id}. It is open: a person settles it.",
            i.kind, i.item_id
        ),
        Err(e) => err_text(&e.to_string()),
    }
}
