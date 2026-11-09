//! Modify on the person's comment before any agent read it (card #105).

use crate::common;

use common::*;
use inbox::notice::{Notice, route_notice};
use inbox::{EditState, ErrorKind, hold, st};

const REF: &str = "comment:C1";

fn comment_to(h: &Harness, to: &[&str], text: &str) -> Vec<String> {
    let n = Notice {
        to: to.iter().map(|s| s.to_string()).collect(),
        kind: "comment".into(),
        text: text.into(),
        ref_: REF.into(),
        ..Notice::default()
    };
    route_notice(&h.ib, &person(), &n, |_, _| None)
        .into_iter()
        .map(|r| {
            assert!(r.err.is_none(), "{:?}", r.err);
            r.delivery.id
        })
        .collect()
}

#[test]
fn modify_holds_cancel_releases_save_drops() {
    let mut h = Harness::new();
    h.abc();
    let ids = comment_to(&h, &[KEY_A, KEY_B], "The person commented:\nuse red");
    assert_eq!(h.ib.edit_state(USER, REF), EditState::Unread);
    assert_eq!(h.ib.edit_state(USER, "comment:other"), EditState::Unread);

    // Modify: held from both, and no agent's turn picks it up.
    assert_eq!(h.ib.hold_for_edit(USER, REF, USER).unwrap(), 2);
    for id in &ids {
        let d = h.want_state(id, st::HELD);
        assert_eq!(d.hold, hold::EDITING);
        assert!(inbox::editing(&d));
    }
    assert_eq!(h.ib.edit_state(USER, REF), EditState::Editing);
    assert!(h.ib.next_append(KEY_A).unwrap().is_none());
    assert!(h.ib.for_agent(KEY_A, false, 0).is_empty());
    assert_eq!(h.ib.held_count(KEY_A), 0, "edit holds are not counted");
    assert_eq!(h.ib.unread_count(KEY_A), 0);
    assert_eq!(
        h.ib.comment_edit_states(USER).get("C1"),
        Some(&EditState::Editing)
    );
    // Modify again is fine; the hold survives a restart.
    assert_eq!(h.ib.hold_for_edit(USER, REF, USER).unwrap(), 2);
    h.reopen();
    assert_eq!(h.ib.edit_state(USER, REF), EditState::Editing);

    // Cancel: queued again, as it was.
    assert_eq!(h.ib.release_edit(USER, REF, USER).unwrap(), 2);
    for id in &ids {
        h.want_state(id, st::QUEUED);
    }
    assert_eq!(h.ib.edit_state(USER, REF), EditState::Unread);

    // Save: the held ones are dropped and handed back with their text.
    h.ib.hold_for_edit(USER, REF, USER).unwrap();
    let held = h.ib.take_edit(USER, REF, USER).unwrap();
    assert_eq!(held.len(), 2);
    assert!(held.iter().all(|d| d.message.text.ends_with("use red")));
    for id in &ids {
        h.want_state(id, st::DISMISSED);
    }
    // The resent copies are what count from now on.
    let again = comment_to(&h, &[KEY_A, KEY_B], "The person commented:\nuse blue");
    assert_eq!(h.ib.edit_state(USER, REF), EditState::Unread);
    assert_eq!(h.ib.hold_for_edit(USER, REF, USER).unwrap(), 2);
    h.ib.release_edit(USER, REF, USER).unwrap();
    let w = h.append_for(KEY_A);
    assert!(w.delivery_ids.iter().any(|d| *d == again[0]));
}

#[test]
fn modify_is_refused_once_an_agent_has_it() {
    let h = Harness::new();
    h.abc();
    let ids = comment_to(&h, &[KEY_A, KEY_B], "The person commented:\nuse red");
    let w = h.append_for(KEY_A);
    // Issued to A's turn: already on its way, even before delivered.
    want_err(h.ib.hold_for_edit(USER, REF, USER), ErrorKind::BadState);
    h.ib.mark_delivered(KEY_A, &w.token).unwrap();
    assert_eq!(h.ib.edit_state(USER, REF), EditState::Read);
    want_err(h.ib.hold_for_edit(USER, REF, USER), ErrorKind::BadState);
    // Nothing was held for B either.
    h.want_state(&ids[1], st::QUEUED);
}
