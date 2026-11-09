//! Picking a PR's reviewer (merge desk §12.5 to §12.7, card #1079): Alice's
//! poll device asks one of Bob's agents to review her PR, at a poll after
//! the one that made the PR card, as her person, once.
//!
//! Two made-up people on one board with rows copied by hand (bridge2's
//! harness) and a fake gh on Alice. Nothing here talks to GitHub. The
//! helpers are for the next parts' tests too.

use crate::bridge2::{Two, bridged, settle_bridged, two_people_opts, wait_cards};
use crate::common::{Rig, wait_for};
use crate::pr_cards::{ALICE, BOB, FakeGh, REPO, pr, url};

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
    set_status(t, file, key, "busy");
}

/// Bob's session `file` reads `status` ("busy" or "idle"), and his
/// presence has seen it.
pub(crate) fn set_status(t: &Two, file: &str, key: &str, status: &str) {
    let path = t.rb.claude.join("sessions").join(file);
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    v["status"] = status.into();
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    wait_for(10, "bob's session reads its status", || {
        t.b()
            .local_agents()
            .iter()
            .any(|a| a.key == key && a.status == status)
    });
}

/// Bob's session file for his local key `key`.
fn session_file(t: &Two, key: &str) -> &'static str {
    if key == t.rb.key_a {
        "900001.json"
    } else {
        assert_eq!(key, t.rb.key_b);
        "900002.json"
    }
}

/// The statuses of the `agent.card` rows for board key `actor` in `st`,
/// in log order.
fn card_statuses(st: &Store, actor: &str) -> Vec<String> {
    st.list(&Filter {
        kinds: vec![store::kinds::AGENT_CARD.into()],
        limit: 10_000,
        ..Default::default()
    })
    .unwrap()
    .iter()
    .filter(|e| e.actor == actor)
    .map(|e| {
        let v: serde_json::Value = serde_json::from_slice(&e.payload).unwrap();
        v["status"].as_str().unwrap_or_default().to_string()
    })
    .collect()
}

/// Whether `s` goes `active` and later `idle`.
fn active_then_idle(s: &[String]) -> bool {
    s.iter()
        .position(|x| x == "active")
        .is_some_and(|i| s[i..].iter().any(|x| x == "idle"))
}

/// `review_claim {pr: PR #1, action}` as Bob's local agent `key`.
fn review_claim(c: &Core, key: &str, action: &str) -> String {
    let caller = tools::Caller {
        sender: inbox::Sender::new(key, "agent", "claude"),
        path: inbox::path::MOD.into(),
    };
    let args = serde_json::json!({"pr": url(1), "action": action, "reason": "not my area"});
    c.tools()
        .call(&caller, "review_claim", &serde_json::to_vec(&args).unwrap())
        .unwrap()
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
    assert!(review_rows(&t.sa()).is_empty(), "the making poll asked");
    assert_eq!(note(), "", "tier 7 at the poll that made the card");
    for n in 1..=2 {
        poll(t.a());
        assert!(review_rows(&t.sa()).is_empty(), "poll {n} asked");
        assert_eq!(note(), "", "tier 7 at standing poll {n}");
    }
    poll(t.a());
    assert!(review_rows(&t.sa()).is_empty(), "a busy agent was asked");
    assert!(note().starts_with("nobody"), "{:?}", note());
}

// ---- Bob's side: agents put on, asks and pings delivered, the catch-up ----

/// A third session of Bob's, "gamma"; its key.
fn bobs_third(t: &Two) -> String {
    let sess = "cccccccc-3333-4333-8333-000000000003";
    crate::common::write_session(
        &t.rb.claude.join("sessions"),
        "900003.json",
        sess,
        "gamma",
        &t.rb.data,
    );
    let key = format!("{}/{sess}", t.b().device_id());
    wait_for(10, "presence lists gamma", || {
        t.b().local_agents().iter().any(|a| a.key == key)
    });
    key
}

/// Puts Bob's sessions `keys` on the board; their board keys, once Alice's
/// side reads them idle.
fn bobs_on(t: &Two, keys: &[&str]) -> Vec<String> {
    let out: Vec<String> = keys
        .iter()
        .map(|k| t.b().board_key(&t.board, k).unwrap())
        .collect();
    t.sync();
    let refs: Vec<&str> = out.iter().map(String::as_str).collect();
    wait_cards(t.a(), &t.board, &refs);
    out
}

