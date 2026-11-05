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
//! space-only row until every member device has said, in its board hello,
//! that it reads them (`irori_mesh::boardmesh::SPACES_PROTOCOL`).

use std::collections::BTreeMap;

use coord::ErrorKind as CK;
use serde::{Deserialize, Serialize};
use store::{Filter, kinds};

use super::Services;
use super::boards::{Error, Result};

/// One board of a space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceBoard {
    /// The board's id: the space's own id for its first board.
    pub board_id: String,
    pub name: String,
    pub tag: String,
}

/// `space.board.created` / `space.board.renamed`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SpaceBoardRow {
    pub board_id: String,
    pub name: String,
    pub tag: String,
    pub by: String,
    pub at: String,
}

/// The boards in a space's log, its own first: `own` is the space's own
/// board (from board.created/renamed), then each `space.board.created` in
/// log order with its newest rename. A rename of a board the log doesn't
/// hold is ignored, as is a second create of one id.
///
/// `own` comes in with the space's name, and keeps it until a rename names
/// the first board itself (card #558): only its name, since its tag is the
/// space's and refs are stamped with it.
pub fn fold_space_boards(mut own: SpaceBoard, rows: &[(String, Vec<u8>)]) -> Vec<SpaceBoard> {
    let mut order: Vec<String> = Vec::new();
    let mut boards: BTreeMap<String, SpaceBoard> = BTreeMap::new();
    for (kind, payload) in rows {
        let Ok(r) = serde_json::from_slice::<SpaceBoardRow>(payload) else {
            continue;
        };
        if r.board_id.is_empty() {
            continue;
        }
        if r.board_id == own.board_id {
            if kind == kinds::SPACE_BOARD_RENAMED && !r.name.is_empty() {
                own.name = r.name;
            }
            continue;
        }
        match kind.as_str() {
            kinds::SPACE_BOARD_CREATED if !boards.contains_key(&r.board_id) => {
                order.push(r.board_id.clone());
                boards.insert(
                    r.board_id.clone(),
                    SpaceBoard {
                        board_id: r.board_id,
                        name: r.name,
                        tag: r.tag,
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

/// The space.board.* rows of a space's log, oldest first.
fn space_board_rows(st: &store::Store) -> store::Result<Vec<(String, Vec<u8>)>> {
    Ok(st
        .list(&Filter {
            kinds: vec![
                kinds::SPACE_BOARD_CREATED.into(),
                kinds::SPACE_BOARD_RENAMED.into(),
            ],
            limit: 10_000,
            ..Default::default()
        })?
        .into_iter()
        .map(|ev| (ev.kind, ev.payload))
        .collect())
}

/// The tag of board `board` inside the space whose log is `st`; None when
/// the log holds no such board.
pub(crate) fn board_tag_in(st: &store::Store, board: &str) -> Option<String> {
    let own = SpaceBoard {
        board_id: String::new(),
        name: String::new(),
        tag: String::new(),
    };
    fold_space_boards(own, &space_board_rows(st).ok()?)
        .into_iter()
        .skip(1)
        .find(|b| b.board_id == board)
        .map(|b| b.tag)
}

/// Every tag the log whose store is `st` has named for a board inside it.
pub(crate) fn board_tags_in(st: &store::Store) -> Vec<String> {
    let rows = space_board_rows(st).unwrap_or_default();
    rows.iter()
        .filter_map(|(_, p)| serde_json::from_slice::<SpaceBoardRow>(p).ok())
        .map(|r| r.tag)
        .filter(|t| !t.is_empty())
        .collect()
}

impl Services {
    /// Whether `id` is a hidden space: one with no boards, that never gets
    /// any and takes no card (spaces-boards.md §1). None yet: #538's DM
    /// spaces wire in here. Never key on a tag or a name.
    pub(crate) fn is_hidden_space(&self, _id: &str) -> bool {
        false
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
        };
        let rows = space_board_rows(&b.store).map_err(Error::from)?;
        Ok(fold_space_boards(own, &rows))
    }

    /// The member devices of space `space_id` not yet heard to read space
    /// rows, as `person/device` (short ids). Empty: it may write them.
    pub(crate) fn space_not_ready(&self, space_id: &str) -> Result<Vec<String>> {
        let b = self.board(space_id)?;
        let ms = b.verifier.membership();
        let me = self.device.id.clone();
        let mesh = self.mesh.board(space_id);
        let devices: Vec<(String, String)> = match &mesh {
            Some(m) => m
                .not_ready_for_spaces()
                .into_iter()
                .map(|d| (d.person_id, d.device_id))
                .collect(),
            // No mesh for it (stopped, or a test without a network): every
            // other member device counts as unheard.
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
        if self.is_hidden_space(space_id) {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!("space {space_id} holds no boards"),
            ));
        }
        let (name, tag) = (name.trim(), tag.trim());
        check_board_name(name)?;
        if !irori_identity::membership::valid_board_tag(tag) {
            return Err(Error::coord(
                CK::Invalid,
                "a board tag is a lowercase letter, then up to 15 lowercase letters, digits or '-'",
            ));
        }
        let b = self.board(space_id)?;
        let me = self.person_id();
        if !b.verifier.membership().active(&me) {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!("you aren't a member of space {space_id}"),
            ));
        }
        if let Some(other) = self.board_tag_holder(tag) {
            return Err(Error::coord(
                CK::Invalid,
                format_args!(
                    "the tag {} is already board {other}'s",
                    gocompat::quote::quote(tag)
                ),
            ));
        }
        if let Some(why) = gate_refusal(&self.space_not_ready(space_id)?) {
            return Err(Error::coord(CK::NotAllowed, why));
        }
        let id = super::boards::new_board_id()?;
        let row = SpaceBoardRow {
            board_id: id.clone(),
            name: name.to_string(),
            tag: tag.to_string(),
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
            by: by.clone(),
            at: (self.now)().format_rfc3339_nano(),
        };
        b.store.append(&by, kinds::SPACE_BOARD_RENAMED, &row)?;
        self.boards.moved();
        Ok(())
    }

    /// The board (a space's own, or one inside a space) already using
    /// `tag` on this device, if any.
    pub(crate) fn board_tag_holder(&self, tag: &str) -> Option<String> {
        let own = self.tag_holder(tag);
        if !own.is_empty() {
            return Some(own);
        }
        for b in self.shown_boards() {
            if let Ok(bs) = self.space_boards(&b.id)
                && let Some(x) = bs.iter().skip(1).find(|x| x.tag == tag)
            {
                return Some(x.board_id.clone());
            }
        }
        None
    }
}

fn short(id: &str) -> &str {
    &id[..id.len().min(6)]
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

/// The version gate's answer (card #392, spaces.md §4): why a space may
/// not take a space-only row yet, given the member devices not yet heard
/// to read them (`space_not_ready`); None when it may.
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

/// The talk.* kinds a space's log carries (read cursors never do).
const SPACE_TALK_KINDS: [&str; 5] = [
    kinds::TALK_CHANNEL_CREATED,
    kinds::TALK_CHANNEL_EDITED,
    kinds::TALK_MEMBER,
    kinds::TALK_POST,
    kinds::TALK_CARD,
];

/// Rewrites the keys a talk.* payload names (`by`, `member.key`, each of
/// `mentions`) with `f`; a key `f` turns into None is dropped from
/// `mentions` and refuses the row anywhere else.
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
    if let Some(serde_json::Value::Array(ms)) = o.get_mut("mentions") {
        let keep: Vec<serde_json::Value> = ms
            .iter()
            .filter_map(|m| m.as_str().and_then(&mut f))
            .map(serde_json::Value::String)
            .collect();
        *ms = keep;
    }
    Ok(())
}

impl Services {
    /// A space log's talk.* row as this device's Talk folds it: keys back
    /// in this device's form, and a channel in the space whose log holds
    /// it whatever its row says. None: not a row Talk folds.
    pub(crate) fn space_talk_row(&self, space_id: &str, ev: &store::Event) -> Option<store::Event> {
        if !SPACE_TALK_KINDS.contains(&ev.kind.as_str()) {
            return None;
        }
        let mut v: serde_json::Value = serde_json::from_slice(&ev.payload).ok()?;
        map_talk_keys(&mut v, |k| Some(self.private_key(k))).ok()?;
        // Every row says which space carried it: a channel is made there,
        // and the rest may only touch that space's channels (card #461).
        if let Some(o) = v.as_object_mut() {
            o.insert("spaceId".into(), space_id.into());
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
            self.hub.publish(crate::events::CoreEvent::Talk);
        }
    }

    /// Appends a talk.* row to space `space_id`'s log as `by` (a key on this
    /// device): refused unless the person is an active member and every
    /// device in the space reads space rows (the version gate).
    fn append_space_talk(
        &self,
        space_id: &str,
        by: &str,
        kind: &str,
        mut payload: serde_json::Value,
    ) -> std::result::Result<store::Event, String> {
        if !SPACE_TALK_KINDS.contains(&kind) {
            return Err(format!("{kind} never goes in a space's log"));
        }
        let b = self.board(space_id).map_err(|e| e.to_string())?;
        if !b.verifier.membership().active(&self.person_id()) {
            return Err(format!("you aren't a member of space {space_id}"));
        }
        if let Some(why) = gate_refusal(&self.space_not_ready(space_id).map_err(|e| e.to_string())?)
        {
            return Err(why);
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
        let ev = b
            .store
            .append(&actor, kind, &payload)
            .map_err(|e| e.to_string())?;
        self.space_talk_row(space_id, &ev)
            .ok_or_else(|| "the row didn't read back".to_string())
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

    /// Adds a board to space `space_id`; refused until every member device
    /// reads space rows.
    pub fn create_space_board(&self, space_id: &str, name: &str, tag: &str) -> Result<String> {
        self.s.create_space_board(space_id, name, tag)
    }

    /// Renames a board of space `space_id`, its first too, leaving the
    /// space's own name alone; refused until every member device reads
    /// space rows.
    pub fn rename_space_board(&self, space_id: &str, board_id: &str, name: &str) -> Result<()> {
        self.s.rename_space_board(space_id, board_id, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: &str, id: &str, name: &str, tag: &str) -> (String, Vec<u8>) {
        let r = SpaceBoardRow {
            board_id: id.into(),
            name: name.into(),
            tag: tag.into(),
            ..Default::default()
        };
        (kind.to_string(), serde_json::to_vec(&r).unwrap())
    }

    #[test]
    fn a_space_lists_its_own_board_first_then_the_rest_in_order() {
        let own = SpaceBoard {
            board_id: "s".into(),
            name: "Wind Waker Multiplayer".into(),
            tag: "wind".into(),
        };
        let got = fold_space_boards(
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
                ("space.board.created".into(), b"not json".to_vec()),
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
        };
        let got = fold_space_boards(
            own,
            &[
                row(kinds::SPACE_BOARD_CREATED, "b1", "Netcode", "net"),
                (
                    kinds::SPACE_BOARD_RENAMED.into(),
                    br#"{"boardId":"b1","name":"Sockets"}"#.to_vec(),
                ),
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
        assert!(!moves(&ev(kinds::TASK_CREATED)));
    }

    /// The first board has a name of its own once a rename gives it one
    /// (card #558), and only its name: its tag is the space's. Until then
    /// it follows the space's name.
    #[test]
    fn a_first_board_takes_a_name_from_a_rename_and_nothing_else() {
        let own = |name: &str| SpaceBoard {
            board_id: "s".into(),
            name: name.into(),
            tag: "wind".into(),
        };
        assert_eq!(fold_space_boards(own("New name"), &[])[0].name, "New name");
        let rows = [
            row(kinds::SPACE_BOARD_RENAMED, "s", "Tasks", "other"),
            row(kinds::SPACE_BOARD_RENAMED, "s", "", ""),
        ];
        let got = fold_space_boards(own("New name"), &rows);
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
