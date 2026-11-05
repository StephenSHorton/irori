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
#[serde(rename_all = "camelCase")]
pub struct SpaceBoardRow {
    pub board_id: String,
    pub name: String,
    pub tag: String,
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    pub at: String,
}

/// The boards in a space's log, its own first: `own` is the space's own
/// board (from board.created/renamed), then each `space.board.created` in
/// log order with its newest rename. A rename of a board the log doesn't
/// hold is ignored, as is a second create of one id.
pub fn fold_space_boards(own: SpaceBoard, rows: &[(String, Vec<u8>)]) -> Vec<SpaceBoard> {
    let mut order: Vec<String> = Vec::new();
    let mut boards: BTreeMap<String, SpaceBoard> = BTreeMap::new();
    for (kind, payload) in rows {
        let Ok(r) = serde_json::from_slice::<SpaceBoardRow>(payload) else {
            continue;
        };
        if r.board_id.is_empty() || r.board_id == own.board_id {
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

impl Services {
    /// The boards in space `space_id`, its own first.
    pub(crate) fn space_boards(&self, space_id: &str) -> Result<Vec<SpaceBoard>> {
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
        let hidden = self.is_dm_space(space_id);
        let devices: Vec<(String, String)> = match &mesh {
            // A hidden log counts only devices heard to be too old (card
            // #538, talk-dms.md §3): one made a moment ago has heard
            // nobody, and the other person can't connect until the
            // hand-off that needs its first row.
            Some(m) if hidden => m
                .said_too_old()
                .into_iter()
                .map(|d| (d.person_id, d.device_id))
                .collect(),
            Some(m) => m
                .not_ready_for_spaces()
                .into_iter()
                .map(|d| (d.person_id, d.device_id))
                .collect(),
            None if hidden => Vec::new(),
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
        let (name, tag) = (name.trim(), tag.trim());
        if !irori_identity::membership::valid_board_name(name) {
            return Err(Error::coord(
                CK::Invalid,
                format_args!(
                    "a board name is 1 to {} characters",
                    irori_identity::membership::MAX_BOARD_NAME
                ),
            ));
        }
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
            self.hub.publish(crate::events::CoreEvent::Talk);
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
            kinds::TALK_POST => {}
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

    /// Adds a board to space `space_id`; refused until every member device
    /// reads space rows.
    pub fn create_space_board(&self, space_id: &str, name: &str, tag: &str) -> Result<String> {
        self.s.create_space_board(space_id, name, tag)
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
                // A second create of one id, a rename of no board, and the
                // space's own id are all ignored.
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
