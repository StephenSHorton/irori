//! Card #521 (docs/design/merge-desk.md §5, §11 slice 3): a GitHub repo an
//! agent works in has a merge desk with no `repo.desk` row. Its device
//! (the one that saw the repo last) polls GitHub, the person's own GitHub
//! login counts as trusted, turning the desk off stops the polling, and a
//! repo with no GitHub remote gets no desk.

use crate::common;

use std::sync::{Arc, Mutex};

use common::*;
use coord::DeskPr;
use irori_core::deskwatch::PrLister;
use serde_json::json;
use store::kinds;

const REPO: &str = "remote:github.com/stephenshorton/ww-multiplayer";

/// A gh signed in as "StephenSHorton", answering with the test's list.
struct FakeGh {
    answer: Mutex<Vec<DeskPr>>,
    calls: Mutex<usize>,
}

impl PrLister for FakeGh {
    fn list(&self, _repo: &str, _limit: usize) -> Result<Vec<DeskPr>, String> {
        *self.calls.lock().unwrap() += 1;
        Ok(self.answer.lock().unwrap().clone())
    }

    fn viewer(&self) -> Result<String, String> {
        Ok("StephenSHorton".into())
    }
}

fn rig() -> (Rig, Arc<FakeGh>) {
    let gh = Arc::new(FakeGh {
        answer: Mutex::new(Vec::new()),
        calls: Mutex::new(0),
    });
    let g = gh.clone();
    (Rig::with(move |o| o.desk_lister = Some(g)), gh)
}

fn pr(n: i64, author: &str) -> DeskPr {
    DeskPr {
        number: n,
        url: format!("https://github.com/StephenSHorton/ww-multiplayer/pull/{n}"),
        title: format!("PR {n}"),
        author: author.into(),
        head_ref_name: format!("branch-{n}"),
        head_ref_oid: format!("{n}abc"),
        ..DeskPr::default()
    }
}

/// What capture writes when an agent here first works in `repo`.
fn agent_works_in(r: &Rig, repo: &str) {
    let row = json!({"repoKey": repo, "root": "C:/src/ww", "at": "2026-11-05T10:00:00Z"});
    r.core
        .store()
        .append("irori", kinds::REPO_SEEN, &row)
        .unwrap();
}

fn desk_rows(r: &Rig) -> usize {
    r.core
        .store()
        .list(&store::Filter {
            kinds: vec![kinds::REPO_DESK.into()],
            limit: 1000,
            ..store::Filter::default()
        })
        .unwrap()
        .len()
}

#[test]
fn a_github_repo_an_agent_works_in_shows_its_open_prs_within_one_poll() {
    let (r, gh) = rig();
    *gh.answer.lock().unwrap() = vec![pr(7, "StephenSHorton"), pr(8, "a-stranger")];
    agent_works_in(&r, REPO);
    wait_for(10, "the default desk", || {
        r.core.coord().desk(REPO).is_some()
    });
    assert_eq!(desk_rows(&r), 0, "no repo.desk row");
    let d = r.core.coord().desk(REPO).unwrap();
    assert!(d.on() && d.is_default());
    assert_eq!(d.settings.poll_device, r.core.device_id());

    r.core.check_desks();
    let s = r.core.coord().desk(REPO).unwrap().prs.expect("one poll");
    assert_eq!(s.prs.iter().map(|p| p.number).collect::<Vec<_>>(), [7, 8]);
    assert_eq!(s.viewer, "StephenSHorton");

    // The person's own login is trusted; a stranger's PR is Outside.
    let v = r.core.desk_verdicts(REPO);
    let outside = |n: i64| v.iter().find(|a| a.pr == n).unwrap().outside;
    assert!(!outside(7), "the person's own PR is trusted");
    assert!(outside(8), "a stranger's PR waits for the person's OK");
}

#[test]
fn turning_the_desk_off_hides_it_and_stops_polling() {
    let (r, gh) = rig();
    agent_works_in(&r, REPO);
    wait_for(10, "the default desk", || {
        r.core.coord().desk(REPO).is_some()
    });
    r.core.check_desks();
    assert_eq!(*gh.calls.lock().unwrap(), 1);

    let mut s = r.core.coord().desk(REPO).unwrap().settings;
    s.enabled = false;
    s.poll_device = String::new();
    let me = format!("user:{}", r.core.device_id());
    r.core.coord().set_desk(&me, s).unwrap();
    assert!(!r.core.coord().desk(REPO).unwrap().on());

    // Due or not, an off desk is never asked about again.
    assert!(r.core.poll_desk(REPO).is_err());
    r.core.check_desks();
    assert_eq!(*gh.calls.lock().unwrap(), 1);
}

#[test]
fn a_repo_with_no_github_remote_gets_no_desk() {
    let (r, gh) = rig();
    agent_works_in(&r, "root:4b825dc642cb6eb9a060e54bf8d69288fbee4904");
    agent_works_in(&r, "remote:gitlab.com/someone/thing");
    // A GitHub repo after them, so the fold has seen all three.
    agent_works_in(&r, REPO);
    wait_for(10, "the GitHub repo's desk", || {
        r.core.coord().desk(REPO).is_some()
    });
    let keys: Vec<String> = r
        .core
        .coord()
        .desks()
        .into_iter()
        .map(|d| d.settings.repo_key)
        .collect();
    assert_eq!(keys, [REPO]);
    r.core.check_desks();
    assert_eq!(
        *gh.calls.lock().unwrap(),
        1,
        "only the GitHub repo is polled"
    );
}
