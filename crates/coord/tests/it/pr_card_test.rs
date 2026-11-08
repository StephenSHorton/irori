//! Merge desk §12.2 (card #1074, B2a): a PR card is fixed by its creation
//! row, hears its PR from its creator's devices, ends with the PR and is
//! never offered. Two made-up people share one board log; p1 is the PR's
//! author.

use crate::common;

use common::two_people::*;
use coord::rows::{PrReview, PrStateRow, TaskCreatedRow};
use coord::*;
use store::{Filter, kinds};

const PR: &str = "https://github.com/a/b/pull/7";

fn new_card() -> NewTask {
    NewTask {
        title: "PR #7: ship it".into(),
        objective: "Irori made this card for the pull request.".into(),
        repo_key: common::REPO_A.into(),
        links: vec![PR.into()],
        pr_card: Some(PrCard {
            pr: PR.into(),
            author: "alice".into(),
        }),
        ..NewTask::default()
    }
}

/// p1 makes the PR card and both sides have it.
fn with_card() -> (Side, Side, String) {
    let (a, b, _) = two_people();
    let t = a.c.create_task(P1, new_card()).unwrap();
    sync(&a, &b);
    (a, b, t.id)
}

fn check(state: &str) -> PrCheck {
    PrCheck {
        pr: PR.into(),
        state: state.into(),
        ..PrCheck::default()
    }
}

fn rows_of(s: &Side) -> usize {
    s.st.list(&Filter {
        kinds: vec![kinds::PR_STATE.into()],
        limit: store::MAX_LIST_LIMIT,
        ..Filter::default()
    })
    .unwrap()
    .len()
}

/// The creation row an agent writes with a PR card's fields.
fn forged(by: &str, id: &str) -> TaskCreatedRow {
    TaskCreatedRow {
        task_id: id.into(),
        title: "PR #7: ship it".into(),
        objective: "Irori made this card for the pull request.".into(),
        repo_key: common::REPO_A.into(),
        links: vec![parse_link(PR).unwrap()],
        pr: PR.into(),
        pr_author: "alice".into(),
        by: by.into(),
        at: common::fixed_now(),
        ..TaskCreatedRow::default()
    }
}

#[test]
fn a_holderless_pr_card_hears_its_creators_device_on_a_second_persons_fold() {
    let (a, b, id) = with_card();
    assert!(pr_card(&b.c.get_task(&id).unwrap()));
    assert!(b.c.get_task(&id).unwrap().holders.is_empty());
    a.c.record_pr_state(&id, check(PR_OPEN)).unwrap();
    sync(&a, &b);
    let got = b.c.pr_state(&id, PR).expect("p2 heard the state");
    assert_eq!(got.state, PR_OPEN);
    assert_eq!(b.c.ignored_rows(), 0);
}

#[test]
fn another_persons_device_does_not_vouch_for_the_pr_card() {
    let (a, b, id) = with_card();
    // p2's Irori says it merged: p2 is neither the creator nor a holder.
    b.c.record_pr_state(&id, check(PR_MERGED)).ok();
    sync(&a, &b);
    assert!(
        a.c.pr_state(&id, PR).is_none(),
        "{:?}",
        a.c.pr_state(&id, PR)
    );
    // Kept beside the card for a voucher that may still come (card #548),
    // never folded.
    assert_eq!(a.c.get_task(&id).unwrap().state, SUBMITTED);
}

