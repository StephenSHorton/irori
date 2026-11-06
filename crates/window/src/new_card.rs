//! New card (card #116): the person makes a card straight from the app,
//! without an ask. It goes up unheld and Irori hands it to one free agent
//! (pull, not push: card #82), or to the agent the person picked.

use coord::NewTask;
use serde::Deserialize;
use specta::Type;

use crate::Window;
use crate::board::{TaskResult, Told, UploadFile};

/// What the New card form sends.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct NewCardInput {
    pub title: String,
    pub objective: String,
    pub done_when: String,
    /// The repo it belongs to ("": none).
    pub repo_key: String,
    /// An agent's key: hand it to that agent instead of the free one.
    pub assign_to: String,
    /// A shared board's id (card #126): the card goes on that board, made
    /// from its page. "": the person's own board.
    pub board: String,
    /// Files pasted, dropped or picked in the form (card #124): they go on
    /// the card as the person's first comment, and with the hand-out.
    pub files: Vec<UploadFile>,
}

impl Window {
    /// BoardService.Create: a card on the person's own board.
    pub fn create_card(&self, input: NewCardInput) -> Result<TaskResult, String> {
        let title = crate::trim(&input.title);
        if title.is_empty() {
            return Err(format!(
                "{}: give the card a title",
                coord::ErrorKind::Invalid.text()
            ));
        }
        let board = crate::trim(&input.board);
        if !board.is_empty() {
            if !input.files.is_empty() {
                return Err(format!(
                    "{}: files on a shared board's new card aren't supported yet; add them in a comment",
                    coord::ErrorKind::Invalid.text()
                ));
            }
            return self.create_on_board(board, title, &input);
        }
        // Stored before the card is made: a file that can't be stored
        // leaves no card behind.
        let files: Vec<coord::FileRef> = self
            .store_uploads(&input.files)?
            .iter()
            .map(crate::coord_ref)
            .collect();
        let co = self.core.coord().clone();
        let me = self.core.user_sender().key;
        let t = co
            .create_task(
                &me,
                NewTask {
                    title: title.into(),
                    objective: crate::trim(&input.objective).into(),
                    done_criteria: crate::trim(&input.done_when).into(),
                    repo_key: crate::trim(&input.repo_key).into(),
                    ..NewTask::default()
                },
            )
            .map_err(|e| e.to_string())?;
        if !files.is_empty() {
            // Nobody holds it yet: the comment tells no one by itself.
            co.comment_on(
                &me,
                &t.id,
                coord::CommentInput {
                    text: "Files with the card.".into(),
                    files: files.clone(),
                    ..Default::default()
                },
            )
            .map_err(|e| e.to_string())?;
        }
        let to = crate::trim(&input.assign_to);
        if !to.is_empty() {
            let (t, mut n) = co.assign(&me, &t.id, to).map_err(|e| e.to_string())?;
            // The person picked this agent: it hears now, idle or not.
            n.wake = true;
            n.attachments = files;
            let told = self.tell_from(&self.core.user_sender(), &n);
            return Ok(TaskResult {
                task: t,
                told,
                ..TaskResult::default()
            });
        }
        let n = co
            .offer_card(&me, &t.id, &self.core.ask_agents())
            .map_err(|e| e.to_string())?;
        let mut n = n;
        n.attachments = files;
        // From Irori, not the person: it hands the card out.
        let told = self.tell_from(&inbox::system_sender(), &n);
        let t = co.get_task(&t.id).unwrap_or(t);
        Ok(TaskResult {
            task: t,
            told,
            ..TaskResult::default()
        })
    }

    /// New card on a shared board's page (card #126): written to that
    /// board's log as the person. Unheld, it waits there for someone on the
    /// board to take it up, as an agent's board_create does; picked, it
    /// goes to that agent, woken.
    fn create_on_board(
        &self,
        board: &str,
        title: &str,
        input: &NewCardInput,
    ) -> Result<TaskResult, String> {
        let co = self.core.board_coord(board).map_err(|e| e.to_string())?;
        let me = self
            .core
            .board_key(board, &self.core.user_sender().key)
            .map_err(|e| e.to_string())?;
        let task = NewTask {
            title: title.into(),
            objective: crate::trim(&input.objective).into(),
            done_criteria: crate::trim(&input.done_when).into(),
            repo_key: crate::trim(&input.repo_key).into(),
            ..NewTask::default()
        };
        let to = crate::trim(&input.assign_to);
        if to.is_empty() {
            let t = co.create_task(&me, task).map_err(|e| e.to_string())?;
            return Ok(TaskResult {
                task: t,
                ..TaskResult::default()
            });
        }
        // Made and given in one step: no sweep sees it unheld (card #573).
        let to = self.core.board_key(board, to).map_err(|e| e.to_string())?;
        let (t, mut n) = co
            .create_and_assign(&me, task, &to)
            .map_err(|e| e.to_string())?;
        n.wake = true;
        let told = self.tell_from(&self.core.user_sender(), &n);
        Ok(TaskResult {
            task: t,
            told,
            ..TaskResult::default()
        })
    }

    /// Routes `n` from `from`; nobody to tell is not an error.
    pub(crate) fn tell_from(&self, from: &inbox::Sender, n: &coord::Notice) -> Told {
        if n.to.is_empty() {
            return Told::default();
        }
        let mut told = Told {
            to: n.to.iter().map(|k| self.core.private_key(k)).collect(),
            mode: if n.wake {
                inbox::mode::WAKE.into()
            } else {
                inbox::mode::NEXT_TURN.into()
            },
            error: String::new(),
        };
        if let Err(e) = self.core.route_notices(from, std::slice::from_ref(n)) {
            told.error = e;
        }
        told
    }
}
