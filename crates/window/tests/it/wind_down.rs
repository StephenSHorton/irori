//! Wind down all's dialog (card #220, #307): each agent's "wound down"
//! shows in the run's status and never as a message to the person.

use crate::common;

use common::*;
use irori_window::wind_down::{SCOPE_THIS, row};

#[test]
fn wound_down_replies_show_only_in_the_dialog() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    let me = r.w.core.user_sender().key;
    let st = r.w.wind_down_all(SCOPE_THIS, false).unwrap();
    let rows: Vec<_> = st
        .rows
        .iter()
        .filter(|x| x.key == r.key_a || x.key == r.key_b)
        .cloned()
        .collect();
    assert_eq!(rows.len(), 2, "{st:?}");
    for (key, name) in [(&r.key_a, "alpha"), (&r.key_b, "beta")] {
        let x = rows.iter().find(|x| &x.key == key).unwrap();
        ib.reply(
            &inbox::Sender::new(key, name, "claude"),
            &x.delivery_id,
            &format!("wound down; {name} left #1"),
        )
        .unwrap();
    }
    let st = r.w.wind_down_status();
    for (key, name) in [(&r.key_a, "alpha"), (&r.key_b, "beta")] {
        let x = st.rows.iter().find(|x| &x.key == key).unwrap();
        assert_eq!(x.state, row::DONE);
        assert_eq!(x.reply, format!("wound down; {name} left #1"));
    }
    assert!(
        ib.for_agent(&me, true, 0).is_empty(),
        "no message to the person"
    );
    assert_eq!(r.w.inbox_counts().unread, 0);
}

#[test]
fn a_straggler_stop_is_followed_and_kept_out_too() {
    let r = Rig::new();
    let ib = r.w.core.inbox();
    let me = r.w.core.user_sender().key;
    let st = r.w.wind_down_all(SCOPE_THIS, false).unwrap();
    let first = st
        .rows
        .iter()
        .find(|x| x.key == r.key_b)
        .unwrap()
        .delivery_id
        .clone();
    r.w.wind_down_stop(&r.key_b).unwrap();
    let st = r.w.wind_down_status();
    let x = st.rows.iter().find(|x| x.key == r.key_b).unwrap();
    assert_ne!(x.delivery_id, first, "the row follows the Stop");
    let d = ib.get(&x.delivery_id).unwrap();
    assert_eq!(d.mode, inbox::mode::STOP);
    assert!(inbox::is_wind_down(&d.message));
    ib.reply(
        &inbox::Sender::new(&r.key_b, "beta", "claude"),
        &x.delivery_id,
        "wound down",
    )
    .unwrap();
    let st = r.w.wind_down_status();
    let x = st.rows.iter().find(|x| x.key == r.key_b).unwrap();
    assert_eq!(
        (x.state.as_str(), x.reply.as_str()),
        (row::DONE, "wound down")
    );
    assert!(ib.for_agent(&me, true, 0).is_empty());
}
