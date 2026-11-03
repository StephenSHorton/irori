//! Spaces (card #392, decision 45, docs/design/talk-channels.md §5).
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
        let rows = b
            .store
            .list(&Filter {
                kinds: vec![
                    kinds::SPACE_BOARD_CREATED.into(),
                    kinds::SPACE_BOARD_RENAMED.into(),
                ],
                limit: 10_000,
                ..Default::default()
            })
            .map_err(Error::from)?
            .into_iter()
            .map(|ev| (ev.kind, ev.payload))
            .collect::<Vec<_>>();
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
        let waiting = self.space_not_ready(space_id)?;
        if !waiting.is_empty() {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!(
                    "every device in the space must run an Irori with spaces first; waiting on {}",
                    waiting.join(", ")
                ),
            ));
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
}
