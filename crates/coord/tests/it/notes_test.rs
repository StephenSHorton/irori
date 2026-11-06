//! Notes on the person's own board (card #401, docs/design/notes.md): an
//! agent's note waits for the person's OK; the person's is live at once;
//! an agent's edit or removal of a live note is a proposal; only accepted
//! notes reach a wake.

use crate::common;

use common::*;
use coord::*;

fn nn(kind: &str, text: &str, tags: &[&str]) -> NewNote {
    NewNote {
        kind: kind.into(),
        text: text.into(),
        tags: tags.iter().map(|t| t.to_string()).collect(),
        ..NewNote::default()
    }
}

fn cx(repo: &str, paths: &[&str], machine: &str) -> NoteContext {
    NoteContext {
        repo_key: repo.into(),
        paths: paths.iter().map(|p| p.to_string()).collect(),
        machine: machine.into(),
        ..NoteContext::default()
    }
}

#[test]
fn an_agents_note_waits_for_the_person() {
    let e = Env::new();
    let n =
        e.c.propose_note(
            ALICE,
            nn("rule", "Never drive Stephen's mouse.", &["everywhere"]),
        )
        .unwrap();
    assert_eq!(n.state, NOTE_PROPOSED);
    assert_eq!(n.by, ALICE);
    // A proposed note reaches no agent, and an agent's search skips it.
    assert!(e.c.notes_for(&cx(REPO_A, &[], "mac")).is_empty());
    assert!(e.c.search_notes("mouse", true).is_empty());
    assert_eq!(e.c.search_notes("mouse", false).len(), 1);
    // An agent can't accept, even another's.
    want_err(e.c.accept_note(BOB, &n.id), ErrorKind::NotAllowed);
    let n = e.c.accept_note(USER, &n.id).unwrap();
    assert_eq!(
        (n.state.as_str(), n.accepted_by.as_str()),
        (NOTE_ACCEPTED, USER)
    );
    let got = e.c.notes_for(&cx(REPO_A, &[], "mac"));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].why, TAG_EVERYWHERE);
    // It survives a replay.
    let c = e.reopen();
    assert_eq!(c.note_by_id(&n.id).unwrap().state, NOTE_ACCEPTED);
}

#[test]
fn the_persons_note_is_live_at_once() {
    let e = Env::new();
    let n =
        e.c.propose_note(
            USER,
            nn("preference", "Say Version, not take.", &["everywhere"]),
        )
        .unwrap();
    assert_eq!(n.state, NOTE_ACCEPTED);
    assert_eq!(n.accepted_by, USER);
}

#[test]
fn an_agents_edit_or_removal_of_a_live_note_is_a_proposal() {
    let e = Env::new();
    let n =
        e.c.propose_note(
            USER,
            nn(
                "gotcha",
                "ww builds need docker image wwppc.",
                &["topic:ww"],
            ),
        )
        .unwrap();
    let edited =
        e.c.edit_note(
            ALICE,
            &n.id,
            NoteEdit {
                text: Some("ww C builds need docker image wwppc on the Mac.".into()),
                ..NoteEdit::default()
            },
            "it's the Mac only",
        )
        .unwrap();
    assert_eq!(edited.text, n.text, "the live text stays until accepted");
    let p = edited.pending.clone().expect("a pending edit");
    assert_eq!(
        (p.by.as_str(), p.why.as_str()),
        (ALICE, "it's the Mac only")
    );
    let n2 = e.c.accept_note(USER, &n.id).unwrap();
    assert_eq!(n2.text, "ww C builds need docker image wwppc on the Mac.");
    assert!(n2.pending.is_none());
    // An agent's removal waits too; the person's accept removes it.
    let r = e.c.remove_note(BOB, &n.id, "the image is gone").unwrap();
    assert_eq!(r.state, NOTE_ACCEPTED);
    assert!(r.pending.as_ref().is_some_and(|c| c.remove));
    let r =
        e.c.accept_note(USER, &n.id)
            .unwrap_or_else(|_| e.c.note_by_id(&n.id).unwrap());
    assert_eq!(r.state, NOTE_REMOVED);
    assert!(e.c.notes(false).is_empty());
    assert_eq!(e.c.notes(true).len(), 1);
    let what: Vec<&str> = r.history.iter().map(|h| h.what.as_str()).collect();
    assert_eq!(
        what,
        [
            "proposed",
            "edit_proposed",
            "edited",
            "removal_proposed",
            "removed"
        ]
    );
}

