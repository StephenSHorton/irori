//! Spaces (card #392, decision 46, docs/design/talk-channels.md §5).
//!
//! A space is what v0.5 called a shared board: one log, one secret, one
//! topic, one set of members. Its id is that board's id, so a v0.10 board
//! is already a one-board space and nothing in its log changes. A space
//! holds more boards with `space.board.created` rows; the first board is
//! the space's own (`board.created`), with the space's id.
//!
//! An Irori before spaces refuses a kind it doesn't know, and every later
//! row of the writer's stream then waits on the gap. So a space writes no
//! space-only row while a member device is known to read less than
//! `irori_mesh::boardmesh::SPACES_PROTOCOL` (docs/design/no-server.md §5).
//!
//! Personal is a space too (card #558): private, no invites, and its log is
//! the private log. `space.board.*` rows go there with no gate, since an
//! older device stores them and folds past them. Its default board is
//! implicit, with no tag, so every private card stays on it as `#N`.

use std::collections::BTreeMap;

use coord::ErrorKind as CK;
use mesh::boardmesh::{BoardReads, DeviceWords, SPACES_PROTOCOL};
use serde::{Deserialize, Serialize};
use store::{Filter, kinds};

use super::Services;
use super::boards::{BoardLog, Error, Result};

/// The id of Personal's default board, in `space.board.renamed` rows and
/// the window. A card's row names only an inner board, so no card carries
/// it; [`super::boards::new_board_id`] makes hex, so no board has it either.
pub const PERSONAL_BOARD: &str = "personal";

/// Personal's default board, before any rename.
const PERSONAL_NAME: &str = "Personal";

/// Longest board description, in characters (card #690).
pub const MAX_BOARD_DESCRIPTION: usize = 2000;

/// One board of a space.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceBoard {
    /// The board's id: the space's own id for its first board.
    pub board_id: String,
    pub name: String,
    pub tag: String,
    /// The board's description (card #690): optional free text, often
    /// the end result it is for. Empty with none.
    #[serde(default)]
    pub description: String,
    /// Who wrote the description, from the row's place (never its `by`):
    /// `person:<id>` in a space, `user` in Personal; empty with none.
    #[serde(default)]
    pub description_by: String,
    /// Closed: read-only and out of the lists (card #690).
    #[serde(default)]
    pub closed: bool,
    /// When it was closed (the close row's `at`), while it is.
    #[serde(default)]
    pub closed_at: String,
    /// Whether this person may close or reopen it now: its maker or a
    /// space owner, anyone in Personal; never a first board.
    #[serde(default)]
    pub may_close: bool,
}

/// `space.board.created` / `space.board.renamed`. A delete is a rename with
/// `deleted` set and both names empty (card #563): an older device stores
/// it and changes nothing, which is why it isn't a kind of its own. `name`
/// and `tag` are always written, since an older decoder needs them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SpaceBoardRow {
    pub board_id: String,
    pub name: String,
    pub tag: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
    /// A new board's description (card #690), on its create row only:
    /// every decoder ignores a field it doesn't know, so one row makes a
    /// board.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub by: String,
    pub at: String,
}

/// `space.board.described` (card #690): a board's description, replaced
/// (empty clears).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BoardDescribedRow {
    pub board_id: String,
    pub description: String,
    pub by: String,
    pub at: String,
}

/// `space.board.closed` (card #690): closed, or reopened with
/// `closed: false`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BoardClosedRow {
    pub board_id: String,
    pub closed: bool,
    pub by: String,
    pub at: String,
}

/// What closing a board does with its open cards (card #690).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenCards {
    /// Nothing: the close is refused while any card is open, naming them.
    Keep,
    /// Moves each open card, its family along, to this board: a board id
    /// of the same space (or Personal), its first board's by the space's
    /// id ([`PERSONAL_BOARD`] in Personal).
    MoveTo(String),
    /// Cancels each open card (a parent's subtasks with it).
    Cancel,
}

/// Who wrote what, at a row's place, for the board fold (card #690).
pub(crate) struct Rules<'a> {
    /// The writer was an active member there: deletes and descriptions.
    pub(crate) member: MayDelete<'a>,
    /// The writer was a space owner there: closes any board.
    pub(crate) owner: MayDelete<'a>,
    /// A device's person key (`person:<id>`; `user` in Personal).
    pub(crate) person: &'a dyn Fn(&str) -> Option<String>,
}

/// Runs `f` with Personal's rules: every row in the private log is the
/// person's.
pub(crate) fn with_personal_rules<R>(f: impl FnOnce(&Rules) -> R) -> R {
    f(&Rules {
        member: &|_| true,
        owner: &|_| true,
        person: &|_| Some("user".to_string()),
    })
}

/// Runs `f` with a space's rules, read from its verifier at each row's
/// place.
pub(crate) fn with_space_rules<R>(
    v: &std::sync::Arc<irori_identity::board::BoardVerifier>,
    f: impl FnOnce(&Rules) -> R,
) -> R {
    let member = member_wrote(v);
    let owner = {
        let v = v.clone();
        move |at: &coord::RowPos| {
            let Some(person) = v.person_of(&at.dev) else {
                return false;
            };
            v.membership().owner_at(
                &person,
                &irori_identity::membership::RowAt {
                    hlc: at.hlc,
                    device: at.dev.clone(),
                    seq: at.seq,
                },
            )
        }
    };
    let person = {
        let v = v.clone();
        move |dev: &str| {
            v.person_of(dev)
                .map(|p| format!("{}{p}", kinds::PERSON_ACTOR_PREFIX))
        }
    };
    f(&Rules {
        member: &member,
        owner: &owner,
        person: &person,
    })
}

/// One `space.board.*` row, with who wrote it and where it sits.
#[derive(Debug, Clone)]
pub struct BoardRowAt {
    pub kind: String,
    pub payload: Vec<u8>,
    pub at: coord::RowPos,
}

/// Whether a delete at this place counts.
pub(crate) type MayDelete<'a> = &'a dyn Fn(&coord::RowPos) -> bool;

/// Each deleted board with its first valid delete's place. A delete wins
/// whatever came before or after it; the first board can't be deleted, so
/// a space always keeps one (fold rules 4, 7, 8).
fn deletes(own: &str, rows: &[BoardRowAt], may: MayDelete) -> BTreeMap<String, coord::RowPos> {
    let mut out: BTreeMap<String, coord::RowPos> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.kind == kinds::SPACE_BOARD_RENAMED) {
        let Ok(p) = serde_json::from_slice::<SpaceBoardRow>(&r.payload) else {
            continue;
        };
        if !p.deleted || p.board_id.is_empty() || p.board_id == own || !may(&r.at) {
            continue;
        }
        let first = out.entry(p.board_id).or_insert_with(|| r.at.clone());
        if r.at.before(first) {
            *first = r.at.clone();
        }
    }
    out
}

