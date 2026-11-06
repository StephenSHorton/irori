//! Merge desk slice 2 on a real Core: after a poll, each trusted PR shows
//! its author card (linked by Irori) or gets a "PR #n" card; an untrusted
//! login's PR isn't ours (card #625): not listed, no card and no wake.

use crate::common;

use std::sync::{Arc, Mutex};

use common::*;
use coord::{DeskPr, NewTask, Where, new_desk};
use irori_core::deskmap::Verdict;
use irori_core::deskwatch::PrLister;

const REPO: &str = "remote:github.com/StephenSHorton/ww-multiplayer";

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

fn url(n: i64) -> String {
    format!("https://github.com/StephenSHorton/ww-multiplayer/pull/{n}")
}

fn pr(n: i64, login: &str, branch: &str) -> DeskPr {
    DeskPr {
        number: n,
        url: url(n),
        title: format!("PR {n} title"),
        author: login.into(),
        head_ref_name: branch.into(),
        head_ref_oid: format!("sha{n}"),
        ..DeskPr::default()
    }
}

fn linked(r: &Rig, n: i64) -> Vec<coord::Task> {
    r.core
        .coord()
        .list_tasks(&coord::TaskFilter::default())
        .into_iter()
        .filter(|t| t.links.iter().any(|l| l.reference == url(n)))
        .collect()
}

#[test]
fn a_poll_maps_each_pr_to_its_author_card() {
    let gh = Arc::new(FakeGh(Mutex::new(Vec::new())));
    let g = gh.clone();
    let r = Rig::with(move |o| o.desk_lister = Some(g));
    let co = r.core.coord();
    let me = format!("user:{}", r.core.device_id());
    let mut d = new_desk(REPO, r.core.device_id());
    d.trusted = vec!["StephenSHorton".into()];
    d.holder = r.key_b.clone();
    co.set_desk(&me, d).unwrap();

    // 1: a card already linking PR 1. 3: a card on PR 3's branch.
    let new = |title: &str| NewTask {
        title: title.into(),
        ..NewTask::default()
    };
    let c1 = co.create_task(&r.key_a, new("one")).unwrap();
    co.claim_task(&r.key_a, &c1.id).unwrap();
    co.add_link(&r.key_a, &c1.id, &url(1)).unwrap();
    let c3 = co.create_task(&r.key_b, new("three")).unwrap();
    co.claim_task(&r.key_b, &c3.id).unwrap();
    co.set_where(
        &c3.id,
        Where {
            who: r.key_b.clone(),
            branch: "kestrel/3-fix".into(),
            ..Where::default()
        },
    )
    .unwrap();

    *gh.0.lock().unwrap() = vec![
        pr(1, "someone", "x"),
        pr(2, "StephenSHorton", "stephen/2"),
        pr(3, "StephenSHorton", "kestrel/3-fix"),
        pr(4, "a-stranger", "kestrel/3-fix"),
        pr(5, "a-stranger", "fork/5"),
    ];
    let before = co.list_tasks(&coord::TaskFilter::default()).len();
    r.core.poll_desk(REPO).unwrap();
    let v: std::collections::BTreeMap<i64, Verdict> = r.core.map_desk(REPO).into_iter().collect();

    // 1: linked by the agent, held by the person's agent: trusted.
    assert!(matches!(&v[&1], Verdict::Card(m) if m.task_id == c1.id));
    // 2: a trusted login with no card: a "PR #2" card, linked, the desk
    // holder watching it.
    let two = linked(&r, 2);
    assert_eq!(two.len(), 1, "{two:?}");
    assert_eq!(two[0].title, "PR #2 \"PR 2 title\"");
    assert!(two[0].watchers.contains(&r.key_b), "{:?}", two[0].watchers);
    assert!(matches!(&v[&2], Verdict::Card(m) if m.task_id == two[0].id));
    // 3: rule 3 found the branch card, and Irori linked it.
    assert_eq!(
        linked(&r, 3).iter().map(|t| &t.id).collect::<Vec<_>>(),
        vec![&c3.id]
    );
    // 4 and 5: a stranger's PR isn't ours, even on our branch name: no
    // card, no link (card #625).
    assert_eq!(v[&4], Verdict::NotOurs);
    assert_eq!(v[&5], Verdict::NotOurs);
    assert!(linked(&r, 4).is_empty() && linked(&r, 5).is_empty());
    // The page's read-only view agrees, and writes nothing.
    let n = co.list_tasks(&coord::TaskFilter::default()).len();
    let authors: std::collections::BTreeMap<i64, irori_core::deskmap::DeskAuthor> = r
        .core
        .desk_verdicts(REPO)
        .into_iter()
        .map(|a| (a.pr, a))
        .collect();
    assert_eq!(co.list_tasks(&coord::TaskFilter::default()).len(), n);
    assert_eq!(authors[&1].card, c1.id);
    assert_eq!(authors[&1].card_ref, c1.short_ref());
    assert_eq!(authors[&1].agent, r.key_a);
    assert_eq!(authors[&3].card, c3.id);
    assert!(!authors[&2].card.is_empty());
    // A PR that isn't ours isn't listed at all.
    assert_eq!(authors.keys().copied().collect::<Vec<_>>(), [1, 2, 3]);
    // Only PR 2's card is new, and a second pass makes no more.
    assert_eq!(
        co.list_tasks(&coord::TaskFilter::default()).len(),
        before + 1
    );
    r.core.map_desk(REPO);
    assert_eq!(
        co.list_tasks(&coord::TaskFilter::default()).len(),
        before + 1
    );
    // No wake for PRs that aren't ours: nobody heard about 4 or 5.
    for k in [&r.key_a, &r.key_b] {
        assert!(
            r.notices_to(k)
                .iter()
                .all(|d| !d.message.text.contains("/pull/4") && !d.message.text.contains("/pull/5")),
        );
    }
}

