//! Merge desk §12.5 to §12.7 (card #1079, B4), coord's side: the
//! person's ask and ping on a PR card, the author check, the pick state
//! from rows, declines, the claim tally, the catch-up's asks, and
//! open_reviewer's order. p1 is the PR's author unless a test says
//! otherwise.

use crate::common;

use common::two_people::*;
use common::{hlc_at, mins, plus};
use coord::roster::AgentCard;
use coord::rows::{PrReview, ReviewRequestedRow, TaskCommentRow};
use coord::*;
use gocompat::fnv::fnv32a;
use store::{Event, Filter, kinds};

fn pr(n: u32) -> String {
    format!("https://github.com/a/b/pull/{n}")
}

/// p1's PR card for PR n, synced to every side.
fn pr_card(sides: &[&Side], n: u32) -> String {
    let t = sides[0]
        .c
        .create_task(
            P1,
            NewTask {
                title: format!("PR #{n}: ship it"),
                objective: "Irori made this card for the pull request.".into(),
                repo_key: common::REPO_A.into(),
                links: vec![pr(n)],
                pr_card: Some(PrCard {
                    pr: pr(n),
                    author: "alice".into(),
                }),
                ..NewTask::default()
            },
        )
        .unwrap();
    sync_all(sides);
    t.id
}

/// A plain shared card of p1's linking `link` (none when empty).
fn plain_card(s: &Side, link: &str) -> String {
    s.c.create_task(
        P1,
        NewTask {
            title: "shared work".into(),
            repo_key: common::REPO_A.into(),
            shared: true,
            links: if link.is_empty() {
                vec![]
            } else {
                vec![link.into()]
            },
            ..NewTask::default()
        },
    )
    .unwrap()
    .id
}

fn count(s: &Side, kind: &str) -> usize {
    s.st.list(&Filter {
        kinds: vec![kind.into()],
        limit: store::MAX_LIST_LIMIT,
        ..Filter::default()
    })
    .unwrap()
    .len()
}

/// A review.requested row as if `dev` wrote it, folded on every side.
fn forge_ask(sides: &[&Side], dev: &str, seq: i64, actor: &str, row: ReviewRequestedRow) {
    forge(sides, dev, seq, actor, kinds::REVIEW_REQUESTED, &row);
}

fn forge<P: serde::Serialize>(
    sides: &[&Side],
    dev: &str,
    seq: i64,
    actor: &str,
    kind: &str,
    row: &P,
) {
    let at = plus(sides[0].clk.now(), mins(seq as u64));
    let ev = Event {
        device_id: dev.into(),
        seq,
        hlc: hlc::Timestamp(hlc_at(at)),
        actor: actor.into(),
        kind: kind.into(),
        payload: gocompat::json::marshal(row).unwrap(),
        sig: Vec::new(),
    };
    for s in sides {
        s.st.insert(&ev).unwrap();
        s.c.fold_live(std::slice::from_ref(&ev));
    }
}

fn ask_row(card: &str, pr: &str, from: &str, to: &str, look: bool) -> ReviewRequestedRow {
    ReviewRequestedRow {
        task_id: card.into(),
        pr: pr.into(),
        from: from.into(),
        to: to.into(),
        at: common::fixed_now(),
        wake: !look,
        look,
    }
}

fn check(n: u32, review: &str, reviews: Vec<PrReview>, head: &str) -> PrCheck {
    PrCheck {
        pr: pr(n),
        state: PR_OPEN.into(),
        review: review.into(),
        reviews,
        head: head.into(),
        ..PrCheck::default()
    }
}

fn verdict(state: &str, sha: &str, s: &Side) -> PrReview {
    PrReview {
        login: "bob".into(),
        state: state.into(),
        sha: sha.into(),
        at: s.clk.now(),
    }
}

fn asked_to(p: &ReviewPhase) -> &str {
    match p {
        ReviewPhase::Asked { ask } => &ask.to,
        other => panic!("not asked: {other:?}"),
    }
}