/// The boards in a space's log, its own first: `own` is the space's own
/// board (from board.created/renamed), then each `space.board.created` in
/// RowPos order with its newest rename. A rename of a board the log doesn't
/// hold is ignored, as is a second create of one id, and a deleted board is
/// gone for good: nothing brings it back.
///
/// `own` comes in with the space's name, and keeps it until a rename names
/// the first board itself (card #558): only its name, since its tag is the
/// space's and refs are stamped with it.
pub fn fold_space_boards(
    mut own: SpaceBoard,
    rows: &[BoardRowAt],
    may: MayDelete,
) -> Vec<SpaceBoard> {
    let gone = deletes(&own.board_id, rows, may);
    let mut rows: Vec<&BoardRowAt> = rows.iter().collect();
    rows.sort_by(|a, b| a.at.cmp(&b.at));
    let mut order: Vec<String> = Vec::new();
    let mut boards: BTreeMap<String, SpaceBoard> = BTreeMap::new();
    for row in rows {
        let Ok(r) = serde_json::from_slice::<SpaceBoardRow>(&row.payload) else {
            continue;
        };
        if r.board_id.is_empty() || gone.contains_key(&r.board_id) {
            continue;
        }
        if r.board_id == own.board_id {
            if row.kind == kinds::SPACE_BOARD_RENAMED && !r.name.is_empty() {
                own.name = r.name;
            }
            continue;
        }
        match row.kind.as_str() {
            kinds::SPACE_BOARD_CREATED if !boards.contains_key(&r.board_id) => {
                order.push(r.board_id.clone());
                boards.insert(
                    r.board_id.clone(),
                    SpaceBoard {
                        board_id: r.board_id,
                        name: r.name,
                        tag: r.tag,
                        ..Default::default()
                    },
                );
            }
            kinds::SPACE_BOARD_RENAMED => {
                if let Some(b) = boards.get_mut(&r.board_id) {
                    if !r.name.is_empty() {
                        b.name = r.name;
                    }
                    if !r.tag.is_empty() {
                        b.tag = r.tag;
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = vec![own];
    out.extend(order.into_iter().filter_map(|id| boards.remove(&id)));
    out
}

/// Each board's maker: the person whose device wrote its first create.
fn makers(rows: &[&BoardRowAt], rules: &Rules) -> BTreeMap<String, Option<String>> {
    let mut out = BTreeMap::new();
    for row in rows.iter().filter(|r| r.kind == kinds::SPACE_BOARD_CREATED) {
        let Ok(r) = serde_json::from_slice::<SpaceBoardRow>(&row.payload) else {
            continue;
        };
        out.entry(r.board_id)
            .or_insert_with(|| (rules.person)(&row.at.dev));
    }
    out
}

/// The closed boards, each with its close row's `at` (card #690): per
/// board the newest valid `space.board.closed` row stands, so a reopen
/// after a close wins and the other way round. Valid: from the board's
/// maker while a member, or a space owner, at the row's place; never the
/// first board (`own`).
fn closes(own: &str, rows: &[BoardRowAt], rules: &Rules) -> BTreeMap<String, String> {
    let mut sorted: Vec<&BoardRowAt> = rows.iter().collect();
    sorted.sort_by(|a, b| a.at.cmp(&b.at));
    let made = makers(&sorted, rules);
    let mut state: BTreeMap<String, Option<String>> = BTreeMap::new();
    for row in sorted
        .iter()
        .filter(|r| r.kind == kinds::SPACE_BOARD_CLOSED)
    {
        let Ok(r) = serde_json::from_slice::<BoardClosedRow>(&row.payload) else {
            continue;
        };
        if r.board_id.is_empty() || r.board_id == own {
            continue;
        }
        let Some(maker) = made.get(&r.board_id) else {
            continue;
        };
        let by = (rules.person)(&row.at.dev);
        let may =
            (rules.owner)(&row.at) || (by.is_some() && by == *maker && (rules.member)(&row.at));
        if may {
            state.insert(r.board_id, r.closed.then_some(r.at));
        }
    }
    state
        .into_iter()
        .filter_map(|(id, at)| at.map(|at| (id, at)))
        .collect()
}

/// Folds each board's description and whether it is closed (card #690)
/// onto `boards` (from [`fold_space_boards`], the first board first): the
/// description from its create row, then each newer `space.board.described`
/// row from a member. `me` is who may close now: `None` in Personal (the
/// person may), else their person key and whether they own the space.
pub(crate) fn fold_board_state(
    boards: &mut [SpaceBoard],
    rows: &[BoardRowAt],
    rules: &Rules,
    me: Option<(&str, bool)>,
) {
    let Some(own) = boards.first().map(|b| b.board_id.clone()) else {
        return;
    };
    let mut sorted: Vec<&BoardRowAt> = rows.iter().collect();
    sorted.sort_by(|a, b| a.at.cmp(&b.at));
    let made = makers(&sorted, rules);
    let closed = closes(&own, rows, rules);
    let mut seen_create: std::collections::BTreeSet<String> = Default::default();
    for row in sorted {
        let (id, description) = match row.kind.as_str() {
            kinds::SPACE_BOARD_CREATED => {
                let Ok(r) = serde_json::from_slice::<SpaceBoardRow>(&row.payload) else {
                    continue;
                };
                if !seen_create.insert(r.board_id.clone()) {
                    continue;
                }
                (r.board_id, r.description)
            }
            kinds::SPACE_BOARD_DESCRIBED => {
                let Ok(r) = serde_json::from_slice::<BoardDescribedRow>(&row.payload) else {
                    continue;
                };
                if !(rules.member)(&row.at) {
                    continue;
                }
                (r.board_id, r.description)
            }
            _ => continue,
        };
        let Some(b) = boards.iter_mut().find(|b| b.board_id == id) else {
            continue;
        };
        if row.kind == kinds::SPACE_BOARD_CREATED && description.is_empty() {
            continue;
        }
        b.description_by = if description.is_empty() {
            String::new()
        } else {
            (rules.person)(&row.at.dev).unwrap_or_default()
        };
        b.description = description;
    }
    for (i, b) in boards.iter_mut().enumerate() {
        if let Some(at) = closed.get(&b.board_id) {
            b.closed = true;
            b.closed_at = at.clone();
        }
        b.may_close = i > 0
            && match me {
                None => true,
                Some((me, owner)) => {
                    owner
                        || made
                            .get(&b.board_id)
                            .is_some_and(|m| m.as_deref() == Some(me))
                }
            };
    }
}

/// The space.board.* rows of a space's log.
fn space_board_rows(st: &store::Store) -> store::Result<Vec<BoardRowAt>> {
    Ok(st
        .list(&Filter {
            kinds: vec![
                kinds::SPACE_BOARD_CREATED.into(),
                kinds::SPACE_BOARD_RENAMED.into(),
                kinds::SPACE_BOARD_DESCRIBED.into(),
                kinds::SPACE_BOARD_CLOSED.into(),
            ],
            limit: 10_000,
            ..Default::default()
        })?
        .into_iter()
        .map(|ev| BoardRowAt {
            at: coord::RowPos::of(&ev),
            kind: ev.kind,
            payload: ev.payload,
        })
        .collect())
}

/// Whether a space's delete at this place came from a device of someone
/// who was an active member there: any member may delete a board, as they
/// may add one, and the fold can't take a row's `by` on trust (rule 7).
pub(crate) fn member_wrote(
    v: &std::sync::Arc<irori_identity::board::BoardVerifier>,
) -> impl Fn(&coord::RowPos) -> bool + use<> {
    let v = v.clone();
    move |at: &coord::RowPos| {
        let Some(person) = v.person_of(&at.dev) else {
            return false;
        };
        v.membership().active_at(
            &person,
            &irori_identity::membership::RowAt {
                hlc: at.hlc,
                device: at.dev.clone(),
                seq: at.seq,
            },
        )
    }
}

/// What coord needs of the board list in log `st`, whose first board is
/// `own`.
pub(crate) fn board_list_in(
    st: &store::Store,
    own: &str,
    rules: &Rules,
) -> Option<coord::BoardList> {
    let rows = space_board_rows(st).ok()?;
    Some(coord::BoardList {
        own: own.to_string(),
        deleted: deletes(own, &rows, rules.member).into_iter().collect(),
        tags: tag_ids(&rows),
        closed: closes(own, &rows, rules).into_keys().collect(),
    })
}

/// Every (tag, board) in these rows.
fn tag_ids(rows: &[BoardRowAt]) -> Vec<(String, String)> {
    rows.iter()
        .filter_map(|r| serde_json::from_slice::<SpaceBoardRow>(&r.payload).ok())
        .filter(|r| !r.tag.is_empty())
        .map(|r| (r.tag, r.board_id))
        .collect()
}

/// Every (tag, board) the log whose store is `st` has named inside it.
pub(crate) fn board_tag_ids(st: &store::Store) -> Vec<(String, String)> {
    tag_ids(&space_board_rows(st).unwrap_or_default())
}

/// The tag of board `board` inside the space whose log is `st`; None when
/// the log holds no such board, or it was deleted.
pub(crate) fn board_tag_in(st: &store::Store, board: &str, may: MayDelete) -> Option<String> {
    let own = SpaceBoard::default();
    fold_space_boards(own, &space_board_rows(st).ok()?, may)
        .into_iter()
        .skip(1)
        .find(|b| b.board_id == board)
        .map(|b| b.tag)
}

/// Every tag the log whose store is `st` has named for a board inside it.
pub(crate) fn board_tags_in(st: &store::Store) -> Vec<String> {
    board_tag_ids(st).into_iter().map(|(t, _)| t).collect()
}

impl Services {
    /// Whether `id` is a hidden space: one with no boards, that never gets
    /// any and takes no card (spaces-boards.md §1). These are #538's DM
    /// spaces, known by their private row only, never by a tag or a name.
    pub(crate) fn is_hidden_space(&self, id: &str) -> bool {
        self.is_dm_space(id)
    }

    /// The boards in space `space_id`, its own first; a hidden space has
    /// none.
    pub(crate) fn space_boards(&self, space_id: &str) -> Result<Vec<SpaceBoard>> {
        if self.is_hidden_space(space_id) {
            return Ok(Vec::new());
        }
        let b = self.board(space_id)?;
        let m = b.verifier.membership();
        let own = SpaceBoard {
            board_id: space_id.to_string(),
            name: m.name.clone(),
            tag: m.tag.clone(),
            ..Default::default()
        };
        let rows = space_board_rows(&b.store).map_err(Error::from)?;
        let me = self.person_id();
        let me_key = format!("{}{me}", kinds::PERSON_ACTOR_PREFIX);
        let owner = m.owner(&me);
        Ok(with_space_rules(&b.verifier, |rules| {
            let mut out = fold_space_boards(own, &rows, rules.member);
            fold_board_state(&mut out, &rows, rules, Some((&me_key, owner)));
            out
        }))
    }

    /// What's known of each other member device of space `b` (no-server.md
    /// §5): its latest word is a live hello, else the newer of a saved hello
    /// and its `board.server` row. All but the live hello need no mesh.
    /// Too old is below [`SPACES_PROTOCOL`].
    pub(crate) fn space_readers(&self, b: &BoardLog) -> Result<SpaceReaders> {
        self.space_readers_at(b, ReadsLevel::Spaces, SPACES_PROTOCOL)
    }

    /// [`Services::space_readers`] for any level: too old is a latest word
    /// whose `level` is below `need`.
    pub(crate) fn space_readers_at(
        &self,
        b: &BoardLog,
        level: ReadsLevel,
        need: i64,
    ) -> Result<SpaceReaders> {
        let ms = b.verifier.membership();
        let me = self.device.id.clone();
        let mesh = self.mesh.board(&b.id);
        let devices: Vec<(String, String)> = match &mesh {
            Some(m) => m
                .member_devices()
                .into_iter()
                .map(|d| (d.person_id, d.device_id))
                .collect(),
            // No mesh for it (stopped, or a test without a network).
            None => ms
                .members
                .iter()
                .filter(|p| ms.active(&p.person_id))
                .flat_map(|p| {
                    b.verifier
                        .devices(&p.person_id)
                        .into_iter()
                        .filter(|d| !d.revoked && d.cert.device_id != me)
                        .map(|d| (p.person_id.clone(), d.cert.device_id))
                        .collect::<Vec<_>>()
                })
                .collect(),
        };
        let live = mesh.map(|m| m.live_reads()).unwrap_or_default();
        let words = DeviceWords::read(&b.store).map_err(|e| Error::other(e.0))?;
        let mut out = SpaceReaders::default();
        for (person, dev) in devices {
            let word = live.get(&dev).cloned().or_else(|| words.reads(&dev));
            match word.map(|r| level.of(&r)) {
                None => out.unheard.push((person, dev)),
                Some(l) if l < need => out.too_old.push((person, dev)),
                Some(_) => {}
            }
        }
        Ok(out)
    }

    /// Whether every other member device of space `space_id` is known to
    /// store an unknown `call.*` or `talk.*` row it pulls instead of
    /// stalling on it (card #652, hello `tolerant` >= [`CALLS_TOLERANT`]).
    /// Unlike the space gate, a device nobody has any word from counts
    /// against it: calls have somewhere else to put what they'd write (a
    /// `talk.post` marker, a file), so this gate fails closed
    /// (irori-fit.md §2, "Recommended path").
    pub(crate) fn space_tolerance(&self, space_id: &str) -> Result<SpaceTolerance> {
        let b = self.board(space_id)?;
        let ms = b.verifier.membership();
        let r = self.space_readers_at(&b, ReadsLevel::Tolerant, CALLS_TOLERANT)?;
        let name = |(p, d): (String, String)| {
            let who = b
                .verifier
                .profile(&p)
                .or_else(|| ms.person(&p).map(|x| x.name.clone()))
                .unwrap_or_else(|| short(&p).to_string());
            format!("{who}/{}", short(&d))
        };
        Ok(SpaceTolerance {
            too_old: r.too_old.into_iter().map(name).collect(),
            unheard: r.unheard.into_iter().map(name).collect(),
        })
    }

    /// The member devices of space `b` too old for `level`'s `need`, and
    /// those nobody has any word from, each as `person/device` (short
    /// ids): a gate that fails closed (card #1077, the PR card's).
    pub(crate) fn space_waiting_at(
        &self,
        b: &BoardLog,
        level: ReadsLevel,
        need: i64,
    ) -> Result<(Vec<String>, Vec<String>)> {
        let ms = b.verifier.membership();
        let r = self.space_readers_at(b, level, need)?;
        let name = |(p, d): (String, String)| {
            let who = b
                .verifier
                .profile(&p)
                .or_else(|| ms.person(&p).map(|x| x.name.clone()))
                .unwrap_or_else(|| short(&p).to_string());
            format!("{who}/{}", short(&d))
        };
        Ok((
            r.too_old.into_iter().map(&name).collect(),
            r.unheard.into_iter().map(&name).collect(),
        ))
    }

    /// The member devices of space `space_id` known to be too old for space
    /// rows, as `person/device` (short ids). Empty: it may write them. One
    /// nobody has any word from doesn't count: it gets a warning line
    /// instead ([`SpaceReaders::unheard`]).
    pub(crate) fn space_not_ready(&self, space_id: &str) -> Result<Vec<String>> {
        let b = self.board(space_id)?;
        let ms = b.verifier.membership();
        let devices = self.space_readers(&b)?.too_old;
        let name = |person: &str| {
            b.verifier
                .profile(person)
                .or_else(|| ms.person(person).map(|p| p.name.clone()))
                .unwrap_or_else(|| short(person).to_string())
        };
        Ok(devices
            .into_iter()
            .map(|(p, d)| format!("{}/{}", name(&p), short(&d)))
            .collect())
    }

    /// Adds board `name` (tag `tag`) to space `space_id` and returns its id.
    /// Refused until every member device reads space rows, naming the ones
    /// that don't yet.
    pub(crate) fn create_space_board(
        &self,
        space_id: &str,
        name: &str,
        tag: &str,
    ) -> Result<String> {
        self.create_space_board_with(space_id, name, tag, "")
    }

    /// [`Services::create_space_board`] with the new board's description
    /// (card #690), on its create row.
    pub(crate) fn create_space_board_with(
        &self,
        space_id: &str,
        name: &str,
        tag: &str,
        description: &str,
    ) -> Result<String> {
        let description = description.trim();
        check_description(description)?;
        if self.is_hidden_space(space_id) {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!("space {space_id} holds no boards"),
            ));
        }
        let (name, tag) = (name.trim(), tag.trim());
        check_board_name(name)?;
        check_board_tag(tag)?;
        let b = self.board(space_id)?;
        let me = self.person_id();
        if !b.verifier.membership().active(&me) {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!("you aren't a member of space {space_id}"),
            ));
        }
        if let Some(other) = self.board_tag_holder(tag) {
            return Err(tag_taken(tag, &other));
        }
        if let Some(why) = gate_refusal(&self.space_not_ready(space_id)?) {
            return Err(Error::coord(CK::NotAllowed, why));
        }
        let id = super::boards::new_board_id()?;
        let row = SpaceBoardRow {
            board_id: id.clone(),
            name: name.to_string(),
            tag: tag.to_string(),
            description: description.to_string(),
            deleted: false,
            by: format!("{}{me}", kinds::PERSON_ACTOR_PREFIX),
            at: (self.now)().format_rfc3339_nano(),
        };
        b.store.append(
            &format!("{}{me}", kinds::PERSON_ACTOR_PREFIX),
            kinds::SPACE_BOARD_CREATED,
            &row,
        )?;
        self.boards.moved();
        Ok(id)
    }

    /// Renames board `board_id` of space `space_id`, its first board too:
    /// any active member may, as for adding one. Only the board's name
    /// changes, so no ref does and its tag needs no check, and a first
    /// board's rename leaves the space's own name alone (card #558).
    /// Refused until every member device reads space rows.
    pub(crate) fn rename_space_board(
        &self,
        space_id: &str,
        board_id: &str,
        name: &str,
    ) -> Result<()> {
        let name = name.trim();
        check_board_name(name)?;
        let b = self.board(space_id)?;
        let me = self.person_id();
        if !b.verifier.membership().active(&me) {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!("you aren't a member of space {space_id}"),
            ));
        }
        if !self
            .space_boards(space_id)?
            .iter()
            .any(|x| x.board_id == board_id)
        {
            return Err(Error::coord(
                CK::NotFound,
                format_args!("no board {board_id} in space {space_id}"),
            ));
        }
        if let Some(why) = gate_refusal(&self.space_not_ready(space_id)?) {
            return Err(Error::coord(CK::NotAllowed, why));
        }
        let by = format!("{}{me}", kinds::PERSON_ACTOR_PREFIX);
        let row = SpaceBoardRow {
            board_id: board_id.to_string(),
            name: name.to_string(),
            tag: String::new(),
            deleted: false,
            description: String::new(),
            by: by.clone(),
            at: (self.now)().format_rfc3339_nano(),
        };
        b.store.append(&by, kinds::SPACE_BOARD_RENAMED, &row)?;
        self.boards.moved();
        Ok(())
    }

    /// Deletes board `board_id` of space `space_id`: any active member may,
    /// as for adding one. No row is written per card: the fold puts its
    /// cards on the first board, with new numbers there (card #563). The
    /// first board can't go. Refused, like adding one, until every member
    /// device reads space rows.
    pub(crate) fn delete_space_board(&self, space_id: &str, board_id: &str) -> Result<()> {
        let b = self.board(space_id)?;
        let me = self.person_id();
        if !b.verifier.membership().active(&me) {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!("you aren't a member of space {space_id}"),
            ));
        }
        if board_id == space_id {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!("a space's first board can't be deleted"),
            ));
        }
        if !self
            .space_boards(space_id)?
            .iter()
            .any(|x| x.board_id == board_id)
        {
            return Err(Error::coord(
                CK::NotFound,
                format_args!("no board {board_id} in space {space_id}"),
            ));
        }
        if let Some(why) = gate_refusal(&self.space_not_ready(space_id)?) {
            return Err(Error::coord(CK::NotAllowed, why));
        }
        let by = format!("{}{me}", kinds::PERSON_ACTOR_PREFIX);
        let row = SpaceBoardRow {
            board_id: board_id.to_string(),
            deleted: true,
            by: by.clone(),
            at: (self.now)().format_rfc3339_nano(),
            ..Default::default()
        };
        b.store.append(&by, kinds::SPACE_BOARD_RENAMED, &row)?;
        b.coord().refresh_boards();
        self.boards.moved();
        Ok(())
    }

    /// The member devices of space `space_id` known to be too old to store
    /// a `space.board.described` or `space.board.closed` row (hello
    /// `tolerant` below [`BOARD_ROWS_TOLERANT`], card #690), as
    /// `person/device`. One nobody has heard from doesn't count, as for the
    /// spaces gate: closing has no fallback.
    pub(crate) fn board_rows_not_ready(&self, space_id: &str) -> Result<Vec<String>> {
        let b = self.board(space_id)?;
        let ms = b.verifier.membership();
        let r = self.space_readers_at(&b, ReadsLevel::Tolerant, BOARD_ROWS_TOLERANT)?;
        Ok(r.too_old
            .into_iter()
            .map(|(p, d)| {
                let who = b
                    .verifier
                    .profile(&p)
                    .or_else(|| ms.person(&p).map(|x| x.name.clone()))
                    .unwrap_or_else(|| short(&p).to_string());
                format!("{who}/{}", short(&d))
            })
            .collect())
    }

    /// Board `board_id` of space `space_id`, checked for a write by this
    /// person: an active member, a board the space holds, and every device
    /// able to store the new rows (card #690). With the space's boards,
    /// the first first.
    fn checked_board(
        &self,
        space_id: &str,
        board_id: &str,
    ) -> Result<(std::sync::Arc<BoardLog>, SpaceBoard, Vec<SpaceBoard>)> {
        let b = self.board(space_id)?;
        let me = self.person_id();
        if !b.verifier.membership().active(&me) {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!("you aren't a member of space {space_id}"),
            ));
        }
        let all = self.space_boards(space_id)?;
        let Some(x) = all.iter().find(|x| x.board_id == board_id).cloned() else {
            return Err(Error::coord(
                CK::NotFound,
                format_args!("no board {board_id} in space {space_id}"),
            ));
        };
        if let Some(why) = board_rows_refusal(&self.board_rows_not_ready(space_id)?) {
            return Err(Error::coord(CK::NotAllowed, why));
        }
        Ok((b, x, all))
    }

    /// Sets the description of board `board_id` in space `space_id`, its
    /// first board too (card #690); empty clears it. Any active member
    /// may, as for a rename. Not on a closed board: reopen it first.
    pub(crate) fn set_space_board_description(
        &self,
        space_id: &str,
        board_id: &str,
        description: &str,
    ) -> Result<()> {
        let description = description.trim();
        check_description(description)?;
        let (b, x, _) = self.checked_board(space_id, board_id)?;
        if x.closed {
            return Err(closed_board(&x));
        }
        let by = self.person_actor();
        let row = BoardDescribedRow {
            board_id: board_id.to_string(),
            description: description.to_string(),
            by: by.clone(),
            at: (self.now)().format_rfc3339_nano(),
        };
        b.store.append(&by, kinds::SPACE_BOARD_DESCRIBED, &row)?;
        self.boards.moved();
        Ok(())
    }

    /// Closes board `board_id` of space `space_id` (card #690), its open
    /// cards first as `open` says. Its maker or a space owner may; never
    /// the first board.
    pub(crate) fn close_space_board(
        &self,
        space_id: &str,
        board_id: &str,
        open: OpenCards,
    ) -> Result<()> {
        self.set_space_board_closed(space_id, board_id, Some(open))
    }

    /// Reopens closed board `board_id` of space `space_id` (card #690):
    /// who may close it may.
    pub(crate) fn reopen_space_board(&self, space_id: &str, board_id: &str) -> Result<()> {
        self.set_space_board_closed(space_id, board_id, None)
    }

    /// A close (`Some`, with what to do with its open cards) or a reopen.
    fn set_space_board_closed(
        &self,
        space_id: &str,
        board_id: &str,
        open: Option<OpenCards>,
    ) -> Result<()> {
        if board_id == space_id {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!("a space's first board is never closed"),
            ));
        }
        let (b, x, all) = self.checked_board(space_id, board_id)?;
        if !x.may_close {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!(
                    "only the person who made board {} or an owner of the space may {} it",
                    gocompat::quote::quote(&x.name),
                    if open.is_some() { "close" } else { "reopen" }
                ),
            ));
        }
        if x.closed == open.is_some() {
            return Ok(());
        }
        let by = self.person_actor();
        if let Some(open) = &open {
            clear_open(b.coord(), &by, &x, &all, open)?;
        }
        let row = BoardClosedRow {
            board_id: board_id.to_string(),
            closed: open.is_some(),
            by: by.clone(),
            at: (self.now)().format_rfc3339_nano(),
        };
        b.store.append(&by, kinds::SPACE_BOARD_CLOSED, &row)?;
        b.coord().refresh_boards();
        self.boards.moved();
        Ok(())
    }

    /// Sets the description of one of Personal's boards, its default too
    /// (card #690); empty clears it.
    pub(crate) fn set_personal_board_description(
        &self,
        board_id: &str,
        description: &str,
    ) -> Result<()> {
        let description = description.trim();
        check_description(description)?;
        let (x, _) = self.personal_board(board_id)?;
        if x.closed {
            return Err(closed_board(&x));
        }
        self.append_personal_row(
            kinds::SPACE_BOARD_DESCRIBED,
            BoardDescribedRow {
                board_id: board_id.to_string(),
                description: description.to_string(),
                by: format!("user:{}", self.device.id),
                at: (self.now)().format_rfc3339_nano(),
            },
        )
    }

    /// Closes one of Personal's boards (card #690), its open cards first
    /// as `open` says. The default board never closes.
    pub(crate) fn close_personal_board(&self, board_id: &str, open: OpenCards) -> Result<()> {
        self.set_personal_board_closed(board_id, Some(open))
    }

    /// Reopens one of Personal's closed boards (card #690).
    pub(crate) fn reopen_personal_board(&self, board_id: &str) -> Result<()> {
        self.set_personal_board_closed(board_id, None)
    }

    fn set_personal_board_closed(&self, board_id: &str, open: Option<OpenCards>) -> Result<()> {
        if board_id == PERSONAL_BOARD {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!("Personal's default board is never closed"),
            ));
        }
        let (x, all) = self.personal_board(board_id)?;
        if x.closed == open.is_some() {
            return Ok(());
        }
        if let Some(open) = &open {
            clear_open(&self.coord, &self.user_sender().key, &x, &all, open)?;
        }
        self.append_personal_row(
            kinds::SPACE_BOARD_CLOSED,
            BoardClosedRow {
                board_id: board_id.to_string(),
                closed: open.is_some(),
                by: format!("user:{}", self.device.id),
                at: (self.now)().format_rfc3339_nano(),
            },
        )
    }

    /// One of Personal's boards, with all of them (its default first).
    fn personal_board(&self, board_id: &str) -> Result<(SpaceBoard, Vec<SpaceBoard>)> {
        let all = self.personal_boards()?;
        let x = all
            .iter()
            .find(|x| x.board_id == board_id)
            .cloned()
            .ok_or_else(|| {
                Error::coord(
                    CK::NotFound,
                    format_args!("no board {board_id} in Personal"),
                )
            })?;
        Ok((x, all))
    }

    fn append_personal_row(&self, kind: &str, row: impl Serialize) -> Result<()> {
        self.store.append("user", kind, &row)?;
        self.coord.refresh_boards();
        self.boards.moved();
        Ok(())
    }

    /// Personal's boards (card #558), its default first.
    pub(crate) fn personal_boards(&self) -> Result<Vec<SpaceBoard>> {
        let own = SpaceBoard {
            board_id: PERSONAL_BOARD.to_string(),
            name: PERSONAL_NAME.to_string(),
            ..Default::default()
        };
        let rows = space_board_rows(&self.store).map_err(Error::from)?;
        Ok(with_personal_rules(|rules| {
            let mut out = fold_space_boards(own, &rows, rules.member);
            fold_board_state(&mut out, &rows, rules, None);
            out
        }))
    }

    /// Adds board `name` (tag `tag`) to Personal and returns its id. It is
    /// the person's own log, so no gate and no membership: only the tag has
    /// to be free among every space here.
    pub(crate) fn create_personal_board(&self, name: &str, tag: &str) -> Result<String> {
        self.create_personal_board_with(name, tag, "")
    }

    /// [`Services::create_personal_board`] with the new board's
    /// description (card #690).
    pub(crate) fn create_personal_board_with(
        &self,
        name: &str,
        tag: &str,
        description: &str,
    ) -> Result<String> {
        let description = description.trim();
        check_description(description)?;
        let (name, tag) = (name.trim(), tag.trim());
        check_board_name(name)?;
        check_board_tag(tag)?;
        if let Some(other) = self.board_tag_holder(tag) {
            return Err(tag_taken(tag, &other));
        }
        let id = super::boards::new_board_id()?;
        self.append_personal(
            kinds::SPACE_BOARD_CREATED,
            SpaceBoardRow {
                board_id: id.clone(),
                name: name.to_string(),
                tag: tag.to_string(),
                description: description.to_string(),
                ..Default::default()
            },
        )?;
        Ok(id)
    }

    /// Renames one of Personal's boards, its default ([`PERSONAL_BOARD`])
    /// too. Only the name changes, so no ref does.
    pub(crate) fn rename_personal_board(&self, board_id: &str, name: &str) -> Result<()> {
        let name = name.trim();
        check_board_name(name)?;
        if !self
            .personal_boards()?
            .iter()
            .any(|x| x.board_id == board_id)
        {
            return Err(Error::coord(
                CK::NotFound,
                format_args!("no board {board_id} in Personal"),
            ));
        }
        self.append_personal(
            kinds::SPACE_BOARD_RENAMED,
            SpaceBoardRow {
                board_id: board_id.to_string(),
                name: name.to_string(),
                ..Default::default()
            },
        )
    }

    /// Deletes one of Personal's boards; its cards go to the default board
    /// (card #563), which can't be deleted itself.
    pub(crate) fn delete_personal_board(&self, board_id: &str) -> Result<()> {
        if board_id == PERSONAL_BOARD {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!("Personal's default board can't be deleted"),
            ));
        }
        if !self
            .personal_boards()?
            .iter()
            .any(|x| x.board_id == board_id)
        {
            return Err(Error::coord(
                CK::NotFound,
                format_args!("no board {board_id} in Personal"),
            ));
        }
        self.append_personal(
            kinds::SPACE_BOARD_RENAMED,
            SpaceBoardRow {
                board_id: board_id.to_string(),
                deleted: true,
                ..Default::default()
            },
        )
    }

    /// Writes a `space.board.*` row to the private log, as the person on
    /// this device.
    fn append_personal(&self, kind: &str, row: SpaceBoardRow) -> Result<()> {
        let row = SpaceBoardRow {
            by: format!("user:{}", self.device.id),
            at: (self.now)().format_rfc3339_nano(),
            ..row
        };
        self.store.append("user", kind, &row)?;
        self.coord.refresh_boards();
        self.boards.moved();
        Ok(())
    }

    /// The board (a space's own, one inside a space, or one of Personal's)
    /// already using `tag` on this device, if any.
    pub(crate) fn board_tag_holder(&self, tag: &str) -> Option<String> {
        let own = self.tag_holder(tag);
        if !own.is_empty() {
            return Some(own);
        }
        // Every tag a log has named, a deleted board's too: its old refs
        // still mean its cards, so it never goes to a new board (card #563).
        if let Some((_, id)) = board_tag_ids(&self.store)
            .into_iter()
            .find(|(t, _)| t == tag)
        {
            return Some(id);
        }
        for b in self.visible_boards() {
            if let Some((_, id)) = board_tag_ids(&b.store).into_iter().find(|(t, _)| t == tag) {
                return Some(id);
            }
        }
        None
    }
}

