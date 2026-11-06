//! The Notes page's commands (decision 45, card #401, docs/design/notes.md
//! §11): list, New note, accept, edit and remove, each through the Core's
//! notes API. Every note is the person's own, in the private log
//! (decision 52, card #616).

use gocompat::time::GoTime;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::Window;

/// A pending agent change to a live note, waiting for a person's accept.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NoteChangeView {
    pub by: String,
    #[specta(type = crate::ts::Time)]
    pub at: GoTime,
    /// The proposed kind, text or tags; empty or absent when unchanged.
    pub kind: String,
    pub text: String,
    pub tags: Option<Vec<String>>,
    /// The proposal is to remove the note.
    pub remove: bool,
    pub why: String,
}

/// One step of a note's history.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NoteEventView {
    /// proposed, accepted, edited, edit_proposed, removal_proposed, removed.
    pub what: String,
    pub by: String,
    #[specta(type = crate::ts::Time)]
    pub at: GoTime,
    pub text: String,
}

/// A note as the Notes page shows it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NoteView {
    /// The bare id; refs are `note:<id>`.
    pub id: String,
    /// rule, gotcha, decision, pointer or preference.
    pub kind: String,
    pub text: String,
    pub tags: Vec<String>,
    /// `task:<id>`, `comment:<id>`, `blocker:<taskId>/<b1>`, `proof:<taskId>`, or "".
    pub source: String,
    pub task_id: String,
    /// The author's key: the person (`user:…`) or an agent.
    pub by: String,
    /// Whether the author is a person (a person's note is live at once).
    pub by_person: bool,
    #[specta(type = crate::ts::Time)]
    pub at: GoTime,
    /// proposed, accepted or removed.
    pub state: String,
    pub accepted_by: String,
    #[specta(type = crate::ts::Time)]
    pub accepted_at: GoTime,
    #[specta(type = crate::ts::Time)]
    pub updated: GoTime,
    pub pending: Option<NoteChangeView>,
    pub history: Vec<NoteEventView>,
    /// Still true? (v1): why the note may be out of date, else "". A
    /// `path:` tag whose path is gone from every checkout of its repo on
    /// this device, or a source card that is gone.
    pub stale: String,
}

/// What New note writes.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct NoteInput {
    pub kind: String,
    pub text: String,
    pub tags: Vec<String>,
}

/// An edit: each given field replaces the note's.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct NoteEditInput {
    pub kind: Option<String>,
    pub text: Option<String>,
    pub tags: Option<Vec<String>>,
}

fn view(n: coord::Note, person: impl Fn(&str) -> bool) -> NoteView {
    NoteView {
        by_person: person(&n.by),
        id: n.id,
        kind: n.kind,
        text: n.text,
        tags: n.tags,
        source: n.source,
        task_id: n.task_id,
        by: n.by,
        at: n.at,
        state: n.state,
        accepted_by: n.accepted_by,
        accepted_at: n.accepted_at,
        updated: n.updated,
        pending: n.pending.map(|c| NoteChangeView {
            by: c.by,
            at: c.at,
            kind: c.kind,
            text: c.text,
            tags: c.tags,
            remove: c.remove,
            why: c.why,
        }),
        history: n
            .history
            .into_iter()
            .map(|h| NoteEventView {
                what: h.what,
                by: h.by,
                at: h.at,
                text: h.text,
            })
            .collect(),
        stale: String::new(),
    }
}

/// A note in New from agents: the frontend's needsLook (lib/notes.ts).
fn needs_look(n: &coord::Note) -> bool {
    n.state == coord::NOTE_PROPOSED || (n.state == coord::NOTE_ACCEPTED && n.pending.is_some())
}

impl Window {
    /// Still true? for one note (v1, docs/design/notes.md §11).
    fn stale(&self, n: &coord::Note) -> String {
        if !n.task_id.is_empty() && self.core.coord().get_task(&n.task_id).is_none() {
            return "its source card is gone".into();
        }
        for t in &n.tags {
            let Some(rest) = t.strip_prefix(coord::TAG_PATH) else {
                continue;
            };
            // path:<repoKey>:<path>; repo keys hold colons, so try each split.
            for (i, _) in rest.match_indices(':') {
                let (repo, path) = (&rest[..i], &rest[i + 1..]);
                let roots = self.core.capture().repo_roots(repo);
                if roots.is_empty() || path.is_empty() {
                    continue;
                }
                let path = path.trim_start_matches('/');
                if !roots
                    .iter()
                    .any(|r| std::path::Path::new(r).join(path).exists())
                {
                    return format!("{path} is gone from {}", coord::repo_name(repo));
                }
                break;
            }
        }
        String::new()
    }

    fn note_view(&self, n: coord::Note) -> NoteView {
        let stale = if n.state == coord::NOTE_REMOVED {
            String::new()
        } else {
            self.stale(&n)
        };
        let core = self.core.clone();
        let mut v = view(n, move |k| core.coord().is_person(k));
        v.stale = stale;
        v
    }

    /// The person's notes, newest first; removed ones too when asked (the
    /// history view).
    pub fn notes_list(&self, removed: bool) -> Result<Vec<NoteView>, String> {
        Ok(self
            .core
            .notes(removed)
            .into_iter()
            .map(|n| self.note_view(n))
            .collect())
    }

    /// The sidebar's Notes count (card #468): notes waiting on the
    /// person's look, an agent's proposal or an agent's pending edit or
    /// removal of a live note. It reads the fold only: no Still true?
    /// checks on disk and no views, which notes_list did for every note on
    /// every board change.
    pub fn notes_waiting(&self) -> Result<usize, String> {
        Ok(self.core.notes(false).iter().filter(|n| needs_look(n)).count())
    }

    /// New note: written by the person, so live at once.
    pub fn note_new(&self, n: NoteInput) -> Result<NoteView, String> {
        let me = self.user_key();
        let s = self
            .core
            .propose_note(
                &me,
                coord::NewNote {
                    kind: n.kind,
                    text: n.text,
                    tags: n.tags,
                    ..Default::default()
                },
            )
            .map_err(|e| e.to_string())?;
        Ok(self.note_view(s))
    }

    /// Keep or Accept: the person accepts an agent's note, or its pending
    /// edit or removal.
    pub fn note_accept(&self, id: &str) -> Result<NoteView, String> {
        let s = self
            .core
            .accept_note(&self.user_key(), crate::trim(id))
            .map_err(|e| e.to_string())?;
        Ok(self.note_view(s))
    }

    /// The person's edit, live at once.
    pub fn note_edit(&self, id: &str, e: NoteEditInput) -> Result<NoteView, String> {
        let s = self
            .core
            .edit_note(
                &self.user_key(),
                crate::trim(id),
                coord::NoteEdit {
                    kind: e.kind,
                    text: e.text,
                    tags: e.tags,
                },
                "",
            )
            .map_err(|e| e.to_string())?;
        Ok(self.note_view(s))
    }

    /// The person removes a note (or turns down an agent's proposal).
    pub fn note_remove(&self, id: &str, why: &str) -> Result<NoteView, String> {
        let s = self
            .core
            .remove_note(&self.user_key(), crate::trim(id), crate::trim(why))
            .map_err(|e| e.to_string())?;
        Ok(self.note_view(s))
    }
}
