//! Asks (I:internal/tools/room.go): the person talking to the room. One
//! agent takes an ask with ask_claim, talks it through on the card, and
//! ends it with ask_close.

use coord::AskOutcome;

use crate::args::Ty;
use crate::board::steps_of;
use crate::format::{one_line, trim};
use crate::{Caller, Inner, decode, err_text, map_err};

const CLAIM_ARGS: &[(&str, Ty)] = &[("task_id", Ty::Str)];

pub(crate) fn ask_claim(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, CLAIM_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    // A board's ask (§9) is claimed in its board's log, as the caller's
    // card there; the asking device decides the take.
    let (co, as_) = match s.on(&id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let r = co.ask_claim(&as_[0], &id, coord::ASK_WAIT);
    let n = s.roster();
    let r = match r {
        Ok(r) => r,
        Err(e) => {
            if e.is(coord::ErrorKind::Held)
                && let Some(t) = co.get_task(&id)
                && let Some(ask) = &t.ask
                && !ask.taker.is_empty()
            {
                return format!(
                    "{} took {}. To add something, board_comment it; otherwise leave it.",
                    n.who(&ask.taker),
                    t.short_ref()
                );
            }
            return map_err(e);
        }
    };
    let r_ = r.task.short_ref();
    if r.mine {
        return format!(
            "You have {r_}. Talk it through with the person on the card first (board_comment) if it isn't clear what \
they want, then ask_close it with what it became: a card, a change to another card, an answer, or nothing.\n{}",
            s.card_line(&r.task, None, &n, &s.now())
        );
    }
    if !r.taker.is_empty() {
        return format!(
            "{} took {r_} first; nothing to do unless you want to chime in (board_comment).",
            n.who(&r.taker)
        );
    }
    format!(
        "Requested. The device that hands out {r_} ({}) hasn't answered (offline?). Don't start: you'll hear at your next turn either way.",
        one_line(&r.owner, 80)
    )
}

const CLOSE_ARGS: &[(&str, Ty)] = &[
    ("task_id", Ty::Str),
    ("outcome", Ty::Str),
    ("note", Ty::Str),
    ("title", Ty::Str),
    ("objective", Ty::Str),
    ("done_when", Ty::Str),
    ("steps", Ty::Steps),
    ("task", Ty::Str),
    ("parent", Ty::Str),
];

pub(crate) fn ask_close(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, CLOSE_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    let kind = gocompat::text::to_lower_str(trim(a.str("outcome")));
    if trim(a.str("note")).is_empty() {
        return err_text("give note: one line on what it became and why.");
    }
    let mut o = AskOutcome {
        kind: kind.clone(),
        note: a.str("note").into(),
        title: a.str("title").into(),
        objective: a.str("objective").into(),
        done_when: a.str("done_when").into(),
        steps: steps_of(&a.steps("steps")),
        task: String::new(),
        parent: trim(a.str("parent")).into(),
    };
    match kind.as_str() {
        coord::OUTCOME_CARD => {
            if trim(a.str("title")).is_empty() {
                return err_text("a card outcome needs title: the card's short title.");
            }
        }
        coord::OUTCOME_EDIT => {
            if trim(a.str("task")).is_empty() {
                return err_text("an edit outcome needs task: the card it changed (#12).");
            }
            match s.task_id(a.str("task")) {
                Ok(t) => o.task = t,
                Err(m) => return m,
            }
        }
        coord::OUTCOME_ANSWERED | coord::OUTCOME_NONE => {}
        _ => {
            return err_text(
                "give outcome: card (it becomes a card), edit (it changes another card), answered (you answered the question) or none.",
            );
        }
    }
    let target = o.task.clone();
    let (co, as_) = match s.on(&id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let t = match co.ask_close(&as_[0], &id, o) {
        Ok(t) => t,
        Err(e) => return map_err(e),
    };
    let n = s.roster();
    match kind.as_str() {
        coord::OUTCOME_CARD => {
            let part = if t.parent.is_empty() {
                String::new()
            } else {
                format!(", a subtask of {},", s.short_of(&t.parent))
            };
            format!(
                "{} is a card now{part} and you hold it. Give it steps (board_steps) if it has none, and keep them ticked.\n{}",
                t.short_ref(),
                s.card_line(&t, None, &n, &s.now())
            )
        }
        coord::OUTCOME_EDIT => format!(
            "Changed {}; the ask {} is done.",
            s.short_of(&target),
            t.short_ref()
        ),
        coord::OUTCOME_ANSWERED => format!(
            "Answered {}; it is done. If the person comments on it again, it reopens and you hear.",
            t.short_ref()
        ),
        _ => format!("Closed {}: nothing came of it.", t.short_ref()),
    }
}
