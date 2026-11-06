//! Card #415: the merge desk's poll device asks GitHub (here a fake gh)
//! for a repo's open PRs and keeps a snapshot, written only on change.

use crate::common;

use std::sync::{Arc, Mutex};

use common::*;
use coord::{DESK_LIMIT, DeskPr, new_desk};
use irori_core::deskwatch::{PrLister, backoff, checks_of, parse_pr_list};
use store::kinds;

const REPO: &str = "remote:github.com/StephenSHorton/ww-multiplayer";

/// A gh that answers with whatever the test set, counting calls.
struct FakeGh {
    answer: Mutex<Result<Vec<DeskPr>, String>>,
    calls: Mutex<Vec<(String, usize)>>,
}

impl FakeGh {
    fn set(&self, a: Result<Vec<DeskPr>, String>) {
        *self.answer.lock().unwrap() = a;
    }
}

impl PrLister for FakeGh {
    fn list(&self, repo: &str, limit: usize) -> Result<Vec<DeskPr>, String> {
        self.calls.lock().unwrap().push((repo.to_string(), limit));
        let a = self.answer.lock().unwrap().clone();
        // gh returns at most `limit`, as the real one does.
        a.map(|mut v| {
            v.truncate(limit);
            v
        })
    }
}

fn pr(n: i64) -> DeskPr {
    DeskPr {
        number: n,
        url: format!("https://github.com/StephenSHorton/ww-multiplayer/pull/{n}"),
        title: format!("PR {n}"),
        author: "StephenSHorton".into(),
        head_ref_name: format!("branch-{n}"),
        ..DeskPr::default()
    }
}

fn rig() -> (Rig, Arc<FakeGh>) {
    let gh = Arc::new(FakeGh {
        answer: Mutex::new(Ok(Vec::new())),
        calls: Mutex::new(Vec::new()),
    });
    let g = gh.clone();
    let r = Rig::with(move |o| o.desk_lister = Some(g));
    (r, gh)
}

fn person(r: &Rig) -> String {
    format!("user:{}", r.core.device_id())
}

/// The desk.prs rows in the private log.
fn snapshots(r: &Rig) -> usize {
    r.core
        .store()
        .list(&store::Filter {
            kinds: vec![kinds::DESK_PRS.into()],
            limit: 1000,
            ..store::Filter::default()
        })
        .unwrap()
        .len()
}

fn desk_here(r: &Rig) {
    let d = new_desk(REPO, r.core.device_id());
    r.core.coord().set_desk(&person(r), d).unwrap();
}

#[test]
fn an_empty_list_is_a_snapshot_and_reads_empty() {
    let (r, gh) = rig();
    desk_here(&r);
    r.core.check_desks();
    assert_eq!(
        *gh.calls.lock().unwrap(),
        vec![("stephenshorton/ww-multiplayer".to_string(), DESK_LIMIT)]
    );
    let d = r.core.coord().desk(REPO).unwrap();
    let s = d.prs.expect("a snapshot, even of nothing");
    assert!(s.prs.is_empty() && !s.truncated);
    assert!(!r.core.desk_poll(REPO).last_poll_at.is_zero());
    // Due again only after the cadence.
    r.core.check_desks();
    assert_eq!(gh.calls.lock().unwrap().len(), 1);
}

#[test]
fn one_pr_then_an_unchanged_list_writes_no_new_row() {
    let (r, gh) = rig();
    desk_here(&r);
    gh.set(Ok(vec![pr(305)]));
    assert_eq!(r.core.poll_desk(REPO), Ok(true));
    let s = r.core.coord().desk(REPO).unwrap().prs.unwrap();
    assert_eq!(s.prs, vec![pr(305)]);
    assert_eq!(snapshots(&r), 1);
    // The same list again: nothing written.
    assert_eq!(r.core.poll_desk(REPO), Ok(false));
    assert_eq!(snapshots(&r), 1);
    // A field changed (a push moved the head): a new snapshot.
    let mut moved = pr(305);
    moved.head_ref_oid = "abc123".into();
    gh.set(Ok(vec![moved.clone()]));
    assert_eq!(r.core.poll_desk(REPO), Ok(true));
    assert_eq!(snapshots(&r), 2);
    assert_eq!(
        r.core.coord().desk(REPO).unwrap().prs.unwrap().prs,
        vec![moved]
    );
}

#[test]
fn a_list_of_the_limit_is_truncated() {
    let (r, gh) = rig();
    desk_here(&r);
    gh.set(Ok((1..=DESK_LIMIT as i64 + 5).map(pr).collect()));
    assert_eq!(r.core.poll_desk(REPO), Ok(true));
    let s = r.core.coord().desk(REPO).unwrap().prs.unwrap();
    assert_eq!(s.prs.len(), DESK_LIMIT);
    assert!(s.truncated, "GitHub may have more");
    // Under the limit again: no longer truncated.
    gh.set(Ok(vec![pr(1)]));
    r.core.poll_desk(REPO).unwrap();
    assert!(!r.core.coord().desk(REPO).unwrap().prs.unwrap().truncated);
}

