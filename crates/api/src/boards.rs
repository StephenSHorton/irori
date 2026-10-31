//! `GET /v1/boards[?detail=1]` (v0.5 §8): the shared boards this device
//! holds. The window's Boards dialog shows names, members and one status
//! dot; fingerprints, devices and the fold's set-aside counters stay here,
//! behind `detail`.

use serde::Serialize;

/// One shared board.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardRow {
    pub board_id: String,
    pub name: String,
    pub tag: String,
    /// Members' names, the person at this device first.
    pub people: Vec<String>,
    /// The person at this device owns the board.
    pub owner: bool,
    pub devices: i64,
    pub cards: i64,
    /// Its board.created folded here, so cards can go on it (a board
    /// another device made shows before its rows arrive).
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<BoardDetail>,
}

/// What `detail` adds.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardDetail {
    pub members: Vec<BoardMember>,
    /// The fold's set-aside rows: unresolved references, rows dropped
    /// past the cap, and rows the writer rules ignored.
    pub set_aside: coord::SetAside,
    /// Membership rows that didn't count, with why.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
}

/// One member as `detail` shows it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardMember {
    pub person_id: String,
    pub name: String,
    pub fingerprint: String,
    pub role: String,
    pub state: String,
    pub devices: i64,
}
