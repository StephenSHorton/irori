//! Ports of I:internal/capture/search_test.go (Search, Show, Overview and
//! the outside sources). Every row is synthetic and goes straight into the
//! fold through `emit_row`, the path the tailer and the repo pass use.
//! TestMergeOwner and TestMatchExpr are unit tests in src/find.rs and
//! src/index.rs; TestFTS5Available is in src/index.rs.

// Table-driven cases, as in Go.
#![allow(clippy::type_complexity)]

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use capture::gitlink::{Commit, FileChange, FileLink, LINK_EXACT};
use capture::transcript::{EXACT, Turn};
use capture::{AgentCard, Blobs};
use capture::{
    CommitLink, CommitRow, Committer, EditRow, HIDDEN_MESSAGE, KIND_COMMIT_LINK, KIND_COMMIT_SEEN,
    KIND_DECISION, KIND_EDIT, KIND_PR, KIND_TURN, Line, OVERVIEW_AGENTS, OVERVIEW_LINE_MAX,
    OVERVIEW_LINES, OverviewInput, PULL_PAGE, TurnRow, clip_runes, edit_row_id, hide_messages,
    turn_row_id,
};
use common::*;
use coord::decision::{Decision, Party};
use gocompat::time::GoTime;
use search::{Doc, Error, Hit, Kind, Query, ShowOptions, Shown, Source};

const REPO_X: &str = "remote:github.com/acme/widgets";
const REPO_Y: &str = "remote:github.com/acme/other";

fn key_a() -> String {
    format!("{DEVICE}/sess-a")
}
fn key_b() -> String {
    format!("{DEVICE}/sess-b")
}

/// Recent: the ledger only shows entries younger than its keep window.
fn t0() -> GoTime {
    static T0: std::sync::OnceLock<GoTime> = std::sync::OnceLock::new();
    *T0.get_or_init(|| {
        let now = capture::gotime::truncate_second(&capture::gotime::now().utc());
        capture::gotime::sub(&now, Duration::from_secs(2 * 3600))
    })
}

fn at(min: i64) -> GoTime {
    at_secs(&t0(), min * 60)
}

fn emit<P: serde::Serialize>(r: &Rig, kind: &str, p: &P) {
    r.c.emit_row("irori", kind, p).unwrap();
}

fn turn(
    r: &Rig,
    key: &str,
    rk: &str,
    turn_id: &str,
    prompt: &str,
    reply: &str,
    started: GoTime,
) -> TurnRow {
    let row = TurnRow {
        id: turn_row_id(key, "", turn_id),
        key: key.into(),
        repo_key: rk.into(),
        turn_id: turn_id.into(),
        turn: Turn {
            id: turn_id.into(),
            prompt: prompt.into(),
            reply: reply.into(),
            started_at: started,
            ended_at: at_secs(&started, 60),
            origin: "human".into(),
            ..Turn::default()
        },
        ..TurnRow::default()
    };
    emit(r, KIND_TURN, &row);
    row
}

fn edit(
    r: &Rig,
    key: &str,
    rk: &str,
    turn_id: &str,
    path: &str,
    when: GoTime,
    patch: &str,
) -> EditRow {
    let tu = format!("tu-{path}");
    let mut row = EditRow {
        id: edit_row_id(key, &tu, path),
        key: key.into(),
        turn_id: turn_id.into(),
        tool_use_id: tu,
        tool: "Edit".into(),
        repo_key: rk.into(),
        path: path.into(),
        adds: 3,
        dels: 1,
        fidelity: EXACT.into(),
        at: when,
        ..EditRow::default()
    };
    if !patch.is_empty() {
        row.patch = r.blobs.put(patch.as_bytes()).unwrap();
    }
    emit(r, KIND_EDIT, &row);
    row
}

fn commit_row(r: &Rig, rk: &str, sha: &str, subject: &str, when: GoTime, paths: &[&str]) {
    let files = paths
        .iter()
        .map(|p| FileChange {
            path: (*p).into(),
            status: "M".into(),
            ..FileChange::default()
        })
        .collect();
    emit(
        r,
        KIND_COMMIT_SEEN,
        &CommitRow {
            repo_key: rk.into(),
            op: "commit".into(),
            branch: "main".into(),
            commit: Commit {
                sha: sha.into(),
                at: when,
                author: "Tester".into(),
                subject: subject.into(),
                files: Some(files),
                ..Commit::default()
            },
            ..CommitRow::default()
        },
    );
}

fn link(r: &Rig, rk: &str, sha: &str, committer: Option<Committer>, e: &EditRow) {
    emit(
        r,
        KIND_COMMIT_LINK,
        &CommitLink {
            repo_key: rk.into(),
            sha: sha.into(),
            committer,
            files: Some(vec![FileLink {
                path: e.path.clone(),
                link: LINK_EXACT.into(),
                edits: vec![e.id.clone()],
            }]),
            sessions: vec![e.key.clone()],
            turns: vec![e.turn_id.clone()],
            at: e.at,
        },
    );
}

fn pr(r: &Rig, repo: &str, n: i64, session: &str, role: ledger::Role, when: GoTime) {
    emit(
        r,
        KIND_PR,
        &ledger::Entry {
            key: ledger::key(repo, n),
            repo: repo.into(),
            number: n,
            session: session.into(),
            role,
            source: "gh".into(),
            at: when,
            url: format!("https://github.com/{repo}/pull/{n}"),
        },
    );
}

fn decision(r: &Rig, d: Decision) {
    emit(r, KIND_DECISION, &d);
}

