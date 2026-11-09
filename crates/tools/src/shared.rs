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
    /// A board inside a space (card #392): its id there, while `board_id`
    /// is the space's (the log it lives in). Empty for a space's own board.
    pub space_board: String,
    /// The space's own name, which its first board's `name` may not be.
    /// Empty: the same as `name`.
    pub space_name: String,
    /// The board's description (card #690); may be empty.
    pub description: String,
    /// Closed: read-only and out of the lists, still found by its tag so
    /// a ref to its cards works (card #690).
    pub closed: bool,
}

impl BoardInfo {
    /// "Visible to: Stephen, Blake" (§7.3).
    pub fn visible_to(&self) -> String {
        format!("Visible to: {}", self.people.join(", "))
    }

    fn space(&self) -> &str {
        if self.space_name.is_empty() {
            &self.name
        } else {
            &self.space_name
        }
    }

    fn label(&self) -> String {
        let board = format!("{} ({})", one_line(&self.name, 80), self.tag);
        if self.space() == self.name {
            return board;
        }
        format!("{board} in {}", one_line(self.space(), 80))
    }
}

/// One of Personal's boards (card #558) as the tools show it: an agent
/// puts a private card on one by its tag (card #698).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersonalBoard {
    /// Its id: [`PERSONAL_DEFAULT`] for the default board.
    pub board_id: String,
    pub name: String,
    /// Its short handle; empty for the default, which has none.
    pub tag: String,
    /// Its description (card #690); empty with none.
    pub description: String,
    /// Closed: it takes no cards and isn't listed (card #690).
    pub closed: bool,
}

/// The id of Personal's default board, as Core names it
/// (`irori_core::PERSONAL_BOARD`), and the word that names it in `board`.
pub const PERSONAL_DEFAULT: &str = "personal";

impl PersonalBoard {
    fn is_default(&self) -> bool {
        self.tag.is_empty() || self.board_id == PERSONAL_DEFAULT
    }

    /// The board a card's row names: empty for the default, as the
    /// window's New card does.
    pub(crate) fn inner(&self) -> &str {
        if self.is_default() {
            ""
        } else {
            &self.board_id
        }
    }

    pub(crate) fn label(&self) -> String {
        if self.is_default() {
            return format!(
                "{}, Personal's default board",
                crate::format::q(&one_line(&self.name, 80))
            );
        }
        format!(
            "{} ({}), one of Personal's boards",
            crate::format::q(&one_line(&self.name, 80)),
            self.tag
        )
    }
}

/// Where a `board` argument points (card #698).
pub(crate) enum BoardTarget {
    /// One of Personal's boards: the card stays private.
    Personal(PersonalBoard),
    /// A board in a shared space.
    Space(BoardInfo),
}

/// Someone this person can DM (card #538).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersonInfo {
    /// `person:<personId>`.
    pub key: String,
    pub name: String,
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
    /// Share to a space (§5.2) without the window's preview: scrubbed, the
    /// private card closed and pointing at the copy. The copy goes on
    /// board `space_board` inside space `board_id`, numbered there; empty
    /// is the space's first board (card #558). `by` is who asked.
    fn share(
        &self,
        task_id: &str,
        board_id: &str,
        space_board: &str,
        by: &str,
    ) -> Result<Task, String>;
    /// Withdraw a shared card (§5.3) for the person.
    fn withdraw(&self, task_id: &str) -> Result<(), String>;
    /// Who an agent's @name on board `board_id`'s cards may also name
    /// (card #186): the other members, as `person:<id>` by their profile
    /// name, and their agents' cards, as `agent:<cardId>`. This person's
    /// own agents are in the roster already. The default is nobody.
    fn board_mentionables(&self, _board_id: &str) -> Vec<coord::roster::AgentCard> {
        Vec::new()
    }
    /// The people this person can DM (card #538): everyone active in a
    /// space they're active in, hidden DM spaces aside. The default is
    /// nobody.
    fn dm_people(&self) -> Vec<PersonInfo> {
        Vec::new()
    }
    /// What the DM with `other` (`person:<id>`) is waiting on:
    /// [`crate::talk::WAITING_FOR_THEM`], [`crate::talk::WAITING_FOR_UPDATE`],
    /// or "" when nothing is on its way to them.
    fn dm_waiting(&self, _other: &str) -> &'static str {
        ""
    }
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
    "board_move",
    "board_assign",
    "board_link",
    "board_edit",
    "board_recommend",
    "board_split",
    "board_watch",
    "board_unwatch",
    "board_later",
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
    "after",
    "not_after",
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

