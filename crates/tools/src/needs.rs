//! Needs you for the Office Assistant (decision 69, plan
//! docs/design/task-proposals.md §5.1–5.2): the `needs_you` read tool, the
//! snapshot the turn's grants resolve against, and the four writes the
//! person's own message can allow: start, not now, give back and give.
//!
//! [`given_rows`] is the one list of what was given to the person: the
//! tool numbers it and the snapshot is taken from it, so "the first three"
//! means the same three in both.

use std::collections::BTreeMap;

use coord::{GivenSay, Task};

use crate::args::Ty;
use crate::assistant_writes::{ItemState, SnapItem, SnapPerson, Snapshot};
use crate::format::{MAX_NAME, SENDER_NOTE, fence_sender, one_line};
use crate::shared::BoardInfo;
use crate::{Backend, CLIENT_ASSISTANT, Caller, Inner, decode, err_text, map_err};

/// How much of a card's objective Needs you shows.
const MAX_ASKS: usize = 400;
/// How much of a batch note it shows.
const MAX_NOTE: usize = 600;
/// How much of a question it shows.
const MAX_QUESTION: usize = 300;

/// One card given to the person, as Needs you lists it.
pub(crate) struct GivenRow {
    pub task: Task,
    /// `ops#12`.
    pub short: String,
    /// `person:<id>` of who gave it.
    pub giver_key: String,
    pub giver_name: String,
    pub batch: String,
    pub note: String,
    pub via_assistant: bool,
    pub state: ItemState,
    /// When it was given, in ms.
    pub given_ms: i64,
    /// Why its agent couldn't start, when it couldn't.
    pub failed: Option<String>,
}

/// The person's own key, whoever calls: an agent's `needs_you` reads what
/// waits on its person, never on itself.
fn person(b: &dyn Backend) -> String {
    format!("user:{}", b.device_id())
}

/// A person's name as this device knows it, one line and clipped.
fn name_of(known: &BTreeMap<String, String>, key: &str) -> String {
    known
        .get(key)
        .map(|n| one_line(n, MAX_NAME))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "another person".into())
}

/// A member's chosen name where Irori's own words are, outside a fence:
/// quoted and escaped, so it reads as a name and can't open a fence or
/// pass for Irori's line.
fn named(name: &str) -> String {
    crate::format::q(&crate::format::escape(&one_line(name, MAX_NAME)))
}

fn state_of(co: &coord::Coord, t: &Task) -> ItemState {
    let Some(g) = t.given.as_ref() else {
        return ItemState::NotAsked;
    };
    if t.blockers
        .iter()
        .any(|q| q.open() && q.question == coord::GIVEN_QUESTION && q.id == g.question)
    {
        return ItemState::Asked;
    }
    if co.agent_on_or_starting(t) || t.given_said.as_ref().is_some_and(|s| s.yes) {
        return ItemState::Starting;
    }
    if g.asker.is_empty() {
        return ItemState::NotAsked;
    }
    ItemState::NotNow
}

/// Everything given to the person on their spaces' boards, still open, in
/// one fixed order: space by space as the board list has them, then as
/// `Coord::given_to` orders them (giver, batch, card number). Never by
/// when it changed, so the order can't move between a read and a turn.
pub(crate) fn given_rows(b: &dyn Backend) -> Vec<GivenRow> {
    let Some(sb) = b.shared_boards() else {
        return Vec::new();
    };
    let me = person(b);
    let known = b.known_names();
    let mut seen: Vec<String> = vec![];
    let mut out = vec![];
    for bi in sb.boards() {
        if seen.contains(&bi.board_id) {
            continue;
        }
        seen.push(bi.board_id.clone());
        let Ok((co, keys)) = sb.board_coord(&bi.board_id, std::slice::from_ref(&me)) else {
            continue;
        };
        for it in co.given_to(&keys[0]) {
            let g = it.task.given.clone().unwrap_or_default();
            let giver_key = format!("{}{}", store::kinds::PERSON_ACTOR_PREFIX, it.giver);
            out.push(GivenRow {
                short: it.task.short_ref(),
                giver_name: name_of(&known, &giver_key),
                giver_key,
                batch: it.batch.clone(),
                note: g.note.clone(),
                via_assistant: g.assistant,
                state: state_of(&co, &it.task),
                given_ms: g.at.unix_milli_value(),
                failed: coord::Coord::given_failed(&it.task),
                task: it.task,
            });
        }
    }
    out
}

/// What was given to the person and who they share a space with, now
/// (§5.1): what a turn's grants resolve against. Core takes it when the
/// turn worker takes a question.
pub fn needs_snapshot(b: &dyn Backend) -> Snapshot {
    needs_snapshot_before(b, i64::MAX)
}

