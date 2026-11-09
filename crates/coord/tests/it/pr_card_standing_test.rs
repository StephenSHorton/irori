//! Merge desk §12.2 (card #1155): which of two PR cards for one PR stands.
//! A PR card stands only for the one active member whose profile names its
//! PR author's GitHub login, so a member's modified client can't make a
//! lower-id card for someone else's PR stand in for theirs. p1 is alice,
//! the PR's author; p2 (bob) writes the forged card; p3 (carol) reviews.

use crate::common;

use common::two_people::*;
use coord::rows::{ReviewRequestedRow, TaskCreatedRow};
use coord::*;
use store::kinds;

const PR: &str = "https://github.com/a/b/pull/7";
/// Below any id a clock makes, so the forged card has the lowest id.
const FORGED: &str = "00000000000000000000000000";

fn alices_card(s: &Side) -> String {
    s.c.create_task(
        P1,
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
        },
    )
    .unwrap()
    .id
}

/// p2's person row, from p2's own device, making a PR card for alice's PR:
/// what a modified client can write.
fn forge_card(b: &Side) {
    let row = TaskCreatedRow {
        task_id: FORGED.into(),
        title: "PR #7: ship it".into(),
        objective: "Irori made this card for the pull request.".into(),
        repo_key: common::REPO_A.into(),
        links: vec![parse_link(PR).unwrap()],
        pr: PR.into(),
        pr_author: "alice".into(),
        by: P2.into(),
        at: common::fixed_now(),
        ..TaskCreatedRow::default()
    };
    let ev = b.st.append(P2, kinds::TASK_CREATED, &row).unwrap();
    b.c.fold_live(&[ev]);
}

/// Three people with alice's card and bob's lower-id forged one, both PR
/// cards on every side.
fn with_both() -> (Side, Side, Side, String) {
    let (a, b, c, _) = three_people();
    forge_card(&b);
    let id = alices_card(&a);
    sync_all(&[&a, &b, &c]);
    for s in [&a, &b, &c] {
        assert!(
            pr_card(&s.c.get_task(FORGED).unwrap()),
            "a PR card's fields"
        );
        assert!(FORGED < id.as_str());
    }
    (a, b, c, id)
}

fn standing(s: &Side) -> Option<String> {
    s.c.pr_card_for(PR).map(|t| t.id)
}

#[test]
fn a_lower_id_card_by_another_member_never_stands_for_the_authors() {
    let (a, b, c, id) = with_both();
    for s in [&a, &b, &c] {
        assert_eq!(standing(s).as_deref(), Some(id.as_str()));
        // Alice's card is in the review; bob's is not.
        assert_eq!(s.c.review_phase(&id), ReviewPhase::Waiting);
        assert_eq!(s.c.review_phase(FORGED), ReviewPhase::Off);
    }
    // Alice's device picks on her card; bob's device can't pick on his.
    let r = a.c.pick_ask(P1, &id, CARD4, false, None).unwrap().unwrap();
    assert_eq!(r.to, CARD4);
    let e = b.c.pick_ask(P2, FORGED, CARD4, false, None).unwrap_err();
    assert!(!e.to_string().is_empty());
    sync_all(&[&a, &b, &c]);
    for s in [&a, &b, &c] {
        match s.c.review_phase(&id) {
            ReviewPhase::Asked { ask } => assert_eq!(ask.to, CARD4),
            p => panic!("{p:?}"),
        }
    }
}

#[test]
fn the_forged_cards_pr_state_never_ends_a_claim_on_the_authors() {
    let (a, b, c, id) = with_both();
    c.c.claim_pr(CARD4, PR).unwrap();
    sync_all(&[&a, &b, &c]);
    // Bob's Irori says the PR merged, on his own card: it folds there (he
    // made that card) but no claim reads it.
    b.clk.add(common::secs(5));
    b.c.record_pr_state(
        FORGED,
        PrCheck {
            pr: PR.into(),
            state: PR_MERGED.into(),
            ..PrCheck::default()
        },
    )
    .unwrap();
    sync_all(&[&a, &b, &c]);
    for s in [&a, &b, &c] {
        let l = s.c.claim_of(PR).expect("the claim still counts");
        assert_eq!(l.holder, CARD4);
        assert!(matches!(s.c.review_phase(&id), ReviewPhase::Claimed { .. }));
    }
}

#[test]
fn an_ask_naming_the_forgers_agent_is_refused_as_an_authors() {
    let (a, b, c, id) = with_both();
    // Alice asks bob's agent on her card: bob made a PR card for the PR, so
    // he reads as its author and the row folds as nothing, on every side.
    let ev =
        a.st.append(
            P1,
            kinds::REVIEW_REQUESTED,
            &ReviewRequestedRow {
                task_id: id.clone(),
                pr: PR.into(),
                from: P1.into(),
                to: CARD2.into(),
                at: common::fixed_now(),
                wake: true,
                look: false,
            },
        )
        .unwrap();
    a.c.fold_live(&[ev]);
    sync_all(&[&a, &b, &c]);
    for s in [&a, &b, &c] {
        assert!(s.c.reviews(&id).is_empty(), "{:?}", s.c.reviews(&id));
        assert_eq!(s.c.review_phase(&id), ReviewPhase::Waiting);
    }
}

#[test]
fn a_login_two_members_claim_stands_for_nobody() {
    let (a, b, c, id) = with_both();
    // Bob's profile now says he is alice too: neither card stands, so he
    // can stop alice's reviews here, visibly, but never take them.
    b.set_login("p2", "alice");
    for s in [&a, &b, &c] {
        assert_eq!(standing(s), None);
        assert_eq!(s.c.review_phase(&id), ReviewPhase::Off);
        assert_eq!(s.c.review_phase(FORGED), ReviewPhase::Off);
    }
    // Once he stops, alice's card stands again.
    b.set_login("p2", "bob");
    for s in [&a, &b, &c] {
        assert_eq!(standing(s).as_deref(), Some(id.as_str()));
    }
}

#[test]
fn a_card_stands_only_once_the_authors_profile_names_the_login() {
    let (a, b, c, _) = three_people();
    a.set_login("p1", "");
    let id = alices_card(&a);
    sync_all(&[&a, &b, &c]);
    for s in [&a, &b, &c] {
        assert_eq!(standing(s), None);
    }
    a.set_login("p1", "Alice");
    for s in [&a, &b, &c] {
        assert_eq!(standing(s).as_deref(), Some(id.as_str()), "any case");
    }
}
