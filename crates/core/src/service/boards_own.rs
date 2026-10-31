//! Shared boards, step 4 (I:internal/core/boards_own.go; design
//! v0.5-shared-boards.md §12): one person, several boards, on the person's
//! own devices only.
//!
//! - A board reaches the person's other devices through the private log:
//!   create writes board.joined {boardId, name, tag, creator}, and every
//!   device of the person opens that board pinned to the creator it names
//!   (§4.1 step 7, own devices only until invites). The board's rows then
//!   sync between them on `board/<id>` streams of the private mesh
//!   connection (`mesh::boards`), never on its sync channel.
//! - Forget drops a board on this device: its file and secrets go, and a
//!   device-local forgotten marker keeps an older board.joined from
//!   bringing it back (§4.6). Leave forgets it on every device of the
//!   person through a private board.forgot row.
//! - Share to board writes a fresh task.created on a board with the card's
//!   scrubbed fields and steps (§5.2), writes a shared_copies row (private,
//!   replicated to the person's devices), closes the private card with a
//!   note naming the shared copy, and refuses later writes to the private
//!   id on every device of the person.
//! - Withdraw cancels a shared card with withdrawn set (§5.3); its holders
//!   carry on in the private card it came from, or a new one.
//! - Short refs carry the board's tag (`mlg#12`, B3), and task_ref
//!   resolves a ref across every log this device holds.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use coord::{BoardFilter, BoardView, Card, Coord, ErrorKind as CK, Notice, Step, Task};
use gocompat::quote::quote as q;
use gocompat::time::GoTime;
use irori_identity::MemberPin;
use irori_identity::membership::MEMBER_ACTIVE;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use store::kinds::{self, AGENT_ACTOR_PREFIX, PERSON_ACTOR_PREFIX};
use store::{Event, Filter};

use super::Services;
use super::boards::{BoardLog, Error, Result, each_row, trim, valid_board_id};
use crate::logf;

/// The private board.joined row.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BoardJoinedRow {
    pub board_id: String,
    pub name: String,
    pub tag: String,
    pub creator: MemberPin,
    pub at: GoTime,
}

impl BoardJoinedRow {
    pub(crate) fn decode(payload: &[u8]) -> Option<BoardJoinedRow> {
        let v: serde_json::Value = serde_json::from_slice(payload).ok()?;
        let s = |k: &str| {
            v.get(k)
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let creator = match v.get("creator") {
            Some(c) => irori_identity::decode::<MemberPin>(c.to_string().as_bytes())?,
            None => MemberPin::default(),
        };
        Some(BoardJoinedRow {
            board_id: s("boardId"),
            name: s("name"),
            tag: s("tag"),
            creator,
            at: GoTime::default(),
        })
    }
}

/// The private board.forgot row: Leave, on every device.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BoardForgotRow {
    pub board_id: String,
    pub at: GoTime,
}

impl BoardForgotRow {
    pub(crate) fn decode(payload: &[u8]) -> Option<BoardForgotRow> {
        #[derive(Deserialize)]
        struct P {
            #[serde(rename = "boardId", default)]
            board_id: String,
        }
        let p: P = serde_json::from_slice(payload).ok()?;
        Some(BoardForgotRow {
            board_id: p.board_id,
            at: GoTime::default(),
        })
    }
}

/// The device-local forgotten marker (`board.forgotten`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ForgottenRow {
    board_id: String,
    /// The newest board.joined HLC it forgot.
    up_to: u64,
}

/// One shared_copies row (`board.sharedcopy`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct SharedCopyRow {
    pub private_task_id: String,
    pub board_id: String,
    pub task_id: String,
}

/// One board for the window's chips and the Boards dialog.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardSummary {
    pub board_id: String,
    pub name: String,
    pub tag: String,
    /// The board's current members' names, this person first ("Visible
    /// to: Stephen").
    pub people: Vec<String>,
    pub owner: bool,
    /// This person's devices certified on the board.
    pub devices: i64,
    /// Its live cards.
    pub cards: i64,
    /// Its board.created counted here, so cards can go on it. A board
    /// another device made shows up before its rows arrive.
    pub ready: bool,
    /// Another board here has the same tag (two devices made one each
    /// while apart), so its refs need the card's task id.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub tag_clash: bool,
    /// The person left it and no other member has said it holds the
    /// `left` row yet: "Leaving…" (§4.4).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub leaving: bool,
    /// What is wrong with it, one line each, most pressing first ("Can't
    /// reach any home server for mlg", "Update Irori: …"; §6.3-6.5, §8).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Where a private card was shared to, when it was: the board and the
