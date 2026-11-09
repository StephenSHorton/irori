//! Ports of I:internal/presence/presence_test.go, names_test.go,
//! v02_test.go, lstart_test.go, live_windows_test.go and exit_test.go.

use crate::common;

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use gocompat::time::GoTime;
use presence::internal::{
    Session, Tail, changed, is_session_file, one_line, parse_lstart, read_session, read_tail,
    scan_mode,
};
use presence::{AgentCard, ChangeKind, Namer, Options, SOURCE_IRORI, SOURCE_USER, Watcher, slug};
use serde_json::json;

#[test]
fn slug_table() {
    for (input, want) in [
        (r"C:\Users\pat", "C--Users-pat"),
        ("/Users/pat/projects/irori", "-Users-pat-projects-irori"),
        (
            r"C:\Users\pat\projects\irori",
            "C--Users-pat-projects-irori",
        ),
        ("/home/me/my_app.v2", "-home-me-my-app-v2"),
        ("/tmp/café", "-tmp-caf-"),
        ("", ""),
    ] {
        assert_eq!(slug(input), want, "Slug({input:?})");
    }
}

#[test]
fn is_session_file_table() {
    for (name, want) in [
        ("13704.json", true),
        (
            "39668.0000000000000000000000000000000000000000000000000000000000000000.key",
            false,
        ),
        ("39668.key", false),
        ("39668.abc.json", false),
        (".json", false),
        ("x1.json", false),
        ("123.json.bak", false),
    ] {
        assert_eq!(is_session_file(name), want, "isSessionFile({name:?})");
    }
}

/// Card #445: a `--bg` session stopped at a permission prompt (as Claude
/// Code 2.1.318 wrote it on the Mac) says so, and the card carries it.
#[test]
fn waiting_for_a_permission_prompt() {
    let raw = br#"{"pid":52788,"sessionId":"ce277a06-054e-4f11-9bd2-17da8a5815f5","cwd":"/Users/pat/scratch","startedAt":1793766937837,"version":"2.1.318","kind":"bg","entrypoint":"cli","name":"probe","jobId":"ce277a06","status":"waiting","updatedAt":1793766942188,"statusUpdatedAt":1793766942188,"waitingFor":"permission prompt"}"#;
    let s = read_session(&raw[..]).expect("session");
    assert_eq!(
        (s.status.as_str(), s.waiting_for.as_str()),
        ("waiting", "permission prompt")
    );
    let c = AgentCard {
        waiting_for: s.waiting_for.clone(),
        ..AgentCard::default()
    };
    let j = serde_json::to_value(&c).unwrap();
    assert_eq!(j["waitingFor"], "permission prompt");
    let idle = serde_json::to_value(AgentCard::default()).unwrap();
    assert!(
        idle.get("waitingFor").is_none(),
        "omitted when empty, as Go's omitempty"
    );
    assert!(changed(&c, &AgentCard::default()));
    assert!(idle.get("statusSince").is_none(), "omitted when zero");
    // Card #489: statusSince is sent, and a change to it alone is a change.
    let c = AgentCard {
        status_since: s.status_since,
        ..AgentCard::default()
    };
    let j = serde_json::to_value(&c).unwrap();
    assert!(j["statusSince"].is_string(), "{j}");
    assert!(changed(&c, &AgentCard::default()));
}

