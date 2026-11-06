//! notes_propose / notes_search (card #412, docs/design/notes.md §11): the
//! agent side of notes. An agent proposes a note, or an edit or removal of
//! one; nothing it writes reaches another agent until a person accepts it.
//! Search reads accepted notes only, in the logs this device reads (Mine,
//! plus each shared board it is on).

use crate::args::Ty;
use crate::format::{one_line, trim};
use crate::{Caller, Inner, decode, err_text, map_err, untrusted};

/// A note as the tools see it: the coord note plus the log it lives in.
#[derive(Debug, Clone)]
pub struct ToolNote {
    /// `"mine"`, or the board's tag.
    pub scope: String,
    pub note: coord::Note,
}

/// The Core's notes API (Go has none; Rust-only, card #412). Every write
/// goes through the Core so a note lands in the log its scope names, and a
/// board's write gate applies.
pub trait NotesBackend: Send + Sync {
    fn propose(&self, by: &str, scope: &str, n: coord::NewNote) -> Result<ToolNote, coord::Error>;
    fn edit(
        &self,
        by: &str,
        id: &str,
        e: coord::NoteEdit,
        why: &str,
    ) -> Result<ToolNote, coord::Error>;
    fn remove(&self, by: &str, id: &str, why: &str) -> Result<ToolNote, coord::Error>;
    /// Accepted notes holding every word of `q`, newest first.
    fn search(&self, q: &str) -> Vec<ToolNote>;
}

const PROPOSE_ARGS: &[(&str, Ty)] = &[
    ("scope", Ty::Str),
    ("kind", Ty::Str),
    ("text", Ty::Str),
    ("tags", Ty::Strs),
    ("source", Ty::Str),
    ("note", Ty::Str),
    ("remove", Ty::Bool),
    ("why", Ty::Str),
];

/// How many notes one search result lists.
const SEARCH_MAX: usize = 20;

fn backend(s: &Inner) -> Result<&dyn NotesBackend, String> {
    s.b.notes()
        .ok_or_else(|| err_text("notes aren't available on this Irori."))
}

pub(crate) fn notes_propose(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, PROPOSE_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let nb = match backend(s) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let by = c.sender.key.as_str();
    let note = trim(a.str("note"));
    let why = trim(a.str("why"));
    if note.is_empty() {
        if a.bool("remove") || !why.is_empty() {
            return err_text(
                "remove and why are for a change to an existing note: give note (its note:<id>) too.",
            );
        }
        let (source, task_id, comment_id) = match source_of(s, trim(a.str("source"))) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let n = coord::NewNote {
            kind: trim(a.str("kind")).to_string(),
            text: trim(a.str("text")).to_string(),
            tags: a.strs("tags"),
            source,
            task_id,
            comment_id,
            blocker_id: String::new(),
        };
        return match nb.propose(by, trim(a.str("scope")), n) {
            Ok(t) => written(&t, "Proposed"),
            Err(e) => map_err(e),
        };
    }
    if !trim(a.str("scope")).is_empty() || !trim(a.str("source")).is_empty() {
        return err_text(
            "a change to an existing note takes note, the fields that change and why; scope and source are only for a new note.",
        );
    }
    if why.is_empty() {
        return err_text(
            "say why the note should change (why): the person reads it before accepting.",
        );
    }
    if a.bool("remove") {
        if !trim(a.str("kind")).is_empty()
            || !trim(a.str("text")).is_empty()
            || !a.strs("tags").is_empty()
        {
            return err_text("a removal takes note, remove = true and why only.");
        }
        return match nb.remove(by, note, why) {
            Ok(t) => changed(&t, true),
            Err(e) => map_err(e),
        };
    }
    let opt = |k: &str| {
        let v = trim(a.str(k));
        (!v.is_empty()).then(|| v.to_string())
    };
    let tags = a.strs("tags");
    let e = coord::NoteEdit {
        kind: opt("kind"),
        text: opt("text"),
        tags: (!tags.is_empty()).then_some(tags),
    };
    if e == coord::NoteEdit::default() {
        return err_text("give what changes: kind, text or tags (or remove = true).");
    }
    match nb.edit(by, note, e, why) {
        Ok(t) => changed(&t, false),
        Err(e) => map_err(e),
    }
}

