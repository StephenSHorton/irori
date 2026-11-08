//! Card #786, job (a): Irori's sweeps keep a record of what they did, with
//! an Undo. Hand-backs get a line, a card whose PRs merged is asked about
//! instead of re-offered, and a lease on an offline device is told, never
//! released.

use crate::common;

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use common::*;
use coord::rows::{DidRow, LeaseAcquiredRow};
use coord::*;
use store::kinds;

const PR: &str = "https://github.com/a/b/pull/7";
const SPAWNED: &str = "dev-a/spawned-session";
const OWNER_KEY: &str = "irori";

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

fn tidy_env() -> Env {
    let e = Env::new();
    e.c.set_tidy(true);
    e
}

/// A card ALICE made and BOB holds, working.
fn card(e: &Env) -> Task {
    let t = e.c.create_task(ALICE, nt("ship it")).unwrap();
    e.c.claim_task(BOB, &t.id).unwrap();
    t
}

fn linked_merged(e: &Env, id: &str, state: &str) {
    e.c.add_link(BOB, id, PR).unwrap();
    e.c.record_pr_state(
        id,
        PrCheck {
            pr: PR.into(),
            state: state.into(),
            ..PrCheck::default()
        },
    )
    .unwrap();
}

/// BOB drops off the roster and the holder grace runs out.
fn bob_goes(e: &Env) {
    e.ros.set(BOB, "");
    e.c.sweep_notices().unwrap();
    e.clk.add(HOLDER_GRACE + Duration::from_secs(1));
    e.c.sweep_notices().unwrap();
}

fn got(e: &Env, id: &str) -> Task {
    e.c.get_task(id).unwrap()
}

fn only(t: &Task, job: &str) -> Did {
    let v: Vec<&Did> = t.did.iter().filter(|d| d.job == job).collect();
    assert_eq!(v.len(), 1, "{:?}", t.did);
    v[0].clone()
}

#[test]
fn a_hand_back_writes_its_record_and_replays() {
    let e = tidy_env();
    let t = card(&e);
    bob_goes(&e);
    let g = got(&e, &t.id);
    assert_eq!(g.state, SUBMITTED, "the sweep's hand-back still happens");
    let d = only(&g, JOB_HANDED);
    assert_eq!(d.by, "irori");
    assert!(d.text.starts_with("Irori handed this back:"), "{}", d.text);
    assert!(d.text.ends_with("went away"), "{}", d.text);
    // A restart re-reads it from the log, and finds nothing to do again.
    let back = e.reopen();
    back.set_tidy(true);
    let g = back.get_task(&t.id).unwrap();
    assert_eq!(only(&g, JOB_HANDED).id, d.id);
    back.sweep_notices().unwrap();
    assert_eq!(e.rows("assistant.did").len(), 1);
}

#[test]
fn off_means_no_record_but_the_sweep_still_hands_back() {
    let e = Env::new();
    let t = card(&e);
    linked_merged(&e, &t.id, PR_MERGED);
    bob_goes(&e);
    let g = got(&e, &t.id);
    assert!(g.did.is_empty(), "{:?}", g.did);
    assert_eq!(g.state, SUBMITTED, "the merged check needs the switch");
    assert!(e.rows("assistant.did").is_empty());
}

#[test]
fn a_finished_agent_has_no_line() {
    let e = tidy_env();
    let t = e.c.create_task(BOB, nt("spawned")).unwrap();
    e.ros.set(SPAWNED, "idle");
    let g =
        e.c.request_spawn(
            OWNER_KEY,
            SpawnAsk {
                task_id: t.id.clone(),
                permission_mode: "default".into(),
                ..SpawnAsk::default()
            },
        )
        .unwrap()
        .unwrap();
    e.c.bind_spawn(&t.id, g, SPAWNED).unwrap();
    e.c.claim_task(SPAWNED, &t.id).unwrap();
    e.st.append(
        "irori",
        kinds::AGENT_GONE,
        &serde_json::json!({ "key": SPAWNED, "reason": "finished" }),
    )
    .unwrap();
    e.c.mark_ended(SPAWNED, e.clk.now());
    e.ros.set(SPAWNED, "");
    e.c.sweep_notices().unwrap();
    e.clk.add(ENDED_GRACE + Duration::from_secs(1));
    e.c.sweep_notices().unwrap();
    let g = got(&e, &t.id);
    assert!(g.holders.is_empty());
    assert!(
        g.did.iter().all(|d| d.job != JOB_HANDED),
        "no line for finished: {:?}",
        g.did
    );
}

