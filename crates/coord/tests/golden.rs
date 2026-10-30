//! Row-dump replay goldens (coord.md §6, architecture.md §6.2): the rows
//! the Go Board wrote in each scenario of tools/goldens/coord_golden_test.go,
//! folded by `Coord::from_rows`, must give the views Go gave, byte for byte
//! as Go marshals them.

use std::sync::Arc;

use base64::Engine as _;
use coord::{BoardFilter, Coord, Options, TaskFilter};
use gocompat::time::GoTime;
use serde_json::Value;
use store::Event;

fn golden() -> Vec<Value> {
    let p = manifest_dir().join("../../testdata/golden/coord/replay.json");
    let raw = std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    serde_json::from_slice::<Value>(&raw)
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn rows(c: &Value) -> Vec<Event> {
    c["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| Event {
            device_id: s(&r["deviceId"]).into(),
            seq: r["seq"].as_i64().unwrap(),
            hlc: hlc::Timestamp(r["hlc"].as_u64().unwrap()),
            actor: s(&r["actor"]).into(),
            kind: s(&r["kind"]).into(),
            payload: base64::engine::general_purpose::STANDARD
                .decode(s(&r["payload"]))
                .unwrap(),
            sig: Vec::new(),
        })
        .collect()
}

fn coord_of(c: &Value) -> Coord {
    let now = GoTime::parse_rfc3339(s(&c["now"])).unwrap();
    let mut o = Options {
        now: Some(Arc::new(move || now)),
        board: c["board"].as_bool().unwrap_or(false),
        person: s(&c["person"]).into(),
        ..Options::default()
    };
    if let Some(owners) = c["owners"].as_array() {
        let owners: Vec<String> = owners.iter().map(|v| s(v).to_string()).collect();
        o.is_owner = Some(Arc::new(move |k: &str| owners.iter().any(|x| x == k)));
    }
    if o.board {
        o.tag = Some(Arc::new(|| "mlg".to_string()));
    }
    Coord::from_rows(s(&c["device"]), &rows(c), None, o)
}

fn json<T: serde::Serialize>(v: &T) -> String {
    String::from_utf8(gocompat::json::marshal(v).unwrap()).unwrap()
}

/// The first byte where two strings part, with some context.
fn diff(want: &str, got: &str) -> String {
    let i = want
        .bytes()
        .zip(got.bytes())
        .position(|(a, b)| a != b)
        .unwrap_or(want.len().min(got.len()));
    let lo = i.saturating_sub(120);
    let cut = |x: &str| {
        let mut a = lo.min(x.len());
        while !x.is_char_boundary(a) {
            a -= 1;
        }
        let mut b = (i + 200).min(x.len());
        while !x.is_char_boundary(b) {
            b += 1;
        }
        x[a..b].to_string()
    };
    format!("at byte {i}\n want: {}\n  got: {}", cut(want), cut(got))
}

fn check(case: &str, what: &str, want: &str, got: &str, fails: &mut Vec<String>) {
    assert!(
        want.starts_with('[') || want.starts_with('{'),
        "{case} {what}: no golden"
    );
    if want != got {
        fails.push(format!("{case} {what}: {}", diff(want, got)));
    }
}

#[test]
fn replay_matches_go() {
    let cases = golden();
    assert!(cases.len() >= 9, "golden has {} cases", cases.len());
    let mut fails = Vec::new();
    for c in &cases {
        let name = s(&c["name"]);
        let co = coord_of(c);
        let tasks = co.list_tasks(&TaskFilter::default());
        check(name, "tasks", s(&c["tasks"]), &json(&tasks), &mut fails);
        check(
            name,
            "leases",
            s(&c["leases"]),
            &json(&co.leases()),
            &mut fails,
        );
        check(
            name,
            "refusals",
            s(&c["refusals"]),
            &json(&co.refusals()),
            &mut fails,
        );
        check(
            name,
            "board",
            s(&c["boardAll"]),
            &json(&co.board(&BoardFilter::default())),
            &mut fails,
        );
        let for_alice = BoardFilter {
            viewer: "dev-a/alice-session".into(),
            ..BoardFilter::default()
        };
        check(
            name,
            "board for alice",
            s(&c["boardFor"]),
            &json(&co.board(&for_alice)),
            &mut fails,
        );
        for t in &tasks {
            let act = co.activity(&t.id).unwrap_or_default();
            check(
                name,
                &format!("activity {}", t.short_ref()),
                s(&c["activity"][&t.id]),
                &json(&act),
                &mut fails,
            );
            let cms = co.comments(&t.id).unwrap_or_default();
            check(
                name,
                &format!("comments {}", t.short_ref()),
                s(&c["comments"][&t.id]),
                &json(&cms),
                &mut fails,
            );
        }
    }
    assert!(
        fails.is_empty(),
        "{} mismatches:\n{}",
        fails.len(),
        fails.join("\n\n")
    );
    // The goldens carry real content, not empty views.
    let tasks: usize = cases.iter().map(|c| s(&c["tasks"]).len()).sum();
    assert!(tasks > 10_000, "only {tasks} bytes of tasks compared");
}

/// The same rows folded one at a time as live rows (not a replay) end in
/// the same tasks: the fold is designed to converge (coord.md §7 trap 4),
/// apart from what is in-memory only.
#[test]
fn live_fold_matches_replay() {
    for c in &golden() {
        let name = s(&c["name"]);
        let replay = coord_of(c);
        let live = coord_of_live(c);
        let a = json(&replay.list_tasks(&TaskFilter::default()));
        let b = json(&live.list_tasks(&TaskFilter::default()));
        assert!(a == b, "{name}: {}", diff(&a, &b));
    }
}

