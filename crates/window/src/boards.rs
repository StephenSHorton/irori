//! Shared boards (I:boards_service.go): the Boards dialog's list and
//! actions, Share to board and Withdraw on a card's page, and the cards of
//! every board in the grid (design v0.5-shared-boards.md §8, step 4).

use coord::{Activity, Comment, Task};
use serde::Serialize;
use specta::Type;

use crate::Window;
use crate::cards::{CardBoard, CardDetail, SharedCopy};

/// A card on a shared board; its scope label is the board's name.
pub const SCOPE_BOARD: &str = "board";

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

impl From<irori_core::BoardSummary> for BoardSummary {
    fn from(s: irori_core::BoardSummary) -> BoardSummary {
        BoardSummary {
            board_id: s.board_id,
            name: s.name,
            tag: s.tag,
            people: s.people,
            owner: s.owner,
            devices: s.devices,
            cards: s.cards,
            ready: s.ready,
            tag_clash: s.tag_clash,
        }
    }
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

impl From<irori_core::SharePreview> for SharePreview {
    fn from(p: irori_core::SharePreview) -> SharePreview {
        SharePreview {
            board_id: p.board_id,
            board_name: p.board_name,
            title: p.title,
            goal: p.goal,
            done_when: p.done_when,
            steps: p.steps,
            links: p.links,
            repo: p.repo,
            visible_to: p.visible_to,
            dropped: p.dropped,
            hash: p.hash,
        }
    }
}

/// The boards:changed event.
pub const EVENT_BOARDS_CHANGED: &str = "boards:changed";

fn text(e: irori_core::BoardError) -> String {
    e.to_string()
}

impl Window {
    /// BoardsService.List: every board on this device, by name.
    pub fn boards_list(&self) -> Vec<BoardSummary> {
        self.core.board_list().into_iter().map(Into::into).collect()
    }

    fn boards_find(&self, id: &str) -> Result<BoardSummary, String> {
        self.boards_list()
            .into_iter()
            .find(|b| b.board_id == id)
            .ok_or_else(|| format!("core: no such board on this device: {id}"))
    }

    /// BoardsService.Create: a board between this person's own devices,
    /// named `name` with the short tag its card refs carry (`mlg#12`).
    pub fn boards_create(&self, name: &str, tag: &str) -> Result<BoardSummary, String> {
        let id = self
            .core
            .create_board(name, &crate::trim(tag).to_lowercase())
            .map_err(text)?;
        self.boards_find(&id)
    }

    /// BoardsService.Rename (owners).
    pub fn boards_rename(&self, board_id: &str, name: &str) -> Result<BoardSummary, String> {
        self.core.rename_board(board_id, name).map_err(text)?;
        self.boards_find(board_id)
    }

    /// BoardsService.Leave: off the board on every one of the person's
    /// devices.
    pub fn boards_leave(&self, board_id: &str) -> Result<(), String> {
        self.core.leave_board(board_id).map_err(text)
    }

    /// BoardsService.Forget: dropped on this device only.
    pub fn boards_forget(&self, board_id: &str) -> Result<(), String> {
        self.core.forget_board(board_id).map_err(text)
    }

    /// BoardsService.SharePreview: exactly what Share would write (§5.2).
    pub fn boards_share_preview(
        &self,
        task_id: &str,
        board_id: &str,
    ) -> Result<SharePreview, String> {
        self.core
            .preview_share(task_id, board_id)
            .map(Into::into)
            .map_err(text)
    }

    /// BoardsService.Share: the private card goes to the board as the
    /// confirmed preview showed; returns the shared card's id.
    pub fn boards_share(
        &self,
        task_id: &str,
        board_id: &str,
        hash: &str,
    ) -> Result<String, String> {
        self.core
            .share_to_board(task_id, board_id, hash)
            .map(|t| t.id)
            .map_err(text)
    }

    /// BoardsService.Withdraw: the shared card comes off its board (§5.3);
    /// returns the private card the work carries on in, or "".
    pub fn boards_withdraw(&self, task_id: &str) -> Result<String, String> {
        self.core
            .withdraw_card(task_id)
            .map(|t| t.map(|t| t.id).unwrap_or_default())
            .map_err(text)
    }

    /// A board card with its board keys as this device knows them (Go
    /// `privTask`).
    pub(crate) fn priv_task(&self, t: Task) -> Task {
        self.core.private_task(t)
    }

    pub(crate) fn priv_comments(&self, mut cs: Vec<Comment>) -> Vec<Comment> {
        for c in &mut cs {
            c.by = self.core.private_key(&c.by);
        }
        cs
    }

    pub(crate) fn priv_activity(&self, mut acts: Vec<Activity>) -> Vec<Activity> {
        for a in &mut acts {
            a.by = self.core.private_key(&a.by);
        }
        acts
    }

    /// A card page's board line, or where a private card was shared to
    /// (Go `boardDetail`).
    pub(crate) fn board_detail(&self, t: &Task, d: &mut CardDetail) {
        if let Some(s) = self.core.board_of(&t.id) {
            d.summary.scope = SCOPE_BOARD.into();
            d.summary.scope_label = s.name.clone();
            d.summary.board = s.board_id.clone();
            d.summary.tag = s.tag.clone();
            // The fold takes a cancel only from the creator's person or an
            // owner (§5.3); with one person on the board that is always
            // them.
            let user = self.core.user_sender().key;
            d.on_board = Some(CardBoard {
                id: s.board_id,
                name: s.name,
                tag: s.tag,
                visible_to: s.people,
                can_withdraw: !t.withdrawn
                    && (s.owner || t.created_by.starts_with("user:") || t.created_by == user),
            });
            return;
        }
        if let Some(sc) = self.core.shared_copy_of(&t.id) {
            d.shared_to = Some(SharedCopy {
                board_id: sc.board_id,
                board_name: sc.board_name,
                task_id: sc.task_id,
                ref_: sc.reference,
                withdrawn: sc.withdrawn,
            });
        }
    }
}

// ---- routing an action to the log a card lives in (I:board_route.go) ----------

impl Window {
    /// The coord the person's action on card `id` goes to, with the
    /// person's key first in the keys returned and then each of `more`, as
    /// that log knows them (Go `on`).
    pub(crate) fn on(
        &self,
        id: &str,
        more: &[&str],
    ) -> Result<(std::sync::Arc<coord::Coord>, Vec<String>), String> {
        let mut keys = vec![self.core.user_sender().key];
        keys.extend(more.iter().map(|k| k.to_string()));
        self.core.task_coord(id, &keys).map_err(|e| e.to_string())
    }

    /// The coord holding card `id`, for reads: no key is translated, so
    /// nothing is written (Go `read`).
    pub(crate) fn read(&self, id: &str) -> std::sync::Arc<coord::Coord> {
        match self.core.route(id) {
            Ok((_, co)) => co,
            Err(_) => self.core.coord().clone(),
        }
    }

    /// `on` for an action kept on the private board alone (pins): a shared
    /// card is refused rather than written to its board (Go `privateOnly`).
    pub(crate) fn private_only(
        &self,
        id: &str,
    ) -> Result<(std::sync::Arc<coord::Coord>, Vec<String>), String> {
        let (co, keys) = self.on(id, &[])?;
        if !std::sync::Arc::ptr_eq(&co, self.core.coord()) {
            return Err(format!(
                "{}: a card on a shared board isn't pinned",
                coord::ErrorKind::Invalid.text()
            ));
        }
        Ok((co, keys))
    }
}
