//! A shared board's log rules (store.md §3.8; I:internal/store/board.go):
//! the allow-list and actor shape on every row, and reference closure on
//! Append (and for the fold, through `Store::unresolved`).
//!
//! Payloads are decoded the way Go's `json.Unmarshal` fills `boardRow`
//! (store.md §7.5): keys match case-insensitively, the last duplicate wins,
//! and a type error anywhere fails the whole row.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use gocompat::json::{self, Out, Schema};
use gocompat::quote::quote;

use crate::error::{Error, ErrorKind, Result};
use crate::kinds::{self, AGENT_ACTOR_PREFIX, PERSON_ACTOR_PREFIX, board_id};

/// What a board store refused since it opened (Go `BoardCounts`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BoardCounts {
    /// Not on the allow-list.
    pub kind: i64,
    /// Not a board actor.
    pub actor: i64,
    /// Append calls refused for a reference outside the board.
    pub outside: i64,
}

#[derive(Default)]
struct State {
    tasks: HashSet<String>,
    comments: HashSet<String>,
    /// taskId "/" blockerId
    blockers: HashSet<String>,
    /// taskId "/" stepId
    steps: HashSet<String>,
    cards: HashSet<String>,
    hashes: HashSet<String>,
    /// lease resources
    leases: HashSet<String>,
    /// board.created's creator, board.member.added's person
    persons: HashSet<String>,
    counts: BoardCounts,
}

/// A board store's index and counters (Go `boardRules`).
#[derive(Default)]
pub(crate) struct BoardRules {
    state: Mutex<State>,
}

#[derive(Default)]
struct On {
    kind: String,
    reference: String,
}

#[derive(Default)]
struct Item {
    id: String,
    on: Option<On>,
    files: Vec<String>,
}

/// Go `boardRow`: every field of a board row that defines or names an id.
#[derive(Default)]
struct Row {
    task_id: String,
    parent: String,
    comment_id: String,
    blocker_id: String,
    step_id: String,
    msg_id: String,
    card_id: String,
    sha256: String,
    resource: String,
    held: String,
    by: String,
    to: String,
    who: String,
    holder: String,
    held_by: String,
    blocked_by: Option<On>,
    blocker: Option<Item>,
    files: Vec<String>,
    steps: Vec<String>,
    stolen: Vec<(String, String)>,
    stolen_from: String,
    conflict: String,
    outcome: Option<String>,
    person_id: String,
    creator: Option<String>,
}

fn schema() -> &'static Schema {
    static S: OnceLock<Schema> = OnceLock::new();
    S.get_or_init(|| {
        let s = || Schema::String;
        let on = || Schema::Ptr(Box::new(Schema::object(vec![("kind", s()), ("ref", s())])));
        let refs = || Schema::Slice(Box::new(Schema::object(vec![("sha256", s())])));
        Schema::object(vec![
            ("taskId", s()),
            ("parent", s()),
            ("commentId", s()),
            ("blockerId", s()),
            ("stepId", s()),
            ("msgId", s()),
            ("cardId", s()),
            ("sha256", s()),
            ("resource", s()),
            ("held", s()),
            ("by", s()),
            ("to", s()),
            ("who", s()),
            ("holder", s()),
            ("heldBy", s()),
            ("blockedBy", on()),
            (
                "blocker",
                Schema::Ptr(Box::new(Schema::object(vec![
                    ("id", s()),
                    ("on", on()),
                    ("attachments", refs()),
                ]))),
            ),
            ("attachments", refs()),
            (
                "steps",
                Schema::Slice(Box::new(Schema::object(vec![("id", s())]))),
            ),
            (
                "stolen",
                Schema::Slice(Box::new(Schema::object(vec![
                    ("resource", s()),
                    ("holder", s()),
                ]))),
            ),
            ("stolenFrom", s()),
            ("conflict", s()),
            (
                "outcome",
                Schema::Ptr(Box::new(Schema::object(vec![("task", s())]))),
            ),
            ("personId", s()),
            (
                "creator",
                Schema::Ptr(Box::new(Schema::object(vec![("personId", s())]))),
            ),
        ])
    })
}

/// A non-nil pointer field's struct.
fn ptr<'a>(o: &'a Out, name: &str) -> Option<&'a Out> {
    match o.get(name) {
        Some(Out::Ptr(Some(inner))) => Some(inner),
        _ => None,
    }
}

fn items<'a>(o: &'a Out, name: &str) -> &'a [Out] {
    o.get(name).map(Out::items).unwrap_or(&[])
}

fn on_of(o: Option<&Out>) -> Option<On> {
    o.map(|o| On {
        kind: o.get_str("kind").into(),
        reference: o.get_str("ref").into(),
    })
}

fn shas(o: &Out, name: &str) -> Vec<String> {
    items(o, name)
        .iter()
        .map(|r| r.get_str("sha256").to_string())
        .collect()
}

