//! Card #265: the person's text has no box limit. A 20,000-character paste
//! goes into an ask, a comment and a blocker answer whole, shows whole on
//! the card, and reaches the agent clipped with a show pointer.

use crate::common;

use common::*;

fn paste() -> String {
    (0..1200).map(|i| format!("log line {i:05} ok\n")).collect()
}

#[test]
fn a_long_paste_is_stored_whole_and_reaches_the_agent_clipped() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let long = paste();
    assert!(long.trim().chars().count() >= 20_000);
    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "Read the log".into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &t.id).unwrap();

    // A comment.
    r.w.comment(&t.id, &long, "wake", &[], &[]).unwrap();
    let d = r.w.card(&t.id).unwrap();
    assert!(
        d.thread.iter().any(|i| i.text == long.trim()),
        "the card shows the comment whole"
    );
    let ds = r.w.core.inbox().for_agent(&r.key_a, true, 0);
    let m = &ds[0].message;
    assert!(
        m.text.chars().count() <= inbox::MAX_TEXT,
        "clipped for the agent"
    );
    assert!(
        m.text.contains("(clipped; show comment:") && m.text.ends_with("for all of it)"),
        "{}",
        &m.text[m.text.len() - 200..]
    );

    // A blocker answer.
    let (t, _) = co
        .raise(
            &r.key_a,
            &t.id,
            coord::NewBlocker {
                question: "what does the log say?".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let bid = t.blockers[0].id.clone();
    r.w.answer(&t.id, &bid, -1, &long, false, &[]).unwrap();
    let d = r.w.card(&t.id).unwrap();
    let b = d.blockers.iter().find(|b| b.id == bid).unwrap();
    assert!(
        b.thread.iter().any(|i| i.text == long.trim())
            || b.resolved.as_ref().is_some_and(|r| r.answer == long.trim()),
        "the card shows the answer whole: {:?}",
        b.thread
            .iter()
            .map(|i| (i.kind.as_str(), i.text.len()))
            .collect::<Vec<_>>()
    );
    let ds = r.w.core.inbox().for_agent(&r.key_a, true, 0);
    assert!(
        ds.iter().any(|d| d
            .message
            .text
            .contains(&format!("(clipped; show task:{} for all of it)", t.id))),
        "the answered notice points at show"
    );

    // An ask (a thread to the room) keeps all of it.
    let res = r.w.ask(&long, "", true, &[], "", &[]).unwrap();
    let a = co.get_task(&res.task.id).unwrap();
    assert_eq!(a.objective, long.trim());
}
