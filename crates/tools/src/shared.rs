//! Shared boards in the tools (v0.5 §7.1, §7.3): `board` on board_create,
//! board_share, the person's autonomy setting deciding what an agent may
//! do and what it must ask first, the "Visible to" line on every result
//! that wrote to a shared board, scrub reasons when a write is refused,
//! and a reply to a bridged message going on the card.
//!
//! A request (a card under Ask me, a share under Trusted) is a
//! notification, not chat (decision 26): an `ask.posted` the person
//! answers on any device. The requesting agent doesn't wait; the asking
//! device carries out the answer and tells the agent at its next turn.
//! Requests live in memory: one still open when the Core stops is answered
//! as a plain notify answer.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use coord::{Coord, NewTask, Task};

use crate::args::Ty;
use crate::board::steps_of;
use crate::format::{add, one_line, trim};
use crate::{Caller, Inner, decode, err_text, map_err};

/// One shared board as the tools show it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoardInfo {
    pub board_id: String,
    /// Its short handle in refs (`mlg#12`, B3).
    pub tag: String,
    pub name: String,
    /// The people on it, by the names this person knows them by, this
    /// person included.
    pub people: Vec<String>,
}

impl BoardInfo {
    /// "Visible to: Stephen, Blake" (§7.3).
    pub fn visible_to(&self) -> String {
        format!("Visible to: {}", self.people.join(", "))
    }

    fn label(&self) -> String {
        format!("{} ({})", one_line(&self.name, 80), self.tag)
    }
}

/// The shared boards this device holds (Go `core.Core`'s boards); the Core
/// provides it.
pub trait SharedBoards: Send + Sync {
    /// Every board this person's agents may post on.
    fn boards(&self) -> Vec<BoardInfo>;
    /// The shared board card `task_id` is on; `None` for a private card or
    /// an unknown id.
    fn board_of(&self, task_id: &str) -> Option<BoardInfo>;
    /// The board log's coord, with `keys` as that log knows them (an agent
    /// gets its `agent.card` there, §2.4).
    fn board_coord(
        &self,
        board_id: &str,
        keys: &[String],
    ) -> Result<(Arc<Coord>, Vec<String>), coord::Error>;
    /// Share to board (§5.2) without the window's preview: scrubbed, the
    /// private card closed and pointing at the copy. `by` is who asked.
    fn share(&self, task_id: &str, board_id: &str, by: &str) -> Result<Task, String>;
    /// Withdraw a shared card (§5.3) for the person.
    fn withdraw(&self, task_id: &str) -> Result<(), String>;
}

/// How long a request to the person waits for an answer.
pub const REQUEST_WAIT: Duration = Duration::from_secs(30 * 60);

pub const CHOICE_CREATE: &str = "Create";
pub const CHOICE_SHARE: &str = "Share";
pub const CHOICE_REVIEW: &str = "Review";
pub const CHOICE_NO: &str = "No";
pub const CHOICE_WITHDRAW: &str = "Withdraw";

/// The board tools that write to the card named by `task_id`.
const CARD_WRITES: &[&str] = &[
    "board_comment",
    "board_block",
    "board_unblock",
    "board_status",
    "board_step",
    "board_steps",
    "board_join",
    "board_leave",
    "board_assign",
    "board_link",
    "board_recommend",
    "board_split",
    "board_watch",
    "board_unwatch",
];

/// Arguments that name things rather than say them: not scrubbed.
const NOT_TEXT: &[&str] = &[
    "task_id",
    "blocker_id",
    "step_id",
    "id",
    "to",
    "assign_to",
    "attach",
    "adopt",
    "pick",
];

/// A request waiting on the person.
#[derive(Debug, Clone)]
pub(crate) enum Request {
    Create {
        agent: String,
        board: BoardInfo,
        task: Box<NewTask>,
        assign_to: String,
    },
    Share {
        agent: String,
        board: BoardInfo,
        task_id: String,
    },
    Withdraw {
        agent: String,
        board: BoardInfo,
        shared_id: String,
        shared_ref: String,
    },
}

pub(crate) type Requests = HashMap<String, Request>;

/// Every reason text can't go on a board as it is (empty: it can).
fn reasons_in(texts: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for t in texts {
        for r in scrub::board_reasons(t) {
            if !out.contains(&r) {
                out.push(r);
            }
        }
    }
    out
}