/// ww's author cards live on a shared board: the desk finds them there
/// (keys mapped back to this device's) and doesn't make a "PR #n" card.
#[test]
fn an_author_card_on_a_shared_board_is_found() {
    let gh = Arc::new(FakeGh(Mutex::new(Vec::new())));
    let g = gh.clone();
    let r = Rig::with(move |o| o.desk_lister = Some(g));
    let c = &r.core;
    let me = format!("user:{}", c.device_id());
    c.coord()
        .set_desk(&me, new_desk(REPO, c.device_id()))
        .unwrap();
    let b = c.create_board("wind", "wind").unwrap();
    let co = c.board_coord(&b).unwrap();
    let card_a = c.board_key(&b, &r.key_a).unwrap();
    let t = co
        .create_task(
            &card_a,
            NewTask {
                title: "co-op sync".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.claim_task(&card_a, &t.id).unwrap();
    co.add_link(&card_a, &t.id, &url(7)).unwrap();

    // Linked on the board by the person's agent: trusted, though the login
    // is on no list, and no new card anywhere.
    *gh.0.lock().unwrap() = vec![pr(7, "agent-bot", "toffee/7")];
    let private_before = c.coord().list_tasks(&coord::TaskFilter::default()).len();
    c.poll_desk(REPO).unwrap();
    let v = c.map_desk(REPO);
    assert!(
        matches!(&v[0].1, Verdict::Card(m) if m.task_id == t.id),
        "{v:?}"
    );
    assert_eq!(
        c.coord().list_tasks(&coord::TaskFilter::default()).len(),
        private_before
    );
}

/// A "PR #n" card Irori makes goes to the ledger's author agent: Irori,
/// as its creator, may assign it.
#[test]
fn irori_may_assign_the_pr_card_it_made() {
    let r = Rig::new();
    let co = r.core.coord();
    let t = co
        .create_task(
            "irori",
            NewTask {
                title: "PR #9".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.assign_task("irori", &t.id, &r.key_a).unwrap();
    let t = co.get_task(&t.id).unwrap();
    assert!(
        t.holders.contains(&r.key_a) || t.assigned.contains(&r.key_a),
        "{t:?}"
    );
}
