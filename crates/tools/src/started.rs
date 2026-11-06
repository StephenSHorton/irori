//! agent_start / agents_started / agent_end (card #575, GitHub #66): agents
//! start and manage agents on the person's cards, inside the limit the
//! person sets in the app. There is deliberately no tool that changes it.

use crate::args::Ty;
use crate::format::{ago, one_line, quoted, trim};
use crate::{Caller, Inner, board, decode, err_text, map_err};

const MAX_TITLE: usize = 80;

const START_ARGS: &[(&str, Ty)] = &[("task_id", Ty::Str)];
const LIST_ARGS: &[(&str, Ty)] = &[];
const END_ARGS: &[(&str, Ty)] = &[("agent", Ty::Str), ("give_to", Ty::Str)];

/// The person's limit, one line.
fn limit_line(s: &Inner) -> String {
    match s.b.agent_limit() {
        Some(l) if l.on => format!(
            "The person's agents may start agents: {} of {} in use. Only the person changes this, in Settings › Agents.\n",
            l.used, l.max
        ),
        Some(_) => "The person has turned off \"My agents may start agents\". Only they change this, in Settings › Agents.\n".into(),
        None => String::new(),
    }
}

pub(crate) fn agent_start(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, START_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    match s.b.agent_start(&c.sender.key, &id) {
        Ok(said) => format!("{said}\n"),
        Err(why) => err_text(&why),
    }
}

pub(crate) fn agents_started(s: &Inner, _: &Caller, raw: &[u8]) -> String {
    if let Err(e) = decode(raw, LIST_ARGS) {
        return e;
    }
    let now = s.now();
    let co = s.b.coord();
    let mut cards: Vec<_> =
        s.b.agents()
            .into_iter()
            .filter(|a| a.status != "gone" && co.is_spawned(&a.key))
            .collect();
    cards.sort_by(|x, y| x.name.cmp(&y.name).then_with(|| x.key.cmp(&y.key)));
    let mut b = limit_line(s);
    if cards.is_empty() {
        b.push_str("Irori has started no agents that are running now.\n");
        return b;
    }
    for a in &cards {
        let Some(m) = co.spawned(&a.key) else {
            continue;
        };
        let card = match s.task(&m.task_id) {
            Some(t) => format!(
                "{} [{}] {}",
                t.short_ref(),
                t.state,
                quoted(&one_line(&t.title, MAX_TITLE), MAX_TITLE)
            ),
            None => format!("task:{}", m.task_id),
        };
        b += &format!(
            "- {} ({}): {}; on {card}; last active {}{}\n",
            a.name,
            a.key,
            a.status,
            ago(&now, &a.last_activity),
            if m.counted {
                "; counts toward the limit"
            } else {
                ""
            }
        );
    }
    b
}

pub(crate) fn agent_end(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, END_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let want = trim(a.str("agent"));
    if want.is_empty() {
        return err_text("give agent: its name or key (see the agents_started tool).");
    }
    let key = match s.assignee(want) {
        Ok(k) => k,
        Err(e) => return map_err(e),
    };
    // Checked first: the end can't be taken back.
    let to = trim(a.str("give_to"));
    if !to.is_empty() {
        match s.assignee(to) {
            Ok(k) if s.b.coord().is_spawned(&k) => {
                return err_text(
                    "that agent was started for its own card and can't take another; give it to an agent the person started, or leave give_to out for a fresh agent.",
                );
            }
            Ok(_) => {}
            Err(e) => return map_err(e),
        }
    }
    let card = match s.b.agent_end(&c.sender.key, &key) {
        Ok(id) => id,
        Err(why) => return err_text(&why),
    };
    let r = s.task(&card).map_or(card.clone(), |t| t.short_ref());
    if to.is_empty() {
        return format!(
            "Ending it: it writes its hand-off, then goes. If {r} is still open, Irori starts a fresh agent for it from Left off.\n"
        );
    }
    // A mark never moves to another card, so reassigning hands the card on.
    let args = serde_json::json!({ "task_id": card, "to": to }).to_string();
    let out = board::board_assign(s, c, args.as_bytes());
    format!("Ending it: it writes its hand-off, then goes.\n{out}")
}
