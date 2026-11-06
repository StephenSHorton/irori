//! Notes are Mine only (decision 53, card #616, docs/design/notes.md §3):
//! every note lives in the person's private log and only their own agents
//! read it. Board logs still accept `note.*` rows, since devices on
//! 0.11.0-0.12.3 can write them and refusing one would stall that writer's
//! stream, but Irori never reads them. Tools and the window reach the log
//! through these.

use store::kinds::AGENT_ACTOR_PREFIX;

use super::Services;

/// A note id without its `note:` prefix.
fn bare(id: &str) -> &str {
    let id = id.trim();
    id.strip_prefix("note:").unwrap_or(id).trim()
}

impl Services {
    // The writes, shared by the window (through `Core`) and the agent tools
    // (through `ToolsView`, card #412).

    pub(crate) fn propose_note(
        &self,
        by: &str,
        n: coord::NewNote,
    ) -> std::result::Result<coord::Note, coord::Error> {
        self.coord.propose_note(by, n)
    }

    pub(crate) fn accept_note(
        &self,
        by: &str,
        id: &str,
    ) -> std::result::Result<coord::Note, coord::Error> {
        self.coord.accept_note(by, bare(id))
    }

    pub(crate) fn edit_note(
        &self,
        by: &str,
        id: &str,
        e: coord::NoteEdit,
        why: &str,
    ) -> std::result::Result<coord::Note, coord::Error> {
        self.coord.edit_note(by, bare(id), e, why)
    }

    pub(crate) fn remove_note(
        &self,
        by: &str,
        id: &str,
        why: &str,
    ) -> std::result::Result<coord::Note, coord::Error> {
        self.coord.remove_note(by, bare(id), why)
    }

    pub(crate) fn search_notes(&self, q: &str, live_only: bool) -> Vec<coord::Note> {
        self.coord.search_notes(q, live_only)
    }

    /// The names a `machine:` tag can match, lower case: each of the
    /// person's devices by the name a wake on it reads
    /// ([`Services::machine_name`] there: the name the person chose, else
    /// its own).
    pub(crate) fn machine_names(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .device_labels()
            .into_values()
            .map(|n| n.trim().to_lowercase())
            .filter(|n| !n.is_empty())
            .collect();
        out.push(self.machine_name().trim().to_lowercase());
        out.retain(|n| !n.is_empty());
        out.sort();
        out.dedup();
        out
    }
}

impl super::Core {
    /// Writes a note as `by`, the person at this device or one of their
    /// agents. It stays in the private log whatever its tags name.
    pub fn propose_note(
        &self,
        by: &str,
        n: coord::NewNote,
    ) -> std::result::Result<coord::Note, coord::Error> {
        self.s.propose_note(by, n)
    }

    /// A person accepts note `id` (or an agent's pending change to it).
    pub fn accept_note(&self, by: &str, id: &str) -> std::result::Result<coord::Note, coord::Error> {
        self.s.accept_note(by, id)
    }

    pub fn edit_note(
        &self,
        by: &str,
        id: &str,
        e: coord::NoteEdit,
        why: &str,
    ) -> std::result::Result<coord::Note, coord::Error> {
        self.s.edit_note(by, id, e, why)
    }

    pub fn remove_note(
        &self,
        by: &str,
        id: &str,
        why: &str,
    ) -> std::result::Result<coord::Note, coord::Error> {
        self.s.remove_note(by, id, why)
    }

    /// The person's notes, newest first; removed ones too when asked (the
    /// history view).
    pub fn notes(&self, removed: bool) -> Vec<coord::Note> {
        self.s.coord.notes(removed)
    }

    /// Notes holding every word of `q`; `live_only`: accepted only, what
    /// agents may read.
    pub fn search_notes(&self, q: &str, live_only: bool) -> Vec<coord::Note> {
        self.s.search_notes(q, live_only)
    }

    /// The names a `machine:` tag can match (lower case).
    pub fn machine_names(&self) -> Vec<String> {
        self.s.machine_names()
    }
}

// ---- the agent tools' seam (card #412) ---------------------------------------

