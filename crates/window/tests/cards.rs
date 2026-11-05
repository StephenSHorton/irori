//! Go cards_service_test.go and board_service_test.go on a scratch Core:
//! the card's thread, a message opened in full, answers and "Mark
//! resolved", pins, and a comment nobody hears. Agent a (alpha) and agent
//! b (beta) are the Rig's two live sessions; Go's fakes become real rows.

mod common;

use common::*;
use irori_window::cards::{
    BLOCKER_ANSWERED, BLOCKER_OPEN, BLOCKER_RESOLVED, CardBlocker, CardDetail, MAX_THREAD_VIEW,
    NOTE_ITEM, THREAD_COMMENT, THREAD_EVENT, THREAD_MESSAGE,
};

/// Go `heldTask`: a task agent a created and claimed.
fn held_task(r: &Rig) -> coord::Task {
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
    co.claim_task(&r.key_a, &t.id).unwrap()
}

/// Another card alpha holds: assigned to it first, since an agent takes a
/// second card only when one is handed to it (one card at a time, #82).
fn another_held_task(r: &Rig) -> coord::Task {
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
    co.assign(&me(r), &t.id, &r.key_a).unwrap();
    co.claim_task(&r.key_a, &t.id).unwrap()
}

fn me(r: &Rig) -> String {
    r.w.core.user_sender().key
}

fn raise(r: &Rig, by: &str, id: &str, q: &str, options: &[&str]) -> coord::Blocker {
    r.w.core
        .coord()
        .raise(
            by,
            id,
            coord::NewBlocker {
                question: q.into(),
                options: options
                    .iter()
                    .map(|a| coord::BlockerOption {
                        answer: (*a).into(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            },
        )
        .unwrap()
        .1
}

fn blocker<'a>(d: &'a CardDetail, id: &str) -> &'a CardBlocker {
    d.blockers
        .iter()
        .find(|b| b.id == id)
        .unwrap_or_else(|| panic!("no blocker {id} in {:?}", d.blockers))
}

/// Go TestCardThread: comments, steps and messages in one thread, the
/// person's comment as "You", a message's sender and receiver, held and
/// gone messages, no comment's own line; then the newest MAX_THREAD_VIEW.
#[test]
fn card_thread() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let t = held_task(&r);
    let ended = format!("{}/ended", r.w.core.device_id());
    co.add_comment(&me(&r), &t.id, "Use the staging DB.", false)
        .unwrap();
    co.set_steps(
        &r.key_a,
        &t.id,
        vec![coord::Step {
            text: "migrate".into(),
            ..Default::default()
        }],
    )
    .unwrap();
    for m in [
        coord::MessageEntry {
            task_id: t.id.clone(),
            msg_id: "msg_1".into(),
            from: r.key_a.clone(),
            to: r.key_b.clone(),
            text: "Can you review the migration?".into(),
            files: 2,
            ..Default::default()
        },
        coord::MessageEntry {
            task_id: t.id.clone(),
            msg_id: "msg_2".into(),
            from: r.key_b.clone(),
            to: r.key_a.clone(),
            held: true,
            ..Default::default()
        },
        coord::MessageEntry {
            task_id: t.id.clone(),
            msg_id: "msg_3".into(),
            from: r.key_a.clone(),
            to: ended.clone(),
            text: "Still there?".into(),
            gone: true,
            writer: "irori".into(),
            ..Default::default()
        },
    ] {
        co.record_message(m).unwrap();
    }
    co.add_comment(&r.key_b, &t.id, "Looks good.", false)
        .unwrap();

    let d = r.w.card(&t.id).unwrap();
    let kinds: Vec<&str> = d.thread.iter().map(|i| i.kind.as_str()).collect();
    assert_eq!(
        kinds,
        [
            THREAD_EVENT,
            THREAD_EVENT,
            THREAD_COMMENT,
            THREAD_EVENT,
            THREAD_MESSAGE,
            THREAD_MESSAGE,
            THREAD_MESSAGE,
            THREAD_COMMENT
        ],
        "thread {:?}",
        d.thread
    );
    for it in &d.thread {
        assert!(
            !(it.kind == THREAD_EVENT && it.text.contains("commented")),
            "a comment's own line is in the thread: {it:?}"
        );
        assert!(it.device.is_empty(), "device chip with one device: {it:?}");
    }
    let th = &d.thread;
    assert_eq!(
        (th[2].who.name.as_str(), th[2].text.as_str()),
        ("You", "Use the staging DB.")
    );
    assert!(th[2].to.is_none());
    assert!(th[3].text.starts_with("alpha "), "event {:?}", th[3]);
    let m = &th[4];
    assert_eq!(
        (m.id.as_str(), m.who.name.as_str(), m.text.as_str(), m.files),
        ("msg_1", "alpha", "Can you review the migration?", 2)
    );
    assert_eq!(m.to.as_ref().unwrap().name, "beta");
    assert!(!m.gone && !m.held);
    let h = &th[5];
    assert!(h.held && h.text.is_empty() && h.who.key == r.key_b);
    assert_eq!(h.to.as_ref().unwrap().key, r.key_a);
    let g = &th[6];
    assert!(g.gone && g.who.key == r.key_a);
    let gt = g.to.as_ref().unwrap();
    assert_eq!(
        (gt.key.as_str(), gt.status.as_str()),
        (ended.as_str(), "gone")
    );
    assert!(!d.history_trimmed && d.thread_earlier == 0 && d.comments.len() == 5);
    let raw = serde_json::to_string(&d.thread[0]).unwrap();
    assert!(!raw.contains("null"), "null in {raw}");

    // Past both caps: coord keeps MAX_ACTIVITY lines, the thread sends
    // the newest MAX_THREAD_VIEW.
    for i in 0..coord::MAX_ACTIVITY {
        co.add_comment(&r.key_a, &t.id, &format!("c{i}"), false)
            .unwrap();
    }
    let d = r.w.card(&t.id).unwrap();
    let total = 5 + coord::MAX_ACTIVITY;
    assert!(d.history_trimmed);
    assert_eq!(d.thread.len(), MAX_THREAD_VIEW);
    assert_eq!(d.thread_earlier as usize, total - MAX_THREAD_VIEW);
    assert_eq!(
        d.thread.last().unwrap().text,
        format!("c{}", coord::MAX_ACTIVITY - 1)
    );
    assert_eq!(d.recent.len(), 8);
}

/// Go TestCardMessage: a message on a card's thread opens in full with
/// its files; another card's message, a plain comment, an unknown id and
/// one not in this inbox are not found.
#[test]
fn card_message() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let t = held_task(&r);
    let other = another_held_task(&r);
    let long = "y".repeat(coord::MAX_COMMENT + 100);
    let to = inbox::Address::new(inbox::addr::AGENT, &r.key_b);
    let from = inbox::Sender::new(&r.key_a, "alpha", "agent");
    let (m1, _) = r.w.core.inbox().send(&from, &to, &long, "").unwrap();
    let (m2, _) = r.w.core.inbox().send(&from, &to, "held words", "").unwrap();
    for (id, text) in [
        (m1.id.as_str(), long.as_str()),
        (m2.id.as_str(), "held words"),
        ("msg_9", "not here"),
    ] {
        co.record_message(coord::MessageEntry {
            task_id: t.id.clone(),
            msg_id: id.into(),
            from: r.key_a.clone(),
            to: r.key_b.clone(),
            text: text.into(),
            ..Default::default()
        })
        .unwrap();
    }
    let (cm, _) = co.add_comment(&r.key_a, &t.id, "a comment", false).unwrap();

    let got = r.w.message(&t.id, &m1.id).unwrap();
    assert_eq!(got.text, long, "the whole text, past the card's cut");
    let got = r.w.message(&t.id, &format!(" {} ", m2.id)).unwrap();
    assert_eq!(got.text, "held words");
    assert!(got.files.is_empty());
    let nf = coord::ErrorKind::NotFound.text();
    for (task, id) in [
        (other.id.as_str(), m1.id.as_str()),
        (t.id.as_str(), cm.id.as_str()),
        (t.id.as_str(), "msg_404"),
        (t.id.as_str(), "msg_9"),
    ] {
        let e = r.w.message(task, id).unwrap_err();
        assert!(e.starts_with(nf), "{id} on {task}: {e}");
    }
}

