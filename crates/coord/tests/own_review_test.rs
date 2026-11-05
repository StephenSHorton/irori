//! Reviews on the person's own board (#88): one of their agents asks
//! another to review a card's work; the card shows each reviewer's state
//! and the reviewer answers with a verdict comment.

mod common;

use common::*;
use coord::*;

fn verdict(c: &Coord, by: &str, id: &str, v: &str, text: &str) -> Result<(Comment, Notice)> {
    c.comment_on(
        by,
        id,
        CommentInput {
            text: text.into(),
            verdict: v.into(),
            ..CommentInput::default()
        },
    )
    .map(|(cm, mut ns)| (cm, ns.remove(0)))
}

fn held_card(e: &Env) -> Task {
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "ship it".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap()
}

#[test]
fn a_review_on_your_own_board_is_requested_then_answered() {
    let e = Env::new();
    let t = held_card(&e);

    // Nobody to ask, yourself, or a PR the card doesn't link: refused.
    want_err(e.c.request_review(ALICE, &t.id, "", ""), ErrorKind::Invalid);
    want_err(
        e.c.request_review(ALICE, &t.id, "", ALICE),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.request_review(ALICE, &t.id, "https://github.com/a/b/pull/9", BOB),
        ErrorKind::Invalid,
    );
    // Nobody asked Bob yet, so his verdict is refused.
    want_err(
        verdict(&e.c, BOB, &t.id, VERDICT_APPROVED, "fine"),
        ErrorKind::NotAllowed,
    );

    // No PR needed: the reviewer is woken once.
    let (rs, n) = e.c.request_review(ALICE, &t.id, "", BOB).unwrap();
    assert_eq!(rs.len(), 1);
    assert_eq!(n.to, vec![BOB.to_string()]);
    assert!(n.wake, "the reviewer is woken");
    assert_eq!(n.kind, NOTICE_REVIEW);
    let v = e.c.reviews_of(&t.id);
    assert_eq!(v.len(), 1);
    assert_eq!((v[0].reviewer.as_str(), v[0].from.as_str()), (BOB, ALICE));
    assert_eq!(v[0].state, REVIEW_REQUESTED);

    // Changes asked: the holder is woken, and the card shows it.
    e.clk.add(secs(5));
    let (cm, n) = verdict(
        &e.c,
        BOB,
        &t.id,
        "Changes",
        "the test misses the shared case",
    )
    .unwrap();
    assert_eq!(cm.verdict, VERDICT_CHANGES);
    assert_eq!(n.to, vec![ALICE.to_string()]);
    assert!(n.wake, "changes asked wakes the holder");
    let v = e.c.reviews_of(&t.id);
    assert_eq!(v[0].state, VERDICT_CHANGES);
    assert_eq!(v[0].text, "the test misses the shared case");
    assert_eq!(v[0].comment_id, cm.id);

    // Asked again: back to requested until Bob answers.
    e.clk.add(secs(5));
    e.c.request_review(ALICE, &t.id, "", BOB).unwrap();
    assert_eq!(e.c.reviews_of(&t.id)[0].state, REVIEW_REQUESTED);

    // Approved: the holder hears at the next turn, not woken.
    e.clk.add(secs(5));
    let (_, n) = verdict(&e.c, BOB, &t.id, VERDICT_APPROVED, "good now").unwrap();
    assert!(!n.wake, "an approval doesn't wake");
    assert_eq!(n.to, vec![ALICE.to_string()]);
    assert_eq!(e.c.reviews_of(&t.id)[0].state, VERDICT_APPROVED);

    // A plain comment from the reviewer leaves the state alone; a bad
    // verdict is refused.
    e.c.add_comment(BOB, &t.id, "one more thought", false)
        .unwrap();
    assert_eq!(e.c.reviews_of(&t.id)[0].state, VERDICT_APPROVED);
    want_err(verdict(&e.c, BOB, &t.id, "maybe", "hm"), ErrorKind::Invalid);

    // It all survives a replay.
    let c2 = e.reopen();
    let v = c2.reviews_of(&t.id);
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].state, VERDICT_APPROVED);
    assert_eq!(v[0].text, "good now");
}

#[test]
fn the_asker_hears_a_verdict_even_when_not_holding() {
    let e = Env::new();
    let t = held_card(&e);
    e.c.request_review(CAROL, &t.id, "", BOB).unwrap();
    let (_, n) = verdict(&e.c, BOB, &t.id, VERDICT_CHANGES, "rename it").unwrap();
    let mut to = n.to.clone();
    to.sort();
    assert_eq!(to, vec![ALICE.to_string(), CAROL.to_string()]);
}

/// Card #471: on the person's own board every agent is theirs, so none of
/// them is asked to review a card that links a PR of the person's; the
/// card's own work with no PR can still get a second pair of eyes.
#[test]
fn your_own_agents_never_review_your_pr() {
    let e = Env::new();
    let t = held_card(&e);
    const PR: &str = "https://github.com/a/b/pull/9";
    e.c.add_link(ALICE, &t.id, PR).unwrap();

    let err = e.c.request_review(ALICE, &t.id, PR, BOB).unwrap_err();
    assert!(err.is(ErrorKind::NotAllowed), "{err}");
    assert!(
        err.to_string()
            .contains("never reviewed by their own agents"),
        "{err}"
    );
    // Leaving pr out doesn't get round it: the card links one.
    want_err(
        e.c.request_review(ALICE, &t.id, "", BOB),
        ErrorKind::NotAllowed,
    );
    assert!(e.c.reviews_of(&t.id).is_empty());
    assert_ne!(e.c.get_task(&t.id).unwrap().state, IN_REVIEW);

    // A card with no PR still can.
    e.c.leave(ALICE, &t.id, ALICE).unwrap();
    let u = held_card(&e);
    e.c.request_review(ALICE, &u.id, "", BOB).unwrap();
    assert_eq!(e.c.reviews_of(&u.id).len(), 1);
}