fn short(id: &str) -> &str {
    &id[..id.len().min(6)]
}

fn check_description(description: &str) -> Result<()> {
    if description.chars().count() <= MAX_BOARD_DESCRIPTION {
        return Ok(());
    }
    Err(Error::coord(
        CK::Invalid,
        format_args!("a board's description is {MAX_BOARD_DESCRIPTION} characters at most"),
    ))
}

fn closed_board(x: &SpaceBoard) -> Error {
    Error::coord(
        CK::NotAllowed,
        format_args!(
            "board {} is closed: reopen it first",
            gocompat::quote::quote(&x.name)
        ),
    )
}

/// The note on a card canceled because its board closed (card #690).
pub const CLOSED_BOARD_NOTE: &str = "its board was closed";

/// Up to eight refs, then "and N more".
fn card_list(refs: &[String]) -> String {
    let shown = refs.iter().take(8).cloned().collect::<Vec<_>>().join(", ");
    if refs.len() > 8 {
        format!("{shown} and {} more", refs.len() - 8)
    } else {
        shown
    }
}

/// Refuses a close while cards on the board are open, naming them.
fn open_cards_refusal(x: &SpaceBoard, open: &[coord::Task]) -> Result<()> {
    if open.is_empty() {
        return Ok(());
    }
    let refs: Vec<String> = open.iter().map(|t| t.short_ref()).collect();
    Err(Error::coord(
        CK::BadState,
        format_args!(
            "board {} has {} open card{} ({}): move or cancel them, or close the board with them moved or canceled",
            gocompat::quote::quote(&x.name),
            open.len(),
            if open.len() == 1 { "" } else { "s" },
            card_list(&refs),
        ),
    ))
}

