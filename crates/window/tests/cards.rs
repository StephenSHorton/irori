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
    let other = held_task(&r);
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
        r.w.answer(&t.id, &bl.id, -1, "Which one is cheaper to run?", true)
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
    assert!(lines[1].contains(&format!("answered blocker {}", bl.id)));
    assert!(lines[2].contains(&format!("replied on blocker {}", bl.id)));
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
    let res = r.w.answer(&t.id, &b1.id, -1, "yes", true).unwrap();
    let b = blocker(&res.card, &b1.id);
    let rs = b.resolved.as_ref().unwrap();
    assert_eq!(b.state, BLOCKER_RESOLVED);
    assert_eq!((rs.answer.as_str(), rs.note.as_str()), ("yes", ""));
    assert_eq!(res.card.summary.state, coord::BLOCKED);
    let res = r.w.answer(&t.id, &b2.id, -1, "eu-west", true).unwrap();
    let b = blocker(&res.card, &b2.id);
    assert_eq!(b.state, BLOCKER_RESOLVED);
    assert_eq!(b.resolved.as_ref().unwrap().answer, "eu-west");
    assert_eq!(res.card.summary.state, coord::WORKING);

    // The person's own item on a card a live agent holds.
    let t2 = held_task(&r);
    let mine = raise(&r, &me(&r), &t2.id, "check the staging DNS", &[]);
    let res = r.w.answer(&t2.id, &mine.id, -1, "checked", false).unwrap();
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
        r.w.resolve_blocker(&t.id, NOTE_ITEM, "")
            .unwrap_err()
            .starts_with(invalid)
    );
    assert!(
        r.w.resolve_blocker(&t.id, &b1.id, "")
            .unwrap_err()
            .starts_with(invalid),
        "resolved an unanswered item with nothing"
    );
    r.w.answer(&t.id, &b1.id, 0, "", false).unwrap();
    r.w.answer(&t.id, &b2.id, -1, "whatever is free", false)
        .unwrap();
    let res = r.w.resolve_blocker(&t.id, &b1.id, "").unwrap();
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
        )
        .unwrap();
    let b = blocker(&res.card, &b2.id);
    let rs = b.resolved.as_ref().unwrap();
    assert_eq!(b.state, BLOCKER_RESOLVED);
    assert_eq!(rs.answer, "whatever is free");
    assert_eq!(rs.note, "I'll pick one myself later.");
    assert_eq!(res.card.summary.state, coord::WORKING);
    assert!(
        r.w.resolve_blocker(&t.id, &b2.id, "")
            .unwrap_err()
            .starts_with(coord::ErrorKind::BadState.text()),
        "resolved twice"
    );
}

/// Go TestCardsPinnedFirst and TestBoardActions' movePin: pinned cards
/// lead in pin order, MovePin reorders them, an agent can't pin, and a
/// pinned parent keeps its subtasks with it.
#[test]
fn cards_pinned_first() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let (a, z, m) = (held_task(&r), held_task(&r), held_task(&r));
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
    let res = r.w.comment(&t.id, "note to self", "wake").unwrap();
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
    r.w.answer(&t.id, &bl.id, -1, "Which one is cheaper to run?", true)
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
        rev.text.contains("revised blocker")
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
    let res = r.w.answer(&t.id, &bl.id, 0, "", true).unwrap();
    let b = blocker(&res.card, &bl.id);
    assert_eq!(b.state, BLOCKER_ANSWERED);
    assert_eq!(b.thread.last().unwrap().text, "Yes");
}