#[test]
fn a_persons_ask_on_their_pr_card_folds_on_the_other_side() {
    let (a, b, _) = two_people();
    let id = pr_card(&[&a, &b], 7);
    let before = (a.c.ignored_rows(), b.c.ignored_rows());
    let state = a.c.get_task(&id).unwrap().state;
    let r = a.c.pick_ask(P1, &id, CARD2, false, None).unwrap().unwrap();
    assert_eq!((r.from.as_str(), r.to.as_str()), (P1, CARD2));
    assert!(r.wake && !r.look);
    assert_eq!(r.pos.dev, "dev-a");
    sync(&a, &b);
    assert_eq!(b.c.reviews(&id), vec![r.clone()]);
    assert_eq!((a.c.ignored_rows(), b.c.ignored_rows()), before);
    // No enter_review: the PR card stays as it was.
    assert_eq!(a.c.get_task(&id).unwrap().state, state);
    for s in [&a, &b] {
        assert_eq!(asked_to(&s.c.review_phase(&id)), CARD2);
    }

    // The writer's own rules: p2 can't ask on p1's card, nor p1 ask its
    // own agent, nor ask on a card that is no PR card.
    let e = b.c.pick_ask(P2, &id, CARD1, false, None).unwrap_err();
    assert!(e.is(ErrorKind::NotAllowed), "{e}");
    let e = a.c.pick_ask(P1, &id, CARD1, false, None).unwrap_err();
    assert!(e.is(ErrorKind::NotAllowed), "{e}");
    let plain = plain_card(&a, &pr(7));
    let e = a.c.pick_ask(P1, &plain, CARD2, false, None).unwrap_err();
    assert!(e.is(ErrorKind::Invalid), "{e}");
    let e = a.c.pick_ask(CARD1, &id, CARD2, false, None).unwrap_err();
    assert!(e.is(ErrorKind::NotAllowed), "{e}");
    sync(&a, &b);

    // Forged person asks: p1's from a device of p2's, p2's on p1's PR
    // card, and p1's on a plain card.
    let before = (a.c.ignored_rows(), b.c.ignored_rows());
    forge_ask(
        &[&a, &b],
        "dev-b2",
        1,
        P1,
        ask_row(&id, &pr(7), P1, CARD2, false),
    );
    forge_ask(
        &[&a, &b],
        "dev-b2",
        2,
        P2,
        ask_row(&id, &pr(7), P2, CARD1, false),
    );
    forge_ask(
        &[&a, &b],
        "dev-a2",
        1,
        P1,
        ask_row(&plain, &pr(7), P1, CARD2, false),
    );
    assert_eq!(
        (a.c.ignored_rows(), b.c.ignored_rows()),
        (before.0 + 3, before.1 + 3)
    );
    for s in [&a, &b] {
        assert_eq!(s.c.reviews(&id).len(), 1);
        assert!(s.c.reviews(&plain).is_empty());
    }
}

#[test]
fn a_third_member_cannot_ask_the_authors_own_agent() {
    let (a, b, c, _) = three_people();
    let id = pr_card(&[&a, &b, &c], 7);
    let plain = plain_card(&a, &pr(7));
    sync_all(&[&a, &b, &c]);
    for card in [&id, &plain] {
        let e = c.c.request_review(CARD4, card, &pr(7), CARD1).unwrap_err();
        assert!(e.is(ErrorKind::NotAllowed), "{e}");
        assert!(e.to_string().contains("PR author's own agent"), "{e}");
    }
    // Another person's agent is fine.
    let (rs, _) = c.c.request_review(CARD4, &id, &pr(7), CARD2).unwrap();
    assert_eq!(rs.len(), 1);
    assert_eq!(rs[0].pos.dev, "dev-c");
    sync_all(&[&a, &b, &c]);
    // A forged row from p3's own device naming p1's agent is dropped on
    // every device, on the PR card and on a card linking its PR.
    let before: Vec<usize> = [&a, &b, &c].iter().map(|s| s.c.ignored_rows()).collect();
    forge_ask(
        &[&a, &b, &c],
        "dev-c2",
        1,
        CARD4,
        ask_row(&id, &pr(7), CARD4, CARD1, false),
    );
    forge_ask(
        &[&a, &b, &c],
        "dev-c2",
        2,
        CARD4,
        ask_row(&plain, &pr(7), CARD4, CARD1, false),
    );
    for (s, n) in [&a, &b, &c].iter().zip(before) {
        assert_eq!(s.c.ignored_rows(), n + 2);
        assert!(s.c.reviews(&id).iter().all(|r| r.to != CARD1));
        assert!(s.c.reviews(&plain).is_empty());
    }
}

