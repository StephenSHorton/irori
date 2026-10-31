//! The person's actions on the Board (I:board_service.go, the answer half
//! of cards_service.go). Every action runs as the person at this device
//! (`Core::user_sender`); notices coord returns go out through the inbox.
//!
//! Only the methods the window calls are here (surface.md 2.9); Go's
//! bound-but-unused Board/Task/Create/Block/Unblock/Assign/SetDone/AddLink/
//! RemoveLink/ReleaseLease are not.

use blob::FileRef;
use coord::{BlockerAnswer, Notice, Task};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::Window;
use crate::cards::{AgentRef, CardDetail, NOTE_ITEM, not_found_task};
use crate::ts;

/// MaxUpload caps one file added from the window.
pub const MAX_UPLOAD: usize = 32 << 20;

/// Who an action's notice went to.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct Told {
    pub to: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub mode: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub error: String,
}

/// A task action's answer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct TaskResult {
    #[specta(type = ts::Task)]
    pub task: Task,
    pub told: Told,
}

/// Comment's answer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct CommentResult {
    #[specta(type = ts::Comment)]
    pub comment: coord::Comment,
    pub told: Told,
    pub answers: Vec<AgentRef>,
    pub watching: Vec<AgentRef>,
}

/// Answer's reply: the card after the answer, and who heard.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct AnswerResult {
    pub card: CardDetail,
    pub told: Told,
}

/// One choice on the ask box's repo chip.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AskRepo {
    pub repo_key: String,
    pub label: String,
}

/// A file the person put in the window: its name and bytes (base64 in
/// JSON, as Go's `[]byte`).
#[derive(Debug, Clone, Default, Deserialize, Type)]
pub struct UploadFile {
    pub name: String,
    /// base64.
    pub data: String,
}

/// The person's per-comment choice of when the agent hears it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tell {
    NextTurn,
    Wake,
    None,
}

fn parse_tell(s: &str) -> Result<Tell, String> {
    match crate::trim(s) {
        "" | "next_turn" => Ok(Tell::NextTurn),
        "wake" => Ok(Tell::Wake),
        "none" => Ok(Tell::None),
        _ => Err(format!(
            "tell is next_turn, wake or none, not {}",
            gocompat::quote::quote(s)
        )),
    }
}

fn trim(s: &str) -> &str {
    crate::trim(s)
}

fn invalid(detail: &str) -> String {
    format!("{}: {detail}", coord::ErrorKind::Invalid.text())
}

impl Window {
    fn user_key(&self) -> String {
        self.core.user_sender().key
    }

    /// Routes `n` as `tl` chose (Go `tell`).
    fn tell(&self, n: &Notice, tl: Tell) -> Told {
        if tl == Tell::None || n.to.is_empty() {
            return Told::default();
        }
        let mut told = Told {
            to: n.to.iter().map(|k| self.core.private_key(k)).collect(),
            mode: inbox::mode::NEXT_TURN.into(),
            error: String::new(),
        };
        if n.wake {
            told.mode = inbox::mode::WAKE.into();
        }
        if let Err(e) = self
            .core
            .route_notices(&self.core.user_sender(), std::slice::from_ref(n))
        {
            told.error = e;
        }
        told
    }

    /// BoardService.Comment: the person's comment on a task.
    pub fn comment(&self, id: &str, text: &str, tell: &str) -> Result<CommentResult, String> {
        let tl = parse_tell(tell)?;
        let (co, keys) = self.on(id, &[])?;
        let me = keys[0].clone();
        let mut res = CommentResult::default();
        if tl == Tell::None {
            let (c, _) = co
                .add_comment(&me, id, text, false)
                .map_err(|e| e.to_string())?;
            res.comment = c;
            return Ok(res);
        }
        let p = self.people();
        // A board card's keys are agent:<cardId>; the roster knows the
        // session behind each (private_key).
        let around = |k: &str| p.ref_(&self.core.private_key(k)).live;
        let (c, ns) = co
            .person_comment(&me, id, text, tl == Tell::Wake, Some(&around))
            .map_err(|e| e.to_string())?;
        res.comment = c;
        for (i, n) in ns.iter().enumerate() {
            let told = self.tell(n, tl);
            res.told.to.extend(told.to.iter().cloned());
            if i == 0 {
                res.told.mode = told.mode.clone();
            }
            if !told.error.is_empty() {
                let joined = format!("{}; {}", res.told.error, told.error);
                res.told.error = joined.trim_start_matches("; ").to_string();
            }
            let to: Vec<String> = n.to.iter().map(|k| self.core.private_key(k)).collect();
            if n.watching {
                res.watching.extend(p.refs(&to));
            } else {
                res.answers.extend(p.refs(&to));
            }
        }
        Ok(res)
    }