/// Go TestAnswerLeavesBlockerOpen: the person's answer waits on the
/// raiser (answered, not a decision); the raiser's reply puts it back in
/// the person's count; only the agent's resolve closes it.
#[test]
fn answer_leaves_blocker_open() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let t = held_task(&r);
    let bl = raise(&r, &r.key_a, &t.id, "Which DB?", &["postgres"]);
    let res =
        r.w.answer(&t.id, &bl.id, -1, "Which one is cheaper to run?", true, &[])
            .unwrap();
    let b = blocker(&res.card, &bl.id);
    assert_eq!(res.card.summary.state, coord::BLOCKED);
    assert_eq!(b.state, BLOCKER_ANSWERED);
    assert!(b.resolved.is_none());
    assert_eq!(
        (res.card.summary.open_blockers, res.card.summary.waiting),
        (0, 1)
    );
    assert_eq!(b.sent_to.as_ref().unwrap().name, "alpha");
    assert_eq!(res.told.to, std::slice::from_ref(&r.key_a));
    assert_eq!(res.told.mode, "wake");
    assert_eq!(r.w.decide_count(), 0, "an answered item is no decision");

    co.comment_on(
        &r.key_a,
        &t.id,
        coord::CommentInput {
            text: "Postgres, but I need the budget.".into(),
            blocker_id: bl.id.clone(),
            missing: vec!["monthly budget".into()],
            ..Default::default()
        },
    )
    .unwrap();
    let d = r.w.card(&t.id).unwrap();
    let b = blocker(&d, &bl.id);
    assert_eq!(b.state, coord::BLOCKER_REPLIED);
    assert_eq!((d.summary.open_blockers, d.summary.waiting), (1, 0));
    assert_eq!(d.summary.blocker_question, "Which DB?");
    assert_eq!(b.missing, ["monthly budget"]);
    assert!(b.sent_to.is_none());
    assert_eq!(b.thread.len(), 2);
    assert_eq!(b.thread[0].who.name, "You");
    assert_eq!(b.thread[0].to.as_ref().unwrap().key, r.key_a);
    assert_eq!(b.thread[1].who.key, r.key_a);
    assert_eq!(b.thread[1].missing, ["monthly budget"]);
    assert_eq!(b.thread[1].blocker_id, bl.id);
    let lines: Vec<&str> = d
        .thread
        .iter()
        .filter(|i| i.blocker_id == bl.id)
        .map(|i| {
            assert_eq!(i.kind, THREAD_EVENT, "blocker comment in the thread: {i:?}");
            i.text.as_str()
        })
        .collect();
    assert_eq!(lines.len(), 3, "lines {lines:?}");
    // Card #490: the person reads the question, never the blocker's id.
    assert!(lines[1].ends_with("answered “Which DB?”"), "{lines:?}");
    assert!(lines[2].ends_with("replied on “Which DB?”"), "{lines:?}");
    assert!(lines.iter().all(|l| !l.contains(&bl.id)), "{lines:?}");
    let gists: Vec<&str> = d
        .thread
        .iter()
        .filter(|i| i.blocker_id == bl.id)
        .skip(1)
        .map(|i| i.blocker_gist.as_str())
        .collect();
    assert_eq!(gists, ["Which DB?", "Which DB?"]);
    assert_eq!(r.w.decide_count(), 1, "a replied item is a decision");

    co.answer(
        &r.key_a,
        &t.id,
        &bl.id,
        coord::BlockerAnswer {
            text: "Going with postgres on the small plan.".into(),
            resolve: true,
            ..Default::default()
        },
    )
    .unwrap();
    let d = r.w.card(&t.id).unwrap();
    let b = blocker(&d, &bl.id);
    let rs = b.resolved.as_ref().unwrap();
    assert_eq!(d.summary.state, coord::WORKING);
    assert_eq!(b.state, BLOCKER_RESOLVED);
    assert_eq!(rs.note, "Going with postgres on the small plan.");
    assert_eq!(rs.answer, "Which one is cheaper to run?");
    assert_eq!((d.summary.open_blockers, d.summary.waiting), (0, 0));
    assert_eq!(b.thread.len(), 2);
    let raw = serde_json::to_string(b).unwrap();
    assert!(!raw.contains("null"), "null in {raw}");
}

