//! Merge desk §12.10 (card #1082, B7): a review card is the PR card's
//! creator's, made by their person and nobody else, and is never handed
//! out. Two made-up people share one board log; p1 is the PR's author.

use crate::common;

use common::two_people::*;
use coord::rows::TaskCreatedRow;
use coord::*;
use store::kinds;

const PR: &str = "https://github.com/a/b/pull/7";

fn pr_card_of_p1(a: &Side) -> Task {
    a.c.create_task(
        P1,
        NewTask {
            title: "PR #7: ship it".into(),
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
}

fn review_card(card: &str) -> NewTask {
    NewTask {
        title: "Review PR #7".into(),
        repo_key: common::REPO_A.into(),
        links: vec![PR.into()],
        review_of: card.into(),
        ..NewTask::default()
    }
}

fn forged(by: &str, id: &str, card: &str) -> TaskCreatedRow {
    TaskCreatedRow {
        task_id: id.into(),
        title: "Review PR #7".into(),
        repo_key: common::REPO_A.into(),
        review_of: card.into(),
        by: by.into(),
        at: common::fixed_now(),
        ..TaskCreatedRow::default()
    }
}

#[test]
fn the_pr_cards_creator_makes_a_review_card_both_sides_see() {
    let (a, b, _) = two_people();
    let pc = pr_card_of_p1(&a);
    let r = a.c.create_task(P1, review_card(&pc.id)).unwrap();
    assert_eq!(r.review_of, pc.id);
    sync(&a, &b);
    assert_eq!(b.c.get_task(&r.id).unwrap().review_of, pc.id);
    assert_eq!(b.c.ignored_rows(), 0);
}

#[test]
fn nobody_else_makes_one() {
    let (a, b, _) = two_people();
    let pc = pr_card_of_p1(&a);
    sync(&a, &b);
    // The calls refuse it: another person, and an agent.
    for by in [P2, CARD2, CARD1] {
        let side = if by == P2 || by == CARD2 { &b } else { &a };
        let e = side
            .c
            .create_task(by, review_card(&pc.id))
            .expect_err("refused");
        assert!(e.to_string().contains("review card"), "{by}: {e}");
    }
    // Rows written by hand fold as plain cards on every device.
    for (i, by) in [P2, CARD2].into_iter().enumerate() {
        let id = format!("forged{i}");
        let ev =
            b.st.append(by, kinds::TASK_CREATED, &forged(by, &id, &pc.id))
                .unwrap();
        b.c.fold_live(&[ev]);
    }
    let ev =
        a.st.append(CARD1, kinds::TASK_CREATED, &forged(P1, "forged2", &pc.id))
            .unwrap();
    a.c.fold_live(&[ev]);
    sync(&a, &b);
    for s in [&a, &b] {
        for id in ["forged0", "forged1", "forged2"] {
            let t = s.c.get_task(id).expect("the card exists");
            assert!(t.review_of.is_empty(), "{id}: {t:?}");
        }
    }
    // Nor does a review card name a card that is no PR card.
    let plain =
        a.c.create_task(
            P1,
            NewTask {
                title: "plain".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    assert!(a.c.create_task(P1, review_card(&plain.id)).is_err());
}

#[test]
fn a_review_card_is_never_handed_out() {
    let (a, _b, _) = two_people();
    let pc = pr_card_of_p1(&a);
    let r = a.c.create_task(P1, review_card(&pc.id)).unwrap();
    let plain =
        a.c.create_task(
            P1,
            NewTask {
                title: "plain work".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    let ids: Vec<String> = a.c.cards_to_spawn().into_iter().map(|t| t.id).collect();
    assert!(!ids.contains(&r.id), "a review card was offered: {ids:?}");
    let _ = plain;
}
