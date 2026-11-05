//! Where a note lives (decision 45, card #401, docs/design/notes.md §3):
//! a Mine note in the person's private log, a board note in that board's
//! log, never the other way round. Tools and the window reach the log
//! through these, as they reach a card's through `task_coord`.

use std::sync::Arc;

use coord::{Coord, ErrorKind as CK};

use super::Services;
use super::boards::{Error, trim};

/// The scope name of the person's own notes.
pub const SCOPE_MINE: &str = "mine";

/// One log notes can live in: Mine, or a shared board.
#[derive(Clone)]
pub struct NoteLog {
    /// `""` for Mine, else the board's id.
    pub board_id: String,
    /// `"mine"`, or the board's tag.
    pub scope: String,
    pub coord: Arc<Coord>,
}

impl Services {
    /// Whether this device may write note rows into board `board_id`'s log
    /// (#411). An older Irori refuses an unknown kind, and every later row
    /// of that writer's stream then fails its gap check there, so one
    /// note.* row would stall this device's whole stream on any member
    /// device that predates notes (nutmeg, #392). The note kinds landed
    /// before the board hello's `spaces` field (0f6544ab, then 9d5a72dc),
    /// so a device heard to read space rows reads note rows too: the
    /// space gate ([`Services::space_not_ready`]) is the notes gate. Mine
    /// notes never wait.
    pub(crate) fn board_notes_ready(
        &self,
        board_id: &str,
    ) -> std::result::Result<(), coord::Error> {
        let waiting = self.space_not_ready(board_id).map_err(Error::into_coord)?;
        if waiting.is_empty() {
            return Ok(());
        }
        Err(Error::coord(
            CK::NotAllowed,
            format_args!(
                "every device in the space must run an Irori with notes first; waiting on {}. Keep it in Mine for now",
                waiting.join(", ")
            ),
        )
        .into_coord())
    }

    /// The log for `scope` (`""` or `mine`: the private log; else a board's
    /// tag or id), with each of `keys` as that log knows the caller.
    pub(crate) fn notes_log(
        self: &Arc<Self>,
        scope: &str,
        keys: &[String],
    ) -> std::result::Result<(NoteLog, Vec<String>), coord::Error> {
        let scope = trim(scope);
        if scope.is_empty() || scope.eq_ignore_ascii_case(SCOPE_MINE) {
            return Ok((
                NoteLog {
                    board_id: String::new(),
                    scope: SCOPE_MINE.into(),
                    coord: self.coord.clone(),
                },
                keys.to_vec(),
            ));
        }
        let boards = self.shown_boards();
        let hits: Vec<_> = boards
            .iter()
            .filter(|b| b.id == scope || b.verifier.membership().tag.eq_ignore_ascii_case(scope))
            .cloned()
            .collect();
        let b = match hits.as_slice() {
            [b] => b.clone(),
            [] => {
                return Err(Error::coord(
                    CK::NotFound,
                    format_args!("no board {scope:?}: give mine, or a board's tag or id"),
                )
                .into_coord());
            }
            _ => {
                return Err(Error::coord(
                    CK::Ambiguous,
                    format_args!("two boards are tagged {scope:?}: give its id"),
                )
                .into_coord());
            }
        };
        let out = keys
            .iter()
            .map(|k| self.board_key(&b.id, k))
            .collect::<super::boards::Result<Vec<_>>>()
            .map_err(Error::into_coord)?;
        Ok((
            NoteLog {
                board_id: b.id.clone(),
                scope: b.verifier.membership().tag.clone(),
                coord: b.coord().clone(),
            },
            out,
        ))
    }

    /// [`Services::board_notes_ready`] for a board log; Mine always passes.
    pub(crate) fn write_gate(&self, l: &NoteLog) -> std::result::Result<(), coord::Error> {
        if l.board_id.is_empty() {
            return Ok(());
        }
        self.board_notes_ready(&l.board_id)
    }

    /// Every log this device reads notes from: Mine first, then each shown
    /// board.
    pub(crate) fn note_logs(&self) -> Vec<NoteLog> {
        let mut out = vec![NoteLog {
            board_id: String::new(),
            scope: SCOPE_MINE.into(),
            coord: self.coord.clone(),
        }];
        for b in self.shown_boards() {
            out.push(NoteLog {
                board_id: b.id.clone(),
                scope: b.verifier.membership().tag.clone(),
                coord: b.coord().clone(),
            });
        }
        out
    }

