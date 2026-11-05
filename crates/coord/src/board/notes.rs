//! Notes (decision 45, docs/design/notes.md, card #401): short rules,
//! gotchas, decisions, pointers and preferences an agent should know before
//! it acts, tagged with where they apply and linked to their source.
//!
//! The private log holds Mine notes and each board's log holds that board's
//! notes; coord folds both the same way. A note an agent writes stays
//! proposed until a person accepts it (on the private log, the person; on a
//! board, any member); a note a person writes is accepted at once. An
//! agent's edit or removal of an accepted note is itself a proposal, kept
//! beside the live text until a person accepts it. Proposed notes reach no
//! agent: [`Fold::notes_for`] and agent searches see accepted notes only.

use gocompat::time::GoTime;
use serde::Serialize;
use store::{Event, kinds};

use super::error::{Result, invalid, not_allowed, not_found};
use super::fold::Fold;
use super::rows::{NoteAcceptedRow, NoteEditedRow, NoteProposedRow, NoteRemovedRow, row};
use super::types::{Change, RowPos};
use super::util::{caller, new_id};
use crate::names::trim_space;

/// The change kind a note row notes.
pub const NOTE_CHANGED: &str = "note";

/// A note's kinds.
pub const NOTE_KINDS: [&str; 5] = ["rule", "gotcha", "decision", "pointer", "preference"];

/// A note's states.
pub const NOTE_PROPOSED: &str = "proposed";
pub const NOTE_ACCEPTED: &str = "accepted";
pub const NOTE_REMOVED: &str = "removed";

/// The longest note text: notes are not documents.
pub const NOTE_MAX_TEXT: usize = 500;
/// The most tags on one note, and the longest tag.
pub const NOTE_MAX_TAGS: usize = 12;
pub const NOTE_MAX_TAG: usize = 200;
/// The most notes one wake carries (§6).
pub const NOTES_PER_WAKE: usize = 8;

/// The tag that matches every card.
pub const TAG_EVERYWHERE: &str = "everywhere";
/// Tag prefixes: `board:<tag>` (a Mine note about a shared board),
/// `repo:<repoKey>`, `path:<repoKey>:<path prefix>`, `machine:<device
/// name>`, `topic:<word>`.
pub const TAG_BOARD: &str = "board:";
pub const TAG_REPO: &str = "repo:";
pub const TAG_PATH: &str = "path:";
pub const TAG_MACHINE: &str = "machine:";
pub const TAG_TOPIC: &str = "topic:";

/// A pending agent change to an accepted note: an edit or a removal.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteChange {
    pub by: String,
    pub at: GoTime,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// The proposal is to remove the note.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub remove: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub why: String,
}

/// One step of a note's history.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteEvent {
    /// proposed, accepted, edited, edit_proposed, removed, removal_proposed.
    pub what: String,
    pub by: String,
    pub at: GoTime,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
}

/// A note as folded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub tags: Vec<String>,
    /// Where it came from: `task:<id>`, `comment:<id>`, `blocker:<taskId>/<b1>`
    /// or `proof:<taskId>`, or empty.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// The card the source is on, when it is on one.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub task_id: String,
    pub by: String,
    pub at: GoTime,
    /// proposed, accepted or removed.
    pub state: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub accepted_by: String,
    #[serde(skip_serializing_if = "GoTime::is_zero")]
    pub accepted_at: GoTime,
    /// When its text, kind or tags last changed (accepted).
    pub updated: GoTime,
    /// An agent's edit or removal waiting for a person's accept.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<NoteChange>,
    pub history: Vec<NoteEvent>,
}

impl Note {
    pub fn live(&self) -> bool {
        self.state == NOTE_ACCEPTED
    }
    /// `note:<id>`.
    pub fn reference(&self) -> String {
        format!("note:{}", self.id)
    }
}

/// What a person or agent writes as a new note.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewNote {
    pub kind: String,
    pub text: String,
    pub tags: Vec<String>,
    pub source: String,
    pub task_id: String,
    /// The source's comment id, when it is a comment (board closure).
    pub comment_id: String,
    /// The source's blocker id, when it is a blocker answer.
    pub blocker_id: String,
}

/// An edit: each `Some` field replaces the note's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteEdit {
    pub kind: Option<String>,
    pub text: Option<String>,
    pub tags: Option<Vec<String>>,
}

