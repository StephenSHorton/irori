//! Card terminals (card #369): which folder a card's shell starts in
//! (inline-terminals.md 6.1), and opening one through the window.

mod common;

use common::*;
use irori_window::terms::Folder;

fn card(r: &Rig, repo: &str) -> coord::Task {
    r.w.core
        .coord()
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "term".into(),
                repo_key: repo.into(),
                ..Default::default()
            },
        )
        .unwrap()
}

fn paths(f: &[Folder]) -> Vec<(String, String)> {
    f.iter().map(|f| (f.path.clone(), f.why.clone())).collect()
}

#[test]
fn a_card_terminal_starts_where_its_holder_works() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let t = card(&r, "remote:github.com/x/nowhere");
    // Unheld, and this device has no checkout of its repo: the page asks.
    assert!(r.w.term_folders(&t.id).unwrap().is_empty());

    // Held by alpha, which runs in its session's folder.
    co.claim_task(&r.key_a, &t.id).unwrap();
    let session_cwd = r._dirs[2].path().to_string_lossy().into_owned();
    assert_eq!(
        paths(&r.w.term_folders(&t.id).unwrap()),
        vec![(session_cwd.clone(), "holder".into())]
    );

    // Its Where line names a worktree: that comes first.
    let wt = tempfile::tempdir().unwrap();
    let wt_path = wt.path().to_string_lossy().into_owned();
    co.set_where(
        &t.id,
        coord::Where {
            who: r.key_a.clone(),
            worktree: wt_path.clone(),
            cwd: session_cwd.clone(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        paths(&r.w.term_folders(&t.id).unwrap()),
        vec![
            (wt_path.clone(), "holder".into()),
            (session_cwd.clone(), "holder".into())
        ]
    );

    // A folder that is gone from this device is never offered.
    drop(wt);
    assert_eq!(
        paths(&r.w.term_folders(&t.id).unwrap()),
        vec![(session_cwd, "holder".into())]
    );
}

#[test]
fn a_card_with_no_repo_and_no_holder_here_opens_at_home() {
    let r = Rig::new();
    let t = card(&r, "");
    let f = r.w.term_folders(&t.id).unwrap();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].why, "home");
}

#[test]
fn a_card_can_hold_several_shells() {
    let r = Rig::new();
    let t = card(&r, "");
    let dir = tempfile::tempdir().unwrap();
    let scope = term::Scope::Card(t.id.clone());
    let cwd = Some(dir.path().to_string_lossy().into_owned());
    let a = r.w.term_open(scope.clone(), 80, 24, cwd.clone()).unwrap();
    let b = r.w.term_open(scope.clone(), 100, 30, cwd).unwrap();
    assert_ne!(a.id, b.id);
    let ids: Vec<u32> = r.w.term_list(&t.id).iter().map(|i| i.id).collect();
    assert_eq!(ids, vec![a.id, b.id]);
    assert!(
        r.w.term_open(
            term::Scope::Card("other".into()),
            80,
            24,
            Some("/no/such/folder".into())
        )
        .is_err()
    );
    r.w.core.terms().close(a.id).unwrap();
    r.w.core.terms().close(b.id).unwrap();
    assert!(r.w.term_list(&t.id).is_empty());
    // A killed shell may stay in its folder for a moment, and Windows can't
    // delete a folder in use. TempDir's own drop would give up at once.
    wait("the shells to leave their folder", || {
        std::fs::remove_dir_all(dir.path()).is_ok()
    });
}