    /// The log holding note `id`, with `keys` as it knows them.
    pub(crate) fn note_log(
        self: &Arc<Self>,
        id: &str,
        keys: &[String],
    ) -> std::result::Result<(NoteLog, Vec<String>), coord::Error> {
        let id = trim(id.strip_prefix("note:").unwrap_or(id)).to_string();
        let Some(l) = self
            .note_logs()
            .into_iter()
            .find(|l| l.coord.note_by_id(&id).is_some())
        else {
            return Err(Error::coord(CK::NotFound, format_args!("note {id}")).into_coord());
        };
        if l.board_id.is_empty() {
            return Ok((l, keys.to_vec()));
        }
        let out = keys
            .iter()
            .map(|k| self.board_key(&l.board_id, k))
            .collect::<super::boards::Result<Vec<_>>>()
            .map_err(Error::into_coord)?;
        Ok((l, out))
    }
}

// ---- the Core's notes API (card #401) ---------------------------------------

/// A note with the log it lives in.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopedNote {
    /// `"mine"`, or the board's tag.
    pub scope: String,
    /// `""` for Mine.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub board_id: String,
    #[serde(flatten)]
    pub note: coord::Note,
}

fn scoped(l: &NoteLog, n: coord::Note) -> ScopedNote {
    ScopedNote {
        scope: l.scope.clone(),
        board_id: l.board_id.clone(),
        note: n,
    }
}

impl Services {
    // The writes, shared by the window (through `Core`) and the agent tools
    // (through `ToolsView`, card #412): a gate on board writes (Notes 2,
    // #411) belongs here so both front ends pass it.

    pub(crate) fn propose_note(
        self: &Arc<Self>,
        by: &str,
        scope: &str,
        n: coord::NewNote,
    ) -> std::result::Result<ScopedNote, coord::Error> {
        let (l, keys) = self.notes_log(scope, &[by.to_string()])?;
        self.write_gate(&l)?;
        let note = l.coord.propose_note(&keys[0], n)?;
        Ok(scoped(&l, note))
    }

    pub(crate) fn accept_note(
        self: &Arc<Self>,
        by: &str,
        id: &str,
    ) -> std::result::Result<ScopedNote, coord::Error> {
        let (l, keys) = self.note_log(id, &[by.to_string()])?;
        self.write_gate(&l)?;
        let id = id.strip_prefix("note:").unwrap_or(id);
        Ok(scoped(&l, l.coord.accept_note(&keys[0], id)?))
    }

    pub(crate) fn edit_note(
        self: &Arc<Self>,
        by: &str,
        id: &str,
        e: coord::NoteEdit,
        why: &str,
    ) -> std::result::Result<ScopedNote, coord::Error> {
        let (l, keys) = self.note_log(id, &[by.to_string()])?;
        self.write_gate(&l)?;
        let id = id.strip_prefix("note:").unwrap_or(id);
        Ok(scoped(&l, l.coord.edit_note(&keys[0], id, e, why)?))
    }

    pub(crate) fn remove_note(
        self: &Arc<Self>,
        by: &str,
        id: &str,
        why: &str,
    ) -> std::result::Result<ScopedNote, coord::Error> {
        let (l, keys) = self.note_log(id, &[by.to_string()])?;
        self.write_gate(&l)?;
        let id = id.strip_prefix("note:").unwrap_or(id);
        Ok(scoped(&l, l.coord.remove_note(&keys[0], id, why)?))
    }

    pub(crate) fn search_notes(&self, q: &str, live_only: bool) -> Vec<ScopedNote> {
        self.note_logs()
            .iter()
            .flat_map(|l| {
                l.coord
                    .search_notes(q, live_only)
                    .into_iter()
                    .map(|n| scoped(l, n))
            })
            .collect()
    }
}