/// Where an agent is working, for [`Fold::notes_for`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteContext {
    /// The shared board's tag, when the card is on one (Mine notes tagged
    /// `board:<tag>` match it).
    pub board: String,
    pub repo_key: String,
    /// Paths the agent works in, relative to the repo root (its worktree's
    /// changed files, or the card's paths); `path:` tags match by prefix.
    pub paths: Vec<String>,
    /// The agent's machine (device name).
    pub machine: String,
}

/// One note picked for a wake, with why it matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteMatch {
    pub note: Note,
    /// The tag that matched.
    pub why: String,
    #[serde(skip)]
    pub score: u32,
}

/// Lower-cases and trims a tag; `""` when it is not a known shape.
pub fn clean_tag(t: &str) -> String {
    let t = trim_space(t);
    if t.eq_ignore_ascii_case(TAG_EVERYWHERE) {
        return TAG_EVERYWHERE.into();
    }
    for p in [TAG_BOARD, TAG_REPO, TAG_PATH, TAG_MACHINE, TAG_TOPIC] {
        if t.len() > p.len() && t[..p.len()].eq_ignore_ascii_case(p) {
            let rest = trim_space(&t[p.len()..]);
            if rest.is_empty() {
                return String::new();
            }
            // Repo keys and paths keep their case; the rest compare folded.
            let rest = if p == TAG_REPO || p == TAG_PATH {
                rest.to_string()
            } else {
                rest.to_lowercase()
            };
            return format!("{p}{rest}");
        }
    }
    String::new()
}

fn clean_tags(tags: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        if trim_space(t).is_empty() {
            continue;
        }
        if t.len() > NOTE_MAX_TAG {
            return Err(invalid(format!(
                "tag too long (at most {NOTE_MAX_TAG} characters)"
            )));
        }
        let c = clean_tag(t);
        if c.is_empty() {
            return Err(invalid(format!(
                "tag {t:?} isn't one of everywhere, board:<tag>, repo:<repoKey>, path:<repoKey>:<path>, machine:<name> or topic:<word>"
            )));
        }
        if !out.contains(&c) {
            out.push(c);
        }
    }
    if out.len() > NOTE_MAX_TAGS {
        return Err(invalid(format!("at most {NOTE_MAX_TAGS} tags")));
    }
    Ok(out)
}

fn clean_kind(k: &str) -> Result<String> {
    let k = trim_space(k).to_lowercase();
    if NOTE_KINDS.contains(&k.as_str()) {
        return Ok(k);
    }
    Err(invalid(format!(
        "kind {k:?} isn't one of {}",
        NOTE_KINDS.join(", ")
    )))
}

fn clean_text(t: &str) -> Result<String> {
    let t = trim_space(t).to_string();
    if t.is_empty() {
        return Err(invalid("a note needs text"));
    }
    if t.chars().count() > NOTE_MAX_TEXT {
        return Err(invalid(format!(
            "a note is a sentence or two, at most {NOTE_MAX_TEXT} characters; long material belongs in the repo's docs (link it with a pointer note)"
        )));
    }
    if super::util::looks_secret(&t) {
        return Err(not_allowed(
            "that looks like a credential or key; notes never carry secrets",
        ));
    }
    Ok(t)
}

/// How specifically `tag` matches `cx`: 0 when it doesn't.
fn tag_score(tag: &str, cx: &NoteContext) -> u32 {
    if tag == TAG_EVERYWHERE {
        return 1;
    }
    if let Some(r) = tag.strip_prefix(TAG_PATH) {
        // Repo keys hold colons themselves (remote:github.com/a/b).
        if cx.repo_key.is_empty() {
            return 0;
        }
        // A tag copied from a pre-#480 card may spell the key in capitals;
        // the path keeps its own case.
        let key = ledger::canonical_repo_key(&cx.repo_key);
        let Some(path) = r
            .get(..key.len())
            .filter(|h| h.eq_ignore_ascii_case(&key))
            .and(r.get(key.len()..))
            .and_then(|p| p.strip_prefix(':'))
        else {
            return 0;
        };
        let path = path.trim_start_matches("./").trim_start_matches('/');
        let hit = cx.paths.iter().any(|p| {
            let p = p.trim_start_matches("./").trim_start_matches('/');
            p == path
                || p.starts_with(path) && (path.ends_with('/') || p[path.len()..].starts_with('/'))
        });
        return if hit { 5 } else { 0 };
    }
    if let Some(r) = tag.strip_prefix(TAG_REPO) {
        return if !cx.repo_key.is_empty() && ledger::same_repo_key(r, &cx.repo_key) {
            4
        } else {
            0
        };
    }
    if let Some(b) = tag.strip_prefix(TAG_BOARD) {
        return if !cx.board.is_empty() && b == cx.board.to_lowercase() {
            3
        } else {
            0
        };
    }
    if let Some(m) = tag.strip_prefix(TAG_MACHINE) {
        return if !cx.machine.is_empty() && m == cx.machine.to_lowercase() {
            2
        } else {
            0
        };
    }
    0
}