/// [`needs_snapshot`] with only the cards given at or before `before_ms`:
/// the person's yes covers what was there when they typed, never a card
/// given while their question waited.
pub fn needs_snapshot_before(b: &dyn Backend, before_ms: i64) -> Snapshot {
    let rows = given_rows(b);
    let all = rows.len();
    let items: Vec<(SnapItem, String)> = rows
        .into_iter()
        .filter(|r| r.given_ms <= before_ms)
        .map(|r| {
            (
                SnapItem {
                    id: r.task.id.clone(),
                    giver_key: r.giver_key,
                    giver_name: r.giver_name,
                    batch: r.batch,
                    state: r.state,
                    given_ms: r.given_ms,
                    ..SnapItem::default()
                },
                r.short,
            )
        })
        .collect();
    let people = b
        .shared_boards()
        .map(|sb| sb.dm_people())
        .unwrap_or_default()
        .into_iter()
        .map(|p| SnapPerson {
            person_key: p.key,
            name: one_line(&p.name, MAX_NAME),
        })
        .collect();
    let partial = items.len() < all;
    Snapshot {
        partial,
        ..Snapshot::new(items, people)
    }
}

/// The words for a given card's state, and what the person can do.
fn state_line(r: &GivenRow) -> String {
    match (r.state, &r.failed) {
        (_, Some(why)) => format!("couldn't start: {}", one_line(why, 120)),
        (ItemState::Asked, _) => "asks: Start an agent, Not now or Give back".into(),
        (ItemState::NotNow, _) => "Not now: no agent; Start an agent or Give back".into(),
        (ItemState::Starting, _) => "an agent is on it or starting".into(),
        (ItemState::NotAsked, _) => {
            "not asked yet: it asks on the computer with its repo once that's on".into()
        }
    }
}

/// A card's title and what it asks, for a fence.
fn card_words(t: &Task) -> String {
    let mut w = format!("Title: {}", one_line(&t.title, 200));
    let asks = t.objective.trim();
    if !asks.is_empty() {
        w.push_str(&format!("\nAsks: {}", one_line(asks, MAX_ASKS)));
    }
    w
}

/// needs_you: what waits on the person, with every word a member wrote
/// fenced (decision 69, §5.1).
pub(crate) fn needs_you(s: &Inner, _c: &Caller, raw: &[u8]) -> String {
    if let Err(e) = decode(raw, &[]) {
        return e;
    }
    let b = s.b.as_ref();
    let me = person(b);
    let rows = given_rows(b);
    let mut out = format!("{SENDER_NOTE}\n");
    if rows.is_empty() {
        out.push_str("\nNothing is given to you.\n");
    } else {
        out.push_str(&format!(
            "\nGiven to you ({}), numbered in the order \"the first N\" counts:\n",
            rows.len()
        ));
        let mut group: Option<(String, String)> = None;
        for (i, r) in rows.iter().enumerate() {
            let here = (r.giver_key.clone(), r.batch.clone());
            if group.as_ref() != Some(&here) {
                let n = rows
                    .iter()
                    .filter(|x| (&x.giver_key, &x.batch) == (&here.0, &here.1))
                    .count();
                let via = if r.via_assistant {
                    format!(" (via {}'s assistant)", named(&r.giver_name))
                } else {
                    String::new()
                };
                let what = if n == 1 {
                    "1 task".to_string()
                } else {
                    format!("{n} tasks")
                };
                out.push_str(&format!(
                    "\n{} gave you {what}{via}.\n",
                    named(&r.giver_name)
                ));
                let note = r.note.trim();
                if !note.is_empty() {
                    out.push_str(&format!(
                        "Their note:\n{}\n",
                        fence_sender(&r.giver_name, &one_line(note, MAX_NOTE))
                    ));
                }
                group = Some(here);
            }
            out.push_str(&format!("{}. {}, {}\n", i + 1, r.short, state_line(r)));
            out.push_str(&fence_sender(&r.giver_name, &card_words(&r.task)));
            out.push('\n');
        }
    }
    // The rest of what waits on them: give-backs, cards an agent made
    // after reading others' text, and their other questions.
    let given: Vec<&str> = rows.iter().map(|r| r.task.id.as_str()).collect();
    let view = s.board_view("", &me, 1);
    let (mut back, mut marked, mut other) = (vec![], vec![], vec![]);
    for card in &view.needs_you {
        let t = &card.task;
        if given.contains(&t.id.as_str()) {
            continue;
        }
        for q in t.blockers.iter().filter(|q| q.open()) {
            let on_person =
                q.on.as_ref()
                    .is_none_or(|o| o.kind == coord::BLOCKED_ON_PERSON);
            if q.given_back {
                let who = bare_name(&s.roster().who(&q.raised_by));
                let why = coord::given_back_reason(&q.context);
                let mut line = format!(
                    "- {}, given back by {}: Keep it or Cancel it",
                    t.short_ref(),
                    named(&who),
                );
                if why.is_empty() {
                    line.push_str(" (no reason given)");
                } else {
                    line.push_str(&format!(
                        "\nTheir reason:\n{}",
                        fence_sender(&who, &one_line(why, MAX_NOTE))
                    ));
                }
                back.push(line);
            } else if q.question == coord::GIVEN_QUESTION && t.given.is_none() {
                marked.push(format!(
                    "- {}: an agent made or changed it after reading another person's text; it asks OK to start an agent",
                    t.short_ref()
                ));
            } else if on_person && q.question != coord::GIVEN_QUESTION {
                let who = bare_name(&s.roster().who(&q.raised_by));
                other.push(format!(
                    "- {} asks:\n{}",
                    t.short_ref(),
                    fence_sender(&who, &one_line(&q.question, MAX_QUESTION))
                ));
            }
        }
    }
    for (head, list) in [
        ("Given back to you", &back),
        ("Made after reading others' text", &marked),
        ("Your other questions", &other),
    ] {
        if !list.is_empty() {
            out.push_str(&format!(
                "\n{head} ({}):\n{}\n",
                list.len(),
                list.join("\n")
            ));
        }
    }
    out
}