fn dec(
    id: &str,
    by: Party,
    statement: &str,
    why: &str,
    supersedes: &[&str],
    when: GoTime,
) -> Decision {
    Decision {
        id: id.into(),
        by,
        statement: statement.into(),
        why: why.into(),
        supersedes: supersedes.iter().map(|s| s.to_string()).collect(),
        trust: "untrusted".into(),
        at: when,
        ..Decision::default()
    }
}

fn party(key: &str, name: &str) -> Party {
    Party {
        key: key.into(),
        name: name.into(),
        ..Party::default()
    }
}

fn search(r: &Rig, q: Query) -> Vec<Hit> {
    r.c.search(&q).unwrap()
}

fn refs(hits: &[Hit]) -> Vec<String> {
    hits.iter().map(|h| h.r#ref.clone()).collect()
}

fn text(t: &str) -> Query {
    Query {
        text: t.into(),
        ..Query::default()
    }
}

fn card(sid: &str, name: &str, status: &str, rk: &str) -> AgentCard {
    AgentCard {
        key: format!("{DEVICE}/{sid}"),
        session_id: sid.into(),
        name: name.into(),
        status: status.into(),
        repo_key: rk.into(),
        ..AgentCard::default()
    }
}

const SHA: &str = "abcdef1234567890abcdef1234567890abcdef12";

#[test]
fn search_capture_kinds() {
    let mut r = Rig::new(|_| {});
    let (ka, kb) = (key_a(), key_b());
    let tr = turn(
        &r,
        &ka,
        REPO_X,
        "p1",
        "Fix the lease refusal\nsecond line",
        "Renamed the gizmo handler.",
        at(0),
    );
    let e = edit(&r, &ka, REPO_X, "p1", "internal/lease/lease.go", at(1), "");
    commit_row(
        &r,
        REPO_X,
        SHA,
        "Teach leases to refuse politely",
        at(2),
        &["internal/lease/lease.go", "docs/zebra.md"],
    );
    pr(&r, "Acme/Widgets", 12, &kb, ledger::Role::REVIEWER, at(3));
    pr(&r, "Acme/Widgets", 12, &ka, ledger::Role::AUTHOR, at(4));
    decision(
        &r,
        dec(
            "d-1",
            party(&ka, ""),
            "Prefer quokka caches",
            "they are friendly",
            &[],
            at(5),
        ),
    );

    let turn_ref = format!("turn:{}", tr.id);
    let edit_ref = format!("edit:{}", e.id);
    let commit_ref = format!("commit:{SHA}");
    let cases: Vec<(&str, Query, &str, &str, &str)> = vec![
        (
            "turn by prompt",
            text("refusal"),
            &turn_ref,
            "Fix the lease refusal",
            &ka,
        ),
        (
            "turn by reply",
            text("gizmo"),
            &turn_ref,
            "Fix the lease refusal",
            &ka,
        ),
        (
            "edit by path",
            Query {
                kinds: vec![Kind::Edit],
                ..text("lease.go")
            },
            &edit_ref,
            "Edit internal/lease/lease.go +3 -1",
            &ka,
        ),
        (
            "commit by subject",
            text("politely"),
            &commit_ref,
            "Teach leases to refuse politely",
            "",
        ),
        (
            "commit by file path",
            text("zebra"),
            &commit_ref,
            "Teach leases to refuse politely",
            "",
        ),
        (
            "pr best role",
            Query {
                kinds: vec![Kind::Pr],
                ..Query::default()
            },
            "pr:acme/widgets#12",
            "PR Acme/Widgets#12 (author)",
            &ka,
        ),
        (
            "decision by why",
            text("friendly"),
            "decision:d-1",
            "Prefer quokka caches",
            &ka,
        ),
        (
            "prefix on last word",
            text("quok"),
            "decision:d-1",
            "Prefer quokka caches",
            &ka,
        ),
        (
            "porter stemming",
            Query {
                kinds: vec![Kind::Commit],
                ..text("refusing")
            },
            &commit_ref,
            "Teach leases to refuse politely",
            "",
        ),
    ];
    for (name, q, want, title, agent) in cases {
        let hits = search(&r, q);
        assert!(
            !hits.is_empty() && hits[0].r#ref == want,
            "{name}: hits {:?}, want {want} first",
            refs(&hits)
        );
        let h = &hits[0];
        assert_eq!(
            (h.title.as_str(), h.agent.as_str()),
            (title, agent),
            "{name}: {h:?}"
        );
        let (kind, _) = search::parse_ref(want).unwrap();
        assert!(h.kind == kind && !h.at.is_zero(), "{name}: {h:?}");
    }

    // The link fills the commit's agent.
    link(
        &r,
        REPO_X,
        SHA,
        Some(Committer {
            key: kb.clone(),
            turn_id: "x".into(),
        }),
        &e,
    );
    let hits = search(&r, text("politely"));
    assert!(
        hits.len() == 1 && hits[0].agent == kb,
        "after link: {hits:?}"
    );
    // Replay rebuilds the same index.
    r.restart();
    let hits = search(&r, text("politely"));
    assert!(
        hits.len() == 1 && hits[0].agent == kb,
        "after restart: {hits:?}"
    );
    assert_eq!(r.c.index_size(), 5, "docs after replay");
}

#[test]
fn search_ranking_and_filters() {
    let r = Rig::new(|_| {});
    let (ka, kb) = (key_a(), key_b());
    let in_body = turn(
        &r,
        &ka,
        REPO_X,
        "p1",
        "Unrelated start",
        "then the walrus showed up",
        at(10),
    );
    let in_title = turn(
        &r,
        &kb,
        REPO_Y,
        "p2",
        "Walrus migration plan",
        "nothing else",
        at(0),
    );
    let newer = turn(
        &r,
        &kb,
        REPO_X,
        "p3",
        "Walrus migration plan",
        "nothing else",
        at(5),
    );
    let e = edit(&r, &ka, REPO_X, "p1", "walrus/tusk.go", at(11), "");
    let t = |x: &TurnRow| format!("turn:{}", x.id);
    let er = format!("edit:{}", e.id);
    let cases: Vec<(&str, Query, Vec<String>)> = vec![
        (
            "title beats body, then newest",
            Query {
                kinds: vec![Kind::Turn],
                ..text("walrus")
            },
            vec![t(&newer), t(&in_title), t(&in_body)],
        ),
        (
            "kinds",
            Query {
                kinds: vec![Kind::Edit],
                ..text("walrus")
            },
            vec![er.clone()],
        ),
        (
            "repo",
            Query {
                repo_key: REPO_Y.into(),
                ..text("walrus")
            },
            vec![t(&in_title)],
        ),
        (
            "agent",
            Query {
                agent: ka.clone(),
                kinds: vec![Kind::Turn],
                ..text("walrus")
            },
            vec![t(&in_body)],
        ),
        (
            "before",
            Query {
                before: at(5),
                ..text("walrus")
            },
            vec![t(&in_title)],
        ),
        (
            "empty text lists newest",
            Query::default(),
            vec![er.clone(), t(&in_body), t(&newer), t(&in_title)],
        ),
        (
            "empty text with limit",
            Query {
                limit: 2,
                ..Query::default()
            },
            vec![er.clone(), t(&in_body)],
        ),
        (
            "all words must match",
            text("walrus nothing"),
            vec![t(&newer), t(&in_title)],
        ),
        ("no match", text("narwhal"), vec![]),
        ("punctuation only", text("* -"), vec![]),
        (
            "quote in text",
            Query {
                repo_key: REPO_Y.into(),
                ..text("\"walrus\"")
            },
            vec![t(&in_title)],
        ),
    ];
    for (name, q, want) in cases {
        assert_eq!(refs(&search(&r, q)), want, "{name}");
    }
}

#[test]
fn search_limits() {
    let r = Rig::new(|_| {});
    let ka = key_a();
    for i in 0..60 {
        turn(
            &r,
            &ka,
            REPO_X,
            &format!("p{i:02}"),
            &format!("otter number {i}"),
            "",
            at(i),
        );
    }
    let max = search::MAX_LIMIT as usize;
    let def = search::DEFAULT_LIMIT as usize;
    for (limit, want) in [
        (0, def),
        (-3, def),
        (7, 7),
        (max as i64, max),
        (max as i64 + 1, max + 1),
        (500, max + 1),
    ] {
        for t in ["", "otter"] {
            let got = search(&r, Query { limit, ..text(t) }).len();
            assert_eq!(got, want, "limit {limit} text {t:?}");
        }
    }
}

#[test]
fn search_snippet() {
    let r = Rig::new(|_| {});
    let ka = key_a();
    let long = "filler words go here ".repeat(40);
    turn(
        &r,
        &ka,
        REPO_X,
        "p1",
        "Short title",
        &format!("{long}\n\nthe capybara\n line two {long}"),
        at(0),
    );
    turn(
        &r,
        &ka,
        REPO_X,
        "p2",
        "Empty-text listing",
        &format!("first line\nsecond line {long}"),
        at(1),
    );
    let h = search(&r, text("capybara"));
    assert_eq!(h.len(), 1, "{:?}", refs(&h));
    let s = &h[0].snippet;
    assert!(
        s.contains("[capybara]")
            && !s.contains(['\r', '\n'])
            && s.chars().count() <= search::MAX_SNIPPET,
        "snippet {s:?}"
    );
    let h = search(
        &r,
        Query {
            limit: 1,
            ..Query::default()
        },
    );
    let s = &h[0].snippet;
    assert!(
        s.starts_with("Empty-text listing first line second line")
            && !s.contains(['\r', '\n'])
            && s.chars().count() <= search::MAX_SNIPPET
            && s.ends_with('…'),
        "listing snippet {s:?}"
    );
    // Titles are one line and capped.
    turn(&r, &ka, REPO_X, "p3", &"tapir ".repeat(50), "", at(2));
    let h = search(&r, text("tapir"));
    assert!(
        h[0].title.chars().count() <= search::MAX_TITLE,
        "title {} runes",
        h[0].title.chars().count()
    );
}

/// A settable search::Source.
#[derive(Default)]
struct FakeSource {
    kinds: Vec<Kind>,
    st: Mutex<FakeState>,
    shown: Option<Shown>,
}

#[derive(Default)]
struct FakeState {
    docs: Vec<Doc>,
    calls: Vec<GoTime>,
    err: Option<String>,
}

impl FakeSource {
    fn new(kinds: &[Kind]) -> Arc<FakeSource> {
        Arc::new(FakeSource {
            kinds: kinds.to_vec(),
            ..FakeSource::default()
        })
    }
    fn set(&self, docs: Vec<Doc>) {
        let mut s = self.st.lock().unwrap();
        s.docs = docs;
        s.calls.clear();
    }
    fn sinces(&self) -> Vec<GoTime> {
        self.st.lock().unwrap().calls.clone()
    }
}

impl Source for FakeSource {
    fn search_kinds(&self) -> Vec<Kind> {
        self.kinds.clone()
    }
    fn search_docs(&self, since: GoTime, limit: usize) -> Result<Vec<Doc>, Error> {
        let mut s = self.st.lock().unwrap();
        s.calls.push(since);
        if let Some(e) = &s.err {
            return Err(Error::Other(e.clone()));
        }
        let mut out: Vec<Doc> = s
            .docs
            .iter()
            .filter(|d| {
                let u = if d.updated.is_zero() { d.at } else { d.updated };
                !capture::gotime::before(&u, &since)
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| capture::gotime::cmp(&a.updated, &b.updated));
        out.truncate(limit);
        Ok(out)
    }
    fn search_show(&self, r: &str, _o: &ShowOptions) -> Result<Shown, Error> {
        match &self.shown {
            Some(s) if s.r#ref == r => Ok(s.clone()),
            _ => Err(Error::NotFound),
        }
    }
}

fn doc(r: &str, upd: GoTime, title: &str) -> Doc {
    Doc {
        r#ref: r.into(),
        kind: Kind::Task,
        at: upd,
        updated: upd,
        title: title.into(),
        body: String::new(),
        agent: String::new(),
        repo_key: String::new(),
        readers: Vec::new(),
    }
}

fn task_doc(id: &str, title: &str, upd: GoTime) -> Doc {
    Doc {
        body: format!("body of {title}"),
        repo_key: REPO_X.into(),
        ..doc(&format!("task:{id}"), upd, title)
    }
}

#[test]
fn search_pulls_sources() {
    let r = Rig::new(|_| {});
    let src = FakeSource::new(&[Kind::Task, Kind::Comment]);
    r.c.add_source(src.clone());

    src.set(vec![
        task_doc("1", "Paint the shed", at(1)),
        task_doc("2", "Oil the gate", at(2)),
    ]);
    assert_eq!(refs(&search(&r, text("shed"))), ["task:1"], "first pull");
    let s = src.sinces();
    assert!(s.len() == 1 && s[0].is_zero(), "first pull since {s:?}");
    let h = search(&r, text("gate"));
    assert!(
        h.len() == 1 && h[0].kind == Kind::Task && h[0].repo_key == REPO_X,
        "{h:?}"
    );

    // A changed doc comes back with a newer Updated and replaces the old.
    src.set(vec![
        task_doc("1", "Paint the barn", at(3)),
        task_doc("2", "Oil the gate", at(2)),
    ]);
    assert_eq!(refs(&search(&r, text("barn"))), ["task:1"], "after change");
    let s = src.sinces();
    assert!(s.len() == 1 && s[0] == at(2), "second pull since {s:?}");
    assert!(search(&r, text("shed")).is_empty(), "old text still found");
    // The boundary doc is re-read and upserted, never duplicated.
    let q = Query {
        kinds: vec![Kind::Task],
        ..Query::default()
    };
    assert_eq!(refs(&search(&r, q)), ["task:1", "task:2"]);

    // Docs of kinds the source doesn't own, and bad refs, are skipped.
    src.set(vec![
        doc("turn:t-forged", at(9), "forged"),
        doc("nope", at(9), "forged"),
    ]);
    assert!(
        search(&r, text("forged")).is_empty(),
        "foreign kinds indexed"
    );

    // A failing source doesn't fail the search.
    src.st.lock().unwrap().err = Some("boom".into());
    assert_eq!(
        refs(&search(&r, text("barn"))),
        ["task:1"],
        "with failing source"
    );
    assert!(
        r.c.status().last_error.contains("boom"),
        "{}",
        r.c.status().last_error
    );
}

#[test]
fn search_pull_paging() {
    let cases = [
        ("short page", 3, false, 1, 3),
        ("pages until short", 1200, false, 3, 1200),
        (
            "exact page then empty-progress",
            PULL_PAGE,
            false,
            2,
            PULL_PAGE,
        ),
        ("no progress stops", PULL_PAGE + 100, true, 2, PULL_PAGE),
    ];
    for (name, n, same, want_calls, want_docs) in cases {
        let r = Rig::new(|_| {});
        let src = FakeSource::new(&[Kind::Msg]);
        r.c.add_source(src.clone());
        let docs = (0..n)
            .map(|i| {
                let u = if same { t0() } else { at_secs(&t0(), i as i64) };
                Doc {
                    kind: Kind::Msg,
                    ..doc(&format!("msg:m{i:05}"), u, "hello")
                }
            })
            .collect();
        src.set(docs);
        search(&r, Query::default());
        assert_eq!(src.sinces().len(), want_calls, "{name}: search_docs calls");
        assert_eq!(r.c.index_size(), want_docs, "{name}: docs indexed");
    }
}

#[test]
fn add_source_panics() {
    let cases: [(&str, &[Kind], &[Kind]); 4] = [
        ("capture's turn", &[], &[Kind::Turn]),
        ("capture's agent", &[], &[Kind::Agent]),
        ("capture's pr", &[], &[Kind::Msg, Kind::Pr]),
        ("duplicate", &[Kind::Task], &[Kind::Comment, Kind::Task]),
    ];
    for (name, first, next) in cases {
        let r = Rig::new(|_| {});
        if !first.is_empty() {
            r.c.add_source(FakeSource::new(first));
        }
        let c = r.c.clone();
        let next = next.to_vec();
        let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            c.add_source(FakeSource::new(&next));
        }));
        assert!(got.is_err(), "{name}: no panic");
        // Nothing of the refused source was registered.
        assert!(
            !r.c.has_source(Kind::Msg),
            "{name}: refused source half-registered"
        );
        assert!(
            !r.c.has_source(Kind::Comment),
            "{name}: refused source half-registered"
        );
    }
}