/// shared card, and its ref there ("mlg#4").
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedCopy {
    pub board_id: String,
    pub board_name: String,
    pub task_id: String,
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub withdrawn: bool,
}

/// Exactly what Share to board would write (§5.2): the card's fields after
/// scrub and the stricter pass, and who will see it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharePreview {
    pub board_id: String,
    pub board_name: String,
    pub title: String,
    pub goal: String,
    pub done_when: String,
    pub steps: Vec<String>,
    pub links: Vec<String>,
    pub repo: String,
    pub visible_to: Vec<String>,
    /// What scrub took out, one line each ("a path in the goal"), so the
    /// person sees why the text differs.
    pub dropped: Vec<String>,
    /// Covers every field above. Share is given it back and refuses when
    /// the card's preview no longer matches, so the board gets only what
    /// the person saw (an agent may edit the card meanwhile).
    pub hash: String,
}

impl SharePreview {
    fn hash(&self) -> String {
        let mut p = self.clone();
        p.hash.clear();
        let raw = gocompat::json::marshal(&p).unwrap_or_default();
        hex::encode(Sha256::digest(&raw))
    }
}

fn or<'a>(a: &'a str, b: &'a str) -> &'a str {
    if a.is_empty() { b } else { a }
}

fn one_line(s: &str, n: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let r: Vec<char> = s.chars().collect();
    if r.len() > n {
        let mut out: String = r[..n - 1].iter().collect();
        out.push('…');
        return out;
    }
    s
}

fn dedupe(v: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    v.into_iter().filter(|s| seen.insert(s.clone())).collect()
}

impl Services {
    /// Opens and forgets boards as the private log's board.joined and
    /// board.forgot rows arrive, and keeps the person's shared copies and
    /// board cards current as they arrive from any of the person's devices
    /// (Go `followOwnBoards`). Runs until `sub` ends.
    pub(crate) fn follow_own_boards(self: &Arc<Self>, sub: &store::Subscription) {
        self.load_copies();
        self.reconcile_boards();
        while let Some(ev) = sub.recv() {
            if ev.seq == 0 {
                self.load_copies();
                self.reconcile_boards();
                for b in self.boards.list() {
                    let _ = b.load(&self.store);
                }
            } else if ev.kind == kinds::BOARD_JOINED || ev.kind == kinds::BOARD_FORGOT {
                self.reconcile_boards();
            } else if ev.kind == kinds::SHARED_COPY {
                self.fold_copy(&ev);
            } else if ev.kind == kinds::BOARD_CARD_OF {
                self.fold_card_key(&ev);
            }
        }
    }