/// Clears board `x`'s open cards before it closes (card #690), as `open`
/// says, writing on `co` as `by` (the person). `all` is its space's
/// boards (Personal's), the first first. A move is checked for every card
/// before the first one moves. A card it couldn't handle is named in the
/// error, and the board stays open.
fn clear_open(
    co: &coord::Coord,
    by: &str,
    x: &SpaceBoard,
    all: &[SpaceBoard],
    open: &OpenCards,
) -> Result<()> {
    let cards = co.open_on_board(&x.board_id);
    if cards.is_empty() {
        return Ok(());
    }
    let failed = match open {
        OpenCards::Keep => return open_cards_refusal(x, &cards),
        OpenCards::MoveTo(to) => {
            let to = to.trim();
            let Some(target) = all.iter().find(|b| b.board_id == to) else {
                return Err(Error::coord(
                    CK::NotFound,
                    format_args!("no board {to} to move the cards to"),
                ));
            };
            if target.board_id == x.board_id {
                return Err(Error::coord(
                    CK::Invalid,
                    format_args!("pick another board to move the cards to"),
                ));
            }
            if target.closed {
                return Err(closed_board(target));
            }
            // A subtask moves with its parent, so one whose parent isn't
            // open here can't move; nor can an open ask.
            let here: std::collections::BTreeSet<&str> =
                cards.iter().map(|t| t.id.as_str()).collect();
            let stuck: Vec<String> = cards
                .iter()
                .filter_map(|t| {
                    if t.open_ask().is_some() {
                        Some(format!("{} is an open ask", t.short_ref()))
                    } else if !t.parent.is_empty() && !here.contains(t.parent.as_str()) {
                        Some(format!(
                            "{} is a subtask whose parent isn't open on this board",
                            t.short_ref()
                        ))
                    } else {
                        None
                    }
                })
                .collect();
            if !stuck.is_empty() {
                return Err(Error::coord(
                    CK::BadState,
                    format_args!(
                        "board {} stays open: {} can't move. Close or cancel {} first, or close the board canceling its cards",
                        gocompat::quote::quote(&x.name),
                        card_list(&stuck),
                        if stuck.len() == 1 { "it" } else { "them" },
                    ),
                ));
            }
            cards
                .iter()
                .filter(|t| t.parent.is_empty())
                .filter_map(|t| {
                    co.move_task(by, &t.id, &target.board_id)
                        .err()
                        .map(|e| format!("{} ({e})", t.short_ref()))
                })
                .collect::<Vec<_>>()
        }
        OpenCards::Cancel => cards
            .iter()
            .filter_map(|t| {
                // A parent's cancel already ended its subtasks.
                if co
                    .get_task(&t.id)
                    .is_none_or(|now| coord::is_terminal(&now.state))
                {
                    return None;
                }
                co.set_task_state(by, &t.id, coord::CANCELED, CLOSED_BOARD_NOTE)
                    .err()
                    .map(|e| format!("{} ({e})", t.short_ref()))
            })
            .collect::<Vec<_>>(),
    };
    let left: Vec<String> = if failed.is_empty() {
        co.open_on_board(&x.board_id)
            .iter()
            .map(|t| t.short_ref())
            .collect()
    } else {
        failed
    };
    if left.is_empty() {
        return Ok(());
    }
    Err(Error::coord(
        CK::BadState,
        format_args!(
            "board {} stays open: {} couldn't be {}",
            gocompat::quote::quote(&x.name),
            card_list(&left),
            if matches!(open, OpenCards::Cancel) {
                "canceled"
            } else {
                "moved"
            },
        ),
    ))
}