fn show(r: &Rig, rf: &str) -> Result<Shown, Error> {
    r.c.show(rf, &ShowOptions::default())
}

#[test]
fn show_renderings() {
    let r = Rig::new(|_| {});
    let (ka, kb) = (key_a(), key_b());
    let tr = turn(
        &r,
        &ka,
        REPO_X,
        "p1",
        "Fix the lease refusal",
        "Done: leases refuse.",
        at(0),
    );
    let e = edit(
        &r,
        &ka,
        REPO_X,
        "p1",
        "internal/lease/lease.go",
        at(1),
        "@@ -1 +1 @@\n-old\n+new\n",
    );
    let plain = edit(&r, &ka, REPO_X, "p1", "README.md", at(1), "");
    commit_row(
        &r,
        REPO_X,
        SHA,
        "Teach leases to refuse",
        at(2),
        &["internal/lease/lease.go"],
    );
    link(
        &r,
        REPO_X,
        SHA,
        Some(Committer {
            key: ka.clone(),
            turn_id: "p1".into(),
        }),
        &e,
    );
    pr(&r, "Acme/Widgets", 12, &ka, ledger::Role::AUTHOR, at(3));
    pr(&r, "Acme/Widgets", 12, &kb, ledger::Role::REVIEWER, at(4));
    decision(
        &r,
        dec(
            "d-1",
            party(&ka, "alice"),
            "Prefer quokka caches",
            "they are friendly",
            &[],
            at(5),
        ),
    );
    decision(
        &r,
        dec(
            "d-2",
            party(&ka, ""),
            "Prefer wombat caches",
            "",
            &["d-1"],
            at(6),
        ),
    );
    r.roster.set(vec![card("sess-a", "alice", "idle", REPO_X)]);
    let src = Arc::new(FakeSource {
        kinds: vec![Kind::Task],
        shown: Some(Shown {
            r#ref: "task:9".into(),
            kind: Kind::Task,
            title: "from source".into(),
            text: String::new(),
            more: false,
            next: 0,
            related: vec![],
        }),
        ..FakeSource::default()
    });
    r.c.add_source(src);

    let tref = format!("turn:{}", tr.id);
    let eref = format!("edit:{}", e.id);
    let pref = format!("edit:{}", plain.id);
    let cref = format!("commit:{SHA}");
    let aref = format!("agent:{ka}");
    let bref = format!("agent:{kb}");
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let cases: Vec<(&str, String, String, &str, Vec<String>, Vec<String>)> = vec![
        (
            "turn",
            tref.clone(),
            tref.clone(),
            "Fix the lease refusal",
            vec![
                "prompt:\nFix the lease refusal".into(),
                "reply:\nDone: leases refuse.".into(),
                format!("{eref} Edit internal/lease/lease.go +3 -1"),
                format!("{cref} Teach leases to refuse"),
            ],
            s(&[&eref, &pref, &cref, &aref]),
        ),
        (
            "edit with patch",
            eref.clone(),
            eref.clone(),
            "Edit internal/lease/lease.go +3 -1",
            vec![
                "fidelity exact".into(),
                format!("turn: {tref}"),
                format!("committed in: {cref}"),
                "patch:\n@@ -1 +1 @@\n-old\n+new".into(),
            ],
            s(&[&tref, &cref]),
        ),
        (
            "edit without patch",
            pref.clone(),
            pref.clone(),
            "Edit README.md +3 -1",
            vec!["(no patch stored)".into()],
            s(&[&tref]),
        ),
        (
            "commit by prefix",
            "commit:ABCDEF1".into(),
            cref.clone(),
            "Teach leases to refuse",
            vec![
                "author: Tester".into(),
                "branch: main".into(),
                "- M internal/lease/lease.go (exact)".into(),
                format!("committed by: {ka}"),
            ],
            s(&[&tref]),
        ),
        (
            "pr",
            "pr:Acme/Widgets#12".into(),
            "pr:acme/widgets#12".into(),
            "PR Acme/Widgets#12",
            vec![
                "url: https://github.com/Acme/Widgets/pull/12".into(),
                format!("{aref} author via gh"),
                format!("{bref} reviewer via gh"),
            ],
            s(&[&aref, &bref]),
        ),
        (
            "decision",
            "decision:d-1".into(),
            "decision:d-1".into(),
            "Prefer quokka caches",
            vec![
                "by alice (untrusted)".into(),
                "why:\nthey are friendly".into(),
                "superseded by: decision:d-2".into(),
            ],
            s(&["decision:d-2"]),
        ),
        (
            "agent",
            aref.clone(),
            aref.clone(),
            "alice",
            vec![
                format!("agent alice ({ka}), idle"),
                format!("{tref} Fix the lease refusal"),
                format!("{cref} Teach leases"),
                "pr:acme/widgets#12 author".into(),
            ],
            s(&["pr:acme/widgets#12", &cref, &tref]), // newest first
        ),
        (
            "source kind",
            "task:9".into(),
            "task:9".into(),
            "from source",
            vec![],
            vec![],
        ),
    ];
    for (name, rf, want_ref, title, texts, related) in cases {
        let sh = show(&r, &rf).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            sh.r#ref == want_ref && sh.title == title && !sh.more,
            "{name}: shown {sh:?}"
        );
        for t in &texts {
            assert!(
                sh.text.contains(t.as_str()),
                "{name}: text lacks {t:?}:\n{}",
                sh.text
            );
        }
        assert_eq!(sh.related, related, "{name}: related");
    }

    let errs: [(&str, Error); 13] = [
        ("", Error::BadRef),
        ("turn", Error::BadRef),
        ("bogus:1", Error::BadRef),
        ("commit:abc", Error::BadRef),
        ("turn:t-missing", Error::NotFound),
        ("edit:e-missing", Error::NotFound),
        ("commit:1234567", Error::NotFound),
        ("pr:acme/widgets#99", Error::NotFound),
        ("decision:nope", Error::NotFound),
        ("agent:dev1/nobody", Error::NotFound),
        ("task:other", Error::NotFound),
        ("msg:m1", Error::NotFound),
        ("comment:c1", Error::NotFound),
    ];
    for (rf, want) in errs {
        let got = show(&r, rf).err();
        let ok = match want {
            Error::BadRef => got == Some(Error::BadRef),
            _ => got.as_ref().is_some_and(Error::is_not_found),
        };
        assert!(ok, "show({rf:?}) = {got:?}, want {want:?}");
    }
}