// ---- the writes ---------------------------------------------------------------

const ONLY_THE_CHAT: &str = "only the window's assistant has this tool.";
const TASK_ARGS: &[(&str, Ty)] = &[("task_id", Ty::Str)];
const BACK_ARGS: &[(&str, Ty)] = &[("task_id", Ty::Str), ("reason", Ty::Str)];
const GIVE_ARGS: &[(&str, Ty)] = &[("to", Ty::Str), ("task_ids", Ty::Strs), ("note", Ty::Str)];

/// The card the API put in `task_id`, from the person's grant.
fn card_of(s: &Inner, v: &str) -> Result<String, String> {
    let id = s.task_id(v)?;
    Ok(id.strip_prefix("task:").unwrap_or(&id).to_string())
}

/// assistant_start: the person's granted yes on a card given to them
/// (§3.2): Core starts it here, or asks the computer with its repo.
pub(crate) fn assistant_start(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    if c.sender.kind != CLIENT_ASSISTANT {
        return err_text(ONLY_THE_CHAT);
    }
    let a = match decode(raw, TASK_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match card_of(s, a.str("task_id")) {
        Ok(id) => id,
        Err(e) => return e,
    };
    let short = s.short_of(&id);
    match s.b.given_start(&c.sender.key, &id) {
        Ok(line) if line.starts_with("Started") => format!("{line}: an agent starts on it."),
        Ok(line) => format!("{short}: {line}."),
        Err(e) => err_text(&e),
    }
}

/// assistant_not_now: Not now on a card given to the person.
pub(crate) fn assistant_not_now(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    if c.sender.kind != CLIENT_ASSISTANT {
        return err_text(ONLY_THE_CHAT);
    }
    let a = match decode(raw, TASK_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match card_of(s, a.str("task_id")) {
        Ok(id) => id,
        Err(e) => return e,
    };
    let (co, keys) = match s.on(&id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    match co.say_given(&keys[0], &id, GivenSay::NotNow) {
        Ok(t) => format!(
            "Put {} in Not now: it stays given to you, with no agent.",
            t.short_ref()
        ),
        Err(e) => map_err(e),
    }
}

/// assistant_give_back: returns a card given to the person to its owner,
/// with the person's own reason from the grant.
pub(crate) fn assistant_give_back(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    if c.sender.kind != CLIENT_ASSISTANT {
        return err_text(ONLY_THE_CHAT);
    }
    let a = match decode(raw, BACK_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match card_of(s, a.str("task_id")) {
        Ok(id) => id,
        Err(e) => return e,
    };
    let reason = a.str("reason").trim().to_string();
    let (co, keys) = match s.on(&id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    match co.give_back(&keys[0], &id, &reason) {
        Ok(t) => {
            // It goes to whoever decides on the card, its owner, who may
            // not be the person who gave it (#432).
            let owner = t
                .blockers
                .iter()
                .rev()
                .find(|b| b.given_back && b.open())
                .and_then(|b| b.on.as_ref())
                .map(|o| name_of(&s.b.known_names(), &o.reference))
                .unwrap_or_else(|| "its owner".into());
            if reason.is_empty() {
                format!("Gave {} back to {owner}.", t.short_ref())
            } else {
                format!(
                    "Gave {} back to {owner}: \u{201c}{}\u{201d}",
                    t.short_ref(),
                    one_line(&reason, 100)
                )
            }
        }
        Err(e) => map_err(e),
    }
}

/// assistant_give: the person's own cards, as they named them, to one
/// person as one batch (§5.2.1, as the person chose on #1206: existing
/// cards only; new tasks the assistant writes go out as a draft they tap).
/// Cards on a space's board are given there; Personal cards are shared to
/// the one space the person shares with them and given in the same step.
pub(crate) fn assistant_give(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    if c.sender.kind != CLIENT_ASSISTANT {
        return err_text(ONLY_THE_CHAT);
    }
    let a = match decode(raw, GIVE_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let to = a.str("to").trim().to_string();
    let refs = a.strs("task_ids");
    if refs.is_empty() || refs.len() > coord::MAX_BATCH {
        return err_text(&format!("give 1 to {} cards at a time.", coord::MAX_BATCH));
    }
    if !to.starts_with(store::kinds::PERSON_ACTOR_PREFIX) {
        return err_text("give to a person.");
    }
    let Some(sb) = s.b.shared_boards() else {
        return err_text("shared spaces aren't on here, so there is no one to give to.");
    };
    let mut ids = Vec::with_capacity(refs.len());
    for r in &refs {
        match card_of(s, r) {
            Ok(id) => ids.push(id),
            Err(e) => return e,
        }
    }
    let name = name_of(&s.b.known_names(), &to);
    // Every card is checked before anything is written: one board, or
    // all Personal.
    let on: Vec<Option<BoardInfo>> = ids.iter().map(|id| sb.board_of(id)).collect();
    let me = c.sender.key.clone();
    if on.iter().all(Option::is_none) {
        // Personal cards go to the one space shared with them.
        let mut spaces: Vec<BoardInfo> = vec![];
        for bi in sb.boards() {
            if bi.closed
                || !bi.space_board.is_empty()
                || spaces.iter().any(|x| x.board_id == bi.board_id)
            {
                continue;
            }
            if s.member_key(&bi.board_id, &to).is_some() {
                spaces.push(bi);
            }
        }
        let b = match spaces.as_slice() {
            [one] => one.clone(),
            [] => {
                return err_text(&format!(
                    "you share no space with {name}, so these cards can't go to them."
                ));
            }
            many => {
                return err_text(&format!(
                    "you share more than one space with {name} ({}): share the cards to one of them first, then give them there.",
                    many.iter()
                        .map(|b| b.tag.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        };
        return match sb.share_give(&ids, &b.board_id, &to) {
            Ok(given) => gave(&given, &name, &b, true),
            Err(e) => err_text(&e),
        };
    }
    let Some(Some(b)) = on.first().cloned() else {
        return err_text("give your Personal cards and your cards on a space's board separately.");
    };
    if on.iter().any(|x| {
        x.as_ref()
            .is_none_or(|x| (&x.board_id, &x.space_board) != (&b.board_id, &b.space_board))
    }) {
        return err_text(
            "these cards are on more than one board: give one board's cards at a time.",
        );
    }
    let (co, keys) = match sb.board_coord(&b.board_id, &[me, to.clone()]) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    match co.give_batch(
        &keys[0],
        &b.space_board,
        &keys[1],
        coord::GiveItems::Cards(ids),
        "",
        true,
    ) {
        Ok(given) => gave(&given, &name, &b, false),
        Err(e) => map_err(e),
    }
}

/// The reply for a give: what went, to whom, and who sees it.
fn gave(given: &[Task], name: &str, b: &BoardInfo, shared: bool) -> String {
    let refs: Vec<String> = given.iter().map(Task::short_ref).collect();
    let what = if given.len() == 1 {
        "1 card".to_string()
    } else {
        format!("{} cards", given.len())
    };
    let mut out = format!(
        "Gave {name} {what} on {} as one batch: {}.",
        b.tag,
        refs.join(", ")
    );
    if shared {
        out.push_str(
            " They were Personal, so each was shared there first: these are the new refs.",
        );
    }
    out.push_str(&format!(
        " {name} is asked before any agent starts on them. {}",
        b.visible_to()
    ));
    out
}

/// A roster name without the key after it: "ringo (abc/def)" is "ringo".
fn bare_name(who: &str) -> String {
    match who.split_once(" (") {
        Some((name, _)) if who.ends_with(')') && !name.trim().is_empty() => name.to_string(),
        _ => who.to_string(),
    }
}