#[test]
fn read_session_fixtures() {
    let f = std::fs::File::open(testdata().join("sessions").join("13704.json")).unwrap();
    let s = read_session(f).expect("13704");
    assert_eq!(
        (s.pid, s.id.as_str(), s.cwd.as_str()),
        (13704, FIXTURE_ID, r"C:\Users\pat")
    );
    assert_eq!(s.proc_start, 134375320001234567);
    assert_eq!(
        (s.status.as_str(), s.kind.as_str(), s.peer_features.len()),
        ("busy", "interactive", 2)
    );
    assert!(s.updated_at.equal(&GoTime::unix_milli(1793063865535)));
    // Card #489: statusUpdatedAt, not the later updatedAt a rename moved.
    assert!(s.status_since.equal(&GoTime::unix_milli(1793061536254)));
    assert!(s.started_at.equal(&GoTime::unix_milli(1793058851218)));

    // 39668.json is a bg session with an escaped Windows cwd.
    let f = std::fs::File::open(testdata().join("sessions").join("39668.json")).unwrap();
    let s = read_session(f).expect("39668");
    assert_eq!(
        (s.pid, s.cwd.as_str(), s.kind.as_str(), s.name.as_str()),
        (39668, r"C:\Users\pat", "bg", "hello-spike")
    );
    assert!(s.status_since.equal(&GoTime::unix_milli(1793064011488)));

    for bad in [
        "",
        "{",
        r#"{"pid":0,"sessionId":"x"}"#,
        r#"{"pid":5}"#,
        "[1,2]",
    ] {
        assert!(read_session(bad.as_bytes()).is_none(), "accepted {bad:?}");
    }
    // Unknown fields and odd types are tolerated.
    let s = read_session(
        &br#"{"pid":"7","sessionId":"abc","procStart":12,"peerFeatures":[1,"x"],"new":{"a":1}}"#[..],
    )
    .expect("tolerant");
    assert_eq!((s.pid, s.proc_start, s.peer_features.len()), (7, 12, 1));
    // No statusUpdatedAt: status_since falls back to updatedAt (card #489).
    let s = read_session(&br#"{"pid":7,"sessionId":"abc","updatedAt":1793064012119}"#[..])
        .expect("no statusUpdatedAt");
    assert!(s.status_since.equal(&GoTime::unix_milli(1793064012119)));
}

#[test]
fn read_tail_skips_partial_line() {
    let p = testdata()
        .join("projects")
        .join("C--Users-pat")
        .join(format!("{FIXTURE_ID}.jsonl"));
    let raw = std::fs::read(p).unwrap();
    let find = |needle: &[u8]| raw.windows(needle.len()).position(|w| w == needle);
    let pad = find(b"PADDING").expect("fixture lost its padding row") as u64;
    let size = raw.len() as u64;
    let n = size - (pad + 100); // window opens inside the padding row

    let got = read_tail(&mut Cursor::new(&raw), size, n);
    assert_eq!(got.title, "Port the roster");
    assert_eq!(got.last_text, "Roster card shows the last reply.");
    assert!(
        got.last_activity
            .equal(&GoTime::parse_rfc3339("2026-10-26T10:03:00Z").unwrap())
    );

    // The partial row's timestamp (10:00:05) and everything before it stay out.
    let cut = &raw[..(pad + 200) as usize];
    let got = read_tail(&mut Cursor::new(cut), pad + 200, 150);
    assert!(got.last_activity.is_zero() && got.title.is_empty() && got.last_text.is_empty());

    // A window opening exactly on a row keeps it; one byte later drops it.
    let row = find(br#"{"type":"custom-title","customTitle":"Port the roster""#)
        .expect("fixture lost its in-window custom title") as u64;
    let got = read_tail(&mut Cursor::new(&raw), size, size - row);
    assert_eq!(got.title, "Port the roster", "aligned window");
    let got = read_tail(&mut Cursor::new(&raw), size, size - row - 1);
    assert_eq!(
        got.title, "Synthetic ai title",
        "window one byte into the row"
    );

    // The whole file: the newest custom title still wins over the ai title.
    let got = read_tail(&mut Cursor::new(&raw), size, size + 10);
    assert_eq!(got.title, "Port the roster");
    assert_eq!(got.last_text, "Roster card shows the last reply.");
}

fn tail_of(src: &str, n: u64) -> Tail {
    read_tail(&mut Cursor::new(src.as_bytes()), src.len() as u64, n)
}

#[test]
fn title_falls_back_to_ai_title() {
    let src = "{\"type\":\"ai-title\",\"aiTitle\":\"first\"}\n{\"type\":\"ai-title\",\"aiTitle\":\"second\"}\n";
    assert_eq!(tail_of(src, 1 << 20).title, "second");
}

#[test]
fn one_line_cases() {
    assert_eq!(one_line(" a\n\n b\t c ", 240), "a b c");
    let got = one_line(&"é".repeat(500), 240);
    assert_eq!(got.chars().count(), 240);
    assert!(got.ends_with('…'));
}

/// The `<pid>.<hash>.key` decoy in testdata must never be opened.
#[test]
fn scan_never_opens_key_files() {
    let (w, o) = test_watcher(&testdata(), true);
    w.rescan();
    for p in o.paths() {
        assert!(!p.ends_with(".key"), "opened secret {p}");
    }
    assert_eq!(
        o.count(".json"),
        2,
        "want both session files read: {:?}",
        o.paths()
    );

    let cards = w.snapshot();
    assert_eq!(cards.len(), 2);
    let (a, b) = (&cards[0], &cards[1]);
    assert_eq!(
        (a.session_id.as_str(), b.session_id.as_str()),
        (FIXTURE_ID, FIXTURE_ID2)
    );
    assert_eq!(a.key, format!("dev1/{FIXTURE_ID}"));
    assert_eq!((a.agent.as_str(), a.device_id.as_str()), ("claude", "dev1"));
    assert_eq!(a.name, "roster-port-oct-26");
    assert_eq!(a.title, "Port the roster");
    assert_eq!(a.last_text, "Roster card shows the last reply.");
    assert_eq!(a.permission_mode, "bypassPermissions");
    assert_eq!(b.permission_mode, "");
    assert!(Path::new(&a.transcript).is_absolute());
    let want = Path::new("C--Users-pat").join(format!("{FIXTURE_ID}.jsonl"));
    assert!(
        a.transcript.ends_with(&*want.to_string_lossy()),
        "{}",
        a.transcript
    );
    // Found through the projects/*/<id>.jsonl fallback.
    let want = Path::new("elsewhere").join(format!("{FIXTURE_ID2}.jsonl"));
    assert!(
        b.transcript.ends_with(&*want.to_string_lossy()),
        "{}",
        b.transcript
    );
    assert_eq!(b.title, "Hello spike");
}

#[test]
fn dead_pid_not_listed() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let ds = dir.to_string_lossy();
    write_session(
        dir,
        json!({"pid": 1 << 30, "sessionId": "dead-session", "cwd": ds}),
    );
    write_session(
        dir,
        json!({"pid": std::process::id(), "sessionId": "me-session", "cwd": ds}),
    );
    let (w, _) = test_watcher(dir, false);
    w.rescan();
    let cards = w.snapshot();
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert_eq!(cards[0].session_id, "me-session");
    assert_eq!(
        cards[0].name,
        dir.file_name().unwrap().to_string_lossy(),
        "derived name"
    );
    assert_eq!(cards[0].transcript, "");
}

#[test]
fn tail_read_only_when_transcript_moves() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let cwd = r"C:\work\demo";
    let id = "0b1c2d3e-0000-4000-8000-000000000001";
    write_session(
        dir,
        json!({"pid": 42, "sessionId": id, "cwd": cwd, "status": "busy"}),
    );
    let tp = dir
        .join("projects")
        .join(slug(cwd))
        .join(format!("{id}.jsonl"));
    append_file(&tp, &assistant_row("2026-10-26T10:00:00Z", "one"));

    let (w, o) = test_watcher(dir, true);
    let sub = w.subscribe();
    w.rescan();
    w.rescan();
    w.rescan();
    assert_eq!(
        o.count(".jsonl"),
        1,
        "transcript opened for an unchanged file"
    );
    let c = sub.try_recv();
    let gocompat::sub::TryRecv::Item(c) = c else {
        panic!("no first change: {c:?}")
    };
    assert_eq!(
        (c.kind, c.card.last_text.as_str()),
        (ChangeKind::Added, "one")
    );

    append_file(&tp, &assistant_row("2026-10-26T10:00:05Z", "two"));
    w.rescan();
    assert_eq!(o.count(".jsonl"), 2, "transcript not reread after it grew");
    let cs = drain(&sub);
    assert_eq!(cs.len(), 1, "{cs:?}");
    assert_eq!(
        (cs[0].kind, cs[0].card.last_text.as_str()),
        (ChangeKind::Updated, "two")
    );
}

