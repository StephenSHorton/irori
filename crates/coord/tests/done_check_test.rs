//! Card #112: the person has the final say on done. With
//! `Options::done_check` on, an agent's completed raises a done check on
//! the person instead of closing the card.

mod common;

use common::*;
use coord::rows::TaskStateRow;
use coord::*;
use store::kinds;

fn env() -> Env {
    let mut e = Env::new();
    e.c = e.reopen_with(Options {
        done_check: true,
        ..e.opts()
    });
    e
}

fn claimed(e: &Env) -> Task {
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "ship it".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(BOB, &t.id).unwrap()
}

fn done_checks(t: &Task) -> Vec<&Blocker> {
    t.blockers
        .iter()
        .filter(|b| b.open() && b.done_check)
        .collect()
}

#[test]
fn an_agents_completed_waits_on_the_person() {
    let e = env();
    let t = claimed(&e);
    let got =
        e.c.set_task_state(BOB, &t.id, COMPLETED, "all green")
            .unwrap();
    assert_eq!(got.state, BLOCKED);
    let checks = done_checks(&got);
    assert_eq!(checks.len(), 1);
    let b = checks[0];
    assert_eq!(b.question, DONE_QUESTION);
    assert_eq!(b.context, "all green");
    assert_eq!(b.raised_by, BOB);
    assert_eq!(b.options.len(), 1);
    assert_eq!(b.options[0].answer, DONE_ANSWER);
    assert!(b.options[0].recommended);
    // Nothing wrote completed.
    let rows = e.rows(kinds::TASK_STATUS);
    assert!(
        rows.iter()
            .all(|r| !String::from_utf8_lossy(&r.payload).contains("\"completed\"")),
        "no completed row"
    );
    // A replay folds the same check.
    let again = e.reopen().get_task(&t.id).unwrap();
    assert_eq!(done_checks(&again).len(), 1);
}

#[test]
fn no_note_gives_the_default_context() {
    let e = env();
    let t = claimed(&e);
    let got = e.c.set_task_state(BOB, &t.id, COMPLETED, "").unwrap();
    assert_eq!(done_checks(&got)[0].context, DONE_CONTEXT);
}

#[test]
fn the_persons_completed_closes_directly() {
    let e = env();
    let t = claimed(&e);
    let got = e.c.set_task_state(USER, &t.id, COMPLETED, "").unwrap();
    assert_eq!(got.state, COMPLETED);
    assert!(got.blockers.is_empty());
}

#[test]
fn failed_and_canceled_stay_direct() {
    let e = env();
    let t = claimed(&e);
    let got = e.c.set_task_state(BOB, &t.id, FAILED, "broke").unwrap();
    assert_eq!(got.state, FAILED);
}

#[test]
fn done_then_completed_closes_it() {
    let e = env();
    let t = claimed(&e);
    let got = e.c.set_task_state(BOB, &t.id, COMPLETED, "").unwrap();
    let bid = done_checks(&got)[0].id.clone();
    let (after, _) =
        e.c.answer(
            USER,
            &t.id,
            &bid,
            BlockerAnswer {
                pick: 1,
                resolve: true,
                ..BlockerAnswer::default()
            },
        )
        .unwrap();
    assert!(done_checks(&after).is_empty());
    assert_eq!(after.state, WORKING);
    let closed = e.c.set_task_state(USER, &t.id, COMPLETED, "").unwrap();
    assert_eq!(closed.state, COMPLETED);
}

#[test]
fn not_done_goes_back_and_a_new_check_replaces_it() {
    let e = env();
    let t = claimed(&e);
    let got = e.c.set_task_state(BOB, &t.id, COMPLETED, "").unwrap();
    let first = done_checks(&got)[0].id.clone();
    e.c.sweep_notices().unwrap();
    // The person says why it isn't done: the agent hears it.
    let (_, ns) =
        e.c.answer(
            USER,
            &t.id,
            &first,
            BlockerAnswer {
                text: "the login page still 500s".into(),
                ..BlockerAnswer::default()
            },
        )
        .unwrap();
    assert!(
        ns.iter().any(|n| n.kind == NOTICE_ANSWERED
            && n.to.iter().any(|k| k == BOB)
            && n.text.contains("the login page still 500s")),
        "{ns:?}"
    );
    // The agent answers them on the check (card #476), fixes it and says
    // done again: the answered check gives way to a fresh one, so only one
    // waits on the person.
    reply(&e, &t.id, &first, "found it: the session cookie; fixing");
    let again =
        e.c.set_task_state(BOB, &t.id, COMPLETED, "fixed the 500")
            .unwrap();
    let checks = done_checks(&again);
    assert_eq!(checks.len(), 1);
    assert_ne!(checks[0].id, first);
    assert_eq!(checks[0].context, "fixed the 500");
    assert_eq!(again.state, BLOCKED);
}