fn check_board_name(name: &str) -> Result<()> {
    if irori_identity::membership::valid_board_name(name) {
        return Ok(());
    }
    Err(Error::coord(
        CK::Invalid,
        format_args!(
            "a board name is 1 to {} characters",
            irori_identity::membership::MAX_BOARD_NAME
        ),
    ))
}

fn check_board_tag(tag: &str) -> Result<()> {
    if irori_identity::membership::valid_board_tag(tag) {
        return Ok(());
    }
    Err(Error::coord(
        CK::Invalid,
        format_args!(
            "a board tag is a lowercase letter, then lowercase letters, digits or '-', {} characters at most",
            irori_identity::membership::MAX_BOARD_TAG
        ),
    ))
}

fn tag_taken(tag: &str, other: &str) -> Error {
    Error::coord(
        CK::Invalid,
        format_args!(
            "the tag {} is already board {other}'s",
            gocompat::quote::quote(tag)
        ),
    )
}

/// [`Services::space_readers`]: the other member devices of a space that
/// aren't known to read space rows, as (person, device).
#[derive(Debug, Default)]
pub(crate) struct SpaceReaders {
    /// Their latest word is below [`SPACES_PROTOCOL`] (or the level
    /// [`Services::space_readers_at`] was given): they block.
    pub(crate) too_old: Vec<(String, String)>,
    /// No word at all: they don't block the space gate, and get a warning
    /// line.
    pub(crate) unheard: Vec<(String, String)>,
}

/// Which level of a device's word [`Services::space_readers_at`] checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadsLevel {
    /// The hello's `spaces`.
    Spaces,
    /// The hello's `tolerant` (card #652).
    Tolerant,
    /// The hello's `reviews` (card #1077): PR cards.
    Reviews,
}

impl ReadsLevel {
    fn of(self, r: &BoardReads) -> i64 {
        match self {
            ReadsLevel::Spaces => r.spaces,
            ReadsLevel::Tolerant => r.tolerant,
            ReadsLevel::Reviews => r.reviews,
        }
    }
}

/// The `tolerant` level that stores unknown `call.*` and `talk.*` rows
/// (card #652). Fixed at 2: [`mesh::boardmesh::TOLERANT_PROTOCOL`] rises
/// when a later family joins, and calls must not wait on that.
pub const CALLS_TOLERANT: i64 = 2;

/// The `tolerant` level that stores unknown `space.*` rows (card #579):
/// what a space needs of every member device before it writes a board's
/// description or close row (card #690). No protocol bump.
pub const BOARD_ROWS_TOLERANT: i64 = 1;

/// [`gate_refusal`] for the board rows of card #690.
pub(crate) fn board_rows_refusal(waiting: &[String]) -> Option<String> {
    (!waiting.is_empty()).then(|| {
        format!(
            "every device in the space must run Irori 0.12.4 or later first; waiting on {}",
            waiting.join(", ")
        )
    })
}

/// The calls gate's answer for one space ([`super::Core::space_tolerance`]):
/// the other member devices not known to be at [`CALLS_TOLERANT`], as
/// `person/device` (short ids).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceTolerance {
    /// Their latest word says a lower `tolerant`: an Irori before this one,
    /// which would stall on a `call.*` row.
    pub too_old: Vec<String>,
    /// Nobody has any word from them, so they may be that old.
    pub unheard: Vec<String>,
}

impl SpaceTolerance {
    /// Every other device in the space is known to be at
    /// [`CALLS_TOLERANT`]: the space may take `call.*` rows.
    pub fn ready(&self) -> bool {
        self.too_old.is_empty() && self.unheard.is_empty()
    }
}

/// The version gate's answer (card #392, spaces.md §4): why a space may
/// not take a space-only row yet, given the member devices known to be too
/// old for them (`space_not_ready`); None when it may.
///
/// One gate covers `space.board.*` and a space's `talk.*` rows alike. The
/// hello's `spaces` (SPACES_PROTOCOL 1) came after Talk 1 marked `talk.*`
/// as accepted on board logs, so any Irori that says `spaces >= 1` stores
/// both; a separate number for channels would wait on nothing more.
pub(crate) fn gate_refusal(waiting: &[String]) -> Option<String> {
    (!waiting.is_empty()).then(|| {
        format!(
            "every device in the space must run an Irori with spaces first; waiting on {}",
            waiting.join(", ")
        )
    })
}

// ---- a space's channels (Talk's rows in a space's log) -----------------------

/// The Talk kinds a space's log carries (read cursors never do): the
/// talk.* rows, and a huddle's transcript lines (card #656).
const SPACE_TALK_KINDS: [&str; 10] = [
    kinds::TALK_CHANNEL_CREATED,
    kinds::TALK_CHANNEL_EDITED,
    kinds::TALK_MEMBER,
    kinds::TALK_POST,
    kinds::TALK_CARD,
    kinds::CALL_SEGMENT,
    kinds::CALL_TITLE,
    kinds::CALL_ASSIST,
    kinds::CALL_ITEM,
    kinds::CALL_ITEM_SET,
];

/// Why a space won't take a `call.segment` row yet (card #656): a device in
/// it may be too old to store one. The caller keeps the lines for a
/// fallback post instead.
pub const TRANSCRIPT_NOT_READY: &str =
    "a device in this space can't store call rows yet, so it takes no transcript";

/// Rewrites the keys a talk.* payload names (`by`, `member.key`, each of
/// `mentions` and of `call.people`) with `f`; a key `f` turns into None is
/// dropped from those lists and refuses the row anywhere else.
fn map_talk_keys(
    v: &mut serde_json::Value,
    mut f: impl FnMut(&str) -> Option<String>,
) -> std::result::Result<(), String> {
    let Some(o) = v.as_object_mut() else {
        return Err("a talk row is an object".into());
    };
    if let Some(serde_json::Value::String(by)) = o.get_mut("by") {
        *by = f(by).ok_or_else(|| format!("{by} can't write in this space"))?;
    }
    if let Some(serde_json::Value::String(k)) = o.get_mut("member").and_then(|m| m.get_mut("key")) {
        *k = f(k).ok_or_else(|| format!("{k} can't be in this space's channel"))?;
    }
    // A canvas item's asker (card #1051) is a person, mapped as `by` is;
    // display only, so one that can't be mapped is dropped, not refused.
    if let Some(a) = o.remove("asker") {
        let mapped = a
            .as_str()
            .filter(|k| coord::talk::is_person_key(k) || coord::talk::is_other_person_key(k))
            .and_then(&mut f);
        if let Some(k) = mapped {
            o.insert("asker".into(), serde_json::Value::String(k));
        }
    }
    if let Some(serde_json::Value::Array(ms)) = o.get_mut("mentions") {
        let keep: Vec<serde_json::Value> = ms
            .iter()
            .filter_map(|m| m.as_str().and_then(&mut f))
            .map(serde_json::Value::String)
            .collect();
        *ms = keep;
    }
    // A huddle's people (#654) are mapped as mentions are. Only people:
    // `board_key` would mint a card for an agent's key.
    if let Some(serde_json::Value::Array(ps)) = o.get_mut("call").and_then(|c| c.get_mut("people"))
    {
        let keep: Vec<serde_json::Value> = ps
            .iter()
            .filter_map(|p| p.as_str())
            .filter(|p| coord::talk::is_person_key(p) || coord::talk::is_other_person_key(p))
            .filter_map(&mut f)
            .map(serde_json::Value::String)
            .collect();
        *ps = keep;
    }
    Ok(())
}

impl Services {
    /// A space log's talk.* row as this device's Talk folds it: keys back
    /// in this device's form, and a channel in the space whose log holds
    /// it whatever its row says. None: not a row Talk folds.
    ///
    /// A hidden space's row folds only into its pair's DM: it gets a
    /// fold-only `dmConv` when every rule of talk-dms.md §4 holds, and any
    /// other is dropped here (card #538). No row keeps a `dmConv` it
    /// arrived with, from any log: that field is the one door into a DM.
    pub(crate) fn space_talk_row(&self, space_id: &str, ev: &store::Event) -> Option<store::Event> {
        if !SPACE_TALK_KINDS.contains(&ev.kind.as_str()) {
            return None;
        }
        let mut v: serde_json::Value = serde_json::from_slice(&ev.payload).ok()?;
        // A huddle's line is its speaker's own device's (card #656), and a
        // title is a person's (card #1043): a person, signing as
        // themselves.
        // A canvas item's set is a person's too (card #1051); an item is a
        // person's, their assistant's (the person's row, flagged), or an
        // agent's outside hidden spaces. `space_talk_row` checks the actor
        // and the row's `by`; the flag and the status are the fold's.
        let by_actor = v.get("by").and_then(|b| b.as_str()) == Some(ev.actor.as_str());
        let person_actor = ev.actor.starts_with(kinds::PERSON_ACTOR_PREFIX);
        let agent_actor = ev.actor.starts_with(kinds::AGENT_ACTOR_PREFIX);
        let allowed = match ev.kind.as_str() {
            kinds::CALL_SEGMENT | kinds::CALL_TITLE | kinds::CALL_ASSIST | kinds::CALL_ITEM_SET => {
                person_actor && by_actor
            }
            kinds::CALL_ITEM => {
                by_actor
                    && (person_actor || (agent_actor && self.dm_spaces.get(space_id).is_none()))
            }
            _ => true,
        };
        if !allowed {
            return None;
        }
        let dm_conv = match self.dm_spaces.get(space_id) {
            Some(d) => {
                let m = self.board(space_id).ok()?.verifier.membership();
                let me = self.person_id();
                let conv = super::dm_spaces::dm_conv_of(&me, &d, &m, &ev.actor, &ev.kind, &v)?;
                Some(conv)
            }
            None => None,
        };
        map_talk_keys(&mut v, |k| Some(self.private_key(k))).ok()?;
        // Every row says which space carried it: a channel is made there,
        // and the rest may only touch that space's channels (card #461).
        if let Some(o) = v.as_object_mut() {
            o.remove("dmConv");
            o.insert("spaceId".into(), space_id.into());
            if let Some(conv) = dm_conv {
                o.insert("dmConv".into(), conv.into());
            }
        }
        let mut out = ev.clone();
        out.payload = serde_json::to_vec(&v).ok()?;
        Some(out)
    }