    /// Brings the open boards in line with the private log: a board.joined
    /// opens its board (pinned to the creator it names) unless a newer
    /// board.forgot or this device's forgotten marker covers it; a
    /// board.forgot newer than every board.joined forgets it here.
    pub(crate) fn reconcile_boards(self: &Arc<Self>) {
        let _g = self.boards.rec.lock().unwrap_or_else(|e| e.into_inner());
        if self.boards.stop.get().is_some_and(|s| s.is_canceled()) {
            return;
        }
        let mut joined: BTreeMap<String, (BoardJoinedRow, u64)> = BTreeMap::new();
        let mut forgot: BTreeMap<String, u64> = BTreeMap::new();
        let res = each_row(
            &self.store,
            Filter {
                kinds: vec![kinds::BOARD_JOINED.into(), kinds::BOARD_FORGOT.into()],
                ..Default::default()
            },
            |ev| {
                let at = ev.hlc.0;
                if ev.kind == kinds::BOARD_JOINED {
                    if let Some(r) = BoardJoinedRow::decode(&ev.payload)
                        && valid_board_id(&r.board_id)
                        && joined.get(&r.board_id).is_none_or(|(_, t)| at >= *t)
                    {
                        joined.insert(r.board_id.clone(), (r, at));
                    }
                } else if let Some(r) = BoardForgotRow::decode(&ev.payload)
                    && valid_board_id(&r.board_id)
                    && forgot.get(&r.board_id).is_none_or(|t| at > *t)
                {
                    forgot.insert(r.board_id, at);
                }
            },
        );
        if res.is_err() {
            return;
        }
        let marks = self.forgotten_marks();
        for (id, f) in &forgot {
            let j = joined.get(id).map(|(_, t)| *t).unwrap_or(0);
            if j > *f {
                continue;
            }
            if self.board(id).is_ok()
                && let Err(e) = self.forget_board_locked(id, j)
            {
                logf(format_args!("board {id}: forget: {e}"));
            }
        }
        for (id, (row, at)) in &joined {
            if forgot.get(id).is_some_and(|f| f >= at) {
                continue;
            }
            if marks.get(id).is_some_and(|up| up >= at) {
                continue;
            }
            if self.board(id).is_ok() {
                continue;
            }
            if let Err(e) = self.open_board(id, false, Some(&row.creator)) {
                logf(format_args!("board {id}: open from board.joined: {e}"));
                continue;
            }
            self.boards.moved();
        }
    }

    /// This device's forgotten list: boardId -> upTo.
    fn forgotten_marks(&self) -> HashMap<String, u64> {
        let mut out: HashMap<String, u64> = HashMap::new();
        let _ = each_row(
            &self.store,
            Filter {
                kinds: vec![kinds::BOARD_FORGOTTEN.into()],
                local: true,
                ..Default::default()
            },
            |ev| {
                if let Ok(r) = serde_json::from_slice::<ForgottenRow>(&ev.payload)
                    && !r.board_id.is_empty()
                {
                    let up = out.entry(r.board_id).or_default();
                    *up = (*up).max(r.up_to);
                }
            },
        );
        out
    }

    /// Forgets a board on this device only (§4.6): its log and secrets are
    /// deleted here, and a later board.joined from another of the person's
    /// devices doesn't bring it back. Other devices keep it.
    pub(crate) fn forget_board(&self, board_id: &str) -> Result<()> {
        self.board(board_id)?;
        let _g = self.boards.rec.lock().unwrap_or_else(|e| e.into_inner());
        let mut up_to = 0u64;
        let _ = each_row(
            &self.store,
            Filter {
                kinds: vec![kinds::BOARD_JOINED.into()],
                ..Default::default()
            },
            |ev| {
                if BoardJoinedRow::decode(&ev.payload).is_some_and(|r| r.board_id == board_id) {
                    up_to = up_to.max(ev.hlc.0);
                }
            },
        );
        self.forget_board_locked(board_id, up_to)
    }

    /// Closes board `id`, deletes its files and records the forgotten
    /// marker. `boards.rec` is held.
    fn forget_board_locked(&self, id: &str, up_to: u64) -> Result<()> {
        self.store.append_local(
            "irori",
            kinds::BOARD_FORGOTTEN,
            &ForgottenRow {
                board_id: id.into(),
                up_to,
            },
        )?;
        let r = self.remove_board(id);
        self.boards.moved();
        r
    }

    /// Closes board `id` and deletes its files: forgetting it, or a board
    /// whose creation failed half way, which must not open again as a
    /// nameless board at the next start.
    pub(crate) fn remove_board(&self, id: &str) -> Result<()> {
        self.drop_board(id);
        let dir = self.data_dir.join(super::boards::BOARDS_DIR);
        let mut errs = Vec::new();
        for name in [
            format!("{id}.db"),
            format!("{id}.db-wal"),
            format!("{id}.db-shm"),
            format!("{id}.secrets"),
        ] {
            match std::fs::remove_file(dir.join(&name)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => errs.push(e.to_string()),
                _ => {}
            }
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(Error::Other(errs.join("\n")))
        }
    }

