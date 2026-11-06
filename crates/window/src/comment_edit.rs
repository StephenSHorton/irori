//! Modify on the person's comment before any agent read it (card #105).
//!
//! Modify holds the comment's notices that still wait for their agents'
//! turns, so none of them gets it while the person edits; Save writes the
//! new text (task.comment.edited) and sends each held notice again with
//! it; Cancel lets the held ones go as they were. Only a private card's
//! comments: a shared board's members bridged theirs on their own devices.

use std::sync::Arc;

use inbox::{EditState, SendOptions};

use crate::Window;

fn comment_ref(cid: &str) -> String {
    format!("comment:{cid}")
}

/// The notice's text with the comment's old text swapped for the new one;
/// a plain line when the old text isn't in it.
fn rewritten(notice: &str, old: &str, new: &str) -> String {
    if !old.is_empty() && notice.contains(old) {
        return notice.replacen(old, new, 1);
    }
    format!("The person edited their comment before you read it:\n{new}")
}

impl Window {
    /// The private coord and the person's key, once the comment is theirs
    /// on card `task_id`.
    fn editable(
        &self,
        task_id: &str,
        cid: &str,
    ) -> Result<(Arc<coord::Coord>, String, coord::Comment), String> {
        let (task_id, cid) = (crate::trim(task_id), crate::trim(cid));
        let (co, keys) = self.on(task_id, &[])?;
        if !Arc::ptr_eq(&co, self.core.coord()) {
            return Err(format!(
                "{}: a comment in a shared space can't be edited: the other people's devices already have it",
                coord::ErrorKind::Invalid.text()
            ));
        }
        let me = keys[0].clone();
        let cm = co.comment_editable(&me, cid).map_err(|e| e.to_string())?;
        if cm.task_id != task_id {
            return Err(format!(
                "{}: comment {cid} isn't on that card",
                coord::ErrorKind::NotFound.text()
            ));
        }
        Ok((co, me, cm))
    }

    /// Modify: holds the comment back from every agent that hasn't read it.
    pub fn comment_modify(&self, task_id: &str, cid: &str) -> Result<(), String> {
        let (_, me, cm) = self.editable(task_id, cid)?;
        let n = self
            .core
            .inbox()
            .hold_for_edit(&me, &comment_ref(&cm.id), &me)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err(format!(
                "{}: no agent is waiting for that comment",
                inbox::ErrorKind::BadState.text()
            ));
        }
        Ok(())
    }

    /// Cancel: the held comment goes out as it was.
    pub fn comment_modify_cancel(&self, task_id: &str, cid: &str) -> Result<(), String> {
        let (_, me, cm) = self.editable(task_id, cid)?;
        self.core
            .inbox()
            .release_edit(&me, &comment_ref(&cm.id), &me)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Save: the comment's new text, sent to every agent it was held from.
    /// Without Modify first it holds them now, so an agent never gets both.
    pub fn comment_modify_save(
        &self,
        task_id: &str,
        cid: &str,
        text: &str,
    ) -> Result<coord::Comment, String> {
        let (co, me, cm) = self.editable(task_id, cid)?;
        let ib = self.core.inbox();
        let r = comment_ref(&cm.id);
        if ib.edit_state(&me, &r) != EditState::Editing {
            self.comment_modify(task_id, cid)?;
        }
        let edited = match co.edit_comment(&me, &cm.id, text) {
            Ok(c) => c,
            Err(e) => {
                // The text was refused: nothing changed, so let it go as it was.
                let _ = ib.release_edit(&me, &r, &me);
                return Err(e.to_string());
            }
        };
        if edited.text == cm.text {
            ib.release_edit(&me, &r, &me).map_err(|e| e.to_string())?;
            return Ok(edited);
        }
        let held = ib.take_edit(&me, &r, &me).map_err(|e| e.to_string())?;
        let from = self.core.user_sender();
        let mut errs = Vec::new();
        for d in held {
            let m = &d.message;
            let o = SendOptions {
                reason: m.reason.clone(),
                mode: d.mode.clone(),
                ref_: m.ref_.clone(),
                notice: true,
                attachments: m.attachments.clone(),
                mentions: m.mentions.clone(),
                parent: m.parent_id.clone(),
            };
            if let Err(e) =
                ib.send_with(&from, &m.to, &rewritten(&m.text, &cm.text, &edited.text), o)
            {
                errs.push(format!("to {}: {e}", d.to));
            }
        }
        if !errs.is_empty() {
            return Err(format!(
                "saved, but it didn't reach every agent again: {}",
                errs.join("; ")
            ));
        }
        Ok(edited)
    }
}

#[cfg(test)]
mod tests {
    use super::rewritten;

    #[test]
    fn rewritten_swaps_the_comment_text() {
        let n = "The person commented on \"x\" (#1):\nold words\nAnswer it on the card.";
        assert_eq!(
            rewritten(n, "old words", "new words"),
            "The person commented on \"x\" (#1):\nnew words\nAnswer it on the card."
        );
        assert!(rewritten("merged digest", "old", "new").ends_with("\nnew"));
    }
}