fn card_at(secs: i64) -> AgentCard {
    AgentCard {
        key: "d/s".into(),
        peer_features: vec!["a".into()],
        last_activity: GoTime::unix(secs, 0),
        ..AgentCard::default()
    }
}

#[test]
fn changed_ignores_sub_second_activity() {
    let base = card_at(100);
    let mut b = base.clone();
    b.last_activity = GoTime::unix(100, 500_000_000);
    assert!(
        !changed(&base, &b),
        "sub-second activity counted as a change"
    );
    b.last_activity = GoTime::unix(101, 0);
    assert!(changed(&base, &b), "a full second of activity was ignored");
    let mut b = base.clone();
    b.peer_features = vec!["a".into(), "b".into()];
    assert!(changed(&base, &b), "peerFeatures change ignored");
    let mut b = base.clone();
    b.status = "idle".into();
    assert!(changed(&base, &b), "status change ignored");
}

#[tokio::test(flavor = "multi_thread")]
async fn subscribe_added_then_removed() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let p = write_session(
        dir,
        json!({"pid": std::process::id(), "sessionId": "sub-session", "cwd": dir.to_string_lossy(), "status": "idle"}),
    );
    let (w, _) = test_watcher(dir, false);
    let sub = w.subscribe();
    let r = run_watcher(&w);
    let c = await_change(&sub, "Added", |_| true).await;
    assert_eq!(
        (c.kind, c.card.session_id.as_str(), c.card.status.as_str()),
        (ChangeKind::Added, "sub-session", "idle")
    );
    std::fs::remove_file(&p).unwrap();
    let c = await_change(&sub, "Removed", |_| true).await;
    assert_eq!(
        (c.kind, c.card.status.as_str(), c.card.key.as_str()),
        (ChangeKind::Removed, "gone", "dev1/sub-session")
    );
    assert!(
        w.snapshot().is_empty(),
        "snapshot still lists the removed session"
    );
    r.stop().await;
    drop(w);
    sub.cancel();
    while sub.recv_async().await.is_some() {}
}

/// Invalid session JSON is skipped, never guessed at: a repair would turn
/// the "\t" in C:\Users\pat\tools into a TAB.
#[test]
fn read_session_rejects_invalid_json() {
    let raw = r#"{"pid":5,"sessionId":"a","cwd":"C:\Users\pat\tools"}"#;
    assert!(
        read_session(raw.as_bytes()).is_none(),
        "invalid JSON was accepted"
    );
}

// --- names_test.go

/// Gives every session it is asked to name "heron", and remembers. "s" was
/// called "otter" before a clash moved it.
#[derive(Default)]
struct FakeNamer(Mutex<HashMap<String, String>>);

impl Namer for FakeNamer {
    fn assigned(&self, id: &str) -> String {
        self.0.lock().unwrap().get(id).cloned().unwrap_or_default()
    }
    fn gave(&self, id: &str, name: &str) -> bool {
        let m = self.0.lock().unwrap();
        let cur = m.get(id).cloned().unwrap_or_default();
        !cur.is_empty()
            && (gocompat::text::equal_fold(&cur, name)
                || (id == "s" && gocompat::text::equal_fold(name, "otter")))
    }
    fn assign(&self, id: &str) -> String {
        let mut m = self.0.lock().unwrap();
        m.entry(id.to_string())
            .or_insert_with(|| "heron".into())
            .clone()
    }
}

fn sess(name: &str, source: &str, cwd: &str) -> Session {
    Session {
        id: "s".into(),
        name: name.into(),
        name_source: source.into(),
        cwd: cwd.into(),
        ..Session::default()
    }
}

fn tail(custom: &str, ai: &str) -> Tail {
    Tail {
        custom: custom.into(),
        ai: ai.into(),
        ..Tail::default()
    }
}