#[test]
fn pings_go_once_and_never_count_as_asks() {
    let (a, b, _) = two_people();
    let id = pr_card(&[&a, &b], 7);
    let p = a.c.pick_ask(P1, &id, CARD2, true, None).unwrap().unwrap();
    assert!(p.look && !p.wake);
    assert_eq!(a.c.pick_ask(P1, &id, CARD2, true, None).unwrap(), None);
    sync(&a, &b);
    // From the log: the other side's fold says the same.
    assert_eq!(b.c.reviews(&id), vec![p]);
    for s in [&a, &b] {
        assert!(s.c.reviews_of(&id).is_empty());
        assert_eq!(s.c.review_phase(&id), ReviewPhase::Waiting);
        assert!(s.c.missed_review_asks().is_empty());
    }
    // A ping doesn't stand in for the ask the caller decided on.
    let r = a.c.pick_ask(P1, &id, CARD2, false, None).unwrap().unwrap();
    assert!(!r.look);
    assert_eq!(a.c.reviews(&id).len(), 2);

    // review_refusal: a card with only a ping has nothing to review.
    let plain = plain_card(&a, "");
    a.c.claim_task(CARD1, &plain).unwrap();
    forge_ask(
        &[&a],
        "dev-a2",
        1,
        CARD1,
        ask_row(&plain, &pr(30), CARD1, CARD2, true),
    );
    let e =
        a.c.set_task_state(CARD1, &plain, IN_REVIEW, "")
            .unwrap_err();
    assert!(e.is(ErrorKind::Invalid), "{e}");
    forge_ask(
        &[&a],
        "dev-a2",
        2,
        CARD1,
        ask_row(&plain, &pr(30), CARD1, CARD2, false),
    );
    a.c.set_task_state(CARD1, &plain, IN_REVIEW, "").unwrap();

    // unreviewed_refusal: in review with only a ping, completed goes
    // through; with an ask it waits.
    for (look, seq) in [(true, 3), (false, 4)] {
        let t = plain_card(&a, &pr(31));
        a.c.claim_task(CARD1, &t).unwrap();
        a.c.set_task_state(CARD1, &t, IN_REVIEW, "").unwrap();
        forge_ask(
            &[&a],
            "dev-a2",
            seq,
            CARD1,
            ask_row(&t, &pr(31), CARD1, CARD2, look),
        );
        let done = a.c.set_task_state(CARD1, &t, COMPLETED, "");
        assert_eq!(done.is_ok(), look, "look={look}: {done:?}");
    }
}

#[test]
fn two_author_devices_ask_once_they_agree() {
    let (a, b, _) = three_people_two_cards();
    let a2 = device("dev-a2", P1, &a);
    let id = pr_card(&[&a, &a2, &b], 7);
    let r1 = a.c.pick_ask(P1, &id, CARD2, false, None).unwrap().unwrap();
    let r2 = a2.c.pick_ask(P1, &id, CARD3, false, None).unwrap().unwrap();
    sync_all(&[&a, &a2, &b]);
    let newest = std::cmp::max(&r1.pos, &r2.pos).clone();
    for s in [&a, &a2, &b] {
        match s.c.review_phase(&id) {
            ReviewPhase::Asked { ask } => assert_eq!(ask.pos, newest),
            other => panic!("{other:?}"),
        }
    }
    // Each device decided on "no ask yet": neither asks again.
    for s in [&a, &a2] {
        assert_eq!(s.c.pick_ask(P1, &id, CARD2, false, None).unwrap(), None);
        assert_eq!(s.c.pick_ask(P1, &id, CARD3, false, None).unwrap(), None);
    }
    assert_eq!(count(&a, kinds::REVIEW_REQUESTED), 2);
    // A re-pick names the ask it replaces.
    let r3 = a.c.pick_ask(P1, &id, CARD2, false, Some(newest)).unwrap();
    assert!(r3.is_some());
}

