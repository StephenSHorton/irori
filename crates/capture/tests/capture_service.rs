//! Ports of I:internal/capture/capture_test.go (with Why and Decisions),
//! noise_test.go, watch_test.go and review_test.go, end to end through the
//! Rust tailer and repo pass. The search, show and overview tests are in
//! capture_search.rs; EventReflogFollowUp drives the private event loop and
//! is a unit test in src/watch.rs. Everything is synthetic: transcripts are
//! built from the documented record shapes, repos are fresh temp repos.

mod common;

use std::sync::Arc;
use std::time::Duration;

use capture::gitlink::{LINK_EXACT, LINK_UNATTRIBUTED};
use capture::gotime::{add, now, sub};
use capture::{
    ChangeKind, Dropped, EditQuery, EntryKind, KIND_COMMIT_CMD, KIND_COMMIT_LINK, KIND_COMMIT_SEEN,
    KIND_EDIT, KIND_PR, KIND_SUMMARY, KIND_TURN, REPLAY_KINDS, TimelineQuery, WhyQuery,
};
use common::*;
use serde_json::{Value as J, json};
use sys::cancel::Cancel;

fn sec(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn key_a() -> String {
    format!("{DEVICE}/{SID_A}")
}

fn join(parts: &[&str]) -> String {
    capture::gopath::join(parts)
}

/// Appends rows split mid-line, tailing between the pieces, as a live
/// writer would leave them.
fn write_in_pieces(r: &Rig, w: &Tw, rows: &[J]) {
    for m in rows {
        let mut b = serde_json::to_vec(m).unwrap();
        b.push(b'\n');
        let half = b.len() / 2;
        w.append_bytes(&b[..half]);
        r.tail();
        w.append_bytes(&b[half..]);
    }
    r.tail();
}

fn counts(r: &Rig) -> Vec<usize> {
    REPLAY_KINDS.iter().map(|k| r.count(k)).collect()
}

/// One session writes foo.go in dir and commits it with the given shell
/// tool, tailing in pieces along the way. Returns the commit.
fn edit_flow(r: &Rig, w: &Tw, dir: &str, content: &str, tool: &str) -> String {
    let foo_go = join(&[dir, "foo.go"]);
    let t = now_early_in_second(); // the edit is stamped 1.1 s after t
    write_in_pieces(
        r,
        w,
        &flat(vec![
            vec![w.prompt("p1", "add foo", &t)],
            w.write_tool("w1", &foo_go, content, None, &add(&t, sec(1))),
        ]),
    );
    r.repos(); // the root is known from the edit: start watching its reflog
    git(dir, &["add", "foo.go"]);
    git(dir, &["commit", "-q", "-m", "add foo"]);
    let sha = head(dir);
    let cmd = if tool == "PowerShell" {
        "git add foo.go; if ($?) { git commit -m @'\nadd foo\n'@ }"
    } else {
        "git add foo.go && git commit -m 'add foo'"
    };
    let at = now();
    write_in_pieces(
        r,
        w,
        &flat(vec![
            w.shell("b1", tool, cmd, "[main abc] add foo\n", None, &at),
            vec![w.reply("done", &at), w.done(&add(&at, sec(1)))],
        ]),
    );
    r.repos();
    sha
}

#[test]
fn end_to_end() {
    let r = Rig::new(|_| {});
    let (_d, dir) = new_repo("false");
    let w = r.session(SID_A, &dir);
    let sha = edit_flow(&r, &w, &dir, "package foo\n\nfunc A() {}\n", "Bash");
    let key = key_a();

    let turns = r.rows(KIND_TURN);
    assert_eq!(turns.len(), 1);
    let tr = &turns[0];
    assert_eq!(tr["key"], json!(key));
    assert_eq!(tr["turnId"], "p1");
    assert_eq!(tr["prompt"], "add foo");
    assert_eq!(tr["reply"], "done");
    assert_eq!(tr["endReason"], "turn_duration");
    assert_eq!(tr["editPaths"], json!(["foo.go"]));
    assert!(tr["repoKey"].as_str().unwrap().starts_with("root:"));

    let edits = r.c.edits(&EditQuery::default());
    assert_eq!(edits.len(), 1);
    let e = &edits[0];
    assert!(e.path == "foo.go" && gocompat::text::equal_fold(&e.root, &dir));
    assert!(
        !e.after_blob.is_empty()
            && !e.patch.is_empty()
            && e.fidelity == "exact"
            && e.create
            && e.adds == 3
    );
    let p = String::from_utf8(r.c.blob(&e.patch).unwrap()).unwrap();
    assert!(p.contains("+func A() {}"), "{p}");

    let cv = r.c.commit(&sha).expect("commit seen");
    let link = cv.link.as_ref().expect("linked");
    assert_eq!(
        (
            cv.row.op.as_str(),
            cv.row.branch.as_str(),
            cv.row.commit.subject.as_str()
        ),
        ("commit", "main", "add foo")
    );
    let files = link.files();
    assert_eq!(files.len(), 1);
    assert_eq!(
        (files[0].path.as_str(), files[0].link.as_str()),
        ("foo.go", LINK_EXACT)
    );
    assert_eq!(files[0].edits, vec![e.id.clone()]);
    assert_eq!(link.sessions, vec![key.clone()]);
    assert_eq!(link.turns, vec!["p1".to_string()]);
    let who = link.committer.as_ref().expect("committer");
    assert_eq!(
        (who.key.as_str(), who.turn_id.as_str()),
        (key.as_str(), "p1")
    );
    assert_eq!(r.c.commits(&e.repo_key, 0).len(), 1);
    assert_eq!(r.c.repos()[&e.repo_key].len(), 1);

    let tl = r.c.timeline(&TimelineQuery {
        key: key.clone(),
        ..TimelineQuery::default()
    });
    assert_eq!(tl.entries.len(), 2, "{tl:?}");
    assert!(!tl.more);
    let turn = tl
        .entries
        .iter()
        .find(|x| x.kind == EntryKind::Turn)
        .unwrap();
    assert_eq!(turn.turn.as_ref().unwrap().edits.len(), 1);
    let com = tl
        .entries
        .iter()
        .find(|x| x.kind == EntryKind::Commit)
        .unwrap();
    assert_eq!(com.commit.as_ref().unwrap().row.commit.sha, sha);

    // Why on a committed line names the commit and the turn.
    let why =
        r.c.why(&WhyQuery {
            path: "foo.go".into(),
            line: 3,
            cwd: dir.clone(),
            ..WhyQuery::default()
        })
        .unwrap();
    assert!(
        why.blame_sha == sha
            && why.commit.is_some()
            && why.edits.len() == 1
            && why.turns.len() == 1
            && why.turns[0].turn.id == "p1"
            && why.path == "foo.go",
        "why = {why:?}"
    );

    // An uncommitted line: the edits since the last commit.
    let t = now();
    let orig = "package foo\n\nfunc A() {}\n";
    write_in_pieces(
        &r,
        &w,
        &flat(vec![
            vec![w.prompt("p2", "add B", &t)],
            w.write_tool(
                "w2",
                &join(&[&dir, "foo.go"]),
                &format!("{orig}\nfunc B() {{}}\n"),
                Some(orig),
                &t,
            ),
            vec![w.done(&add(&t, sec(1)))],
        ]),
    );
    let why =
        r.c.why(&WhyQuery {
            path: join(&[&dir, "foo.go"]),
            line: 5,
            ..WhyQuery::default()
        })
        .unwrap();
    assert!(
        why.blame_sha.is_empty()
            && why.commit.is_none()
            && why.edits.len() == 1
            && why.edits[0].turn_id == "p2"
            && why.turns.len() == 1,
        "uncommitted why = {why:?}"
    );
    assert_eq!(
        r.c.why(&WhyQuery {
            path: "foo.go".into(),
            ..WhyQuery::default()
        })
        .err(),
        Some(capture::Error::NoRepo),
        "no cwd"
    );
}

/// TestDecisions: record, anchor, re-check, supersede, fall back to a live
/// card's root, replay.
#[test]
fn decisions() {
    use coord::decision::{AnchorState, Filter, Party};
    let mut r = Rig::new(|_| {});
    let (_d, dir) = new_repo("false");
    write_file(&join(&[&dir, "a.go"]), "one\ntwo\nthree\nfour\n");
    let by = Party {
        key: key_a(),
        name: "alpha".into(),
        kind: "claude".into(),
    };
    let d =
        r.c.record_decision(&capture::DecisionInput {
            by: by.clone(),
            statement: " Keep two and three ".into(),
            why: "they pair".into(),
            path: "a.go".into(),
            lines: "2-3".into(),
            cwd: dir.clone(),
            ..Default::default()
        })
        .unwrap();
    let a = d.anchor.as_ref().expect("anchored");
    assert!(
        d.id.starts_with("d-")
            && d.statement == "Keep two and three"
            && a.path == "a.go"
            && a.start == 2
            && a.sha == head(&dir)
            && d.trust == "untrusted",
        "decision = {d:?}"
    );
    let list = r.c.decisions(&Filter::default());
    assert!(
        list.len() == 1 && list[0].status.is_some_and(|s| s.state == AnchorState::Ok),
        "decisions = {list:?}"
    );
    write_file(&join(&[&dir, "a.go"]), "zero\none\ntwo\nthree\nfour\n");
    let s = r.c.decisions(&Filter::default())[0].status.unwrap();
    assert!(
        s.state == AnchorState::Moved && s.start == 3,
        "status = {s:?}"
    );
    let q = |line| WhyQuery {
        path: "a.go".into(),
        line,
        cwd: dir.clone(),
        ..WhyQuery::default()
    };
    let why = r.c.why(&q(3)).unwrap();
    assert_eq!(
        why.decisions.len(),
        1,
        "why decisions = {:?}",
        why.decisions
    );
    assert_eq!(r.c.why(&q(1)).unwrap().decisions.len(), 0, "line 1");

    // Only the author or the person supersedes.
    let other = Party {
        key: format!("{DEVICE}/{SID_B}"),
        kind: "claude".into(),
        ..Party::default()
    };
    let refused = r.c.record_decision(&capture::DecisionInput {
        by: other,
        statement: "no".into(),
        supersedes: vec![d.id.clone()],
        ..Default::default()
    });
    assert!(refused.is_err(), "another agent superseded");
    let user = Party {
        key: format!("user:{DEVICE}"),
        kind: "user".into(),
        ..Party::default()
    };
    let d2 =
        r.c.record_decision(&capture::DecisionInput {
            by: user,
            trust: "user".into(),
            statement: "replaced".into(),
            supersedes: vec![d.id.clone()],
            ..Default::default()
        })
        .unwrap();
    let list = r.c.decisions(&Filter::default());
    assert!(
        list.len() == 1 && list[0].listed.decision.id == d2.id && list[0].status.is_none(),
        "after supersede = {list:?}"
    );
    let all = Filter {
        all: true,
        ..Filter::default()
    };
    assert_eq!(r.c.decisions(&all).len(), 2);
    let empty = r.c.record_decision(&capture::DecisionInput {
        by: by.clone(),
        ..Default::default()
    });
    assert_eq!(
        empty.err(),
        Some(capture::Error::Decision(coord::decision::Error::Empty))
    );

    // The recorded root is gone: a live card's root in the same repo is
    // used, else the status is unknown.
    let (_t, base) = tempdir();
    let wt = join(&[&base, "wt"]);
    git(&dir, &["worktree", "add", "-q", "-b", "side", &wt]);
    write_file(&join(&[&wt, "b.go"]), "x\ny\n");
    let db =
        r.c.record_decision(&capture::DecisionInput {
            by: by.clone(),
            statement: "b".into(),
            path: join(&[&wt, "b.go"]),
            ..Default::default()
        })
        .unwrap();
    git(&dir, &["worktree", "remove", "--force", &wt]);
    let status = |r: &Rig| {
        r.c.decisions(&Filter::default())
            .into_iter()
            .find(|v| v.listed.decision.id == db.id)
            .expect("missing")
            .status
            .unwrap()
            .state
    };
    assert_eq!(status(&r), AnchorState::Unknown, "no worktree");
    r.roster.set(vec![live_card(SID_A, &dir, "idle")]);
    assert_eq!(
        status(&r),
        AnchorState::Gone,
        "live card's root (b.go is not on main)"
    );
    assert_eq!(r.count(capture::KIND_DECISION), 3);
    r.restart();
    assert_eq!(r.c.decisions(&all).len(), 3, "after restart");
}

#[test]
fn crlf_links_exact() {
    let r = Rig::new(|_| {});
    let (_d, dir) = new_repo("true");
    let sha = edit_flow(
        &r,
        &r.session(SID_A, &dir),
        &dir,
        "package foo\r\n\r\nfunc A() {}\r\n",
        "Bash",
    );
    let cv = r.c.commit(&sha).unwrap();
    let l = cv.link.as_ref().unwrap();
    assert_eq!(l.files()[0].link, LINK_EXACT);
    let e = &r.c.edits(&EditQuery::default())[0];
    let blob = &cv.row.commit.files.as_ref().unwrap()[0].blob;
    assert!(!e.after_blob_lf.is_empty() && &e.after_blob_lf == blob && &e.after_blob != blob);
}

#[test]
fn powershell_committer() {
    let r = Rig::new(|_| {});
    let (_d, dir) = new_repo("false");
    let sha = edit_flow(
        &r,
        &r.session(SID_A, &dir),
        &dir,
        "package foo\n",
        "PowerShell",
    );
    let cmds = r.rows(KIND_COMMIT_CMD);
    assert_eq!(cmds.len(), 1);
    assert_eq!(
        cmds[0]["command"],
        "git add foo.go; if ($?) { git commit -m @'"
    );
    assert_eq!(cmds[0]["turnId"], "p1");
    let cv = r.c.commit(&sha).unwrap();
    assert_eq!(cv.link.unwrap().committer.unwrap().key, key_a());
}

/// A workflow agent commits in a linked worktree, the worktree is removed,
/// and main fast-forwards to its branch: the commit links to the edit by
/// blob through main's reflog.
#[test]
fn fast_forward_from_removed_worktree() {
    let r = Rig::new(|_| {});
    let (_d, main) = new_repo("false");
    let (_t, base) = tempdir();
    let wt = join(&[&base, "wt"]);
    git(&main, &["worktree", "add", "-q", "-b", "wf", &wt]);
    r.roster.set(vec![live_card(SID_B, &main, "idle")]);
    let w = r.session(SID_A, &wt);
    let t = now();
    w.write(&flat(vec![
        vec![w.prompt("p1", "work", &t)],
        w.write_tool(
            "w1",
            &join(&[&wt, "pkg", "foo.go"]),
            "package pkg\n",
            None,
            &t,
        ),
        vec![w.done(&add(&t, sec(1)))],
    ]));
    r.tail();
    r.repos();
    assert_eq!(r.c.repos().len(), 1, "{:?}", r.c.repos());
    git(&wt, &["add", "pkg/foo.go"]);
    git(&wt, &["commit", "-q", "-m", "wf work"]);
    let sha = head(&wt);
    git(&main, &["worktree", "remove", &wt]);
    git(&main, &["merge", "-q", "--ff-only", "wf"]);
    r.repos();
    let cv = r.c.commit(&sha).expect("commit seen");
    assert!(gocompat::text::equal_fold(&cv.row.root, &main) && cv.row.op.starts_with("merge"));
    let l = cv.link.unwrap();
    let f = l.files();
    assert!(
        f.len() == 1
            && f[0].link == LINK_EXACT
            && f[0].path == "pkg/foo.go"
            && f[0].edits.len() == 1
    );
    assert_eq!(l.sessions, vec![key_a()]);
}

/// Two worktrees edit one path differently; each commit links only its own
/// worktree's edit.
#[test]
fn two_worktrees_same_path() {
    let r = Rig::new(|_| {});
    let (_d, main) = new_repo("false");
    let (_t, base) = tempdir();
    let wt = join(&[&base, "wt2"]);
    git(&main, &["worktree", "add", "-q", "-b", "other", &wt]);
    let (a, b) = (r.session(SID_A, &main), r.session(SID_B, &wt));
    let t = now();
    a.write(&flat(vec![
        vec![a.prompt("pa", "a", &t)],
        a.write_tool("wa", &join(&[&main, "foo.go"]), "A\n", None, &t),
        vec![a.done(&t)],
    ]));
    b.write(&flat(vec![
        vec![b.prompt("pb", "b", &t)],
        b.write_tool("wb", &join(&[&wt, "foo.go"]), "B\n", None, &t),
        vec![b.done(&t)],
    ]));
    r.tail();
    r.repos();
    git(&main, &["add", "foo.go"]);
    git(&main, &["commit", "-q", "-m", "a"]);
    let sha_a = head(&main);
    git(&wt, &["add", "foo.go"]);
    git(&wt, &["commit", "-q", "-m", "b"]);
    let sha_b = head(&wt);
    r.repos();
    for (sha, sid) in [(sha_a, SID_A), (sha_b, SID_B)] {
        let l = r.c.commit(&sha).unwrap().link.unwrap();
        assert_eq!(l.files()[0].link, LINK_EXACT);
        assert_eq!(l.sessions, vec![format!("{DEVICE}/{sid}")]);
    }
}

#[test]
fn restart_no_duplicates() {
    let mut r = Rig::new(|_| {});
    let (_d, dir) = new_repo("false");
    let w = r.session(SID_A, &dir);
    edit_flow(&r, &w, &dir, "package foo\n", "Bash");
    // An open turn at shutdown: re-read whole after the restart.
    let t = now();
    w.write(&flat(vec![
        vec![w.prompt("p2", "more", &t)],
        w.write_tool("w2", &join(&[&dir, "bar.go"]), "package foo\n", None, &t),
    ]));
    r.tail();
    let before = counts(&r);
    r.restart();
    r.tail();
    r.repos();
    assert_eq!(counts(&r), before, "rows changed across a restart");
    w.write(&[w.done(&now())]);
    r.tail();
    let p2 = r
        .rows(KIND_TURN)
        .into_iter()
        .find(|t| t["turnId"] == "p2")
        .expect("p2 logged");
    assert_eq!(p2["prompt"], "more");
    assert_eq!(p2["origin"], "human");
    assert_eq!(p2["editPaths"], json!(["bar.go"]));
}

#[test]
fn crash_before_cursor_replays_without_duplicates() {
    let mut r = Rig::new(|_| {});
    let (_d, dir) = new_repo("false");
    let w = r.session(SID_A, &dir);
    edit_flow(&r, &w, &dir, "package foo\n", "Bash");
    let before = counts(&r);
    // The rows were appended but the cursors never saved, and the reflog
    // cursor stopped before the commit's line.
    let (mut cf, err) = capture::cursors::CursorFile::load(&r.data);
    assert!(err.is_none());
    let files: Vec<String> = cf.files.keys().cloned().collect();
    for p in files {
        cf.set_file(&p, 0);
    }
    let log = std::fs::read_to_string(join(&[&dir, ".git", "logs", "HEAD"])).unwrap();
    let trimmed = log.strip_suffix('\n').unwrap();
    let last = trimmed.rfind('\n').map_or(0, |i| i + 1) as i64;
    assert_eq!(cf.reflogs.len(), 1, "{:?}", cf.reflogs);
    let roots: Vec<String> = cf.reflogs.keys().cloned().collect();
    for p in roots {
        cf.set_reflog(&p, last);
    }
    cf.save(&r.data).unwrap();
    r.restart();
    r.tail();
    r.repos();
    assert_eq!(counts(&r), before, "replay duplicated rows");
}

#[test]
fn backfill_rules() {
    const MAX: i64 = 4096;
    let r = Rig::new(|o| o.backfill_max_bytes = MAX);
    let (_d, cwd) = tempdir();
    let old = sub(&now(), Duration::from_secs(100 * 3600));
    let turns_of = |w: &Tw, prefix: &str, n: usize| -> Vec<J> {
        (0..n)
            .flat_map(|i| {
                vec![
                    w.prompt(&format!("{prefix}{i}"), &"x".repeat(300), &old),
                    w.done(&old),
                ]
            })
            .collect()
    };
    let recent = r.session("recent0-1111-2222-3333-444444444444", &cwd);
    recent.write(&turns_of(&recent, "r", 2));
    let stale = r.session("stale000-1111-2222-3333-444444444444", &cwd);
    stale.write(&turns_of(&stale, "s", 2));
    let past = std::fs::FileTimes::new()
        .set_modified(old.to_system())
        .set_accessed(old.to_system());
    std::fs::File::options()
        .write(true)
        .open(&stale.path)
        .unwrap()
        .set_times(past)
        .unwrap();
    let big_dead = r.session("bigdead0-1111-2222-3333-444444444444", &cwd);
    big_dead.write(&turns_of(&big_dead, "d", 12));
    let big_live = r.session("biglive0-1111-2222-3333-444444444444", &cwd);
    big_live.write(&turns_of(&big_live, "b", 12));
    big_live.write(&[big_live.common(
        "system",
        &old,
        json!({"subtype": "compact_boundary", "content": "Conversation compacted"}),
    )]);
    big_live.write(&turns_of(&big_live, "c", 2));
    let no_boundary = r.session("nobound0-1111-2222-3333-444444444444", &cwd);
    no_boundary.write(&turns_of(&no_boundary, "n", 12));
    r.roster.set(vec![
        live_card(&big_live.sid, &cwd, "idle"),
        live_card(&no_boundary.sid, &cwd, "idle"),
    ]);

    r.tail();
    let ids = |sid: &str| -> Vec<String> {
        r.rows(KIND_TURN)
            .iter()
            .filter(|t| t["sessionId"] == sid)
            .map(|t| t["turnId"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(ids(&recent.sid), ["r0", "r1"]);
    assert!(ids(&stale.sid).is_empty() && ids(&big_dead.sid).is_empty());
    assert_eq!(ids(&big_live.sid), ["c0", "c1"]);
    let nb = ids(&no_boundary.sid);
    assert!(
        !nb.is_empty() && nb.len() < 12 && nb.last().unwrap() == "n11",
        "{nb:?}"
    );
    let sums = r.rows(KIND_SUMMARY);
    assert_eq!(sums.len(), 1);
    assert_eq!(sums[0]["kind"], "compact_boundary");

    // Files that started at their end still pick up new rows.
    let t = now();
    stale.write(&[stale.prompt("s9", "new", &t), stale.done(&t)]);
    big_dead.write(&[big_dead.prompt("d9", "new", &t), big_dead.done(&t)]);
    r.tail();
    let n = r
        .rows(KIND_TURN)
        .iter()
        .filter(|t| t["turnId"] == "s9" || t["turnId"] == "d9")
        .count();
    assert_eq!(n, 2);
    let st = r.c.status();
    assert!(
        !st.backfilling && st.files == 5 && st.backlog == 0 && !st.last_scan.is_zero(),
        "{st:?}"
    );
}

#[test]
fn subagent_attribution() {
    let r = Rig::new(|_| {});
    let (_d, dir) = new_repo("false");
    let main = r.session(SID_A, &dir);
    let sa = r.subagent(SID_A, &dir, "a1b2c3");
    let t = now();
    main.write(&[main.prompt("p1", "delegate", &t), main.done(&t)]);
    sa.write(&flat(vec![
        vec![sa.prompt("p1", "do it", &add(&t, sec(1)))],
        sa.write_tool(
            "s1",
            &join(&[&dir, "sub.go"]),
            "package sub\n",
            None,
            &add(&t, sec(1)),
        ),
        vec![sa.done(&add(&t, sec(2)))],
    ]));
    r.tail();
    let key = key_a();
    let es = r.c.edits(&EditQuery {
        key: key.clone(),
        ..EditQuery::default()
    });
    assert!(
        es.len() == 1 && es[0].agent_id == "a1b2c3" && es[0].key == key && es[0].path == "sub.go"
    );
    assert_eq!(
        r.count(KIND_TURN),
        2,
        "the subagent's same promptId must not collide"
    );
    let tl = r.c.timeline(&TimelineQuery {
        key: key.clone(),
        ..TimelineQuery::default()
    });
    assert!(
        tl.entries.len() == 1
            && tl.entries[0]
                .turn
                .as_ref()
                .unwrap()
                .row
                .turn
                .agent_id
                .is_empty()
    );
    let tl = r.c.timeline(&TimelineQuery {
        key,
        subagents: true,
        ..TimelineQuery::default()
    });
    assert_eq!(tl.entries.len(), 2);
    let first = tl.entries[0].turn.as_ref().unwrap();
    assert!(first.row.turn.agent_id == "a1b2c3" && first.edits.len() == 1);
}

#[test]
fn pr_ledger() {
    let mut r = Rig::new(|_| {});
    let (_d, dir) = new_repo("false");
    git(
        &dir,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/Owner/Repo.git",
        ],
    );
    let w = r.session(SID_A, &dir);
    let t = now();
    w.write(&flat(vec![
        vec![w.prompt("p1", "open a pr", &t)],
        w.shell("b1", "Bash", "gh pr create --fill", "https://github.com/Owner/Repo/pull/13\n",
            Some(json!({"pr": {"number": 13, "url": "https://github.com/Owner/Repo/pull/13", "action": "created"}})), &t),
        w.shell("b2", "PowerShell", "gh pr review 7 --approve", "", None, &add(&t, sec(1))),
        vec![
            json!({"type": "pr-link", "sessionId": SID_A, "prNumber": "12", "prUrl": "https://github.com/Owner/Repo/pull/12",
                   "prRepository": "Owner/Repo", "timestamp": ts(&t)}),
            w.done(&add(&t, sec(2))),
        ],
    ]));
    r.tail();
    let key = key_a();
    assert_eq!(
        r.c.pr_owners(&ledger::key("owner/repo", 13)),
        vec![key.clone()]
    );
    assert_eq!(r.c.pr_owners("pr:owner/repo#7"), vec![key.clone()]);
    let mut roles: Vec<(i64, String)> =
        r.c.prs("owner/repo")
            .iter()
            .map(|e| (e.number, e.role.as_str().to_string()))
            .collect();
    roles.sort();
    assert_eq!(
        roles,
        vec![
            (7, "reviewer".into()),
            (12, "touched".into()),
            (13, "author".into())
        ]
    );
    for ev in
        r.st.list(&store::Filter {
            kinds: vec![KIND_PR.into()],
            limit: 100,
            ..store::Filter::default()
        })
        .unwrap()
    {
        let e: J = serde_json::from_slice(&ev.payload).unwrap();
        let want = if e["source"] == "pr-link" {
            "irori".to_string()
        } else {
            format!("agent:{key}")
        };
        assert_eq!(ev.actor, want);
    }
    let tl = r.c.timeline(&TimelineQuery {
        key: key.clone(),
        ..TimelineQuery::default()
    });
    assert_eq!(
        tl.entries
            .iter()
            .filter(|e| e.kind == EntryKind::Pr)
            .count(),
        3
    );
    r.restart();
    assert_eq!(r.c.pr_owners(&ledger::key("owner/repo", 13)), vec![key]);
}

#[test]
fn scrubbed_secrets_never_stored() {
    let exact = "irori-lockfile-token-0123456789";
    let gh_token = format!("ghp_{}", "A1b2".repeat(9));
    let scrub = Arc::new(ExactScrub(vec![exact.into(), gh_token.clone()]));
    let r = Rig::new(|o| o.scrub = scrub);
    let (_d, dir) = new_repo("false");
    let w = r.session(SID_A, &dir);
    let t = now();
    let content = format!("package cfg\n\nconst key = \"{gh_token}\"\nconst tok = \"{exact}\"\n");
    w.write(&flat(vec![
        vec![w.prompt("p1", &format!("use token {gh_token} and {exact}"), &t)],
        w.write_tool("w1", &join(&[&dir, "cfg.go"]), &content, None, &t),
        w.write_tool(
            "w2",
            &join(&[&dir, ".env"]),
            &format!("TOKEN={gh_token}\n"),
            None,
            &t,
        ),
        w.shell(
            "b1",
            "Bash",
            &format!("git commit -m x # token={exact}"),
            "",
            None,
            &t,
        ),
        vec![w.reply(&format!("set {gh_token}"), &t), w.done(&t)],
    ]));
    r.tail();
    for ev in
        r.st.list(&store::Filter {
            limit: 1000,
            ..store::Filter::default()
        })
        .unwrap()
    {
        let s = String::from_utf8(ev.payload).unwrap();
        assert!(
            !s.contains(&gh_token) && !s.contains(exact),
            "{} row holds a secret: {s}",
            ev.kind
        );
    }
    let es = r.c.edits(&EditQuery::default());
    assert_eq!(es.len(), 2);
    for e in es {
        if e.path == ".env" {
            assert!(e.patch.is_empty() && e.adds == 1, "{e:?}");
            continue;
        }
        let p = String::from_utf8(r.c.blob(&e.patch).unwrap()).unwrap();
        assert!(
            !p.contains(&gh_token) && !p.contains(exact) && p.contains("[redacted:"),
            "{p}"
        );
    }
    let tr = &r.rows(KIND_TURN)[0];
    assert!(tr["prompt"].as_str().unwrap().contains("[redacted:"));
    assert!(tr["reply"].as_str().unwrap().contains("[redacted:"));
}

#[test]
fn quiet_close() {
    let r = Rig::new(|o| o.quiet_after = Duration::from_secs(60));
    let (_d, cwd) = tempdir();
    let w = r.session(SID_A, &cwd);
    r.roster.set(vec![live_card(SID_A, &cwd, "busy")]);
    let t = now();
    w.write(&[w.prompt("p1", "long job", &t), w.reply("working", &t)]);
    r.tail();
    let q = TimelineQuery {
        key: key_a(),
        ..TimelineQuery::default()
    };
    let tl = r.c.timeline(&q);
    assert_eq!(tl.entries.len(), 1);
    let tv = tl.entries[0].turn.as_ref().unwrap();
    assert!(tv.open && tv.row.turn.prompt == "long job");
    r.clock.advance(Duration::from_secs(120));
    r.tail();
    assert_eq!(r.count(KIND_TURN), 0, "closed a busy session's turn");
    r.roster.set(vec![live_card(SID_A, &cwd, "idle")]);
    r.tail();
    let turns = r.rows(KIND_TURN);
    assert!(turns.len() == 1 && turns[0]["endReason"] == "quiet");
    let tl = r.c.timeline(&q);
    assert!(tl.entries.len() == 1 && !tl.entries[0].turn.as_ref().unwrap().open);
}

#[test]
fn timeline_paging() {
    let r = Rig::new(|_| {});
    let (_d, cwd) = tempdir();
    let w = r.session(SID_A, &cwd);
    let base = sub(&now(), Duration::from_secs(3600));
    for i in 0..5u64 {
        let at = add(&base, Duration::from_secs(60 * i));
        w.write(&[
            w.prompt(&format!("p{i}"), "n", &at),
            w.done(&add(&at, sec(1))),
        ]);
    }
    w.write(&[w.common(
        "ai-title",
        &gocompat::time::GoTime::ZERO,
        json!({"aiTitle": "A title"}),
    )]);
    r.tail();
    let ids = |tl: &capture::Timeline| -> Vec<String> {
        tl.entries
            .iter()
            .map(|e| match &e.turn {
                Some(t) => t.row.turn.id.clone(),
                None => "summary".into(),
            })
            .collect()
    };
    let mut q = TimelineQuery {
        key: key_a(),
        limit: 2,
        ..TimelineQuery::default()
    };
    let page = r.c.timeline(&q);
    assert_eq!(ids(&page), ["summary", "p4", "p3"]);
    assert!(page.more);
    q.before = page.entries.last().unwrap().at;
    let page = r.c.timeline(&q);
    assert_eq!(ids(&page), ["p2", "p1"]);
    assert!(page.more);
    q.before = page.entries.last().unwrap().at;
    let page = r.c.timeline(&q);
    assert_eq!(ids(&page), ["p0"]);
    assert!(!page.more);
}

#[test]
fn relink_late_edits() {
    let r = Rig::new(|_| {});
    let (_d, dir) = new_repo("false");
    r.roster.set(vec![live_card(SID_B, &dir, "idle")]);
    r.repos(); // watching dir
    let foo_go = join(&[&dir, "foo.go"]);
    write_file(&foo_go, "package foo\n");
    let at = now();
    git(&dir, &["add", "foo.go"]);
    git(&dir, &["commit", "-q", "-m", "x"]);
    let sha = head(&dir);
    r.repos();
    let l = r.c.commit(&sha).unwrap().link.unwrap();
    assert_eq!(l.files()[0].link, LINK_UNATTRIBUTED);
    // The transcript catches up after the commit was seen.
    let w = r.session(SID_A, &dir);
    let early = sub(&at, sec(1));
    w.write(&flat(vec![
        vec![w.prompt("p1", "x", &early)],
        w.write_tool("w1", &foo_go, "package foo\n", None, &early),
        vec![w.done(&at)],
    ]));
    r.tail();
    let l = r.c.commit(&sha).unwrap().link.unwrap();
    assert_eq!(l.files()[0].link, LINK_EXACT);
    assert_eq!(l.sessions, vec![key_a()]);
    assert_eq!(r.count(KIND_COMMIT_LINK), 2);
}

#[test]
fn run_and_subscribe() {
    let r = Rig::new(|_| {});
    let (_d, cwd) = tempdir();
    let changes = r.c.subscribe();
    let cancel = Cancel::new();
    let c = r.c.clone();
    let cc = cancel.clone();
    let h = std::thread::spawn(move || c.run(&cc));
    let w = r.session(SID_A, &cwd);
    let t = now();
    w.write(&[w.prompt("p1", "hi", &t), w.done(&t)]);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match changes.recv_timeout(left) {
            Ok(Some(ch)) if ch.kind == ChangeKind::Turn && ch.key == key_a() => break,
            Ok(Some(_)) => {}
            _ => panic!("no turn change"),
        }
    }
    cancel.cancel();
    h.join().unwrap();
    assert!(std::path::Path::new(&join(&[&r.data, "capture-cursors.json"])).exists());
}

/// The edit is made and committed before the tailer read it, so the root is
/// first seen after its commit; first sight still links it exactly.
#[test]
fn commit_before_root_first_seen() {
    let r = Rig::new(|_| {});
    let (_d, dir) = new_repo("false");
    let w = r.session(SID_A, &dir);
    let foo_go = join(&[&dir, "foo.go"]);
    let t = sub(&now(), sec(5));
    w.write(&flat(vec![
        vec![w.prompt("p1", "add foo", &t)],
        w.write_tool("w1", &foo_go, "package foo\n", None, &add(&t, sec(1))),
    ]));
    git(&dir, &["add", "foo.go"]);
    git(&dir, &["commit", "-q", "-m", "add foo"]);
    let sha = head(&dir);
    r.tail();
    r.repos();
    let l = r.c.commit(&sha).expect("commit seen").link.expect("linked");
    assert!(l.files().len() == 1 && l.files()[0].link == LINK_EXACT);
}

/// A repo pass over its budget stops after a commit and the next pass
/// carries on from the same reflog line; nothing is lost or doubled.
#[test]
fn repo_pass_budget() {
    let r = Rig::new(|o| o.repo_budget = Duration::ZERO);
    let (_d, dir) = new_repo("false");
    let w = r.session(SID_A, &dir);
    let foo_go = join(&[&dir, "foo.go"]);
    let t = sub(&now(), sec(5));
    w.write(&flat(vec![
        vec![w.prompt("p1", "add foo", &t)],
        w.write_tool("w1", &foo_go, "package foo\n", None, &add(&t, sec(1))),
    ]));
    git(&dir, &["checkout", "-q", "-b", "side"]);
    for (i, body) in [
        "package foo\n",
        "package foo\n// b\n",
        "package foo\n// c\n",
    ]
    .iter()
    .enumerate()
    {
        write_file(&foo_go, body);
        git(&dir, &["add", "foo.go"]);
        git(
            &dir,
            &["commit", "-q", "--allow-empty", "-m", &format!("c{i}")],
        );
    }
    git(&dir, &["checkout", "-q", "main"]);
    r.tail();
    git(&dir, &["merge", "-q", "--ff-only", "side"]);
    for _ in 0..10 {
        r.repos();
    }
    let rows = r.rows(KIND_COMMIT_SEEN);
    assert!(rows.len() >= 3, "{} commit.seen rows", rows.len());
    let mut seen = std::collections::HashSet::new();
    for row in rows {
        assert!(
            seen.insert(row["sha"].as_str().unwrap().to_string()),
            "logged twice"
        );
    }
}

/// A long command writes its result minutes after the commit, so the commit
/// is linked before the command is read; then the commit gets its committer.
#[test]
fn late_commit_command_sets_committer() {
    let r = Rig::new(|_| {});
    let (_d, dir) = new_repo("false");
    let w = r.session(SID_A, &dir);
    let foo_go = join(&[&dir, "foo.go"]);
    let start = sub(&now(), sec(5));
    let cmd = "python - <<'EOF'\nprint(1)\nEOF\ngit add foo.go && git commit -m 'add foo' && gh pr checks x --watch";
    w.write(&flat(vec![
        vec![w.prompt("p1", "add foo", &start)],
        w.write_tool("w1", &foo_go, "package foo\n", None, &add(&start, sec(1))),
        vec![w.tool_use(
            "b1",
            "Bash",
            json!({"command": cmd, "description": "commit and watch"}),
            &add(&start, sec(2)),
        )],
    ]));
    git(&dir, &["add", "foo.go"]);
    git(&dir, &["commit", "-q", "-m", "add foo"]);
    let sha = head(&dir);
    r.tail();
    r.repos();
    let l = r.c.commit(&sha).expect("seen").link.expect("linked");
    assert!(l.committer.is_none());
    w.write(&[w.result("b1", json!({"stdout": "all checks passed\n", "stderr": "", "interrupted": false, "isImage": false}), &add(&now(), sec(180)))]);
    r.tail();
    let l = r.c.commit(&sha).unwrap().link.unwrap();
    assert_eq!(l.committer.as_ref().unwrap().key, key_a());
    assert!(l.files().len() == 1 && l.files()[0].link == LINK_EXACT);
}

// --- noise_test.go ---

/// A repo ignoring bin/ and *.log, with keep.log tracked anyway.
fn noisy_repo() -> (tempfile::TempDir, String) {
    let (d, dir) = new_repo("false");
    write_file(&join(&[&dir, ".gitignore"]), "bin/\n*.log\n");
    write_file(&join(&[&dir, "keep.log"]), "tracked\n");
    git(&dir, &["add", ".gitignore"]);
    git(&dir, &["add", "-f", "keep.log"]);
    git(&dir, &["commit", "-q", "-m", "ignore"]);
    (d, dir)
}

fn bash_diff(w: &Tw, id: &str, changed: &[String], at: &gocompat::time::GoTime) -> Vec<J> {
    let files: Vec<J> = changed
        .iter()
        .map(|p| json!({"filePath": p, "hunks": [{"oldStart": 1, "oldLines": 0, "newStart": 1, "newLines": 1, "lines": ["+x"]}]}))
        .collect();
    bash_diff_files(w, id, changed, files, at)
}

/// bash_diff with the files array given.
fn bash_diff_files(
    w: &Tw,
    id: &str,
    changed: &[String],
    files: Vec<J>,
    at: &gocompat::time::GoTime,
) -> Vec<J> {
    let tur = json!({"stdout": "", "stderr": "", "interrupted": false, "isImage": false,
        "bashEditDiff": {"files": files, "changedFiles": changed}});
    vec![
        w.tool_use(
            id,
            "Bash",
            json!({"command": "build", "description": "run"}),
            at,
        ),
        w.result(id, tur, at),
    ]
}

#[test]
fn heuristic_noise_dropped() {
    let r = Rig::new(|_| {});
    let (_d, dir) = noisy_repo();
    let (_o, outside) = tempdir();
    let w = r.session(SID_A, &dir);
    let t = now();
    let gen_path = join(&[&dir, "bin", "gen.txt"]);
    w.write(&flat(vec![
        vec![w.prompt("p1", "build", &t)],
        w.write_tool("w1", &gen_path, "generated\n", None, &add(&t, sec(1))),
        bash_diff(
            &w,
            "b1",
            &[
                join(&[&dir, "foo.go"]),
                join(&[&dir, "keep.log"]),
                join(&[&dir, "bin", "app.txt"]),
                join(&[&dir, "debug.log"]),
                gen_path.clone(),
                join(&[&dir, "out", "app.DLL"]),
                join(&[&dir, ".git", "index.txt"]),
                join(&[&outside, "notes.txt"]),
            ],
            &add(&t, sec(2)),
        ),
        vec![w.reply("done", &add(&t, sec(3))), w.done(&add(&t, sec(4)))],
    ]));
    r.tail();
    let mut by: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for e in r.rows(KIND_EDIT) {
        by.entry(e["fidelity"].as_str().unwrap().into())
            .or_default()
            .push(e["path"].as_str().unwrap().into());
    }
    for v in by.values_mut() {
        v.sort();
    }
    assert_eq!(by["heuristic"], ["foo.go", "keep.log"]);
    assert_eq!(by["exact"], ["bin/gen.txt"]);
    assert_eq!(
        r.c.status().dropped,
        Dropped {
            artifact: 2,
            outside_repo: 1,
            ignored: 3,
            ..Dropped::default()
        }
    );
    let turns = r.rows(KIND_TURN);
    assert_eq!(turns.len(), 1);
    let mut paths: Vec<String> = turns[0]["editPaths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap().to_string())
        .collect();
    paths.sort();
    assert_eq!(paths, ["bin/gen.txt", "foo.go", "keep.log"]);
}

#[test]
fn heuristic_cap_per_call() {
    let r = Rig::new(|_| {});
    let (_d, dir) = noisy_repo();
    let w = r.session(SID_A, &dir);
    let t = now();
    let many: Vec<String> = (0..60)
        .map(|i| join(&[&dir, &format!("f{i:02}.go")]))
        .collect();
    w.write(&flat(vec![
        vec![w.prompt("p1", "gen", &t)],
        bash_diff(&w, "b1", &many, &t),
        vec![w.done(&t)],
    ]));
    r.tail();
    assert_eq!(r.count(KIND_EDIT), 50);
    assert_eq!(r.c.status().dropped.over_cap, 10);
}

// --- watch_test.go: Run on file-change events ---

fn event_rig(modify: impl FnOnce(&mut capture::Options)) -> Option<Rig> {
    let (_t, probe) = tempdir();
    if let Err(sys::fswatch::Error::Unsupported) =
        sys::fswatch::watch(&Cancel::new(), &probe, false, Duration::ZERO)
    {
        return None;
    }
    let r = Rig::new(|o| {
        o.scan_interval = Duration::from_secs(3600);
        o.repo_interval = Duration::from_secs(3600);
        o.safety_interval = Duration::from_secs(3600);
        modify(o);
    });
    std::fs::create_dir_all(join(&[&r.claude, "projects"])).unwrap();
    Some(r)
}

fn wait_for(what: &str, cond: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !cond() {
        assert!(
            std::time::Instant::now() < deadline,
            "not seen in time: {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

struct Running {
    cancel: Cancel,
    h: Option<std::thread::JoinHandle<()>>,
}

impl Running {
    fn start(r: &Rig) -> Running {
        let cancel = Cancel::new();
        let (c, cc) = (r.c.clone(), cancel.clone());
        let h = std::thread::spawn(move || c.run(&cc));
        wait_for("first pass", || !r.c.status().backfilling);
        Running { cancel, h: Some(h) }
    }
    fn stop(&mut self) {
        self.cancel.cancel();
        if let Some(h) = self.h.take() {
            h.join().unwrap();
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop();
    }
}

#[test]
fn event_tail_new_and_appended() {
    let Some(r) = event_rig(|_| {}) else { return };
    let (_d, cwd) = tempdir();
    let mut run = Running::start(&r);
    let w = r.session(SID_A, &cwd);
    let t = now();
    w.write(&[w.prompt("p1", "one", &t), w.done(&t)]);
    wait_for("first turn", || r.count(KIND_TURN) == 1);
    w.write(&[
        w.prompt("p2", "two", &add(&t, sec(1))),
        w.done(&add(&t, sec(1))),
    ]);
    wait_for("second turn", || r.count(KIND_TURN) == 2);
    let sa = r.subagent(SID_A, &cwd, "a1");
    sa.write(&[sa.prompt("p3", "sub", &t), sa.done(&t)]);
    wait_for("subagent turn", || r.count(KIND_TURN) == 3);
    run.stop();
}

#[test]
fn event_quiet_close_timer() {
    let Some(r) = event_rig(|o| o.quiet_after = Duration::from_millis(300)) else {
        return;
    };
    let _run = Running::start(&r);
    let (_d, cwd) = tempdir();
    let w = r.session(SID_A, &cwd);
    let t = now();
    w.write(&[w.prompt("p1", "open", &t)]);
    wait_for("quiet close", || {
        r.rows(KIND_TURN).iter().any(|t| t["endReason"] == "quiet")
    });
}

#[test]
fn event_reflog() {
    let Some(r) = event_rig(|_| {}) else { return };
    let (_d, dir) = new_repo("false");
    r.roster.set(vec![live_card(SID_A, &dir, "idle")]);
    let _run = Running::start(&r);
    wait_for("root watched", || r.c.status().repos == 1);
    write_file(&join(&[&dir, "x.txt"]), "x\n");
    git(&dir, &["add", "x.txt"]);
    git(&dir, &["commit", "-q", "-m", "x"]);
    let sha = head(&dir);
    wait_for("commit by event", || r.c.commit(&sha).is_some());
}

/// Only hot roots hold a logs handle, and it must not block `git worktree
/// remove` on Windows.
#[test]
fn event_worktree_removable() {
    let Some(r) = event_rig(|_| {}) else { return };
    let (_d, main) = new_repo("false");
    let (_t, base) = tempdir();
    let wt = join(&[&base, "wt"]);
    git(&main, &["worktree", "add", "-q", &wt]);
    r.roster.set(vec![live_card(SID_A, &wt, "idle")]);
    let _run = Running::start(&r);
    wait_for("worktree watched", || r.c.status().repos == 1);
    write_file(&join(&[&wt, "w.txt"]), "w\n");
    git(&wt, &["add", "w.txt"]);
    git(&wt, &["commit", "-q", "-m", "w"]);
    let sha = head(&wt);
    wait_for("worktree commit", || r.c.commit(&sha).is_some());
    git(&main, &["worktree", "remove", "--force", &wt]);
    assert!(!std::path::Path::new(&join(&[&main, ".git", "worktrees", "wt"])).exists());
}

// --- ports of I:internal/capture/review_test.go ---

/// TestAmendKeepsExactLink: an amended or cherry-picked commit of the same
/// tree links exactly to the same edit.
#[test]
fn amend_keeps_exact_link() {
    type Redo = fn(&str) -> String;
    let redos: [(&str, Redo); 2] = [
        ("amend", |dir| {
            git(dir, &["commit", "--amend", "-q", "-m", "reworded"]);
            head(dir)
        }),
        ("cherry-pick", |dir| {
            let sha = head(dir);
            git(dir, &["checkout", "-q", "-b", "other", "HEAD~1"]);
            write_file(&join(&[dir, "bar.go"]), "package bar\n");
            git(dir, &["add", "bar.go"]);
            git(dir, &["commit", "-q", "-m", "bar"]);
            git(dir, &["cherry-pick", sha.as_str()]);
            head(dir)
        }),
    ];
    for (name, redo) in redos {
        let r = Rig::new(|_| {});
        let (_d, dir) = new_repo("false");
        let w = r.session(SID_A, &dir);
        let first = edit_flow(&r, &w, &dir, "package foo\n\nfunc A() {}\n", "Bash");
        let cv = r.c.commit(&first).expect("first commit seen");
        assert!(
            cv.link
                .as_ref()
                .is_some_and(|l| l.files()[0].link == LINK_EXACT),
            "{name}: first commit = {cv:?}"
        );
        let again = redo(&dir);
        assert_ne!(again, first, "{name}: redo made no new commit");
        r.repos();
        let cv = r.c.commit(&again).expect("rewritten commit seen");
        let l = cv.link.expect("rewritten commit linked");
        let f = &l.files()[0];
        assert!(
            f.path == "foo.go"
                && f.link == LINK_EXACT
                && f.edits.len() == 1
                && l.sessions.len() == 1,
            "{name}: rewritten link = {l:?}"
        );
    }
}

/// Fails the first `left` git runs whose args hold `sub`, then runs git.
struct FlakyGit {
    sub: &'static str,
    left: std::sync::Mutex<usize>,
}

impl capture::gitlink::Git for FlakyGit {
    fn run(
        &self,
        cancel: &Cancel,
        timeout: Duration,
        dir: &str,
        args: &[&str],
    ) -> Result<String, capture::gitlink::GitError> {
        {
            let mut left = self.left.lock().unwrap();
            if *left > 0 && args.contains(&self.sub) {
                *left -= 1;
                return Err(capture::gitlink::GitError::Other("transient".into()));
            }
        }
        capture::gitlink::Exec.run(cancel, timeout, dir, args)
    }
}

/// TestTransientGitFailureRetries: a commit whose rev-list or show fails
/// twice is still seen on a later pass.
#[test]
fn transient_git_failure_retries() {
    for sub in ["rev-list", "show"] {
        let g = Arc::new(FlakyGit {
            sub,
            left: std::sync::Mutex::new(0),
        });
        let gg = g.clone();
        let r = Rig::new(move |o| o.git = gg);
        let (_d, dir) = new_repo("false");
        r.roster.set(vec![live_card(SID_A, &dir, "busy")]);
        r.repos();
        write_file(&join(&[&dir, "foo.go"]), "package foo\n");
        git(&dir, &["add", "foo.go"]);
        git(&dir, &["commit", "-q", "-m", "add foo"]);
        let sha = head(&dir);
        *g.left.lock().unwrap() = 2;
        r.repos();
        r.repos();
        r.repos();
        assert!(
            r.c.commit(&sha).is_some(),
            "commit lost after a transient {sub} failure"
        );
        assert_eq!(*g.left.lock().unwrap(), 0, "{sub}: the failures never ran");
    }
}

fn flaky(sub: &'static str, left: usize) -> Arc<FlakyGit> {
    Arc::new(FlakyGit {
        sub,
        left: std::sync::Mutex::new(left),
    })
}

/// A later commit of a different foo.go content must not pick up the
/// earlier, already committed edit as partial.
#[test]
fn linked_edits_never_make_partial() {
    let r = Rig::new(|_| {});
    let (_d, dir) = new_repo("false");
    let w = r.session(SID_A, &dir);
    edit_flow(&r, &w, &dir, "package foo\n\nfunc A() {}\n", "Bash");
    write_file(&join(&[&dir, "foo.go"]), "package foo\n\nfunc B() {}\n");
    git(&dir, &["commit", "-q", "-am", "by hand"]);
    let sha = head(&dir);
    r.repos();
    let cv = r.c.commit(&sha).expect("hand commit seen");
    let l = cv.link.expect("hand commit linked");
    assert_eq!(l.files()[0].link, LINK_UNATTRIBUTED, "hand commit = {l:?}");
}

#[test]
fn commit_before_first_repo_pass() {
    let r = Rig::new(|_| {});
    let (_d, dir) = new_repo("false");
    let w = r.session(SID_A, &dir);
    let t = now_early_in_second(); // the edit is stamped 1.1 s after t
    w.write(&flat(vec![
        vec![w.prompt("p1", "add foo", &t)],
        w.write_tool(
            "w1",
            &join(&[&dir, "foo.go"]),
            "package foo\n",
            None,
            &add(&t, sec(1)),
        ),
    ]));
    r.tail(); // the root is first seen here; no repo pass yet
    git(&dir, &["add", "foo.go"]);
    git(&dir, &["commit", "-q", "-m", "add foo"]);
    let sha = head(&dir);
    r.repos();
    r.repos();
    let cv = r.c.commit(&sha).expect("commit seen");
    assert!(
        cv.link
            .as_ref()
            .is_some_and(|l| l.files()[0].link == LINK_EXACT),
        "commit after first sight = {cv:?}"
    );
}

const EXACT_SECRET: &str = "irori-lockfile-token-0123456789abcdef";

fn exact_scrub(o: &mut capture::Options) {
    o.scrub = Arc::new(ExactScrub(vec![EXACT_SECRET.into()]));
}

#[test]
fn straddling_secret_not_stored() {
    let r = Rig::new(exact_scrub);
    let (_d, dir) = new_repo("false");
    let w = r.session(SID_A, &dir);
    let t = now();
    let prompt = format!("{} {EXACT_SECRET} tail", "a".repeat(1985));
    let cmd = format!("curl -s {} -H {EXACT_SECRET}", "x".repeat(140));
    w.write(&flat(vec![
        vec![w.prompt("p1", &prompt, &t)],
        w.shell("b1", "Bash", &cmd, "", None, &t),
        vec![w.reply("ok", &t), w.done(&t)],
    ]));
    r.tail();
    let tr = &r.rows(KIND_TURN)[0];
    let p = tr["prompt"].as_str().unwrap();
    let prefix = &EXACT_SECRET[..13];
    assert!(
        !p.contains(prefix),
        "prompt holds a token prefix: {:?}",
        &p[p.len() - 40..]
    );
    let tools = tr["tools"].as_array().unwrap();
    assert!(
        tools.len() == 1 && !tools[0]["target"].as_str().unwrap().contains(prefix),
        "target holds a token prefix: {tools:?}"
    );
}

#[test]
fn commit_subject_scrubbed() {
    let r = Rig::new(exact_scrub);
    let (_d, dir) = new_repo("false");
    r.roster.set(vec![live_card(SID_A, &dir, "busy")]);
    r.repos();
    write_file(&join(&[&dir, "foo.go"]), "package foo\n");
    git(&dir, &["add", "foo.go"]);
    git(
        &dir,
        &["commit", "-q", "-m", &format!("use {EXACT_SECRET}")],
    );
    r.repos();
    let rows = r.rows(KIND_COMMIT_SEEN);
    assert!(!rows.is_empty(), "no commit.seen");
    for raw in rows {
        assert!(
            !raw.to_string().contains(EXACT_SECRET),
            "commit.seen holds a secret: {raw}"
        );
    }
}

#[test]
fn stuck_reflog_line_skipped() {
    let g = flaky("show", 0);
    let gg = g.clone();
    let r = Rig::new(move |o| o.git = gg);
    let (_d, dir) = new_repo("false");
    r.roster.set(vec![live_card(SID_A, &dir, "busy")]);
    r.repos();
    git(&dir, &["commit", "-q", "--allow-empty", "-m", "bad"]);
    let bad = head(&dir);
    *g.left.lock().unwrap() = 1 << 20;
    for _ in 0..capture::MAX_REFLOG_TRIES {
        r.repos();
    }
    *g.left.lock().unwrap() = 0;
    git(&dir, &["commit", "-q", "--allow-empty", "-m", "good"]);
    let good = head(&dir);
    r.repos();
    assert!(r.c.commit(&bad).is_none(), "stuck commit imported");
    assert!(
        r.c.commit(&good).is_some(),
        "a stuck line blocked the commits after it"
    );
}

#[test]
fn late_commit_commands_link_once() {
    let r = Rig::new(|_| {});
    let (_d, dir) = new_repo("false");
    let w = r.session(SID_A, &dir);
    let foo = join(&[&dir, "foo.go"]);
    let start = sub(&now(), sec(5));
    let cmd = "git add foo.go && git commit -m 'add foo' && gh pr checks x --watch";
    let input = json!({"command": cmd, "description": "commit and watch"});
    w.write(&flat(vec![
        vec![w.prompt("p1", "add foo", &start)],
        w.write_tool("w1", &foo, "package foo\n", None, &add(&start, sec(1))),
        vec![w.tool_use("b1", "Bash", input.clone(), &add(&start, sec(2)))],
        vec![w.tool_use("b2", "Bash", input, &add(&start, sec(2)))],
    ]));
    git(&dir, &["add", "foo.go"]);
    git(&dir, &["commit", "-q", "-m", "add foo"]);
    let sha = head(&dir);
    r.tail();
    r.repos();
    let links = || -> Vec<J> {
        r.rows(KIND_COMMIT_LINK)
            .into_iter()
            .filter(|l| l["sha"] == json!(sha))
            .collect()
    };
    let first = links();
    assert!(
        first.len() == 1 && first[0].get("committer").is_none(),
        "before the results: {first:?}"
    );
    let done = add(&now(), sec(180));
    let res = json!({"stdout": "ok\n", "stderr": "", "interrupted": false, "isImage": false});
    w.write(&[
        w.result("b1", res.clone(), &done),
        w.result("b2", res, &done),
    ]);
    r.tail();
    let got = links();
    assert!(
        got.len() == 2 && got[1].get("committer").is_some(),
        "after both results: {got:?}"
    );

    // Race two calls on a commit that has lost its committer again.
    let before = capture::decode_commit_link(first[0].to_string().as_bytes()).unwrap();
    r.c.set_commit_link(&before.repo_key, &sha, Some(before.clone()));
    let cmds = r.c.commit_cmds();
    assert!(cmds.len() >= 2, "commands = {}", cmds.len());
    std::thread::scope(|s| {
        for cm in &cmds[..2] {
            let c = r.c.clone();
            s.spawn(move || c.relink_committer(cm).unwrap());
        }
    });
    assert_eq!(links().len(), 3, "racing calls");
    // Once it has a committer, another late command adds nothing.
    r.c.relink_committer(&cmds[0]).unwrap();
    assert_eq!(links().len(), 3, "repeat");
}

fn edits_by(r: &Rig) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut by: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for e in r.rows(KIND_EDIT) {
        by.entry(e["fidelity"].as_str().unwrap().into())
            .or_default()
            .push(e["path"].as_str().unwrap().into());
    }
    for v in by.values_mut() {
        v.sort();
    }
    by
}

fn heuristic(r: &Rig) -> Vec<String> {
    edits_by(r).remove("heuristic").unwrap_or_default()
}

#[test]
fn check_ignore_fails_open() {
    let g = flaky("check-ignore", 1);
    let r = Rig::new(move |o| o.git = g);
    let (_d, dir) = noisy_repo();
    let w = r.session(SID_A, &dir);
    let t = now();
    let ign = join(&[&dir, "bin", "app.txt"]);
    w.write(&flat(vec![
        vec![w.prompt("p1", "build", &t)],
        bash_diff(&w, "b1", std::slice::from_ref(&ign), &add(&t, sec(1))),
        bash_diff(&w, "b2", std::slice::from_ref(&ign), &add(&t, sec(2))),
        vec![w.done(&add(&t, sec(3)))],
    ]));
    r.tail();
    assert_eq!(heuristic(&r), ["bin/app.txt"], "want the first call's only");
    assert_eq!(
        r.c.status().dropped,
        Dropped {
            ignored: 1,
            ..Dropped::default()
        }
    );
    // Kept by one call, so the turn still lists it.
    let paths = r.rows(KIND_TURN)[0]["editPaths"].clone();
    assert_eq!(
        paths.as_array().map(Vec::len),
        Some(1),
        "turn edit paths {paths}"
    );
}

#[test]
fn heuristic_deleted_worktree_kept() {
    let r = Rig::new(|_| {});
    let (_d, dir) = new_repo("false");
    write_file(&join(&[&dir, ".git", "info", "exclude"]), ".claude/\n");
    std::fs::create_dir_all(join(&[&dir, ".claude", "worktrees"])).unwrap();
    let gone = join(&[&dir, ".claude", "worktrees", "wf-gone"]); // never created
    let w = r.session(SID_A, &gone);
    let t = now();
    w.write(&flat(vec![
        vec![w.prompt("p1", "fmt", &t)],
        bash_diff(
            &w,
            "b1",
            &[join(&[&gone, "internal", "main.go"])],
            &add(&t, sec(1)),
        ),
        vec![w.done(&add(&t, sec(2)))],
    ]));
    r.tail();
    let h = heuristic(&r);
    assert!(
        h.len() == 1 && capture::gopath::to_slash(&h[0]).ends_with("wf-gone/internal/main.go"),
        "heuristic edits = {h:?}, want the deleted worktree's main.go"
    );
    assert_eq!(r.c.status().dropped, Dropped::default());
}

/// A failed rev-parse inside a repo keeps the edit (without a root) rather
/// than calling it outside every repo.
#[test]
fn heuristic_rev_parse_fails_open() {
    let g = flaky("rev-parse", 1 << 20);
    let r = Rig::new(move |o| o.git = g);
    let (_d, dir) = new_repo("false");
    let (_o, outside) = tempdir();
    let w = r.session(SID_A, &dir);
    let t = now();
    w.write(&flat(vec![
        vec![w.prompt("p1", "fmt", &t)],
        bash_diff(
            &w,
            "b1",
            &[
                join(&[&dir, "main.go"]),
                join(&[&outside, "notes.txt"]), // no .git above: still outside
            ],
            &add(&t, sec(1)),
        ),
        vec![w.done(&add(&t, sec(2)))],
    ]));
    r.tail();
    assert_eq!(heuristic(&r), [join(&[&dir, "main.go"])]);
    assert_eq!(
        r.c.status().dropped,
        Dropped {
            outside_repo: 1,
            ..Dropped::default()
        }
    );
}

/// A shell call that touched a file without changing a line (+0-0) is
/// noise.
#[test]
fn heuristic_unchanged_dropped() {
    let r = Rig::new(|_| {});
    let (_d, dir) = noisy_repo();
    let w = r.session(SID_A, &dir);
    let t = now();
    let bar = join(&[&dir, "bar.go"]);
    w.write(&flat(vec![
        vec![w.prompt("p1", "touch", &t)],
        bash_diff_files(
            &w,
            "b1",
            &[join(&[&dir, "foo.go"]), bar.clone()],
            vec![json!({"filePath": bar, "hunks": []})],
            &add(&t, sec(1)),
        ),
        bash_diff(&w, "b2", &[join(&[&dir, "real.go"])], &add(&t, sec(2))),
        vec![w.done(&add(&t, sec(3)))],
    ]));
    r.tail();
    assert_eq!(heuristic(&r), ["real.go"]);
    assert_eq!(
        r.c.status().dropped,
        Dropped {
            unchanged: 2,
            ..Dropped::default()
        }
    );
    assert_eq!(r.rows(KIND_TURN)[0]["editPaths"], json!(["real.go"]));
}

#[test]
fn event_quiet_busy_recheck() {
    let Some(r) = event_rig(|o| o.quiet_after = Duration::from_millis(100)) else {
        return;
    };
    let (_d, cwd) = tempdir();
    r.roster.set(vec![live_card(SID_A, &cwd, "busy")]);
    let _run = Running::start(&r);
    let w = r.session(SID_A, &cwd);
    let t = now();
    w.write(&[w.prompt("p1", "hi", &t), w.reply("hello", &t)]);
    wait_for("open turn", || r.c.open_turns() == 1);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        r.count(KIND_TURN),
        0,
        "a busy session's turn closed as quiet"
    );
}

/// Events that pile up while the loop is busy (a long pass) are either
/// delivered or reported lost, so a transcript written in that window is
/// tailed at once, not at the safety pass.
#[test]
fn event_busy_loop_loses_nothing() {
    let Some(r) = event_rig(|_| {}) else { return };
    let (_d, cwd) = tempdir();
    let _run = Running::start(&r);
    let hold = r.roster.hold();
    let a = r.session(SID_A, &cwd);
    let t = now();
    a.write(&[a.prompt("p1", "one", &t), a.done(&t)]); // the loop stalls tailing this
    std::thread::sleep(Duration::from_millis(300));

    // Many files changing while the loop is stuck: far more events than
    // any buffer between the watcher and the loop holds.
    let flood = join(&[&r.claude, "projects", "flood"]);
    std::fs::create_dir_all(&flood).unwrap();
    for round in 0..15 {
        for i in 0..100 {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(join(&[&flood, &format!("f{i:03}.txt")]))
                .unwrap();
            writeln!(f, "{round}").unwrap();
        }
        std::thread::sleep(Duration::from_millis(70));
    }
    let b = r.session(SID_B, &cwd);
    b.write(&[b.prompt("p2", "two", &t), b.done(&t)]);
    std::thread::sleep(Duration::from_millis(300));
    hold.release();
    wait_for("both turns", || r.count(KIND_TURN) == 2);
}

/// Only roots a live session works in keep a logs handle open: once the
/// session is gone the repo folder, and the folder holding it, can be
/// renamed again.
#[test]
fn event_gone_root_released() {
    let Some(r) = event_rig(|o| o.safety_interval = Duration::from_millis(200)) else {
        return;
    };
    let (_t, base) = tempdir();
    let parent = join(&[&base, "parent"]);
    let dir = join(&[&parent, "repo"]);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "core.autocrlf", "false"]);
    write_file(&join(&[&dir, "a.txt"]), "a\n");
    git(&dir, &["add", "a.txt"]);
    git(&dir, &["commit", "-q", "-m", "a"]);
    r.roster.set(vec![live_card(SID_A, &dir, "idle")]);
    let _run = Running::start(&r);
    wait_for("root watched", || r.c.status().repos == 1);
    write_file(&join(&[&dir, "b.txt"]), "b\n");
    git(&dir, &["add", "b.txt"]);
    git(&dir, &["commit", "-q", "-m", "b"]);
    let sha = head(&dir);
    wait_for("commit", || r.c.commit(&sha).is_some());

    r.roster.set(vec![]); // the session is gone
    let moved = format!("{parent}-moved");
    wait_for("parent folder renamable", || {
        std::fs::rename(&parent, &moved).is_ok()
    });
    std::fs::rename(join(&[&moved, "repo"]), join(&[&moved, "repo2"])).unwrap();
}
