//! notes_propose / notes_search (card #412, docs/design/notes.md §11): the
//! agent side of notes. An agent writes a note, or edits or removes one,
//! and it applies at once: the person sees, edits and removes notes on the
//! Notes page, with no approval step (card #616). Notes are the person's
//! own (decision 53): there are no shared notes.

use crate::args::Ty;
use crate::format::{one_line, trim};
use crate::{Caller, Inner, decode, err_text, map_err, untrusted};

/// The Core's notes API (Go has none; Rust-only, card #412). Every write
/// goes through the Core, into the person's private log.
pub trait NotesBackend: Send + Sync {
    fn propose(&self, by: &str, n: coord::NewNote) -> Result<coord::Note, coord::Error>;
    fn edit(
        &self,
        by: &str,
        id: &str,
        e: coord::NoteEdit,
        why: &str,
    ) -> Result<coord::Note, coord::Error>;
    fn remove(&self, by: &str, id: &str, why: &str) -> Result<coord::Note, coord::Error>;
    /// Live notes holding every word of `q`, newest first.
    fn search(&self, q: &str) -> Vec<coord::Note>;
    /// The names a `machine:` tag can match, lower case: each of the
    /// person's devices as a wake on it names it. Empty: not checked.
    fn machines(&self) -> Vec<String>;
}

const PROPOSE_ARGS: &[(&str, Ty)] = &[
    // No longer in the spec (card #616), still read so an agent with an
    // older spec gets a plain answer, not an unknown argument.
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

/// Refuses tags that can never match a wake (card #616): a `*` (path tags
/// are literal prefixes, nothing expands it) or a machine no device is
/// called. Shapes and lengths are the coord's to check.
fn check_tags(nb: &dyn NotesBackend, tags: &[String]) -> Result<(), String> {
    let mut machines: Option<Vec<String>> = None;
    for t in tags {
        let t = trim(t);
        if t.contains('*') {
            return Err(err_text(&format!(
                "tag {} has a *, and tags never expand one: path:<repoKey>:<path> matches that path and everything under it, so give the folder itself (path:<repoKey>:src/ui), or several path: tags.",
                crate::format::q(t)
            )));
        }
        let c = coord::clean_tag(t);
        let Some(name) = c.strip_prefix(coord::TAG_MACHINE) else {
            continue;
        };
        let known = machines.get_or_insert_with(|| nb.machines());
        if !known.is_empty() && !known.iter().any(|k| k == name) {
            return Err(err_text(&format!(
                "no device is called {}, so machine:{name} would never match a wake. Devices: {}. Give one of those, or leave machine: out.",
                crate::format::q(name),
                known
                    .iter()
                    .map(|k| format!("machine:{k}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
    }
    Ok(())
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
    let scope = trim(a.str("scope"));
    if !scope.is_empty() && !scope.eq_ignore_ascii_case("mine") {
        return err_text(
            "notes are your person's own: only their agents read them, and there are no shared notes, so leave scope out. For a note about a shared space, tag it board:<tag>.",
        );
    }
    if let Err(e) = check_tags(nb, &a.strs("tags")) {
        return e;
    }
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
        return match nb.propose(by, n) {
            Ok(n) => format!(
                "Wrote {} ({}): it is live, and agents get it on wake when its tags match. The person sees it on the Notes page.",
                n.reference(),
                n.kind
            ),
            Err(e) => map_err(e),
        };
    }
    if !trim(a.str("source")).is_empty() {
        return err_text(
            "a change to an existing note takes note, the fields that change and why; source is only for a new note.",
        );
    }
    if why.is_empty() {
        return err_text(
            "say why the note should change (why): it stays in the note's history for the person.",
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
            Ok(n) => format!("Removed {}.", n.reference()),
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
        Ok(n) => format!("Changed {}.", n.reference()),
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
        return format!("No note holds all of {}.", crate::format::q(q));
    }
    let names = s.roster();
    let mut out = format!(
        "{} note{} match {}. {}",
        hits.len(),
        if hits.len() == 1 { "" } else { "s" },
        crate::format::q(q),
        untrusted()
    );
    for n in hits.iter().take(SEARCH_MAX) {
        out.push_str(&format!(
            "\n- {} [{}] {}\n  tags: {}; by {}",
            n.reference(),
            n.kind,
            one_line(&n.text, 600),
            one_line(&n.tags.join(", "), 300),
            names.who(&n.by),
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