/// How much of a board's description the board tool's list shows.
const DESCRIPTION_CHARS: usize = 120;

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

/// Whether `v` (no `#`) names board `b`: its id or its tag.
fn names_board(b: &BoardInfo, v: &str) -> bool {
    (b.space_board.is_empty() && b.board_id == v)
        || (!b.space_board.is_empty() && b.space_board == v)
        || b.tag.eq_ignore_ascii_case(v)
}

/// The id that names board `b` alone: its own inside a space, else the
/// space's.
fn inner_id(b: &BoardInfo) -> &str {
    if b.space_board.is_empty() {
        &b.board_id
    } else {
        &b.space_board
    }
}

impl Inner {
    fn shared(&self) -> Option<&dyn SharedBoards> {
        self.b.shared_boards()
    }

    /// The Personal board `v` names by its tag or id (card #698); None:
    /// none of Personal's boards past its default.
    fn personal_board(&self, v: &str) -> Option<PersonalBoard> {
        let v = trim(v).trim_start_matches('#');
        if v.is_empty() {
            return None;
        }
        self.b
            .personal_boards()
            .into_iter()
            .find(|b| !b.is_default() && (b.board_id == v || b.tag.eq_ignore_ascii_case(v)))
    }

    /// Personal's default board, which `personal` names.
    fn personal_default(&self) -> PersonalBoard {
        self.b
            .personal_boards()
            .into_iter()
            .find(PersonalBoard::is_default)
            .unwrap_or_else(|| PersonalBoard {
                board_id: PERSONAL_DEFAULT.into(),
                name: "Personal".into(),
                ..Default::default()
            })
    }