#[test]
fn undo_of_a_hand_back_holds_the_card_and_stops_the_offer() {
    let e = tidy_env();
    let t = card(&e);
    bob_goes(&e);
    assert!(e.c.needs_agent(&t.id), "offered again before Undo");
    let d = only(&got(&e, &t.id), JOB_HANDED);
    assert!(e.c.did_can_undo(&got(&e, &t.id), &d));
    e.c.undo_did(USER, &t.id, &d.id).unwrap();
    let g = got(&e, &t.id);
    assert_eq!(g.state, BLOCKED);
    assert!(g.parked().is_some(), "held as Not yet");
    assert!(
        !e.c.needs_agent(&t.id),
        "Not yet takes it out of the hand-out"
    );
    assert!(only(&g, JOB_HANDED).undone);
    // Comes back on the person's word, as any Not yet.
    e.c.bring_back(USER, &t.id, "").unwrap();
    assert!(e.c.needs_agent(&t.id), "brought back, offered again");
}

#[test]
fn undo_is_gone_once_someone_holds_the_card() {
    let e = tidy_env();
    let t = card(&e);
    bob_goes(&e);
    let d = only(&got(&e, &t.id), JOB_HANDED);
    e.c.claim_task(CAROL, &t.id).unwrap();
    assert!(!e.c.did_can_undo(&got(&e, &t.id), &d));
    want_err(e.c.undo_did(USER, &t.id, &d.id), ErrorKind::BadState);
    assert_eq!(got(&e, &t.id).state, WORKING, "the holder stays");
}

#[test]
fn only_the_person_undoes() {
    let e = tidy_env();
    let t = card(&e);
    bob_goes(&e);
    let d = only(&got(&e, &t.id), JOB_HANDED);
    want_err(e.c.undo_did(ALICE, &t.id, &d.id), ErrorKind::NotAllowed);
}

#[test]
fn a_taker_gone_from_an_ask_gets_a_line_with_no_undo() {
    let e = tidy_env();
    let (tk, _) =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "Look at the crash report".into(),
                ..NewAsk::default()
            },
            &[],
        )
        .unwrap();
    assert!(
        e.c.ask_claim(ALICE, &tk.id, Duration::from_secs(1))
            .unwrap()
            .mine
    );
    e.ros.set(ALICE, "");
    e.c.sweep_asks(&[]).unwrap();
    e.clk.add(GONE_GRACE + Duration::from_secs(1));
    e.c.sweep_asks(&[]).unwrap();
    let g = got(&e, &tk.id);
    assert_eq!(g.ask.as_ref().unwrap().taker, "");
    let d = only(&g, JOB_HANDED_ASK);
    assert!(!e.c.did_can_undo(&g, &d), "a Not yet doesn't hold an ask");
    assert!(d.text.ends_with("went away"), "{}", d.text);
}

#[test]
fn a_merged_card_is_asked_about_instead_of_handed_back() {
    let e = tidy_env();
    let t = card(&e);
    linked_merged(&e, &t.id, PR_MERGED);
    bob_goes(&e);
    let g = got(&e, &t.id);
    assert_eq!(g.state, BLOCKED, "never back on the Board");
    assert!(!e.c.needs_agent(&t.id), "no new agent on finished work");
    let d = only(&g, JOB_MERGED);
    assert!(g.did.iter().all(|x| x.job != JOB_HANDED), "{:?}", g.did);
    let b = g.blockers.iter().find(|b| b.id == d.blocker_id).unwrap();
    assert!(b.done_check && b.open(), "{b:?}");
    assert_eq!(b.raised_by, "irori");
    // Done closes it.
    let closed = e.c.say_done(USER, &t.id, &b.id, &[]).unwrap().0;
    assert_eq!(closed.state, COMPLETED);
}