/// `json.Unmarshal(payload, &boardRow{})`; `None` on any error.
fn decode(payload: &[u8]) -> Option<Row> {
    let o = json::unmarshal(payload, schema()).ok()?;
    let g = |n: &str| o.get_str(n).to_string();
    Some(Row {
        task_id: g("taskId"),
        parent: g("parent"),
        comment_id: g("commentId"),
        blocker_id: g("blockerId"),
        step_id: g("stepId"),
        msg_id: g("msgId"),
        card_id: g("cardId"),
        sha256: g("sha256"),
        resource: g("resource"),
        held: g("held"),
        by: g("by"),
        to: g("to"),
        who: g("who"),
        holder: g("holder"),
        held_by: g("heldBy"),
        blocked_by: on_of(ptr(&o, "blockedBy")),
        blocker: ptr(&o, "blocker").map(|b| Item {
            id: b.get_str("id").into(),
            on: on_of(ptr(b, "on")),
            files: shas(b, "attachments"),
        }),
        files: shas(&o, "attachments"),
        steps: items(&o, "steps")
            .iter()
            .map(|s| s.get_str("id").to_string())
            .collect(),
        stolen: items(&o, "stolen")
            .iter()
            .map(|s| {
                (
                    s.get_str("resource").to_string(),
                    s.get_str("holder").to_string(),
                )
            })
            .collect(),
        stolen_from: g("stolenFrom"),
        conflict: g("conflict"),
        outcome: ptr(&o, "outcome").map(|x| x.get_str("task").to_string()),
        person_id: g("personId"),
        creator: ptr(&o, "creator").map(|x| x.get_str("personId").to_string()),
    })
}

impl BoardRules {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn counts(&self) -> BoardCounts {
        self.lock().counts
    }

    pub(crate) fn count_actor(&self) {
        self.lock().counts.actor += 1;
    }

    pub(crate) fn count_outside(&self) {
        self.lock().counts.outside += 1;
    }