#[test]
fn unreachable_keeps_the_previous_snapshot_and_backs_off() {
    let (r, gh) = rig();
    desk_here(&r);
    gh.set(Ok(vec![pr(316), pr(317)]));
    r.core.poll_desk(REPO).unwrap();
    gh.set(Err("gh couldn't list this repo's PRs: HTTP 502".into()));
    assert!(r.core.poll_desk(REPO).is_err());
    assert!(r.core.poll_desk(REPO).is_err());
    // The queue still shows what GitHub last said; nothing was written.
    let s = r.core.coord().desk(REPO).unwrap().prs.unwrap();
    assert_eq!(s.prs.len(), 2);
    assert_eq!(snapshots(&r), 1);
    let p = r.core.desk_poll(REPO);
    assert!(!p.unreachable_since.is_zero());
    assert_eq!(p.failures, 2);
    assert!(p.error.contains("502"), "{}", p.error);
    // Back: the unreachable line goes.
    gh.set(Ok(vec![pr(316), pr(317)]));
    assert_eq!(r.core.poll_desk(REPO), Ok(false));
    let p = r.core.desk_poll(REPO);
    assert!(p.unreachable_since.is_zero() && p.failures == 0 && p.error.is_empty());
}

#[test]
fn a_device_that_isnt_the_poll_device_writes_nothing() {
    let (r, gh) = rig();
    let d = new_desk(REPO, "some-other-device");
    r.core.coord().set_desk(&person(&r), d).unwrap();
    gh.set(Ok(vec![pr(318)]));
    r.core.check_desks();
    assert!(gh.calls.lock().unwrap().is_empty());
    assert!(r.core.poll_desk(REPO).is_err());
    assert!(r.core.coord().desk(REPO).unwrap().prs.is_none());
    assert_eq!(snapshots(&r), 0);
}

#[test]
fn only_the_person_sets_up_a_desk() {
    let (r, _) = rig();
    let d = new_desk(REPO, r.core.device_id());
    assert!(r.core.coord().set_desk(&r.key_a, d.clone()).is_err());
    let not_github = new_desk("remote:gitlab.com/a/b", r.core.device_id());
    assert!(r.core.coord().set_desk(&person(&r), not_github).is_err());
    r.core.coord().set_desk(&person(&r), d).unwrap();
    assert!(r.core.coord().desk(REPO).unwrap().on());
}

#[test]
fn backoff_doubles_to_fifteen_minutes() {
    let m = |n| backoff(n).as_secs();
    assert_eq!(
        [m(0), m(1), m(2), m(3), m(4), m(9)],
        [180, 360, 720, 900, 900, 900]
    );
}

#[test]
fn ghs_answer_reads_into_the_snapshot() {
    let v = serde_json::json!([{
        "number": 12, "url": "https://github.com/o/n/pull/12", "title": "Fix",
        "author": {"login": "kite", "is_bot": false}, "isDraft": true,
        "headRefName": "fix", "headRefOid": "deadbeef", "baseRefName": "main",
        "mergeable": "CONFLICTING", "mergeStateStatus": "DIRTY",
        "statusCheckRollup": [{"__typename": "CheckRun", "status": "IN_PROGRESS", "conclusion": ""}],
        "createdAt": "2026-11-03T10:00:00Z", "updatedAt": "2026-11-03T11:00:00Z"
    }, {"title": "no number: skipped"}]);
    let prs = parse_pr_list(&v).unwrap();
    assert_eq!(prs.len(), 1);
    let p = &prs[0];
    assert_eq!(
        (p.number, p.author.as_str(), p.is_draft),
        (12, "kite", true)
    );
    assert_eq!(
        (p.mergeable.as_str(), p.merge_state_status.as_str()),
        ("CONFLICTING", "DIRTY")
    );
    assert_eq!(p.checks, "pending");
    assert!(!p.created_at.is_zero());
    assert!(parse_pr_list(&serde_json::json!({"x": 1})).is_err());
}

#[test]
fn checks_sum_up_the_rollup() {
    let c = |v: serde_json::Value| checks_of(&v);
    // No GitHub checks (ww has no CI): "", never read as passing.
    assert_eq!(c(serde_json::json!([])), "");
    assert_eq!(c(serde_json::Value::Null), "");
    assert_eq!(
        c(serde_json::json!([{"status": "COMPLETED", "conclusion": "SUCCESS"}])),
        "success"
    );
    assert_eq!(c(serde_json::json!([{"state": "PENDING"}])), "pending");
    assert_eq!(
        c(
            serde_json::json!([{"status": "IN_PROGRESS"}, {"status": "COMPLETED", "conclusion": "FAILURE"}])
        ),
        "failure"
    );
    assert_eq!(c(serde_json::json!([{"state": "ERROR"}])), "failure");
}

/// Live (run by hand on each machine with `--ignored`): the person's own
/// gh, through the desk's lister, sees the same open PRs `gh pr list`
/// shows for ww. Set IRORI_DESK_LIVE_REPO to try another repo.
#[test]
#[ignore]
fn live_ww_matches_gh_pr_list() {
    let repo = std::env::var("IRORI_DESK_LIVE_REPO")
        .unwrap_or_else(|_| "StephenSHorton/ww-multiplayer".into());
    let ours = irori_core::deskwatch::GhLister
        .list(&repo, DESK_LIMIT)
        .unwrap();
    let out = std::process::Command::new("gh")
        .args([
            "pr",
            "list",
            "--repo",
            &repo,
            "--state",
            "open",
            "--limit",
            "1000",
            "--json",
            "number,headRefOid",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let theirs: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
    let mut a: Vec<(i64, String)> = ours
        .iter()
        .map(|p| (p.number, p.head_ref_oid.clone()))
        .collect();
    let mut b: Vec<(i64, String)> = theirs
        .iter()
        .map(|p| {
            (
                p["number"].as_i64().unwrap(),
                p["headRefOid"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    a.sort();
    b.sort();
    println!("{repo}: {} open PRs: {a:?}", a.len());
    assert_eq!(a, b);
}
