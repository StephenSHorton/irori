//! Merge desk slice 6 on a real Core (card #420): after each poll, Irori
//! posts on a PR's author card that it is on the desk, each push, a
//! conflict with its base and failed GitHub checks (with the log), once
//! each. Only a conflict or failed checks wake the author. A PR that isn't
//! ours gets nothing.

use crate::common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use coord::{DeskPr, NewTask, new_desk};
use irori_core::deskpost::FailedCheck;
use irori_core::deskwatch::PrLister;

const REPO: &str = "remote:github.com/StephenSHorton/ww-multiplayer";

#[derive(Default)]
struct FakeGh {
    prs: Mutex<Vec<DeskPr>>,
    failed: Mutex<Vec<FailedCheck>>,
    asked: Mutex<Vec<i64>>,
}

impl PrLister for FakeGh {
    fn list(&self, _repo: &str, _limit: usize) -> Result<Vec<DeskPr>, String> {
        Ok(self.prs.lock().unwrap().clone())
    }
    fn failed_checks(&self, repo: &str, number: i64) -> Result<Vec<FailedCheck>, String> {
        assert_eq!(repo, "StephenSHorton/ww-multiplayer");
        self.asked.lock().unwrap().push(number);
        Ok(self.failed.lock().unwrap().clone())
    }
}

fn url(n: i64) -> String {
    format!("https://github.com/StephenSHorton/ww-multiplayer/pull/{n}")
}

fn pr(n: i64, login: &str, head: &str) -> DeskPr {
    DeskPr {
        number: n,
        url: url(n),
        title: format!("PR {n} title"),
        author: login.into(),
        head_ref_name: format!("b/{n}"),
        head_ref_oid: head.into(),
        base_ref_name: "main".into(),
        mergeable: "MERGEABLE".into(),
        merge_state_status: "CLEAN".into(),
        checks: "success".into(),
        ..DeskPr::default()
    }
}

/// Irori's comments on card `id`, oldest first.
fn posts(co: &coord::Coord, id: &str) -> Vec<coord::Comment> {
    co.comments(id)
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.by == "irori")
        .collect()
}

/// What `key` was told about PR `n`'s card, and whether each woke it.
fn told(r: &Rig, key: &str, needle: &str) -> Vec<(String, bool)> {
    r.notices_to(key)
        .into_iter()
        .filter(|d| d.message.text.contains(needle))
        .map(|d| (d.message.text, d.mode == inbox::mode::WAKE))
        .collect()
}

fn set_pr(gh: &FakeGh, p: DeskPr) {
    let mut prs = gh.prs.lock().unwrap();
    prs.retain(|x| x.number != p.number);
    prs.push(p);
}