/// A new note's source: a card (`#12`, `task:<id>`) or `comment:<id>`.
fn source_of(s: &Inner, src: &str) -> Result<(String, String, String), String> {
    if src.is_empty() {
        return Ok(Default::default());
    }
    if let Some(id) = src.strip_prefix("comment:") {
        let id = trim(id);
        if id.is_empty() {
            return Err(err_text("source comment:<id> needs the comment's id."));
        }
        return Ok((format!("comment:{id}"), String::new(), id.to_string()));
    }
    let id = s.task_id(src)?;
    let id = id.strip_prefix("task:").unwrap_or(&id).to_string();
    Ok((format!("task:{id}"), id, String::new()))
}

fn where_(t: &ToolNote) -> String {
    if t.scope == "mine" {
        "Mine".into()
    } else {
        format!("space {}", one_line(&t.scope, 80))
    }
}

fn written(t: &ToolNote, verb: &str) -> String {
    let n = &t.note;
    if n.live() {
        return format!(
            "Wrote {} ({}) in {}: it is live, and agents get it on wake when its tags match.",
            n.reference(),
            n.kind,
            where_(t)
        );
    }
    let who = if t.scope == "mine" {
        "the person"
    } else {
        "a person in the space"
    };
    format!(
        "{verb} {} ({}) in {}. It reaches no agent until {who} accepts it on the Notes page. To change it before then, call notes_propose with note = {} and why.",
        n.reference(),
        n.kind,
        where_(t),
        n.reference()
    )
}

fn changed(t: &ToolNote, remove: bool) -> String {
    let n = &t.note;
    let r = n.reference();
    if n.state == coord::NOTE_REMOVED {
        return format!("Removed {r} from {}.", where_(t));
    }
    if n.pending.is_some() {
        let what = if remove { "removal" } else { "edit" };
        return format!(
            "Proposed the {what} of {r} in {}. The live note stays as it is until a person accepts the change.",
            where_(t)
        );
    }
    if !n.live() {
        return format!(
            "Changed your proposal {r} in {}; it still waits for a person's accept.",
            where_(t)
        );
    }
    format!("Changed {r} in {}.", where_(t))
}

pub(crate) fn notes_search(s: &Inner, _c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, &[("q", Ty::Str)]) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let nb = match backend(s) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let q = trim(a.str("q"));
    if q.is_empty() {
        return err_text(
            "give q: words a note's text, kind or tags hold (all of them must match).",
        );
    }
    let hits = nb.search(q);
    if hits.is_empty() {
        return format!(
            "No accepted note holds all of {}. Proposed notes aren't searched: they reach no agent until a person accepts them.",
            crate::format::q(q)
        );
    }
    let names = s.roster();
    let mut out = format!(
        "{} accepted note{} match {}. {}",
        hits.len(),
        if hits.len() == 1 { "" } else { "s" },
        crate::format::q(q),
        untrusted()
    );
    for t in hits.iter().take(SEARCH_MAX) {
        let n = &t.note;
        out.push_str(&format!(
            "\n- {} [{}] ({}) {}\n  tags: {}; by {}, accepted by {}",
            n.reference(),
            n.kind,
            where_(t),
            one_line(&n.text, 600),
            one_line(&n.tags.join(", "), 300),
            names.who(&n.by),
            names.who(&n.accepted_by)
        ));
    }
    if hits.len() > SEARCH_MAX {
        out.push_str(&format!(
            "\n({} more: add words to q to narrow it.)",
            hits.len() - SEARCH_MAX
        ));
    }
    out
}