impl super::Core {
    /// Writes a note in `scope` (`mine`, or a board's tag or id) as `by`,
    /// the person at this device or one of their agents. A Mine note stays
    /// in the private log whatever its tags name.
    pub fn propose_note(
        &self,
        by: &str,
        scope: &str,
        n: coord::NewNote,
    ) -> std::result::Result<ScopedNote, coord::Error> {
        self.s.propose_note(by, scope, n)
    }

    /// A person accepts note `id` (or an agent's pending change to it).
    pub fn accept_note(&self, by: &str, id: &str) -> std::result::Result<ScopedNote, coord::Error> {
        self.s.accept_note(by, id)
    }

    pub fn edit_note(
        &self,
        by: &str,
        id: &str,
        e: coord::NoteEdit,
        why: &str,
    ) -> std::result::Result<ScopedNote, coord::Error> {
        self.s.edit_note(by, id, e, why)
    }

    pub fn remove_note(
        &self,
        by: &str,
        id: &str,
        why: &str,
    ) -> std::result::Result<ScopedNote, coord::Error> {
        self.s.remove_note(by, id, why)
    }

    /// The notes in `scope` (`""`: every log), newest first.
    pub fn notes(
        &self,
        scope: &str,
        removed: bool,
    ) -> std::result::Result<Vec<ScopedNote>, coord::Error> {
        let logs = if trim(scope).is_empty() {
            self.s.note_logs()
        } else {
            vec![self.s.notes_log(scope, &[])?.0]
        };
        Ok(logs
            .iter()
            .flat_map(|l| l.coord.notes(removed).into_iter().map(|n| scoped(l, n)))
            .collect())
    }

    /// Notes holding every word of `q` across every log this device reads;
    /// `live_only`: accepted only, what agents may read.
    pub fn search_notes(&self, q: &str, live_only: bool) -> Vec<ScopedNote> {
        self.s.search_notes(q, live_only)
    }
}

// ---- the agent tools' seam (card #412) ---------------------------------------

fn tool_note(n: ScopedNote) -> tools::ToolNote {
    tools::ToolNote {
        scope: n.scope,
        note: n.note,
    }
}

impl tools::NotesBackend for super::ToolsView {
    fn propose(
        &self,
        by: &str,
        scope: &str,
        n: coord::NewNote,
    ) -> std::result::Result<tools::ToolNote, coord::Error> {
        self.0.propose_note(by, scope, n).map(tool_note)
    }
    fn edit(
        &self,
        by: &str,
        id: &str,
        e: coord::NoteEdit,
        why: &str,
    ) -> std::result::Result<tools::ToolNote, coord::Error> {
        self.0.edit_note(by, id, e, why).map(tool_note)
    }
    fn remove(
        &self,
        by: &str,
        id: &str,
        why: &str,
    ) -> std::result::Result<tools::ToolNote, coord::Error> {
        self.0.remove_note(by, id, why).map(tool_note)
    }
    fn search(&self, q: &str) -> Vec<tools::ToolNote> {
        // Agents read accepted notes only (§6).
        self.0
            .search_notes(q, true)
            .into_iter()
            .map(tool_note)
            .collect()
    }
}

// ---- the notes a wake carries (card #412, §6) --------------------------------

/// The repo root holding `cwd`: the nearest directory with a `.git` entry
/// (a worktree's `.git` is a file).
fn repo_root(cwd: &str) -> Option<std::path::PathBuf> {
    let mut p = std::path::Path::new(cwd);
    loop {
        if p.join(".git").exists() {
            return Some(p.to_path_buf());
        }
        p = p.parent()?;
    }
}