    /// BoardService.AskWithFiles (Ask with no files): the person asking
    /// the room. With `board_id` the ask goes on that shared board (v0.5
    /// §9): every member's free agents hear it, the others' through their
    /// own devices. A board's agents carry no repo, and files don't go on
    /// a board ask yet (a board row may name only attachments offered in
    /// it).
    pub fn ask(
        &self,
        text: &str,
        repo_key: &str,
        wake: bool,
        files: &[UploadFile],
        board_id: &str,
    ) -> Result<TaskResult, String> {
        let board_id = trim(board_id);
        if !board_id.is_empty() {
            if !files.is_empty() {
                return Err(invalid(
                    "files go with an ask on your own board only; ask without them, or attach them on a card",
                ));
            }
            let (t, n) = self
                .core
                .board_ask(
                    board_id,
                    coord::NewAsk {
                        text: text.into(),
                        repo_key: trim(repo_key).into(),
                        wake,
                        ..Default::default()
                    },
                )
                .map_err(|e| e.to_string())?;
            let told = self.tell(&n, Tell::NextTurn);
            return Ok(TaskResult { task: t, told });
        }
        let refs = self.store_uploads(files)?;
        let agents = self.core.ask_agents();
        let (t, n) = self
            .core
            .coord()
            .post_ask(
                &self.user_key(),
                coord::NewAsk {
                    text: text.into(),
                    files: refs.iter().map(crate::coord_ref).collect(),
                    repo_key: trim(repo_key).into(),
                    wake,
                },
                &agents,
            )
            .map_err(|e| e.to_string())?;
        let told = self.tell(&n, Tell::NextTurn);
        Ok(TaskResult { task: t, told })
    }

    /// BoardService.AskAgain: "Wake them" on an ask nobody took.
    pub fn ask_again(&self, id: &str) -> Result<(), String> {
        self.core
            .coord()
            .ask_again(&self.user_key(), trim(id))
            .map_err(|e| e.to_string())
    }

    /// BoardService.CloseAsk: an ask ends with nothing coming of it.
    pub fn close_ask(&self, id: &str, note: &str) -> Result<Task, String> {
        self.core
            .coord()
            .ask_close(
                &self.user_key(),
                trim(id),
                coord::AskOutcome {
                    kind: coord::OUTCOME_NONE.into(),
                    note: note.into(),
                    ..Default::default()
                },
            )
            .map_err(|e| e.to_string())
    }

    /// The person closes a card by hand (card #57): Done (completed) or
    /// Canceled, with an optional note. Any card; an open ask closes as
    /// nothing came of it.
    pub fn close_card(&self, id: &str, state: &str, note: &str) -> Result<Task, String> {
        let state = trim(state);
        if state != coord::COMPLETED && state != coord::CANCELED {
            return Err(format!(
                "{}: close a card as completed or canceled",
                coord::ErrorKind::Invalid.text()
            ));
        }
        // A shared card closes in its board's log, as the person there; its
        // fold decides who may cancel it (v0.5 §5.3).
        let id = trim(id);
        let (co, keys) = self
            .core
            .task_coord(id, &[self.user_key()])
            .map_err(|e| e.to_string())?;
        co.set_task_state(&keys[0], id, state, note)
            .map_err(|e| e.to_string())
    }