/// two_people with p2's second agent c3 on the board.
fn three_people_two_cards() -> (Side, Side, Owners) {
    let (a, b, c, owners) = three_people();
    drop(c);
    (a, b, owners)
}

#[test]
fn the_review_phase_follows_asks_claims_verdicts_and_heads() {
    let (a, b, _) = two_people();
    let id = pr_card(&[&a, &b], 7);
    assert_eq!(a.c.review_phase(&id), ReviewPhase::Waiting);
    let r = a.c.pick_ask(P1, &id, CARD2, false, None).unwrap().unwrap();
    sync(&a, &b);
    assert_eq!(asked_to(&b.c.review_phase(&id)), CARD2);
    b.c.claim_pr(CARD2, &pr(7)).unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        assert!(
            matches!(s.c.review_phase(&id), ReviewPhase::Claimed { lease } if lease.holder == CARD2)
        );
    }
    // Changes asked at the head h1: Reviewed once the claim ends.
    a.clk.add(common::secs(60));
    let v = verdict(PR_CHANGES_REQUESTED, "h1", &a);
    a.c.record_pr_state(&id, check(7, PR_CHANGES_REQUESTED, vec![v.clone()], "h1"))
        .unwrap();
    sync(&a, &b);
    assert!(matches!(b.c.review_phase(&id), ReviewPhase::Claimed { .. }));
    assert!(b.c.release_claim(CARD2, &pr(7)).unwrap());
    sync(&a, &b);
    for s in [&a, &b] {
        assert_eq!(s.c.review_phase(&id), ReviewPhase::Reviewed);
        assert_eq!(s.c.pick_facts(&id).unwrap().pr_state.head, "h1");
    }
    // A push: the verdict is at an older head, and the ask was claimed.
    a.clk.add(common::secs(60));
    a.c.record_pr_state(&id, check(7, PR_CHANGES_REQUESTED, vec![v], "h2"))
        .unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        assert_eq!(s.c.review_phase(&id), ReviewPhase::Waiting);
    }
    // A new ask after it is out again.
    a.c.pick_ask(P1, &id, CARD2, false, Some(r.pos))
        .unwrap()
        .unwrap();
    assert_eq!(asked_to(&a.c.review_phase(&id)), CARD2);

    // A decline by the asked agent after the ask: Waiting. A new ask
    // after the decline stands.
    let id8 = pr_card(&[&a, &b], 8);
    let r8 = a.c.pick_ask(P1, &id8, CARD2, false, None).unwrap().unwrap();
    sync(&a, &b);
    b.c.decline_review(CARD2, &pr(8), "not mine").unwrap();
    sync(&a, &b);
    assert_eq!(a.c.review_phase(&id8), ReviewPhase::Waiting);
    assert!(a.c.declined(&id8).contains_key(CARD2));
    a.c.pick_ask(P1, &id8, CARD2, false, Some(r8.pos))
        .unwrap()
        .unwrap();
    assert_eq!(asked_to(&a.c.review_phase(&id8)), CARD2);

    // A claim taken after the ask and let go: Waiting.
    let id9 = pr_card(&[&a, &b], 9);
    a.c.pick_ask(P1, &id9, CARD2, false, None).unwrap().unwrap();
    sync(&a, &b);
    b.c.claim_pr(CARD2, &pr(9)).unwrap();
    b.c.release_claim(CARD2, &pr(9)).unwrap();
    sync(&a, &b);
    assert_eq!(a.c.review_phase(&id9), ReviewPhase::Waiting);

    // Approved: by GitHub's decision, or with none by an approval at the
    // head. One at an older head doesn't count.
    let id10 = pr_card(&[&a, &b], 10);
    a.c.record_pr_state(&id10, check(10, "APPROVED", vec![], "h1"))
        .unwrap();
    assert_eq!(a.c.review_phase(&id10), ReviewPhase::Off);
    let id11 = pr_card(&[&a, &b], 11);
    let old = verdict("APPROVED", "h0", &a);
    a.c.record_pr_state(&id11, check(11, "", vec![old.clone()], "h1"))
        .unwrap();
    assert_eq!(a.c.review_phase(&id11), ReviewPhase::Waiting);
    a.clk.add(common::secs(60));
    let at_head = verdict("APPROVED", "h1", &a);
    a.c.record_pr_state(&id11, check(11, "", vec![old, at_head], "h1"))
        .unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        assert_eq!(s.c.review_phase(&id11), ReviewPhase::Off);
    }
    // Merged: Off.
    a.clk.add(common::secs(60));
    a.c.record_pr_state(
        &id9,
        PrCheck {
            pr: pr(9),
            state: PR_MERGED.into(),
            ..PrCheck::default()
        },
    )
    .unwrap();
    assert_eq!(a.c.review_phase(&id9), ReviewPhase::Off);
}