#[test]
fn an_agent_made_card_with_the_same_text_is_not_a_pr_card() {
    let (a, b, _) = two_people();
    // The calls refuse it.
    want_err_pr(b.c.create_task(CARD2, new_card()));
    // A row written by hand folds as a plain card, on every device.
    let ev =
        b.st.append(CARD2, kinds::TASK_CREATED, &forged(CARD2, "forged1"))
            .unwrap();
    b.c.fold_live(&[ev]);
    sync(&a, &b);
    for s in [&a, &b] {
        let t = s.c.get_task("forged1").expect("the card exists");
        assert!(!pr_card(&t), "{t:?}");
    }
    // Nor does p2 as a person make p1's card: their row names them.
    let ev =
        b.st.append(P2, kinds::TASK_CREATED, &forged(P2, "forged2"))
            .unwrap();
    b.c.fold_live(&[ev]);
    sync(&a, &b);
    // (p2's own person row is a PR card by its creator; B2b checks that the
    // creator is the PR author's person, as coord knows no logins.)
    assert_eq!(
        a.c.get_task("forged2").unwrap().pr_card.map(|c| c.author),
        Some("alice".into())
    );
    // A row under a person key from an agent's actor is no person row.
    let ev =
        b.st.append(CARD2, kinds::TASK_CREATED, &forged(P1, "forged3"))
            .unwrap();
    b.c.fold_live(&[ev]);
    sync(&a, &b);
    assert!(!a.c.get_task("forged3").is_some_and(|t| pr_card(&t)));
}

fn want_err_pr(r: coord::Result<Task>) {
    let e = r.expect_err("an agent can't make a PR card");
    assert!(e.to_string().contains("PR card"), "{e}");
}

#[test]
fn editing_or_relinking_a_pr_card_changes_nothing() {
    let (a, b, id) = with_card();
    let was = a.c.get_task(&id).unwrap().pr_card;
    a.c.edit_task(
        P1,
        &id,
        TaskEdit {
            title: Some("something else".into()),
            objective: Some("about another PR".into()),
            ..TaskEdit::default()
        },
    )
    .unwrap();
    a.c.remove_link(P1, &id, PR).unwrap();
    a.c.add_link(P1, &id, "https://github.com/a/b/pull/9")
        .unwrap();
    // A second creation row for its id, from an agent, replaces nothing.
    let ev =
        b.st.append(CARD2, kinds::TASK_CREATED, &forged(CARD2, &id))
            .unwrap();
    b.c.fold_live(&[ev]);
    sync(&a, &b);
    for s in [&a, &b] {
        let t = s.c.get_task(&id).unwrap();
        assert_eq!(t.pr_card, was, "{t:?}");
        assert_eq!(t.created_by, P1);
    }
}

#[test]
fn an_agent_or_person_actor_pr_state_row_is_dropped() {
    let (a, b, id) = with_card();
    a.c.record_pr_state(&id, check(PR_OPEN)).unwrap();
    sync(&a, &b);
    let merged = |by: &str| PrStateRow {
        task_id: id.clone(),
        pr: PR.into(),
        state: PR_MERGED.into(),
        by: by.into(),
        at: a.clk.now(),
        ..PrStateRow::default()
    };
    for actor in [CARD1, P1] {
        let ev = a.st.append(actor, kinds::PR_STATE, &merged(actor)).unwrap();
        a.c.fold_live(std::slice::from_ref(&ev));
        b.c.fold_live(&[ev]);
    }
    for s in [&a, &b] {
        assert_eq!(s.c.pr_state(&id, PR).unwrap().state, PR_OPEN);
        assert!(s.c.ignored_rows() > 0);
        assert_eq!(s.c.get_task(&id).unwrap().state, SUBMITTED);
    }
}

#[test]
fn a_merge_completes_the_card_as_its_creator_on_every_device() {
    let (a, b, id) = with_card();
    a.c.record_pr_state(
        &id,
        PrCheck {
            merge_sha: "abc123".into(),
            ..check(PR_MERGED)
        },
    )
    .unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        let t = s.c.get_task(&id).unwrap();
        assert_eq!(t.state, COMPLETED, "{t:?}");
        assert!(
            t.note.contains("was merged on GitHub as abc123"),
            "{}",
            t.note
        );
    }
    assert_eq!(a.c.ignored_rows() + b.c.ignored_rows(), 0);
}

