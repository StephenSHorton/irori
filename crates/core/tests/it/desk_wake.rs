//! Merge desk slice 4 on a real Core (card #418): after a poll, the desk
//! holder is woken with the run's steps for the oldest trusted PR Waiting
//! on desk, once per PR and head. A PR that isn't ours never wakes it, nor does a
//! paused desk or a run already on.

use crate::common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use coord::{DeskPr, new_desk};
use irori_core::deskwatch::PrLister;

const REPO: &str = "remote:github.com/StephenSHorton/ww-multiplayer";
const LEASE: &str = "desk/stephenshorton/ww-multiplayer";

struct FakeGh(Mutex<Vec<DeskPr>>);

impl PrLister for FakeGh {
    fn list(&self, _repo: &str, _limit: usize) -> Result<Vec<DeskPr>, String> {
        Ok(self.0.lock().unwrap().clone())
    }

    /// The repo's owner: it is the person's repo (card #625).
    fn viewer(&self) -> Result<String, String> {
        Ok("StephenSHorton".into())
    }
}

fn pr(n: i64, login: &str, age_min: i64) -> DeskPr {
    let t = gocompat::time::GoTime::unix_milli(1_800_000_000_000 - age_min * 60_000);
    DeskPr {
        number: n,
        url: format!("https://github.com/StephenSHorton/ww-multiplayer/pull/{n}"),
        title: format!("PR {n} title"),
        author: login.into(),
        head_ref_name: format!("b/{n}"),
        head_ref_oid: format!("sha{n}aaaaaaaaaaaa"),
        mergeable: "MERGEABLE".into(),
        merge_state_status: "CLEAN".into(),
        checks: "success".into(),
        created_at: t,
        updated_at: t,
        ..DeskPr::default()
    }
}

/// The desk run wakes delivered to `key`.
fn wakes(r: &Rig, key: &str) -> Vec<String> {
    r.notices_to(key)
        .into_iter()
        .map(|d| d.message.text)
        .filter(|t| t.contains("is Waiting on desk"))
        .collect()
}

#[test]
fn the_holder_is_woken_once_for_the_oldest_trusted_waiting_pr() {
    let gh = Arc::new(FakeGh(Mutex::new(Vec::new())));
    let g = gh.clone();
    let r = Rig::with(move |o| o.desk_lister = Some(g));
    let co = r.core.coord();
    let me = format!("user:{}", r.core.device_id());
    let mut d = new_desk(REPO, r.core.device_id());
    d.trusted = vec!["StephenSHorton".into()];
    d.holder = r.key_b.clone();
    d.checks = vec![coord::DeskCheck {
        command: "cargo test".into(),
        ..coord::DeskCheck::default()
    }];
    co.set_desk(&me, d).unwrap();

    let mut draft = pr(1, "StephenSHorton", 90);
    draft.is_draft = true;
    let mut failing = pr(2, "StephenSHorton", 80);
    failing.checks = "failure".into();
    *gh.0.lock().unwrap() = vec![
        // The oldest of all: a stranger's PR, not ours, never the one.
        pr(9, "a-stranger", 120),
        draft,
        failing,
        pr(4, "StephenSHorton", 30),
        pr(3, "StephenSHorton", 60),
    ];
    r.core.poll_desk(REPO).unwrap();
    wait_for(30, "the holder's wake", || !wakes(&r, &r.key_b).is_empty());
    let w = wakes(&r, &r.key_b);
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
        w[0].contains("StephenSHorton/ww-multiplayer#3 is Waiting on desk"),
        "{}",
        w[0]
    );
    assert!(w[0].contains(LEASE), "{}", w[0]);
    assert!(w[0].contains("`cargo test`"), "{}", w[0]);
    assert!(w[0].contains(coord::DESK_NEVER), "{}", w[0]);
    // Nobody else hears it.
    assert!(wakes(&r, &r.key_a).is_empty());

    // The same PR at the same head: no second wake.
    r.core.poll_desk(REPO).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(wakes(&r, &r.key_b).len(), 1);

    // While a run is on, nobody is woken: the holder takes #3.
    co.acquire(&r.key_b, LEASE, Duration::from_secs(1800), false)
        .unwrap();
    co.record_desk_run(&r.key_b, REPO, 3, "sha3aaaaaaaaaaaa", "")
        .unwrap();
    gh.0.lock().unwrap().retain(|p| p.number != 3);
    r.core.poll_desk(REPO).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(wakes(&r, &r.key_b).len(), 1);

    // #3 merged and the lease released: the next one, #4, wakes it.
    co.release(&r.key_b, LEASE).unwrap();
    r.core.poll_desk(REPO).unwrap();
    wait_for(30, "the wake for #4", || wakes(&r, &r.key_b).len() == 2);
    assert!(
        wakes(&r, &r.key_b)
            .iter()
            .any(|w| w.contains("#4 is Waiting on desk"))
    );
}

#[test]
fn a_paused_desk_or_only_others_prs_wake_nobody() {
    let gh = Arc::new(FakeGh(Mutex::new(vec![pr(5, "a-stranger", 10)])));
    let g = gh.clone();
    let r = Rig::with(move |o| o.desk_lister = Some(g));
    let co = r.core.coord();
    let me = format!("user:{}", r.core.device_id());
    let mut d = new_desk(REPO, r.core.device_id());
    d.trusted = vec!["StephenSHorton".into()];
    d.holder = r.key_b.clone();
    co.set_desk(&me, d).unwrap();
    r.core.poll_desk(REPO).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(wakes(&r, &r.key_b).is_empty());

    co.pause_desk(&me, REPO, true).unwrap();
    gh.0.lock().unwrap().push(pr(6, "StephenSHorton", 5));
    r.core.poll_desk(REPO).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(wakes(&r, &r.key_b).is_empty());

    co.pause_desk(&me, REPO, false).unwrap();
    r.core.poll_desk(REPO).unwrap();
    wait_for(30, "the wake after resume", || {
        wakes(&r, &r.key_b).len() == 1
    });
    assert!(wakes(&r, &r.key_b)[0].contains("#6 is Waiting on desk"));
}