/// Go TestAnswerResolvesWhenNobodyCanAct and
/// TestAnswerResolvesPersonRaisedWithLiveHolder: with no live agent to act
/// on it (the raiser ended, nobody live holds it, or the person raised
/// it), the person's answer resolves the item at once.
#[test]
fn answer_resolves_when_nobody_can_act() {
    let r = Rig::new();
    let co = r.w.core.coord();
    // A card an ended session holds.
    let ended = format!("{}/ended-session", r.w.core.device_id());
    let t = co
        .create_task(
            &ended,
            coord::NewTask {
                title: "orphan".into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.claim_task(&ended, &t.id).unwrap();
    let b1 = raise(&r, &ended, &t.id, "Ship it?", &[]);
    let b2 = raise(&r, &ended, &t.id, "Which region?", &[]);
    let res = r.w.answer(&t.id, &b1.id, -1, "yes", true, &[]).unwrap();
    let b = blocker(&res.card, &b1.id);
    let rs = b.resolved.as_ref().unwrap();
    assert_eq!(b.state, BLOCKER_RESOLVED);
    assert_eq!((rs.answer.as_str(), rs.note.as_str()), ("yes", ""));
    assert_eq!(res.card.summary.state, coord::BLOCKED);
    let res = r.w.answer(&t.id, &b2.id, -1, "eu-west", true, &[]).unwrap();
    let b = blocker(&res.card, &b2.id);
    assert_eq!(b.state, BLOCKER_RESOLVED);
    assert_eq!(b.resolved.as_ref().unwrap().answer, "eu-west");
    assert_eq!(res.card.summary.state, coord::WORKING);

    // The person's own item on a card a live agent holds.
    let t2 = another_held_task(&r);
    let mine = raise(&r, &me(&r), &t2.id, "check the staging DNS", &[]);
    let res =
        r.w.answer(&t2.id, &mine.id, -1, "checked", false, &[])
            .unwrap();
    let b = blocker(&res.card, &mine.id);
    assert_eq!(b.state, BLOCKER_RESOLVED);
    assert_eq!(b.resolved.as_ref().unwrap().answer, "checked");
    assert_ne!(res.card.summary.state, coord::BLOCKED);
    assert_eq!(res.card.summary.waiting, 0);
    assert_eq!(r.w.decide_count(), 0);
}

/// Go TestResolveBlockerByPerson: "Mark resolved" keeps the person's
/// answer (no note) or adds the note beside it; an unanswered item, the
/// note item and a second resolve are refused.
#[test]
fn resolve_blocker_by_person() {
    let r = Rig::new();
    let t = held_task(&r);
    let b1 = raise(&r, &r.key_a, &t.id, "Which DB?", &["postgres", "8080"]);
    let b2 = raise(&r, &r.key_a, &t.id, "Which port?", &["postgres", "8080"]);
    let invalid = coord::ErrorKind::Invalid.text();
    assert!(
        r.w.resolve_blocker(&t.id, NOTE_ITEM, "", &[])
            .unwrap_err()
            .starts_with(invalid)
    );
    assert!(
        r.w.resolve_blocker(&t.id, &b1.id, "", &[])
            .unwrap_err()
            .starts_with(invalid),
        "resolved an unanswered item with nothing"
    );
    r.w.answer(&t.id, &b1.id, 0, "", false, &[]).unwrap();
    r.w.answer(&t.id, &b2.id, -1, "whatever is free", false, &[])
        .unwrap();
    let res = r.w.resolve_blocker(&t.id, &b1.id, "", &[]).unwrap();
    let b = blocker(&res.card, &b1.id);
    let rs = b.resolved.as_ref().unwrap();
    assert_eq!(b.state, BLOCKER_RESOLVED);
    assert_eq!((rs.answer.as_str(), rs.option), ("postgres", 0));
    assert_eq!((rs.note.as_str(), rs.by.name.as_str()), ("", "You"));
    assert_eq!(res.card.summary.state, coord::BLOCKED);
    assert_eq!(res.told.mode, "next_turn");

    let res =
        r.w.resolve_blocker(
            &t.id,
            &format!(" {} ", b2.id),
            "I'll pick one myself later.",
            &[],
        )
        .unwrap();
    let b = blocker(&res.card, &b2.id);
    let rs = b.resolved.as_ref().unwrap();
    assert_eq!(b.state, BLOCKER_RESOLVED);
    assert_eq!(rs.answer, "whatever is free");
    assert_eq!(rs.note, "I'll pick one myself later.");
    assert_eq!(res.card.summary.state, coord::WORKING);
    let twice = r.w.resolve_blocker(&t.id, &b2.id, "", &[]).unwrap_err();
    assert!(
        twice.starts_with(coord::ErrorKind::BadState.text()),
        "resolved twice"
    );
    // Card #490: the person reads the question, not "blocker b2".
    assert!(
        twice.contains("“Which port?”") && !twice.contains(&format!("blocker {}", b2.id)),
        "{twice}"
    );
    let again =
        r.w.answer(&t.id, &b2.id, -1, "late", false, &[])
            .unwrap_err();
    assert!(!again.contains(&format!("blocker {}", b2.id)), "{again}");
}

/// Go TestCardsPinnedFirst and TestBoardActions' movePin: pinned cards
/// lead in pin order, MovePin reorders them, an agent can't pin, and a
/// pinned parent keeps its subtasks with it.
#[test]
fn cards_pinned_first() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let (a, z, m) = (held_task(&r), another_held_task(&r), another_held_task(&r));
    r.w.pin(&z.id, true).unwrap();
    r.w.pin(&a.id, true).unwrap();
    assert!(
        co.pin(&r.key_a, &m.id, true).is_err(),
        "an agent pinned a card"
    );
    let ids = || -> Vec<String> { r.w.cards().into_iter().map(|s| s.id).collect() };
    let got = ids();
    assert_eq!(got[..2], [z.id.clone(), a.id.clone()]);
    assert_eq!(got[2], m.id);
    r.w.move_pin(&a.id, &z.id).unwrap();
    assert_eq!(ids()[..2], [a.id.clone(), z.id.clone()]);
    r.w.pin(&a.id, false).unwrap();
    let cs = r.w.cards();
    assert!(cs[0].pinned && cs[0].id == z.id && !cs[1].pinned);

    let p = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "parent".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let k = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "kid".into(),
                parent: p.id.clone(),
                ..Default::default()
            },
        )
        .unwrap();
    r.w.pin(&p.id, true).unwrap();
    assert_eq!(ids()[..3], [z.id.clone(), p.id.clone(), k.id.clone()]);
}

/// Go TestBoardCommentNobody: a comment on the person's own card with
/// nobody on it tells nobody, whatever the person picked.
#[test]
fn comment_with_nobody_on_the_card() {
    let r = Rig::new();
    let t =
        r.w.core
            .coord()
            .create_task(
                &me(&r),
                coord::NewTask {
                    title: "mine".into(),
                    ..Default::default()
                },
            )
            .unwrap();
    let res =
        r.w.comment(&t.id, "note to self", "wake", &[], &[])
            .unwrap();
    assert!(res.told.mode.is_empty() && res.told.to.is_empty());
    assert!(res.answers.is_empty() && res.watching.is_empty());
    assert_eq!(r.w.card(&t.id).unwrap().comments.len(), 1);
}

/// Card #50: the person answers with a question, the raiser answers it and
/// revises the blocker. The revision shows in the blocker's thread under
/// the person's reply, and the person picks from the current options.
#[test]
fn revised_blocker_shows_in_its_thread() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let t = held_task(&r);
    let bl = raise(&r, &r.key_a, &t.id, "Which DB?", &["postgres", "mysql"]);
    r.w.answer(&t.id, &bl.id, -1, "Which one is cheaper to run?", true, &[])
        .unwrap();
    co.comment_on(
        &r.key_a,
        &t.id,
        coord::CommentInput {
            text: "Postgres on the small plan is cheapest.".into(),
            blocker_id: bl.id.clone(),
            ..Default::default()
        },
    )
    .unwrap();
    co.revise(
        &r.key_a,
        &t.id,
        &bl.id,
        coord::BlockerRevision {
            question: "Postgres on the small plan?".into(),
            options: vec![
                coord::BlockerOption {
                    answer: "Yes".into(),
                    recommended: true,
                    ..Default::default()
                },
                coord::BlockerOption {
                    answer: "No, a bigger plan".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
    )
    .unwrap();
    let d = r.w.card(&t.id).unwrap();
    let b = blocker(&d, &bl.id);
    assert_eq!(b.question, "Postgres on the small plan?");
    let answers: Vec<&str> = b.options.iter().map(|o| o.answer.as_str()).collect();
    assert_eq!(answers, ["Yes", "No, a bigger plan"]);
    assert_eq!(b.state, BLOCKER_OPEN);
    assert_eq!(d.summary.open_blockers, 1);
    assert_eq!(b.thread.len(), 3, "{:?}", b.thread);
    assert_eq!(b.thread[0].who.name, "You");
    assert_eq!(b.thread[1].who.key, r.key_a);
    let rev = &b.thread[2];
    assert!(rev.revised && rev.kind == THREAD_EVENT, "{rev:?}");
    assert_eq!(rev.who.key, r.key_a);
    assert!(
        rev.text.contains("revised it: asks now")
            && !rev.text.contains(&bl.id)
            && rev.text.contains("Postgres on the small plan?")
            && rev
                .text
                .contains("options now: Yes (recommended) | No, a bigger plan"),
        "{}",
        rev.text
    );
    assert!(
        !d.thread.iter().any(|i| i.revised),
        "the revision is in the card's own thread too"
    );
    let raw = serde_json::to_string(b).unwrap();
    assert!(raw.contains("\"revised\":true"), "{raw}");
    // The pick is from the revised options.
    let res = r.w.answer(&t.id, &bl.id, 0, "", true, &[]).unwrap();
    let b = blocker(&res.card, &bl.id);
    assert_eq!(b.state, BLOCKER_ANSWERED);
    assert_eq!(b.thread.last().unwrap().text, "Yes");
}

/// Card #93: a card dropped on the canvas keeps its spot, on the summary.
#[test]
fn cards_carry_their_canvas_spot() {
    let r = Rig::new();
    let (a, b) = (held_task(&r), another_held_task(&r));
    r.w.place(&a.id, 340, -60).unwrap();
    let cs = r.w.cards();
    let get = |id: &str| cs.iter().find(|s| s.id == id).unwrap().clone();
    assert_eq!(
        get(&a.id).pos,
        Some(irori_window::cards::CanvasPos { x: 340, y: -60 })
    );
    assert_eq!(get(&b.id).pos, None);
    assert!(r.w.place("01NOPE", 0, 0).is_err());
}

/// Card #98: an agent gone from the roster (a Grok member after a restart,
/// say) shows by the last name it went by, in Who's on it and in the
/// thread's lines, never by its raw key.
#[test]
fn a_gone_agent_keeps_its_name() {
    let gone = "c99db25c/grok-b7494dc724f6";
    let now = unix_ms() / 1000;
    let r = Rig::with(|o| {
        std::fs::write(
            o.data_dir.join(irori_core::agentnames::FILE),
            serde_json::json!({ gone: {"name": "juniper", "at": now} }).to_string(),
        )
        .unwrap();
    });
    let co = r.w.core.coord();
    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "hand it on".into(),
                ..coord::NewTask::default()
            },
        )
        .unwrap();
    co.assign(&r.key_a, &t.id, gone).unwrap();
    let d = r.w.card(&t.id).unwrap();
    let w = d
        .summary
        .working
        .iter()
        .find(|w| w.key == gone)
        .expect("on the card");
    assert_eq!((w.name.as_str(), w.status.as_str()), ("juniper", "gone"));
    assert!(
        d.recent
            .iter()
            .any(|a| a.text.ends_with("assigned it to juniper")),
        "{:?}",
        d.recent
    );
    assert!(
        d.thread.iter().all(|e| !e.text.contains(gone)),
        "{:?}",
        d.thread
    );
}