#[test]
fn name_precedence() {
    let cases: Vec<(&str, Session, Tail, &str, &str, &str)> = vec![
        (
            "a /rename wins",
            sess("Bob", "user", ""),
            tail("Bob", "Fix it"),
            "heron",
            "Bob",
            SOURCE_USER,
        ),
        (
            "a session name with no source is a choice",
            sess("api", "", ""),
            tail("", ""),
            "",
            "api",
            SOURCE_USER,
        ),
        (
            "a custom title is a choice",
            sess("", "", ""),
            tail("parser work", "Fix it"),
            "",
            "parser work",
            SOURCE_USER,
        ),
        (
            "Irori's name written back as a title stays Irori's",
            sess("", "", ""),
            tail("Heron", ""),
            "heron",
            "Heron",
            SOURCE_IRORI,
        ),
        (
            "Irori's name written back as the session name stays Irori's",
            sess("heron", "user", ""),
            tail("", ""),
            "heron",
            "heron",
            SOURCE_IRORI,
        ),
        (
            "Irori's earlier name written back as a title becomes its current one",
            sess("", "", ""),
            tail("otter", ""),
            "heron",
            "heron",
            SOURCE_IRORI,
        ),
        (
            "so does an earlier name written back as the session name",
            sess("otter", "user", ""),
            tail("otter", ""),
            "heron",
            "heron",
            SOURCE_IRORI,
        ),
        (
            "a name Claude set itself isn't a choice",
            sess("Fix it", "auto", ""),
            tail("", "Fix it"),
            "",
            "heron",
            SOURCE_IRORI,
        ),
        (
            "an ai-title never is",
            sess("", "", r"C:\work\api"),
            tail("", "Fix the parser"),
            "",
            "heron",
            SOURCE_IRORI,
        ),
        (
            "no name at all gets Irori's",
            sess("", "", r"C:\work\api"),
            tail("", ""),
            "",
            "heron",
            SOURCE_IRORI,
        ),
    ];
    for (what, s, t, assigned, want_name, want_source) in cases {
        let f = Arc::new(FakeNamer::default());
        if !assigned.is_empty() {
            f.0.lock().unwrap().insert("s".into(), assigned.into());
        }
        let w = Watcher::new(Options {
            names: Some(f),
            ..Options::default()
        });
        let (name, source) = w.name_of(&s, &t);
        assert_eq!(
            (name.as_str(), source.as_str()),
            (want_name, want_source),
            "{what}"
        );
    }
}

#[test]
fn name_without_namer_keeps_the_old_fallback() {
    let w = Watcher::new(Options::default());
    let mut t = tail("", "Fix it");
    t.title = "Fix it".into();
    assert_eq!(
        w.name_of(&sess("", "", r"C:\work\api"), &t),
        ("Fix it".into(), String::new())
    );
    assert_eq!(
        w.name_of(&sess("", "", r"C:\work\api"), &Tail::default()),
        ("api".into(), String::new())
    );
}

#[test]
fn changed_sees_name_source_and_ai_title() {
    let base = AgentCard {
        key: "d/s".into(),
        ..AgentCard::default()
    };
    let mut b = base.clone();
    b.name_source = SOURCE_IRORI.into();
    assert!(changed(&base, &b), "nameSource change ignored");
    let mut b = base.clone();
    b.ai_title = "Fix it".into();
    assert!(changed(&base, &b), "aiTitle change ignored");
}

#[test]
fn read_tail_keeps_custom_and_ai_titles_apart() {
    let src = "{\"type\":\"ai-title\",\"aiTitle\":\"Fix it\"}\n{\"type\":\"custom-title\",\"customTitle\":\"Bob\"}\n";
    let got = tail_of(src, 1 << 20);
    assert_eq!(
        (got.custom.as_str(), got.ai.as_str(), got.title.as_str()),
        ("Bob", "Fix it", "Bob")
    );
}

// --- v02_test.go

#[test]
fn read_tail_permission_mode() {
    let user = r#"{"type":"user","timestamp":"2026-10-26T10:00:00Z","permissionMode":"acceptEdits","message":{"content":"x"}}"#;
    let side = r#"{"type":"user","isSidechain":true,"permissionMode":"bypassPermissions","message":{"content":"x"}}"#;
    let sys = r#"{"type":"system","permissionMode":"bypassPermissions"}"#;
    let hi = assistant_row("2026-10-26T10:00:00Z", "hi");
    let cases: Vec<(&str, String, &str)> = vec![
        ("none", hi.clone(), ""),
        (
            "newest wins",
            format!(
                "{}{hi}{}",
                mode_row("default"),
                mode_row("bypassPermissions")
            ),
            "bypassPermissions",
        ),
        (
            "back to plan",
            format!("{}{}", mode_row("bypassPermissions"), mode_row("plan")),
            "plan",
        ),
        (
            "user row",
            format!("{}{user}\n", mode_row("default")),
            "acceptEdits",
        ),
        (
            "sidechain ignored",
            format!("{}{side}\n", mode_row("default")),
            "default",
        ),
        (
            "other types ignored",
            format!("{}{sys}\n", mode_row("default")),
            "default",
        ),
        (
            "empty ignored",
            format!("{}{}", mode_row("default"), mode_row("")),
            "default",
        ),
    ];
    for (name, src, want) in cases {
        assert_eq!(tail_of(&src, 1 << 20).permission_mode, want, "{name}");
    }
}

/// A session that starts in default mode and switches to bypassPermissions:
/// the switch is an Updated, and the mode survives once its row scrolls out.
#[test]
fn permission_mode_change_emits_updated() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let cwd = r"C:\work\modes";
    let id = "0b1c2d3e-0000-4000-8000-0000000000aa";
    write_session(
        dir,
        json!({"pid": 42, "sessionId": id, "cwd": cwd, "status": "busy"}),
    );
    let tp = dir
        .join("projects")
        .join(slug(cwd))
        .join(format!("{id}.jsonl"));
    append_file(
        &tp,
        &format!(
            "{}{}",
            mode_row("default"),
            assistant_row("2026-10-26T10:00:00Z", "one")
        ),
    );

    let (mut o, _) = test_opts(dir, true);
    o.tail_bytes = 400;
    let w = Watcher::new(o);
    let sub = w.subscribe();
    w.rescan();
    let cs = drain(&sub);
    assert!(
        cs.len() == 1 && cs[0].kind == ChangeKind::Added && cs[0].card.permission_mode == "default",
        "first scan: {cs:?}"
    );

    // The mode switch alone: the row has no timestamp, nothing else moves.
    append_file(&tp, &mode_row("bypassPermissions"));
    w.rescan();
    let cs = drain(&sub);
    assert!(
        cs.len() == 1
            && cs[0].kind == ChangeKind::Updated
            && cs[0].card.permission_mode == "bypassPermissions",
        "after switch: {cs:?}"
    );
    assert_eq!(cs[0].card.last_text, "one", "switch lost other fields");

    // Push the mode row out of the 400-byte window; the cached mode stays.
    for i in 0..6 {
        append_file(
            &tp,
            &assistant_row(&format!("2026-10-26T10:0{}:00Z", i + 1), &"x".repeat(80)),
        );
    }
    let raw = std::fs::read_to_string(&tp).unwrap();
    assert_eq!(
        tail_of(&raw, 400).permission_mode,
        "",
        "test setup: window still holds a mode row"
    );
    w.rescan();
    let cards = w.snapshot();
    assert!(
        cards.len() == 1 && cards[0].permission_mode == "bypassPermissions",
        "mode lost after scrolling out: {cards:?}"
    );
    for c in drain(&sub) {
        assert_eq!(
            c.card.permission_mode, "bypassPermissions",
            "change with lost mode"
        );
    }
}