/// A desk trusting StephenSHorton, and a card held by agent a that links
/// PR 1.
fn rig() -> (Rig, Arc<FakeGh>, String) {
    let gh = Arc::new(FakeGh::default());
    let g = gh.clone();
    let r = Rig::with(move |o| o.desk_lister = Some(g));
    let co = r.core.coord();
    let me = format!("user:{}", r.core.device_id());
    let mut d = new_desk(REPO, r.core.device_id());
    d.trusted = vec!["StephenSHorton".into()];
    co.set_desk(&me, d).unwrap();
    let t = co
        .create_task(
            &r.key_a,
            NewTask {
                title: "co-op sync".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &t.id).unwrap();
    co.add_link(&r.key_a, &t.id, &url(1)).unwrap();
    (r, gh, t.id)
}

#[test]
fn a_pr_is_posted_on_its_card_once_then_each_push() {
    let (r, gh, id) = rig();
    let co = r.core.coord();
    set_pr(&gh, pr(1, "StephenSHorton", "aaaaaaa1111"));
    // A stranger's PR on the same desk: not ours, so no card, no post.
    set_pr(&gh, pr(5, "a-stranger", "eeeeeee5555"));
    r.core.poll_desk(REPO).unwrap();
    let p = posts(&co, &id);
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(
        p[0].text
            .starts_with("PR #1 is on the merge desk at `aaaaaaa`: https://github.com/StephenSHorton/ww-multiplayer/pull/1"),
        "{}",
        p[0].text
    );
    assert!(!p[0].wake);

    // The same list again, and again: nothing new.
    r.core.poll_desk(REPO).unwrap();
    r.core.poll_desk(REPO).unwrap();
    assert_eq!(posts(&co, &id).len(), 1);

    // A push: "Pushed <sha>", waking nobody.
    set_pr(&gh, pr(1, "StephenSHorton", "bbbbbbb2222"));
    r.core.poll_desk(REPO).unwrap();
    let p = posts(&co, &id);
    assert_eq!(p.len(), 2, "{p:?}");
    assert_eq!(p[1].text, "Pushed `bbbbbbb` to PR #1.");
    assert!(!p[1].wake);
    r.core.poll_desk(REPO).unwrap();
    assert_eq!(posts(&co, &id).len(), 2);
    std::thread::sleep(Duration::from_millis(300));
    assert!(told(&r, &r.key_a, "PR #1").is_empty(), "opened and pushed tell nobody");

    // Nothing about the stranger's PR anywhere.
    for t in co.list_tasks(&coord::TaskFilter::default()) {
        assert!(
            posts(&co, &t.id).iter().all(|c| !c.text.contains("PR #5")),
            "{t:?}"
        );
    }
}

#[test]
fn a_conflict_wakes_the_author_once_per_head() {
    let (r, gh, id) = rig();
    let co = r.core.coord();
    let mut p = pr(1, "StephenSHorton", "aaaaaaa1111");
    set_pr(&gh, p.clone());
    r.core.poll_desk(REPO).unwrap();

    // GitHub still working it out: no conflict yet.
    p.mergeable = "UNKNOWN".into();
    set_pr(&gh, p.clone());
    r.core.poll_desk(REPO).unwrap();
    assert_eq!(posts(&co, &id).len(), 1);

    p.mergeable = "CONFLICTING".into();
    p.merge_state_status = "DIRTY".into();
    set_pr(&gh, p.clone());
    r.core.poll_desk(REPO).unwrap();
    let c = posts(&co, &id);
    assert_eq!(c.len(), 2, "{c:?}");
    assert!(
        c[1].text.starts_with("PR #1 conflicts with `main` at `aaaaaaa`"),
        "{}",
        c[1].text
    );
    assert!(c[1].wake);
    wait_for(30, "the author's wake", || {
        told(&r, &r.key_a, "conflicts with").iter().any(|(_, w)| *w)
    });
    // Still conflicting at the same head: no second post or wake.
    r.core.poll_desk(REPO).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(posts(&co, &id).len(), 2);
    assert_eq!(told(&r, &r.key_a, "conflicts with").len(), 1);

    // Behind main alone is no post.
    p.head_ref_oid = "ccccccc3333".into();
    p.mergeable = "MERGEABLE".into();
    p.merge_state_status = "BEHIND".into();
    set_pr(&gh, p.clone());
    r.core.poll_desk(REPO).unwrap();
    let c = posts(&co, &id);
    assert_eq!(c.len(), 3, "{c:?}");
    assert_eq!(c[2].text, "Pushed `ccccccc` to PR #1.");
}

#[test]
fn failed_checks_post_the_log_and_wake_the_author() {
    let (r, gh, id) = rig();
    let co = r.core.coord();
    let mut p = pr(1, "StephenSHorton", "aaaaaaa1111");
    set_pr(&gh, p.clone());
    r.core.poll_desk(REPO).unwrap();

    let mut log = String::new();
    for i in 0..40 {
        log.push_str(&format!(
            "test\tRun tests\t2026-11-06T17:00:00.0000000Z line {i}\n"
        ));
    }
    log.push_str("test\tRun tests\t2026-11-06T17:00:01.0000000Z error[E0308]: mismatched types\n");
    *gh.failed.lock().unwrap() = vec![FailedCheck {
        name: "CI / test".into(),
        url: "https://github.com/StephenSHorton/ww-multiplayer/actions/runs/11/job/22".into(),
        log,
    }];
    p.checks = "pending".into();
    set_pr(&gh, p.clone());
    r.core.poll_desk(REPO).unwrap();
    assert!(gh.asked.lock().unwrap().is_empty(), "no fetch while pending");

    p.checks = "failure".into();
    set_pr(&gh, p.clone());
    r.core.poll_desk(REPO).unwrap();
    let c = posts(&co, &id);
    assert_eq!(c.len(), 2, "{c:?}");
    let f = &c[1];
    assert!(
        f.text.starts_with("GitHub checks failed on PR #1 at `aaaaaaa`:"),
        "{}",
        f.text
    );
    assert!(
        f.text.contains("- CI / test: https://github.com/StephenSHorton/ww-multiplayer/actions/runs/11/job/22"),
        "{}",
        f.text
    );
    assert!(f.text.contains("error[E0308]: mismatched types"), "{}", f.text);
    assert!(f.wake);
    assert_eq!(f.attachments.len(), 1, "{:?}", f.attachments);
    assert!(f.attachments[0].name.ends_with(".log"), "{:?}", f.attachments);
    assert!(f.attachments[0].size > 0);
    wait_for(30, "the author's wake", || {
        told(&r, &r.key_a, "GitHub checks failed").iter().any(|(_, w)| *w)
    });

    // Asked once: the same head failing again posts and fetches nothing.
    r.core.poll_desk(REPO).unwrap();
    assert_eq!(*gh.asked.lock().unwrap(), vec![1]);
    assert_eq!(posts(&co, &id).len(), 2);

    // The fix: a push whose checks pass is only "Pushed".
    p.head_ref_oid = "ddddddd4444".into();
    p.checks = "success".into();
    set_pr(&gh, p);
    r.core.poll_desk(REPO).unwrap();
    let c = posts(&co, &id);
    assert_eq!(c.len(), 3);
    assert_eq!(c[2].text, "Pushed `ddddddd` to PR #1.");
}

/// A PR with no card gets Irori's "PR #n" card on one poll and its post
/// on the next. Nobody holds that card, so a failure there wakes nobody.
#[test]
fn an_unheld_pr_card_gets_posts_and_wakes_nobody() {
    let (r, gh, _) = rig();
    let co = r.core.coord();
    let mut p = pr(2, "StephenSHorton", "fffffff6666");
    p.checks = "failure".into();
    set_pr(&gh, p);
    r.core.poll_desk(REPO).unwrap();
    let card = co
        .list_tasks(&coord::TaskFilter::default())
        .into_iter()
        .find(|t| t.links.iter().any(|l| l.reference == url(2)))
        .expect("a PR #2 card");
    r.core.poll_desk(REPO).unwrap();
    let c = posts(&co, &card.id);
    assert_eq!(c.len(), 2, "{c:?}");
    assert!(c[0].text.starts_with("PR #2 is on the merge desk"));
    assert!(c[1].text.starts_with("GitHub checks failed on PR #2"));
    std::thread::sleep(Duration::from_millis(300));
    for k in [&r.key_a, &r.key_b] {
        assert!(told(&r, k, "PR #2").is_empty(), "{k}");
    }
}

/// An author card on a shared board gets the posts on that board.
#[test]
fn an_author_card_on_a_shared_board_gets_the_posts_there() {
    let (r, gh, _) = rig();
    let c = &r.core;
    let b = c.create_board("wind", "wind").unwrap();
    let co = c.board_coord(&b).unwrap();
    let card_b = c.board_key(&b, &r.key_b).unwrap();
    let t = co
        .create_task(
            &card_b,
            NewTask {
                title: "co-op sync".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.claim_task(&card_b, &t.id).unwrap();
    co.add_link(&card_b, &t.id, &url(7)).unwrap();
    let mut p = pr(7, "agent-bot", "1234567abcd");
    p.mergeable = "CONFLICTING".into();
    set_pr(&gh, p);
    c.poll_desk(REPO).unwrap();
    let ps = posts(&co, &t.id);
    assert_eq!(ps.len(), 2, "{ps:?}");
    assert!(ps[0].text.starts_with("PR #7 is on the merge desk"));
    assert!(ps[1].text.starts_with("PR #7 conflicts with `main`"));
    wait_for(30, "the author's wake on the board", || {
        told(&r, &r.key_b, "conflicts with").iter().any(|(_, w)| *w)
    });
    c.poll_desk(REPO).unwrap();
    assert_eq!(posts(&co, &t.id).len(), 2);
}
