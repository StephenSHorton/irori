//! The bridge from a board log into this device's inbox (v0.5 §7.1), and
//! the Core side of the tools' shared boards (§7.3).
//!
//! A row another person's device wrote on a board becomes a private
//! message.sent from a `board:` sender, through [`inbox::Inbox::bridge`],
//! which keeps only the agents this device hosts, applies the person's
//! receive setting (§7.2, B2) and never bridges one row to one agent
//! twice. What crosses:
//! - a comment, an assignment or a blocker change, to the card's holders,
//!   watchers and assignees (the assignee alone for an assignment);
//! - a board ask (§9): the asking device's `board.ask.state` telling free
//!   agents, to those of them this device hosts, once per round;
//! - a review request (§10), to the agent it names or, without one, to
//!   this person's dedicated reviewer for the card's repo or the board;
//! - a lease release request (§5.4), to the lease's holder only.
//!
//! The person's own rows don't cross here: the tool that wrote one already
//! routed its notice. Rows met on a resync aren't bridged either, so a
//! re-fold doesn't replay old talk.

use std::sync::Arc;

use coord::{Coord, RELEASE_REQUEST, Task};
use inbox::{BoardFrom, BridgeRow, row_kind};
use serde::Deserialize;
use store::Event;
use store::kinds::{self, AGENT_ACTOR_PREFIX, PERSON_ACTOR_PREFIX};

use super::Services;
use super::boards::BoardLog;
use crate::logf;

/// The fields of a board row the bridge reads; each kind fills some.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct RowFields {
    task_id: String,
    text: String,
    wake: bool,
    to: String,
    answer: String,
    note: String,
    pr: String,
    blocker: Option<serde_json::Value>,
    // board.ask.state
    taker: String,
    was: String,
    reason: String,
    heard: Vec<String>,
    since: String,
}

/// The message text of a plan, given how the sender shows.
type TextFn = Box<dyn FnOnce(&str) -> String>;

/// What one board row says to this device's agents, before the inbox's
/// per-agent checks.
struct Plan {
    kind: &'static str,
    /// The writing agent's card; empty when the person wrote it.
    card_id: String,
    /// Board keys of the agents it is for.
    to: Vec<String>,
    /// The row the inbox dedupes on: `<deviceId>/<seq>`, or an ask's round.
    row: String,
    wake: bool,
    text: TextFn,
}

/// The take reasons that tell an ask's free agents (coord's asks): posted,
/// asked again, given back, its taker gone, or still unclaimed.
const ASK_ROUND: &[&str] = &["posted", "reopened", "released", "gone", "unclaimed"];

impl Services {
    /// Bridges one board row, live from `b`'s follower.
    pub(crate) fn bridge_row(&self, b: &BoardLog, ev: &Event) {
        if ev.seq == 0 {
            return;
        }
        let bridged = matches!(
            ev.kind.as_str(),
            kinds::TASK_COMMENT
                | kinds::TASK_ASSIGNED
                | kinds::BLOCKER_CLOSED
                | kinds::BLOCKER_OPTION
                | kinds::BLOCKER_RESOLVED
                | kinds::TASK_STATUS
                | kinds::REVIEW_REQUESTED
                | kinds::BOARD_ASK_STATE
        );
        if !bridged {
            return;
        }
        let Ok(f) = serde_json::from_slice::<RowFields>(&ev.payload) else {
            return;
        };
        if f.task_id.is_empty() {
            return;
        }
        let me = self.person_id();
        let Some(person) = b.verifier.person_of(&ev.device_id) else {
            return;
        };
        if person.is_empty() || person == me {
            return;
        }
        let co = b.coord();
        // The coord folds on its own thread: a card that arrived with this
        // row (an ask's task.created and its first board.ask.state) may not
        // have folded yet, so fold what the log holds before giving up.
        // The coord thread then skips those rows (apply_remote folds a
        // device's rows past its last only).
        let Some(t) = co
            .get_task(&f.task_id)
            .or_else(|| co.catch_up().ok().and_then(|()| co.get_task(&f.task_id)))
        else {
            return;
        };
        let plan = if ev.kind == kinds::BOARD_ASK_STATE {
            self.ask_plan(b, ev, f, &t)
        } else {
            row_plan(co, &me, ev, f, &t)
        };
        let Some(p) = plan else {
            return;
        };
        let to: Vec<String> =
            p.to.iter()
                .filter(|k| k.starts_with(AGENT_ACTOR_PREFIX))
                .map(|k| self.private_key(k))
                .collect();
        if to.is_empty() {
            return;
        }
        let who = self.board_person_name(b, &person);
        let name = if p.card_id.is_empty() {
            who.clone()
        } else {
            let card = super::boards::lock(&b.maps)
                .cards
                .get(&p.card_id)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            if card.is_empty() {
                format!("{who}'s agent")
            } else {
                format!("{who}'s agent {card}")
            }
        };
        let text = (p.text)(&name);
        let tag = b.verifier.membership().tag;
        let r = BridgeRow {
            from: BoardFrom {
                board_id: b.id.clone(),
                tag,
                person_id: person,
                card_id: p.card_id,
                row: p.row,
                kind: p.kind.into(),
                wake: p.wake,
            },
            name,
            to,
            text,
            ref_: format!("task:{}", t.id),
            hop: coord::row_hop(&ev.payload),
            attachments: vec![],
        };
        if let Err(e) = self.inbox.bridge(&r) {
            logf(format_args!("board {}: bridge {}: {e}", b.id, r.from.row));
        }
    }