/// Each of Bob's agents `keys` comments on the PR card (tier 1, same
/// work), and Alice's side has the comments.
fn comment_on(t: &Two, card: &Task, keys: &[String]) {
    let bco = t.b().board_coord(&t.board).unwrap();
    wait_for(10, "bob folds the PR card", || {
        bco.get_task(&card.id).is_some()
    });
    for k in keys {
        bco.add_comment(k, &card.id, "this touches my loader work", false)
            .unwrap();
    }
    t.sync();
    let aco = t.a().board_coord(&t.board).unwrap();
    wait_for(10, "alice has the comments", || {
        aco.pick_facts(&card.id).is_some_and(|f| {
            f.related
                .iter()
                .any(|r| keys.iter().all(|k| r.commenters.contains(k)))
        })
    });
}

/// The asks (pings left out) and the pings in `st`.
fn asks_and_pings(st: &Store) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    review_rows(st)
        .into_iter()
        .partition(|r| r.get("look").is_none())
}

/// Bob's idle agent goes on the board for Alice's PR card by itself
/// (card_idle_agents), one only; Alice's poll asks it and it hears the ask
/// woken, from Alice's person. Alice's agents never go on for her own PR
/// and never hear the ask.
#[test]
fn bobs_idle_agent_comes_on_for_alices_pr_and_is_asked() {
    let gh = alices_gh();
    let t = pick_world(&gh, |_| {});
    for (c, k) in [
        (t.a(), &t.ra.key_a),
        (t.a(), &t.ra.key_b),
        (t.b(), &t.rb.key_a),
        (t.b(), &t.rb.key_b),
    ] {
        c.set_agent_offer(k, true).unwrap();
    }
    let card = make_pr_card(&t);
    t.sync();
    wait_for(10, "bob folds the PR card", || {
        pr_card(t.b(), &t.board).is_some()
    });
    let cards_by = |dev: &str| {
        t.sb()
            .list(&Filter {
                kinds: vec![store::kinds::AGENT_CARD.into()],
                limit: 10_000,
                ..Default::default()
            })
            .unwrap()
            .into_iter()
            .filter(|e| e.device_id == dev)
            .map(|e| e.actor)
            .collect::<std::collections::BTreeSet<_>>()
    };
    let bob_dev = t.b().device_id().to_string();
    wait_for(10, "bob's agent on the board", || {
        t.b().sweep_coord().ok();
        !cards_by(&bob_dev).is_empty()
    });
    // One is enough: once it reads idle there, no more come.
    for _ in 0..3 {
        t.b().sweep_coord().ok();
    }
    let on = cards_by(&bob_dev);
    assert_eq!(on.len(), 1, "{on:?}");
    let b1 = on.into_iter().next().unwrap();
    let local = t.b().private_key(&b1);
    assert!(local == t.rb.key_a || local == t.rb.key_b, "{local}");

    // Alice's own agents stay off her PR's board.
    t.a().sweep_coord().ok();
    t.sync();
    wait_cards(t.a(), &t.board, &[&b1]);
    t.a().sweep_coord().ok();
    assert!(
        cards_by(t.a().device_id()).is_empty(),
        "alice's agents went on for her own PR"
    );

    poll(t.a());
    let (asks, _) = asks_and_pings(&t.sa());
    assert_eq!(asks.len(), 1, "{asks:?}");
    assert_eq!(asks[0]["to"], b1.as_str());
    t.sync();
    let ds = crate::bridge2::wait_bridged(t.b(), &local, 1);
    let d = &ds[0];
    assert_eq!(d.message.board.kind, row_kind::REVIEW);
    assert_eq!(d.mode, mode::WAKE, "{d:?}");
    assert_eq!(d.hold, "", "{d:?}");
    assert!(d.message.board.card_id.is_empty(), "from Alice's person");
    assert_eq!(d.message.board.person_id, t.a().person().id);
    assert_eq!(d.message.ref_, format!("task:{}", card.id));
    let text = &d.message.text;
    assert!(
        text.contains("Irori picked you to review")
            && text.contains(&format!("review_claim pr={}", url(1)))
            && text.contains(&format!("gh pr view {} --comments", url(1))),
        "{text}"
    );
    settle_bridged(t.b(), &local, 1);
    for k in [&t.ra.key_a, &t.ra.key_b] {
        assert!(bridged(t.a(), k).is_empty(), "alice's {k} heard it");
    }
}