    /// Folds space `b`'s talk.* rows into Talk (a space just opened here).
    pub(crate) fn fold_space_talk(&self, b: &super::boards::BoardLog) {
        if self.board_leaving(b) {
            return; // left: out of Talk until it's forgotten (card #461)
        }
        let mut changed = false;
        let f = Filter {
            kinds: SPACE_TALK_KINDS.iter().map(|k| k.to_string()).collect(),
            ..Default::default()
        };
        let _ = super::boards::each_row(&b.store, f, |ev| {
            if let Some(ev) = self.space_talk_row(&b.id, ev) {
                changed |= self.talk.apply(&ev);
            }
        });
        if changed {
            self.hub.publish(crate::events::CoreEvent::Talk(
                crate::events::TalkChange::all(),
            ));
        }
    }

    /// What a row of hidden space `d` may not be, checked before anything
    /// else (card #538): `space_talk_row` runs after the append, and a row
    /// refused there would stay in the log. `board_key` would mint a card
    /// for an agent's key, so this runs before it too.
    fn dm_refusal(
        &self,
        by: &str,
        kind: &str,
        payload: &mut serde_json::Value,
    ) -> std::result::Result<(), String> {
        if !coord::talk::is_person_key(by) || !self.chain.is_owner(by) {
            return Err("only the person writes in a DM with a person".into());
        }
        match kind {
            kinds::TALK_POST
            | kinds::CALL_SEGMENT
            | kinds::CALL_TITLE
            | kinds::CALL_ASSIST
            | kinds::CALL_ITEM
            | kinds::CALL_ITEM_SET => {}
            kinds::TALK_MEMBER => {
                if payload.get("op").and_then(|o| o.as_str()) != Some(coord::talk::JOIN) {
                    return Err("a DM with a person has no leave row".into());
                }
                let key = payload
                    .pointer("/member/key")
                    .and_then(|k| k.as_str())
                    .unwrap_or_default();
                if !coord::talk::is_person_key(key) && !key.starts_with(kinds::PERSON_ACTOR_PREFIX)
                {
                    return Err("only the two people are in a DM with a person".into());
                }
            }
            _ => return Err(format!("{kind} never goes in a DM's log")),
        }
        // Nobody is woken through one, and no card is minted for a mention.
        if let Some(m) = payload.get_mut("mentions") {
            *m = serde_json::Value::Array(Vec::new());
        }
        Ok(())
    }

    /// Appends a talk.* row to space `space_id`'s log as `by` (a key on this
    /// device): refused unless the person is an active member and every
    /// device in the space reads space rows (the version gate). A hidden
    /// space takes only what folds into its DM.
    pub(super) fn append_space_talk(
        &self,
        space_id: &str,
        by: &str,
        kind: &str,
        mut payload: serde_json::Value,
    ) -> std::result::Result<store::Event, String> {
        if !SPACE_TALK_KINDS.contains(&kind) {
            return Err(format!("{kind} never goes in a space's log"));
        }
        let hidden = self.dm_spaces.get(space_id);
        if hidden.is_some() {
            self.dm_refusal(by, kind, &mut payload)?;
        }
        let b = self.board(space_id).map_err(|e| e.to_string())?;
        if !b.verifier.membership().active(&self.person_id()) {
            return Err(format!("you aren't a member of space {space_id}"));
        }
        if let Some(why) = gate_refusal(&self.space_not_ready(space_id).map_err(|e| e.to_string())?)
        {
            return Err(why);
        }
        // A device too old for call.* rows would stall on a line (#652), or
        // on a title (#1043).
        if kind.starts_with("call.")
            && !self
                .space_tolerance(space_id)
                .map_err(|e| e.to_string())?
                .ready()
        {
            return Err(TRANSCRIPT_NOT_READY.into());
        }
        let actor = self.board_key(space_id, by).map_err(|e| e.to_string())?;
        let mut bad = None;
        map_talk_keys(&mut payload, |k| match self.board_key(space_id, k) {
            Ok(x) => Some(x),
            Err(e) => {
                bad.get_or_insert(e.to_string());
                None
            }
        })
        .map_err(|e| bad.clone().unwrap_or(e))?;
        if let Some(d) = &hidden {
            let (m, me) = (b.verifier.membership(), self.person_id());
            if super::dm_spaces::dm_conv_of(&me, d, &m, &actor, kind, &payload).is_none() {
                return Err("that row wouldn't fold into this DM".into());
            }
            // An offer carries the whole log, so a pending one stops
            // growing where a frame stops carrying it.
            if self.dm_pending(&b, d)
                && Services::dm_offer_size(&b)? > super::dm_spaces::MAX_PENDING_LOG
            {
                return Err(
                    "this DM hasn't reached them yet, and can't hold more until it does".into(),
                );
            }
        }
        // A post carries the writer's hop, as a board row does (§7.1): the
        // other person's Core wakes their agent at it, so MAX_HOP stops two
        // people's agents naming each other back and forth (card #461).
        if kind == kinds::TALK_POST
            && let Some(o) = payload.as_object_mut()
        {
            let hop = self.board_hop(&actor);
            if hop > 0 {
                o.insert("hop".into(), hop.into());
            }
        }
        if kind == kinds::TALK_CHANNEL_CREATED
            && let Some(o) = payload.as_object_mut()
        {
            o.insert("spaceId".into(), space_id.into());
        }
        // A post's files are offered in this log first, as a board
        // comment's are (card #643): a member's device may fetch a file
        // only where one of ours offered it here (v0.5 §5.5).
        if kind == kinds::TALK_POST
            && let Some(files) = payload.get("attach").and_then(|a| a.as_array())
        {
            let files: Vec<&serde_json::Value> = files.iter().collect();
            coord::board::offer_files_in(
                &b.store,
                &self.device.id,
                &|sha: &str| self.files.has(sha),
                &files,
            )
            .map_err(|e| e.to_string())?;
        }
        let ev = b
            .store
            .append(&actor, kind, &payload)
            .map_err(|e| e.to_string())?;
        self.space_talk_row(space_id, &ev)
            .ok_or_else(|| "the row didn't read back".to_string())
    }
}

impl Services {
    /// Makes space `space_id`'s #office-assistants channel if Talk hasn't
    /// folded it (assistants-talk.md §4). Not in a hidden DM space, which
    /// takes only what folds into its DM, and never in Personal (not a
    /// board here). A space that can't take the row yet (its gate, or not
    /// a member) is retried by the next sweep.
    pub(crate) fn ensure_assistants_channel(&self, space_id: &str) {
        if self.is_hidden_space(space_id) {
            return;
        }
        let _ = self.talk.ensure_assistants_channel(space_id);
    }

    /// [`Self::ensure_assistants_channel`] for each space the person sees.
    pub(crate) fn ensure_assistants_channels(&self) {
        for b in self.visible_boards() {
            self.ensure_assistants_channel(&b.id);
        }
    }
}

impl Services {
    /// Names for the keys other people's rows carry in this device's
    /// spaces: `person:<id>` by their profile (else member) name, and
    /// `agent:<cardId>` by its card, for Talk's member and post names.
    pub(crate) fn space_names(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for b in self.boards.list() {
            let ms = b.verifier.membership();
            for p in &ms.members {
                let name = b
                    .verifier
                    .profile(&p.person_id)
                    .unwrap_or_else(|| p.name.clone());
                if !name.trim().is_empty() {
                    out.insert(
                        format!("{}{}", kinds::PERSON_ACTOR_PREFIX, p.person_id),
                        name,
                    );
                }
            }
            for (id, c) in &super::boards::lock(&b.maps).cards {
                if !c.name.trim().is_empty() {
                    out.insert(format!("{}{id}", kinds::AGENT_ACTOR_PREFIX), c.name.clone());
                }
            }
        }
        out
    }
}

/// Talk's way into the spaces' logs ([`coord::talk::SpaceLogs`]).
pub(super) struct SpaceTalk(pub(super) std::sync::Weak<Services>);

impl coord::talk::SpaceLogs for SpaceTalk {
    fn append(
        &self,
        space: &str,
        by: &str,
        kind: &str,
        payload: serde_json::Value,
    ) -> std::result::Result<store::Event, String> {
        let s = self.0.upgrade().ok_or("the Core is closing")?;
        s.append_space_talk(space, by, kind, payload)
    }

    fn rows(&self) -> Vec<store::Event> {
        let Some(s) = self.0.upgrade() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        // Not a space the person left (card #461).
        for b in s.shown_boards() {
            let f = Filter {
                kinds: SPACE_TALK_KINDS.iter().map(|k| k.to_string()).collect(),
                ..Default::default()
            };
            let _ = super::boards::each_row(&b.store, f, |ev| {
                if let Some(ev) = s.space_talk_row(&b.id, ev) {
                    out.push(ev);
                }
            });
        }
        out
    }

    fn person_id(&self) -> Option<String> {
        Some(self.0.upgrade()?.person_id())
    }

    fn dm_for(&self, other: &str) -> std::result::Result<String, String> {
        let s = self.0.upgrade().ok_or("the Core is closing")?;
        s.dm_for(other)
    }

    fn dm_route(&self, conv: &str) -> Option<String> {
        self.0.upgrade()?.dm_route(conv)
    }
}

// ---- the Core's spaces API ----------------------------------------------------

impl super::Core {
    /// The boards in space `space_id` (a v0.5 board's id), its own first.
    pub fn space_boards(&self, space_id: &str) -> Result<Vec<SpaceBoard>> {
        self.s.space_boards(space_id)
    }

    /// The member devices of `space_id` that don't read space rows yet.
    pub fn space_not_ready(&self, space_id: &str) -> Result<Vec<String>> {
        self.s.space_not_ready(space_id)
    }

    /// The calls gate (card #652): whether every other device in space
    /// `space_id` is known to store an unknown `call.*` or `talk.*` row
    /// (hello `tolerant` >= [`CALLS_TOLERANT`]), and which aren't.
    /// [`SpaceTolerance::ready`] says the space may take `call.*` rows.
    pub fn space_tolerance(&self, space_id: &str) -> Result<SpaceTolerance> {
        self.s.space_tolerance(space_id)
    }