impl tools::NotesBackend for super::ToolsView {
    fn propose(&self, by: &str, n: coord::NewNote) -> std::result::Result<coord::Note, coord::Error> {
        self.0.propose_note(by, n)
    }
    fn edit(
        &self,
        by: &str,
        id: &str,
        e: coord::NoteEdit,
        why: &str,
    ) -> std::result::Result<coord::Note, coord::Error> {
        self.0.edit_note(by, id, e, why)
    }
    fn remove(&self, by: &str, id: &str, why: &str) -> std::result::Result<coord::Note, coord::Error> {
        self.0.remove_note(by, id, why)
    }
    fn search(&self, q: &str) -> Vec<coord::Note> {
        // Agents read accepted notes only (§6).
        self.0.search_notes(q, true)
    }
    fn machines(&self) -> Vec<String> {
        self.0.machine_names()
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
    /// path; the device's name is the machine. Only Mine is read: a card
    /// on a shared space gives its tag, for `board:` tags on Mine notes.
    /// [`coord::NOTES_PER_WAKE`] at most, most specific first.
    ///
    /// Runs under the inbox lock, so it never calls the inbox and never
    /// waits on a board coord (card #561): a board coord's write takes the
    /// inbox lock under its own (`hop_of`). Boards are only tried. A busy
    /// board reads as no card here: a note left out goes in a later wake.
    /// The boards list it may wait on: nothing waits on a coord while
    /// holding that ([`Services::route`]).
    pub(crate) fn wake_notes(&self, agent: &str, refs: &[String]) -> inbox::WakeNotes {
        let me = self.roster.agents().into_iter().rfind(|a| a.key == agent);
        // The card, and the tag of the space it is on ("" for the private
        // log).
        let mut card: Option<(coord::Task, String)> = None;
        // A comment: ref is for the person's words below, not for notes.
        for r in refs.iter().filter(|r| r.starts_with("task:")) {
            let id = r.strip_prefix("task:").unwrap_or(r);
            if let Some(t) = self.coord.get_task(id) {
                card = Some((t, String::new()));
                break;
            }
            if let Some((t, b)) = self
                .visible_boards()
                .into_iter()
                .find_map(|b| b.coord().try_get_task(id).flatten().map(|t| (t, b)))
            {
                card = Some((t, b.verifier.membership().tag.clone()));
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
                .map(|t| (t, String::new()));
        }
        let mut cx = coord::NoteContext {
            machine: self.machine_name(),
            ..Default::default()
        };
        if let Some((t, board)) = &card {
            cx.board = board.clone();
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
        // Most specific first, newest first within a score.
        let mut picked = self.coord.notes_for(&cx);
        picked.truncate(coord::NOTES_PER_WAKE);
        let names = self.agent_names.all();
        let who = |key: &str| -> String {
            if key.is_empty() {
                return String::new();
            }
            if self.coord.is_person(key) {
                return "the person".into();
            }
            match names.get(key).filter(|n| !n.trim().is_empty()) {
                Some(n) => format!("{n} ({key})"),
                None => key.to_string(),
            }
        };
        let notes: Vec<inbox::WakeNote> = picked
            .into_iter()
            .map(|m| inbox::WakeNote {
                by: who(&m.note.by),
                accepted_by: who(&m.note.accepted_by),
                id: m.note.id,
                why: m.why,
                kind: m.note.kind,
                text: m.note.text,
            })
            .collect();
        let (said_for, said) = self.wake_said(agent, refs);
        inbox::WakeNotes {
            task: if notes.is_empty() {
                String::new()
            } else {
                card.map(|(t, _)| format!("task:{}", t.id))
                    .unwrap_or_default()
            },
            notes,
            said_for,
            said,
        }
    }

    /// What the person said lately around `agent`'s card, for a wake (card
    /// #555), and that card's short ref. Its card is the one the wake is
    /// about when that is the agent's own, else the card it works now: a
    /// wake about a card it only watches says nothing about that card's
    /// neighbours. Runs under the inbox lock, so it never waits on a board:
    /// a board coord's write takes the inbox lock under its own (`hop_of`).
    /// A busy board is skipped, and its words go in a later wake.
    fn wake_said(&self, agent: &str, refs: &[String]) -> (String, Vec<inbox::WakeSaid>) {
        let boards = self.shown_boards();
        let read = |working: bool| {
            self.coord.said_for(agent, refs, working).or_else(|| {
                boards.iter().find_map(|b| {
                    // Board logs hold agent:<cardId>, not the session key.
                    let id = b
                        .maps
                        .try_lock()
                        .ok()?
                        .keys
                        .get(agent.strip_prefix("api:").unwrap_or(agent))?
                        .clone();
                    let key = format!("{AGENT_ACTOR_PREFIX}{id}");
                    b.coord().try_said_for(&key, refs, working).flatten()
                })
            })
        };
        let Some((card, said)) = read(false).or_else(|| read(true)) else {
            return Default::default();
        };
        let said = said
            .into_iter()
            .map(|s| inbox::WakeSaid {
                id: s.comment_id,
                card: s.card,
                text: s.text,
            })
            .collect();
        (card, said)
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
