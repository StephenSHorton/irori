//! The Notes page's commands (card #411, docs/design/notes.md §11).

use crate::common;

use common::*;
use irori_window::notes::{NoteEditInput, NoteInput};

fn input(text: &str, tags: &[&str]) -> NoteInput {
    NoteInput {
        kind: "rule".into(),
        text: text.into(),
        tags: tags.iter().map(|t| t.to_string()).collect(),
    }
}

#[test]
fn notes_in_the_window() {
    let r = Rig::new();
    let w = &r.w;

    // An agent proposes a Mine note: it waits, from an agent.
    let p = w
        .core
        .propose_note(
            &r.key_a,
            coord::NewNote {
                kind: "gotcha".into(),
                text: "Load 20 makes presence tests time out.".into(),
                tags: vec!["topic:tests".into()],
                ..Default::default()
            },
        )
        .unwrap();
    let list = w.notes_list(false).unwrap();
    assert_eq!(list.len(), 1);
    let v = &list[0];
    assert_eq!(v.state, "proposed");
    assert!(!v.by_person, "{v:?}");
    assert_eq!(v.stale, "");

    // Keep: live, accepted by the person.
    let k = w.note_accept(&format!("note:{}", p.id)).unwrap();
    assert_eq!(k.state, "accepted");
    assert_eq!(k.accepted_by, w.core.user_sender().key);

    // The agent proposes an edit: shown as pending; the person's edit
    // applies at once and drops it.
    w.core
        .edit_note(
            &r.key_a,
            &p.id,
            coord::NoteEdit {
                text: Some("Load 20 times presence tests out.".into()),
                ..Default::default()
            },
            "shorter",
        )
        .unwrap();
    let v = &w.notes_list(false).unwrap()[0];
    let pend = v.pending.as_ref().expect("pending edit");
    assert_eq!((pend.why.as_str(), pend.remove), ("shorter", false));
    let e = w
        .note_edit(
            &p.id,
            NoteEditInput {
                text: Some("Under load, rerun presence tests alone.".into()),
                ..NoteEditInput::default()
            },
        )
        .unwrap();
    assert!(e.pending.is_none());
    assert_eq!(e.text, "Under load, rerun presence tests alone.");

    // An agent's note from a card, kept: once the card goes, it asks
    // Still true?
    let t = w
        .core
        .coord()
        .create_task(
            &w.core.user_sender().key,
            coord::NewTask {
                title: "the source".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let from_card = w
        .core
        .propose_note(
            &r.key_a,
            coord::NewNote {
                kind: "decision".into(),
                text: "Answers on this card are decisions.".into(),
                tags: vec!["everywhere".into()],
                source: format!("task:{}", t.id),
                task_id: t.id.clone(),
                ..Default::default()
            },
        )
        .unwrap();
    let kept = w
        .note_accept(&format!("note:{}", from_card.id))
        .unwrap();
    assert_eq!(kept.state, "accepted");
    assert_eq!(kept.task_id, t.id);
    w.delete_card(&t.id).unwrap();
    let kept = w
        .notes_list(false)
        .unwrap()
        .into_iter()
        .find(|n| n.id == kept.id)
        .unwrap();
    assert_eq!(kept.stale, "its source card is gone");

    // Removed: gone from the list, kept in the history view.
    w.note_remove(&kept.id, "card gone").unwrap();
    assert_eq!(w.notes_list(false).unwrap().len(), 1);
    assert_eq!(w.notes_list(true).unwrap().len(), 2);

    // New note about a shared space (card #616): the person's own, live at
    // once, in the one list.
    w.boards_create("MLG", "mlg").unwrap();
    let about = w
        .note_new(input("Ship to the repo.", &["board:mlg"]))
        .unwrap();
    assert_eq!(about.state, "accepted");
    assert!(about.by_person);
    assert_eq!(w.notes_list(false).unwrap().len(), 2);
}

/// Card #468: the sidebar's count, without notes_list's views and Still
/// true? checks: an agent's proposal and an agent's pending edit of a
/// live note wait; a person's note and a kept one don't.
#[test]
fn notes_waiting_counts_what_new_shows() {
    let r = Rig::new();
    let w = &r.w;
    assert_eq!(w.notes_waiting().unwrap(), 0);
    let p = w
        .core
        .propose_note(
            &r.key_a,
            coord::NewNote {
                kind: "gotcha".into(),
                text: "An agent's proposal waits.".into(),
                tags: vec!["everywhere".into()],
                ..Default::default()
            },
        )
        .unwrap();
    w.note_new(input("A person's note is live at once.", &["everywhere"]))
        .unwrap();
    assert_eq!(w.notes_waiting().unwrap(), 1);
    w.note_accept(&format!("note:{}", p.id)).unwrap();
    assert_eq!(w.notes_waiting().unwrap(), 0);
    w.core
        .edit_note(
            &r.key_a,
            &p.id,
            coord::NoteEdit {
                text: Some("A pending edit waits too.".into()),
                ..Default::default()
            },
            "clearer",
        )
        .unwrap();
    assert_eq!(w.notes_waiting().unwrap(), 1);
    // The same count the Notes page's New shows from notes_list.
    let listed = w.notes_list(false).unwrap();
    let waiting = listed
        .iter()
        .filter(|n| n.state == "proposed" || (n.state == "accepted" && n.pending.is_some()))
        .count();
    assert_eq!(waiting, 1);
}

/// Card #468's numbers: what the sidebar's count cost per board:changed
/// before (notes_list(""): every note's view and its Still true? checks,
/// each path: tag a stat in every checkout) and after (notes_waiting).
/// `cargo test -p irori-window --test it -- --ignored --nocapture notes::`
#[test]
#[ignore]
fn notes_waiting_against_notes_list_timing() {
    let r = Rig::new();
    let w = &r.w;
    let root = tempfile::tempdir().unwrap();
    let repo = "remote:example.com/perf/repo";
    w.core
        .capture()
        .emit_row(
            "irori",
            capture::KIND_REPO_SEEN,
            &capture::RepoSeen {
                repo_key: repo.into(),
                root: root.path().to_string_lossy().into(),
                at: gocompat::time::GoTime::default(),
            },
        )
        .unwrap();
    assert_eq!(w.core.capture().repo_roots(repo).len(), 1);
    const N: usize = 300;
    for i in 0..N {
        let p = root.path().join(format!("src/f{i}.rs"));
        if i % 2 == 0 {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "").unwrap();
        }
        w.core
            .propose_note(
                &r.key_a,
                coord::NewNote {
                    kind: "gotcha".into(),
                    text: format!("Note {i} about src/f{i}.rs."),
                    tags: vec![format!("path:{repo}:src/f{i}.rs"), "topic:perf".into()],
                    ..Default::default()
                },
            )
            .unwrap();
    }
    let list = w.notes_list(false).unwrap();
    assert_eq!(list.len(), N);
    assert_eq!(list.iter().filter(|n| !n.stale.is_empty()).count(), N / 2);
    let best = |f: &dyn Fn() -> usize| {
        let mut best = std::time::Duration::MAX;
        for _ in 0..20 {
            let t = std::time::Instant::now();
            std::hint::black_box(f());
            best = best.min(t.elapsed());
        }
        best
    };
    // What the IPC reply carried: the views as JSON.
    let before = best(&|| {
        serde_json::to_vec(&w.notes_list(false).unwrap())
            .unwrap()
            .len()
    });
    let bytes = serde_json::to_vec(&list).unwrap().len();
    let after = best(&|| w.notes_waiting().unwrap());
    eprintln!(
        "card #468, {N} notes with path: tags: notes_list(\"\") + JSON {before:?} ({bytes} bytes); notes_waiting {after:?} (a number)"
    );
}