    /// Closes board `id`'s log and takes it out of the Core.
    fn drop_board(&self, id: &str) {
        let b = {
            let mut inner = self.boards.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.task_of.retain(|_, on| on != id);
            inner.logs.remove(id)
        };
        self.mesh.close_board(id, irori_meshseam::reason::LEAVING);
        if let Some(b) = b {
            b.stop_threads();
            let _ = b.store.close();
        }
    }

    /// Every board on this device, by name.
    pub(crate) fn board_list(&self) -> Vec<BoardSummary> {
        let me = self.person_id();
        let mut out: Vec<BoardSummary> = self
            .boards
            .list()
            .iter()
            .map(|b| {
                let m = b.verifier.membership();
                let mut s = BoardSummary {
                    board_id: b.id.clone(),
                    name: m.name.clone(),
                    tag: m.tag.clone(),
                    owner: m.owner(&me),
                    ready: m.created,
                    people: self.board_people(b),
                    leaving: self.board_leaving(b),
                    warnings: self.board_status(b),
                    ..Default::default()
                };
                if s.name.is_empty() {
                    (s.name, s.tag) = self.joined_name(&b.id);
                }
                s.devices = b
                    .verifier
                    .devices(&me)
                    .iter()
                    .filter(|d| !d.revoked)
                    .count() as i64;
                s.cards = b
                    .coord()
                    .list_tasks(&coord::TaskFilter {
                        open: true,
                        ..Default::default()
                    })
                    .iter()
                    .filter(|t| !t.withdrawn)
                    .count() as i64;
                s
            })
            .collect();
        let mut tags: HashMap<String, usize> = HashMap::new();
        for s in &out {
            if !s.tag.is_empty() {
                *tags.entry(s.tag.clone()).or_default() += 1;
            }
        }
        for s in &mut out {
            s.tag_clash = tags.get(&s.tag).copied().unwrap_or(0) > 1;
        }
        out.sort_by_key(|s| s.name.to_lowercase());
        out
    }

    /// A board's name and tag from its board.joined row, for a board whose
    /// rows haven't arrived yet.
    fn joined_name(&self, id: &str) -> (String, String) {
        let mut out = (String::new(), String::new());
        let _ = each_row(
            &self.store,
            Filter {
                kinds: vec![kinds::BOARD_JOINED.into()],
                ..Default::default()
            },
            |ev| {
                if let Some(r) = BoardJoinedRow::decode(&ev.payload)
                    && r.board_id == id
                {
                    out = (r.name, r.tag);
                }
            },
        );
        out
    }

    /// `b`'s current members' names, this person first.
    pub(crate) fn board_people(&self, b: &BoardLog) -> Vec<String> {
        let me = self.person_id();
        let m = b.verifier.membership();
        let name_of = |p: &str| {
            let mut name = b.verifier.profile(p).unwrap_or_default();
            if name.is_empty()
                && let Some(bp) = m.person(p)
            {
                name = bp.name.clone();
            }
            if name.is_empty() && p == me {
                name = self.ident.current().name;
            }
            or(&name, "someone").to_string()
        };
        let mut out = Vec::new();
        if m.active(&me) || !m.created {
            out.push(name_of(&me));
        }
        for p in &m.members {
            if p.person_id != me && p.state == MEMBER_ACTIVE {
                out.push(name_of(&p.person_id));
            }
        }
        out
    }

    /// The board a task lives on, or `None` for a private card.
    pub(crate) fn board_of(&self, task_id: &str) -> Option<BoardSummary> {
        let id = self
            .route(task_id)
            .ok()
            .map(|(id, _)| id)
            .filter(|id| !id.is_empty())?;
        self.board_list().into_iter().find(|s| s.board_id == id)
    }

    /// The open boards tagged `tag`. Create keeps a tag to one of the
    /// person's boards, but two devices apart can each make one.
    fn boards_by_tag(&self, tag: &str) -> Vec<Arc<BoardLog>> {
        self.boards
            .list()
            .into_iter()
            .filter(|b| b.verifier.membership().tag == tag)
            .collect()
    }