impl Fold {
    fn note_changed(&mut self, id: &str) {
        self.note(Change {
            kind: NOTE_CHANGED.into(),
            resource: id.into(),
            ..Change::default()
        });
    }

    /// Whether the writer of an agent's note row is that agent's device.
    fn agent_row(&self, ev: &Event, by: &str) -> bool {
        self.is_agent_key(by) && self.may_write(by, &ev.device_id)
    }

    pub(crate) fn fold_note(&mut self, ev: &Event, at: &dyn Fn(GoTime) -> GoTime) {
        match ev.kind.as_str() {
            kinds::NOTE_PROPOSED => {
                let Some(p) = row::<NoteProposedRow>(&ev.payload) else {
                    return;
                };
                if p.note_id.is_empty() || self.notes.contains_key(&p.note_id) {
                    return;
                }
                let person = self.person_row(ev, &p.by);
                if !person && !self.agent_row(ev, &p.by) {
                    self.ignore(&ev.device_id);
                    return;
                }
                let t = at(p.at);
                let mut n = Note {
                    id: p.note_id.clone(),
                    kind: p.kind,
                    text: p.text.clone(),
                    tags: p.tags,
                    source: p.source,
                    task_id: p.task_id,
                    by: p.by.clone(),
                    at: t,
                    state: NOTE_PROPOSED.into(),
                    updated: t,
                    history: vec![NoteEvent {
                        what: "proposed".into(),
                        by: p.by.clone(),
                        at: t,
                        text: p.text,
                    }],
                    ..Note::default()
                };
                if person {
                    n.state = NOTE_ACCEPTED.into();
                    n.accepted_by = p.by;
                    n.accepted_at = t;
                }
                self.notes.insert(n.id.clone(), (n, RowPos::of(ev)));
                self.note_changed(&p.note_id);
            }
            kinds::NOTE_ACCEPTED => {
                let Some(p) = row::<NoteAcceptedRow>(&ev.payload) else {
                    return;
                };
                if !self.person_row(ev, &p.by) {
                    self.ignore(&ev.device_id);
                    return;
                }
                let Some((n, _)) = self.notes.get_mut(&p.note_id) else {
                    return;
                };
                let t = at(p.at);
                match n.pending.take() {
                    Some(c) if c.remove => {
                        n.state = NOTE_REMOVED.into();
                        n.history.push(NoteEvent {
                            what: "removed".into(),
                            by: p.by.clone(),
                            at: t,
                            text: c.why,
                        });
                    }
                    Some(c) => {
                        if !c.kind.is_empty() {
                            n.kind = c.kind;
                        }
                        if !c.text.is_empty() {
                            n.text = c.text.clone();
                        }
                        if let Some(tags) = c.tags {
                            n.tags = tags;
                        }
                        n.updated = t;
                        n.state = NOTE_ACCEPTED.into();
                        n.accepted_by = p.by.clone();
                        n.accepted_at = t;
                        n.history.push(NoteEvent {
                            what: "edited".into(),
                            by: p.by.clone(),
                            at: t,
                            text: c.text,
                        });
                    }
                    None => {
                        if n.state != NOTE_PROPOSED {
                            return;
                        }
                        n.state = NOTE_ACCEPTED.into();
                        n.accepted_by = p.by.clone();
                        n.accepted_at = t;
                        n.history.push(NoteEvent {
                            what: "accepted".into(),
                            by: p.by.clone(),
                            at: t,
                            text: String::new(),
                        });
                    }
                }
                self.note_changed(&p.note_id);
            }
            kinds::NOTE_EDITED => {
                let Some(p) = row::<NoteEditedRow>(&ev.payload) else {
                    return;
                };
                let person = self.person_row(ev, &p.by);
                if !person && !self.agent_row(ev, &p.by) {
                    self.ignore(&ev.device_id);
                    return;
                }
                let Some((n, _)) = self.notes.get_mut(&p.note_id) else {
                    return;
                };
                if n.state == NOTE_REMOVED {
                    return;
                }
                let t = at(p.at);
                // A person's edit, or an agent's on its own proposal, applies
                // at once; an agent's on a live note waits for an accept.
                if person || n.state == NOTE_PROPOSED && n.by == p.by {
                    if !p.kind.is_empty() {
                        n.kind = p.kind;
                    }
                    if !p.text.is_empty() {
                        n.text = p.text.clone();
                    }
                    if let Some(tags) = p.tags {
                        n.tags = tags;
                    }
                    n.updated = t;
                    if person {
                        n.pending = None;
                    }
                    n.history.push(NoteEvent {
                        what: "edited".into(),
                        by: p.by,
                        at: t,
                        text: p.text,
                    });
                } else {
                    n.pending = Some(NoteChange {
                        by: p.by.clone(),
                        at: t,
                        kind: p.kind,
                        text: p.text.clone(),
                        tags: p.tags,
                        remove: false,
                        why: p.why,
                    });
                    n.history.push(NoteEvent {
                        what: "edit_proposed".into(),
                        by: p.by,
                        at: t,
                        text: p.text,
                    });
                }
                self.note_changed(&p.note_id);
            }
            kinds::NOTE_REMOVED => {
                let Some(p) = row::<NoteRemovedRow>(&ev.payload) else {
                    return;
                };
                let person = self.person_row(ev, &p.by);
                if !person && !self.agent_row(ev, &p.by) {
                    self.ignore(&ev.device_id);
                    return;
                }
                let Some((n, _)) = self.notes.get_mut(&p.note_id) else {
                    return;
                };
                if n.state == NOTE_REMOVED {
                    return;
                }
                let t = at(p.at);
                if person || n.state == NOTE_PROPOSED && n.by == p.by {
                    n.state = NOTE_REMOVED.into();
                    n.pending = None;
                    n.history.push(NoteEvent {
                        what: "removed".into(),
                        by: p.by,
                        at: t,
                        text: p.why,
                    });
                } else {
                    n.pending = Some(NoteChange {
                        by: p.by.clone(),
                        at: t,
                        remove: true,
                        why: p.why.clone(),
                        ..NoteChange::default()
                    });
                    n.history.push(NoteEvent {
                        what: "removal_proposed".into(),
                        by: p.by,
                        at: t,
                        text: p.why,
                    });
                }
                self.note_changed(&p.note_id);
            }
            _ => {}
        }
    }