/// RepoKey is resolved once per cwd, not on every scan or for every session.
#[test]
fn repo_key_resolved_once_per_cwd() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let calls: Arc<Mutex<HashMap<String, usize>>> = Arc::default();
    let c2 = calls.clone();
    let resolve = move |cwd: &str| -> String {
        *c2.lock().unwrap().entry(cwd.to_string()).or_default() += 1;
        let base = Path::new(cwd)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if base == "outside" {
            String::new()
        } else {
            format!("key:{base}")
        }
    };
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let repo_a = s(&dir.join("a"));
    let outside = s(&dir.join("outside"));
    write_session(dir, json!({"pid": 1, "sessionId": "s1", "cwd": repo_a}));
    write_session(dir, json!({"pid": 2, "sessionId": "s2", "cwd": repo_a}));
    write_session(dir, json!({"pid": 3, "sessionId": "s3", "cwd": outside}));

    let (mut o, _) = test_opts(dir, true);
    o.repo_key = Some(Arc::new(resolve));
    let w = Watcher::new(o);
    let sub = w.subscribe();
    for _ in 0..3 {
        w.rescan();
    }
    let n = |k: &str| calls.lock().unwrap().get(k).copied().unwrap_or(0);
    assert!(
        n(&repo_a) == 1 && n(&outside) == 1,
        "resolver calls: {:?}",
        calls.lock().unwrap()
    );
    let by_id: HashMap<String, AgentCard> = w
        .snapshot()
        .into_iter()
        .map(|c| (c.session_id.clone(), c))
        .collect();
    assert_eq!(by_id["s1"].repo_key, "key:a");
    assert_eq!(by_id["s2"].repo_key, "key:a");
    assert_eq!(by_id["s3"].repo_key, "");
    drain(&sub);

    // s3 moves into a repo: new cwd, new key, one Updated.
    let repo_b = s(&dir.join("b"));
    write_session(dir, json!({"pid": 3, "sessionId": "s3", "cwd": repo_b}));
    w.rescan();
    let cs = drain(&sub);
    assert!(
        cs.len() == 1 && cs[0].kind == ChangeKind::Updated && cs[0].card.repo_key == "key:b",
        "cwd move: {cs:?}"
    );
    assert_eq!(n(&repo_b), 1);
    // A cwd no live session uses any more is forgotten.
    assert!(
        !w.repo_cached(&outside),
        "cache kept a cwd with no live session"
    );
}

#[test]
fn repo_key_nil_resolver() {
    let t = tempfile::tempdir().unwrap();
    write_session(
        t.path(),
        json!({"pid": 1, "sessionId": "s1", "cwd": t.path().to_string_lossy()}),
    );
    let (w, _) = test_watcher(t.path(), true);
    w.rescan();
    let cards = w.snapshot();
    assert!(
        cards.len() == 1 && cards[0].repo_key.is_empty(),
        "{cards:?}"
    );
}

#[test]
fn changed_sees_v02_fields() {
    let base = AgentCard {
        key: "d/s".into(),
        ..AgentCard::default()
    };
    let mut b = base.clone();
    b.repo_key = "k".into();
    assert!(changed(&base, &b), "repoKey change ignored");
    let mut b = base.clone();
    b.permission_mode = "bypassPermissions".into();
    assert!(changed(&base, &b), "permissionMode change ignored");
}

/// Cold start: a fresh watcher whose tail window holds no mode row still
/// finds the newest one further back, and records that it looked.
#[test]
fn cold_start_finds_mode_beyond_tail() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let cwd = r"C:\work\cold";
    let id = "0b1c2d3e-0000-4000-8000-0000000000bb";
    write_session(
        dir,
        json!({"pid": 42, "sessionId": id, "cwd": cwd, "status": "busy"}),
    );
    let tp = dir
        .join("projects")
        .join(slug(cwd))
        .join(format!("{id}.jsonl"));
    append_file(
        &tp,
        &format!(
            "{}{}\n",
            mode_row("default"),
            r#"{"type":"user","timestamp":"2026-10-26T09:59:00Z","permissionMode":"bypassPermissions","message":{"content":"go"}}"#
        ),
    );
    for i in 0..6 {
        append_file(
            &tp,
            &assistant_row(&format!("2026-10-26T10:0{i}:00Z"), &"x".repeat(80)),
        );
    }
    let raw = std::fs::read_to_string(&tp).unwrap();
    assert_eq!(tail_of(&raw, 400).permission_mode, "", "test setup");

    let (mut o, _) = test_opts(dir, true);
    o.tail_bytes = 400;
    let w = Watcher::new(o);
    w.rescan();
    let cards = w.snapshot();
    assert!(
        cards.len() == 1 && cards[0].permission_mode == "bypassPermissions",
        "cold start mode: {cards:?}"
    );
    assert_eq!(w.mode_scanned(id), Some(true), "scan not recorded");
}

