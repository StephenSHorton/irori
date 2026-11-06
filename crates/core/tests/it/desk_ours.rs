//! Card #625 on a real Core: a merge desk only on the person's repos, those
//! they or their people open PRs in. A cloned upstream repo an agent only
//! reads has a stray desk: never polled or listed. A repo the person's
//! login owns, or one GitHub's search finds their PRs in, is theirs, and
//! its desk lists only their PRs: a stranger's is not shown and gets no
//! card.

use crate::common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use common::*;
use coord::DeskPr;
use irori_core::deskmap::Verdict;
use irori_core::deskwatch::PrLister;
use serde_json::json;
use store::kinds;

/// The person's own repo: their login owns it.
const OWNED: &str = "remote:github.com/stephenshorton/ww-multiplayer";
/// Someone else's repo the person opened a PR in (GitHub's search says).
const AUTHORED: &str = "remote:github.com/ggml-org/llama.cpp";
/// A cloned upstream an agent only reads: nobody here opened a PR there.
const UPSTREAM: &str = "remote:github.com/blaizzy/mlx-video";

/// A gh signed in as "StephenSHorton" whose PR search finds llama.cpp.
struct FakeGh {
    answers: Mutex<HashMap<String, Vec<DeskPr>>>,
    listed: Mutex<Vec<String>>,
    searched: Mutex<Vec<String>>,
}

impl PrLister for FakeGh {
    fn list(&self, repo: &str, _limit: usize) -> Result<Vec<DeskPr>, String> {
        let repo = repo.to_ascii_lowercase();
        self.listed.lock().unwrap().push(repo.clone());
        Ok(self
            .answers
            .lock()
            .unwrap()
            .get(&repo)
            .cloned()
            .unwrap_or_default())
    }

    fn viewer(&self) -> Result<String, String> {
        Ok("StephenSHorton".into())
    }

    fn authored_repos(&self, login: &str) -> Result<Vec<String>, String> {
        self.searched.lock().unwrap().push(login.to_string());
        Ok(vec!["ggml-org/llama.cpp".into()])
    }
}

impl FakeGh {
    fn listed(&self, repo: &str) -> usize {
        let repo = coord::github_repo(repo).unwrap();
        self.listed
            .lock()
            .unwrap()
            .iter()
            .filter(|r| **r == repo)
            .count()
    }
}

fn pr(repo: &str, n: i64, author: &str) -> DeskPr {
    let repo = coord::github_repo(repo).unwrap();
    DeskPr {
        number: n,
        url: format!("https://github.com/{repo}/pull/{n}"),
        title: format!("PR {n}"),
        author: author.into(),
        head_ref_name: format!("branch-{n}"),
        head_ref_oid: format!("{n}abcdef0"),
        ..DeskPr::default()
    }
}

/// The search answer is set before the Core starts: its first desk tick
/// searches at once, and the answer is kept for half an hour.
fn rig() -> (Rig, Arc<FakeGh>) {
    let mut answers = HashMap::new();
    for (repo, prs) in [
        (
            OWNED,
            vec![pr(OWNED, 7, "StephenSHorton"), pr(OWNED, 8, "a-stranger")],
        ),
        (
            AUTHORED,
            vec![
                pr(AUTHORED, 1, "stephenshorton"),
                pr(AUTHORED, 2, "a-stranger"),
            ],
        ),
        (UPSTREAM, vec![pr(UPSTREAM, 3, "a-stranger")]),
    ] {
        answers.insert(coord::github_repo(repo).unwrap(), prs);
    }
    let gh = Arc::new(FakeGh {
        answers: Mutex::new(answers),
        listed: Mutex::new(Vec::new()),
        searched: Mutex::new(Vec::new()),
    });
    let g = gh.clone();
    let r = Rig::with(move |o| o.desk_lister = Some(g));
    // The Core's first desk tick, at start, learns the login and searches.
    wait_for(10, "the first desk tick", || {
        !gh.searched.lock().unwrap().is_empty()
    });
    (r, gh)
}

