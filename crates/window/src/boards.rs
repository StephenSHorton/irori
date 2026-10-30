//! Shared boards (I:boards_service.go): the window's list and actions.
//! Stage 2 boards are M9 (architecture.md 3.3); until then the list is
//! empty and every action says so.

use serde::Serialize;
use specta::Type;

use crate::Window;

/// One shared board in the list (Go `core.BoardSummary`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BoardSummary {
    pub board_id: String,
    pub name: String,
    pub tag: String,
    pub people: Vec<String>,
    pub owner: bool,
    pub devices: i64,
    pub cards: i64,
    pub ready: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub tag_clash: bool,
}

/// What sharing a card would put on a board (Go `core.SharePreview`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
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
    pub dropped: Vec<String>,
    pub hash: String,
}

/// The boards:changed event.
pub const EVENT_BOARDS_CHANGED: &str = "boards:changed";

const NO_BOARDS: &str = "shared boards aren't in this build yet";

impl Window {
    /// BoardsService.List.
    pub fn boards_list(&self) -> Vec<BoardSummary> {
        Vec::new()
    }

    /// BoardsService.Create, Rename, Leave, Forget, Share, Withdraw.
    pub fn boards_refused(&self) -> String {
        NO_BOARDS.into()
    }
}