    /// Resolves what someone typed for a card across every log this device
    /// holds (§2.5, B3): "mlg#12" on the board tagged mlg; a bare "#12"
    /// only when exactly one log has it, else Ambiguous naming each; a full
    /// id, with or without "task:", as given.
    pub(crate) fn task_ref(&self, r: &str) -> std::result::Result<String, coord::Error> {
        let mut v = trim(r);
        if let Some((k, id)) = v.split_once(':')
            && trim(k).eq_ignore_ascii_case("task")
        {
            v = trim(id);
        }
        let Some((tag, _)) = coord::parse_ref(v) else {
            return self.coord.task_ref(r);
        };
        let coords: Vec<Arc<Coord>> = if !tag.is_empty() {
            // A tag two boards share (made on two devices apart) names a
            // card only when one of them has it, like a bare number (B3).
            let tagged = self.boards_by_tag(&tag);
            match tagged.len() {
                0 => {
                    return Err(coord::Error::new(
                        CK::NotFound,
                        format_args!("no board here is tagged {}", q(&tag)),
                    ));
                }
                1 => return tagged[0].coord().task_ref(v),
                _ => tagged.iter().map(|b| b.coord().clone()).collect(),
            }
        } else {
            std::iter::once(self.coord.clone())
                .chain(self.boards.list().iter().map(|b| b.coord().clone()))
                .collect()
        };
        let mut hits: Vec<Task> = Vec::new();
        for co in &coords {
            match co.task_ref(v) {
                Err(e) if e.kind() == CK::Ambiguous => return Err(e),
                Err(_) => continue,
                Ok(id) => {
                    if let Some(t) = co.get_task(&id) {
                        hits.push(t);
                    }
                }
            }
        }
        match hits.len() {
            0 if !tag.is_empty() => Err(coord::Error::new(
                CK::NotFound,
                format_args!("no card {v} on the boards tagged {}", q(&tag)),
            )),
            0 => self.coord.task_ref(r),
            1 => Ok(hits.remove(0).id),
            _ => {
                let names: Vec<String> = hits
                    .iter()
                    .map(|t| {
                        format!(
                            "{} {} (task:{})",
                            t.short_ref(),
                            q(&one_line(&t.title, 60)),
                            t.id
                        )
                    })
                    .collect();
                if !tag.is_empty() {
                    return Err(coord::Error::new(
                        CK::Ambiguous,
                        format_args!(
                            "more than one board here is tagged {}, and each has {v}: {}; say which by its task id",
                            q(&tag),
                            names.join(", ")
                        ),
                    ));
                }
                Err(coord::Error::new(
                    CK::Ambiguous,
                    format_args!(
                        "#{} is on more than one board here: {}; say which",
                        v.trim_start_matches('#'),
                        names.join(", ")
                    ),
                ))
            }
        }
    }

    /// Reads the person's shared_copies rows: the private ones every device
    /// of the person holds, and this device's local ones from before they
    /// replicated.
    fn load_copies(&self) {
        let mut rows: Vec<Event> = Vec::new();
        let f = Filter {
            kinds: vec![kinds::SHARED_COPY.into()],
            ..Default::default()
        };
        let _ = each_row(
            &self.store,
            Filter {
                local: true,
                ..f.clone()
            },
            |ev| rows.push(ev.clone()),
        );
        let _ = each_row(&self.store, f, |ev| rows.push(ev.clone()));
        for ev in &rows {
            self.fold_copy(ev);
        }
    }

    fn fold_copy(&self, ev: &Event) {
        let Ok(r) = serde_json::from_slice::<SharedCopyRow>(&ev.payload) else {
            return;
        };
        if r.private_task_id.is_empty() || r.task_id.is_empty() {
            return;
        }
        self.boards
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .copies
            .insert(r.private_task_id.clone(), r);
    }

    fn copies_snapshot(&self) -> HashMap<String, SharedCopyRow> {
        self.boards
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .copies
            .clone()
    }

    /// Where private card `task_id` was shared to, while that board is open
    /// on this device: a forgotten board's copy is no longer anywhere the
    /// person can reach, so the private card is theirs again.
    pub(crate) fn shared_copy_of(&self, task_id: &str) -> Option<SharedCopy> {
        let sc = self.copies_snapshot().remove(task_id)?;
        let b = self.board(&sc.board_id).ok()?;
        let mut out = SharedCopy {
            board_id: sc.board_id.clone(),
            task_id: sc.task_id.clone(),
            reference: format!("task:{}", sc.task_id),
            board_name: b.verifier.membership().name,
            withdrawn: false,
        };
        if let Some(t) = b.coord().get_task(&sc.task_id) {
            out.reference = t.short_ref();
            out.withdrawn = t.withdrawn;
        }
        Some(out)
    }