impl Services {
    /// The accepted notes for a wake to `agent` (the inbox's Notes hook):
    /// the first of `refs` (`task:<id>`) that is a card here, else the card
    /// the agent works now, gives the board, repo and paths; the agent's
    /// roster card gives its repo when the card has none, and its cwd a
    /// path; the device's name is the machine. Mine's matches and the
    /// board's are merged, most specific first, [`coord::NOTES_PER_WAKE`]
    /// at most. Runs under the inbox lock: it never calls the inbox, and
    /// the locks it takes (the boards list, the roster, the logs' coords,
    /// the name folds) are never held while the inbox is called.
    pub(crate) fn wake_notes(&self, agent: &str, refs: &[String]) -> inbox::WakeNotes {
        let me = self.roster.agents().into_iter().rfind(|a| a.key == agent);
        // The card, and the board log it is on (None: the private log).
        let mut card: Option<(coord::Task, Option<NoteLog>)> = None;
        for r in refs {
            let id = r.strip_prefix("task:").unwrap_or(r);
            if let Some(t) = self.coord.get_task(id) {
                card = Some((t, None));
                break;
            }
            if let Some((t, b)) = self
                .shown_boards()
                .into_iter()
                .find_map(|b| b.coord().get_task(id).map(|t| (t, b)))
            {
                card = Some((
                    t,
                    Some(NoteLog {
                        board_id: b.id.clone(),
                        scope: b.verifier.membership().tag.clone(),
                        coord: b.coord().clone(),
                    }),
                ));
                break;
            }
        }
        if card.is_none() {
            card = self
                .coord
                .list_tasks(&coord::TaskFilter {
                    holder: agent.to_string(),
                    open: true,
                    ..Default::default()
                })
                .into_iter()
                .find(|t| t.state == coord::WORKING)
                .map(|t| (t, None));
        }
        let mut cx = coord::NoteContext {
            machine: self.machine_name(),
            ..Default::default()
        };
        if let Some((t, b)) = &card {
            if let Some(b) = b {
                cx.board = b.scope.clone();
            }
            cx.repo_key = t.repo_key.clone();
            cx.paths = t
                .links
                .iter()
                .filter(|l| l.kind == coord::LINK_PATH)
                .map(|l| l.reference.clone())
                .collect();
        }
        if let Some(a) = &me {
            if cx.repo_key.is_empty() {
                cx.repo_key = a.repo_key.clone();
            }
            // The agent's cwd as a path, when it works in the card's repo
            // below the root.
            if !a.cwd.is_empty()
                && coord::same_repo_key(&a.repo_key, &cx.repo_key)
                && let Some(root) = repo_root(&a.cwd)
                && let Ok(rel) = std::path::Path::new(&a.cwd).strip_prefix(&root)
            {
                let rel = rel.to_string_lossy().replace('\\', "/");
                if !rel.is_empty() && !cx.paths.contains(&rel) {
                    cx.paths.push(rel);
                }
            }
        }
        let mut picked: Vec<(coord::NoteMatch, bool)> = self
            .coord
            .notes_for(&cx)
            .into_iter()
            .map(|m| (m, true))
            .collect();
        let board = card.as_ref().and_then(|(_, b)| b.clone());
        if let Some(b) = &board {
            picked.extend(b.coord.notes_for(&cx).into_iter().map(|m| (m, false)));
        }
        // Most specific first (stable: each log's newest first within a
        // score, Mine's before the board's).
        picked.sort_by_key(|(m, _)| std::cmp::Reverse(m.score));
        picked.truncate(coord::NOTES_PER_WAKE);
        let names = self.agent_names.all();
        let who = |key: &str, mine: bool| -> String {
            if key.is_empty() {
                return String::new();
            }
            let co = if mine {
                &self.coord
            } else {
                match &board {
                    Some(b) => &b.coord,
                    None => &self.coord,
                }
            };
            if co.is_person(key) {
                return if mine {
                    "the person".into()
                } else {
                    "a person in the space".into()
                };
            }
            match names.get(key).filter(|n| !n.trim().is_empty()) {
                Some(n) => format!("{n} ({key})"),
                None => key.to_string(),
            }
        };
        let notes: Vec<inbox::WakeNote> = picked
            .into_iter()
            .map(|(m, mine)| inbox::WakeNote {
                by: who(&m.note.by, mine),
                accepted_by: who(&m.note.accepted_by, mine),
                id: m.note.id,
                why: m.why,
                kind: m.note.kind,
                text: m.note.text,
            })
            .collect();
        inbox::WakeNotes {
            task: if notes.is_empty() {
                String::new()
            } else {
                card.map(|(t, _)| format!("task:{}", t.id))
                    .unwrap_or_default()
            },
            notes,
        }
    }

    /// This device's name as the person sees it (`machine:` tags).
    pub(crate) fn machine_name(&self) -> String {
        self.device_names
            .chosen()
            .remove(&self.device.id)
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| self.device.name.clone())
    }
}
