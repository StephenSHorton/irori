//! Merge desk slice 7 (card #421): the desk's polling device hears a PR
//! merged when it leaves the open list, records it on the author card
//! wherever that card is held, says the merge sha there as Irori, and a
//! "PR #n" card Irori made completes itself.

use crate::common;

use std::sync::{Arc, Mutex};

use common::*;
use coord::{DeskPr, NewTask, PR_MERGED, PrCheck, new_desk};
use irori_core::deskwatch::PrLister;
use irori_core::prwatch::PrChecker;

const REPO: &str = "remote:github.com/StephenSHorton/desk-scratch";
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const OTHER: &str = "otherdevice/sess";

/// One fake gh for both the desk's list and `gh pr view`.
#[derive(Default)]
struct FakeGh {
    open: Mutex<Vec<DeskPr>>,
    /// What `gh pr view` says: state and mergeCommit.
    view: Mutex<(String, String)>,
    viewed: Mutex<Vec<String>>,
}

impl PrLister for FakeGh {
    fn list(&self, _repo: &str, _limit: usize) -> Result<Vec<DeskPr>, String> {
        Ok(self.open.lock().unwrap().clone())
    }
    fn viewer(&self) -> Result<String, String> {
        Ok("StephenSHorton".into())
    }
}

impl PrChecker for FakeGh {
    fn check(&self, url: &str) -> PrCheck {
        self.viewed.lock().unwrap().push(url.to_string());
        let (state, sha) = self.view.lock().unwrap().clone();
        PrCheck {
            pr: url.into(),
            state,
            merge_sha: sha,
            ..PrCheck::default()
        }
    }
}

fn url(n: i64) -> String {
    format!("https://github.com/StephenSHorton/desk-scratch/pull/{n}")
}

fn pr(n: i64) -> DeskPr {
    DeskPr {
        number: n,
        url: url(n),
        title: format!("Test PR {n}"),
        author: "StephenSHorton".into(),
        head_ref_name: format!("branch-{n}"),
        head_ref_oid: "feedbeef".into(),
        ..DeskPr::default()
    }
}

fn rig() -> (Rig, Arc<FakeGh>) {
    let gh = Arc::new(FakeGh::default());
    let (l, c) = (gh.clone(), gh.clone());
    let r = Rig::with(move |o| {
        o.desk_lister = Some(l);
        o.pr_checker = Some(c);
    });
    let d = new_desk(REPO, r.core.device_id());
    r.core
        .coord()
        .set_desk(&format!("user:{}", r.core.device_id()), d)
        .unwrap();
    (r, gh)
}

fn merged(gh: &FakeGh, sha: &str) {
    *gh.view.lock().unwrap() = (PR_MERGED.into(), sha.into());
}

/// Irori's comments on card `id` that carry `sha`.
fn sha_comments(r: &Rig, id: &str, sha: &str) -> usize {
    r.core
        .coord()
        .comments(id)
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.by == "irori" && c.text.contains(sha))
        .count()
}

#[test]
fn an_unheld_pr_card_hears_its_merge_says_the_sha_and_completes() {
    let (r, gh) = rig();
    let co = r.core.coord();
    *gh.open.lock().unwrap() = vec![pr(7)];
    r.core.poll_desk(REPO).unwrap();
    // The desk made a "PR #7" card for it; nobody holds it.
    let card = co
        .list_tasks(&coord::TaskFilter::default())
        .into_iter()
        .find(|t| t.links.iter().any(|l| l.reference == url(7)))
        .expect("a PR #7 card");
    assert!(coord::desk_pr_card(&card), "{card:?}");
    assert!(card.holders.is_empty());
    // Still open: nothing asked.
    assert!(gh.viewed.lock().unwrap().is_empty());

    // Merged on GitHub (by an agent on the person's yes, or by them).
    gh.open.lock().unwrap().clear();
    merged(&gh, SHA);
    r.core.poll_desk(REPO).unwrap();
    assert_eq!(*gh.viewed.lock().unwrap(), vec![url(7)]);
    let st = co.pr_state(&card.id, &url(7)).unwrap();
    assert_eq!((st.state.as_str(), st.merge_sha.as_str()), (PR_MERGED, SHA));
    assert_eq!(sha_comments(&r, &card.id, SHA), 1);
    assert_eq!(co.get_task(&card.id).unwrap().state, coord::COMPLETED);

    // The next poll asks nothing and says nothing again.
    r.core.poll_desk(REPO).unwrap();
    assert_eq!(gh.viewed.lock().unwrap().len(), 1);
    assert_eq!(sha_comments(&r, &card.id, SHA), 1);
}