    /// `Shared` for a write to a private card that was shared to a board and
    /// not withdrawn, naming the shared card and who sees it.
    pub(crate) fn shared_refusal(&self, task_id: &str) -> Option<Error> {
        let sc = self.shared_copy_of(task_id).filter(|sc| !sc.withdrawn)?;
        let b = self.board(&sc.board_id).ok()?; // the board went just now
        Some(Error::Shared(format!(
            "core: the card was shared to a board: it was shared to {} as {} (visible to: {}), and this private card is closed; write to {} instead",
            or(&sc.board_name, "a board"),
            sc.reference,
            self.board_people(&b).join(", "),
            sc.reference
        )))
    }

    /// The private card `task_id`, if it can be shared.
    fn shareable(&self, task_id: &str) -> Result<Task> {
        let Some(t) = self.coord.get_task(task_id) else {
            if self.route(task_id).is_ok_and(|(id, _)| !id.is_empty()) {
                return Err(Error::coord(CK::Invalid, "the card is already on a board"));
            }
            return Err(Error::coord(
                CK::NotFound,
                format_args!("task {}", q(task_id)),
            ));
        };
        if coord::is_terminal(&t.state) {
            return Err(Error::coord(
                CK::BadState,
                format_args!(
                    "{} is {}; share a card that is still in play",
                    t.short_ref(),
                    t.state
                ),
            ));
        }
        if t.ask.is_some() {
            return Err(Error::coord(
                CK::Invalid,
                "an ask isn't shared; its outcome card can be",
            ));
        }
        if !t.parent.is_empty() || !t.subtasks.is_empty() {
            return Err(Error::coord(
                CK::Invalid,
                "a card with subtasks, or a subtask, isn't shared yet",
            ));
        }
        if self.shared_copy_of(task_id).is_some() {
            return Err(Error::coord(
                CK::Invalid,
                format_args!("{} was shared already", t.short_ref()),
            ));
        }
        Ok(t)
    }

    /// The share preview for private card `task_id` on `board_id`.
    pub(crate) fn preview_share(&self, task_id: &str, board_id: &str) -> Result<SharePreview> {
        let t = self.shareable(task_id)?;
        let b = self.board(board_id)?;
        let m = b.verifier.membership();
        if !m.created || !m.active(&self.person_id()) {
            return Err(Error::coord(
                CK::BadState,
                format_args!(
                    "board {} isn't ready on this device yet",
                    or(&m.name, board_id)
                ),
            ));
        }
        let mut p = SharePreview {
            board_id: board_id.into(),
            board_name: m.name.clone(),
            visible_to: self.board_people(&b),
            ..Default::default()
        };
        let mut dropped = Vec::new();
        let mut clean = |what: &str, input: &str| {
            let mut s = self.scrubber.string(input);
            if s != input {
                dropped.push(format!("a secret in the {what}"));
            }
            let st = scrub::strict(&s);
            if st != s {
                dropped.push(format!("a path in the {what}"));
                s = st;
            }
            s
        };
        p.title = clean("title", &t.title);
        p.goal = clean("goal", &t.objective);
        p.done_when = clean("done-when", &t.done_criteria);
        p.steps = t.steps.iter().map(|s| clean("steps", &s.text)).collect();
        for l in &t.links {
            if l.kind == coord::LINK_PATH && scrub::strict_dropped(&l.reference) {
                dropped.push("a link to a path on this machine".into());
            } else if self.scrubber.string(&l.reference) != l.reference {
                dropped.push("a link with a secret in it".into());
            } else {
                p.links.push(l.reference.clone());
            }
        }
        if !t.repo_key.is_empty() && !scrub::strict_dropped(&t.repo_key) {
            p.repo = t.repo_key.clone();
        }
        p.dropped = dedupe(dropped);
        p.hash = p.hash();
        Ok(p)
    }

