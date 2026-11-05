//! The person's mentions from the window (card #77; board.md "Mentions").

mod common;

use common::*;
use irori_window::ts::Mention;

fn m(key: &str, name: &str) -> Mention {
    Mention {
        key: key.into(),
        name: name.into(),
        gone: false,
    }
}

#[test]
fn a_comment_mention_wakes_and_watches() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "Port the window".into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &t.id).unwrap();
    // The Tell is next_turn; the mention wakes beta anyway.
    let c =
        r.w.comment(
            &t.id,
            "@beta can you check the build?",
            "next_turn",
            &[m(&r.key_b, "beta"), m("nodev/nosuch", "ghost")],
            &[],
        )
        .unwrap();
    assert_eq!(c.told.to, vec![r.key_a.clone()]);
    assert_eq!(c.told.mode, "next_turn");
    let states: Vec<(&str, &str)> = c
        .mentioned
        .iter()
        .map(|o| (o.name.as_str(), o.state.as_str()))
        .collect();
    assert_eq!(states, [("beta", "woke"), ("ghost", "unknown")]);
    let ds = r.w.core.inbox().for_agent(&r.key_b, true, 0);
    assert_eq!(ds.len(), 1, "{ds:?}");
    assert_eq!(ds[0].mode, inbox::mode::WAKE);
    assert_eq!(ds[0].message.reason, coord::NOTICE_MENTIONED);
    let got = co.get_task(&t.id).unwrap();
    assert_eq!(got.watchers, std::slice::from_ref(&r.key_b));
    assert_eq!(got.holders, std::slice::from_ref(&r.key_a));
    // The thread shows the mention with the agent as the window knows it.
    let d = r.w.card(&t.id).unwrap();
    let it = d
        .thread
        .iter()
        .find(|i| i.text.starts_with("@beta"))
        .unwrap();
    assert_eq!(it.mentions.len(), 1);
    assert_eq!(it.mentions[0].name, "beta");
    assert_eq!(it.mentions[0].who.name, "beta");
    assert!(!it.mentions[0].gone);
}

#[test]
fn a_mention_with_tell_none_still_wakes() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "quiet".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let c =
        r.w.comment(&t.id, "fyi @beta", "none", &[m(&r.key_b, "beta")], &[])
            .unwrap();
    assert_eq!(c.mentioned[0].state, "woke");
    let ds = r.w.core.inbox().for_agent(&r.key_b, true, 0);
    assert_eq!(ds.len(), 1);
    assert!(
        ds[0].message.text.contains("Nobody holds it"),
        "{}",
        ds[0].message.text
    );
}

#[test]
fn an_ask_with_mentions_goes_only_to_them() {
    let r = Rig::new();
    let res =
        r.w.ask(
            "@beta why is it slow?",
            "",
            false,
            &[],
            "",
            &[m(&r.key_b, "beta")],
        )
        .unwrap();
    assert_eq!(res.told.to, vec![r.key_b.clone()]);
    assert_eq!(res.told.mode, "wake");
    assert_eq!(res.mentioned[0].state, "woke");
    assert!(r.w.core.inbox().for_agent(&r.key_a, true, 0).is_empty());
    assert_eq!(r.w.core.inbox().for_agent(&r.key_b, true, 0).len(), 1);
    // A shared board's ask refuses them.
    assert!(
        r.w.ask("x", "", false, &[], "brd", &[m(&r.key_b, "beta")])
            .unwrap_err()
            .contains("mentions work on an ask in Personal")
    );
}

#[test]
fn a_message_mention_gets_a_copy() {
    let r = Rig::new();
    let s =
        r.w.inbox_send(&r.key_a, "", "loop in @beta", &[m(&r.key_b, "beta")])
            .unwrap();
    assert_eq!(s.message.mentions.len(), 1);
    assert_eq!(s.mentioned[0].state, "woke");
    let ds = r.w.core.inbox().for_agent(&r.key_b, true, 0);
    assert_eq!(ds.len(), 1);
    assert_eq!(ds[0].message.reason, inbox::REASON_MENTIONED);
    assert_eq!(ds[0].message.parent_id, s.message.id);
}

#[test]
fn candidates_list_every_live_agent() {
    let r = Rig::new();
    let cs = r.w.mention_candidates();
    let names: Vec<&str> = cs.iter().map(|c| c.who.name.as_str()).collect();
    assert_eq!(names, ["alpha", "beta"]);
    assert!(cs.iter().all(|c| c.who.live && !c.cwd.is_empty()));
}

#[test]
fn a_paused_ask_still_reaches_a_mentioned_agent() {
    let r = Rig::new();
    r.w.inbox_set_paused(true).unwrap();
    let res =
        r.w.ask(
            "@beta fix the build",
            "",
            false,
            &[],
            "",
            &[m(&r.key_b, "beta")],
        )
        .unwrap();
    // Pause all holds the notice; the agent isn't called gone.
    assert_eq!(res.told.to, vec![r.key_b.clone()]);
    assert_ne!(res.mentioned[0].state, "gone", "{:?}", res.mentioned);
    assert!(!res.task.ask.unwrap().unclaimed);
}

#[test]
fn a_re_answer_tells_the_mentioned_holder_once() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let me = r.w.core.user_sender().key;
    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "ship it".into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &t.id).unwrap();
    // The person's own item, answered once already (not open).
    let (t, _) = co
        .raise(
            &me,
            &t.id,
            coord::NewBlocker {
                question: "check the staging DNS".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let bid = t.blockers[0].id.clone();
    co.answer(
        &me,
        &t.id,
        &bid,
        coord::BlockerAnswer {
            text: "looking".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let before = r.w.core.inbox().for_agent(&r.key_a, true, 0).len();
    let res =
        r.w.answer(
            &t.id,
            &bid,
            -1,
            "@alpha it's fine",
            false,
            &[m(&r.key_a, "alpha")],
        )
        .unwrap();
    assert_eq!(res.mentioned[0].state, "woke");
    let ds = r.w.core.inbox().for_agent(&r.key_a, true, 0);
    assert_eq!(ds.len(), before + 1, "{ds:?}");
    assert_eq!(ds[0].mode, inbox::mode::WAKE);
}

#[test]
fn a_resolve_note_mention_wakes() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "ship it".into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &t.id).unwrap();
    let (t, _) = co
        .raise(
            &r.key_a,
            &t.id,
            coord::NewBlocker {
                question: "which port?".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let bid = t.blockers[0].id.clone();
    r.w.answer(&t.id, &bid, -1, "8080", false, &[]).unwrap();
    let res =
        r.w.resolve_blocker(
            &t.id,
            &bid,
            "@beta take the follow-up",
            &[m(&r.key_b, "beta")],
        )
        .unwrap();
    assert_eq!(res.mentioned[0].state, "woke");
    let ds = r.w.core.inbox().for_agent(&r.key_b, true, 0);
    assert_eq!(ds.len(), 1, "{ds:?}");
    assert_eq!(ds[0].mode, inbox::mode::WAKE);
    assert!(co.get_task(&t.id).unwrap().watchers.contains(&r.key_b));
}
