//! The bridge from a board log into this device's inbox (v0.5 §7.1), and
//! the Core side of the tools' shared boards (§7.3).
//!
//! A row another person's device wrote on a board (a comment, an
//! assignment, a blocker) about a card one of this person's agents holds,
//! watches or is assigned becomes a private message.sent from a `board:`
//! sender, through [`inbox::Inbox::bridge`], which keeps only the agents
//! this device hosts and never bridges one row to one agent twice. The
//! person's own rows don't cross here: the tool that wrote one already
//! routed its notice. Rows met on a resync aren't bridged either, so a
//! re-fold doesn't replay old talk.

use std::sync::Arc;

use coord::Coord;
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
    state: String,
    pr: String,
    blocker: Option<serde_json::Value>,
}

/// A board's review request (§10, card #47's `review.requested`).
const REVIEW_REQUESTED: &str = "review.requested";

impl Services {
    /// Bridges one board row, live from `b`'s follower.
    pub(crate) fn bridge_row(&self, b: &BoardLog, ev: &Event) {
        if ev.seq == 0 {
            return;
        }
        let kind = match ev.kind.as_str() {
            kinds::TASK_COMMENT => row_kind::COMMENT,
            kinds::TASK_ASSIGNED => row_kind::ASSIGN,
            kinds::BLOCKER_CLOSED | kinds::BLOCKER_OPTION | kinds::BLOCKER_RESOLVED => {
                row_kind::BLOCKER
            }
            kinds::TASK_STATUS => row_kind::BLOCKER,
            REVIEW_REQUESTED => row_kind::ASSIGN,
            _ => return,
        };
        let Ok(f) = serde_json::from_slice::<RowFields>(&ev.payload) else {
            return;
        };
        if f.task_id.is_empty() || (ev.kind == kinds::TASK_STATUS && f.blocker.is_none()) {
            return;
        }
        let me = self.person_id();
        let Some(person) = b.verifier.person_of(&ev.device_id) else {
            return;
        };
        if person.is_empty() || person == me {
            return;
        }
        let card_id = ev
            .actor
            .strip_prefix(AGENT_ACTOR_PREFIX)
            .unwrap_or_default()
            .to_string();
        if card_id.is_empty() && !ev.actor.starts_with(PERSON_ACTOR_PREFIX) {
            return;
        }
        let co = b.coord();
        let Some(t) = co.get_task(&f.task_id) else {
            return;
        };
        // A review request names its agent: one asked without `to` is
        // written once per dedicated reviewer (coord's request_review).
        let mut to: Vec<String> = if kind == row_kind::ASSIGN {
            vec![f.to.clone()]
        } else {
            let mut v = t.holders.clone();
            v.extend(t.watchers.iter().cloned());
            v.extend(t.assigned.iter().cloned());
            v
        };
        to = to
            .iter()
            .filter(|k| k.starts_with(AGENT_ACTOR_PREFIX))
            .map(|k| self.private_key(k))
            .collect();
        if to.is_empty() {
            return;
        }
        let who = self.board_person_name(b, &person);
        let name = if card_id.is_empty() {
            who.clone()
        } else {
            let card = super::boards::lock(&b.maps)
                .cards
                .get(&card_id)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            if card.is_empty() {
                format!("{who}'s agent")
            } else {
                format!("{who}'s agent {card}")
            }
        };
        let title = gocompat::quote::quote(&t.title);
        let text = match ev.kind.as_str() {
            kinds::TASK_COMMENT => f.text.clone(),
            kinds::TASK_ASSIGNED => format!("{name} assigned you {title}."),
            REVIEW_REQUESTED => format!("{name} asks you to review {} for {title}.", f.pr),
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
                let _ = &f.state;
                format!("{name} raised a blocker on {title}: {q}")
            }
        };
        let tag = b.verifier.membership().tag;
        let r = BridgeRow {
            from: BoardFrom {
                board_id: b.id.clone(),
                tag,
                person_id: person,
                card_id,
                row: format!("{}/{}", ev.device_id, ev.seq),
                kind: kind.into(),
                wake: f.wake,
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