/// The string values of a tool's args that say something.
fn arg_texts(v: &serde_json::Value, key: &str, out: &mut Vec<String>) {
    if NOT_TEXT.contains(&key) {
        return;
    }
    match v {
        serde_json::Value::String(s) => out.push(s.clone()),
        serde_json::Value::Array(a) => a.iter().for_each(|x| arg_texts(x, "", out)),
        serde_json::Value::Object(o) => o.iter().for_each(|(k, x)| arg_texts(x, k, out)),
        _ => {}
    }
}

/// The refusal for a write scrub or the stricter pass blocks (§7.3).
fn refusal(b: &BoardInfo, reasons: &[String]) -> String {
    err_text(&format!(
        "nothing was written: {} is shared ({}), and the text has {}. Rephrase without it and try again.",
        b.label(),
        b.visible_to()
            .to_lowercase()
            .replacen("visible to: ", "visible to ", 1),
        reasons.join(" and ")
    ))
}

impl Inner {
    fn shared(&self) -> Option<&dyn SharedBoards> {
        self.b.shared_boards()
    }

    /// The board `v` names, by tag or boardId.
    pub(crate) fn find_board(&self, v: &str) -> Result<BoardInfo, String> {
        let Some(sb) = self.shared() else {
            return Err(err_text(
                "shared boards aren't on here; leave out board for a private card.",
            ));
        };
        let v = trim(v).trim_start_matches('#');
        let all = sb.boards();
        let hits: Vec<&BoardInfo> = all
            .iter()
            .filter(|b| b.board_id == v || b.tag.eq_ignore_ascii_case(v))
            .collect();
        match hits.as_slice() {
            [one] => Ok((*one).clone()),
            [] => {
                let list: Vec<String> = all.iter().map(|b| b.tag.clone()).collect();
                Err(err_text(&format!(
                    "no shared board {}. {}",
                    crate::format::q(v),
                    if list.is_empty() {
                        "You aren't on any shared board.".to_string()
                    } else {
                        format!("Yours: {}.", list.join(", "))
                    }
                )))
            }
            _ => Err(err_text(&format!(
                "more than one board has the tag {}: give its boardId ({}).",
                crate::format::q(v),
                hits.iter()
                    .map(|b| b.board_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    }

    /// The shared board a call's `task_id` names a card on.
    fn target_board(&self, args: &serde_json::Value) -> Option<BoardInfo> {
        let sb = self.shared()?;
        let v = args.get("task_id")?.as_str()?;
        let id = self.task_id(v).ok()?;
        sb.board_of(&id)
    }

    /// Runs a board tool through the shared-board rules: a write to a
    /// shared card is refused when scrub or the stricter pass would change
    /// its text, and a result that wrote ends with the Visible-to line. The
    /// board tool also lists the boards the agent may post on.
    pub(crate) fn around_board(
        &self,
        name: &str,
        args: &[u8],
        run: impl FnOnce() -> String,
    ) -> String {
        if self.shared().is_none() {
            return run();
        }
        if name == "board" {
            let out = run();
            return match self.boards_section() {
                Some(s) => format!("{out}\n{s}"),
                None => out,
            };
        }
        if !CARD_WRITES.contains(&name) {
            return run();
        }
        let v: serde_json::Value = serde_json::from_slice(args).unwrap_or_default();
        let Some(b) = self.target_board(&v) else {
            return run();
        };
        let mut texts = vec![];
        arg_texts(&v, "", &mut texts);
        let reasons = reasons_in(&texts.iter().map(String::as_str).collect::<Vec<_>>());
        if !reasons.is_empty() {
            return refusal(&b, &reasons);
        }
        let out = run();
        if out.starts_with("Error") {
            return out;
        }
        format!("{out}\n{}", b.visible_to())
    }

    /// The board tool's list of shared boards, with who sees each and the
    /// agents' autonomy there.
    fn boards_section(&self) -> Option<String> {
        let all = self.shared()?.boards();
        if all.is_empty() {
            return None;
        }
        let mut s = String::from(
            "Shared boards you may post on (board_create board=<tag>; anything there is seen by everyone on it):",
        );
        for b in &all {
            s += &format!(
                "\n- {} {}: {}; your autonomy there: {}",
                b.tag,
                crate::format::q(&one_line(&b.name, 80)),
                b.people.join(", "),
                self.b.inbox().autonomy(&b.board_id)
            );
        }
        Some(s)
    }

    /// Creates the card on the board as `agent` and assigns it.
    fn create_on(
        &self,
        agent: &str,
        b: &BoardInfo,
        task: NewTask,
        assign_to: &str,
    ) -> Result<String, String> {
        let sb = self.shared().ok_or("shared boards aren't on here")?;
        let (co, keys) = sb
            .board_coord(&b.board_id, &[agent.to_string(), assign_to.to_string()])
            .map_err(|e| map_err(e).trim_start_matches("Error: ").to_string())?;
        let t = co
            .create_task(&keys[0], task)
            .map_err(|e| map_err(e).trim_start_matches("Error: ").to_string())?;
        let n = self.roster();
        let mut out = format!(
            "Created {} (task:{}) on {}.",
            t.short_ref(),
            t.id,
            b.label()
        );
        if assign_to.is_empty() {
            out += " Take it up with board_join, or hand it over with board_assign.";
        } else {
            match co.assign(&keys[0], &t.id, &keys[1]) {
                Ok((_, notice)) => {
                    out += &format!(
                        " Assigned to {}.{}",
                        n.who(assign_to),
                        self.told(&self.tell(&inbox::Sender::new(agent, "", ""), notice), &n)
                    )
                }
                Err(e) => {
                    out += &format!(
                        " Assigning it failed: {}",
                        map_err(e).trim_start_matches("Error: ")
                    )
                }
            }
        }
        Ok(format!("{out}\n{}", b.visible_to()))
    }

    /// Posts a request to the person and keeps it until they answer.
    fn request(
        &self,
        c: &Caller,
        title: &str,
        text: &str,
        actions: &[&str],
        r: Request,
    ) -> Result<String, String> {
        let Some(book) = self.b.asks() else {
            return Err(err_text(
                "this needs your person's yes, and this Irori can't ask them (no approvals here).",
            ));
        };
        if let Err(e) = self.b.inbox().record_roster() {
            return Err(map_err(e));
        }
        let mut who = trim(&c.sender.name).to_string();
        if who.is_empty() {
            who = self.roster().who(&c.sender.key);
        }
        let ask = coord::approvals::Ask {
            by: c.sender.key.clone(),
            name: who,
            title: title.into(),
            text: text.into(),
            level: "info".into(),
            actions: actions.iter().map(|a| a.to_string()).collect(),
            until: add(&self.now(), REQUEST_WAIT),
            ..coord::approvals::Ask::default()
        };
        let posted = book
            .post(ask)
            .map_err(|e| err_text(&format!("the request couldn't be recorded: {e}")))?;
        self.board_requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(posted.id.clone(), r);
        Ok(posted.id)
    }
}

// --- board_create on a shared board ---

/// board_create with `board` (§7.3): Trusted and Autonomous create it now,
/// Ask me asks the person first.
pub(crate) fn create_on_board(
    s: &Inner,
    c: &Caller,
    a: &crate::args::Args,
    board: &str,
    repo: String,
    assign_to: String,
) -> String {
    let b = match s.find_board(board) {
        Ok(b) => b,
        Err(e) => return e,
    };
    if !trim(a.str("parent")).is_empty() {
        return err_text("a subtask goes on its parent's board: give parent and leave out board.");
    }
    let task = NewTask {
        title: trim(a.str("title")).into(),
        objective: trim(a.str("objective")).into(),
        done_criteria: trim(a.str("done_when")).into(),
        repo_key: repo,
        shared: a.bool("shared"),
        links: a.strs("links"),
        steps: steps_of(&a.steps("steps")),
        ..NewTask::default()
    };
    let mut texts: Vec<&str> = vec![&task.title, &task.objective, &task.done_criteria];
    texts.extend(task.links.iter().map(String::as_str));
    texts.extend(task.steps.iter().map(|st| st.text.as_str()));
    let reasons = reasons_in(&texts);
    if !reasons.is_empty() {
        return refusal(&b, &reasons);
    }
    let me = c.sender.key.clone();
    if s.b.inbox().autonomy(&b.board_id) == inbox::autonomy::ASK {
        let text = format!(
            "{} wants to put a card on {}, visible to {}: {}",
            s.roster().who(&me),
            b.label(),
            b.people.join(", "),
            crate::format::q(&one_line(&task.title, 200))
        );
        let r = Request::Create {
            agent: me,
            board: b.clone(),
            task: Box::new(task),
            assign_to,
        };
        return match s.request(
            c,
            &format!("New card on {}?", b.tag),
            &text,
            &[CHOICE_CREATE, CHOICE_NO],
            r,
        ) {
            Ok(id) => format!(
                "Asked your person first ({id}): your autonomy on {} is Ask me. Their answer reaches you at your next turn, and if they choose Create the card is made for you then. Carry on meanwhile and don't ask again.",
                b.tag
            ),
            Err(e) => e,
        };
    }
    match s.create_on(&me, &b, task, &assign_to) {
        Ok(out) => out,
        Err(e) => err_text(&e),
    }
}

// --- board_share ---

const SHARE_ARGS: &[(&str, Ty)] = &[("task_id", Ty::Str), ("board", Ty::Str)];

pub(crate) fn board_share(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, SHARE_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    if trim(a.str("board")).is_empty() {
        return err_text("say which shared board: board = its tag (the board tool lists them).");
    }
    let b = match s.find_board(a.str("board")) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let Some(sb) = s.shared() else {
        return err_text("shared boards aren't on here.");
    };
    if let Some(on) = sb.board_of(&id) {
        return err_text(&format!("that card is already on {}.", on.label()));
    }
    let Some(t) = s.task(&id) else {
        return err_text(&format!(
            "no card {}.",
            crate::format::q(&one_line(&id, 80))
        ));
    };
    let me = c.sender.key.clone();
    match s.b.inbox().autonomy(&b.board_id).as_str() {
        inbox::autonomy::ASK => err_text(&format!(
            "your person's autonomy on {} is Ask me, which doesn't let agents share private cards. Ask them to use Share to board on {} in the app.",
            b.tag,
            t.short_ref()
        )),
        inbox::autonomy::AUTONOMOUS => {
            let shared = match sb.share(&id, &b.board_id, &me) {
                Ok(st) => st,
                Err(e) => return err_text(&format!("sharing failed: {e}")),
            };
            let text = format!(
                "{} shared {} to {} as {}, visible to {}.",
                s.roster().who(&me),
                t.short_ref(),
                b.label(),
                shared.short_ref(),
                b.people.join(", ")
            );
            let r = Request::Withdraw {
                agent: me,
                board: b.clone(),
                shared_id: shared.id.clone(),
                shared_ref: shared.short_ref(),
            };
            let notice = match s.request(
                c,
                &format!("Shared to {}", b.tag),
                &text,
                &[CHOICE_WITHDRAW],
                r,
            ) {
                Ok(_) => " Your person got a notice with Withdraw.",
                Err(_) => "",
            };
            format!(
                "Shared {} to {} as {} (task:{}), after scrub; the private card is closed and points at it. Work the shared card from now on.{notice}\n{}",
                t.short_ref(),
                b.label(),
                shared.short_ref(),
                shared.id,
                b.visible_to()
            )
        }
        _ => {
            let text = format!(
                "{} asks to share {} {} to {}, visible to {}. Only its title, objective, done-when, steps and links go, scrubbed.",
                s.roster().who(&me),
                t.short_ref(),
                crate::format::q(&one_line(&t.title, 200)),
                b.label(),
                b.people.join(", ")
            );
            let r = Request::Share {
                agent: me,
                board: b.clone(),
                task_id: id,
            };
            match s.request(
                c,
                &format!("Share to {}?", b.tag),
                &text,
                &[CHOICE_SHARE, CHOICE_REVIEW, CHOICE_NO],
                r,
            ) {
                Ok(rid) => format!(
                    "Asked your person ({rid}) with Share, Review and No: your autonomy on {} is Trusted. Their answer reaches you at your next turn; carry on meanwhile and don't ask again.",
                    b.tag
                ),
                Err(e) => e,
            }
        }
    }
}

// --- answers ---

/// Carries out a board request the person answered; false when the ask
/// wasn't one (the notify tool's answer handles it then).
pub(crate) fn request_resolved(s: &Inner, a: &coord::approvals::Ask) -> bool {
    let r = s
        .board_requests
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&a.id);
    let Some(r) = r else {
        return false;
    };
    let declined = |what: &str| -> String {
        if !a.answered {
            format!(
                "No answer within {} minutes: {what}",
                REQUEST_WAIT.as_secs() / 60
            )
        } else if a.choice.is_empty() {
            format!("They dismissed it: {what}")
        } else {
            format!("They chose {}: {what}", a.choice)
        }
    };
    let (agent, text) = match r {
        Request::Create {
            agent,
            board,
            task,
            assign_to,
        } => {
            let title = one_line(&task.title, 80);
            let text = if a.answered && a.choice == CHOICE_CREATE {
                match s.create_on(&agent, &board, *task, &assign_to) {
                    Ok(out) => format!("Your person said Create for {}. {out}", q(&title)),
                    Err(e) => format!(
                        "Your person said Create for {}, but it failed: {e}",
                        q(&title)
                    ),
                }
            } else {
                declined(&format!("no card {} on {}.", q(&title), board.tag))
            };
            (agent, text)
        }
        Request::Share {
            agent,
            board,
            task_id,
        } => {
            let card = s.short_of(&task_id);
            let text = match (a.answered, a.choice.as_str()) {
                (true, CHOICE_SHARE) => match s
                    .shared()
                    .map(|sb| sb.share(&task_id, &board.board_id, &agent))
                {
                    Some(Ok(t)) => format!(
                        "Your person said Share: {card} is on {} as {} (task:{}), and the private card is closed. Work the shared card from now on.\n{}",
                        board.label(),
                        t.short_ref(),
                        t.id,
                        board.visible_to()
                    ),
                    Some(Err(e)) => {
                        format!("Your person said Share for {card}, but it failed: {e}")
                    }
                    None => format!(
                        "Your person said Share for {card}, but shared boards are off here."
                    ),
                },
                (true, CHOICE_REVIEW) => format!(
                    "Your person chose Review: they will look at {card} in the app and share it from there if they want. Nothing is shared yet."
                ),
                _ => declined(&format!("{card} stays private.")),
            };
            (agent, text)
        }
        Request::Withdraw {
            agent,
            board,
            shared_id,
            shared_ref,
        } => {
            if !(a.answered && a.choice == CHOICE_WITHDRAW) {
                return true;
            }
            let text = match s.shared().map(|sb| sb.withdraw(&shared_id)) {
                Some(Ok(())) => format!(
                    "Your person withdrew {shared_ref} from {}: the private card is back and yours; the others on the board keep what they already have.",
                    board.label()
                ),
                Some(Err(e)) => {
                    format!("Your person chose Withdraw for {shared_ref}, but it failed: {e}")
                }
                None => return true,
            };
            (agent, text)
        }
    };
    crate::notify::notify_answer(
        s.b.inbox(),
        &inbox::Sender::new(&agent, "", ""),
        &a.id,
        &text,
    );
    true
}

fn q(s: &str) -> String {
    crate::format::q(s)
}

// --- replies to bridged messages ---

/// A reply to a message bridged from a board goes on the card, never as a
/// private message to another person (§7.1). `None` when the delivery
/// isn't a bridged one.
pub(crate) fn reply_on_card(s: &Inner, c: &Caller, did: &str, text: &str) -> Option<String> {
    let ib = s.b.inbox();
    let d = ib.get(did)?;
    if d.message.board.is_empty() || d.to != c.sender.key {
        return None;
    }
    let card = d
        .message
        .ref_
        .strip_prefix("task:")
        .unwrap_or("")
        .to_string();
    if card.is_empty() {
        return Some(err_text(
            "that message came from a shared board but names no card; answer on the card with board_comment.",
        ));
    }
    let b = s.shared().and_then(|sb| sb.board_of(&card));
    if let Some(b) = &b {
        let reasons = reasons_in(&[text]);
        if !reasons.is_empty() {
            return Some(refusal(b, &reasons));
        }
    }
    let (co, keys) = match s.on(&card, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return Some(map_err(e)),
    };
    let (cm, notice) = match co.comment_on(
        &keys[0],
        &card,
        coord::CommentInput {
            text: trim(text).into(),
            ..coord::CommentInput::default()
        },
    ) {
        Ok(v) => v,
        Err(e) => return Some(map_err(e)),
    };
    let n = s.roster();
    let told = s.told(&s.tell(&c.sender, notice), &n);
    if let Err(e) = ib.answered_on_card(
        &c.sender.key,
        did,
        &format!("answered on the card (comment:{})", cm.id),
    ) {
        return Some(map_err(e));
    }
    let mut out = format!(
        "Replied on {} (comment:{}): it came from a shared board, so your answer went on the card, not as a private message. Delivery {} is now answered.{told}",
        s.short_of(&card),
        cm.id,
        one_line(did, 80)
    );
    if let Some(b) = b {
        out += &format!("\n{}", b.visible_to());
    }
    Some(out)
}

/// A send to a board: key is refused: other people are reached on cards.
pub(crate) fn send_to_board(to: &str) -> Option<String> {
    if !trim(to).starts_with("board:") {
        return None;
    }
    Some(err_text(
        "that is another person or their agent on a shared board: say it on the card with board_comment (or reply to their message), where everyone on the board sees it.",
    ))
}

// --- board_review ---

const REVIEW_ARGS: &[(&str, Ty)] = &[("task_id", Ty::Str), ("pr", Ty::Str), ("to", Ty::Str)];

/// `to` as a board key: `agent:<cardId>`, the `board:<boardId>/<cardId>`
/// sender of a bridged message, or a bare cardId. A person is refused:
/// reviews are their agents' work (§10).
fn review_to(to: &str) -> Result<String, String> {
    let to = trim(to);
    if to.is_empty() {
        return Ok(String::new());
    }
    let card = match to.strip_prefix("board:") {
        Some(rest) => rest.split_once('/').map_or("", |(_, c)| c),
        None => to,
    };
    if card.starts_with(store::kinds::PERSON_ACTOR_PREFIX) || card.is_empty() {
        return Err(err_text(
            "to names another person's agent (agent:<cardId>, or the board:… key their message came from); a person isn't asked to review. Or leave to out for their dedicated reviewer.",
        ));
    }
    let card = card
        .strip_prefix(store::kinds::AGENT_ACTOR_PREFIX)
        .unwrap_or(card);
    Ok(format!("{}{card}", store::kinds::AGENT_ACTOR_PREFIX))
}

/// board_review (§10): asks another person's agent, or each other
/// person's dedicated reviewer, to review a PR a shared card links. The
/// review itself is the reviewer's own `gh`; Irori never posts reviews or
/// merges.
pub(crate) fn board_review(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, REVIEW_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = match s.task_id(a.str("task_id")) {
        Ok(id) => id,
        Err(m) => return m,
    };
    let pr = trim(a.str("pr"));
    if pr.is_empty() {
        return err_text("give pr: the pull request the card links, as board_link added it.");
    }
    let to = match review_to(a.str("to")) {
        Ok(t) => t,
        Err(e) => return e,
    };
    let Some(b) = s.shared().and_then(|sb| sb.board_of(&id)) else {
        return err_text(
            "that card is private: review requests go to another person's agents on a shared card. Ask your own agents with send.",
        );
    };
    let (co, as_) = match s.on(&id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let (rs, _) = match co.request_review(&as_[0], &id, pr, &to) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let asked: Vec<String> = rs.iter().map(|r| r.to.clone()).collect();
    format!(
        "Asked {} to review {} for {}: they hear at their next turn and review with their own gh (Irori never posts reviews or merges). Their review shows in prs once they post it.\n{}",
        asked.join(", "),
        one_line(pr, 200),
        s.short_of(&id),
        b.visible_to()
    )
}

#[cfg(test)]
mod review_tests {
    use super::review_to;

    #[test]
    fn review_to_takes_the_keys_an_agent_sees() {
        let want = Ok("agent:c0ffee".to_string());
        assert_eq!(review_to("agent:c0ffee"), want);
        assert_eq!(review_to("c0ffee"), want);
        assert_eq!(review_to("board:b1/c0ffee"), want);
        assert_eq!(review_to("board:b1/agent:c0ffee"), want);
        assert_eq!(review_to(" "), Ok(String::new()));
        assert!(review_to("board:b1/person:p2").is_err());
        assert!(review_to("board:b1").is_err());
    }
}