/// Under receive Ask me, Bob's idle agents don't go on for Alice's PR
/// card; one put on by hand and asked has the ask held for Bob.
#[test]
fn under_ask_me_no_agent_comes_on_and_the_ask_is_held() {
    let gh = alices_gh();
    let t = pick_world(&gh, |_| {});
    crate::bridge2::set_receive(t.b(), &t.board, inbox::receive::ASK);
    t.b().set_agent_offer(&t.rb.key_a, true).unwrap();
    t.b().set_agent_offer(&t.rb.key_b, true).unwrap();
    let card = make_pr_card(&t);
    t.sync();
    wait_for(10, "bob folds the PR card", || {
        pr_card(t.b(), &t.board).is_some()
    });
    for _ in 0..3 {
        t.b().sweep_coord().ok();
    }
    let bob_cards = t
        .sb()
        .list(&Filter {
            kinds: vec![store::kinds::AGENT_CARD.into()],
            limit: 10_000,
            ..Default::default()
        })
        .unwrap()
        .into_iter()
        .filter(|e| e.device_id == t.b().device_id())
        .count();
    assert_eq!(bob_cards, 0, "an agent went on under Ask me");

    bobs_on(&t, &[&t.rb.key_a]);
    poll(t.a());
    assert_eq!(asks_and_pings(&t.sa()).0.len(), 1);
    t.sync();
    let ds = crate::bridge2::wait_bridged(t.b(), &t.rb.key_a, 1);
    assert_eq!(ds[0].message.board.kind, row_kind::REVIEW);
    assert_eq!(ds[0].hold, inbox::hold::CROSS_PERSON, "{:?}", ds[0]);
    assert_eq!(ds[0].message.ref_, format!("task:{}", card.id));
}

/// A ping reaches a related agent at its next turn, never woken, even when
/// a forged row asks to wake; each related agent is pinged once across two
/// picks.
#[test]
fn a_ping_never_wakes_and_goes_once_per_agent() {
    let gh = alices_gh();
    let t = pick_world(&gh, |_| {});
    let kc = bobs_third(&t);
    let keys = bobs_on(&t, &[&t.rb.key_a, &t.rb.key_b, &kc]);
    let card = make_pr_card(&t);
    t.sync();
    comment_on(&t, &card, &keys);

    // Pick 1: one ask, both others pinged.
    poll(t.a());
    let (asks, pings) = asks_and_pings(&t.sa());
    assert_eq!(asks.len(), 1, "{asks:?}");
    assert_eq!(pings.len(), 2, "{pings:?}");
    let first = asks[0]["to"].as_str().unwrap().to_string();
    let mut pinged: Vec<String> = pings
        .iter()
        .map(|p| p["to"].as_str().unwrap().to_string())
        .collect();
    pinged.sort();
    let mut others: Vec<String> = keys.iter().filter(|k| **k != first).cloned().collect();
    others.sort();
    assert_eq!(pinged, others);
    assert!(pings.iter().all(|p| p.get("wake").is_none()), "{pings:?}");

    // Bob's side: the ask wakes, the pings come at the next turn.
    t.sync();
    let local = |k: &str| t.b().private_key(k);
    let ds = crate::bridge2::wait_bridged(t.b(), &local(&first), 1);
    assert_eq!(ds[0].mode, mode::WAKE);
    let agent = t
        .b()
        .roster()
        .agents()
        .into_iter()
        .find(|a| a.key == local(&first))
        .unwrap()
        .name;
    for k in &others {
        let ds = crate::bridge2::wait_bridged(t.b(), &local(k), 1);
        let d = &ds[0];
        assert_eq!(d.message.board.kind, row_kind::REVIEW);
        assert_eq!(d.mode, mode::NEXT_TURN, "{d:?}");
        assert!(
            d.message.text.contains("Irori: look at")
                && d.message.text.contains(&format!("{agent} has the review")),
            "{}",
            d.message.text
        );
    }

    // The asked agent declines: pick 2 asks one of the pinged, and pings
    // nobody again.
    let bco = t.b().board_coord(&t.board).unwrap();
    bco.decline_review(&first, &url(1), "not my area").unwrap();
    t.sync();
    let aco = t.a().board_coord(&t.board).unwrap();
    // Not the phase: her background poll may re-pick at once.
    wait_for(10, "alice folds the decline", || {
        aco.declined(&card.id).contains_key(&first)
    });
    poll(t.a());
    let (asks, pings) = asks_and_pings(&t.sa());
    assert_eq!(asks.len(), 2, "{asks:?}");
    assert_eq!(pings.len(), 2, "pinged again: {pings:?}");
    let second = asks[1]["to"].as_str().unwrap().to_string();
    assert!(others.contains(&second), "{second}");

    // A forged ping that says wake still comes at the next turn.
    let quiet = others.iter().find(|k| **k != second).unwrap().clone();
    let forged = coord::rows::ReviewRequestedRow {
        task_id: card.id.clone(),
        pr: url(1),
        from: Two::person(t.a()),
        to: quiet.clone(),
        at: gocompat::time::GoTime::from_system(std::time::SystemTime::now()),
        wake: true,
        look: true,
    };
    t.sa()
        .append(&Two::person(t.a()), store::kinds::REVIEW_REQUESTED, &forged)
        .unwrap();
    t.sync();
    let ds = crate::bridge2::wait_bridged(t.b(), &local(&quiet), 2);
    assert!(ds.iter().all(|d| d.mode == mode::NEXT_TURN), "{ds:?}");
}