fn mode_of(src: &str, n: u64) -> String {
    scan_mode(&mut Cursor::new(src.as_bytes()), src.len() as u64, n)
}

/// The backward scan is bounded and skips the partial first line.
#[test]
fn scan_mode_cases() {
    let far = mode_row("acceptEdits");
    let pad = assistant_row("2026-10-26T10:00:00Z", &"y".repeat(200));
    let body = format!("{far}{pad}");
    assert_eq!(
        mode_of(&body, body.len() as u64),
        "acceptEdits",
        "whole file"
    );
    assert_eq!(
        mode_of(&body, pad.len() as u64),
        "",
        "row outside bound found"
    );
    // A window opening exactly on the mode row keeps it.
    let edge = format!("{pad}{body}");
    assert_eq!(
        mode_of(&edge, body.len() as u64),
        "acceptEdits",
        "row on boundary"
    );
    let cut = format!("{pad}{}{pad}", &far[3..]);
    assert_eq!(
        mode_of(&cut, (cut.len() - pad.len() - 2) as u64),
        "",
        "partial line parsed"
    );
    let newest = format!(
        "{}{far}{}\n{pad}",
        mode_row("default"),
        r#"{"type":"user","isSidechain":true,"permissionMode":"bypassPermissions"}"#
    );
    assert_eq!(
        mode_of(&newest, 1 << 20),
        "acceptEdits",
        "newest non-sidechain"
    );
}

// --- lstart_test.go

