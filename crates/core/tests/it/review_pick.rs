//! Picking a PR's reviewer (merge desk §12.5 to §12.7, card #1079): Alice's
//! poll device asks one of Bob's agents to review her PR, at a poll after
//! the one that made the PR card, as her person, once.
//!
//! Two made-up people on one board with rows copied by hand (bridge2's
//! harness) and a fake gh on Alice. Nothing here talks to GitHub. The
//! helpers are for the next parts' tests too.

use crate::bridge2::{Two, bridged, settle_bridged, two_people_opts, wait_cards};
use crate::common::{Rig, wait_for};
use crate::pr_cards::{ALICE, FakeGh, REPO, pr, url};

use std::sync::Arc;

use coord::{NewTask, ReviewPhase, Task, TaskFilter, new_desk};
use inbox::{mode, row_kind};
use irori_core::Core;
use store::{Filter, Store};

/// Alice's fake gh lists her PR #1, open and in need of a review.
pub(crate) fn alices_gh() -> Arc<FakeGh> {
    FakeGh::new(ALICE, vec![pr(1, ALICE, 5_000_000_000_000)])
}

/// Two people on one board, `gh` as Alice's desk lister, `edit` on Bob's
/// options; Alice's desk is on and polled once (so her login is known).
/// The space works in the repo (a done card of Alice's in it), and Alice's
/// board store holds a hello from Bob's device saying it reads PR cards,
/// so the gate (§12.2) passes with no mesh.
pub(crate) fn pick_world(gh: &Arc<FakeGh>, edit: impl FnOnce(&mut irori_core::Options)) -> Two {
    let g = gh.clone();
    let t = two_people_opts(Rig::with(move |o| o.desk_lister = Some(g)), edit);
    desk_on(t.a());
    poll(t.a());
    space_works_in_repo(&t);
    hello_reads_pr_cards(t.a(), &t.board, t.b().device_id());
    t
}

/// Turns `c`'s merge desk on for [`REPO`], polled by `c`'s device.
pub(crate) fn desk_on(c: &Core) {
    c.coord()
        .set_desk(
            &format!("user:{}", c.device_id()),
            new_desk(REPO, c.device_id()),
        )
        .unwrap();
}

/// One desk poll of [`REPO`]: the PR cards, then the pick.
pub(crate) fn poll(c: &Core) {
    let _ = c.poll_desk(REPO);
}