    /// A board ask's round of telling (§9): the asking device's
    /// board.ask.state naming the free agents it told. A round (its
    /// `since`) reaches an agent once, however often a sweep widens it. A
    /// take, a give-back or a gone taker tells nobody here: the claimer's
    /// own device answers its claim.
    fn ask_plan(&self, b: &BoardLog, ev: &Event, f: RowFields, t: &Task) -> Option<Plan> {
        let a = t.ask.as_ref()?;
        if ev.device_id != a.owner
            || a.outcome.is_some()
            || !f.taker.is_empty()
            || !f.was.is_empty()
            || f.heard.is_empty()
            || f.since.is_empty()
            || !ASK_ROUND.contains(&f.reason.as_str())
        {
            return None;
        }
        // As the asker's own sweep wakes: only while the round is young.
        let young = gocompat::time::GoTime::parse_rfc3339(&f.since)
            .and_then(|since| {
                (self.now)()
                    .to_system()
                    .duration_since(since.to_system())
                    .ok()
            })
            .is_none_or(|age| age < coord::ASK_UNCLAIMED);
        let wake = young && (f.reason == "reopened" || f.reason == "posted" && a.wake);
        let why = match f.reason.as_str() {
            "reopened" => ", again: the person wants it picked up",
            "released" => ", again: the agent that took it gave it back",
            "gone" => ", again: the agent that took it went away",
            _ => "",
        };
        let ask_me = self.inbox.autonomy(&b.id) == inbox::autonomy::ASK;
        let (refs, short, objective) = (t.refs(), t.short_ref(), t.objective.clone());
        Some(Plan {
            kind: row_kind::ASK,
            card_id: String::new(),
            to: f.heard,
            row: format!("ask:{}@{}", t.id, f.since),
            wake,
            text: Box::new(move |name| {
                let mut s = format!(
                    "{name} asks the room ({refs}){why}:\n{objective}\nIf you can take it on now, ask_claim {short}: one agent takes it, and you hear if someone else got there first. If you're busy, leave it."
                );
                if ask_me {
                    s.push_str(" Your autonomy on this board is Ask me: a new card from it is a request to your person first.");
                }
                s
            }),
        })
    }

    /// The hop a board row an agent writes now carries (coord's `hop_of`):
    /// its running turn's, by the agent's private key.
    pub(crate) fn board_hop(&self, actor: &str) -> i64 {
        if !actor.starts_with(AGENT_ACTOR_PREFIX) {
            return 0;
        }
        self.inbox.current_hop(&self.private_key(actor))
    }

    fn board_person_name(&self, b: &BoardLog, person: &str) -> String {
        let mut name = b.verifier.profile(person).unwrap_or_default();
        if name.is_empty()
            && let Some(p) = b.verifier.membership().person(person)
        {
            name = p.name.clone();
        }
        if name.is_empty() {
            "another person".into()
        } else {
            name
        }
    }

    /// The tools' view of one board.
    fn tools_board(&self, s: super::boards::BoardSummary) -> tools::BoardInfo {
        tools::BoardInfo {
            board_id: s.board_id,
            tag: s.tag,
            name: s.name,
            people: s.people,
        }
    }
}