#[test]
fn show_paging() {
    let r = Rig::new(|_| {});
    let reply = "ünïcode text, ".repeat(1000);
    let tr = turn(&r, &key_a(), REPO_X, "p1", "Long one", &reply, at(0));
    let rf = format!("turn:{}", tr.id);
    let sh = show(&r, &rf).unwrap();
    assert!(
        sh.more && sh.text.len() as i64 <= search::DEFAULT_SHOW && sh.next == sh.text.len() as i64,
        "first page: more {} next {} len {}",
        sh.more,
        sh.next,
        sh.text.len()
    );
    let big =
        r.c.show(
            &rf,
            &ShowOptions {
                limit: 1 << 20,
                ..ShowOptions::default()
            },
        )
        .unwrap();
    assert!(
        big.text.len() as i64 <= search::MAX_SHOW,
        "limit not capped: {}",
        big.text.len()
    );
    // Walking next reassembles the whole rendering (Rust strings are
    // always whole runes, so a cut rune would have panicked in page).
    let mut all = String::new();
    let mut o = 0;
    loop {
        let p =
            r.c.show(
                &rf,
                &ShowOptions {
                    offset: o,
                    limit: 1000,
                    ..ShowOptions::default()
                },
            )
            .unwrap();
        all.push_str(&p.text);
        if !p.more {
            break;
        }
        o = p.next;
    }
    assert!(
        all.contains(reply.trim()),
        "pages don't reassemble the reply"
    );
}