#[test]
fn a_decline_counts_only_from_the_agents_own_person() {
    let (a, b, _) = two_people();
    let id = pr_card(&[&a, &b], 7);
    a.c.pick_ask(P1, &id, CARD2, false, None).unwrap().unwrap();
    sync(&a, &b);
    // From a device of p1's, as p2's agent.
    let row = TaskCommentRow {
        task_id: id.clone(),
        comment_id: "01FORGEDDECLINE00000000000".into(),
        by: CARD2.into(),
        text: "Declined the review.".into(),
        at: a.clk.now(),
        declines: pr(7),
        ..TaskCommentRow::default()
    };
    forge(&[&a, &b], "dev-a2", 1, CARD2, kinds::TASK_COMMENT, &row);
    for s in [&a, &b] {
        assert!(s.c.declined(&id).is_empty());
        assert_eq!(asked_to(&s.c.review_phase(&id)), CARD2);
        let cm = s.c.comments(&id).unwrap();
        assert!(cm.iter().all(|c| c.declines.is_empty()), "{cm:?}");
    }
    let real = b.c.decline_review(CARD2, &pr(7), "").unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        let d = s.c.declined(&id);
        assert_eq!(d.keys().collect::<Vec<_>>(), [CARD2]);
        assert_eq!(d[CARD2].dev, real.device);
        assert_eq!(s.c.review_phase(&id), ReviewPhase::Waiting);
    }
}

#[test]
fn claim_counts_tally_open_and_recent_claims_by_person() {
    let (a, b, _) = three_people_two_cards();
    let id7 = pr_card(&[&a, &b], 7);
    let _id8 = pr_card(&[&a, &b], 8);
    b.c.claim_pr(CARD2, &pr(7)).unwrap();
    b.c.claim_pr(CARD3, &pr(8)).unwrap();
    sync(&a, &b);
    let want = |open, recent| ClaimCounts { open, recent };
    for s in [&a, &b] {
        assert_eq!(s.c.claim_counts().get("p2"), Some(&want(2, 2)));
        assert_eq!(s.c.claim_counts().get("p1"), None);
    }
    // A renewal adds nothing; a release keeps the 14-day count.
    b.clk.add(common::secs(60));
    b.c.claim_pr(CARD2, &pr(7)).unwrap();
    b.c.release_claim(CARD3, &pr(8)).unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        assert_eq!(s.c.claim_counts().get("p2"), Some(&want(1, 2)));
    }
    let f = a.c.pick_facts(&id7).unwrap();
    assert_eq!(f.claims.len(), 1);
    assert_eq!(f.claims[0].holder, CARD2);
    assert_eq!(
        f.live_claim.as_ref().map(|l| l.holder.as_str()),
        Some(CARD2)
    );
    // The same PR claimed again by another agent of p2 is one PR.
    b.c.claim_pr(CARD3, &pr(8)).unwrap();
    sync(&a, &b);
    assert_eq!(a.c.claim_counts().get("p2"), Some(&want(2, 2)));
    // 15 days on: the claims lapsed and dropped out of the tally.
    for s in [&a, &b] {
        s.clk.add(common::secs(15 * 24 * 3600));
    }
    for s in [&a, &b] {
        assert_eq!(s.c.claim_counts().get("p2"), None);
    }
}