#[test]
fn undo_of_a_merged_flag_clears_it_and_stops_it_coming_back() {
    let e = tidy_env();
    let t = card(&e);
    linked_merged(&e, &t.id, PR_MERGED);
    bob_goes(&e);
    let d = only(&got(&e, &t.id), JOB_MERGED);
    e.c.undo_did(USER, &t.id, &d.id).unwrap();
    let g = got(&e, &t.id);
    assert!(
        !g.blockers.iter().any(|b| b.open() && b.done_check),
        "{:?}",
        g.blockers
    );
    assert_eq!(g.state, BLOCKED, "held, not back on the Board");
    assert!(g.parked().is_some(), "on the person's word");
    assert!(!e.c.needs_agent(&t.id), "no new agent on finished work");
    assert!(only(&g, JOB_MERGED).undone);
    assert!(!e.c.flag_merged(&t.id).unwrap(), "the record stops it");
    assert_eq!(only(&got(&e, &t.id), JOB_MERGED).id, d.id);
    assert_eq!(e.rows("assistant.did").len(), 2, "record and undo only");
}

/// A held card with its one PR merged, that nobody is on.
fn holderless_merged(e: &Env) -> Task {
    let t = card(e);
    linked_merged(e, &t.id, PR_MERGED);
    e.c.leave(BOB, &t.id, BOB).unwrap();
    got(e, &t.id)
}

#[test]
fn the_listener_path_flags_a_holderless_card_once() {
    let e = tidy_env();
    let t = holderless_merged(&e);
    assert!(e.c.flag_merged(&t.id).unwrap());
    assert!(!e.c.flag_merged(&t.id).unwrap(), "asked once");
    assert_eq!(e.rows("assistant.did").len(), 1);
    assert_eq!(
        got(&e, &t.id)
            .blockers
            .iter()
            .filter(|b| b.open() && b.done_check)
            .count(),
        1
    );
}

#[test]
fn a_restart_doesnt_ask_again() {
    let e = tidy_env();
    let t = holderless_merged(&e);
    assert!(e.c.flag_merged(&t.id).unwrap());
    let c2 = e.reopen();
    c2.set_tidy(true);
    assert!(!c2.flag_merged(&t.id).unwrap());
    assert_eq!(e.rows("assistant.did").len(), 1);
}

#[test]
fn exclusions_from_the_merged_flag() {
    let e = tidy_env();
    // A live holder.
    let t = card(&e);
    linked_merged(&e, &t.id, PR_MERGED);
    assert!(!e.c.flag_merged(&t.id).unwrap(), "held");
    e.c.leave(BOB, &t.id, BOB).unwrap();
    // Switch off.
    let t2 = holderless_merged(&e);
    e.c.set_tidy(false);
    assert!(!e.c.flag_merged(&t2.id).unwrap(), "off");
    e.c.set_tidy(true);
    // An open done check: raised by the agent's completed.
    let e2 = {
        let mut e2 = Env::new();
        e2.c = e2.reopen_with(Options {
            done_check: true,
            ..e2.opts()
        });
        e2.c.set_tidy(true);
        e2
    };
    let t3 = card(&e2);
    linked_merged(&e2, &t3.id, PR_MERGED);
    e2.c.set_task_state(BOB, &t3.id, COMPLETED, "tests pass")
        .unwrap();
    e2.c.leave(BOB, &t3.id, BOB).unwrap();
    assert!(!e2.c.flag_merged(&t3.id).unwrap(), "done check open");
    // No PR at all, and a PR still open.
    let t4 = e.c.create_task(ALICE, nt("no pr")).unwrap();
    assert!(!e.c.flag_merged(&t4.id).unwrap(), "no PR");
    let t5 = card(&e);
    linked_merged(&e, &t5.id, PR_OPEN);
    e.c.leave(BOB, &t5.id, BOB).unwrap();
    assert!(!e.c.flag_merged(&t5.id).unwrap(), "still open");
    // A parent with open subtasks.
    let t6 = holderless_merged(&e);
    e.c.create_task(
        ALICE,
        NewTask {
            parent: t6.id.clone(),
            ..nt("kid")
        },
    )
    .unwrap();
    assert!(!e.c.flag_merged(&t6.id).unwrap(), "open subtask");
    // Terminal.
    let t7 = holderless_merged(&e);
    e.c.set_task_state(USER, &t7.id, COMPLETED, "").unwrap();
    assert!(!e.c.flag_merged(&t7.id).unwrap(), "terminal");
    // No records anywhere for what it left alone.
    assert!(e.rows("assistant.did").is_empty());
}