    /// Go `checkShape`: the allow-list and the actor, on every row.
    pub(crate) fn check_shape(&self, actor: &str, kind: &str, payload: &[u8]) -> Result<()> {
        if !kinds::board_accepted(kind) {
            self.lock().counts.kind += 1;
            return Err(Error::wrap(ErrorKind::NotBoardKind, kind));
        }
        if !kinds::board_actor_ok(actor) {
            self.count_actor();
            return Err(Error::wrap(ErrorKind::BoardActor, quote(actor)));
        }
        if kind == kinds::AGENT_CARD {
            // A card is written by the agent it describes, and nobody else.
            // On a decode error Go reports whatever cardId it had filled;
            // this reports it empty (the verdict is the same).
            let card = decode(payload).map(|r| r.card_id);
            let ok = card
                .as_deref()
                .is_some_and(|c| actor.strip_prefix(AGENT_ACTOR_PREFIX) == Some(c));
            if !ok {
                self.count_actor();
                return Err(Error::wrap(
                    ErrorKind::BoardActor,
                    format!(
                        "agent.card for {} written as {}",
                        quote(card.as_deref().unwrap_or("")),
                        quote(actor)
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Go `define`: the ids a stored row brings into the board. Sets only
    /// grow.
    // The arms mirror Go's switch case by case.
    #[allow(clippy::collapsible_match)]
    pub(crate) fn define(&self, kind: &str, payload: &[u8]) {
        let Some(p) = decode(payload) else {
            return;
        };
        let mut b = self.lock();
        match kind {
            kinds::TASK_CREATED => {
                if !p.task_id.is_empty() {
                    b.tasks.insert(p.task_id);
                }
            }
            kinds::TASK_COMMENT => {
                if !p.comment_id.is_empty() {
                    b.comments.insert(p.comment_id);
                }
            }
            kinds::TASK_STATUS => {
                if let Some(bl) = p.blocker
                    && !bl.id.is_empty()
                {
                    b.blockers.insert(format!("{}/{}", p.task_id, bl.id));
                }
            }
            kinds::TASK_STEPS => {
                for s in p.steps {
                    b.steps.insert(format!("{}/{}", p.task_id, s));
                }
            }
            kinds::AGENT_CARD => {
                b.cards.insert(p.card_id);
            }
            kinds::ATTACH_OFFERED => {
                if !p.sha256.is_empty() {
                    b.hashes.insert(p.sha256);
                }
            }
            kinds::LEASE_ACQUIRED => {
                if !p.resource.is_empty() {
                    b.leases.insert(p.resource);
                }
            }
            kinds::BOARD_CREATED => {
                if let Some(c) = p.creator
                    && board_id(&c)
                {
                    b.persons.insert(c);
                }
            }
            kinds::BOARD_MEMBER_ADDED => {
                if board_id(&p.person_id) {
                    b.persons.insert(p.person_id);
                }
            }
            _ => {}
        }
    }

    /// Go `resolves`, the closure check: `Ok` when every id the row names is
    /// held in this board, else the first that isn't, by field.
    #[allow(clippy::collapsible_match)]
    pub(crate) fn resolves(&self, kind: &str, payload: &[u8]) -> Result<()> {
        if kind.starts_with("member.") || kind.starts_with("board.") {
            return Ok(());
        }
        let Some(p) = decode(payload) else {
            return Err(Error::wrap(
                ErrorKind::OutsideBoard,
                format!("{kind} payload is not an object"),
            ));
        };
        let b = self.lock();
        let out =
            |reason: String| Error::wrap(ErrorKind::OutsideBoard, format!("{kind}: {reason}"));
        if !p.task_id.is_empty() && kind != kinds::TASK_CREATED && !b.tasks.contains(&p.task_id) {
            return Err(out(format!(
                "taskId {} is not a card on this board",
                p.task_id
            )));
        }
        if !p.parent.is_empty() && !b.tasks.contains(&p.parent) {
            return Err(out(format!(
                "parent {} is not a card on this board",
                p.parent
            )));
        }
        if !p.blocker_id.is_empty()
            && !b
                .blockers
                .contains(&format!("{}/{}", p.task_id, p.blocker_id))
        {
            return Err(out(format!(
                "blockerId {} is not a blocker of that card on this board",
                p.blocker_id
            )));
        }
        if !p.step_id.is_empty() && !b.steps.contains(&format!("{}/{}", p.task_id, p.step_id)) {
            return Err(out(format!(
                "stepId {} is not a step of that card on this board",
                p.step_id
            )));
        }
        if !p.comment_id.is_empty()
            && kind != kinds::TASK_COMMENT
            && !b.comments.contains(&p.comment_id)
        {
            return Err(out(format!(
                "commentId {} is not a comment on this board",
                p.comment_id
            )));
        }
        if !p.msg_id.is_empty() {
            return Err(out(
                "msgId names a private message; messages never go on a board".into(),
            ));
        }
        let self_card = if kind == kinds::AGENT_CARD {
            p.card_id.as_str()
        } else {
            ""
        };
        let mut keys: Vec<(&str, &str)> = vec![
            ("by", &p.by),
            ("to", &p.to),
            ("who", &p.who),
            ("holder", &p.holder),
            ("heldBy", &p.held_by),
        ];
        for (resource, holder) in &p.stolen {
            keys.push(("stolen.holder", holder));
            if !b.leases.contains(resource) {
                return Err(out(format!("stolen lease {resource} is not on this board")));
            }
        }
        for h in p.stolen_from.split(',') {
            keys.push(("stolenFrom", h.trim_matches(go_space)));
        }
        keys.push(("conflict", &p.conflict));
        if let Some(t) = &p.outcome
            && !t.is_empty()
            && !b.tasks.contains(t)
        {
            return Err(out(format!(
                "its outcome names task {t}, which is not on this board"
            )));
        }
        for (field, k) in &keys {
            let why = key(&b, k, self_card);
            if !why.is_empty() {
                return Err(out(format!("{field} {} {why}", quote(k))));
            }
        }
        let mut files: Vec<&String> = p.files.iter().collect();
        let mut ons: Vec<Option<&On>> = vec![p.blocked_by.as_ref()];
        if let Some(bl) = &p.blocker {
            files.extend(bl.files.iter());
            ons.push(bl.on.as_ref());
        }
        for f in files {
            if !b.hashes.contains(f) {
                return Err(out(format!("attachment {f} was not offered on this board")));
            }
        }
        for on in ons.into_iter().flatten() {
            match on.kind.as_str() {
                "task" => {
                    if !b.tasks.contains(&on.reference) {
                        return Err(out(format!(
                            "it waits on task {}, which is not on this board",
                            on.reference
                        )));
                    }
                }
                "lease" => {
                    if !b.leases.contains(&on.reference) {
                        return Err(out(format!(
                            "it waits on lease {}, which is not on this board",
                            on.reference
                        )));
                    }
                }
                "agent" | "person" => {
                    let why = key(&b, &on.reference, self_card);
                    if !why.is_empty() {
                        return Err(out(format!(
                            "it waits on {}, which {why}",
                            quote(&on.reference)
                        )));
                    }
                }
                _ => {}
            }
        }
        match kind {
            kinds::LEASE_RELEASED | kinds::LEASE_STOLEN => {
                if !b.leases.contains(&p.resource) {
                    return Err(out(format!("lease {} is not on this board", p.resource)));
                }
            }
            kinds::LEASE_REFUSED => {
                if !p.held.is_empty() && !b.leases.contains(&p.held) {
                    return Err(out(format!("lease {} is not on this board", p.held)));
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Go `strings.TrimSpace`'s set: `unicode.IsSpace`.
fn go_space(c: char) -> bool {
    gocompat::text::is_space(c)
}

/// Go `key`: "" when an actor-like key a row names is fine, else why not.
fn key(b: &State, k: &str, self_card: &str) -> &'static str {
    if k.is_empty() || k == "irori" {
        return "";
    }
    if let Some(id) = k.strip_prefix(PERSON_ACTOR_PREFIX) {
        if !board_id(id) {
            return "is not a person's board key";
        }
        if !b.persons.contains(id) {
            return "is not a person on this board";
        }
        return "";
    }
    if let Some(id) = k.strip_prefix(AGENT_ACTOR_PREFIX) {
        if !board_id(id) || (id != self_card && !b.cards.contains(id)) {
            return "is not an agent card on this board";
        }
        return "";
    }
    "is a private key; a board names only person: and agent: keys"
}