    /// Whether space `space_id` may take a space-only row now: the gate's
    /// refusal if not ([`gate_refusal`]).
    pub fn space_gate(&self, space_id: &str) -> std::result::Result<(), String> {
        let waiting = self
            .s
            .space_not_ready(space_id)
            .map_err(|e| e.to_string())?;
        gate_refusal(&waiting).map_or(Ok(()), Err)
    }

    /// Adds a board to space `space_id`; refused until every member device
    /// reads space rows.
    pub fn create_space_board(&self, space_id: &str, name: &str, tag: &str) -> Result<String> {
        self.s.create_space_board(space_id, name, tag)
    }

    /// Renames a board of space `space_id`, its first too, leaving the
    /// space's own name alone; refused until every member device reads
    /// space rows.
    pub fn delete_space_board(&self, space_id: &str, board_id: &str) -> Result<()> {
        self.s.delete_space_board(space_id, board_id)
    }

    pub fn delete_personal_board(&self, board_id: &str) -> Result<()> {
        self.s.delete_personal_board(board_id)
    }

    pub fn rename_space_board(&self, space_id: &str, board_id: &str, name: &str) -> Result<()> {
        self.s.rename_space_board(space_id, board_id, name)
    }

    /// [`Core::create_space_board`] with the new board's description
    /// (card #690), on its one create row; empty for none.
    pub fn create_space_board_with(
        &self,
        space_id: &str,
        name: &str,
        tag: &str,
        description: &str,
    ) -> Result<String> {
        self.s
            .create_space_board_with(space_id, name, tag, description)
    }

    /// Sets a board's description in space `space_id`, its first board's
    /// too (card #690); empty clears it. Refused on a closed board, and
    /// until every member device stores the row ([`BOARD_ROWS_TOLERANT`]).
    pub fn set_space_board_description(
        &self,
        space_id: &str,
        board_id: &str,
        description: &str,
    ) -> Result<()> {
        self.s
            .set_space_board_description(space_id, board_id, description)
    }

    /// Closes a board in space `space_id` (card #690): its maker or a
    /// space owner, never the first board. Its open cards are first kept
    /// (the close is refused while any is open), moved or canceled, as
    /// `open` says, as this person.
    pub fn close_space_board(&self, space_id: &str, board_id: &str, open: OpenCards) -> Result<()> {
        self.s.close_space_board(space_id, board_id, open)
    }

    /// Reopens a closed board in space `space_id` (card #690).
    pub fn reopen_space_board(&self, space_id: &str, board_id: &str) -> Result<()> {
        self.s.reopen_space_board(space_id, board_id)
    }

    /// The member devices of `space_id` too old to store a board's
    /// description or close row (card #690).
    pub fn board_rows_not_ready(&self, space_id: &str) -> Result<Vec<String>> {
        self.s.board_rows_not_ready(space_id)
    }

    /// The open cards on a board of space `space_id` (`""` for Personal):
    /// what a close keeps, moves or cancels (card #690).
    pub fn open_on_board(&self, space_id: &str, board_id: &str) -> Result<Vec<coord::Task>> {
        if space_id.is_empty() {
            return Ok(self.s.coord.open_on_board(board_id));
        }
        Ok(self.s.board(space_id)?.coord().open_on_board(board_id))
    }

    /// [`Core::create_personal_board`] with a description (card #690).
    pub fn create_personal_board_with(
        &self,
        name: &str,
        tag: &str,
        description: &str,
    ) -> Result<String> {
        self.s.create_personal_board_with(name, tag, description)
    }

    /// Sets the description of one of Personal's boards (card #690).
    pub fn set_personal_board_description(&self, board_id: &str, description: &str) -> Result<()> {
        self.s.set_personal_board_description(board_id, description)
    }

    /// Closes one of Personal's boards, never its default (card #690),
    /// its open cards first as `open` says.
    pub fn close_personal_board(&self, board_id: &str, open: OpenCards) -> Result<()> {
        self.s.close_personal_board(board_id, open)
    }

    /// Reopens one of Personal's closed boards (card #690).
    pub fn reopen_personal_board(&self, board_id: &str) -> Result<()> {
        self.s.reopen_personal_board(board_id)
    }

    /// Personal's boards, its default first (card #558).
    pub fn personal_boards(&self) -> Result<Vec<SpaceBoard>> {
        self.s.personal_boards()
    }

    /// Adds a board to Personal and returns its id.
    pub fn create_personal_board(&self, name: &str, tag: &str) -> Result<String> {
        self.s.create_personal_board(name, tag)
    }

    /// Moves a space nobody else is in into Personal as a board, every
    /// card with it, then forgets the space on every device (card #640).
    pub fn move_space_to_personal(
        &self,
        space_id: &str,
        o: &super::space_move::MoveOpts,
    ) -> Result<super::space_move::SpaceMove> {
        self.s.move_space_to_personal(space_id, o)
    }