#[test]
fn two_devices_make_one_record() {
    let e = tidy_env();
    let t = holderless_merged(&e);
    assert!(e.c.flag_merged(&t.id).unwrap());
    let d = only(&got(&e, &t.id), JOB_MERGED);
    // The same record from dev-b, as if both heard it at once.
    e.remote(
        "dev-b",
        1,
        hlc_at(e.clk.now()),
        "irori",
        kinds::ASSISTANT_DID,
        &DidRow {
            id: d.id.clone(),
            task_id: t.id.clone(),
            job: JOB_MERGED.into(),
            key: d.key.clone(),
            text: "the same words from dev-b".into(),
            by: "irori".into(),
            at: e.clk.now(),
            ..DidRow::default()
        },
    );
    let g = got(&e, &t.id);
    assert_eq!(g.did.len(), 1, "{:?}", g.did);
    assert_eq!(g.did[0].text, d.text, "the first to fold stands");
}

#[test]
fn an_undo_from_another_device_counts_even_before_its_record() {
    let e = tidy_env();
    let t = e.c.create_task(ALICE, nt("x")).unwrap();
    let id = did_id(JOB_MERGED, &t.id, "k");
    e.remote(
        "dev-b",
        1,
        hlc_at(e.clk.now()),
        "user",
        kinds::ASSISTANT_DID,
        &DidRow {
            id: id.clone(),
            task_id: t.id.clone(),
            undo: true,
            by: USER.into(),
            at: e.clk.now(),
            ..DidRow::default()
        },
    );
    e.remote(
        "dev-b",
        2,
        hlc_at(e.clk.now()) + 1,
        "irori",
        kinds::ASSISTANT_DID,
        &DidRow {
            id: id.clone(),
            task_id: t.id.clone(),
            job: JOB_MERGED.into(),
            key: "k".into(),
            text: "asked".into(),
            by: "irori".into(),
            at: e.clk.now(),
            ..DidRow::default()
        },
    );
    let g = got(&e, &t.id);
    assert_eq!(g.did.len(), 1);
    assert!(g.did[0].undone);
    assert_eq!(g.did[0].text, "asked");
}

fn away_env(away: bool) -> Env {
    let mut e = Env::new();
    let now = e.clk.now().unix_nano();
    let d = AgentDevices {
        away: if away {
            HashSet::from(["dev-b".to_string()])
        } else {
            HashSet::new()
        },
        away_since: BTreeMap::from([("dev-b".to_string(), now)]),
        ..AgentDevices::default()
    };
    e.c = e.reopen_with(Options {
        agent_devices: Some(Arc::new(move || d.clone())),
        ..e.opts()
    });
    e.c.set_tidy(true);
    e
}

fn lease_on_b(e: &Env, resource: &str, ttl: Duration) {
    e.remote(
        "dev-b",
        1,
        hlc_at(e.clk.now()),
        "dev-b/agent",
        kinds::LEASE_ACQUIRED,
        &LeaseAcquiredRow {
            lease: Lease {
                resource: resource.into(),
                holder: "dev-b/agent".into(),
                acquired_at: e.clk.now(),
                until: plus(e.clk.now(), ttl),
                ..Lease::default()
            },
            ..LeaseAcquiredRow::default()
        },
    );
}

#[test]
fn a_lease_on_an_offline_device_is_told_never_released() {
    let e = away_env(true);
    lease_on_b(&e, "ww/slot-1", Duration::from_secs(60));
    // Not past its time yet, and not away long enough.
    e.c.sweep_notices().unwrap();
    assert!(e.c.lease_notices().is_empty());
    e.clk.add(LEASE_OFFLINE_AFTER + Duration::from_secs(5));
    e.c.sweep_notices().unwrap();
    let n = e.c.lease_notices();
    assert_eq!(n.len(), 1, "{n:?}");
    assert_eq!(n[0].job, JOB_LEASE);
    assert_eq!(e.c.leases().len(), 1, "never released automatically");
    e.c.sweep_notices().unwrap();
    assert_eq!(e.rows("assistant.did").len(), 1, "told once");

    // The person's Release is the only way it goes.
    e.c.steal_lease(USER, "ww/slot-1").unwrap();
    assert!(e.c.leases().is_empty());
    assert!(e.c.lease_notices().is_empty(), "nothing left to tell");
}