#[test]
fn an_agent_withdraws_its_own_proposal() {
    let e = Env::new();
    let n =
        e.c.propose_note(ALICE, nn("rule", "Use worktrees.", &["everywhere"]))
            .unwrap();
    // Another agent's removal is only a proposal.
    let r = e.c.remove_note(BOB, &n.id, "").unwrap();
    assert_eq!(r.state, NOTE_PROPOSED);
    let r = e.c.remove_note(ALICE, &n.id, "dup").unwrap();
    assert_eq!(r.state, NOTE_REMOVED);
}

#[test]
fn bad_notes_are_refused() {
    let e = Env::new();
    for (n, kind) in [
        (nn("musing", "x", &["everywhere"]), ErrorKind::Invalid),
        (nn("rule", "", &["everywhere"]), ErrorKind::Invalid),
        (nn("rule", "no tags", &[]), ErrorKind::Invalid),
        (nn("rule", "odd tag", &["somewhere"]), ErrorKind::Invalid),
        (
            nn("rule", &"x".repeat(NOTE_MAX_TEXT + 1), &["everywhere"]),
            ErrorKind::Invalid,
        ),
        (
            nn(
                "pointer",
                "the key is ghp_0123456789abcdefghijklmnopqrstuvwxyzAB",
                &["everywhere"],
            ),
            ErrorKind::NotAllowed,
        ),
    ] {
        want_err(e.c.propose_note(ALICE, n), kind);
    }
}

#[test]
fn a_wake_gets_the_most_specific_notes_first() {
    let e = Env::new();
    let mk = |text: &str, tags: &[&str]| e.c.propose_note(USER, nn("rule", text, tags)).unwrap();
    let every = mk("everywhere rule", &["everywhere"]);
    let repo = mk("repo rule", &[&format!("repo:{REPO_A}")]);
    let path = mk("path rule", &[&format!("path:{REPO_A}:crates/inbox")]);
    let mac = mk("mac rule", &["machine:Mac"]);
    let _topic = mk("topic only", &["topic:ww"]);
    let _other = mk("other repo", &[&format!("repo:{REPO_B}")]);
    let _near = mk("not a prefix", &[&format!("path:{REPO_A}:crates/in")]);
    let got =
        e.c.notes_for(&cx(REPO_A, &["crates/inbox/src/watch.rs"], "mac"));
    let ids: Vec<&str> = got.iter().map(|m| m.note.id.as_str()).collect();
    assert_eq!(ids, [path.id.as_str(), &repo.id, &mac.id, &every.id]);
    // At most eight.
    for i in 0..10 {
        mk(&format!("rule {i}"), &["everywhere"]);
    }
    assert_eq!(e.c.notes_for(&cx("", &[], "")).len(), NOTES_PER_WAKE);
}

#[test]
fn accepting_another_agents_edit_of_a_proposal_makes_it_live() {
    let e = Env::new();
    let n =
        e.c.propose_note(ALICE, nn("rule", "Use worktrees.", &["everywhere"]))
            .unwrap();
    let edit = NoteEdit {
        text: Some("Use a worktree per card.".into()),
        ..NoteEdit::default()
    };
    let p = e.c.edit_note(BOB, &n.id, edit, "clearer").unwrap();
    assert_eq!(p.state, NOTE_PROPOSED);
    assert!(p.pending.is_some());
    let a = e.c.accept_note(USER, &n.id).unwrap();
    assert_eq!(
        (a.state.as_str(), a.text.as_str()),
        (NOTE_ACCEPTED, "Use a worktree per card.")
    );
}
