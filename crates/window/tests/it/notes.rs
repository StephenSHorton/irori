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

    // An agent writes a note: live at once (card #616), from an agent.
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
    assert_eq!((v.state.as_str(), v.accepted_by.as_str()), ("accepted", ""));
    assert!(!v.by_person, "{v:?}");
    assert_eq!(v.stale, "");

    // The agent's edit applies at once, and so does the person's.
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
    assert_eq!(
        w.notes_list(false).unwrap()[0].text,
        "Load 20 times presence tests out."
    );
    let e = w
        .note_edit(
            &p.id,
            NoteEditInput {
                text: Some("Under load, rerun presence tests alone.".into()),
                ..NoteEditInput::default()
            },
        )
        .unwrap();
    assert_eq!(e.text, "Under load, rerun presence tests alone.");

    // An agent's note from a card: once the card goes, it asks Still
    // true?
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
    assert_eq!(from_card.task_id, t.id);
    w.delete_card(&t.id).unwrap();
    let kept = w
        .notes_list(false)
        .unwrap()
        .into_iter()
        .find(|n| n.id == from_card.id)
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