/// Bob's two sessions on the board under cards his device hasn't mapped to
/// them yet (as when a card's card-of row is late): his bridge can't tell
/// whose they are, so Alice's ask and ping to them are missed live. Then
/// the mappings come. Returns the world, the card, the asked and pinged
/// agents' board keys and the ask's row key.
fn missed_world(gh: &Arc<FakeGh>) -> (Two, Task, String, String, String) {
    let t = pick_world(gh, |_| {});
    let cards = [
        (
            "c0ffee00000000000000000000000001",
            "alpha",
            t.rb.key_a.clone(),
        ),
        (
            "c0ffee00000000000000000000000002",
            "beta",
            t.rb.key_b.clone(),
        ),
    ];
    for (id, name, _) in &cards {
        t.sb()
            .append(
                &format!("agent:{id}"),
                store::kinds::AGENT_CARD,
                &serde_json::json!({
                    "cardId": id, "name": name, "kind": "claude",
                    "personId": t.b().person().id, "status": "idle",
                }),
            )
            .unwrap();
    }
    let keys: Vec<String> = cards
        .iter()
        .map(|(id, _, _)| format!("agent:{id}"))
        .collect();
    t.sync();
    let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    wait_cards(t.a(), &t.board, &refs);
    let card = make_pr_card(&t);
    t.sync();
    comment_on(&t, &card, &keys);
    poll(t.a());
    let evs = t
        .sa()
        .list(&Filter {
            kinds: vec![store::kinds::REVIEW_REQUESTED.into()],
            limit: 100,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(evs.len(), 2, "one ask and one ping");
    let row = |look: bool| {
        evs.iter()
            .find(|e| {
                let v: serde_json::Value = serde_json::from_slice(&e.payload).unwrap();
                v.get("look").is_some() == look
            })
            .unwrap()
    };
    let ask = row(false);
    let to = |e: &store::Event| {
        let v: serde_json::Value = serde_json::from_slice(&e.payload).unwrap();
        v["to"].as_str().unwrap().to_string()
    };
    let (asked, pinged) = (to(ask), to(row(true)));
    let key = format!("{}/{}", ask.device_id, ask.seq);
    t.sync();
    let bco = t.b().board_coord(&t.board).unwrap();
    wait_for(10, "bob folds the ask", || {
        matches!(bco.review_phase(&card.id), ReviewPhase::Asked { .. })
    });
    for k in [&t.rb.key_a, &t.rb.key_b] {
        settle_bridged(t.b(), k, 0);
    }
    // Now Bob's device knows whose cards they are.
    for (id, _, agent) in &cards {
        t.b()
            .store()
            .append(
                "irori",
                store::kinds::BOARD_CARD_OF,
                &serde_json::json!({"boardId": t.board, "cardId": id, "agentKey": agent}),
            )
            .unwrap();
    }
    wait_for(10, "bob maps his cards", || {
        cards
            .iter()
            .all(|(id, _, agent)| t.b().private_key(&format!("agent:{id}")) == *agent)
    });
    (t, card, asked, pinged, key)
}

/// An ask missed live comes through the catch-up once, under the live
/// row's key; the missed ping never comes as an ask.
#[test]
fn an_ask_missed_live_comes_once_through_the_catch_up() {
    let gh = alices_gh();
    let (t, card, asked, pinged, key) = missed_world(&gh);
    let (la, lp) = (t.b().private_key(&asked), t.b().private_key(&pinged));
    t.b().check_prs();
    let ds = crate::bridge2::wait_bridged(t.b(), &la, 1);
    let d = &ds[0];
    assert_eq!(d.message.board.kind, row_kind::REVIEW);
    assert_eq!(d.mode, mode::WAKE, "{d:?}");
    assert_eq!(d.message.board.row, key);
    assert_eq!(d.message.ref_, format!("task:{}", card.id));
    assert!(
        d.message.text.contains("Irori picked you to review"),
        "{}",
        d.message.text
    );
    assert!(t.b().inbox().bridged(&t.board, &key, &la));
    t.b().check_prs();
    settle_bridged(t.b(), &la, 1);
    settle_bridged(t.b(), &lp, 0);
}

/// Once the asked agent claims the PR, a missed ask is never delivered.
#[test]
fn a_missed_ask_is_not_caught_up_once_the_pr_is_claimed() {
    let gh = alices_gh();
    let (t, card, asked, _, _) = missed_world(&gh);
    let bco = t.b().board_coord(&t.board).unwrap();
    bco.claim_pr(&asked, &url(1)).unwrap();
    assert!(matches!(
        bco.review_phase(&card.id),
        ReviewPhase::Claimed { .. }
    ));
    t.b().check_prs();
    for k in [&t.rb.key_a, &t.rb.key_b] {
        settle_bridged(t.b(), k, 0);
    }
}

/// A mention on a PR card tells the agent to answer on the PR by its URL,
/// never to join the card.
#[test]
fn a_mention_on_a_pr_card_points_at_the_pr() {
    let gh = alices_gh();
    let t = pick_world(&gh, |_| {});
    let card = make_pr_card(&t);
    let b1 = bobs_on(&t, &[&t.rb.key_a]).remove(0);
    let aco = t.a().board_coord(&t.board).unwrap();
    aco.person_comment(
        &Two::person(t.a()),
        &card.id,
        "@alpha what do you make of this?",
        false,
        &[coord::Mention {
            key: b1.clone(),
            name: "alpha".into(),
            gone: false,
        }],
        None,
    )
    .unwrap();
    t.sync();
    let ds = wait_for_kind(t.b(), &t.rb.key_a, row_kind::COMMENT);
    let text = &ds.message.text;
    assert!(
        text.contains(&format!("gh pr view {} --comments", url(1)))
            && text.contains(&format!("review_claim pr={}", url(1))),
        "{text}"
    );
    assert!(!text.contains("board_join"), "{text}");
}

/// The first bridged delivery of `kind` to `agent`.
fn wait_for_kind(c: &Core, agent: &str, kind: &str) -> inbox::Delivery {
    wait_for(10, "the delivery", || {
        bridged(c, agent)
            .iter()
            .any(|d| d.message.board.kind == kind)
    });
    bridged(c, agent)
        .into_iter()
        .find(|d| d.message.board.kind == kind)
        .unwrap()
}

/// A card one device mints for an agent on the person's other device names
/// that device as the host, in the row and in both devices' folds.
#[test]
fn a_card_minted_for_another_devices_agent_names_its_host() {
    use crate::common::pair::paired;
    let (a, b) = paired();
    let board = a.core.create_board("Hosts", "host").unwrap();
    wait_for(45, "b opens the board", || {
        b.core.board_coord(&board).is_ok()
    });
    let av = a.core.board_verifier(&board).unwrap();
    wait_for(45, "a knows b's device on the board", || {
        av.person_of(b.core.device_id()).as_deref() == Some(a.core.person().id.as_str())
    });
    wait_for(45, "a's roster lists b's agent", || {
        a.core.roster().agents().iter().any(|x| x.key == b.agent)
    });
    let key = a.core.board_key(&board, &b.agent).unwrap();
    let row = a
        .core
        .board_store(&board)
        .unwrap()
        .list(&Filter {
            kinds: vec![store::kinds::AGENT_CARD.into()],
            ..Default::default()
        })
        .unwrap()
        .into_iter()
        .find(|e| e.actor == key)
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&row.payload).unwrap();
    assert_eq!(row.device_id, a.core.device_id(), "a minted it");
    assert_eq!(v["host"], b.core.device_id());
    assert_eq!(a.core.board_card_host(&board, &key), b.core.device_id());
    wait_for(45, "b folds the card", || {
        !b.core.board_card_host(&board, &key).is_empty()
    });
    assert_eq!(b.core.board_card_host(&board, &key), b.core.device_id());
}

// ---- Re-picks: a turn that ended untouched, a decline, changes asked ----

/// The asked agent on Alice's view, waiting for her fold.
fn wait_asked(c: &Core, board: &str, card: &str, to: &str) {
    let co = c.board_coord(board).unwrap();
    wait_for(
        10,
        "the card reads asked",
        || matches!(co.review_phase(card), ReviewPhase::Asked { ask } if ask.to == to),
    );
}

/// Bob's asked agent takes a turn and is idle again between two of
/// Alice's polls, saying nothing: her roster reads it idle as before, but
/// its card rows show the turn ended untouched, so her next poll asks his
/// other agent; later polls never ask the first again.
#[test]
fn a_turn_that_ended_untouched_between_two_polls_re_picks() {
    let gh = alices_gh();
    let t = pick_world(&gh, |_| {});
    let keys = bobs_on(&t, &[&t.rb.key_a, &t.rb.key_b]);
    let card = make_pr_card(&t);
    poll(t.a());
    let (asks, _) = asks_and_pings(&t.sa());
    assert_eq!(asks.len(), 1, "{asks:?}");
    let first = asks[0]["to"].as_str().unwrap().to_string();
    let other = keys.iter().find(|k| **k != first).unwrap().clone();
    // Bob has the ask before the turn, so his card rows come after it.
    t.sync();
    wait_asked(t.b(), &t.board, &card.id, &first);
    poll(t.a());
    assert_eq!(asks_and_pings(&t.sa()).0.len(), 1, "re-picked with no turn");

    let local = t.b().private_key(&first);
    let file = session_file(&t, &local);
    set_status(&t, file, &local, "busy");
    wait_for(10, "bob's card row reads active", || {
        card_statuses(&t.sb(), &first).iter().any(|s| s == "active")
    });
    set_status(&t, file, &local, "idle");
    wait_for(10, "bob's card row reads idle again", || {
        active_then_idle(&card_statuses(&t.sb(), &first))
    });
    t.sync();
    wait_for(10, "alice has the turn's rows", || {
        active_then_idle(&card_statuses(&t.sa(), &first))
    });
    wait_cards(t.a(), &t.board, &[&first]);

    poll(t.a());
    let (asks, _) = asks_and_pings(&t.sa());
    assert_eq!(asks.len(), 2, "{asks:?}");
    assert_eq!(asks[1]["to"], other.as_str());
    wait_asked(t.a(), &t.board, &card.id, &other);
    for _ in 0..3 {
        poll(t.a());
    }
    let (asks, _) = asks_and_pings(&t.sa());
    assert_eq!(asks.len(), 2, "{asks:?}");
}

/// Bob's agent that works on the PR is asked first, and declines through
/// review_claim. Alice's next poll asks his other agent, though the one
/// that declined still ranks first and isn't pinged either. Once the
/// other declines too, no poll asks anyone again.
#[test]
fn a_declined_agent_is_never_asked_again_for_the_pr() {
    let gh = alices_gh();
    let t = pick_world(&gh, |_| {});
    let keys = bobs_on(&t, &[&t.rb.key_a, &t.rb.key_b]);
    let card = make_pr_card(&t);
    t.sync();
    comment_on(&t, &card, &keys[..1]);
    poll(t.a());
    let (asks, _) = asks_and_pings(&t.sa());
    assert_eq!(asks.len(), 1, "{asks:?}");
    assert_eq!(asks[0]["to"], keys[0].as_str(), "tier 1 first");

    let aco = t.a().board_coord(&t.board).unwrap();
    for (n, k) in keys.iter().enumerate() {
        let out = review_claim(t.b(), &t.b().private_key(k), "decline");
        assert!(out.contains("won't ask you again"), "{out}");
        t.sync();
        // Not the phase: her background poll may re-pick at once.
        wait_for(10, "alice folds the decline", || {
            aco.declined(&card.id).contains_key(k)
        });
        for _ in 0..3 {
            poll(t.a());
        }
        let (asks, pings) = asks_and_pings(&t.sa());
        assert!(pings.is_empty(), "a decliner was pinged: {pings:?}");
        assert_eq!(asks.len(), 2, "after decline {n}: {asks:?}");
        assert_eq!(asks[1]["to"], keys[1].as_str());
    }
    assert_eq!(aco.review_phase(&card.id), ReviewPhase::Waiting);
}

/// Bob's agent claims Alice's PR and his login asks for changes at the
/// head: no poll asks anyone while the PR sits at that head, though his
/// other agent now works on it. A push: the next poll asks the same agent
/// again, and pings the other.
#[test]
fn after_changes_requested_no_ask_until_a_new_head_then_the_same_agent() {
    let gh = alices_gh();
    let gb = FakeGh::new(BOB, vec![]);
    let g = gb.clone();
    let t = pick_world(&gh, move |o| o.desk_lister = Some(g));
    // Alice knows Bob's login: his verdicts map to his agents' claims.
    // His desk check learns it from gh; an identity change (here, his
    // colour) writes his profile, which this harness's join never did.
    t.b().check_desks();
    t.b().set_person_color("pink").unwrap();
    let bob = t.b().person().id;
    let av = t.a().board_verifier(&t.board).unwrap();
    wait_for(10, "alice reads bob's login", || {
        t.sync();
        av.profile_github_login(&bob).eq_ignore_ascii_case(BOB)
    });
    let keys = bobs_on(&t, &[&t.rb.key_a, &t.rb.key_b]);
    let card = make_pr_card(&t);
    poll(t.a());
    let (asks, _) = asks_and_pings(&t.sa());
    assert_eq!(asks.len(), 1, "{asks:?}");
    let first = asks[0]["to"].as_str().unwrap().to_string();
    let other = keys.iter().find(|k| **k != first).unwrap().clone();
    t.sync();
    // The other agent works on the PR: on rank alone it comes first now.
    comment_on(&t, &card, std::slice::from_ref(&other));

    let bco = t.b().board_coord(&t.board).unwrap();
    bco.claim_pr(&first, &url(1)).unwrap();
    t.sync();
    let aco = t.a().board_coord(&t.board).unwrap();
    wait_for(10, "alice reads the claim", || {
        matches!(aco.review_phase(&card.id), ReviewPhase::Claimed { .. })
    });
    let since = aco.claim_of(&url(1)).unwrap().acquired_at;
    let changes = coord::PrReview {
        login: BOB.into(),
        state: coord::PR_CHANGES_REQUESTED.into(),
        sha: "feedbeefcafe".into(),
        at: gocompat::time::GoTime::unix_milli(since.unix_milli_value() + 60_000),
    };
    let push = |head: &str| {
        gh.act.lock().unwrap().head = head.into();
        let mut prs = gh.prs.lock().unwrap();
        prs[0].head_ref_oid = head.into();
        prs[0].review_decision = coord::PR_CHANGES_REQUESTED.into();
        prs[0].updated_at =
            gocompat::time::GoTime::unix_milli(prs[0].updated_at.unix_milli_value() + 1_000);
    };
    gh.act.lock().unwrap().reviews = vec![changes];
    push("feedbeefcafe");
    poll(t.a());
    assert!(matches!(
        aco.review_phase(&card.id),
        ReviewPhase::Claimed { .. }
    ));
    bco.release_claim(&first, &url(1)).unwrap();
    t.sync();
    wait_for(10, "alice reads the PR reviewed", || {
        aco.review_phase(&card.id) == ReviewPhase::Reviewed
    });
    for _ in 0..3 {
        poll(t.a());
    }
    let (asks, pings) = asks_and_pings(&t.sa());
    assert_eq!(asks.len(), 1, "asked at the reviewed head: {asks:?}");
    assert!(pings.is_empty(), "{pings:?}");

    push("0ddba11c0ffee");
    poll(t.a());
    let (asks, pings) = asks_and_pings(&t.sa());
    assert_eq!(asks.len(), 2, "{asks:?}");
    assert_eq!(asks[1]["to"], first.as_str(), "the same agent");
    assert_eq!(pings.len(), 1, "{pings:?}");
    assert_eq!(pings[0]["to"], other.as_str());
    wait_asked(t.a(), &t.board, &card.id, &first);
}