#[test]
fn parse_lstart_table() {
    let utc = |s: &str| GoTime::parse_rfc3339(s).unwrap();
    let cases: Vec<(&str, Option<GoTime>)> = vec![
        (
            "Mon Oct 27 14:29:03 2026",
            Some(utc("2026-10-27T14:29:03Z")),
        ),
        (
            "Wed Oct  7 09:05:00 2026",
            Some(utc("2026-10-07T09:05:00Z")),
        ),
        (
            "  Wed Oct  7 09:05:00 2026\n",
            Some(utc("2026-10-07T09:05:00Z")),
        ),
        ("134375320001234567", None),
        ("", None),
    ];
    for (input, want) in cases {
        let got = parse_lstart(input);
        assert_eq!(
            got.map(|t| (t.secs, t.nanos)),
            want.map(|t| (t.secs, t.nanos)),
            "{input:?}"
        );
    }
    let s =
        read_session(&br#"{"pid":7,"sessionId":"a","procStart":"Mon Oct 27 14:29:03 2026"}"#[..])
            .unwrap();
    assert_eq!(s.proc_start, utc("2026-10-27T14:29:03Z").secs as u64);
}

// --- live_windows_test.go

/// A live pid whose procStart doesn't match is a reused pid, not the session.
/// (The units differ: FILETIME ticks on Windows, Unix seconds on macOS.)
#[cfg(any(windows, target_os = "macos"))]
#[test]
fn reused_pid_not_listed() {
    use presence::internal::{PROC_START_SLACK, process_start_ticks};
    let me = i64::from(std::process::id());
    let ticks = process_start_ticks(me).expect("no start time for our own process");
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    write_session(
        dir,
        json!({"pid": me, "sessionId": "reused", "cwd": dir.to_string_lossy(),
            "procStart": (ticks - 5 * PROC_START_SLACK).to_string()}),
    );
    let (w, _) = test_watcher(dir, false);
    w.rescan();
    assert!(w.snapshot().is_empty(), "reused pid listed");

    write_session(
        dir,
        json!({"pid": me, "sessionId": "mine", "cwd": dir.to_string_lossy(),
            "procStart": (ticks + PROC_START_SLACK / 2).to_string()}),
    );
    w.rescan();
    let cards = w.snapshot();
    assert!(
        cards.len() == 1 && cards[0].session_id == "mine",
        "{cards:?}"
    );
}

// --- exit_test.go

/// The child process for exit_removes_at_once.
#[test]
fn helper_sleeper() {
    if std::env::var("IRORI_PRESENCE_SLEEPER").as_deref() != Ok("1") {
        return; // helper process only
    }
    std::thread::sleep(Duration::from_secs(60));
}

/// An agent killed without removing its session file leaves the roster
/// when its process ends, not at the next safety rescan.
#[cfg(any(windows, target_os = "macos"))]
#[tokio::test(flavor = "multi_thread")]
async fn exit_removes_at_once() {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["presence::helper_sleeper", "--exact", "--nocapture"])
        .env("IRORI_PRESENCE_SLEEPER", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    write_session(
        dir,
        json!({"pid": child.id(), "sessionId": "killed", "cwd": dir.to_string_lossy()}),
    );
    let (mut o, _) = test_opts(dir, false);
    o.interval = Duration::from_secs(3600);
    o.safety_interval = Duration::from_secs(3600);
    let w = Watcher::new(o);
    let sub = w.subscribe();
    let r = run_watcher(&w);
    await_change(&sub, "Added", is(ChangeKind::Added, "killed")).await;
    child.kill().unwrap();
    let _ = child.wait();
    await_change(&sub, "Removed", is(ChangeKind::Removed, "killed")).await;
    r.stop().await;
}

#[test]
fn last_tool_is_the_tool_running_now() {
    // card #299: the newest assistant row ends in a tool_use, no result yet.
    let a = |content: &str| {
        format!(
            "{{\"type\":\"assistant\",\"timestamp\":\"2026-11-03T05:00:00Z\",\"message\":{{\"content\":[{content}]}}}}\n"
        )
    };
    let user = "{\"type\":\"user\",\"timestamp\":\"2026-11-03T05:00:01Z\",\"message\":{\"content\":[{\"type\":\"tool_result\"}]}}\n";
    let tool = a(r#"{"type":"text","text":"Looking."},{"type":"tool_use","name":"Bash"}"#);
    let n = 1 << 16;
    assert_eq!(tail_of(&tool, n).last_tool, "Bash", "running");
    assert_eq!(
        tail_of(&format!("{tool}{user}"), n).last_tool,
        "",
        "its result came back"
    );
    let prose = a(r#"{"type":"tool_use","name":"Read"},{"type":"text","text":"Done."}"#);
    assert_eq!(tail_of(&prose, n).last_tool, "", "ends in prose");
    let side = "{\"type\":\"assistant\",\"isSidechain\":true,\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Grep\"}]}}\n";
    assert_eq!(
        tail_of(&format!("{tool}{side}"), n).last_tool,
        "Bash",
        "a subagent's tool isn't the session's"
    );
}

/// Card #433: a session file that is still there but can't be read (a
/// truncated write on a full disk) keeps its session while the process
/// lives; a dead process or a deleted file still means gone.
#[test]
fn unreadable_session_file_keeps_a_live_session() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let alive = Arc::new(AtomicBool::new(true));
    let (mut o, _) = test_opts(dir, false);
    let a = alive.clone();
    o.live = Some(Arc::new(move |pid, _| {
        pid != 4243 || a.load(Ordering::SeqCst)
    }));
    let w = Watcher::new(o);
    let p = write_session(
        dir,
        json!({"pid": 4243, "sessionId": "gudgeon", "cwd": "/w", "status": "idle"}),
    );
    w.rescan();
    let sub = w.subscribe();
    let listed = |w: &Watcher| w.snapshot().iter().any(|c| c.session_id == "gudgeon");
    assert!(listed(&w), "never listed");

    for bad in [&b"{\"pid\":"[..], b"", b"\0\0\0\0"] {
        std::fs::write(&p, bad).unwrap();
        w.rescan();
        assert!(listed(&w), "{bad:?}: dropped while the process lives");
    }
    assert!(
        drain(&sub).iter().all(|c| c.kind != ChangeKind::Removed),
        "an unreadable file read as gone"
    );

    // The file reads again: its new status is an update, not a new card.
    write_session(
        dir,
        json!({"pid": 4243, "sessionId": "gudgeon", "cwd": "/w", "status": "busy"}),
    );
    w.rescan();
    let got = drain(&sub);
    assert!(
        got.len() == 1 && got[0].kind == ChangeKind::Updated && got[0].card.status == "busy",
        "{got:?}"
    );

    // Unreadable and the process ended: gone.
    std::fs::write(&p, b"{").unwrap();
    alive.store(false, Ordering::SeqCst);
    w.rescan();
    assert!(!listed(&w), "a dead process stayed listed");
    let got = drain(&sub);
    assert!(
        got.len() == 1 && got[0].kind == ChangeKind::Removed,
        "{got:?}"
    );
    // Its cache went with it: alive again, still unreadable, still gone.
    alive.store(true, Ordering::SeqCst);
    w.rescan();
    assert!(!listed(&w), "came back from a stale cache");

    // A deleted file is gone even while the process lives.
    write_session(
        dir,
        json!({"pid": 4243, "sessionId": "gudgeon", "cwd": "/w", "status": "idle"}),
    );
    w.rescan();
    assert!(listed(&w));
    std::fs::remove_file(&p).unwrap();
    w.rescan();
    assert!(!listed(&w), "a deleted session file stayed listed");
}

/// Card #783: the window assistant's headless `claude -p` writes a session
/// file in its run dir; that folder (and any under it) is left off the
/// roster, every other session listed.
#[test]
fn skip_cwd_leaves_the_assistant_off_the_roster() {
    let td = tempfile::tempdir().unwrap();
    let dir = td.path();
    let run = dir.join("data").join("assistant");
    let sep = std::path::MAIN_SEPARATOR;
    for (pid, id, cwd) in [
        (1, "assistant-session", run.to_string_lossy().into_owned()),
        (
            2,
            "assistant-sub",
            format!("{}{sep}x", run.to_string_lossy()),
        ),
        (
            3,
            "assistant-slash",
            format!("{}{sep}", run.to_string_lossy()),
        ),
        (4, "neighbour", format!("{}-other", run.to_string_lossy())),
        (5, "agent", dir.join("repo").to_string_lossy().into_owned()),
    ] {
        write_session(dir, json!({"pid": pid, "sessionId": id, "cwd": cwd}));
    }
    let (mut opts, _) = test_opts(dir, true);
    let skip = if cfg!(windows) {
        // Either slash, any case.
        run.to_string_lossy().replace('\\', "/").to_uppercase()
    } else {
        run.to_string_lossy().into_owned()
    };
    opts.skip_cwd = vec![skip];
    let w = Watcher::new(opts);
    w.rescan();
    let mut ids: Vec<String> = w.snapshot().into_iter().map(|c| c.session_id).collect();
    ids.sort();
    assert_eq!(ids, vec!["agent".to_string(), "neighbour".to_string()]);
}

// ---- context measure (agent-continuity §4, A.1) ---------------------------

fn usage_row(ts: &str, model: &str, sidechain: bool, usage: serde_json::Value) -> String {
    format!(
        "{}\n",
        json!({"type": "assistant", "timestamp": ts, "isSidechain": sidechain,
               "message": {"model": model, "content": [{"type": "text", "text": "x"}], "usage": usage}})
    )
}

fn u(input: i64, read: i64, create: i64) -> serde_json::Value {
    json!({"input_tokens": input, "cache_read_input_tokens": read,
           "cache_creation_input_tokens": create, "output_tokens": 999})
}

#[test]
fn ctx_plain_row_sums_input_and_caches() {
    let src = usage_row("2026-10-26T10:00:00Z", "claude-x", false, u(1, 1000, 200))
        + &usage_row("2026-10-26T10:01:00Z", "claude-x", false, u(3, 5000, 400));
    let t = tail_of(&src, 1 << 20);
    assert_eq!(t.ctx, 5403);
    assert_eq!(
        t.ctx_at,
        GoTime::parse_rfc3339("2026-10-26T10:01:00Z").unwrap()
    );
    assert!(!t.compacted);
}

#[test]
fn ctx_advisor_row_reads_the_last_message_iteration() {
    let mut usage = u(10, 20000, 0); // top-level adds up every iteration
    usage["iterations"] = json!([
        {"type": "message", "input_tokens": 2, "cache_read_input_tokens": 9000, "cache_creation_input_tokens": 100},
        {"type": "advisor_message", "input_tokens": 50000, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0},
        {"type": "message", "input_tokens": 3, "cache_read_input_tokens": 9500, "cache_creation_input_tokens": 200}
    ]);
    let t = tail_of(
        &usage_row("2026-10-26T10:00:00Z", "claude-x", false, usage),
        1 << 20,
    );
    assert_eq!(t.ctx, 9703);
}

#[test]
fn ctx_ignores_sidechain_and_synthetic_rows() {
    let src = usage_row("2026-10-26T10:00:00Z", "claude-x", false, u(1, 7000, 0))
        + &usage_row("2026-10-26T10:01:00Z", "claude-x", true, u(1, 90000, 0))
        + &usage_row("2026-10-26T10:02:00Z", "<synthetic>", false, u(0, 0, 0));
    let t = tail_of(&src, 1 << 20);
    assert_eq!(t.ctx, 7001);
    assert_eq!(
        t.ctx_at,
        GoTime::parse_rfc3339("2026-10-26T10:00:00Z").unwrap()
    );
}

#[test]
fn ctx_compaction_is_marked() {
    let src = usage_row("2026-10-26T10:00:00Z", "claude-x", false, u(1, 190000, 0))
        + "{\"type\":\"system\",\"subtype\":\"compact_boundary\",\"timestamp\":\"2026-10-26T10:05:00Z\",\"compactMetadata\":{\"preTokens\":190001}}\n";
    let t = tail_of(&src, 1 << 20);
    assert!(t.compacted);
    assert_eq!(t.ctx, 190001); // size unknown until the next call
    let src = src + &usage_row("2026-10-26T10:06:00Z", "claude-x", false, u(1, 9000, 0));
    let t = tail_of(&src, 1 << 20);
    assert!(t.compacted, "compacted must not clear when the size drops");
    assert_eq!(t.ctx, 9001);
}

#[test]
fn ctx_survives_a_huge_tool_result_with_no_assistant_row_in_the_tail() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let cwd = r"C:\work\ctx";
    let id = "0b1c2d3e-0000-4000-8000-0000000000c1";
    write_session(
        dir,
        json!({"pid": 43, "sessionId": id, "cwd": cwd, "status": "busy"}),
    );
    let tp = dir
        .join("projects")
        .join(slug(cwd))
        .join(format!("{id}.jsonl"));
    append_file(
        &tp,
        &usage_row("2026-10-26T10:00:00Z", "claude-x", false, u(1, 8000, 0)),
    );
    let (mut o, _) = test_opts(dir, true);
    o.tail_bytes = 400;
    let w = Watcher::new(o);
    let sub = w.subscribe();
    w.rescan();
    let cs = drain(&sub);
    assert_eq!(cs.len(), 1, "{cs:?}");
    assert_eq!(cs[0].card.ctx, 8001);
    let at = cs[0].card.ctx_at;
    assert!(!at.is_zero());

    // A tool result far bigger than the window: no assistant row in the tail.
    let big = format!(
        "{}\n",
        json!({"type": "user", "timestamp": "2026-10-26T10:00:30Z",
               "message": {"content": [{"type": "tool_result", "content": "z".repeat(5000)}]}})
    );
    append_file(&tp, &big);
    w.rescan();
    let card = w.snapshot().into_iter().next().unwrap();
    assert_eq!((card.ctx, card.ctx_at), (8001, at));
}

#[test]
fn a_growing_transcript_changes_no_card_for_ctx_alone() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let cwd = r"C:\work\ctx2";
    let id = "0b1c2d3e-0000-4000-8000-0000000000c2";
    write_session(
        dir,
        json!({"pid": 44, "sessionId": id, "cwd": cwd, "status": "busy"}),
    );
    let tp = dir
        .join("projects")
        .join(slug(cwd))
        .join(format!("{id}.jsonl"));
    append_file(
        &tp,
        &usage_row("2026-10-26T10:00:00Z", "claude-x", false, u(1, 8000, 0)),
    );
    let (w, _) = test_watcher(dir, true);
    let sub = w.subscribe();
    w.rescan();
    assert_eq!(drain(&sub).len(), 1);
    // Same text, same second: only the token count moves.
    append_file(
        &tp,
        &usage_row("2026-10-26T10:00:00Z", "claude-x", false, u(1, 8500, 0)),
    );
    w.rescan();
    assert!(
        drain(&sub).is_empty(),
        "a ctx move alone published a change"
    );
    assert_eq!(w.snapshot().into_iter().next().unwrap().ctx, 8501);

    let mut a = AgentCard::default();
    let mut b = a.clone();
    b.ctx = 5;
    b.compacted = true;
    a.key = "k".into();
    b.key = "k".into();
    assert!(!changed(&a, &b));
}