fn coord_of_live(c: &Value) -> Coord {
    let now = GoTime::parse_rfc3339(s(&c["now"])).unwrap();
    let mut o = Options {
        now: Some(Arc::new(move || now)),
        board: c["board"].as_bool().unwrap_or(false),
        person: s(&c["person"]).into(),
        ..Options::default()
    };
    if let Some(owners) = c["owners"].as_array() {
        let owners: Vec<String> = owners.iter().map(|v| s(v).to_string()).collect();
        o.is_owner = Some(Arc::new(move |k: &str| owners.iter().any(|x| x == k)));
    }
    if o.board {
        o.tag = Some(Arc::new(|| "mlg".to_string()));
    }
    let co = Coord::from_rows(s(&c["device"]), &[], None, o);
    co.fold_live(&rows(c));
    co
}

mod common;

/// Notice texts (coord.md §3.18) from the script of
/// TestGoldenCoordNotices: the same calls here, every 26-character id
/// replaced on both sides, each step's notices compared as a sorted set.
#[test]
fn notice_texts_match_go() {
    use common::*;
    use coord::roster::Agents as _;
    use coord::*;
    let p = manifest_dir().join("../../testdata/golden/coord/notices.json");
    let want: Vec<Value> = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    let ids = regex::Regex::new("[0-9A-HJKMNP-TV-Z]{26}").unwrap();
    let norm = |step: &str, kind: &str, to: &str, wake: bool, text: &str| {
        format!(
            "{step}|{kind}|{to}|{wake}|{}",
            ids.replace_all(text, "<ID>")
        )
    };
    let mut got: Vec<String> = Vec::new();
    let mut add = |step: &str, ns: &[Notice]| {
        for n in ns.iter().filter(|n| !n.kind.is_empty()) {
            got.push(norm(step, &n.kind, &n.to.join(","), n.wake, &n.text));
        }
    };
    let e = Env::new();
    let t1 =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "Fix \"it\" <now>".into(),
                objective: "obj & more".into(),
                done_criteria: "dc".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    add("assign", &[e.c.assign(ALICE, &t1.id, BOB).unwrap().1]);
    e.c.claim_task(BOB, &t1.id).unwrap();
    e.c.block(BOB, &t1.id, "the db", None).unwrap();
    add("unblock", &[e.c.unblock(USER, &t1.id, "back").unwrap().1]);
    add(
        "comment",
        &[e.c.add_comment(CAROL, &t1.id, "hi <b>", false).unwrap().1],
    );
    e.c.raise(
        BOB,
        &t1.id,
        NewBlocker {
            question: "Which?".into(),
            options: vec![
                BlockerOption {
                    answer: "A".into(),
                    recommended: true,
                    ..BlockerOption::default()
                },
                BlockerOption {
                    answer: "B".into(),
                    ..BlockerOption::default()
                },
            ],
            ..NewBlocker::default()
        },
    )
    .unwrap();
    add(
        "answer",
        &[e.c
            .answer(
                USER,
                &t1.id,
                "b2",
                BlockerAnswer {
                    pick: 1,
                    ..BlockerAnswer::default()
                },
            )
            .unwrap()
            .1],
    );
    add(
        "resolve",
        &[e.c
            .answer(
                BOB,
                &t1.id,
                "b2",
                BlockerAnswer {
                    resolve: true,
                    text: "ok".into(),
                    ..BlockerAnswer::default()
                },
            )
            .unwrap()
            .1],
    );
    e.c.set_task_state(BOB, &t1.id, COMPLETED, "").unwrap();
    add("reopen", &[e.c.reopen(USER, &t1.id).unwrap().1]);
    add(
        "person comment",
        &e.c.person_comment(USER, &t1.id, "more?", true, None)
            .unwrap()
            .1,
    );
    e.c.watch(CAROL, &t1.id).unwrap();
    e.c.set_steps(
        BOB,
        &t1.id,
        vec![Step {
            text: "x".into(),
            ..Step::default()
        }],
    )
    .unwrap();
    add("watch digest", &e.c.sweep_notices().unwrap());
    let (ask, n) =
        e.c.post_ask(
            USER,
            NewAsk {
                text: "Look at this\nplease".into(),
                wake: true,
                ..NewAsk::default()
            },
            &e.ros.agents(),
        )
        .unwrap();
    add("ask", &[n]);
    e.c.ask_claim(ALICE, &ask.id, std::time::Duration::from_secs(1))
        .unwrap();
    e.ros.set(ALICE, "");
    add("ask sweep 1", &e.c.sweep_asks(&e.ros.agents()).unwrap());
    e.clk.add(GONE_GRACE + secs(1));
    add("ask sweep 2", &e.c.sweep_asks(&e.ros.agents()).unwrap());
    let t3 =
        e.c.create_task(
            BOB,
            NewTask {
                title: "Left behind".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(CAROL, &t3.id).unwrap();
    e.ros.set(CAROL, "");
    add("returned 1", &e.c.sweep_notices().unwrap());
    e.clk.add(HOLDER_GRACE + secs(1));
    add("returned 2", &e.c.sweep_notices().unwrap());
    let mut want: Vec<String> = want
        .iter()
        .map(|w| {
            norm(
                s(&w["step"]),
                s(&w["kind"]),
                s(&w["to"]),
                w["wake"].as_bool().unwrap(),
                s(&w["text"]),
            )
        })
        .collect();
    assert!(want.len() >= 10, "{} notices in the golden", want.len());
    want.sort();
    got.sort();
    let missing: Vec<&String> = want.iter().filter(|w| !got.contains(w)).collect();
    let extra: Vec<&String> = got.iter().filter(|g| !want.contains(g)).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "missing (Go has):\n{missing:#?}\nextra (Rust has):\n{extra:#?}"
    );
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