    /// Renames one of Personal's boards, its default ([`PERSONAL_BOARD`])
    /// too.
    pub fn rename_personal_board(&self, board_id: &str, name: &str) -> Result<()> {
        self.s.rename_personal_board(board_id, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folded(own: SpaceBoard, rows: &[BoardRowAt]) -> Vec<SpaceBoard> {
        fold_space_boards(own, rows, &|_| true)
    }

    fn row(kind: &str, id: &str, name: &str, tag: &str) -> BoardRowAt {
        let r = SpaceBoardRow {
            board_id: id.into(),
            name: name.into(),
            tag: tag.into(),
            ..Default::default()
        };
        BoardRowAt {
            kind: kind.to_string(),
            payload: serde_json::to_vec(&r).unwrap(),
            at: coord::RowPos::default(),
        }
    }

    #[test]
    fn a_space_lists_its_own_board_first_then_the_rest_in_order() {
        let own = SpaceBoard {
            board_id: "s".into(),
            name: "Wind Waker Multiplayer".into(),
            tag: "wind".into(),
            ..Default::default()
        };
        let got = folded(
            own.clone(),
            &[
                row(kinds::SPACE_BOARD_CREATED, "b2", "Netcode", "net"),
                row(kinds::SPACE_BOARD_CREATED, "b1", "Art", "art"),
                row(kinds::SPACE_BOARD_RENAMED, "b2", "Netcode v2", ""),
                // A second create of one id, a rename of no board, and a
                // create of the space's own id are all ignored.
                row(kinds::SPACE_BOARD_CREATED, "b2", "dup", "dup"),
                row(kinds::SPACE_BOARD_RENAMED, "nope", "x", "x"),
                row(kinds::SPACE_BOARD_CREATED, "s", "self", "self"),
                BoardRowAt {
                    kind: "space.board.created".into(),
                    payload: b"not json".to_vec(),
                    at: coord::RowPos::default(),
                },
            ],
        );
        let names: Vec<(&str, &str)> = got
            .iter()
            .map(|b| (b.name.as_str(), b.tag.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("Wind Waker Multiplayer", "wind"),
                ("Netcode v2", "net"),
                ("Art", "art")
            ]
        );
    }

    /// A row with only some fields still folds (card #558): a failed decode
    /// would lose it without a word.
    #[test]
    fn a_row_missing_fields_still_folds() {
        let own = SpaceBoard {
            board_id: "s".into(),
            name: "Wind".into(),
            tag: "wind".into(),
            ..Default::default()
        };
        let got = folded(
            own,
            &[
                row(kinds::SPACE_BOARD_CREATED, "b1", "Netcode", "net"),
                BoardRowAt {
                    kind: kinds::SPACE_BOARD_RENAMED.into(),
                    payload: br#"{"boardId":"b1","name":"Sockets"}"#.to_vec(),
                    at: coord::RowPos::default(),
                },
            ],
        );
        assert_eq!(got[1].name, "Sockets");
        assert_eq!(got[1].tag, "net");
    }

    /// A peer's new or renamed board reaches the boards list live.
    #[test]
    fn a_space_board_row_moves_the_boards_list() {
        let ev = |kind: &str| store::Event {
            seq: 1,
            kind: kind.into(),
            ..Default::default()
        };
        let moves = super::super::boards::summary_row;
        assert!(moves(&ev(kinds::SPACE_BOARD_CREATED)));
        assert!(moves(&ev(kinds::SPACE_BOARD_RENAMED)));
        assert!(moves(&ev(kinds::SPACE_BOARD_DESCRIBED)));
        assert!(moves(&ev(kinds::SPACE_BOARD_CLOSED)));
        assert!(!moves(&ev(kinds::TASK_CREATED)));
    }

    /// The first board has a name of its own once a rename gives it one
    /// (card #558), and only its name: its tag is the space's. Until then
    /// it follows the space's name.
    fn deleted_row(id: &str, seq: i64) -> BoardRowAt {
        let r = SpaceBoardRow {
            board_id: id.into(),
            deleted: true,
            ..Default::default()
        };
        BoardRowAt {
            kind: kinds::SPACE_BOARD_RENAMED.into(),
            payload: serde_json::to_vec(&r).unwrap(),
            at: coord::RowPos {
                hlc: seq as u64,
                dev: "d".into(),
                seq,
            },
        }
    }

    fn at(mut r: BoardRowAt, seq: i64) -> BoardRowAt {
        r.at = coord::RowPos {
            hlc: seq as u64,
            dev: "d".into(),
            seq,
        };
        r
    }

    fn the_own() -> SpaceBoard {
        SpaceBoard {
            board_id: "s".into(),
            name: "Space".into(),
            tag: "wind".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_delete_row_keeps_the_fields_an_older_decoder_needs() {
        let v: serde_json::Value = serde_json::from_slice(&deleted_row("b1", 1).payload).unwrap();
        assert_eq!(v["name"], "");
        assert_eq!(v["tag"], "");
        assert_eq!(v["deleted"], true);
        let plain = serde_json::to_value(SpaceBoardRow::default()).unwrap();
        assert!(plain.get("deleted").is_none());
    }

    #[test]
    fn a_delete_wins_whatever_order_the_rows_sit_in() {
        let create = || at(row(kinds::SPACE_BOARD_CREATED, "b1", "Net", "net"), 1);
        let rename = || at(row(kinds::SPACE_BOARD_RENAMED, "b1", "Later", "later"), 9);
        let again = || at(row(kinds::SPACE_BOARD_CREATED, "b1", "Again", "again"), 8);
        for rows in [
            vec![create(), deleted_row("b1", 5), rename(), again()],
            vec![rename(), again(), deleted_row("b1", 5), create()],
            vec![deleted_row("b1", 5), create()],
        ] {
            let got = folded(the_own(), &rows);
            assert_eq!(got.len(), 1, "only the first board is left: {got:?}");
        }
    }

    #[test]
    fn the_first_board_cant_be_deleted() {
        let got = folded(the_own(), &[deleted_row("s", 1)]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].board_id, "s");
    }

    #[test]
    fn a_delete_from_someone_not_a_member_there_counts_for_nothing() {
        let rows = [
            at(row(kinds::SPACE_BOARD_CREATED, "b1", "Net", "net"), 1),
            deleted_row("b1", 5),
        ];
        let got = fold_space_boards(the_own(), &rows, &|_| false);
        assert_eq!(got.len(), 2, "the board stays: {got:?}");
    }

    // ---- card #690: descriptions and closing ----

    /// A row of `kind` with payload `p`, written by device `dev` at `seq`.
    fn by_dev(kind: &str, p: impl Serialize, dev: &str, seq: i64) -> BoardRowAt {
        BoardRowAt {
            kind: kind.into(),
            payload: serde_json::to_vec(&p).unwrap(),
            at: coord::RowPos {
                hlc: seq as u64,
                dev: dev.into(),
                seq,
            },
        }
    }

    fn created(id: &str, tag: &str, description: &str, dev: &str, seq: i64) -> BoardRowAt {
        by_dev(
            kinds::SPACE_BOARD_CREATED,
            SpaceBoardRow {
                board_id: id.into(),
                name: tag.into(),
                tag: tag.into(),
                description: description.into(),
                ..Default::default()
            },
            dev,
            seq,
        )
    }

    fn described(id: &str, d: &str, dev: &str, seq: i64) -> BoardRowAt {
        by_dev(
            kinds::SPACE_BOARD_DESCRIBED,
            BoardDescribedRow {
                board_id: id.into(),
                description: d.into(),
                ..Default::default()
            },
            dev,
            seq,
        )
    }

    fn close(id: &str, closed: bool, dev: &str, seq: i64) -> BoardRowAt {
        by_dev(
            kinds::SPACE_BOARD_CLOSED,
            BoardClosedRow {
                board_id: id.into(),
                closed,
                at: format!("t{seq}"),
                ..Default::default()
            },
            dev,
            seq,
        )
    }

    /// A space where device `alice` is Alice (an owner), `bob` is Bob (a
    /// member), and `eve` is no one.
    fn space_rules<R>(f: impl FnOnce(&Rules) -> R) -> R {
        let person = |d: &str| match d {
            "alice" | "bob" => Some(format!("person:{d}")),
            _ => None,
        };
        f(&Rules {
            member: &|at: &coord::RowPos| at.dev != "eve",
            owner: &|at: &coord::RowPos| at.dev == "alice",
            person: &person,
        })
    }

    fn boards_of(rows: &[BoardRowAt], me: Option<(&str, bool)>) -> Vec<SpaceBoard> {
        space_rules(|r| {
            let mut out = fold_space_boards(the_own(), rows, r.member);
            fold_board_state(&mut out, rows, r, me);
            out
        })
    }

    #[test]
    fn a_description_comes_from_the_create_then_the_newest_row_and_survives_a_rename() {
        let rows = [
            created("b1", "net", "Ship netcode", "bob", 1),
            described("b1", "Rollback netcode: two PCs play a match", "alice", 3),
            described("b1", "Lost to an older row", "bob", 2),
            at(row(kinds::SPACE_BOARD_RENAMED, "b1", "Netcode", ""), 5),
            described("s", "The space's first board", "bob", 4),
            // Someone who isn't a member there counts for nothing.
            described("b1", "Vandalised", "eve", 9),
        ];
        let got = boards_of(&rows, None);
        assert_eq!(got[1].name, "Netcode");
        assert_eq!(got[1].description, "Rollback netcode: two PCs play a match");
        assert_eq!(got[1].description_by, "person:alice");
        assert_eq!(got[0].description, "The space's first board");
        // A board with none keeps none, and nobody wrote it.
        let got = boards_of(&[created("b2", "art", "", "bob", 1)], None);
        assert_eq!(
            (got[1].description.as_str(), got[1].description_by.as_str()),
            ("", "")
        );
        // An empty row clears it.
        let got = boards_of(
            &[
                created("b2", "art", "Posters", "bob", 1),
                described("b2", "", "bob", 2),
            ],
            None,
        );
        assert_eq!(
            (got[1].description.as_str(), got[1].description_by.as_str()),
            ("", "")
        );
    }

    #[test]
    fn a_close_and_a_reopen_take_the_newest_whatever_order_the_rows_sit_in() {
        let base = [created("b1", "net", "", "bob", 1)];
        let mut rows = base.to_vec();
        rows.extend([close("b1", true, "bob", 5), close("b1", false, "alice", 7)]);
        for order in [rows.clone(), rows.iter().rev().cloned().collect()] {
            let got = boards_of(&order, None);
            assert!(!got[1].closed, "the reopen is newer: {got:?}");
        }
        rows.push(close("b1", true, "bob", 9));
        let got = boards_of(&rows, None);
        assert!(got[1].closed);
        assert_eq!(got[1].closed_at, "t9");
        let list = space_rules(|r| closes("s", &rows, r));
        assert_eq!(list.keys().collect::<Vec<_>>(), ["b1"]);
    }

    #[test]
    fn only_a_boards_maker_or_a_space_owner_closes_it_and_never_the_first() {
        // Bob made b1, Alice (an owner) made b2.
        let rows = [
            created("b1", "net", "", "bob", 1),
            created("b2", "art", "", "alice", 2),
            // Bob closes Alice's board: not his, and he isn't an owner.
            close("b2", true, "bob", 3),
            // Alice closes Bob's: an owner may.
            close("b1", true, "alice", 4),
            // A first board never closes, whoever writes it.
            close("s", true, "alice", 5),
            close("b2", true, "eve", 6),
        ];
        let got = boards_of(&rows, Some(("person:bob", false)));
        assert!(!got[0].closed && got[1].closed && !got[2].closed, "{got:?}");
        // Who may now: never the first; Bob his own; an owner any.
        let may: Vec<bool> = got.iter().map(|b| b.may_close).collect();
        assert_eq!(may, [false, true, false]);
        let got = boards_of(&rows, Some(("person:alice", true)));
        let may: Vec<bool> = got.iter().map(|b| b.may_close).collect();
        assert_eq!(may, [false, true, true]);
    }

    /// An Irori before #690 folds a create row with a description as it
    /// always did: the extra field is ignored, and a described or closed
    /// row is no board row it reads.
    #[test]
    fn an_older_fold_reads_a_create_with_a_description_as_before() {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Old {
            board_id: String,
            name: String,
            tag: String,
        }
        let r = created("b1", "net", "Ship it", "bob", 1);
        let old: Old = serde_json::from_slice(&r.payload).unwrap();
        assert_eq!(
            (old.board_id.as_str(), old.name.as_str(), old.tag.as_str()),
            ("b1", "net", "net")
        );
        let v: serde_json::Value = serde_json::from_slice(&r.payload).unwrap();
        assert_eq!(v["description"], "Ship it");
        // No description: no field at all.
        let v: serde_json::Value =
            serde_json::from_slice(&created("b2", "art", "", "bob", 1).payload).unwrap();
        assert!(v.get("description").is_none(), "{v}");
        // The tags and deletes an older device keeps never see these rows.
        let rows = [
            r,
            described("b1", "x", "bob", 2),
            close("b1", true, "bob", 3),
        ];
        assert_eq!(tag_ids(&rows), [("net".to_string(), "b1".to_string())]);
        assert!(deletes("s", &rows, &|_| true).is_empty());
    }

    #[test]
    fn personal_boards_close_by_the_person_except_the_default() {
        let rows = [
            created("b1", "home", "Fix the house", "d1", 1),
            close("b1", true, "d2", 2),
            close(PERSONAL_BOARD, true, "d1", 3),
        ];
        let own = SpaceBoard {
            board_id: PERSONAL_BOARD.into(),
            name: "Personal".into(),
            ..Default::default()
        };
        let got = with_personal_rules(|r| {
            let mut out = fold_space_boards(own, &rows, r.member);
            fold_board_state(&mut out, &rows, r, None);
            out
        });
        assert!(!got[0].closed && !got[0].may_close);
        assert!(got[1].closed && got[1].may_close);
        assert_eq!(got[1].description_by, "user");
    }

    #[test]
    fn a_first_board_takes_a_name_from_a_rename_and_nothing_else() {
        let own = |name: &str| SpaceBoard {
            board_id: "s".into(),
            name: name.into(),
            tag: "wind".into(),
            ..Default::default()
        };
        assert_eq!(folded(own("New name"), &[])[0].name, "New name");
        let rows = [
            row(kinds::SPACE_BOARD_RENAMED, "s", "Tasks", "other"),
            row(kinds::SPACE_BOARD_RENAMED, "s", "", ""),
        ];
        let got = folded(own("New name"), &rows);
        assert_eq!(got.len(), 1);
        assert_eq!(
            (got[0].name.as_str(), got[0].tag.as_str()),
            ("Tasks", "wind")
        );
    }

    /// The version gate (card #392): no device waiting, no refusal; any
    /// waiting, a refusal naming each one.
    #[test]
    fn the_gate_names_every_device_it_waits_on() {
        assert_eq!(gate_refusal(&[]), None);
        let why = gate_refusal(&["Bob/abc123".into(), "Bob/def456".into()]).unwrap();
        assert!(why.contains("run an Irori with spaces"), "{why}");
        assert!(why.ends_with("waiting on Bob/abc123, Bob/def456"), "{why}");
    }

    /// A space's talk row's keys: `by` and a member's key must map, a
    /// mention that doesn't is dropped, and other fields stay.
    #[test]
    fn talk_keys_map_into_a_space_and_back() {
        let to_space = |k: &str| match k {
            "user:dev-a" => Some("person:p1".to_string()),
            "dev-a/s1" => Some("agent:c1".to_string()),
            k if k.starts_with("person:") || k.starts_with("agent:") => Some(k.to_string()),
            _ => None,
        };
        let mut post = serde_json::json!({
            "postId": "post_1", "convId": "ch_1", "text": "hi",
            "mentions": ["dev-a/s1", "nobody", "agent:c9"], "by": "user:dev-a"
        });
        map_talk_keys(&mut post, to_space).unwrap();
        assert_eq!(post["by"], "person:p1");
        assert_eq!(
            post["mentions"],
            serde_json::json!(["agent:c1", "agent:c9"])
        );
        assert_eq!(post["text"], "hi");

        let mut member = serde_json::json!({
            "convId": "ch_1", "member": {"kind": "agent", "key": "dev-a/s1"},
            "op": "join", "by": "user:dev-a"
        });
        map_talk_keys(&mut member, to_space).unwrap();
        assert_eq!(member["member"]["key"], "agent:c1");

        let mut bad = serde_json::json!({"by": "someone"});
        assert!(map_talk_keys(&mut bad, to_space).is_err());
        let mut bad = serde_json::json!({"member": {"key": "nobody"}, "by": "user:dev-a"});
        assert!(map_talk_keys(&mut bad, to_space).is_err());
        assert!(map_talk_keys(&mut serde_json::json!([]), to_space).is_err());
    }
}
