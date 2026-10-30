//! Port of I:internal/coord/short_test.go.
#![allow(clippy::type_complexity)]

mod common;

use std::collections::BTreeMap;

use common::*;
use coord::rows::TaskCreatedRow;
use coord::*;
use store::{Event, kinds};

fn new_task(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

#[test]
fn test_short_refs_count_up_and_resolve() {
    let e = Env::new();
    let a = e.c.create_task(ALICE, new_task("one")).unwrap();
    let b = e.c.create_task(ALICE, new_task("two")).unwrap();
    assert_eq!(
        (
            a.short.as_str(),
            b.short.as_str(),
            a.short_ref().as_str(),
            b.short_was.as_str()
        ),
        ("1", "2", "#1", "")
    );
    for input in [
        "#2",
        "2",
        " #2 ",
        "task:#2",
        "TASK:2",
        &b.id,
        &format!("task:{}", b.id),
    ] {
        assert_eq!(
            e.c.task_ref(input).as_deref(),
            Ok(b.id.as_str()),
            "{input:?}"
        );
    }
    want_err(e.c.task_ref("#9"), ErrorKind::NotFound);
    for input in ["#0", "#01", "#x", "12-b", "#1234567890123"] {
        assert!(parse_short(input).is_none(), "{input:?}");
    }
    let c2 = e.reopen();
    assert_eq!(c2.get_task(&b.id).unwrap().short, "2");
    assert_eq!(c2.create_task(ALICE, new_task("three")).unwrap().short, "3");
    // show finds a card by its short ref, and search docs carry it.
    let sh = search::Source::search_show(&c2, "task:#2", &search::ShowOptions::default()).unwrap();
    assert!(
        sh.r#ref == format!("task:{}", b.id) && sh.text.contains("Task #2 "),
        "{sh:?}"
    );
}

fn created_row(dev: &str, seq: i64, at: u64, id: &str, num: i64) -> Event {
    let raw = gocompat::json::marshal(&TaskCreatedRow {
        task_id: id.into(),
        num,
        title: id.into(),
        by: format!("{dev}/s"),
        at: fixed_now(),
        ..TaskCreatedRow::default()
    })
    .unwrap();
    Event {
        device_id: dev.into(),
        seq,
        hlc: hlc::Timestamp(at),
        actor: format!("agent:{dev}/s"),
        kind: kinds::TASK_CREATED.into(),
        payload: raw,
        sig: Vec::new(),
    }
}

#[test]
fn test_short_ref_clashes_settle_in_any_order() {
    let base = hlc_at(fixed_now());
    let rows = [
        created_row("dev-b", 1, base + 10, "B1", 1),
        created_row("dev-b", 2, base + 20, "B2", 2),
        created_row("dev-c", 1, base + 15, "C1", 1),
        created_row("dev-c", 2, base + 20, "C2", 2),
        created_row("dev-c", 3, base + 30, "C3", 2),
        created_row("dev-b", 3, base + 40, "B3", 3),
    ];
    let want = [
        ("B1", "1a", "1"),
        ("C1", "1b", "1"),
        ("B2", "2a", "2"),
        ("C2", "2b", "2"),
        ("C3", "2c", "2"),
        ("B3", "3", ""),
    ];
    let check = |name: &str, c: &Coord| {
        for (id, short, was) in want {
            let x = c.get_task(id).unwrap();
            assert_eq!(
                (x.short.as_str(), x.short_was.as_str()),
                (short, was),
                "{name}: {id}"
            );
            assert_eq!(
                c.task_ref(&format!("#{short}")).as_deref(),
                Ok(id),
                "{name}"
            );
        }
        for (bare, ids) in [
            ("#1", vec!["B1", "C1"]),
            ("2", vec!["B2", "C2", "C3"]),
            ("task:#2", vec!["B2", "C2", "C3"]),
        ] {
            let err = c.task_ref(bare).unwrap_err();
            assert!(err.is(ErrorKind::Ambiguous), "{name}: {bare}: {err}");
            for id in ids {
                assert!(
                    err.to_string().contains(&format!("(task:{id})")),
                    "{name}: {bare}: {err}"
                );
            }
        }
    };
    let orders: BTreeMap<&str, [usize; 6]> = BTreeMap::from([
        ("in order", [0, 1, 2, 3, 4, 5]),
        ("b first", [0, 1, 5, 2, 3, 4]),
        ("c first", [2, 3, 4, 0, 1, 5]),
        ("mixed", [2, 3, 0, 4, 1, 5]),
    ]);
    for (name, order) in orders {
        let e = Env::new();
        let ch = e.c.subscribe();
        for i in order {
            e.c.handle(&rows[i]).unwrap();
        }
        check(name, &e.c);
        assert!(!ch.is_empty(), "{name}: no changes");
        assert_eq!(
            e.c.create_task(ALICE, new_task("after")).unwrap().short,
            "4",
            "{name}"
        );
    }
    let e = Env::new();
    for ev in &rows {
        e.st.insert(ev).unwrap();
    }
    check("replay", &e.reopen());
}

#[test]
fn test_short_refs_for_older_rows() {
    let e = Env::new();
    for (i, title) in ["old one", "old two"].iter().enumerate() {
        e.st.append(
            &format!("agent:{ALICE}"),
            kinds::TASK_CREATED,
            &TaskCreatedRow {
                task_id: format!("01OLDROW000000000000000000{i}"),
                title: title.to_string(),
                by: ALICE.into(),
                at: fixed_now(),
                ..TaskCreatedRow::default()
            },
        )
        .unwrap();
        e.clk.add(secs(1));
    }
    let c = e.reopen();
    let got: Vec<String> = c
        .list_tasks(&TaskFilter::default())
        .into_iter()
        .map(|t| t.short)
        .collect();
    assert_eq!(got, ["1", "2"]);
    assert_eq!(c.create_task(ALICE, new_task("new")).unwrap().short, "3");
}

#[test]
fn test_short_suffix() {
    for (i, want) in [
        (0, "a"),
        (1, "b"),
        (2, "c"),
        (25, "z"),
        (26, "aa"),
        (27, "ab"),
    ] {
        assert_eq!(short_suffix(i), want);
    }
}

#[test]
fn test_short_ref_clash_is_heard_and_refused() {
    let e = Env::new();
    let mine = e.c.create_task(ALICE, new_task("mine")).unwrap();
    assert_eq!(mine.short, "1");
    e.c.sweep_notices().unwrap();
    e.clk.add(mins(1));
    let since = e.clk.now();
    let base = hlc_at(fixed_now());
    e.c.handle(&created_row("dev-z", 1, base - (1 << 16), "Z1", 1))
        .unwrap();
    let x = e.c.get_task(&mine.id).unwrap();
    assert_eq!((x.short.as_str(), x.short_was.as_str()), ("1b", "1"));
    let err = e.c.task_ref("#1").unwrap_err();
    assert!(err.is(ErrorKind::Ambiguous));
    let msg = err.to_string();
    assert!(
        msg.contains("#1a \"Z1\"") && msg.contains("#1b \"mine\""),
        "{msg}"
    );
    let ns = e.c.sweep_notices().unwrap();
    assert!(
        ns.iter().any(|n| n.kind == NOTICE_WATCHED
            && n.task_id == mine.id
            && n.to.iter().any(|t| t == ALICE)
            && n.text.contains("renumbered it #1b (it was #1)")),
        "{ns:?}"
    );
    assert!(e.c.short_moved_since(&mine.id, &since));
    let docs = search::Source::search_docs(&e.c, since, 0).unwrap();
    assert!(
        docs.iter()
            .any(|d| d.r#ref == format!("task:{}", mine.id) && d.body.starts_with("#1b")),
        "search didn't re-read the moved card: {docs:?}"
    );
}

#[test]
fn test_short_refs_after_a_refold() {
    let e = Env::new();
    let mine = e.c.create_task(ALICE, new_task("mine")).unwrap();
    let base = hlc_at(fixed_now());
    e.c.handle(&created_row("dev-z", 1, base - (1 << 16), "Z1", 1))
        .unwrap();
    e.c.handle(&created_row("dev-z", 2, base + (1 << 16), "Z2", 2))
        .unwrap();
    assert_eq!(e.c.get_task(&mine.id).unwrap().short, "1b");
    e.c.sweep_notices().unwrap();
    e.clk.add(mins(1));
    let since = e.clk.now();
    let vec = e.st.vector().unwrap();
    e.c.refold_if_cut(&vec).unwrap();
    let x = e.c.get_task(&mine.id).unwrap();
    assert_eq!((x.short.as_str(), x.short_was.as_str()), ("1", ""));
    assert!(e.c.get_task("Z1").is_none());
    let ns = e.c.sweep_notices().unwrap();
    assert_eq!(ns.len(), 1, "{ns:?}");
    assert!(ns[0].text.contains("renumbered it #1 (it was #1b)"));
    assert!(e.c.short_moved_since(&mine.id, &since));
    assert_eq!(e.c.create_task(ALICE, new_task("next")).unwrap().short, "3");
}

/// A small deterministic generator (the Go test uses PCG(7, 11); the
/// property, not the sequence, is what is ported).
struct Rng(u64);
impl Rng {
    fn n(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

#[test]
fn test_short_refs_incremental_matches_replay() {
    let base = hlc_at(fixed_now());
    let mut rnd = Rng(0x9e37_79b9_7f4a_7c15);
    let devs = ["dev-b", "dev-c", "dev-d"];
    let mut per_dev: [Vec<Event>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut top = [0i64; 3];
    for i in 0..300u64 {
        let d = rnd.n(3);
        if rnd.n(4) == 0 {
            top[d] = top[0].max(top[1]).max(top[2]);
        }
        top[d] += 1;
        let mut at = base + ((i / 2) << 16) + rnd.n(3) as u64;
        if let Some(last) = per_dev[d].last() {
            at = at.max(last.hlc.0 + 1);
        }
        let seq = per_dev[d].len() as i64 + 1;
        let id = format!("{}-{seq}", devs[d]);
        per_dev[d].push(created_row(devs[d], seq, at, &id, top[d]));
    }
    let e = Env::new();
    let mut next = [0usize; 3];
    loop {
        let open: Vec<usize> = (0..3).filter(|&d| next[d] < per_dev[d].len()).collect();
        if open.is_empty() {
            break;
        }
        let d = open[rnd.n(open.len())];
        let ev = per_dev[d][next[d]].clone();
        next[d] += 1;
        e.c.handle(&ev).unwrap();
        e.st.insert(&ev).unwrap();
    }
    let re = e.reopen();
    let mut seen = BTreeMap::new();
    let mut clashed = 0;
    for x in re.list_tasks(&TaskFilter::default()) {
        if !x.short_was.is_empty() {
            clashed += 1;
        }
        let y = e.c.get_task(&x.id).unwrap();
        assert_eq!(
            (&y.short, &y.short_was),
            (&x.short, &x.short_was),
            "{}",
            x.id
        );
        assert!(
            seen.insert(x.short.clone(), x.id.clone()).is_none(),
            "#{} twice",
            x.short
        );
    }
    assert_eq!(seen.len(), 300);
    assert!(clashed > 0);
}

#[test]
fn test_parse_ref() {
    assert_eq!(parse_ref("mlg#12"), Some(("mlg".into(), "12".into())));
    assert_eq!(parse_ref("MLG#12B"), Some(("mlg".into(), "12b".into())));
    assert_eq!(parse_ref("#12"), Some(("".into(), "12".into())));
    assert_eq!(parse_ref("12"), Some(("".into(), "12".into())));
    assert_eq!(parse_ref("1bad#12"), None);
    assert_eq!(parse_ref("a-very-long-board-tag#1"), None);
}