/// Card #105: Modify holds the person's comment from its agent, Save sends
/// the new text instead, and once the agent has it Modify is gone.
#[test]
fn modify_a_comment_before_it_is_read() {
    let r = Rig::new();
    let t = held_task(&r);
    let ib = r.w.core.inbox();
    let res =
        r.w.comment(&t.id, "use the red one", "next_turn", &[], &[])
            .unwrap();
    let cid = res.comment.id.clone();
    let item = |r: &Rig| {
        r.w.card(&t.id)
            .unwrap()
            .thread
            .into_iter()
            .find(|it| it.id == cid)
            .unwrap()
    };
    assert_eq!(item(&r).edit, "can");

    r.w.comment_modify(&t.id, &cid).unwrap();
    assert_eq!(item(&r).edit, "editing");
    assert!(
        ib.next_append(&r.key_a).unwrap().is_none(),
        "held while editing"
    );

    r.w.comment_modify_save(&t.id, &cid, "use the blue one")
        .unwrap();
    let it = item(&r);
    assert_eq!(
        (it.text.as_str(), it.edited, it.edit.as_str()),
        ("use the blue one", true, "can")
    );
    let w = ib
        .next_append(&r.key_a)
        .unwrap()
        .expect("the new text goes out");
    let got: Vec<String> = w
        .delivery_ids
        .iter()
        .map(|id| ib.get(id).unwrap().message.text)
        .collect();
    assert_eq!(got.len(), 1);
    assert!(
        got[0].contains("use the blue one") && !got[0].contains("red"),
        "{got:?}"
    );
    ib.mark_appended_delivered(&r.key_a, &w.token).unwrap();

    // Alpha has it now: no Modify, and asking anyway is refused.
    assert_eq!(item(&r).edit, "");
    assert!(r.w.comment_modify(&t.id, &cid).is_err());
    assert!(r.w.comment_modify_save(&t.id, &cid, "green").is_err());
    assert_eq!(item(&r).text, "use the blue one");
}

/// The person turns "Offer it cards" on for both of the rig's terminal
/// agents (card #453), and the roster says so.
fn offer_both(r: &Rig) {
    for k in [&r.key_a, &r.key_b] {
        r.w.agents_offer_set(k, true).unwrap();
    }
    let agents = r.w.agents();
    for k in [&r.key_a, &r.key_b] {
        let a = agents.iter().find(|a| &a.card.key == k).unwrap();
        assert!(a.offers_cards && !a.spawned, "{k}");
    }
}