    /// The board a call's `board` names (card #698): one of Personal's or
    /// one in a shared space, by tag or id. A tag both have is refused.
    /// `personal` is Personal's default unless a board has it as its tag.
    pub(crate) fn resolve_board(&self, v: &str) -> Result<BoardTarget, String> {
        let key = trim(v).trim_start_matches('#');
        let spaces: Vec<BoardInfo> = self
            .shared()
            .map(|sb| sb.boards())
            .unwrap_or_default()
            .into_iter()
            .filter(|b| names_board(b, key))
            .collect();
        match self.personal_board(v) {
            Some(p) if !spaces.is_empty() => Err(err_text(&format!(
                "more than one board has the tag {}: give its boardId ({} on Personal, {}).",
                crate::format::q(key),
                p.board_id,
                spaces
                    .iter()
                    .map(|b| format!("{} on {}", inner_id(b), b.label()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
            Some(p) => Ok(BoardTarget::Personal(p)),
            None if !spaces.is_empty() => self.find_board(v).map(BoardTarget::Space),
            None if key.eq_ignore_ascii_case(PERSONAL_DEFAULT) => {
                Ok(BoardTarget::Personal(self.personal_default()))
            }
            None => Err(format!(
                "{}{}",
                self.find_board(v).err().unwrap_or_default(),
                self.personal_tags()
            )),
        }
    }

    /// " Personal's boards: hw, bc." when Personal has more than its
    /// default, for a refusal to list.
    fn personal_tags(&self) -> String {
        let tags: Vec<String> = self
            .b
            .personal_boards()
            .into_iter()
            .filter(|b| !b.is_default() && !b.closed)
            .map(|b| b.tag)
            .collect();
        if tags.is_empty() {
            return String::new();
        }
        format!(" Personal's boards: {}.", tags.join(", "))
    }

    /// The board `v` names in a shared space, by tag or boardId.
    pub(crate) fn find_board(&self, v: &str) -> Result<BoardInfo, String> {
        let Some(sb) = self.shared() else {
            return Err(err_text(&format!(
                "no board {}: shared spaces aren't on here; leave out board for a private card.",
                crate::format::q(trim(v).trim_start_matches('#'))
            )));
        };
        let v = trim(v).trim_start_matches('#');
        let all = sb.boards();
        let hits: Vec<&BoardInfo> = all.iter().filter(|b| names_board(b, v)).collect();
        match hits.as_slice() {
            [one] => Ok((*one).clone()),
            [] => {
                let list: Vec<String> = all
                    .iter()
                    .filter(|b| !b.closed)
                    .map(|b| b.tag.clone())
                    .collect();
                Err(err_text(&format!(
                    "no board {} in a shared space. {}",
                    crate::format::q(v),
                    if list.is_empty() {
                        "You aren't in any shared space.".to_string()
                    } else {
                        format!("Yours: {}.", list.join(", "))
                    }
                )))
            }
            _ => Err(err_text(&format!(
                "more than one board has the tag {}: give its boardId ({}).",
                crate::format::q(v),
                hits.iter()
                    .map(|b| inner_id(b))
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
        if name == "board" {
            let out = run();
            return match self.boards_section() {
                Some(s) => format!("{out}\n{s}"),
                None => out,
            };
        }
        if self.shared().is_none() {
            return run();
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

    /// The board tool's list of boards to post on: Personal's, when it has
    /// more than its default (card #698), then the spaces.
    fn boards_section(&self) -> Option<String> {
        let parts: Vec<String> = [self.personal_section(), self.spaces_section()]
            .into_iter()
            .flatten()
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n"))
    }

    /// Personal's boards past its default, private like every Personal card.
    fn personal_section(&self) -> Option<String> {
        let all = self.b.personal_boards();
        let boards: Vec<String> = all
            .iter()
            // A closed board takes no cards, so it isn't listed (card #690).
            .filter(|b| !b.is_default() && !b.closed)
            .map(|b| {
                let mut line = format!("{} {}", b.tag, crate::format::q(&one_line(&b.name, 80)));
                if !b.description.trim().is_empty() {
                    line.push_str(&format!(
                        ": {}",
                        crate::format::q(&one_line(&b.description, DESCRIPTION_CHARS))
                    ));
                }
                line
            })
            .collect();
        if boards.is_empty() {
            return None;
        }
        let default = all
            .iter()
            .find(|b| b.is_default())
            .map_or("Personal", |b| b.name.as_str());
        Some(format!(
            "Personal's boards (board_create board=<board tag>, or board_share one of your person's private cards onto one; private: only your person sees them): {}; leave board out for its default, {}.",
            boards.join(", "),
            crate::format::q(&one_line(default, 80))
        ))
    }

    /// The spaces, each with who sees it, the agents' autonomy there and
    /// its boards (card #558).
    fn spaces_section(&self) -> Option<String> {
        let all = self.shared()?.boards();
        if all.is_empty() {
            return None;
        }
        let mut s = String::from(
            "Spaces you may post on (board_create board=<board tag>; anything there is seen by everyone in the space):",
        );
        // A closed board takes no cards, so it isn't listed (card #690).
        let live: Vec<&BoardInfo> = all.iter().filter(|b| !b.closed).collect();
        let mut seen: Vec<&str> = Vec::new();
        for b in &live {
            if seen.contains(&b.board_id.as_str()) {
                continue;
            }
            seen.push(&b.board_id);
            let here: Vec<&&BoardInfo> = live.iter().filter(|x| x.board_id == b.board_id).collect();
            let boards: Vec<String> = here
                .iter()
                .map(|x| format!("{} {}", x.tag, crate::format::q(&one_line(&x.name, 80))))
                .collect();
            s += &format!(
                "\n- {}: {}; your autonomy there: {}\n  boards: {}",
                crate::format::q(&one_line(b.space(), 80)),
                b.people.join(", "),
                self.b.inbox().autonomy(&b.board_id),
                boards.join(", ")
            );
            // Each board's description, so a card goes where it fits.
            for x in here.iter().filter(|x| !x.description.trim().is_empty()) {
                s += &format!(
                    "\n  {}: {}",
                    x.tag,
                    crate::format::q(&one_line(&x.description, DESCRIPTION_CHARS))
                );
            }
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
        let sb = self.shared().ok_or("shared spaces aren't on here")?;
        let (co, keys) = sb
            .board_coord(&b.board_id, &[agent.to_string(), assign_to.to_string()])
            .map_err(|e| map_err(e).trim_start_matches("Error: ").to_string())?;
        // Given to a member: made and given in one step, so no sweep sees
        // it unheld and starts its author's agent first (card #573). A card
        // held from the start isn't given (card #570): nobody takes it yet.
        let (t, given) =
            if task.later.is_none() && keys[1].starts_with(store::kinds::PERSON_ACTOR_PREFIX) {
                let (t, n) = co
                    .create_and_assign(&keys[0], task, &keys[1])
                    .map_err(|e| map_err(e).trim_start_matches("Error: ").to_string())?;
                (t, Some(n))
            } else {
                let t = co
                    .create_task(&keys[0], task)
                    .map_err(|e| map_err(e).trim_start_matches("Error: ").to_string())?;
                (t, None)
            };
        let n = self.roster();
        let mut out = format!(
            "Created {} (task:{}) on {}.",
            t.short_ref(),
            t.id,
            b.label()
        );
        // Card #570: a card that waits from the start isn't taken now.
        let waits = crate::board::waits_from_start(self, &t);
        if !waits.is_empty() {
            out += &waits;
            if !assign_to.is_empty() && t.parked().is_some() {
                out += " It isn't assigned: Irori hands it out once it comes back.";
                return Ok(format!("{out}\n{}", b.visible_to()));
            }
        }
        if assign_to.is_empty() {
            if waits.is_empty() {
                out += " Take it up with board_join, or hand it over with board_assign.";
            }
        } else {
            let assigned = match given {
                Some(notice) => Ok((t.clone(), notice)),
                None => co.assign(&keys[0], &t.id, &keys[1]),
            };
            match assigned {
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_on_board(
    s: &Inner,
    c: &Caller,
    a: &crate::args::Args,
    board: &str,
    repo: String,
    assign_to: String,
    waits: Option<(String, coord::BlockedBy)>,
    after: Vec<String>,
    later: crate::board::NewLater,
) -> String {
    let b = match s.find_board(board) {
        Ok(b) => b,
        Err(e) => return e,
    };
    if b.closed {
        return err_text(&format!(
            "{} is closed, so it takes no new cards. Reopen it in Irori first, or pick another board (the board tool lists them).",
            b.label()
        ));
    }
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
        block_reason: match &later {
            Some((_, why)) => why.clone(),
            None => waits.as_ref().map(|w| w.0.clone()).unwrap_or_default(),
        },
        blocked_by: waits.map(|w| w.1),
        board: b.space_board.clone(),
        board_named: true,
        after,
        later: later.map(|l| l.0),
        weight: crate::board::new_weight(a).0,
        weight_reason: crate::board::new_weight(a).1,
        ..NewTask::default()
    };
    let mut texts: Vec<&str> = vec![
        &task.title,
        &task.objective,
        &task.done_criteria,
        &task.block_reason,
        &task.weight_reason,
    ];
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

// --- talk_thread in a space ---

/// talk_thread in space `space`'s own log (card #631): scrubbed like any
/// write there, and only where the agent may write without asking. The
/// agents it names, by the log's keys, are woken; this person's own hear
/// it from the returned notice, the others' from their devices' bridge.
pub(crate) fn thread_on_board(
    s: &Inner,
    c: &Caller,
    space: &str,
    text: &str,
    given: &[String],
    channel: &str,
) -> Result<(Task, String), String> {
    let b = s.find_board(space)?;
    let reasons = reasons_in(&[text]);
    if !reasons.is_empty() {
        return Err(refusal(&b, &reasons));
    }
    if s.b.inbox().autonomy(&b.board_id) == inbox::autonomy::ASK {
        return Err(err_text(&format!(
            "your autonomy on {} is Ask me, so you can't open a thread there yourself: open it in Personal (leave out space), or ask your person.",
            b.label()
        )));
    }
    let sb = s
        .shared()
        .ok_or_else(|| err_text("shared spaces aren't on here"))?;
    let mut agents = crate::mention::cards(s);
    agents.extend(sb.board_mentionables(&b.board_id));
    let res = coord::mentions::resolve(&agents, &c.sender.key, text, given);
    let mut keys = vec![c.sender.key.clone()];
    keys.extend(res.mentions.iter().map(|m| m.key.clone()));
    let (co, logk) = sb
        .board_coord(&b.board_id, &keys)
        .map_err(|e| err_text(map_err(e).trim_start_matches("Error: ")))?;
    let mentions = res
        .mentions
        .iter()
        .zip(logk.iter().skip(1))
        .map(|(m, k)| coord::Mention {
            key: k.clone(),
            ..m.clone()
        })
        .collect();
    let (t, n) = co
        .post_agent_thread(
            &logk[0],
            coord::NewAsk {
                text: text.to_string(),
                thread: true,
                mentions,
                channel: channel.to_string(),
                ..coord::NewAsk::default()
            },
        )
        .map_err(|e| err_text(map_err(e).trim_start_matches("Error: ")))?;
    let names = s.roster();
    let mut out = String::new();
    if let Some(n) = n {
        out += &s.told(&s.tell(&c.sender, n), &names);
    }
    out += &crate::mention::mention_line(&[], &res.misses, &names);
    out += &format!("\n{}", b.visible_to());
    Ok((t, out))
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
        return err_text(
            "say which shared space, or which of Personal's boards: board = its tag (the board tool lists them).",
        );
    }
    let b = match s.resolve_board(a.str("board")) {
        Ok(BoardTarget::Space(b)) => b,
        Ok(BoardTarget::Personal(p)) => return move_to_personal(s, c, &id, &p),
        Err(e) => return e,
    };
    let Some(sb) = s.shared() else {
        return err_text("shared spaces aren't on here.");
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
            "your person's autonomy on {} is Ask me, which doesn't let agents share private cards. Ask them to use Share to a space on {} in the app.",
            b.tag,
            t.short_ref()
        )),
        inbox::autonomy::AUTONOMOUS => {
            let shared = match sb.share(&id, &b.board_id, &b.space_board, &me) {
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

/// board_share onto one of Personal's boards (card #698): a private card
/// moves there, with its subtasks, and takes the next number. It stays
/// private, so nobody is asked.
fn move_to_personal(s: &Inner, c: &Caller, id: &str, p: &PersonalBoard) -> String {
    let (co, keys) = match s.on(id, std::slice::from_ref(&c.sender.key)) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    if !Arc::ptr_eq(&co, s.b.coord()) {
        let on = s
            .shared()
            .and_then(|sb| sb.board_of(id))
            .map_or_else(|| "a shared space".to_string(), |b| b.label());
        return err_text(&format!(
            "that card is on {on}: a card in a shared space stays there. Personal's boards take your person's private cards."
        ));
    }
    let Some(t) = co.get_task(id) else {
        return err_text(&format!("no card {}.", crate::format::q(&one_line(id, 80))));
    };
    if t.board == p.inner() {
        return format!("{} is already on {}.", t.short_ref(), p.label());
    }
    match co.move_task(&keys[0], id, p.inner()) {
        Ok(moved) => format!(
            "Moved {} to {} as {} (task:{}); its old ref still finds it. It stays private: only your person sees it.",
            t.short_ref(),
            p.label(),
            moved.short_ref(),
            moved.id
        ),
        Err(e) => map_err(e),
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
                    .map(|sb| sb.share(&task_id, &board.board_id, &board.space_board, &agent))
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
                        "Your person said Share for {card}, but shared spaces are off here."
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
                    "Your person withdrew {shared_ref} from {}: the private card is back and yours; the others in the space keep what they already have.",
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

/// Writes `text` as a comment on shared card `card` for the caller, after
/// scrub and the stricter pass, and tells its holders (§7.1). The comment
/// and what it said to whom, or the refusal.
fn comment_on_card(
    s: &Inner,
    c: &Caller,
    card: &str,
    text: &str,
) -> Result<(String, String), String> {
    let b = s.shared().and_then(|sb| sb.board_of(card));
    if let Some(b) = &b {
        let reasons = reasons_in(&[text]);
        if !reasons.is_empty() {
            return Err(refusal(b, &reasons));
        }
    }
    let (co, keys) = s
        .on(card, std::slice::from_ref(&c.sender.key))
        .map_err(map_err)?;
    let (cm, notice) = co
        .comment_on(
            &keys[0],
            card,
            coord::CommentInput {
                text: trim(text).into(),
                ..coord::CommentInput::default()
            },
        )
        .map_err(map_err)?;
    let notice = notice.into_iter().next().unwrap_or_default();
    let n = s.roster();
    let mut told = s.told(&s.tell(&c.sender, notice), &n);
    if let Some(b) = b {
        told += &format!("\n{}", b.visible_to());
    }
    Ok((cm.id, told))
}

/// The card a bridged delivery is about (`""` for none).
fn card_of(d: &inbox::Delivery) -> String {
    d.message
        .ref_
        .strip_prefix("task:")
        .unwrap_or("")
        .to_string()
}

/// A reply to a message bridged from a board goes on the card, never as a
/// private message to another person (§7.1). `None` when the delivery
/// isn't a bridged one.
pub(crate) fn reply_on_card(s: &Inner, c: &Caller, did: &str, text: &str) -> Option<String> {
    let ib = s.b.inbox();
    let d = ib.get(did)?;
    if d.message.board.is_empty() || d.to != c.sender.key {
        return None;
    }
    if let Some(conv) = d.message.ref_.strip_prefix("talk:")
        && coord::talk::is_channel(conv)
    {
        return Some(reply_in_channel(s, c, did, conv, text));
    }
    let card = card_of(&d);
    if card.is_empty() {
        return Some(err_text(
            "that message came from a shared space but names no card; answer on the card with board_comment.",
        ));
    }
    let (cm, told) = match comment_on_card(s, c, &card, text) {
        Ok(v) => v,
        Err(e) => return Some(e),
    };
    if let Err(e) = ib.answered_on_card(
        &c.sender.key,
        did,
        &format!("answered on the card (comment:{cm})"),
    ) {
        return Some(map_err(e));
    }
    let (rest, visible) = split_visible(&told);
    let mut out = format!(
        "Replied on {} (comment:{cm}): it came from a shared space, so your answer went on the card, not as a private message. Delivery {} is now answered.{rest}",
        s.short_of(&card),
        one_line(did, 80)
    );
    out += visible;
    Some(out)
}

/// A reply to a space channel's mention bridged from another person (card
/// #461): it goes in the channel, where they read it, and the delivery is
/// answered.
fn reply_in_channel(s: &Inner, c: &Caller, did: &str, conv: &str, text: &str) -> String {
    let target = crate::talk::Target::Channel(conv.to_string());
    let p = match crate::talk::post(s.b.as_ref(), &c.sender, &target, text, &[], true, None, &[]) {
        Ok(p) => p,
        Err(e) => return err_text(&e),
    };
    let post = &p.post.post_id;
    let name =
        s.b.talk()
            .and_then(|t| t.channel(conv))
            .map(|ch| format!("#{}", ch.name))
            .unwrap_or_else(|| conv.to_string());
    let ib = s.b.inbox();
    if let Err(e) = ib.answered_on_card(
        &c.sender.key,
        did,
        &format!("answered in {name} (post:{post})"),
    ) {
        return map_err(e);
    }
    let n = s.roster();
    let got = crate::talk::outcomes(&p);
    format!(
        "Replied in {name} (post:{post}): it came from a space's channel, so your answer went there, not as a private message. Delivery {} is now answered.{}",
        one_line(did, 80),
        crate::mention::mention_line(&got, &p.misses, &n)
    )
}

/// told without its Visible-to line, and that line (with its newline).
fn split_visible(told: &str) -> (&str, &str) {
    match told.find("\nVisible to: ") {
        Some(i) => (&told[..i], &told[i..]),
        None => (told, ""),
    }
}

/// A send to another person or their agent on a shared board (a board:
/// key) becomes a comment on the card, never a private message (§7.1): the
/// card task_id names, else the one their newest message to the caller was
/// about. `None` when `to` isn't a board: key.
pub(crate) fn send_to_board(
    s: &Inner,
    c: &Caller,
    to: &str,
    task_id: &str,
    text: &str,
) -> Option<String> {
    let to = trim(to);
    if !to.starts_with("board:") {
        return None;
    }
    if trim(text).is_empty() {
        return Some(err_text("give the message text."));
    }
    let board = to
        .strip_prefix("board:")
        .and_then(|r| r.split('/').next())
        .unwrap_or("");
    let mut card = String::new();
    if !trim(task_id).is_empty() {
        card = match s.task_id(task_id) {
            Ok(id) => id,
            Err(msg) => return Some(msg),
        };
    } else if let Some(d) =
        s.b.inbox()
            .for_agent(&c.sender.key, true, 0)
            .into_iter()
            .find(|d| d.message.from.key == to && !card_of(d).is_empty())
    {
        card = card_of(&d);
    }
    if card.is_empty() {
        return Some(err_text(
            "that is another person or their agent in a shared space, and nothing says which card: give task_id, the card to say it on. Everyone in the space sees it there.",
        ));
    }
    let on = s.shared().and_then(|sb| sb.board_of(&card));
    if !on.as_ref().is_some_and(|b| b.board_id == board) {
        return Some(err_text(&format!(
            "{} isn't in a space with that person: a message to another person goes on a card of the space you share with them.",
            s.short_of(&card)
        )));
    }
    match comment_on_card(s, c, &card, text) {
        Ok((cm, told)) => {
            let (rest, visible) = split_visible(&told);
            Some(format!(
                "Said on {} (comment:{cm}): {} is another person or their agent in a shared space, so it went on the card, not as a private message.{rest}{visible}",
                s.short_of(&card),
                one_line(to, 120)
            ))
        }
        Err(e) => Some(e),
    }
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
/// merges. On a private card (#88) it asks one of the person's own agents
/// to review the card's work, PR or not.
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
    if s.shared().and_then(|sb| sb.board_of(&id)).is_none() {
        return own_review(s, c, &id, pr, trim(a.str("to")));
    }
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
    // Merge desk §12.5: a PR with a PR card gets its reviewer from the
    // author's device, so nothing was asked.
    if rs.is_empty() {
        return format!(
            "Irori picks this PR's reviewer from the PR author's device; to review it yourself, run review_claim pr={}",
            one_line(pr, 200)
        );
    }
    let asked: Vec<String> = rs.iter().map(|r| r.to.clone()).collect();
    format!(
        "Asked {} to review {} for {}: an idle agent is woken now, a busy one hears at its next turn, and if their person has Ask me on, their person is asked first. They review with their own gh (Irori never posts reviews or merges). Their review shows in prs once they post it.\n{}",
        asked.join(", "),
        one_line(pr, 200),
        s.short_of(&id),
        b.visible_to()
    )
}

/// board_review on a private card: `to` names one of the person's agents,
/// who is woken once and answers with board_comment verdict=….
fn own_review(s: &Inner, c: &Caller, id: &str, pr: &str, to: &str) -> String {
    if to.is_empty() {
        return err_text(
            "give to: the agent to review it, by name or key (see the agents tool). On a private card nobody reviews by default.",
        );
    }
    let to = match s.assignee(to) {
        Ok(k) => k,
        Err(e) => return map_err(e),
    };
    let (co, as_) = match s.on(id, &[c.sender.key.clone(), to.clone()]) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let (_, notice) = match co.request_review(&as_[0], id, pr, &as_[1]) {
        Ok(v) => v,
        Err(e) => return map_err(e),
    };
    let n = s.roster();
    format!(
        "Asked {} to review {}{}: the card shows the review as requested until they answer with board_comment verdict=approved or verdict=changes. You are woken only if they ask for changes.{}",
        n.who(&to),
        s.short_of(id),
        if pr.is_empty() {
            String::new()
        } else {
            format!(" ({})", one_line(pr, 200))
        },
        s.told(&s.tell(&c.sender, notice), &n)
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