    fn writer(&self, by: &str) -> Result<(String, bool)> {
        let by = caller(by)?;
        if self.is_person(&by) {
            return Ok((by, true));
        }
        if self.is_agent_key(&by) {
            return Ok((by, false));
        }
        Err(not_allowed("only a person or an agent writes notes"))
    }

    pub(crate) fn propose_note(&mut self, by: &str, n: NewNote) -> Result<Note> {
        let (by, _) = self.writer(by)?;
        let kind = clean_kind(&n.kind)?;
        let text = clean_text(&n.text)?;
        let tags = clean_tags(&n.tags)?;
        if tags.is_empty() {
            return Err(invalid(
                "give at least one tag: everywhere, repo:<repoKey>, path:<repoKey>:<path>, machine:<name>, board:<tag> or topic:<word>",
            ));
        }
        if !n.task_id.is_empty() && !self.tasks.contains_key(&n.task_id) {
            return Err(not_found(format!("task {}", n.task_id)));
        }
        let at = self.stamp();
        let id = new_id(&at);
        self.write(
            &by,
            kinds::NOTE_PROPOSED,
            &NoteProposedRow {
                note_id: id.clone(),
                kind,
                text,
                tags,
                source: trim_space(&n.source).into(),
                task_id: n.task_id,
                comment_id: n.comment_id,
                blocker_id: n.blocker_id,
                by: by.clone(),
                at,
            },
        )?;
        self.notes
            .get(&id)
            .map(|(n, _)| n.clone())
            .ok_or_else(|| invalid("the note row was refused"))
    }

    fn live_note(&self, id: &str) -> Result<Note> {
        match self.notes.get(id) {
            Some((n, _)) if n.state != NOTE_REMOVED => Ok(n.clone()),
            Some(_) => Err(invalid(format!("note {id} was removed"))),
            None => Err(not_found(format!("note {id}"))),
        }
    }

    pub(crate) fn accept_note(&mut self, by: &str, id: &str) -> Result<Note> {
        let (by, person) = self.writer(by)?;
        if !person {
            return Err(not_allowed(
                "a person accepts a note; an agent proposes it (notes_propose)",
            ));
        }
        let n = self.live_note(id)?;
        if n.state == NOTE_ACCEPTED && n.pending.is_none() {
            return Ok(n);
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::NOTE_ACCEPTED,
            &NoteAcceptedRow {
                note_id: id.into(),
                by: by.clone(),
                at,
            },
        )?;
        self.live_note(id).or_else(|_| Ok(self.notes[id].0.clone()))
    }