/// Card #116: New card makes a card on the person's own board. Unpicked,
/// Irori hands it to a free agent; picked, it goes to that agent. Card
/// #453: a terminal agent is free only with "Offer it cards" on.
#[test]
fn new_card_from_the_app() {
    use irori_window::new_card::NewCardInput;
    let r = Rig::new();
    let unoffered =
        r.w.create_card(NewCardInput {
            title: "Nobody takes this".into(),
            ..Default::default()
        })
        .unwrap();
    assert!(
        unoffered.told.to.is_empty(),
        "terminal agents without Offer it cards aren't handed it: {:?}",
        unoffered.told
    );
    offer_both(&r);
    assert!(
        r.w.create_card(NewCardInput::default()).is_err(),
        "a title is needed"
    );
    let res =
        r.w.create_card(NewCardInput {
            title: "  Tidy the logs ".into(),
            objective: "They grow forever".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(res.task.title, "Tidy the logs");
    assert_eq!(res.task.objective, "They grow forever");
    assert!(res.task.ask.is_none(), "not an ask");
    assert_eq!(
        res.told.to.len(),
        1,
        "handed to one free agent: {:?}",
        res.told
    );

    let res =
        r.w.create_card(NewCardInput {
            title: "Review the PR".into(),
            assign_to: r.key_b.clone(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(res.told.to, vec![r.key_b.clone()]);
    assert_eq!(res.told.mode, "wake");
    assert!(res.task.assigned.contains(&r.key_b));
}

/// Card #116: the header box starts a thread. It stays off the grid, shows
/// in the Threads panel, goes to the agent on the card it names, and the
/// person resolves it.
#[test]
fn a_thread_from_the_header_box() {
    let r = Rig::new();
    let t = held_task(&r);
    let res =
        r.w.start_thread(&format!("Is #{} still moving?", t.short), &[], &[], "")
            .unwrap();
    let id = res.task.id.clone();
    assert_eq!(
        res.told.to,
        vec![r.key_a.clone()],
        "alpha holds the card it names"
    );
    assert!(r.w.cards().iter().all(|c| c.id != id), "not in the grid");
    let ts = r.w.threads("");
    assert_eq!(ts.len(), 1);
    assert_eq!((ts[0].state.as_str(), ts[0].told.len()), ("waiting", 1));

    r.w.core
        .coord()
        .add_comment(&r.key_a, &id, "Yes, half done.", false)
        .unwrap();
    let ts = r.w.threads("");
    assert_eq!(ts[0].state, "answered");
    assert_eq!(ts[0].last_text, "Yes, half done.");
    let c = r.w.comment(&id, "Thanks", "wake", &[], &[]).unwrap();
    assert_eq!(
        c.told.to,
        vec![r.key_a.clone()],
        "follow-up goes to who answered"
    );

    r.w.resolve_thread(&id).unwrap();
    assert_eq!(r.w.threads("")[0].state, "resolved");
    let done = r.w.done_cards(0, "").unwrap();
    assert!(
        done.cards.iter().all(|c| c.id != id),
        "not in Closed either"
    );
}

/// Card #124: files go with a thread, a reply in it, and a new card; the
/// agent that hears each gets them with the message, for attachment_get.
#[test]
fn files_with_a_thread_a_reply_and_a_new_card() {
    use base64::Engine as _;
    let r = Rig::new();
    offer_both(&r);
    let t = held_task(&r);
    let ib = r.w.core.inbox();
    let up = |name: &str, body: &[u8]| irori_window::board::UploadFile {
        name: name.into(),
        data: base64::engine::general_purpose::STANDARD.encode(body),
    };
    // The file names on each message alpha got since the last look.
    let seen = std::cell::RefCell::new(std::collections::HashSet::new());
    let heard = |r: &Rig| -> Vec<Vec<String>> {
        let mut ds = ib.for_agent(&r.key_a, true, 100);
        ds.reverse();
        ds.into_iter()
            .filter(|d| seen.borrow_mut().insert(d.id.clone()))
            .map(|d| {
                d.message
                    .attachments
                    .iter()
                    .map(|a| a.name.clone())
                    .collect()
            })
            .collect()
    };
    let files_on = |r: &Rig, id: &str| -> Vec<String> {
        r.w.card(id)
            .unwrap()
            .comments
            .iter()
            .flat_map(|c| c.attachments.iter().map(|a| a.name.clone()))
            .collect()
    };

    let res =
        r.w.start_thread(
            &format!("What is this on #{}?", t.short),
            &[],
            &[up("shot.png", b"\x89PNG one")],
            "",
        )
        .unwrap();
    let id = res.task.id.clone();
    assert_eq!(res.told.to, vec![r.key_a.clone()]);
    assert_eq!(files_on(&r, &id), vec!["shot.png"]);
    assert_eq!(heard(&r), vec![vec!["shot.png".to_string()]]);

    r.w.comment(&id, "and this one", "wake", &[], &[up("log.txt", b"boom")])
        .unwrap();
    assert_eq!(files_on(&r, &id), vec!["shot.png", "log.txt"]);
    assert_eq!(heard(&r), vec![vec!["log.txt".to_string()]]);
    let sha =
        r.w.card(&id)
            .unwrap()
            .comments
            .iter()
            .flat_map(|c| c.attachments.clone())
            .find(|a| a.name == "log.txt")
            .unwrap()
            .sha256;
    assert!(r.w.attachment_info(&sha).unwrap().here);

    let res =
        r.w.create_card(irori_window::new_card::NewCardInput {
            title: "Look at the crash".into(),
            assign_to: r.key_a.clone(),
            files: vec![up("crash.txt", b"trace")],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(files_on(&r, &res.task.id), vec!["crash.txt"]);
    assert_eq!(heard(&r), vec![vec!["crash.txt".to_string()]]);

    // The thread went at once to the other free agent (card #512: nobody
    // waits on alpha); resolved, it frees that agent for the card below.
    r.w.resolve_thread(&id).unwrap();

    // Nobody picked: Irori offers it to a free agent, files and all.
    let res =
        r.w.create_card(irori_window::new_card::NewCardInput {
            title: "And this one".into(),
            files: vec![up("second.txt", b"more")],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(files_on(&r, &res.task.id), vec!["second.txt"]);
    let to = res.told.to.clone();
    assert_eq!(to.len(), 1, "offered to one free agent: {:?}", res.told);
    let got: Vec<Vec<String>> = ib
        .for_agent(&to[0], true, 100)
        .into_iter()
        .filter(|d| d.message.text.contains("And this one"))
        .map(|d| {
            d.message
                .attachments
                .iter()
                .map(|a| a.name.clone())
                .collect()
        })
        .collect();
    assert_eq!(got, vec![vec!["second.txt".to_string()]]);
}

/// Card #169: Reopen brings a resolved thread back as a live one, from the
/// Threads panel or the card page's Reopen, and a reply goes to who
/// answered it.
#[test]
fn a_resolved_thread_reopens_from_the_window() {
    let r = Rig::new();
    let t = held_task(&r);
    let id =
        r.w.start_thread(&format!("Is #{} still moving?", t.short), &[], &[], "")
            .unwrap()
            .task
            .id;
    r.w.core
        .coord()
        .add_comment(&r.key_a, &id, "Yes, half done.", false)
        .unwrap();
    r.w.resolve_thread(&id).unwrap();
    r.w.reopen_thread(&id).unwrap();
    assert_eq!(r.w.threads("")[0].state, "answered", "live again");
    assert!(
        r.w.cards().iter().all(|c| c.id != id),
        "still not in the grid"
    );
    let c =
        r.w.comment(&id, "One more thing", "wake", &[], &[])
            .unwrap();
    assert_eq!(c.told.to, vec![r.key_a.clone()]);

    // The card page's Reopen does the same for a resolved thread.
    r.w.resolve_thread(&id).unwrap();
    r.w.reopen(&id).unwrap();
    assert_eq!(r.w.threads("")[0].state, "answered");
}

/// Card #144: a #ref in text names a done card or a resolved thread too,
/// by its short ref, so the window links it without the done list.
#[test]
fn card_ref_finds_done_cards_and_threads() {
    let r = Rig::new();
    let t = held_task(&r);
    r.w.close_card(&t.id, "completed", "").unwrap();
    let got = r.w.card_ref(&format!("#{}", t.short)).expect("a done card");
    assert_eq!(
        (got.id.as_str(), got.thread, got.done),
        (t.id.as_str(), false, true)
    );
    assert_eq!(got.title, "ship it");

    let th =
        r.w.start_thread("Where do the builds go?", &[], &[], "")
            .unwrap()
            .task;
    let got = r.w.card_ref(&format!("#{}", th.short)).expect("a thread");
    assert_eq!(
        (got.id.as_str(), got.thread, got.done),
        (th.id.as_str(), true, false)
    );
    r.w.resolve_thread(&th.id).unwrap();
    assert!(r.w.card_ref(&th.short).unwrap().done, "resolved");

    assert!(r.w.card_ref("#9999").is_none(), "nothing has it");
    assert!(r.w.card_ref("not a ref").is_none());
}

/// Card #343: a ref's chip shows the card's state live: working, then
/// blocked on the person, then done, with its steps' progress.
#[test]
fn card_ref_carries_state_blocked_and_steps() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let t = held_task(&r);
    co.set_steps(
        &r.key_a,
        &t.id,
        vec![
            coord::Step {
                text: "one".into(),
                done: true,
                ..Default::default()
            },
            coord::Step {
                text: "two".into(),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    let at = format!("#{}", t.short);
    let got = r.w.card_ref(&at).unwrap();
    assert_eq!(
        (
            got.state.as_str(),
            got.blocked,
            got.progress.done,
            got.progress.total
        ),
        ("working", false, 1, 2)
    );
    co.block(&r.key_a, &t.id, "Which DB?", None).unwrap();
    assert!(r.w.card_ref(&at).unwrap().blocked, "blocked on the person");
    r.w.close_card(&t.id, "completed", "").unwrap();
    let got = r.w.card_ref(&at).unwrap();
    assert_eq!((got.state.as_str(), got.done), ("completed", true));
}

/// Card #158: a blocker on another card waits by itself: it is a wait
/// ("Waiting on #N"), not a question for the person, and the tray leaves
/// it out; a question for the person still counts.
#[test]
fn a_wait_on_another_card_is_no_decision() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let first = held_task(&r);
    let t = another_held_task(&r);
    co.raise(
        &r.key_a,
        &t.id,
        coord::NewBlocker {
            question: "Needs the save format from the other card".into(),
            blocked_by: Some(coord::BlockedBy {
                kind: coord::BLOCKED_ON_TASK.into(),
                reference: first.id.clone(),
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let d = r.w.card(&t.id).unwrap();
    assert_eq!(d.summary.state, coord::BLOCKED);
    assert_eq!((d.summary.open_blockers, d.summary.waiting_on), (0, 1));
    assert_eq!(d.summary.wait_on, first.short_ref());
    assert_eq!(
        d.summary.wait_question,
        "Needs the save format from the other card"
    );
    let s = r.w.cards().into_iter().find(|c| c.id == t.id).unwrap();
    assert_eq!(s.wait_on, first.short_ref());
    assert_eq!(r.w.decide_count(), 0, "a wait on a card is no decision");

    raise(&r, &r.key_a, &t.id, "Which DB?", &["postgres"]);
    let d = r.w.card(&t.id).unwrap();
    assert_eq!((d.summary.open_blockers, d.summary.waiting_on), (1, 1));
    assert_eq!(d.summary.blocker_question, "Which DB?");
    assert_eq!(r.w.decide_count(), 1);
}

/// Card #347: what waits on the person says so. A question with options is
/// a decision whatever it's filed on (#290 was "on external"); a card
/// waiting on a card with the person's question names it. Card #471: a card
/// in review on an open PR waits on that PR, never on the person's review,
/// and a card waiting on it doesn't name the person either.
#[test]
fn what_waits_on_the_person_says_so() {
    const PR: &str = "https://github.com/a/suzuri/pull/96";
    let r = Rig::new();
    let co = r.w.core.coord();
    let summary = |id: &str| r.w.cards().into_iter().find(|c| c.id == id).unwrap();
    let wait_on = |by: &str, id: &str, on: &str| {
        co.raise(
            by,
            id,
            coord::NewBlocker {
                question: "after it".into(),
                blocked_by: Some(coord::BlockedBy {
                    kind: coord::BLOCKED_ON_TASK.into(),
                    reference: on.into(),
                }),
                ..Default::default()
            },
        )
        .unwrap()
    };
    let take = |by: &str, title: &str| {
        let t = co
            .create_task(
                &me(&r),
                coord::NewTask {
                    title: title.into(),
                    ..Default::default()
                },
            )
            .unwrap();
        co.assign(&me(&r), &t.id, by).unwrap();
        co.claim_task(by, &t.id).unwrap()
    };

    // #290: options on an external wait.
    let ext = held_task(&r);
    co.raise(
        &r.key_a,
        &ext.id,
        coord::NewBlocker {
            question: "Park until stable, or swap now?".into(),
            options: ["Park", "Swap now"]
                .iter()
                .map(|a| coord::BlockerOption {
                    answer: (*a).into(),
                    ..Default::default()
                })
                .collect(),
            blocked_by: Some(coord::BlockedBy {
                kind: coord::BLOCKED_ON_EXTERNAL.into(),
                reference: "https://github.com/ghostty-org/ghostty".into(),
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let s = summary(&ext.id);
    assert_eq!((s.open_blockers, s.waiting_on), (1, 0));
    assert_eq!(s.blocker_question, "Park until stable, or swap now?");
    assert_eq!(r.w.decide_count(), 1);

    // #334 and #329: a card waits on one that then goes in review.
    let pr = take(&r.key_b, "installers");
    let after = take(&r.key_a, "move it");
    wait_on(&r.key_a, &after.id, &pr.id);
    co.add_link(&r.key_b, &pr.id, PR).unwrap();
    co.set_task_state(&r.key_b, &pr.id, coord::IN_REVIEW, "")
        .unwrap();
    let s = summary(&pr.id);
    let wp = s.wait_pr.expect("waits on its PR");
    assert_eq!((wp.number, wp.url.as_str()), (96, PR));
    assert_eq!(
        summary(&after.id).wait_you,
        "",
        "a PR never waits on the person"
    );

    // A card waiting on #290's question names the person's answer.
    let on_ext = take(&r.key_b, "after the swap");
    wait_on(&r.key_b, &on_ext.id, &ext.id);
    assert_eq!(
        summary(&on_ext.id).wait_you,
        format!("your answer on {}", ext.short_ref())
    );

    // None of the person's own agents may review it: it still waits on
    // the PR.
    let e = co
        .request_review(&r.key_b, &pr.id, PR, &r.key_a)
        .unwrap_err();
    assert!(e.is(coord::ErrorKind::NotAllowed), "{e}");
    assert_eq!(summary(&pr.id).wait_pr.map(|p| p.number), Some(96));
    assert_eq!(summary(&after.id).wait_you, "");

    // Merged: nothing to wait on.
    co.record_pr_state(
        &pr.id,
        coord::PrCheck {
            pr: PR.into(),
            state: coord::PR_MERGED.into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(summary(&pr.id).wait_pr.is_none());
}

/// Card #509: an agent's external wait on a PR (#454's "Review and merge
/// PR #41") is the person's merge: the card carries the PR for Home's
/// Needs you and counts in the tray, until the PR is heard merged. A wait
/// on CI stays a wait.
#[test]
fn a_wait_on_a_prs_merge_is_the_persons() {
    const PR: &str = "https://github.com/a/irori/pull/41";
    let r = Rig::new();
    let co = r.w.core.coord();
    let summary = |id: &str| r.w.cards().into_iter().find(|c| c.id == id).unwrap();
    let wait = |title: &str, on: &str| {
        let t = co
            .create_task(
                &r.key_a,
                coord::NewTask {
                    title: title.into(),
                    ..Default::default()
                },
            )
            .unwrap();
        co.claim_task(&r.key_a, &t.id).unwrap();
        co.block(
            &r.key_a,
            &t.id,
            "Review and merge PR #41 so part 10 lands on main",
            Some(coord::BlockedBy {
                kind: coord::BLOCKED_ON_EXTERNAL.into(),
                reference: on.into(),
            }),
        )
        .unwrap()
    };
    let ci = wait("release", "CI on the release");
    assert!(summary(&ci.id).merge_pr.is_none());
    assert_eq!(r.w.decide_count(), 0, "a wait on CI is no decision");

    let t = wait("part 10", PR);
    let s = summary(&t.id);
    let m = s.merge_pr.expect("the person's merge");
    assert_eq!(
        (m.repo.as_str(), m.number, m.url.as_str()),
        ("a/irori", 41, PR)
    );
    assert_eq!((s.open_blockers, s.waiting_on), (0, 1));
    assert_eq!(r.w.decide_count(), 1);

    co.record_pr_state(
        &t.id,
        coord::PrCheck {
            pr: PR.into(),
            state: coord::PR_MERGED.into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(summary(&t.id).merge_pr.is_none(), "merged");
    assert_eq!(r.w.decide_count(), 0);
}

/// Card #481: a wait on another card (#394's "Waits on Talk 4 (#392)")
/// is drawn as a wait with what it waits on, never as a question, and is
/// never counted as a decision; one filed on a card with options is still
/// the person's question (card #347).
#[test]
fn a_wait_on_a_card_is_never_a_question() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let talk4 = co
        .create_task(
            &me(&r),
            coord::NewTask {
                title: "Talk 4: Spaces".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let t = held_task(&r);
    let on = |kind: &str, reference: &str| {
        Some(coord::BlockedBy {
            kind: kind.into(),
            reference: reference.into(),
        })
    };
    let (_, b1) = co
        .raise(
            &r.key_a,
            &t.id,
            coord::NewBlocker {
                question: format!("Waits on Talk 4 ({})", talk4.short_ref()),
                blocked_by: on(coord::BLOCKED_ON_TASK, &talk4.id),
                ..Default::default()
            },
        )
        .unwrap();
    let (_, b2) = co
        .raise(
            &r.key_a,
            &t.id,
            coord::NewBlocker {
                question: "after beta".into(),
                blocked_by: on(coord::BLOCKED_ON_AGENT, &r.key_b),
                ..Default::default()
            },
        )
        .unwrap();
    let d = r.w.card(&t.id).unwrap();
    let w = blocker(&d, &b1.id).waits_on.clone().expect("a wait");
    assert_eq!(
        (
            w.kind.as_str(),
            w.label.clone(),
            w.task_id.as_str(),
            w.title.as_str()
        ),
        (
            coord::BLOCKED_ON_TASK,
            talk4.short_ref(),
            talk4.id.as_str(),
            "Talk 4: Spaces"
        )
    );
    let w = blocker(&d, &b2.id).waits_on.clone().expect("a wait");
    assert_eq!(w.kind, coord::BLOCKED_ON_AGENT);
    assert!(
        !w.label.is_empty() && w.label != r.key_b,
        "named, not a key: {}",
        w.label
    );
    assert_eq!((d.summary.open_blockers, d.summary.waiting_on), (0, 2));
    assert_eq!(r.w.decide_count(), 0);

    // With options it is the person's to decide, whatever it's filed on.
    let (_, b3) = co
        .raise(
            &r.key_a,
            &t.id,
            coord::NewBlocker {
                question: "Wait for Talk 4, or ship without it?".into(),
                options: ["Wait", "Ship"]
                    .iter()
                    .map(|a| coord::BlockerOption {
                        answer: (*a).into(),
                        ..Default::default()
                    })
                    .collect(),
                blocked_by: on(coord::BLOCKED_ON_TASK, &talk4.id),
                ..Default::default()
            },
        )
        .unwrap();
    let d = r.w.card(&t.id).unwrap();
    assert!(blocker(&d, &b3.id).waits_on.is_none());
    assert!(d.summary.wait_loop.is_empty() && d.summary.wait_stuck.is_none());
    assert_eq!((d.summary.open_blockers, d.summary.waiting_on), (1, 2));
    assert_eq!(r.w.decide_count(), 1);
}

/// Card #481: #392 waited on #396, #396 on #394 and #394 on #392. Each
/// card's summary names the same loop, none of it counts as a decision,
/// and it's gone once one of the cards is done.
#[test]
fn cards_that_wait_on_each_other_name_the_loop() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let summary = |id: &str| r.w.cards().into_iter().find(|c| c.id == id).unwrap();
    let card = |title: &str| {
        co.create_task(
            &me(&r),
            coord::NewTask {
                title: title.into(),
                ..Default::default()
            },
        )
        .unwrap()
    };
    let wait_on = |id: &str, on: &str| {
        co.raise(
            &me(&r),
            id,
            coord::NewBlocker {
                question: "after it".into(),
                blocked_by: Some(coord::BlockedBy {
                    kind: coord::BLOCKED_ON_TASK.into(),
                    reference: on.into(),
                }),
                ..Default::default()
            },
        )
        .unwrap()
    };
    let (t4, t6, t8) = (card("Talk 4"), card("Talk 6"), card("Talk 8"));
    wait_on(&t4.id, &t8.id);
    wait_on(&t8.id, &t6.id);
    assert!(summary(&t4.id).wait_loop.is_empty());
    wait_on(&t6.id, &t4.id);

    let ids =
        |id: &str| -> Vec<String> { summary(id).wait_loop.iter().map(|c| c.id.clone()).collect() };
    let ring = ids(&t4.id);
    assert_eq!(ring.len(), 3, "{ring:?}");
    for t in [&t4, &t6, &t8] {
        assert_eq!(ids(&t.id), ring, "{}", t.title);
        let s = summary(&t.id);
        assert_eq!((s.open_blockers, s.waiting_on), (0, 1), "{}", t.title);
        assert!(s.wait_stuck.is_none());
    }
    let s = summary(&t4.id);
    let named = s.wait_loop.iter().find(|c| c.id == t6.id).unwrap();
    assert_eq!(
        (named.short.as_str(), named.title.as_str()),
        (t6.short.as_str(), "Talk 6")
    );
    assert_eq!(r.w.decide_count(), 0);
    // The card page carries it too.
    assert_eq!(r.w.card(&t6.id).unwrap().summary.wait_loop.len(), 3);

    co.set_task_state(&me(&r), &t6.id, coord::CANCELED, "not now")
        .unwrap();
    for t in [&t4, &t8] {
        assert!(ids(&t.id).is_empty(), "{}", t.title);
    }
}

/// Card #138: clearing done cards takes them out of Closed (by id, by age
/// or all); they stay in the log, and a reopened one is back on the grid.
#[test]
fn clearing_done_cards() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let me = r.w.core.user_sender().key;
    let mk = |title: &str| {
        let t = co
            .create_task(
                &r.key_a,
                coord::NewTask {
                    title: title.into(),
                    ..Default::default()
                },
            )
            .unwrap();
        co.set_task_state(&me, &t.id, coord::CANCELED, "").unwrap();
        t
    };
    let a = mk("a");
    let b = mk("b");
    let c = mk("c");
    let closed = || -> Vec<String> {
        r.w.done_cards(0, "")
            .unwrap()
            .cards
            .into_iter()
            .map(|c| c.id)
            .collect()
    };
    assert_eq!(closed().len(), 3);
    assert_eq!(r.w.clear_done_count(3600, ""), 0, "none older than an hour");
    assert_eq!(r.w.clear_done(3600, &[], "").unwrap(), 0);
    assert_eq!(
        r.w.clear_done(0, std::slice::from_ref(&a.id), "").unwrap(),
        1
    );
    assert!(!closed().contains(&a.id));
    assert!(co.get_task(&a.id).is_some(), "show still finds it");
    assert_eq!(r.w.clear_done_count(0, ""), 2);
    assert_eq!(r.w.clear_done(0, &[], "").unwrap(), 2);
    assert!(closed().is_empty());
    r.w.reopen(&b.id).unwrap();
    assert!(
        r.w.cards().iter().any(|x| x.id == b.id),
        "a reopened card is back on the grid"
    );
    let _ = c;
}

/// Card #195: files go with a blocker answer and with a message Reply; the
/// agent that hears each gets them, and the card's question takes none.
#[test]
fn files_with_a_blocker_answer_and_a_message_reply() {
    use base64::Engine as _;
    let r = Rig::new();
    let t = held_task(&r);
    let ib = r.w.core.inbox();
    let up = |name: &str, body: &[u8]| irori_window::board::UploadFile {
        name: name.into(),
        data: base64::engine::general_purpose::STANDARD.encode(body),
    };
    let newest_files = |r: &Rig| -> Vec<String> {
        ib.for_agent(&r.key_a, true, 1)
            .first()
            .map(|d| {
                d.message
                    .attachments
                    .iter()
                    .map(|a| a.name.clone())
                    .collect()
            })
            .unwrap_or_default()
    };

    let b = raise(&r, &r.key_a, &t.id, "Which build?", &["debug", "release"]);
    r.w.answer_files(
        &t.id,
        &b.id,
        -1,
        "this one, see the log",
        true,
        &[],
        &[up("build.log", b"error: boom")],
    )
    .unwrap();
    let d = r.w.card(&t.id).unwrap();
    assert_eq!(blocker(&d, &b.id).state, "answered");
    let on: Vec<String> = d
        .comments
        .iter()
        .flat_map(|c| c.attachments.iter().map(|a| a.name.clone()))
        .collect();
    assert_eq!(on, vec!["build.log"]);
    assert_eq!(newest_files(&r), vec!["build.log"]);

    // A pick carries files too.
    let b2 = raise(&r, &r.key_a, &t.id, "Ship it?", &["yes", "no"]);
    r.w.answer_files(
        &t.id,
        &b2.id,
        0,
        "",
        true,
        &[],
        &[up("shot.png", b"\x89PNG")],
    )
    .unwrap();
    assert_eq!(newest_files(&r), vec!["shot.png"]);

    // A message Reply carries them to its sender.
    r.w.inbox_send(&r.key_a, "", "hello alpha", &[]).unwrap();
    let sent =
        r.w.inbox_list(0)
            .into_iter()
            .find(|d| d.to == r.key_a)
            .unwrap();
    let alpha = inbox::Sender::new(&r.key_a, "alpha", "agent");
    let (_, d) = ib.reply(&alpha, &sent.id, "a question back").unwrap();
    r.w.inbox_reply_files(&d.id, "here it is", &[], &[up("notes.md", b"# notes")])
        .unwrap();
    assert_eq!(r.w.inbox_get(&d.id).unwrap().state, "answered");
    assert_eq!(newest_files(&r), vec!["notes.md"]);
}

/// Card #485: an item whose agent is gone, on a card nobody holds, says
/// so (goneAgent) and its answer goes out with the card, not to nobody:
/// #453's "Is this done?" raised by piranha, after piranha went.
#[test]
fn a_not_done_on_a_gone_agents_check_goes_to_the_next_agent() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let gone = format!("{}/piranha-session", r.w.core.device_id());
    let t = co
        .create_task(
            &me(&r),
            coord::NewTask {
                title: "terminal agents".into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.claim_task(&gone, &t.id).unwrap();
    co.watch(&gone, &t.id).ok();
    co.watch(&r.key_b, &t.id).unwrap();
    let got = co
        .set_task_state(&gone, &t.id, coord::COMPLETED, "on main at 8845c11a")
        .unwrap();
    let check = got
        .blockers
        .iter()
        .find(|b| b.done_check)
        .expect("a done check")
        .id
        .clone();
    // While piranha holds it, nothing is marked.
    assert!(
        blocker(&r.w.card(&t.id).unwrap(), &check)
            .gone_agent
            .is_none()
    );
    co.leave(&gone, &t.id, &gone).unwrap();
    let d = r.w.card(&t.id).unwrap();
    let b = blocker(&d, &check);
    assert_eq!(
        b.gone_agent.as_ref().map(|a| a.key.as_str()),
        Some(gone.as_str())
    );
    // A gone watcher is left out; a live one stays.
    let ws: Vec<&str> = d.summary.watching.iter().map(|w| w.key.as_str()).collect();
    assert_eq!(ws, [r.key_b.as_str()]);

    let res =
        r.w.answer(
            &t.id,
            &check,
            -1,
            "Not done: the others haven't been told the next steps",
            false,
            &[],
        )
        .unwrap();
    // Not resolved in place: the card is back on the Board, and the
    // person's words are on the check's thread for the next agent's brief.
    assert_eq!(res.card.summary.state, coord::SUBMITTED);
    let cs = co.comments(&t.id).unwrap_or_default();
    assert!(
        cs.iter()
            .any(|c| c.blocker_id == check && c.by == me(&r) && c.text.contains("next steps")),
        "{cs:?}"
    );
}

/// Card #485: "Hand to an agent" on a wait whose agent is gone (#445
/// waited on kestrel): the card goes back on the Board, what was open
/// said on it.
#[test]
fn hand_on_a_wait_on_a_gone_agent() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let kestrel = format!("{}/kestrel-session", r.w.core.device_id());
    let t = held_task(&r);
    co.raise(
        &r.key_a,
        &t.id,
        coord::NewBlocker {
            question: "Waiting on kestrel's no-daemon GUI spawn".into(),
            blocked_by: Some(coord::BlockedBy {
                kind: coord::BLOCKED_ON_AGENT.into(),
                reference: kestrel.clone(),
            }),
            ..Default::default()
        },
    )
    .unwrap();
    // Alpha holds it: not stranded, and Hand to an agent is refused.
    assert!(r.w.hand_on(&t.id).is_err());
    co.leave(&r.key_a, &t.id, &r.key_a).unwrap();
    let d = r.w.card(&t.id).unwrap();
    let b = blocker(&d, "b1");
    let w = b.waits_on.as_ref().expect("a wait");
    assert!(w.gone);
    assert_eq!(b.gone_agent.as_ref().unwrap().key, kestrel);
    let said = r.w.hand_on(&t.id).unwrap();
    assert!(said.contains("back on the Board"), "{said}");
    let d = r.w.card(&t.id).unwrap();
    assert_eq!(d.summary.state, coord::SUBMITTED);
    assert!(d.blockers.iter().all(|b| b.gone_agent.is_none()));
}

/// Card #520: a wait no agent will end is a Needs you row (goneWait) and
/// counts in the tray: #73 waits on kestrel (gone), #75's outside wait
/// was raised by piranha (gone). A wait on a PR stays #509's merge row,
/// once. Handed on, it leaves.
#[test]
fn a_wait_on_a_gone_agent_is_the_persons() {
    const PR: &str = "https://github.com/a/irori/pull/41";
    let r = Rig::new();
    let co = r.w.core.coord();
    let kestrel = format!("{}/kestrel-session", r.w.core.device_id());
    let piranha = format!("{}/piranha-session", r.w.core.device_id());
    let summary = |id: &str| r.w.cards().into_iter().find(|c| c.id == id).unwrap();
    let wait = |by: &str, title: &str, kind: &str, on: &str, q: &str| {
        let t = co
            .create_task(
                &me(&r),
                coord::NewTask {
                    title: title.into(),
                    ..Default::default()
                },
            )
            .unwrap();
        co.claim_task(by, &t.id).unwrap();
        co.block(
            by,
            &t.id,
            q,
            Some(coord::BlockedBy {
                kind: kind.into(),
                reference: on.into(),
            }),
        )
        .unwrap();
        t
    };
    let t73 = wait(
        &r.key_a,
        "the spawner",
        coord::BLOCKED_ON_AGENT,
        &kestrel,
        "Waiting on kestrel's no-daemon GUI spawn",
    );
    // Alpha holds it: its wait is alpha's to drop, not the person's.
    assert!(summary(&t73.id).gone_wait.is_none());
    assert_eq!(r.w.decide_count(), 0);
    co.leave(&r.key_a, &t73.id, &r.key_a).unwrap();
    let g = summary(&t73.id).gone_wait.expect("a gone wait");
    assert_eq!(g.agent.key, kestrel);
    assert!(g.waited_on);
    assert_eq!(g.on, "");
    assert_eq!(g.question, "Waiting on kestrel's no-daemon GUI spawn");
    assert_eq!(g.more, 0);
    assert_eq!(g.no_hand_on, "", "Hand to an agent can move it");
    assert_eq!(r.w.decide_count(), 1);

    let t75 = wait(
        &piranha,
        "release",
        coord::BLOCKED_ON_EXTERNAL,
        "CI on the release",
        "Waiting on CI",
    );
    co.leave(&piranha, &t75.id, &piranha).unwrap();
    let g = summary(&t75.id)
        .gone_wait
        .expect("a gone raiser's outside wait");
    assert_eq!(g.agent.key, piranha);
    assert!(!g.waited_on);
    assert_eq!(g.on, "CI on the release");
    assert_eq!(r.w.decide_count(), 2);

    // A wait on a PR is one merge row, not a gone wait too.
    let t76 = wait(
        &piranha,
        "part 10",
        coord::BLOCKED_ON_EXTERNAL,
        PR,
        "Merge PR #41",
    );
    co.leave(&piranha, &t76.id, &piranha).unwrap();
    let s = summary(&t76.id);
    assert!(s.merge_pr.is_some());
    assert!(s.gone_wait.is_none());
    assert_eq!(r.w.decide_count(), 3, "counted once");

    // Handed on, it leaves Needs you and the tray.
    r.w.hand_on(&t73.id).unwrap();
    assert!(summary(&t73.id).gone_wait.is_none());
    assert_eq!(r.w.decide_count(), 2);
}