/// A card row's plan: a comment, an assignment, a blocker change, a review
/// request or a release request.
fn row_plan(co: &Coord, me: &str, ev: &Event, f: RowFields, t: &Task) -> Option<Plan> {
    let release =
        ev.kind == kinds::TASK_COMMENT && !f.to.is_empty() && f.text.starts_with(RELEASE_REQUEST);
    let kind = match ev.kind.as_str() {
        kinds::TASK_COMMENT if release => row_kind::LEASE,
        kinds::TASK_COMMENT => row_kind::COMMENT,
        kinds::TASK_ASSIGNED | kinds::REVIEW_REQUESTED => row_kind::ASSIGN,
        kinds::TASK_STATUS if f.blocker.is_none() => return None,
        _ => row_kind::BLOCKER,
    };
    let card_id = ev
        .actor
        .strip_prefix(AGENT_ACTOR_PREFIX)
        .unwrap_or_default()
        .to_string();
    if card_id.is_empty() && !ev.actor.starts_with(PERSON_ACTOR_PREFIX) {
        return None;
    }
    let to: Vec<String> = if release || ev.kind == kinds::TASK_ASSIGNED {
        vec![f.to.clone()]
    } else if ev.kind == kinds::REVIEW_REQUESTED {
        // Without `to` it goes to this person's dedicated reviewer, for
        // the card's repo or else the board (§10).
        if f.to.is_empty() {
            vec![co.reviewer(me, &t.repo_key)]
        } else {
            vec![f.to.clone()]
        }
    } else {
        let mut v = t.holders.clone();
        v.extend(t.watchers.iter().cloned());
        v.extend(t.assigned.iter().cloned());
        v
    };
    let title = gocompat::quote::quote(&t.title);
    let short = t.short_ref();
    let k = ev.kind.clone();
    Some(Plan {
        kind,
        card_id,
        to,
        row: format!("{}/{}", ev.device_id, ev.seq),
        wake: f.wake,
        text: Box::new(move |name| match k.as_str() {
            kinds::TASK_COMMENT => f.text,
            kinds::TASK_ASSIGNED => format!("{name} assigned you {title}."),
            kinds::REVIEW_REQUESTED => format!(
                "{name} asks you to review {} for {title} ({short}). Review it with your own gh; Irori never posts reviews or merges.",
                f.pr
            ),
            kinds::BLOCKER_OPTION => {
                format!("{name} recommends an answer on {title}: {}", f.answer)
            }
            kinds::BLOCKER_RESOLVED => {
                format!("{name} answered a blocker on {title}: {}", f.answer)
            }
            kinds::BLOCKER_CLOSED => {
                let mut s = format!("{name} closed the blockers on {title}.");
                if !f.note.is_empty() {
                    s += &format!(" {}", f.note);
                }
                s
            }
            _ => {
                let q = f
                    .blocker
                    .as_ref()
                    .and_then(|v| v.get("question").or_else(|| v.get("reason")))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                format!("{name} raised a blocker on {title}: {q}")
            }
        }),
    })
}

impl tools::SharedBoards for super::ToolsView {
    fn boards(&self) -> Vec<tools::BoardInfo> {
        self.0
            .board_list()
            .into_iter()
            .filter(|s| s.ready)
            .map(|s| self.0.tools_board(s))
            .collect()
    }

    fn board_of(&self, task_id: &str) -> Option<tools::BoardInfo> {
        self.0.board_of(task_id).map(|s| self.0.tools_board(s))
    }

    fn board_coord(
        &self,
        board_id: &str,
        keys: &[String],
    ) -> Result<(Arc<Coord>, Vec<String>), coord::Error> {
        let co = self
            .0
            .board(board_id)
            .map_err(super::boards::Error::into_coord)?
            .coord()
            .clone();
        let mut out = Vec::with_capacity(keys.len());
        for k in keys {
            if k.is_empty() {
                out.push(String::new());
                continue;
            }
            out.push(
                self.0
                    .board_key(board_id, k)
                    .map_err(super::boards::Error::into_coord)?,
            );
        }
        Ok((co, out))
    }

    fn share(&self, task_id: &str, board_id: &str, _by: &str) -> Result<coord::Task, String> {
        let p = self
            .0
            .preview_share(task_id, board_id)
            .map_err(|e| e.to_string())?;
        self.0
            .share_to_board(task_id, board_id, &p.hash)
            .map_err(|e| e.to_string())
    }

    fn withdraw(&self, task_id: &str) -> Result<(), String> {
        self.0
            .withdraw_card(task_id)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