    /// The person deletes a card (card #57): it leaves the Board on every
    /// device, and the signed log keeps it. A card on a shared board isn't
    /// deleted: other people already have it, so it is withdrawn (§5.3).
    pub fn delete_card(&self, id: &str) -> Result<(), String> {
        let id = trim(id);
        if let Ok((board_id, _)) = self.core.route(id)
            && !board_id.is_empty()
        {
            return Err(format!(
                "{}: a card on a shared board is withdrawn, not deleted: the others on it already have it",
                coord::ErrorKind::Invalid.text()
            ));
        }
        self.core
            .coord()
            .delete_task(&self.user_key(), id)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// BoardService.Pin.
    pub fn pin(&self, id: &str, pinned: bool) -> Result<(), String> {
        let (co, keys) = self.private_only(trim(id))?;
        co.pin(&keys[0], trim(id), pinned)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// BoardService.MovePin: just before another pinned card ("" last).
    pub fn move_pin(&self, id: &str, before: &str) -> Result<(), String> {
        let (co, keys) = self.private_only(trim(id))?;
        co.move_pin(&keys[0], trim(id), trim(before))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// BoardService.AskRepos: the repos the roster's agents are in.
    pub fn ask_repos(&self) -> Vec<AskRepo> {
        let mut out: Vec<AskRepo> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for a in self.core.mesh_agents() {
            if a.repo_key.is_empty() || a.status == "gone" || !seen.insert(a.repo_key.clone()) {
                continue;
            }
            out.push(AskRepo {
                label: repo_label(&a.repo_key),
                repo_key: a.repo_key,
            });
        }
        out.sort_by(|a, b| a.label.cmp(&b.label));
        out
    }

    /// BoardService.Reopen.
    pub fn reopen(&self, id: &str) -> Result<TaskResult, String> {
        let (co, keys) = self.on(trim(id), &[])?;
        let (t, n) = co.reopen(&keys[0], trim(id)).map_err(|e| e.to_string())?;
        let told = self.tell(&n, Tell::NextTurn);
        Ok(TaskResult { task: t, told })
    }

    /// BoardService.Answer: the person's answer to one blocker; option is
    /// the picked recommendation's index, or -1 for own words.
    pub fn answer(
        &self,
        task_id: &str,
        blocker_id: &str,
        option: i64,
        text: &str,
        wake: bool,
    ) -> Result<AnswerResult, String> {
        if option < -1 {
            return Err(invalid("option is an index or -1"));
        }
        let (task_id, blocker_id) = (trim(task_id), trim(blocker_id));
        if blocker_id == NOTE_ITEM {
            return self.answer_input(task_id, option, text, wake);
        }
        let (co, keys) = self.on(task_id, &[])?;
        let me = keys[0].clone();
        let t = co
            .get_task(task_id)
            .ok_or_else(|| not_found_task(task_id))?;
        let Some(bl) = t.blockers.iter().find(|b| b.id == blocker_id) else {
            return Err(format!(
                "{}: task {task_id} has no blocker {}",
                coord::ErrorKind::NotFound.text(),
                gocompat::quote::quote(blocker_id)
            ));
        };
        let mut a = BlockerAnswer {
            pick: option + 1,
            text: text.into(),
            wake,
            resolve: self.nobody_can_act(&t, &bl.raised_by),
        };
        let status = if bl.status.is_empty() {
            coord::BLOCKER_OPEN
        } else {
            bl.status.as_str()
        };
        if a.resolve && a.pick == 0 && status != coord::BLOCKER_OPEN {
            co.answer(
                &me,
                task_id,
                blocker_id,
                BlockerAnswer {
                    text: text.into(),
                    wake,
                    ..BlockerAnswer::default()
                },
            )
            .map_err(|e| e.to_string())?;
            a.text = String::new();
        }
        let (t, n) = co
            .answer(&me, task_id, blocker_id, a)
            .map_err(|e| e.to_string())?;
        let told = self.tell(&n, Tell::NextTurn);
        Ok(AnswerResult {
            card: self.card(&t.id)?,
            told,
        })
    }

    /// Whether no agent will act on an answer to an item `raiser` raised.
    fn nobody_can_act(&self, t: &Task, raiser: &str) -> bool {
        let p = self.people();
        let r = p.ref_(raiser);
        if raiser.is_empty() || r.status == "person" || r.status == "irori" {
            return true;
        }
        let live = |key: &str| {
            let r = p.ref_(key);
            r.live && r.status != "person" && r.status != "irori"
        };
        !live(raiser) && !t.holders.iter().any(|h| live(h))
    }

    /// BoardService.ResolveBlocker: "Mark resolved" on an item.
    pub fn resolve_blocker(
        &self,
        task_id: &str,
        blocker_id: &str,
        note: &str,
    ) -> Result<AnswerResult, String> {
        let (task_id, blocker_id) = (trim(task_id), trim(blocker_id));
        if blocker_id == NOTE_ITEM {
            return Err(invalid("answer the card's question instead"));
        }
        let (co, keys) = self.on(task_id, &[])?;
        let (t, n) = co
            .answer(
                &keys[0],
                task_id,
                blocker_id,
                BlockerAnswer {
                    text: note.into(),
                    resolve: true,
                    ..BlockerAnswer::default()
                },
            )
            .map_err(|e| e.to_string())?;
        let told = self.tell(&n, Tell::NextTurn);
        Ok(AnswerResult {
            card: self.card(&t.id)?,
            told,
        })
    }

    /// Answers an input_required card's note.
    fn answer_input(
        &self,
        id: &str,
        option: i64,
        text: &str,
        wake: bool,
    ) -> Result<AnswerResult, String> {
        let (co, keys) = self.on(id, &[])?;
        let me = keys[0].clone();
        let t = co.get_task(id).ok_or_else(|| not_found_task(id))?;
        if t.state != coord::INPUT_REQUIRED {
            return Err(format!(
                "{}: task {id} is {}, not waiting on your input",
                coord::ErrorKind::BadState.text(),
                t.state
            ));
        }
        if option != -1 {
            return Err(invalid(
                "this question has no recommended answers; answer in your own words",
            ));
        }
        let (_, n) = co
            .add_comment(&me, id, text, wake)
            .map_err(|e| e.to_string())?;
        let told = self.tell(&n, Tell::NextTurn);
        let back = if t.holders.len() > t.assigned.len() {
            coord::WORKING
        } else {
            coord::SUBMITTED
        };
        co.set_task_state(&me, id, back, "")
            .map_err(|e| e.to_string())?;
        Ok(AnswerResult {
            card: self.card(id)?,
            told,
        })
    }

    /// Writes files to a temporary folder outside the data dir, stores
    /// them as the person's attachments, then removes the folder (Go
    /// `storeUploads` + `Core.AttachFiles`).
    fn store_uploads(&self, files: &[UploadFile]) -> Result<Vec<FileRef>, String> {
        use base64::Engine as _;
        if files.is_empty() {
            return Ok(Vec::new());
        }
        if files.len() > inbox::MAX_ATTACHMENTS {
            return Err(format!("at most {} files", inbox::MAX_ATTACHMENTS));
        }
        let dir = tempfile::Builder::new()
            .prefix("irori-upload-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let mut out = Vec::with_capacity(files.len());
        for (i, f) in files.iter().enumerate() {
            let data = base64::engine::general_purpose::STANDARD
                .decode(f.data.as_bytes())
                .map_err(|e| format!("{}: {e}", f.name))?;
            if data.len() > MAX_UPLOAD {
                return Err(format!(
                    "{} is over {} MiB; ask an agent to attach it from disk",
                    f.name,
                    MAX_UPLOAD >> 20
                ));
            }
            let sub = dir.path().join(i.to_string());
            std::fs::create_dir(&sub).map_err(|e| e.to_string())?;
            let base = std::path::Path::new(&f.name)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let p = sub.join(blob::safe_name(&base, "file"));
            std::fs::write(&p, &data).map_err(|e| e.to_string())?;
            let path = p.to_string_lossy().into_owned();
            let real = blob::check_attach_path(&path, "", &self.core.data_dir().to_string_lossy())
                .map_err(|e| e.to_string())?;
            let mut r = self
                .core
                .files()
                .put_file(&real)
                .map_err(|e| format!("{}: {e}", base))?;
            r.name = blob::safe_name(&base, &r.sha256);
            r.device = self.core.device_id().to_string();
            self.seams.mesh.offer(&r)?;
            out.push(r);
        }
        Ok(out)
    }
}

/// A repoKey's last path part ("remote:github.com/a/irori" is "irori").
pub fn repo_label(key: &str) -> String {
    let k = key.trim_end_matches('/');
    match k.rfind(['/', ':', '\\']) {
        Some(i) if i < k.len() - 1 => k[i + 1..].to_string(),
        _ => k.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_labels() {
        assert_eq!(repo_label("remote:github.com/a/irori"), "irori");
        assert_eq!(repo_label("remote:github.com/a/irori/"), "irori");
        assert_eq!(repo_label("path:C:\\x\\y"), "y");
        assert_eq!(repo_label("plain"), "plain");
        assert_eq!(repo_label("a:"), "a:");
    }

    #[test]
    fn tells() {
        assert!(parse_tell("").unwrap() == Tell::NextTurn);
        assert!(parse_tell(" wake ").unwrap() == Tell::Wake);
        assert!(parse_tell("none").unwrap() == Tell::None);
        assert_eq!(
            parse_tell("loud").unwrap_err(),
            "tell is next_turn, wake or none, not \"loud\""
        );
    }
}