#[test]
fn asking_twice_keeps_one_check() {
    let e = env();
    let t = claimed(&e);
    e.c.set_task_state(BOB, &t.id, COMPLETED, "").unwrap();
    let got = e.c.set_task_state(BOB, &t.id, COMPLETED, "").unwrap();
    assert_eq!(done_checks(&got).len(), 1);
}

#[test]
fn an_old_windows_done_closes_on_the_next_completed() {
    // A window that predates done checks sends Done as a plain answer;
    // the agent resolves it and says completed again: the person's Done
    // stands, so the card closes instead of asking again.
    let e = env();
    let t = claimed(&e);
    let got = e.c.set_task_state(BOB, &t.id, COMPLETED, "").unwrap();
    let bid = done_checks(&got)[0].id.clone();
    e.c.answer(
        USER,
        &t.id,
        &bid,
        BlockerAnswer {
            pick: 1,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    // The agent resolves the answered check (board_unblock blocker_id).
    e.c.answer(
        BOB,
        &t.id,
        &bid,
        BlockerAnswer {
            text: "closing it".into(),
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    let closed = e.c.set_task_state(BOB, &t.id, COMPLETED, "").unwrap();
    assert_eq!(closed.state, COMPLETED);
    assert!(done_checks(&closed).is_empty());
}

/// Card #140: an agent's completed brings proof. The whole note (past the
/// 800-character blocker context) and its screenshots ride on the done
/// check, fold again on replay, and stay on the card after the person's
/// Done closes it.
#[test]
fn proof_rides_on_the_done_check() {
    let e = env();
    assert!(e.c.done_check());
    let t = claimed(&e);
    let shot = FileRef {
        sha256: "ab".repeat(32),
        name: "shot.png".into(),
        size: 12,
        mime: "image/png".into(),
        device: "dev-a".into(),
    };
    let note = format!(
        "Widened the dock collider.\nChecked: cargo test, and 20 dockings in the app.\nCommit abc123.\n{}",
        "Caveat: ".repeat(150).trim_end()
    );
    assert!(note.chars().count() > 800);
    let got =
        e.c.set_task_state_with(BOB, &t.id, COMPLETED, &note, vec![shot.clone()], true)
            .unwrap();
    assert_eq!(got.state, BLOCKED);
    let b = done_checks(&got)[0].clone();
    assert_eq!(b.context, note, "the whole proof, not cut at 800");
    assert_eq!(b.attachments, vec![shot.clone()]);

    let again = e.reopen().get_task(&t.id).unwrap();
    assert_eq!(done_checks(&again)[0].attachments, vec![shot.clone()]);

    e.c.answer(
        USER,
        &t.id,
        &b.id,
        BlockerAnswer {
            pick: 1,
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    let closed = e.c.set_task_state(USER, &t.id, COMPLETED, "").unwrap();
    assert_eq!(closed.state, COMPLETED);
    assert!(
        closed
            .blockers
            .iter()
            .any(|x| x.id == b.id && x.context == note && x.attachments == vec![shot.clone()]),
        "the proof stays on the closed card"
    );
}

#[test]
fn proof_files_are_capped() {
    let e = env();
    let t = claimed(&e);
    let files: Vec<FileRef> = (0..11)
        .map(|i| FileRef {
            sha256: format!("{i:02}").repeat(32),
            name: format!("{i}.png"),
            size: 1,
            mime: "image/png".into(),
            device: "dev-a".into(),
        })
        .collect();
    assert!(
        e.c.set_task_state_with(BOB, &t.id, COMPLETED, "done", files, true)
            .is_err()
    );
    assert_eq!(e.c.get_task(&t.id).unwrap().state, WORKING);
}

/// Card #140: coord refuses an agent's completed with no proof when asked
/// to, but only after its own refusals; once the person said Done the card
/// closes without one; the person never needs it.
#[test]
fn proof_is_required_after_other_refusals() {
    let e = env();
    let t = claimed(&e);
    let err =
        e.c.set_task_state_with(BOB, &t.id, COMPLETED, "", Vec::new(), true)
            .unwrap_err()
            .to_string();
    assert!(err.contains(NEEDS_PROOF), "{err}");
    assert_eq!(e.c.get_task(&t.id).unwrap().state, WORKING);
    // Any agent may change it (card #320), and needs proof the same way.
    let err =
        e.c.set_task_state_with(CAROL, &t.id, COMPLETED, "", Vec::new(), true)
            .unwrap_err()
            .to_string();
    assert!(err.contains(NEEDS_PROOF), "{err}");
    // The person's completed needs none.
    let got =
        e.c.set_task_state_with(USER, &t.id, COMPLETED, "", Vec::new(), true)
            .unwrap();
    assert_eq!(got.state, COMPLETED);
}

/// Card #140: a done check's long proof doesn't stop its raiser revising
/// it (revise checks a done check's context against MAX_OBJECTIVE).
#[test]
fn a_long_proof_check_can_be_revised() {
    let e = env();
    let t = claimed(&e);
    let note = "checked ".repeat(200).trim_end().to_string();
    assert!(note.chars().count() > 800);
    let got =
        e.c.set_task_state_with(BOB, &t.id, COMPLETED, &note, Vec::new(), true)
            .unwrap();
    let b = done_checks(&got)[0].clone();
    e.c.revise(
        BOB,
        &t.id,
        &b.id,
        BlockerRevision {
            question: "Is this done now?".into(),
            ..BlockerRevision::default()
        },
    )
    .unwrap();
}

/// Card #315: the person's Done completes the card in one step. Its
/// holders hear it is completed (never "working again"), the check reads
/// answered Done, and no row ever puts the card back to working.
#[test]
fn done_goes_straight_to_completed() {
    let e = env();
    let t = claimed(&e);
    let got =
        e.c.set_task_state(BOB, &t.id, COMPLETED, "all green")
            .unwrap();
    let bid = done_checks(&got)[0].id.clone();
    let resolved_before = e.rows(kinds::BLOCKER_RESOLVED).len();
    let status_before = e.rows(kinds::TASK_STATUS).len();

    let (closed, ns) = e.c.say_done(USER, &t.id, &bid, &[]).unwrap();
    assert_eq!(closed.state, COMPLETED);
    assert!(done_checks(&closed).is_empty());
    let b = closed.blockers.iter().find(|b| b.id == bid).unwrap();
    let res = b.resolved.as_ref().expect("resolved");
    assert_eq!(res.answer, DONE_ANSWER);
    assert_eq!(res.option, 0);
    assert!(!res.closed, "{res:?}");
    assert_eq!(res.by, USER);

    let n = &ns[0];
    assert_eq!(n.kind, NOTICE_ANSWERED);
    assert_eq!(n.to, vec![BOB.to_string()]);
    assert_eq!(
        n.text,
        format!(
            "The person said Done on \"ship it\" (#1, task:{id}): it is completed.\nshow task:{id} for the card, its comments and links.",
            id = t.id
        )
    );
    assert!(!n.text.contains("working"), "{}", n.text);

    // The state history: one completed row and no resolve row, so the
    // card went from waiting on the person straight to completed.
    assert_eq!(e.rows(kinds::BLOCKER_RESOLVED).len(), resolved_before);
    let status = e.rows(kinds::TASK_STATUS);
    assert_eq!(status.len(), status_before + 1);
    let row: TaskStateRow = payload(status.last().unwrap());
    assert_eq!(row.state, COMPLETED);
    let states: Vec<String> = status
        .iter()
        .map(|ev| payload::<TaskStateRow>(ev).state)
        .collect();
    assert!(!states.iter().skip(status_before).any(|s| s == WORKING));

    // It folds the same on replay.
    let again = e.reopen_with(Options {
        done_check: true,
        ..e.opts()
    });
    let t2 = again.get_task(&t.id).unwrap();
    assert_eq!(t2.state, COMPLETED);
    let r2 = t2.blockers.iter().find(|b| b.id == bid).unwrap();
    assert_eq!(r2.resolved.as_ref().unwrap().answer, DONE_ANSWER);
}

#[test]
fn say_done_is_the_persons_and_only_on_a_done_check() {
    let e = env();
    let t = claimed(&e);
    let got = e.c.set_task_state(BOB, &t.id, COMPLETED, "").unwrap();
    let bid = done_checks(&got)[0].id.clone();
    assert!(e.c.say_done(BOB, &t.id, &bid, &[]).is_err());
    assert!(e.c.say_done(USER, &t.id, "b99", &[]).is_err());
    let u = claimed_other(&e);
    let (blocked, _) =
        e.c.raise(
            BOB,
            &u.id,
            NewBlocker {
                question: "which port?".into(),
                ..NewBlocker::default()
            },
        )
        .unwrap();
    let qb = blocked.blockers[0].id.clone();
    assert!(e.c.say_done(USER, &u.id, &qb, &[]).is_err());
    assert_eq!(e.c.get_task(&t.id).unwrap().state, BLOCKED);
}

fn claimed_other(e: &Env) -> Task {
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "other".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(BOB, &t.id).unwrap()
}

/// Card #315: an old window's Done left on an open check closes the card
/// on the agent's next completed, with no resolve row putting it back to
/// working first.
#[test]
fn an_old_windows_open_done_closes_without_working() {
    let e = env();
    let t = claimed(&e);
    let got = e.c.set_task_state(BOB, &t.id, COMPLETED, "").unwrap();
    let bid = done_checks(&got)[0].id.clone();
    e.c.answer(
        USER,
        &t.id,
        &bid,
        BlockerAnswer {
            pick: 1,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    let resolved_before = e.rows(kinds::BLOCKER_RESOLVED).len();
    let closed = e.c.set_task_state(BOB, &t.id, COMPLETED, "").unwrap();
    assert_eq!(closed.state, COMPLETED);
    assert!(done_checks(&closed).is_empty());
    assert_eq!(e.rows(kinds::BLOCKER_RESOLVED).len(), resolved_before);
}

fn reply(e: &Env, id: &str, bid: &str, text: &str) -> Comment {
    e.c.comment_on(
        BOB,
        id,
        CommentInput {
            text: text.into(),
            blocker_id: bid.into(),
            ..CommentInput::default()
        },
    )
    .unwrap()
    .0
}

fn not_done(e: &Env, id: &str, bid: &str, text: &str) {
    e.c.answer(
        USER,
        id,
        bid,
        BlockerAnswer {
            text: text.into(),
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
}

/// Card #476: a Not done is answered on its check before the next one,
/// and the next one brings changed proof; the exchange stays on the old
/// check's thread.
#[test]
fn after_not_done_the_agent_answers_on_the_check_before_asking_again() {
    let e = env();
    let t = claimed(&e);
    let got =
        e.c.set_task_state(BOB, &t.id, COMPLETED, "PR merged")
            .unwrap();
    let first = done_checks(&got)[0].id.clone();
    not_done(
        &e,
        &t.id,
        &first,
        "the others haven't been told the next steps",
    );
    let sb = e.c.sent_back(&t.id).expect("sent back");
    assert_eq!(sb.id, first);
    assert!(!sb.answered);
    assert_eq!(sb.proof, "PR merged");

    // Asking again without answering them is refused, and names the check.
    let err =
        e.c.set_task_state(BOB, &t.id, COMPLETED, "PR merged, team told")
            .unwrap_err()
            .to_string();
    assert!(err.contains(&format!("blocker_id={first}")), "{err}");
    assert!(err.contains("the others haven't been told"), "{err}");
    assert_eq!(done_checks(&e.c.get_task(&t.id).unwrap())[0].id, first);

    // The answer goes on the check's thread.
    reply(
        &e,
        &t.id,
        &first,
        "posted the next steps in #team and made #108",
    );
    assert!(e.c.sent_back(&t.id).unwrap().answered);

    // The same proof again is refused: it must say what changed.
    let err =
        e.c.set_task_state(BOB, &t.id, COMPLETED, "PR merged")
            .unwrap_err()
            .to_string();
    assert!(err.contains("what changed"), "{err}");

    let again =
        e.c.set_task_state(BOB, &t.id, COMPLETED, "PR merged; next steps posted, #108")
            .unwrap();
    let checks = done_checks(&again);
    assert_eq!(checks.len(), 1);
    assert_ne!(checks[0].id, first);
    // The old check is resolved, its exchange kept in order on its thread.
    let old = again.blockers.iter().find(|b| b.id == first).unwrap();
    assert!(!old.open());
    let thread: Vec<(bool, String)> =
        e.c.comments(&t.id)
            .unwrap()
            .into_iter()
            .filter(|c| c.blocker_id == first)
            .map(|c| (c.by == USER, c.text))
            .collect();
    assert_eq!(
        thread,
        vec![
            (
                true,
                "the others haven't been told the next steps".to_string()
            ),
            (
                false,
                "posted the next steps in #team and made #108".to_string()
            ),
        ]
    );
    // The new check isn't sent back: nothing holds the next completed.
    assert!(e.c.sent_back(&t.id).is_none());
}

/// Card #476: the Not done notice tells the agent to answer on the check,
/// not to revise it with new options.
#[test]
fn the_not_done_notice_says_answer_on_the_check() {
    let e = env();
    let t = claimed(&e);
    let got = e.c.set_task_state(BOB, &t.id, COMPLETED, "done").unwrap();
    let bid = done_checks(&got)[0].id.clone();
    let (_, ns) =
        e.c.answer(
            USER,
            &t.id,
            &bid,
            BlockerAnswer {
                text: "also the docs".into(),
                ..BlockerAnswer::default()
            },
        )
        .unwrap();
    let n = ns.iter().find(|n| n.kind == NOTICE_ANSWERED).unwrap();
    assert!(n.text.contains(&format!("blocker_id={bid}")), "{}", n.text);
    assert!(n.text.contains("what changed"), "{}", n.text);
    assert!(!n.text.contains("board_block"), "{}", n.text);
}

/// Card #476: once the next check (or the Board) closed the sent-back
/// check, the agent can still answer on it; any other resolved blocker
/// stays closed to replies.
#[test]
fn a_closed_sent_back_check_still_takes_the_agents_answer() {
    let e = env();
    let t = claimed(&e);
    let got = e.c.set_task_state(BOB, &t.id, COMPLETED, "done").unwrap();
    let bid = done_checks(&got)[0].id.clone();
    not_done(&e, &t.id, &bid, "also the docs");
    // The person (or the Board) closes it.
    e.c.answer(
        USER,
        &t.id,
        &bid,
        BlockerAnswer {
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    assert!(
        !e.c.get_task(&t.id)
            .unwrap()
            .blockers
            .iter()
            .any(|b| b.id == bid && b.open())
    );
    reply(&e, &t.id, &bid, "docs updated in the same PR");
    assert!(e.c.sent_back(&t.id).unwrap().answered);

    let b = e.c.block(BOB, &t.id, "which port?", None).unwrap();
    let q = b
        .blockers
        .iter()
        .find(|b| b.open() && !b.done_check)
        .unwrap()
        .id
        .clone();
    e.c.answer(
        BOB,
        &t.id,
        &q,
        BlockerAnswer {
            text: "dropped".into(),
            resolve: true,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    let err =
        e.c.comment_on(
            BOB,
            &t.id,
            CommentInput {
                text: "late".into(),
                blocker_id: q.clone(),
                ..CommentInput::default()
            },
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("already resolved"), "{err}");
}

/// Card #476 (review): a sent-back done check isn't revised, so the gate can't be
/// sidestepped by board_block blocker_id.
#[test]
fn a_done_check_is_not_revised() {
    let e = env();
    let t = claimed(&e);
    let got = e.c.set_task_state(BOB, &t.id, COMPLETED, "done").unwrap();
    let bid = done_checks(&got)[0].id.clone();
    not_done(&e, &t.id, &bid, "also the docs");
    let err =
        e.c.revise(
            BOB,
            &t.id,
            &bid,
            BlockerRevision {
                context: Some("done, docs too".into()),
                ..BlockerRevision::default()
            },
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("isn't revised"), "{err}");
    assert!(err.contains(&format!("blocker_id={bid}")), "{err}");
}

/// Card #476 (review): the person's Done after their Not done withdraws
/// the reason; a reopened card's next completed isn't held by it.
#[test]
fn done_after_not_done_withdraws_the_reason() {
    let e = env();
    let t = claimed(&e);
    let got = e.c.set_task_state(BOB, &t.id, COMPLETED, "done").unwrap();
    let bid = done_checks(&got)[0].id.clone();
    not_done(&e, &t.id, &bid, "also the docs");
    assert!(e.c.sent_back(&t.id).is_some());
    e.c.say_done(USER, &t.id, &bid, &[]).unwrap();
    assert_eq!(e.c.get_task(&t.id).unwrap().state, COMPLETED);
    assert!(e.c.sent_back(&t.id).is_none());
}

/// Card #548: a card waiting on the person's Done is past hearing its PR
/// merged: the agent has already completed it.
#[test]
fn a_card_waiting_on_done_is_no_pr_news() {
    const PR: &str = "https://github.com/a/b/pull/7";
    let e = env();
    let t = claimed(&e);
    e.c.add_link(ALICE, &t.id, PR).unwrap();
    e.c.record_pr_state(
        &t.id,
        PrCheck {
            pr: PR.into(),
            state: PR_MERGED.into(),
            ..PrCheck::default()
        },
    )
    .unwrap();
    assert_eq!(e.c.pr_news().len(), 1);
    e.c.set_task_state(ALICE, &t.id, COMPLETED, "landed")
        .unwrap();
    assert!(e.c.pr_news().is_empty());
}