#[test]
fn the_missed_ask_is_the_newest_ask_while_asked() {
    let (a, b, _) = three_people_two_cards();
    let id = pr_card(&[&a, &b], 7);
    let r1 = a.c.pick_ask(P1, &id, CARD2, false, None).unwrap().unwrap();
    let r2 =
        a.c.pick_ask(P1, &id, CARD3, false, Some(r1.pos))
            .unwrap()
            .unwrap();
    a.c.pick_ask(P1, &id, CARD2, true, None).unwrap().unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        let m = s.c.missed_review_asks();
        assert_eq!(m.len(), 1, "{m:?}");
        assert_eq!(
            (m[0].card.as_str(), m[0].to.as_str(), m[0].from.as_str()),
            (id.as_str(), CARD3, P1)
        );
        assert_eq!(m[0].pr, pr(7));
        assert_eq!(m[0].pos, r2.pos);
        assert!(m[0].wake);
        assert_eq!(s.c.wanting_review(), vec![(id.clone(), "p1".to_string())]);
    }
    b.c.claim_pr(CARD3, &pr(7)).unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        assert!(s.c.missed_review_asks().is_empty());
        assert!(s.c.wanting_review().is_empty());
    }
}

#[test]
fn board_review_with_no_to_on_a_pr_asks_nobody() {
    let (a, b, _) = two_people();
    let id = pr_card(&[&a, &b], 7);
    let (rs, _) = b.c.request_review(CARD2, &id, &pr(7), "").unwrap();
    assert!(rs.is_empty());
    // A card of p2's linking the PR: nothing asked, but the holder's card
    // goes in review as before.
    let t =
        b.c.create_task(
            P2,
            NewTask {
                title: "my part".into(),
                repo_key: common::REPO_A.into(),
                links: vec![pr(7)],
                ..NewTask::default()
            },
        )
        .unwrap();
    b.c.claim_task(CARD2, &t.id).unwrap();
    let (rs, _) = b.c.request_review(CARD2, &t.id, &pr(7), "").unwrap();
    assert!(rs.is_empty());
    assert_eq!(b.c.get_task(&t.id).unwrap().state, IN_REVIEW);
    assert_eq!(count(&b, kinds::REVIEW_REQUESTED), 0);
    // Named, the author's own agent is refused.
    let e = b.c.request_review(CARD2, &t.id, &pr(7), CARD1).unwrap_err();
    assert!(e.is(ErrorKind::NotAllowed), "{e}");
    sync(&a, &b);
    assert_eq!(a.c.review_phase(&id), ReviewPhase::Waiting);
}

fn idle(key: &str, hours: u64) -> AgentCard {
    AgentCard {
        key: key.into(),
        status: "idle".into(),
        updated_at: plus(common::fixed_now(), common::secs(3600 * hours)),
        ..Default::default()
    }
}