    /// Shares private card `task_id` to `board_id` as the person (§5.2): a
    /// fresh task.created with the preview's fields and steps, the
    /// shared_copies row, and the private card closed with a note naming
    /// the shared copy. Its comments, notes and blockers stay private.
    /// `confirmed` is the Hash of the preview the person confirmed: a card
    /// that changed since is refused with `PreviewStale`.
    pub(crate) fn share_to_board(
        &self,
        task_id: &str,
        board_id: &str,
        confirmed: &str,
    ) -> Result<Task> {
        let p = self.preview_share(task_id, board_id)?;
        if confirmed.is_empty() || confirmed != p.hash {
            return Err(Error::PreviewStale(
                "core: the card changed since its preview: look at the preview again before sharing"
                    .into(),
            ));
        }
        let b = self.board(board_id)?;
        let priv_t = self.coord.get_task(task_id).unwrap_or_default();
        let steps = p
            .steps
            .iter()
            .enumerate()
            .map(|(i, s)| Step {
                text: s.clone(),
                done: priv_t.steps.get(i).is_some_and(|x| x.done),
                ..Default::default()
            })
            .collect();
        let shared = b.coord().create_task(
            &self.person_actor(),
            coord::NewTask {
                title: p.title.clone(),
                objective: p.goal.clone(),
                done_criteria: p.done_when.clone(),
                repo_key: p.repo.clone(),
                links: p.links.clone(),
                steps,
                ..Default::default()
            },
        )?;
        self.boards
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .task_of
            .insert(shared.id.clone(), board_id.into());
        let row = SharedCopyRow {
            private_task_id: task_id.into(),
            board_id: board_id.into(),
            task_id: shared.id.clone(),
        };
        self.store.append("irori", kinds::SHARED_COPY, &row)?;
        self.boards
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .copies
            .insert(task_id.into(), row);
        // The private card closes; its holders hear it at their next
        // turn, with where the card went.
        let note = format!(
            "Shared to {} as {} (visible to: {}). Work it there; this private card is closed.",
            p.board_name,
            shared.short_ref(),
            p.visible_to.join(", ")
        );
        self.coord
            .set_task_state(&self.user_sender().key, task_id, coord::CANCELED, &note)?;
        Ok(shared)
    }