#[test]
fn show_related_cap() {
    let r = Rig::new(|_| {});
    let ka = key_a();
    let tr = turn(&r, &ka, REPO_X, "p1", "Many edits", "", at(0));
    for i in 0..20 {
        edit(&r, &ka, REPO_X, "p1", &format!("f{i:02}.go"), at(1), "");
    }
    let sh = show(&r, &format!("turn:{}", tr.id)).unwrap();
    assert_eq!(sh.related.len(), search::MAX_RELATED);
    assert_eq!(
        sh.text.matches("edit:e-").count(),
        20,
        "text should list every edit:\n{}",
        sh.text
    );
}

#[test]
fn show_open_turn() {
    let r = Rig::new(|_| {});
    let ka = key_a();
    r.c.set_open_turn(
        &ka,
        "",
        Some(Turn {
            id: "p9".into(),
            prompt: "Still going".into(),
            started_at: at(0),
            ..Turn::default()
        }),
    );
    let sh = show(&r, &format!("turn:{}", turn_row_id(&ka, "", "p9"))).unwrap();
    assert!(
        sh.text.contains(", open;") && sh.text.contains("Still going"),
        "open turn:\n{}",
        sh.text
    );
}

#[test]
fn overview() {
    let r = Rig::new(|_| {});
    r.roster.set(vec![
        card("sess-z", "zed", "idle", REPO_Y),
        card("sess-b", "Bob", "busy", REPO_X),
        card("sess-a", "alice", "busy", REPO_X),
        card("sess-me", "me", "busy", REPO_X),
        card("sess-c", "carol", "idle", REPO_X),
        card("sess-gone", "ghost", "gone", REPO_X),
    ]);
    let k = |s: &str| format!("{DEVICE}/{s}");
    let (me, bob, alice, carol, zed) = (
        k("sess-me"),
        k("sess-b"),
        k("sess-a"),
        k("sess-c"),
        k("sess-z"),
    );

    // alice: open turn (wins over everything). bob: a current task (wins
    // over his last reply). carol: only replies, the newest wins. zed:
    // nothing.
    r.c.set_open_turn(
        &alice,
        "",
        Some(Turn {
            id: "pa".into(),
            prompt: "Refactor the ferret\nmore".into(),
            started_at: at(50),
            ..Turn::default()
        }),
    );
    turn(&r, &alice, REPO_X, "a1", "old", "alice old reply", at(1));
    turn(&r, &bob, REPO_X, "b1", "q", "bob reply", at(2));
    turn(&r, &carol, REPO_X, "c1", "q", "carol older reply", at(3));
    let newest = turn(
        &r,
        &carol,
        REPO_X,
        "c2",
        "q",
        &"carol newest reply ".repeat(20),
        at(4),
    );
    turn(&r, &carol, REPO_X, "c3", "q", "", at(5)); // no reply: skipped

    for i in 0..7 {
        commit_row(
            &r,
            REPO_X,
            &format!("{:040x}", i + 1),
            &format!("commit {i}"),
            at(10 + i),
            &[],
        );
    }
    commit_row(
        &r,
        REPO_Y,
        &format!("{:040x}", 99),
        "other repo",
        at(30),
        &[],
    );
    for i in 0..7 {
        pr(
            &r,
            "acme/widgets",
            100 + i,
            &bob,
            ledger::Role::AUTHOR,
            at(20),
        );
        pr(
            &r,
            "acme/widgets",
            100 + i,
            &alice,
            ledger::Role::REVIEWER,
            at(21),
        ); // listed once
    }
    pr(&r, "acme/other", 5, &bob, ledger::Role::AUTHOR, at(20));

    let line = |rf: &str, t: &str| Line {
        r#ref: rf.into(),
        text: t.into(),
        at: GoTime::ZERO,
    };
    let long = "x".repeat(300);
    let input = OverviewInput {
        inbox_open: 3,
        inbox_held: 1,
        inbox: vec![line("msg:m1", "hello\nworld")],
        blockers: vec![line("task:b1", &long)],
        tasks: (1..=6).map(|i| line(&format!("task:{i}"), "")).collect(),
        more_tasks: 2,
        current: [
            (bob.clone(), line("task:7", "Ship the badger")),
            (alice.clone(), line("task:8", "not shown")),
        ]
        .into_iter()
        .collect(),
        repo_key: REPO_X.into(),
        board_count: 9,
    };
    let ov = r.c.overview(&me, &input);
    let order: Vec<&str> = ov.agents.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(order, ["me", "alice", "Bob", "carol", "zed"]);
    let doing = |key: &str| {
        ov.agents
            .iter()
            .find(|a| a.key == key)
            .unwrap()
            .doing
            .clone()
    };
    let carol_text = clip_runes("carol newest reply ".repeat(20).trim(), OVERVIEW_LINE_MAX);
    let cases = [
        (
            &alice,
            format!("agent:{alice}"),
            "Refactor the ferret".to_string(),
        ),
        (&bob, "task:7".to_string(), "Ship the badger".to_string()),
        (&carol, format!("turn:{}", newest.id), carol_text),
        (&zed, format!("agent:{zed}"), String::new()),
        (&me, format!("agent:{me}"), String::new()),
    ];
    for (key, rf, t) in cases {
        let d = doing(key);
        assert!(
            d.r#ref == rf && d.text == t,
            "{key} doing {d:?}, want {rf} {t:?}"
        );
        assert!(d.text.chars().count() <= OVERVIEW_LINE_MAX);
    }
    assert!(
        ov.commits.len() == OVERVIEW_LINES
            && ov.commits[0].r#ref == format!("commit:{:040x}", 7)
            && ov.commits[0].text == "commit 6",
        "commits {:?}",
        ov.commits
    );
    assert_eq!(ov.prs.len(), OVERVIEW_LINES, "prs {:?}", ov.prs);
    let mut seen = std::collections::HashSet::new();
    for p in &ov.prs {
        assert!(
            seen.insert(p.r#ref.clone()) && p.r#ref.starts_with("pr:acme/widgets#"),
            "prs {:?}",
            ov.prs
        );
    }
    assert!(
        ov.prs[0].r#ref == "pr:acme/widgets#106"
            && ov.prs[0].text == "PR acme/widgets#106 author Bob",
        "first pr {:?}",
        ov.prs[0]
    );
    assert!(
        ov.inbox_open == 3 && ov.inbox_held == 1 && ov.inbox[0].text == "hello",
        "{ov:?}"
    );
    assert_eq!(
        ov.blockers[0].text.chars().count(),
        OVERVIEW_LINE_MAX,
        "blocker not clipped"
    );
    assert!(
        ov.tasks.len() == OVERVIEW_LINES && ov.more_tasks == 3,
        "tasks {} more {}",
        ov.tasks.len(),
        ov.more_tasks
    );
    assert!(
        ov.r#for == me
            && ov.repo_key == REPO_X
            && ov.hint.contains("show <ref>")
            && ov.hint.contains("(9 open)"),
        "{ov:?}"
    );
}

#[test]
fn overview_caps_and_repo_fallback() {
    let r = Rig::new(|_| {});
    let cards = (0..15)
        .map(|i| {
            let rk = if i % 2 == 0 { REPO_X } else { REPO_Y };
            card(&format!("s{i:02}"), &format!("agent {i:02}"), "idle", rk)
        })
        .collect();
    r.roster.set(cards);
    let me = format!("{DEVICE}/s07"); // in repo Y
    commit_row(&r, REPO_Y, &format!("{:040x}", 1), "in y", at(0), &[]);
    commit_row(&r, REPO_X, &format!("{:040x}", 2), "in x", at(1), &[]);

    let ov = r.c.overview(&me, &OverviewInput::default());
    assert!(
        ov.agents.len() == OVERVIEW_AGENTS && ov.more_agents == 3,
        "{} agents, {} more",
        ov.agents.len(),
        ov.more_agents
    );
    assert!(
        ov.agents[0].key == me && ov.repo_key == REPO_Y,
        "first {} repo {}",
        ov.agents[0].key,
        ov.repo_key
    );
    for (i, a) in ov.agents[1..7].iter().enumerate() {
        assert_eq!(a.repo_key, REPO_Y, "agent {} {}", i + 1, a.key);
    }
    assert!(
        ov.commits.len() == 1 && ov.commits[0].text == "in y",
        "commits {:?}",
        ov.commits
    );
    assert!(ov.hint.contains("3 more agents"), "hint {:?}", ov.hint);

    // A repo that isn't on GitHub never lists every repo's PRs.
    pr(&r, "acme/widgets", 1, &me, ledger::Role::AUTHOR, at(0));
    let ov = r.c.overview(
        &me,
        &OverviewInput {
            repo_key: "path:C:/somewhere".into(),
            ..OverviewInput::default()
        },
    );
    assert!(ov.prs.is_empty(), "prs for a non-GitHub repo: {:?}", ov.prs);
    // No repo at all: no commits or PRs.
    let ov =
        r.c.overview(&format!("{DEVICE}/not-live"), &OverviewInput::default());
    assert!(
        ov.repo_key.is_empty() && ov.commits.is_empty() && ov.prs.is_empty(),
        "no-repo overview {ov:?}"
    );
}

/// Offset pages ranked and plain results alike: a newer, weaker match and a
/// doc sharing the last hit's at are both reached on the next page.
#[test]
fn search_offset_paging() {
    let r = Rig::new(|_| {});
    let ka = key_a();
    turn(&r, &ka, REPO_X, "p1", "wombat wombat", "", at(0)); // strong title match, older
    turn(
        &r,
        &ka,
        REPO_X,
        "p2",
        "feeding time",
        "a wombat came",
        at(10),
    ); // weak body match, newer
    turn(&r, &ka, REPO_X, "p3", "same minute", "", at(10));
    for t in ["wombat", ""] {
        let all = refs(&search(&r, text(t)));
        let mut paged = Vec::new();
        for off in 0..=all.len() as i64 {
            paged.extend(refs(&search(
                &r,
                Query {
                    limit: 1,
                    offset: off,
                    ..text(t)
                },
            )));
        }
        assert_eq!(paged, all, "text {t:?}");
    }
    assert_eq!(
        search(
            &r,
            Query {
                offset: -4,
                ..Query::default()
            }
        )
        .len(),
        3,
        "negative offset"
    );
}

/// A doc with readers is found only by the person ("") and the agents it
/// names; docs without readers are found by everyone.
#[test]
fn search_viewer() {
    let r = Rig::new(|_| {});
    let (ka, kb) = (key_a(), key_b());
    let src = FakeSource::new(&[Kind::Msg]);
    r.c.add_source(src.clone());
    src.set(vec![
        Doc {
            kind: Kind::Msg,
            body: "secret pangolin plan".into(),
            readers: vec![ka.clone(), kb.clone()],
            ..doc("msg:1", at(1), "alpha -> beta")
        },
        Doc {
            kind: Kind::Msg,
            body: "public pangolin note".into(),
            ..doc("msg:2", at(2), "note")
        },
    ]);
    let cases = [
        (String::new(), vec!["msg:1", "msg:2"]),
        (ka.clone(), vec!["msg:1", "msg:2"]),
        (kb.clone(), vec!["msg:1", "msg:2"]),
        (format!("{DEVICE}/sess-c"), vec!["msg:2"]),
        (ka[..ka.len() - 1].to_string(), vec!["msg:2"]), // a prefix of a reader is not that reader
    ];
    for (viewer, want) in cases {
        let mut got = refs(&search(
            &r,
            Query {
                viewer: viewer.clone(),
                ..text("pangolin")
            },
        ));
        got.sort();
        assert_eq!(got, want, "viewer {viewer:?}");
        let listing = search(
            &r,
            Query {
                viewer: viewer.clone(),
                ..Query::default()
            },
        );
        assert_eq!(
            listing.len(),
            want.len(),
            "viewer {viewer:?} listing {:?}",
            refs(&listing)
        );
    }
}

/// A wake's fenced messages in a turn prompt are kept from viewers who are
/// neither the turn's agent (the recipient) nor the message's sender, and
/// never reach the shared index.
#[test]
fn hide_messages_cases() {
    let (ka, kb) = (key_a(), key_b());
    let kc = format!("{DEVICE}/sess-c");
    let msg = |key: &str, body: &str| {
        format!(
            "\n<irori-message from=\"x\" key=\"{key}\" delivery=\"dlv_1\" trust=\"untrusted\">\n{body}\n</irori-message>\n"
        )
    };
    let two = format!(
        "[irori-wake:w] delivered.{}{}",
        msg(&ka, "pangolin one"),
        msg(&kc, "pangolin two")
    );
    let clipped = format!(
        "[irori-wake:w] delivered.\n<IRORI-MESSAGE from=\"x\" key=\"{ka}\">\npangolin cut off"
    );
    let hide_all = "\0";
    let cases: [(&str, &str, &str, &[&str], &[&str]); 6] = [
        (
            "person sees all",
            &two,
            "",
            &["pangolin one", "pangolin two"],
            &[],
        ),
        (
            "recipient sees all",
            &two,
            &kb,
            &["pangolin one", "pangolin two"],
            &[],
        ),
        (
            "sender sees its own",
            &two,
            &ka,
            &["pangolin one", HIDDEN_MESSAGE],
            &["pangolin two"],
        ),
        (
            "index sees none",
            &two,
            hide_all,
            &["[irori-wake:w] delivered.", HIDDEN_MESSAGE],
            &["pangolin"],
        ),
        (
            "clipped body",
            &clipped,
            &kc,
            &[HIDDEN_MESSAGE],
            &["pangolin"],
        ),
        (
            "no fence",
            "plain pangolin prompt",
            &kc,
            &["plain pangolin prompt"],
            &[],
        ),
    ];
    for (name, prompt, viewer, want, want_not) in cases {
        let got = hide_messages(prompt, viewer, &kb);
        for s in want {
            assert!(got.contains(s), "{name}: lacks {s:?}: {got:?}");
        }
        for s in want_not {
            assert!(!got.contains(s), "{name}: has {s:?}: {got:?}");
        }
    }
    let r = Rig::new(|_| {});
    let tr = turn(&r, &kb, REPO_X, "w1", &two, "replied", at(0));
    assert!(
        search(&r, text("pangolin")).is_empty(),
        "index holds fenced bodies"
    );
    let sh =
        r.c.show(
            &format!("turn:{}", tr.id),
            &ShowOptions {
                viewer: kc,
                ..ShowOptions::default()
            },
        )
        .unwrap();
    assert!(
        sh.text.contains("pangolin two") && !sh.text.contains("pangolin one"),
        "show as the second sender:\n{}",
        sh.text
    );
}