#[test]
fn a_card_held_on_another_device_hears_its_merge_from_the_desk() {
    // A polls the desk; B's agent holds the author card. B asks GitHub
    // about nothing (no gh), as a device that is off or busy.
    let gh = Arc::new(FakeGh::default());
    let (l, c) = (gh.clone(), gh.clone());
    let (a, b) = common::pair::paired_with(move |o| {
        o.desk_lister = Some(l);
        o.pr_checker = Some(c);
    });
    a.core
        .coord()
        .set_desk(
            &format!("user:{}", a.core.device_id()),
            new_desk(REPO, a.core.device_id()),
        )
        .unwrap();
    let cb = b.core.coord();
    let t = cb
        .create_task(
            &b.agent,
            NewTask {
                title: "land the fix".into(),
                repo_key: REPO.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    cb.claim_task(&b.agent, &t.id).unwrap();
    cb.add_link(&b.agent, &t.id, &url(9)).unwrap();
    let ca = a.core.coord();
    wait_for(45, "A sees B's card with its PR", || {
        ca.get_task(&t.id)
            .is_some_and(|t| t.links.iter().any(|l| l.reference == url(9)))
    });
    *gh.open.lock().unwrap() = vec![pr(9)];
    a.core.poll_desk(REPO).unwrap();
    // A's own PR poller never asks: no agent on A holds it.
    a.core.check_prs();
    assert!(gh.viewed.lock().unwrap().is_empty());

    gh.open.lock().unwrap().clear();
    merged(&gh, SHA);
    a.core.poll_desk(REPO).unwrap();
    assert_eq!(*gh.viewed.lock().unwrap(), vec![url(9)]);
    // B's card hears it, with the sha said once as Irori.
    wait_for(45, "the merge and its sha reach B", || {
        cb.pr_state(&t.id, &url(9))
            .is_some_and(|st| st.state == PR_MERGED && st.merge_sha == SHA)
            && cb
                .comments(&t.id)
                .unwrap_or_default()
                .iter()
                .any(|c| c.by == "irori" && c.text.contains(SHA))
    });
    // Its holder on B is told to complete it; the card isn't completed for it.
    wait_for(45, "the holder on B told", || {
        b.core
            .inbox()
            .for_agent(&b.agent, true, 0)
            .iter()
            .any(|d| d.message.notice && d.message.text.contains("was merged"))
    });
    assert_eq!(cb.get_task(&t.id).unwrap().state, coord::WORKING);
    let said = cb
        .comments(&t.id)
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.by == "irori" && c.text.contains(SHA))
        .count();
    assert_eq!(said, 1);
}

#[test]
fn a_merge_heard_without_its_sha_gets_the_sha_once() {
    let (r, gh) = rig();
    let co = r.core.coord();
    let t = co
        .create_task(
            OTHER,
            NewTask {
                title: "land it".into(),
                repo_key: REPO.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.claim_task(OTHER, &t.id).unwrap();
    co.add_link(OTHER, &t.id, &url(11)).unwrap();
    // An older device heard the merge first, without the sha.
    co.record_pr_state(
        &t.id,
        PrCheck {
            pr: url(11),
            state: PR_MERGED.into(),
            ..PrCheck::default()
        },
    )
    .unwrap();
    assert_eq!(sha_comments(&r, &t.id, SHA), 0);

    merged(&gh, SHA);
    r.core.poll_desk(REPO).unwrap();
    assert_eq!(co.pr_state(&t.id, &url(11)).unwrap().merge_sha, SHA);
    assert_eq!(sha_comments(&r, &t.id, SHA), 1);
    // Heard again, by the holder's poller or another desk: said once.
    co.record_pr_state(
        &t.id,
        PrCheck {
            pr: url(11),
            state: PR_MERGED.into(),
            merge_sha: SHA.into(),
            ..PrCheck::default()
        },
    )
    .unwrap();
    r.core.poll_desk(REPO).unwrap();
    assert_eq!(sha_comments(&r, &t.id, SHA), 1);
    assert_eq!(gh.viewed.lock().unwrap().len(), 1);
}

#[test]
fn a_pr_of_another_repo_or_still_open_is_not_asked() {
    let (r, gh) = rig();
    let co = r.core.coord();
    let t = co
        .create_task(
            OTHER,
            NewTask {
                title: "two PRs".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.claim_task(OTHER, &t.id).unwrap();
    co.add_link(OTHER, &t.id, &url(3)).unwrap();
    co.add_link(OTHER, &t.id, "https://github.com/someone/else/pull/4")
        .unwrap();
    *gh.open.lock().unwrap() = vec![pr(3)];
    r.core.poll_desk(REPO).unwrap();
    assert!(gh.viewed.lock().unwrap().is_empty());
}