#[test]
fn open_reviewer_goes_by_claims_then_a_hash_of_the_pr() {
    let (mut a, b, c, _) = three_people();
    // A PR with no PR card whose hash puts c2 before c4.
    let n = (20..)
        .find(|n| {
            let h = |k: &str| fnv32a(format!("{}{k}", pr(*n)).as_bytes());
            h(CARD2) < h(CARD4)
        })
        .unwrap();
    // c2 idle longest: the old order picked it whatever the claims.
    a.with_roster(vec![idle(CARD1, 0), idle(CARD2, 1), idle(CARD4, 5)]);
    let _ = pr_card(&[&a, &b, &c], 7);
    let _ = pr_card(&[&a, &b, &c], 8);
    b.c.claim_pr(CARD2, &pr(7)).unwrap();
    sync_all(&[&a, &b, &c]);
    let ask = |a: &Side| {
        let t = plain_card(a, &pr(n));
        let (rs, _) = a.c.request_review(CARD1, &t, &pr(n), "").unwrap();
        assert_eq!(rs.len(), 1);
        rs[0].to.clone()
    };
    // p2 has a claim open, p3 none: p3's agent.
    assert_eq!(ask(&a), CARD4);
    // Equal counts: the hash, the same every time.
    c.c.claim_pr(CARD4, &pr(8)).unwrap();
    sync_all(&[&a, &b, &c]);
    assert_eq!(a.c.claim_counts().get("p2"), a.c.claim_counts().get("p3"));
    assert_eq!(ask(&a), CARD2);
    assert_eq!(ask(&a), CARD2);
}

#[test]
fn pick_facts_gather_the_cards_on_the_same_work() {
    let (a, b, _) = two_people();
    let id = pr_card(&[&a, &b], 7);
    let parent = plain_card(&a, "");
    let linking = a.c.create_task(
        P1,
        NewTask {
            title: "the work".into(),
            repo_key: common::REPO_A.into(),
            links: vec![pr(7)],
            parent: parent.clone(),
            scope: vec!["src/**".into()],
            ..NewTask::default()
        },
    );
    let linking = linking.unwrap().id;
    let sibling =
        a.c.create_task(
            P1,
            NewTask {
                title: "next to it".into(),
                parent: parent.clone(),
                ..NewTask::default()
            },
        )
        .unwrap()
        .id;
    a.c.claim_task(CARD1, &linking).unwrap();
    sync(&a, &b);
    b.c.watch(CARD2, &sibling).unwrap();
    sync(&a, &b);
    let f = b.c.pick_facts(&id).unwrap();
    assert_eq!(f.pr, pr(7));
    assert_eq!(f.author_person, "p1");
    assert!(f.created_pos.is_some());
    assert_eq!(b.c.created_pos(&id), f.created_pos);
    let why = |id: &str| f.related.iter().find(|r| r.id == id).map(|r| r.why);
    assert_eq!(why(&id), Some(RELATED_PR_CARD));
    assert_eq!(why(&linking), Some(RELATED_LINKS));
    assert_eq!(why(&parent), Some(RELATED_PARENT));
    assert_eq!(why(&sibling), Some(RELATED_SIBLING));
    let l = f.related.iter().find(|r| r.id == linking).unwrap();
    assert_eq!(l.holders, vec![CARD1.to_string()]);
    let s = f.related.iter().find(|r| r.id == sibling).unwrap();
    assert_eq!(s.watchers, vec![CARD2.to_string()]);
    assert!(f.scopes.iter().any(|(h, _, g)| h == CARD1 && g == "src/**"));
    assert_eq!(f.phase, ReviewPhase::Waiting);
    assert!(b.c.pick_facts(&linking).is_none(), "not a PR card");
}