/// Alice's done card in [`REPO`] on the board: the space works there.
pub(crate) fn space_works_in_repo(t: &Two) -> Task {
    let co = t.a().board_coord(&t.board).unwrap();
    let me = Two::person(t.a());
    let task = co
        .create_task(
            &me,
            NewTask {
                title: "loader rewrite".into(),
                repo_key: REPO.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.set_task_state(&me, &task.id, coord::COMPLETED, "done")
        .unwrap();
    task
}

/// Saves in `c`'s board store a hello from device `dev` that says it reads
/// PR cards (the gate, §12.2).
pub(crate) fn hello_reads_pr_cards(c: &Core, board: &str, dev: &str) {
    c.board_store(board)
        .unwrap()
        .append_local(
            "irori",
            mesh::boardmesh::KIND_BOARD_HELLO,
            &mesh::boardmesh::SavedHello {
                device_id: dev.into(),
                core_version: "0.13.0".into(),
                spaces: 1,
                tolerant: 2,
                reviews: mesh::boardmesh::REVIEWS_PROTOCOL,
                seq: i64::MAX,
                ..Default::default()
            },
        )
        .unwrap();
}

/// The PR card for PR #1 on `c`'s view of the board, if any.
pub(crate) fn pr_card(c: &Core, board: &str) -> Option<Task> {
    c.board_coord(board)
        .unwrap()
        .list_tasks(&TaskFilter::default())
        .into_iter()
        .find(|t| t.pr_card.as_ref().is_some_and(|p| p.pr == url(1)))
}

/// Polls Alice until her PR card is made; it is the poll that made it.
pub(crate) fn make_pr_card(t: &Two) -> Task {
    wait_for(10, "the PR card", || {
        poll(t.a());
        pr_card(t.a(), &t.board).is_some()
    });
    pr_card(t.a(), &t.board).unwrap()
}

/// The `review.requested` rows in `st`, asks and pings, as JSON.
pub(crate) fn review_rows(st: &Store) -> Vec<serde_json::Value> {
    st.list(&Filter {
        kinds: vec![store::kinds::REVIEW_REQUESTED.into()],
        limit: 10_000,
        ..Default::default()
    })
    .unwrap()
    .iter()
    .map(|ev| serde_json::from_slice(&ev.payload).unwrap())
    .collect()
}

/// Bob's session `file` (900001.json is `rb.key_a`) reads busy, and his
/// presence has seen it.
pub(crate) fn make_busy(t: &Two, file: &str, key: &str) {
    let path = t.rb.claude.join("sessions").join(file);
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    v["status"] = "busy".into();
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    wait_for(10, "bob's session reads busy", || {
        t.b()
            .local_agents()
            .iter()
            .any(|a| a.key == key && a.status == "busy")
    });
}

/// Alice's PR card is made with Bob's idle agent already on the board, and
/// asks nobody; the next poll asks that agent, as Alice's person, waking
/// it; more polls ask nobody else. Bob's side folds the ask.
#[test]
fn alices_poll_asks_one_of_bobs_agents_once() {
    let gh = alices_gh();
    let t = pick_world(&gh, |_| {});
    let b1 = t.b().board_key(&t.board, &t.rb.key_a).unwrap();
    t.sync();
    wait_cards(t.a(), &t.board, &[&b1]);

    let card = make_pr_card(&t);
    assert!(
        review_rows(&t.sa()).is_empty(),
        "the poll that made the card asked: {:?}",
        review_rows(&t.sa())
    );

    poll(t.a());
    let rows = review_rows(&t.sa());
    assert_eq!(rows.len(), 1, "{rows:?}");
    let r = &rows[0];
    assert_eq!(
        r["from"],
        Two::person(t.a()).as_str(),
        "from Alice's person"
    );
    assert_eq!(r["to"], b1.as_str());
    assert_eq!(r["taskId"], card.id.as_str());
    assert_eq!(r["pr"], url(1).as_str());
    assert_eq!(r["wake"], true);
    assert!(r.get("look").is_none(), "an ask, not a ping");
    let aco = t.a().board_coord(&t.board).unwrap();
    assert!(
        matches!(aco.review_phase(&card.id), ReviewPhase::Asked { ask } if ask.to == b1),
        "{:?}",
        aco.review_phase(&card.id)
    );
    assert_eq!(
        aco.get_task(&card.id).unwrap().state,
        card.state,
        "no review state"
    );

    // Asked: further polls ask nobody.
    for _ in 0..3 {
        poll(t.a());
    }
    assert_eq!(review_rows(&t.sa()).len(), 1, "a second ask");

    // Bob's board folds the ask from Alice's person, and the bridge wakes
    // his agent with it; Alice's own agents hear nothing.
    let ignored = t.b().board_coord(&t.board).unwrap().ignored_rows();
    t.sync();
    let bco = t.b().board_coord(&t.board).unwrap();
    wait_for(10, "bob folds the ask", || {
        bco.pick_facts(&card.id).is_some_and(|f| f.asks.len() == 1)
    });
    assert_eq!(bco.ignored_rows(), ignored, "bob's fold dropped a row");
    wait_for(10, "the ask reaches bob's agent", || {
        !bridged(t.b(), &t.rb.key_a).is_empty()
    });
    settle_bridged(t.b(), &t.rb.key_a, 1);
    let d = bridged(t.b(), &t.rb.key_a).pop().unwrap();
    assert_eq!(d.message.board.kind, row_kind::REVIEW);
    assert_eq!(d.mode, mode::WAKE);
    assert!(bridged(t.b(), &t.rb.key_b).is_empty(), "only the one asked");
    assert!(bridged(t.a(), &t.ra.key_a).is_empty());
    assert!(bridged(t.a(), &t.ra.key_b).is_empty());
}

/// Bob's only agent on the board is busy and nothing ties it to the PR: no
/// tier fits. The first standing poll asks nobody and doesn't go on to
/// tier 7; after three standing polls with no new agent of Bob's on the
/// board, the desk says nobody is free (B7 starts a new reviewer there).
#[test]
fn a_busy_agent_with_no_signal_is_not_asked_and_tier_7_waits() {
    let gh = alices_gh();
    let t = pick_world(&gh, |_| {});
    make_busy(&t, "900001.json", &t.rb.key_a);
    let b1 = t.b().board_key(&t.board, &t.rb.key_a).unwrap();
    t.sync();
    wait_for(10, "alice reads bob's agent busy", || {
        t.a()
            .board_roster(&t.board)
            .iter()
            .any(|a| a.key == b1 && a.status == "active" && a.device_state != "offline")
    });
    make_pr_card(&t);
    let note = || {
        t.a()
            .desk_verdicts(REPO)
            .into_iter()
            .find(|v| v.pr == 1)
            .map(|v| v.review_note)
            .unwrap_or_default()
    };
    for n in 1..=2 {
        poll(t.a());
        assert!(review_rows(&t.sa()).is_empty(), "poll {n} asked");
        assert_eq!(note(), "", "tier 7 at standing poll {n}");
    }
    poll(t.a());
    assert!(review_rows(&t.sa()).is_empty(), "a busy agent was asked");
    assert!(note().starts_with("nobody"), "{:?}", note());
}