#[test]
fn undo_of_a_lease_notice_takes_it_back_and_it_stays_back() {
    let e = away_env(true);
    lease_on_b(&e, "ww/slot-1", Duration::from_secs(60));
    e.clk.add(LEASE_OFFLINE_AFTER + Duration::from_secs(5));
    e.c.sweep_notices().unwrap();
    let n = e.c.lease_notices();
    e.c.undo_did(USER, "", &n[0].id).unwrap();
    assert!(e.c.lease_notices().is_empty());
    assert_eq!(e.c.leases().len(), 1, "the lease is untouched");
    e.c.sweep_notices().unwrap();
    assert!(e.c.lease_notices().is_empty(), "stays back");
    assert_eq!(e.rows("assistant.did").len(), 2, "record and undo");
}

#[test]
fn a_device_not_away_isnt_told_about() {
    let e = away_env(false);
    lease_on_b(&e, "ww/slot-1", Duration::from_secs(60));
    e.clk.add(LEASE_OFFLINE_AFTER * 2);
    e.c.sweep_notices().unwrap();
    assert!(e.rows("assistant.did").is_empty());
}

#[test]
fn switch_off_tells_nothing_about_leases() {
    let e = away_env(true);
    e.c.set_tidy(false);
    lease_on_b(&e, "ww/slot-1", Duration::from_secs(60));
    e.clk.add(LEASE_OFFLINE_AFTER + Duration::from_secs(5));
    e.c.sweep_notices().unwrap();
    assert!(e.rows("assistant.did").is_empty());
}

#[test]
fn a_shared_board_does_nothing() {
    let (a, _b, _owners) = common::two_people::two_people();
    a.c.set_tidy(true);
    let t =
        a.c.create_task(common::two_people::P1, nt("board"))
            .unwrap();
    assert!(!a.c.flag_merged(&t.id).unwrap());
    assert!(a.c.get_task(&t.id).unwrap().did.is_empty());
}

#[test]
fn undo_of_a_merged_flag_puts_an_in_review_card_back_in_review() {
    let e = tidy_env();
    let t = card(&e);
    linked_merged(&e, &t.id, PR_OPEN);
    e.c.set_task_state(BOB, &t.id, IN_REVIEW, "PR open")
        .unwrap();
    e.c.leave(BOB, &t.id, BOB).unwrap();
    let g = got(&e, &t.id);
    assert_eq!(g.state, IN_REVIEW);
    assert!(g.holders.is_empty());
    e.c.record_pr_state(
        &t.id,
        PrCheck {
            pr: PR.into(),
            state: PR_MERGED.into(),
            ..PrCheck::default()
        },
    )
    .unwrap();
    assert!(e.c.flag_merged(&t.id).unwrap());
    assert_eq!(got(&e, &t.id).state, BLOCKED);
    let d = only(&got(&e, &t.id), JOB_MERGED);
    e.c.undo_did(USER, &t.id, &d.id).unwrap();
    let g = got(&e, &t.id);
    assert_eq!(g.state, IN_REVIEW, "as it was before the flag");
    assert!(g.blockers.iter().all(|b| !b.open()), "{:?}", g.blockers);
    assert!(!e.c.needs_agent(&t.id));
    assert!(!e.c.flag_merged(&t.id).unwrap(), "the record stops it");
}

#[test]
fn undo_of_a_merged_flag_after_the_sweep_never_offers_the_card() {
    let e = tidy_env();
    let t = holderless_merged(&e);
    assert!(e.c.flag_merged(&t.id).unwrap());
    let d = only(&got(&e, &t.id), JOB_MERGED);
    e.c.undo_did(USER, &t.id, &d.id).unwrap();
    assert!(!e.c.needs_agent(&t.id));
    e.c.sweep_notices().unwrap();
    assert!(!e.c.needs_agent(&t.id), "a sweep doesn't offer it either");
    assert_eq!(e.rows("assistant.did").len(), 2);
}