/// What capture writes when an agent here first works in `repo`.
fn agent_works_in(r: &Rig, repo: &str) {
    let row = json!({"repoKey": repo, "root": "C:/src/x", "at": "2026-11-05T10:00:00Z"});
    r.core
        .store()
        .append("irori", kinds::REPO_SEEN, &row)
        .unwrap();
}

fn cards_linking(r: &Rig, url: &str) -> usize {
    r.core
        .coord()
        .list_tasks(&coord::TaskFilter::default())
        .into_iter()
        .filter(|t| t.links.iter().any(|l| l.reference == url))
        .count()
}

#[test]
fn only_the_persons_repos_have_a_desk_and_only_their_prs_show() {
    let (r, gh) = rig();
    let co = r.core.coord();
    for repo in [OWNED, AUTHORED, UPSTREAM] {
        agent_works_in(&r, repo);
    }
    wait_for(10, "the three desks", || co.desks().len() == 3);
    r.core.check_desks();

    // The cloned upstream: stray, never polled, nothing to show.
    let up = co.desk(UPSTREAM).unwrap();
    assert!(up.stray && !up.on(), "{up:?}");
    assert!(up.prs.is_none());
    assert_eq!(gh.listed(UPSTREAM), 0, "a stray desk is never polled");
    assert!(r.core.poll_desk(UPSTREAM).is_err());
    assert!(r.core.desk_verdicts(UPSTREAM).is_empty());

    // The person's own repo (their login owns it) and the one GitHub's
    // search found their PR in: on and polled.
    for repo in [OWNED, AUTHORED] {
        let d = co.desk(repo).unwrap();
        assert!(d.on() && !d.stray, "{repo}: {d:?}");
        assert_eq!(gh.listed(repo), 1, "{repo}");
    }
    let shown = |repo: &str| {
        r.core
            .desk_verdicts(repo)
            .into_iter()
            .map(|a| a.pr)
            .collect::<Vec<_>>()
    };
    assert_eq!(shown(OWNED), [7]);
    assert_eq!(shown(AUTHORED), [1]);

    // A stranger's PR is not ours: no card, while ours gets one.
    let v: HashMap<i64, Verdict> = r.core.map_desk(AUTHORED).into_iter().collect();
    assert_eq!(v[&2], Verdict::NotOurs);
    assert!(matches!(v[&1], Verdict::Card(_)), "{v:?}");
    assert_eq!(cards_linking(&r, &pr(AUTHORED, 1, "").url), 1);
    assert_eq!(cards_linking(&r, &pr(AUTHORED, 2, "").url), 0);
    assert_eq!(cards_linking(&r, &pr(OWNED, 8, "").url), 0);

    // GitHub's search is asked once per login, not every tick.
    r.core.check_desks();
    assert_eq!(*gh.searched.lock().unwrap(), ["stephenshorton"]);
    // And the upstream stays stray.
    assert!(co.desk(UPSTREAM).unwrap().stray);
    assert_eq!(gh.listed(UPSTREAM), 0);
}

/// A repo whose owner is the person's login is theirs with no PR at all.
#[test]
fn a_repo_the_persons_login_owns_is_theirs_with_no_pr() {
    let (r, gh) = rig();
    gh.answers.lock().unwrap().clear();
    agent_works_in(&r, OWNED);
    wait_for(10, "the desk", || r.core.coord().desk(OWNED).is_some());
    r.core.check_desks();
    let d = r.core.coord().desk(OWNED).unwrap();
    assert!(d.on() && !d.stray, "{d:?}");
    assert_eq!(gh.listed(OWNED), 1);
    assert!(d.prs.unwrap().prs.is_empty());
}

/// The person's login goes on their member.profile in each space, so
/// their members' desks count their PRs.
#[test]
fn the_persons_login_goes_on_their_space_profile() {
    let (r, _gh) = rig();
    let c = &r.core;
    let id = c.create_board("wind", "wind").unwrap();
    let me = c.identity().current().id;
    let v = c.board_verifier(&id).unwrap();
    c.check_desks();
    wait_for(10, "the login on the profile", || {
        v.profile_github_login(&me)
            .eq_ignore_ascii_case("StephenSHorton")
    });
}