#[test]
fn close_then_reopen_brings_the_card_back() {
    let (a, b, id) = with_card();
    a.c.record_pr_state(&id, check(PR_OPEN)).unwrap();
    a.clk.add(common::secs(5));
    a.c.record_pr_state(&id, check(PR_CLOSED)).unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        let t = s.c.get_task(&id).unwrap();
        assert_eq!(t.state, CANCELED, "{t:?}");
        assert!(t.note.starts_with(PR_CARD_CLOSED), "{}", t.note);
    }
    a.clk.add(common::secs(5));
    a.c.record_pr_state(&id, check(PR_OPEN)).unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        let t = s.c.get_task(&id).unwrap();
        assert_eq!(t.state, SUBMITTED, "{t:?}");
    }
    // And again: it closes again.
    a.clk.add(common::secs(5));
    a.c.record_pr_state(&id, check(PR_CLOSED)).unwrap();
    assert_eq!(a.c.get_task(&id).unwrap().state, CANCELED);
}

#[test]
fn a_duplicate_cancelled_by_hand_stays_cancelled_when_its_pr_reopens() {
    let (a, b, id) = with_card();
    a.c.set_task_state(P1, &id, CANCELED, "duplicate of #2")
        .unwrap();
    a.clk.add(common::secs(5));
    a.c.record_pr_state(&id, check(PR_CLOSED)).unwrap();
    a.clk.add(common::secs(5));
    a.c.record_pr_state(&id, check(PR_OPEN)).unwrap();
    sync(&a, &b);
    assert_eq!(a.c.get_task(&id).unwrap().state, CANCELED);
}

#[test]
fn a_comment_only_change_writes_a_row_and_an_unchanged_poll_none() {
    let (a, _b, id) = with_card();
    a.c.record_pr_state(&id, check(PR_OPEN)).unwrap();
    let n = rows_of(&a);
    a.clk.add(common::secs(5));
    a.c.record_pr_state(&id, check(PR_OPEN)).unwrap();
    assert_eq!(rows_of(&a), n, "nothing changed, nothing written");

    let heard = |commenters: &[&str]| PrCheck {
        commenters: commenters.iter().map(|s| s.to_string()).collect(),
        reviews: vec![PrReview {
            login: "bob".into(),
            state: "COMMENTED".into(),
            sha: "abc".into(),
            at: common::fixed_now(),
        }],
        paths: vec!["src/a.rs".into()],
        paths_cut: true,
        ..check(PR_OPEN)
    };
    a.clk.add(common::secs(5));
    a.c.record_pr_state(&id, heard(&["bob"])).unwrap();
    assert_eq!(rows_of(&a), n + 1, "a comment alone is a row");
    a.clk.add(common::secs(5));
    a.c.record_pr_state(&id, heard(&["bob"])).unwrap();
    assert_eq!(rows_of(&a), n + 1, "the same comments write nothing");
    a.clk.add(common::secs(5));
    a.c.record_pr_state(&id, heard(&["bob", "carol"])).unwrap();
    assert_eq!(rows_of(&a), n + 2);

    // A row without them says nothing of them: it keeps the last known,
    // and a poll without them against the same state writes nothing.
    a.clk.add(common::secs(5));
    a.c.record_pr_state(&id, check(PR_OPEN)).unwrap();
    assert_eq!(rows_of(&a), n + 2);
    a.clk.add(common::secs(5));
    a.c.record_pr_state(
        &id,
        PrCheck {
            review: PR_CHANGES_REQUESTED.into(),
            ..check(PR_OPEN)
        },
    )
    .unwrap();
    let got = a.c.pr_state(&id, PR).unwrap();
    assert_eq!(got.review, PR_CHANGES_REQUESTED);
    assert_eq!(got.commenters, ["bob", "carol"]);
    assert_eq!(got.reviews.len(), 1);
    assert_eq!(got.paths, ["src/a.rs"]);
    assert!(got.paths_cut);
}

#[test]
fn a_pr_card_is_never_offered() {
    let e = common::Env::new();
    let mut nt = new_card();
    nt.repo_key = common::REPO_A.into();
    let pr = e.c.create_task(common::USER, nt.clone()).unwrap();
    assert!(pr_card(&pr), "{pr:?}");
    let plain =
        e.c.create_task(
            common::USER,
            NewTask {
                pr_card: None,
                ..nt
            },
        )
        .unwrap();
    let room = [];
    assert!(e.c.offer_card(common::USER, &plain.id, &room).is_ok());
    assert!(e.c.offer_card(common::USER, &pr.id, &room).is_err());
}
