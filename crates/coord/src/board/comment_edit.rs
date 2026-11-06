//! The person edits their comment before any agent read it (card #105).
//!
//! The log is append-only: an edit is a `task.comment.edited` row carrying
//! the new text, and the comment folds to it. Coord can't see the inbox, so
//! whether an agent read the comment yet is the caller's check (the window
//! holds the comment's deliveries while the person edits, and resends them
//! with the new text). A shared board's members bridge the first row on
//! their own devices, so only a private card's comment is edited.

use gocompat::time::GoTime;
use store::{Event, kinds};

use super::comments::check_comment;
use super::error::{Result, invalid, not_allowed, not_found};
use super::fold::Fold;
use super::rows::*;
use super::types::{COMMENT_ADDED, Change, Comment};
use crate::names::trim_space;

impl Fold {
    /// The comment `by` may edit: their own, on a private card, not a
    /// message recorded on the thread.
    pub(crate) fn comment_editable(&self, by: &str, comment_id: &str) -> Result<Comment> {
        if self.on_board {
            return Err(invalid(
                "a comment in a shared space can't be edited: other people's devices already have it",
            ));
        }
        let Some(cm) = self.comments.get(comment_id) else {
            return Err(not_found(format!("comment {comment_id}")));
        };
        if !self.is_owner(by) || cm.by != by {
            return Err(not_allowed("only the person edits their own comment"));
        }
        if !cm.msg_id.is_empty() {
            return Err(invalid("a message on the thread can't be edited"));
        }
        if !self.tasks.contains_key(&cm.task_id) {
            return Err(not_found(format!("task {}", cm.task_id)));
        }
        Ok(cm.clone())
    }

    /// Writes the person's new text for their comment.
    pub(crate) fn edit_comment(
        &mut self,
        by: &str,
        comment_id: &str,
        text: &str,
    ) -> Result<Comment> {
        let (by, text) = check_comment(by, text, &[], self.is_owner(trim_space(by)))?;
        let cm = self.comment_editable(&by, comment_id)?;
        if cm.text == text {
            return Ok(cm);
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_COMMENT_EDITED,
            &TaskCommentEditedRow {
                task_id: cm.task_id.clone(),
                comment_id: comment_id.into(),
                by: by.clone(),
                text,
                at,
            },
        )?;
        self.comments
            .get(comment_id)
            .cloned()
            .ok_or_else(|| invalid(format!("comment {comment_id} did not fold")))
    }

    pub(crate) fn fold_comment_edited(&mut self, ev: &Event, at: &dyn Fn(GoTime) -> GoTime) {
        if self.on_board {
            return;
        }
        let Some(p) = row::<TaskCommentEditedRow>(&ev.payload) else {
            return;
        };
        if p.text.is_empty() || !self.is_owner(&p.by) {
            return;
        }
        let Some(cm) = self.comments.get_mut(&p.comment_id) else {
            return;
        };
        if cm.by != p.by || cm.task_id != p.task_id || !cm.msg_id.is_empty() {
            return;
        }
        cm.text = p.text;
        cm.edited_at = at(p.at);
        self.note(Change {
            kind: COMMENT_ADDED.into(),
            task_id: p.task_id,
            ..Change::default()
        });
    }
}