    /// Takes a shared card back off its board as the person (§5.3): it is
    /// canceled with withdrawn set, other members hide it, and its holders
    /// on this device carry on in the private card it came from, or a new
    /// one, which is returned (`None` when nobody here held it).
    pub(crate) fn withdraw_card(&self, task_id: &str) -> Result<Option<Task>> {
        let (board_id, co) = self.route(task_id)?;
        if board_id.is_empty() {
            return Err(Error::coord(
                CK::Invalid,
                format_args!("{task_id} is a private card; cancel it instead"),
            ));
        }
        let t = co.withdraw(&self.person_actor(), task_id, "withdrawn from the board")?;
        let user = self.user_sender().key;
        let locals: Vec<String> = t
            .holders
            .iter()
            .filter_map(|h| {
                let k = self.private_key(h);
                (k != *h && k != user).then_some(k)
            })
            .collect();
        // A private card it was shared from comes back into play;
        // otherwise a new private card carries the work on, if anyone here
        // held it.
        if let Some(priv_id) = self
            .copies_snapshot()
            .into_iter()
            .find(|(_, sc)| sc.task_id == task_id)
            .map(|(p, _)| p)
        {
            let (back, n) = self.coord.reopen(&user, &priv_id)?;
            self.tell_notices(&n);
            for k in &locals {
                if let Ok((_, n)) = self.coord.assign(&user, &priv_id, k) {
                    self.tell_notices(&n);
                }
            }
            return Ok(Some(back));
        }
        if locals.is_empty() {
            return Ok(None);
        }
        let nt = self.coord.create_task(
            &user,
            coord::NewTask {
                title: t.title.clone(),
                objective: t.objective.clone(),
                done_criteria: t.done_criteria.clone(),
                repo_key: t.repo_key.clone(),
                links: t.links.iter().map(|l| l.reference.clone()).collect(),
                steps: t
                    .steps
                    .iter()
                    .map(|s| Step {
                        text: s.text.clone(),
                        done: s.done,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            },
        )?;
        for k in &locals {
            match self.coord.assign(&user, &nt.id, k) {
                Ok((_, n)) => self.tell_notices(&n),
                Err(e) => logf(format_args!("withdraw {task_id}: assign {k}: {e}")),
            }
        }
        Ok(Some(nt))
    }

    /// `key` as board `b` knows it, without minting a card: the person's
    /// person: key for the person here, an agent's card when it has one on
    /// `b`, else "" (it is on no card of b, so nothing there is its).
    fn known_key(&self, b: &BoardLog, key: &str) -> String {
        if key.is_empty() {
            return String::new();
        }
        if self.chain.is_owner(key) {
            return self.person_actor();
        }
        if key.starts_with(PERSON_ACTOR_PREFIX) || key.starts_with(AGENT_ACTOR_PREFIX) {
            return key.into();
        }
        let m = b.maps.lock().unwrap_or_else(|e| e.into_inner());
        m.keys
            .get(key.strip_prefix("api:").unwrap_or(key))
            .map(|id| format!("{AGENT_ACTOR_PREFIX}{id}"))
            .unwrap_or_default()
    }

    /// A board card with its board keys as this device knows them: the
    /// person's person:<id> is the person here, the person's agents' cards
    /// are their sessions.
    pub(crate) fn private_task(&self, mut t: Task) -> Task {
        let k = |s: &str| self.private_key(s);
        t.created_by = k(&t.created_by);
        t.updated_by = k(&t.updated_by);
        t.holder = k(&t.holder);
        for v in [&mut t.holders, &mut t.assigned, &mut t.watchers] {
            for s in v.iter_mut() {
                *s = k(s);
            }
        }
        t
    }

    /// A notice with a board card's keys (agent:<cardId>, this person's
    /// person:<id>) as this device's inbox knows them, so a notice about a
    /// card on a shared board reaches this person's own agents (§2.4).
    /// Other people's keys stay as they are; the bridge (§7.1) handles them.
    pub(crate) fn private_notice(&self, mut n: Notice) -> Notice {
        for k in n.to.iter_mut() {
            *k = self.private_key(k);
        }
        n.by = self.private_key(&n.by);
        n
    }

    /// `f`'s Board on every ready board this device holds (§2.1: tools see
    /// shared cards as the window does), the viewer's key as each board
    /// knows it and every key in the result as this device knows it. A
    /// viewer on no card of a board gets that board's view with nothing for
    /// it.
    pub(crate) fn board_views(&self, f: &BoardFilter) -> Vec<BoardView> {
        let mut out = Vec::new();
        for b in self.boards.list() {
            if !b.verifier.membership().created {
                continue;
            }
            let mut bf = f.clone();
            if !f.viewer.is_empty() {
                bf.viewer = self.known_key(&b, &f.viewer);
                if bf.viewer.is_empty() {
                    bf.viewer = format!("{AGENT_ACTOR_PREFIX}-"); // no card: nobody
                }
            }
            let mut v = b.coord().board(&bf);
            for lane in [
                &mut v.needs_you,
                &mut v.blocked,
                &mut v.working,
                &mut v.open,
                &mut v.done,
            ] {
                let kept: Vec<Card> = std::mem::take(lane)
                    .into_iter()
                    .filter(|c| !c.task.withdrawn)
                    .map(|mut c| {
                        c.task = self.private_task(c.task);
                        if let Some(lc) = c.last_comment.as_mut() {
                            lc.by = self.private_key(&lc.by);
                        }
                        c
                    })
                    .collect();
                *lane = kept;
            }
            for l in v.leases.iter_mut() {
                l.lease.holder = self.private_key(&l.lease.holder);
            }
            out.push(v);
        }
        out
    }

    /// Routes a coord notice from the person at its next turn.
    fn tell_notices(&self, n: &Notice) {
        if let Some(e) = crate::notice::notice_err(&self.route_notice(&self.user_sender(), n)) {
            logf(format_args!("boards: notice: {e}"));
        }
    }
}