fn record_merged_at(e: &Env, id: &str, pr: &str, when: gocompat::time::GoTime) {
    e.c.add_link(BOB, id, pr).unwrap();
    e.c.record_pr_state(
        id,
        PrCheck {
            pr: pr.into(),
            state: PR_MERGED.into(),
            merged_at: when,
            ..PrCheck::default()
        },
    )
    .unwrap();
}

#[test]
fn a_pr_merged_before_the_card_was_made_is_context_not_its_work() {
    let e = tidy_env();
    let before = e.clk.now();
    e.clk.add(Duration::from_secs(3600));
    let t = e.c.create_task(ALICE, nt("uses that PR")).unwrap();
    record_merged_at(&e, &t.id, PR, before);
    assert!(!e.c.flag_merged(&t.id).unwrap(), "only context is merged");
    assert!(got(&e, &t.id).did.is_empty());

    // Its own PR merging next to the context one does ask.
    e.clk.add(Duration::from_secs(60));
    record_merged_at(&e, &t.id, "https://github.com/a/b/pull/8", e.clk.now());
    assert!(e.c.flag_merged(&t.id).unwrap());
}

/// Device b's away state, changeable while the test runs.
fn away_env_live() -> (Env, Arc<std::sync::Mutex<AgentDevices>>) {
    let mut e = Env::new();
    let d = Arc::new(std::sync::Mutex::new(AgentDevices {
        away: HashSet::from(["dev-b".to_string()]),
        away_since: BTreeMap::from([("dev-b".to_string(), e.clk.now().unix_nano())]),
        ..AgentDevices::default()
    }));
    let shared = d.clone();
    e.c = e.reopen_with(Options {
        agent_devices: Some(Arc::new(move || shared.lock().unwrap().clone())),
        ..e.opts()
    });
    e.c.set_tidy(true);
    (e, d)
}

fn renew_on_b(e: &Env, seq: i64, acquired_at: gocompat::time::GoTime, ttl: Duration) {
    e.remote(
        "dev-b",
        seq,
        hlc_at(e.clk.now()),
        "dev-b/agent",
        kinds::LEASE_ACQUIRED,
        &LeaseAcquiredRow {
            lease: Lease {
                resource: "ww/slot-1".into(),
                holder: "dev-b/agent".into(),
                acquired_at,
                until: plus(e.clk.now(), ttl),
                ..Lease::default()
            },
            ..LeaseAcquiredRow::default()
        },
    );
}

fn told(e: &Env) {
    e.clk.add(LEASE_OFFLINE_AFTER + Duration::from_secs(120));
    e.c.sweep_notices().unwrap();
    assert_eq!(e.c.lease_notices().len(), 1);
}

#[test]
fn a_renewed_lease_loses_its_notice_and_regains_it_if_it_strands_again() {
    let (e, _d) = away_env_live();
    let got_at = e.clk.now();
    lease_on_b(&e, "ww/slot-1", Duration::from_secs(60));
    told(&e);
    renew_on_b(&e, 2, got_at, Duration::from_secs(600));
    assert!(e.c.lease_notices().is_empty(), "its time isn't up now");
    e.clk.add(Duration::from_secs(601));
    assert_eq!(e.c.lease_notices().len(), 1, "stranded again");
}

#[test]
fn a_lease_taken_again_by_the_same_holder_is_not_the_one_told_about() {
    let (e, _d) = away_env_live();
    lease_on_b(&e, "ww/slot-1", Duration::from_secs(60));
    told(&e);
    // A new acquisition, itself already past its time and still away.
    renew_on_b(&e, 2, e.clk.now(), Duration::from_secs(1));
    e.clk.add(Duration::from_secs(5));
    assert!(e.c.lease_notices().is_empty(), "a different lease");
}

#[test]
fn a_device_that_comes_back_loses_the_notice_on_its_lease() {
    let (e, d) = away_env_live();
    lease_on_b(&e, "ww/slot-1", Duration::from_secs(60));
    told(&e);
    d.lock().unwrap().away.clear();
    assert!(e.c.lease_notices().is_empty(), "it's online");
    assert_eq!(e.c.leases().len(), 1, "the lease is untouched");
}