#[test]
fn a_re_pick_in_waiting_names_the_newest_ask_from_the_facts() {
    let (a, b, _) = three_people_two_cards();
    let id = pr_card(&[&a, &b], 7);
    a.c.pick_ask(P1, &id, CARD2, false, None).unwrap().unwrap();
    sync(&a, &b);
    b.c.decline_review(CARD2, &pr(7), "not mine").unwrap();
    sync(&a, &b);
    // An ask naming nobody, newer than the one declined: it is no ask the
    // phase or pick_ask goes by, so the facts leave it out too.
    forge_ask(
        &[&a, &b],
        "dev-b2",
        1,
        CARD3,
        ask_row(&id, &pr(7), CARD3, "", false),
    );
    let f = a.c.pick_facts(&id).unwrap();
    assert_eq!(f.phase, ReviewPhase::Waiting);
    assert!(f.asks.iter().all(|r| !r.to.is_empty()), "{:?}", f.asks);
    // Waiting has no Asked ask to hand over: None writes nothing, the
    // newest ask in the facts does.
    assert_eq!(a.c.pick_ask(P1, &id, CARD3, false, None).unwrap(), None);
    let after = f.asks.last().map(|r| r.pos.clone());
    let r = a.c.pick_ask(P1, &id, CARD3, false, after).unwrap();
    assert!(r.is_some());
    assert_eq!(asked_to(&a.c.review_phase(&id)), CARD3);
}

#[test]
fn a_refold_keeps_asks_pings_claims_and_declines_once() {
    let (a, b, _) = three_people_two_cards();
    let id = pr_card(&[&a, &b], 7);
    let id8 = pr_card(&[&a, &b], 8);
    a.c.pick_ask(P1, &id, CARD2, false, None).unwrap().unwrap();
    a.c.pick_ask(P1, &id, CARD3, true, None).unwrap().unwrap();
    a.c.pick_ask(P1, &id8, CARD3, false, None).unwrap().unwrap();
    sync(&a, &b);
    b.c.claim_pr(CARD2, &pr(7)).unwrap();
    b.c.decline_review(CARD3, &pr(8), "").unwrap();
    sync(&a, &b);
    for s in [&a, &b] {
        let was = (
            s.c.reviews(&id),
            s.c.reviews(&id8),
            s.c.review_phase(&id),
            s.c.review_phase(&id8),
            s.c.claim_counts(),
            s.c.declined(&id8),
            s.c.created_pos(&id),
        );
        assert_eq!(was.0.len(), 2);
        assert!(was.6.is_some());
        s.c.refold().unwrap();
        let now = (
            s.c.reviews(&id),
            s.c.reviews(&id8),
            s.c.review_phase(&id),
            s.c.review_phase(&id8),
            s.c.claim_counts(),
            s.c.declined(&id8),
            s.c.created_pos(&id),
        );
        assert_eq!(now, was);
        assert_eq!(s.c.pick_facts(&id).unwrap().claims.len(), 1);
    }
}

#[test]
fn a_named_ask_on_a_work_card_is_the_prs_ask() {
    let (a, b, c, _) = three_people();
    let id = pr_card(&[&a, &b, &c], 7);
    // p3's work card links p1's PR and names p2's agent: that is the
    // PR's ask, so the author's device has nothing to pick.
    let w =
        c.c.create_task(
            P3,
            NewTask {
                title: "the loader".into(),
                repo_key: common::REPO_A.into(),
                links: vec![pr(7)],
                ..NewTask::default()
            },
        )
        .unwrap();
    let (rs, _) = c.c.request_review(CARD4, &w.id, &pr(7), CARD2).unwrap();
    assert_eq!(rs.len(), 1);
    sync_all(&[&a, &b, &c]);
    for s in [&a, &b, &c] {
        assert_eq!(asked_to(&s.c.review_phase(&id)), CARD2);
        let m = s.c.missed_review_asks();
        assert_eq!(m.len(), 1, "{m:?}");
        assert_eq!(
            (m[0].card.as_str(), m[0].to.as_str()),
            (w.id.as_str(), CARD2)
        );
    }
    // A pick from the author's device with a stale view writes nothing.
    assert!(a.c.pick_ask(P1, &id, CARD3, false, None).unwrap().is_none());
    let f = a.c.pick_facts(&id).unwrap();
    assert_eq!(f.asks.last().map(|r| r.to.as_str()), Some(CARD2));
}