    pub(crate) fn edit_note(&mut self, by: &str, id: &str, e: NoteEdit, why: &str) -> Result<Note> {
        let (by, _) = self.writer(by)?;
        self.live_note(id)?;
        let kind = match &e.kind {
            Some(k) => clean_kind(k)?,
            None => String::new(),
        };
        let text = match &e.text {
            Some(t) => clean_text(t)?,
            None => String::new(),
        };
        let tags = match &e.tags {
            Some(t) => {
                let t = clean_tags(t)?;
                if t.is_empty() {
                    return Err(invalid("a note keeps at least one tag"));
                }
                Some(t)
            }
            None => None,
        };
        if kind.is_empty() && text.is_empty() && tags.is_none() {
            return Err(invalid("nothing to change: give kind, text or tags"));
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::NOTE_EDITED,
            &NoteEditedRow {
                note_id: id.into(),
                kind,
                text,
                tags,
                why: trim_space(why).into(),
                by: by.clone(),
                at,
            },
        )?;
        Ok(self.notes[id].0.clone())
    }

    pub(crate) fn remove_note(&mut self, by: &str, id: &str, why: &str) -> Result<Note> {
        let (by, _) = self.writer(by)?;
        self.live_note(id)?;
        let at = self.stamp();
        self.write(
            &by,
            kinds::NOTE_REMOVED,
            &NoteRemovedRow {
                note_id: id.into(),
                why: trim_space(why).into(),
                by: by.clone(),
                at,
            },
        )?;
        Ok(self.notes[id].0.clone())
    }

    /// Every note, newest first; removed ones only when asked.
    pub(crate) fn list_notes(&self, removed: bool) -> Vec<Note> {
        let mut v: Vec<(RowPos, Note)> = self
            .notes
            .values()
            .filter(|(n, _)| removed || n.state != NOTE_REMOVED)
            .map(|(n, p)| (p.clone(), n.clone()))
            .collect();
        v.sort_by(|a, b| b.0.cmp(&a.0));
        v.into_iter().map(|(_, n)| n).collect()
    }

    /// Notes whose text, kind or tags hold every word of `q`, newest
    /// first. `live_only`: accepted notes only (what an agent may read).
    pub(crate) fn search_notes(&self, q: &str, live_only: bool) -> Vec<Note> {
        let words: Vec<String> = q.split_whitespace().map(|w| w.to_lowercase()).collect();
        self.list_notes(false)
            .into_iter()
            .filter(|n| !live_only || n.live())
            .filter(|n| {
                let hay = format!("{} {} {}", n.kind, n.text, n.tags.join(" ")).to_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .collect()
    }

    /// The accepted notes that match where an agent works, most specific
    /// first, at most [`NOTES_PER_WAKE`]. `topic:` tags never match a
    /// wake; those notes are for search.
    pub(crate) fn notes_for(&self, cx: &NoteContext) -> Vec<NoteMatch> {
        let mut out: Vec<NoteMatch> = Vec::new();
        for n in self.list_notes(false) {
            if !n.live() {
                continue;
            }
            let mut best = (0u32, String::new());
            for t in &n.tags {
                let s = tag_score(t, cx);
                if s > best.0 {
                    best = (s, t.clone());
                }
            }
            if best.0 == 0 {
                continue;
            }
            out.push(NoteMatch {
                note: n,
                why: best.1,
                score: best.0,
            });
        }
        // Stable: newest first within one score.
        out.sort_by_key(|m| std::cmp::Reverse(m.score));
        out.truncate(NOTES_PER_WAKE);
        out
    }
}

#[cfg(test)]
mod repo_case_tests {
    use super::*;

    /// Card #480: a tag copied from a pre-#480 card spells the key in
    /// capitals, while the card's key now reads back lowercase.
    #[test]
    fn mixed_case_repo_and_path_tags_still_match() {
        let cx = NoteContext {
            repo_key: "remote:github.com/stephenshorton/irori".into(),
            paths: vec!["src/x.rs".into()],
            ..NoteContext::default()
        };
        assert_eq!(
            tag_score("repo:remote:github.com/StephenSHorton/irori", &cx),
            4
        );
        assert_eq!(
            tag_score("path:remote:github.com/StephenSHorton/irori:src/x.rs", &cx),
            5
        );
        assert_eq!(
            tag_score("path:remote:github.com/StephenSHorton/irori:src/Y.rs", &cx),
            0
        );
        assert_eq!(tag_score("repo:remote:github.com/a/suzuri", &cx), 0);
    }
}
