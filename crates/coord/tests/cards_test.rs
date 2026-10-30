//! Port of I:internal/coord/cards_test.go.
#![allow(clippy::type_complexity)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use coord::rows::{BlockerResolvedRow, TaskCommentRow};
use coord::*;
use store::kinds;

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

fn step(text: &str) -> Step {
    Step {
        text: text.into(),
        ..Step::default()
    }
}

fn st(id: &str, text: &str, done: bool, doing: bool) -> Step {
    Step {
        id: id.into(),
        text: text.into(),
        done,
        doing,
    }
}

fn step_ids(ss: &[Step]) -> Vec<String> {
    ss.iter().map(|s| s.id.clone()).collect()
}

fn claimed(e: &Env) -> Task {
    let t = e.c.create_task(ALICE, nt("ship it")).unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap()
}

fn q(question: &str) -> NewBlocker {
    NewBlocker {
        question: question.into(),
        ..NewBlocker::default()
    }
}

fn opt(answer: &str) -> BlockerOption {
    BlockerOption {
        answer: answer.into(),
        ..BlockerOption::default()
    }
}

fn bb(kind: &str, r: &str) -> Option<BlockedBy> {
    Some(BlockedBy {
        kind: kind.into(),
        reference: r.into(),
    })
}

fn resolve_text(text: &str) -> BlockerAnswer {
    BlockerAnswer {
        resolve: true,
        text: text.into(),
        ..BlockerAnswer::default()
    }
}

fn done(b: bool) -> StepUpdate {
    StepUpdate {
        done: Some(b),
        doing: None,
    }
}

fn doing(b: bool) -> StepUpdate {
    StepUpdate {
        done: None,
        doing: Some(b),
    }
}

#[test]
fn test_set_steps() {
    let long = "x".repeat(MAX_STEP_TEXT + 1);
    let many: Vec<Step> = (0..=MAX_STEPS).map(|_| step("s")).collect();
    let cases: Vec<(&str, &str, Vec<Step>, Option<ErrorKind>, Vec<&str>)> = vec![
        (
            "holder sets three",
            ALICE,
            vec![
                st("", "a", true, false),
                st("", "b", false, true),
                step("c"),
            ],
            None,
            vec!["s1", "s2", "s3"],
        ),
        ("person may", USER, vec![step("a")], None, vec!["s1"]),
        ("creator may", ALICE, vec![step("a")], None, vec!["s1"]),
        (
            "stranger may not",
            CAROL,
            vec![step("a")],
            Some(ErrorKind::NotAllowed),
            vec![],
        ),
        (
            "empty text",
            ALICE,
            vec![step("  ")],
            Some(ErrorKind::Invalid),
            vec![],
        ),
        (
            "too long",
            ALICE,
            vec![step(&long)],
            Some(ErrorKind::Invalid),
            vec![],
        ),
        (
            "two lines",
            ALICE,
            vec![step("a\nb")],
            Some(ErrorKind::Invalid),
            vec![],
        ),
        ("too many", ALICE, many, Some(ErrorKind::Invalid), vec![]),
        (
            "two doing",
            ALICE,
            vec![st("", "a", false, true), st("", "b", false, true)],
            Some(ErrorKind::Invalid),
            vec![],
        ),
        (
            "done and doing",
            ALICE,
            vec![st("", "a", true, true)],
            Some(ErrorKind::Invalid),
            vec![],
        ),
        (
            "empty list clears nothing: no-op",
            ALICE,
            vec![],
            None,
            vec![],
        ),
    ];
    for (name, by, steps, want_e, want_ids) in cases {
        let e = Env::new();
        let task = claimed(&e);
        let r = e.c.set_steps(by, &task.id, steps);
        if let Some(k) = want_e {
            assert!(r.as_ref().is_err_and(|x| x.is(k)), "{name}: {r:?}");
            assert!(e.rows(kinds::TASK_STEPS).is_empty(), "{name}");
            continue;
        }
        assert_eq!(step_ids(&r.unwrap().steps), want_ids, "{name}");
    }
}

#[test]
fn test_create_task_with_steps() {
    let e = Env::new();
    want_err(
        e.c.create_task(
            ALICE,
            NewTask {
                title: "x".into(),
                steps: vec![st("", "a", false, true), st("", "b", false, true)],
                ..NewTask::default()
            },
        ),
        ErrorKind::Invalid,
    );
    want_err(
        e.c.create_task(
            ALICE,
            NewTask {
                title: "x".into(),
                steps: vec![step(" ")],
                ..NewTask::default()
            },
        ),
        ErrorKind::Invalid,
    );
    assert!(e.rows(kinds::TASK_CREATED).is_empty());
    let task =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "ship".into(),
                steps: vec![
                    st("s7", " notes ", true, false),
                    st("", "tag", false, true),
                    step("announce"),
                ],
                ..NewTask::default()
            },
        )
        .unwrap();
    let want = vec![
        st("s1", "notes", true, false),
        st("s2", "tag", false, true),
        st("s3", "announce", false, false),
    ];
    assert_eq!(task.steps, want);
    assert_eq!(task.progress(), (1, 3));
    let mut more = want.clone();
    more.push(step("rest"));
    let got = e.c.set_steps(ALICE, &task.id, more).unwrap();
    assert_eq!(step_ids(&got.steps), ["s1", "s2", "s3", "s4"]);
    assert_eq!(
        step_ids(&e.reopen().get_task(&task.id).unwrap().steps),
        ["s1", "s2", "s3", "s4"]
    );
    let plain = e.c.create_task(ALICE, nt("plain")).unwrap();
    for r in e.rows(kinds::TASK_STEPS) {
        assert!(!String::from_utf8_lossy(&r.payload).contains(&plain.id));
    }
}

#[test]
fn test_set_steps_keeps_ids() {
    let e = Env::new();
    let task = claimed(&e);
    e.c.set_steps(ALICE, &task.id, vec![step("a"), step("b"), step("c")])
        .unwrap();
    let got =
        e.c.set_steps(
            ALICE,
            &task.id,
            vec![step("c"), st("s1", "a, renamed", true, false), step("d")],
        )
        .unwrap();
    let want = vec![
        st("s3", "c", false, false),
        st("s1", "a, renamed", true, false),
        st("s4", "d", false, false),
    ];
    assert_eq!(got.steps, want);
    e.c.set_steps(ALICE, &task.id, want.clone()).unwrap();
    assert_eq!(e.rows(kinds::TASK_STEPS).len(), 2);
    let mut more = want.clone();
    more.push(step("e"));
    let got = e.c.set_steps(ALICE, &task.id, more).unwrap();
    assert_eq!(got.steps[3].id, "s5");
    assert_eq!(got.progress(), (1, 4));
    assert_eq!(Task::default().progress(), (0, 0));
}

#[test]
fn test_set_step() {
    let cases: Vec<(
        &str,
        &str,
        &str,
        StepUpdate,
        Option<ErrorKind>,
        Option<Vec<Step>>,
    )> = vec![
        (
            "tick",
            ALICE,
            "s1",
            done(true),
            None,
            Some(vec![
                st("s1", "a", true, false),
                st("s2", "b", false, true),
                st("s3", "c", false, false),
            ]),
        ),
        (
            "mark current moves doing",
            ALICE,
            "s3",
            doing(true),
            None,
            Some(vec![
                st("s1", "a", false, false),
                st("s2", "b", false, false),
                st("s3", "c", false, true),
            ]),
        ),
        (
            "done clears doing",
            ALICE,
            "s2",
            done(true),
            None,
            Some(vec![
                st("s1", "a", false, false),
                st("s2", "b", true, false),
                st("s3", "c", false, false),
            ]),
        ),
        (
            "untick doing step stops it",
            ALICE,
            "s2",
            doing(false),
            None,
            Some(vec![
                st("s1", "a", false, false),
                st("s2", "b", false, false),
                st("s3", "c", false, false),
            ]),
        ),
        ("no change is a no-op", ALICE, "s1", done(false), None, None),
        (
            "person may",
            USER,
            "s1",
            done(true),
            None,
            Some(vec![
                st("s1", "a", true, false),
                st("s2", "b", false, true),
                st("s3", "c", false, false),
            ]),
        ),
        (
            "stranger may not",
            CAROL,
            "s1",
            done(true),
            Some(ErrorKind::NotAllowed),
            None,
        ),
        (
            "unknown step",
            ALICE,
            "s9",
            done(true),
            Some(ErrorKind::NotFound),
            None,
        ),
        (
            "nothing given",
            ALICE,
            "s1",
            StepUpdate::default(),
            Some(ErrorKind::Invalid),
            None,
        ),
        (
            "done and doing",
            ALICE,
            "s1",
            StepUpdate {
                done: Some(true),
                doing: Some(true),
            },
            Some(ErrorKind::Invalid),
            None,
        ),
    ];
    for (name, by, sid, u, want_e, want) in cases {
        let e = Env::new();
        let task = claimed(&e);
        let before =
            e.c.set_steps(
                ALICE,
                &task.id,
                vec![step("a"), st("", "b", false, true), step("c")],
            )
            .unwrap();
        let r = e.c.set_step(by, &task.id, sid, u);
        if let Some(k) = want_e {
            assert!(r.as_ref().is_err_and(|x| x.is(k)), "{name}: {r:?}");
            continue;
        }
        let got = r.unwrap();
        let (want, rows) = match want {
            Some(w) => (w, 1),
            None => (before.steps.clone(), 0),
        };
        assert_eq!(got.steps, want, "{name}");
        assert_eq!(e.rows(kinds::TASK_STEP).len(), rows, "{name}");
    }
}

#[test]
fn test_raise_validation() {
    let too_many: Vec<BlockerOption> = (0..=MAX_OPTIONS).map(|i| opt(&"a".repeat(i + 1))).collect();
    let with = |q: &str, c: &str, opts: Vec<BlockerOption>| NewBlocker {
        question: q.into(),
        context: c.into(),
        options: opts,
        ..NewBlocker::default()
    };
    let cases: Vec<(&str, &str, NewBlocker, Option<ErrorKind>)> = vec![
        (
            "holder with options",
            ALICE,
            with(
                "Ship today?",
                "CI passes.",
                vec![
                    BlockerOption {
                        why: "it's ready".into(),
                        ..opt("Yes")
                    },
                    opt("No"),
                ],
            ),
            None,
        ),
        ("person", USER, q("q"), None),
        ("stranger", CAROL, q("q"), Some(ErrorKind::NotAllowed)),
        ("no question", ALICE, q(" "), Some(ErrorKind::Invalid)),
        (
            "question too long",
            ALICE,
            q(&"q".repeat(MAX_QUESTION + 1)),
            Some(ErrorKind::Invalid),
        ),
        (
            "question at the cap",
            ALICE,
            q(&"é".repeat(MAX_QUESTION)),
            None,
        ),
        (
            "context too long",
            ALICE,
            with("q", &"c".repeat(MAX_CONTEXT + 1), vec![]),
            Some(ErrorKind::Invalid),
        ),
        (
            "too many options",
            ALICE,
            with("q", "", too_many),
            Some(ErrorKind::Invalid),
        ),
        (
            "empty option",
            ALICE,
            with("q", "", vec![opt(" ")]),
            Some(ErrorKind::Invalid),
        ),
        (
            "answer too long",
            ALICE,
            with("q", "", vec![opt(&"a".repeat(MAX_ANSWER + 1))]),
            Some(ErrorKind::Invalid),
        ),
        (
            "why too long",
            ALICE,
            with(
                "q",
                "",
                vec![BlockerOption {
                    why: "w".repeat(MAX_WHY + 1),
                    ..opt("a")
                }],
            ),
            Some(ErrorKind::Invalid),
        ),
        (
            "bad blocked-by",
            ALICE,
            NewBlocker {
                blocked_by: bb("weather", ""),
                ..q("q")
            },
            Some(ErrorKind::Invalid),
        ),
    ];
    for (name, by, n, want_e) in cases {
        let e = Env::new();
        let task = claimed(&e);
        let nopts = n.options.len();
        let question = n.question.trim().to_string();
        let r = e.c.raise(by, &task.id, n);
        if let Some(k) = want_e {
            assert!(r.as_ref().is_err_and(|x| x.is(k)), "{name}: {r:?}");
            let g = e.c.get_task(&task.id).unwrap();
            assert!(g.state == WORKING && g.blockers.is_empty(), "{name}");
            continue;
        }
        let (got, b) = r.unwrap();
        assert!(
            got.state == BLOCKED
                && got.reason == question
                && b.id == "b1"
                && b.question == question
                && b.raised_by == by
                && b.open(),
            "{name}"
        );
        for o in &b.options {
            assert!(o.by == by && !o.at.is_zero(), "{name}");
        }
        assert_eq!(b.options.len(), nopts, "{name}");
    }
}

#[test]
fn test_raise_adds_dedupes_and_caps() {
    let e = Env::new();
    let task = claimed(&e);
    e.c.acquire(BOB, "gpu/qwen", Duration::from_secs(3600), false)
        .unwrap();
    let (_, b1) =
        e.c.raise(
            ALICE,
            &task.id,
            NewBlocker {
                blocked_by: bb(BLOCKED_ON_LEASE, "gpu/qwen"),
                ..q("one")
            },
        )
        .unwrap();
    let (_, again) = e.c.raise(ALICE, &task.id, q("one")).unwrap();
    assert_eq!(again.id, b1.id);
    let (got, b2) = e.c.raise(ALICE, &task.id, q("two")).unwrap();
    assert!(
        b2.id == "b2"
            && got.reason == "one"
            && got
                .blocked_by
                .as_ref()
                .is_some_and(|b| b.kind == BLOCKED_ON_LEASE)
            && got.open_blockers().len() == 2
    );
    for i in 3..=MAX_OPEN_BLOCKERS {
        e.c.raise(ALICE, &task.id, q(&"q".repeat(i))).unwrap();
    }
    want_err(
        e.c.raise(ALICE, &task.id, q("one too many")),
        ErrorKind::Invalid,
    );
    e.c.set_task_state(ALICE, &task.id, COMPLETED, "").unwrap();
    want_err(e.c.raise(ALICE, &task.id, q("late")), ErrorKind::BadState);
}

#[test]
fn test_recommend() {
    type Setup = Option<Box<dyn Fn(&Env, &str)>>;
    let cases: Vec<(&str, &str, Setup, &str, &str, Option<ErrorKind>, usize)> = vec![
        ("holder", ALICE, None, "b1", "Wait", None, 2),
        (
            "watcher",
            BOB,
            Some(Box::new(|e: &Env, id: &str| {
                e.c.watch(BOB, id).unwrap();
            })),
            "b1",
            "Wait",
            None,
            2,
        ),
        ("person", USER, None, "b1", "Wait", None, 2),
        (
            "stranger must watch first",
            CAROL,
            None,
            "b1",
            "Wait",
            Some(ErrorKind::NotAllowed),
            1,
        ),
        (
            "same answer is a no-op",
            ALICE,
            None,
            "b1",
            "ship now",
            None,
            1,
        ),
        (
            "unknown blocker",
            ALICE,
            None,
            "b9",
            "Wait",
            Some(ErrorKind::NotFound),
            1,
        ),
        (
            "empty answer",
            ALICE,
            None,
            "b1",
            " ",
            Some(ErrorKind::Invalid),
            1,
        ),
        (
            "resolved blocker",
            ALICE,
            Some(Box::new(|e: &Env, id: &str| {
                e.c.answer(
                    USER,
                    id,
                    "b1",
                    BlockerAnswer {
                        resolve: true,
                        pick: 1,
                        ..BlockerAnswer::default()
                    },
                )
                .unwrap();
            })),
            "b1",
            "Wait",
            Some(ErrorKind::BadState),
            1,
        ),
    ];
    for (name, by, setup, blocker, answer, want_e, want_n) in cases {
        let e = Env::new();
        let task = claimed(&e);
        e.c.raise(
            ALICE,
            &task.id,
            NewBlocker {
                options: vec![opt("Ship now")],
                ..q("Ship?")
            },
        )
        .unwrap();
        if let Some(s) = setup {
            s(&e, &task.id);
        }
        let r = e.c.recommend(by, &task.id, blocker, answer, "because");
        match want_e {
            Some(k) => assert!(r.as_ref().is_err_and(|x| x.is(k)), "{name}: {r:?}"),
            None => {
                r.unwrap();
            }
        }
        let got = e.c.get_task(&task.id).unwrap();
        assert_eq!(got.blockers[0].options.len(), want_n, "{name}");
        if want_e.is_none() && want_n == 2 {
            let o = &got.blockers[0].options[1];
            assert!(
                o.by == by && o.answer == answer && o.why == "because",
                "{name}: {o:?}"
            );
        }
    }
}

#[test]
fn test_recommend_cap() {
    let e = Env::new();
    let task = claimed(&e);
    let opts: Vec<BlockerOption> = (0..MAX_OPTIONS).map(|i| opt(&"o".repeat(i + 1))).collect();
    e.c.raise(
        ALICE,
        &task.id,
        NewBlocker {
            options: opts,
            ..q("q")
        },
    )
    .unwrap();
    want_err(
        e.c.recommend(ALICE, &task.id, "b1", "another", ""),
        ErrorKind::Invalid,
    );
    let got = e.c.recommend(ALICE, &task.id, "b1", "ooo", "").unwrap();
    assert!(got.blockers[0].options[2].recommended && got.blockers[0].options.len() == MAX_OPTIONS);
}

fn rec(answer: &str) -> BlockerOption {
    BlockerOption {
        recommended: true,
        ..opt(answer)
    }
}

#[test]
fn test_recommended_duplicate_merges() {
    let e = Env::new();
    let task = claimed(&e);
    let (_, b) =
        e.c.raise(
            ALICE,
            &task.id,
            NewBlocker {
                options: vec![rec("Wintun"), opt("TAP"), rec("wintun")],
                ..q("Which adapter?")
            },
        )
        .unwrap();
    assert!(b.options.len() == 2 && b.options[0].recommended && !b.options[1].recommended);
}

#[test]
fn test_recommended_flag() {
    let e = Env::new();
    let task = claimed(&e);
    want_err(
        e.c.raise(
            ALICE,
            &task.id,
            NewBlocker {
                options: vec![rec("Wintun"), rec("TAP")],
                ..q("Which adapter?")
            },
        ),
        ErrorKind::Invalid,
    );
    assert!(e.rows(kinds::TASK_STATUS).is_empty());
    let (_, b) =
        e.c.raise(
            ALICE,
            &task.id,
            NewBlocker {
                options: vec![opt("Wintun"), opt("TAP"), rec("wintun")],
                ..q("Which adapter?")
            },
        )
        .unwrap();
    assert!(b.options.len() == 2 && b.options[0].recommended && !b.options[1].recommended);
    e.c.watch(BOB, &task.id).unwrap();
    e.c.recommend(BOB, &task.id, "b1", "TAP", "broadcasts")
        .unwrap();
    e.c.recommend(BOB, &task.id, "b1", "Radmin", "").unwrap();
    let n = e.rows(kinds::BLOCKER_OPTION).len();
    e.c.recommend(BOB, &task.id, "b1", "tap", "").unwrap();
    assert_eq!(e.rows(kinds::BLOCKER_OPTION).len(), n);
    let check = |tk: &Task| {
        let got: Vec<String> = tk.blockers[0]
            .options
            .iter()
            .map(|o| format!("{} {} {}", o.answer, o.recommended, o.by))
            .collect();
        assert_eq!(
            got,
            [
                format!("Wintun true {ALICE}"),
                format!("TAP true {ALICE}"),
                format!("Radmin true {BOB}")
            ]
        );
    };
    check(&e.c.get_task(&task.id).unwrap());
    check(&e.reopen().get_task(&task.id).unwrap());
}

#[test]
fn test_answer() {
    let pick = |p: i64, wake: bool| BlockerAnswer {
        resolve: true,
        pick: p,
        wake,
        ..BlockerAnswer::default()
    };
    let cases: Vec<(
        &str,
        &str,
        &str,
        BlockerAnswer,
        Option<ErrorKind>,
        &str,
        String,
        i64,
        Vec<&str>,
    )> = vec![
        (
            "person picks an option, last item",
            USER,
            "b2",
            pick(2, true),
            None,
            WORKING,
            "TAP-Windows".into(),
            1,
            vec![BOB, ALICE],
        ),
        (
            "own words",
            USER,
            "b2",
            resolve_text(" use both "),
            None,
            WORKING,
            "use both".into(),
            -1,
            vec![BOB, ALICE],
        ),
        (
            "an agent may (decision 19)",
            CAROL,
            "b2",
            pick(1, false),
            None,
            WORKING,
            "Wintun".into(),
            0,
            vec![BOB, ALICE],
        ),
        (
            "the raiser itself: holders hear",
            BOB,
            "b2",
            pick(1, false),
            None,
            WORKING,
            "Wintun".into(),
            0,
            vec![ALICE],
        ),
        (
            "pick out of range",
            USER,
            "b2",
            pick(3, false),
            Some(ErrorKind::Invalid),
            "",
            String::new(),
            0,
            vec![],
        ),
        (
            "negative pick",
            USER,
            "b2",
            pick(-1, false),
            Some(ErrorKind::Invalid),
            "",
            String::new(),
            0,
            vec![],
        ),
        (
            "no text",
            USER,
            "b2",
            resolve_text("  "),
            Some(ErrorKind::Invalid),
            "",
            String::new(),
            0,
            vec![],
        ),
        (
            "text too long",
            USER,
            "b2",
            resolve_text(&"x".repeat(MAX_OWN_ANSWER + 1)),
            Some(ErrorKind::Invalid),
            "",
            String::new(),
            0,
            vec![],
        ),
        (
            "unknown item",
            USER,
            "b7",
            pick(1, false),
            Some(ErrorKind::NotFound),
            "",
            String::new(),
            0,
            vec![],
        ),
        (
            "already resolved",
            USER,
            "b1",
            pick(1, false),
            Some(ErrorKind::BadState),
            "",
            String::new(),
            0,
            vec![],
        ),
    ];
    for (name, by, blocker, a, want_e, want_state, want_ans, want_opt, want_to) in cases {
        let e = Env::new();
        let task = claimed(&e);
        e.c.watch(BOB, &task.id).unwrap();
        e.c.raise(
            ALICE,
            &task.id,
            NewBlocker {
                options: vec![opt("yes")],
                ..q("first?")
            },
        )
        .unwrap();
        e.c.set_shared(ALICE, &task.id, true).unwrap();
        e.c.join(BOB, &task.id, BOB).unwrap();
        e.c.raise(
            BOB,
            &task.id,
            NewBlocker {
                context: "Radmin isn't open source.".into(),
                options: vec![
                    BlockerOption {
                        why: "signed, modern".into(),
                        ..rec("Wintun")
                    },
                    BlockerOption {
                        why: "broadcasts just work".into(),
                        ..opt("TAP-Windows")
                    },
                ],
                ..q("Which adapter?")
            },
        )
        .unwrap();
        let (mid, n1) = e.c.answer(USER, &task.id, "b1", pick(1, false)).unwrap();
        assert!(
            mid.state == BLOCKED
                && mid.reason == "Which adapter?"
                && mid.open_blockers().len() == 1,
            "{name}"
        );
        assert_eq!(n1.to, [ALICE, BOB], "{name}");
        assert!(n1.text.contains("1 more blockers are still open"), "{name}");
        let wake = a.wake;
        let p = a.pick;
        let r = e.c.answer(by, &task.id, blocker, a);
        if let Some(k) = want_e {
            assert!(r.as_ref().is_err_and(|x| x.is(k)), "{name}: {r:?}");
            assert_eq!(e.c.get_task(&task.id).unwrap().state, BLOCKED, "{name}");
            continue;
        }
        let (got, n) = r.unwrap();
        let b = &got.blockers[1];
        let res = b.resolved.as_ref().expect("resolved");
        assert!(
            got.state == want_state
                && got.reason.is_empty()
                && got.blocked_by.is_none()
                && !b.open(),
            "{name}"
        );
        assert!(
            res.answer == want_ans && res.option == want_opt && res.by == by && !res.closed,
            "{name}: {res:?}"
        );
        assert!(
            n.kind == NOTICE_ANSWERED
                && n.to == want_to
                && n.wake == wake
                && n.reference == format!("task:{}", task.id),
            "{name}: {n:?}"
        );
        assert!(
            n.text.contains("Which adapter?")
                && n.text.contains(&want_ans)
                && n.text.contains("working again"),
            "{name}: {}",
            n.text
        );
        if p == 1 {
            assert!(
                n.text.contains(&format!("the answer {BOB} recommended")),
                "{name}: {}",
                n.text
            );
        }
        if p == 2 {
            assert!(
                n.text.contains(&format!("option 2, offered by {BOB}")),
                "{name}: {}",
                n.text
            );
        }
        let rows = e.rows(kinds::BLOCKER_RESOLVED);
        let row: BlockerResolvedRow = payload(rows.last().unwrap());
        assert!(
            row.restored == WORKING && row.option == want_opt,
            "{name}: {row:?}"
        );
    }
}

#[test]
fn test_answer_unclaimed_restores_submitted() {
    let e = Env::new();
    let task = e.c.create_task(ALICE, nt("nobody on it")).unwrap();
    e.c.raise(ALICE, &task.id, q("q")).unwrap();
    let (got, n) =
        e.c.answer(USER, &task.id, "b1", resolve_text("go"))
            .unwrap();
    assert_eq!(got.state, SUBMITTED);
    assert_eq!(n.to, [ALICE]);
}

#[test]
fn test_answer_tells_holders() {
    let e = Env::new();
    let task = e.c.create_task(ALICE, nt("db")).unwrap();
    e.c.claim_task(BOB, &task.id).unwrap();
    e.c.raise(ALICE, &task.id, q("which db?")).unwrap();
    let (got, n) =
        e.c.answer(
            USER,
            &task.id,
            "b1",
            BlockerAnswer {
                wake: true,
                ..resolve_text("postgres")
            },
        )
        .unwrap();
    assert!(got.state == WORKING && n.to == [ALICE, BOB] && n.wake);
}

#[test]
fn test_leaving_blocked_closes_items() {
    type Leave = Box<dyn Fn(&Env, &str) -> Task>;
    let cases: Vec<(&str, Leave, &str, &str)> = vec![
        (
            "unblock resolves all with the note",
            Box::new(|e: &Env, id: &str| e.c.unblock(CAROL, id, "sorted it").unwrap().0),
            CAROL,
            "sorted it",
        ),
        (
            "a state change closes them",
            Box::new(|e: &Env, id: &str| {
                e.c.set_task_state(ALICE, id, WORKING, "moving on").unwrap()
            }),
            ALICE,
            "moving on",
        ),
    ];
    for (name, leave, by, note) in cases {
        let e = Env::new();
        let task = claimed(&e);
        e.c.acquire(BOB, "gpu/qwen", Duration::from_secs(3600), false)
            .unwrap();
        e.c.raise(
            ALICE,
            &task.id,
            NewBlocker {
                blocked_by: bb(BLOCKED_ON_LEASE, "gpu/qwen"),
                ..q("one")
            },
        )
        .unwrap();
        e.c.raise(ALICE, &task.id, q("two")).unwrap();
        let got = leave(&e, &task.id);
        assert!(
            got.state == WORKING
                && got.reason.is_empty()
                && got.open_blockers().is_empty()
                && got.blockers.len() == 2,
            "{name}"
        );
        for b in &got.blockers {
            let r = b.resolved.as_ref().unwrap();
            assert!(
                r.closed && r.by == by && r.answer == note && r.option == -1,
                "{name}: {r:?}"
            );
        }
    }
}

#[test]
fn test_sweeper_closes_only_what_went_away() {
    let cases: Vec<(&str, Option<BlockedBy>, &str, Vec<&str>)> = vec![
        ("a question stays open", None, BLOCKED, vec!["b2"]),
        (
            "everything went away: unblocked",
            bb(BLOCKED_ON_LEASE, "gpu/qwen"),
            WORKING,
            vec![],
        ),
    ];
    for (name, second_on, want_state, want_open) in cases {
        let e = Env::new();
        let task = claimed(&e);
        e.c.acquire(BOB, "gpu/qwen", Duration::from_secs(3600), false)
            .unwrap();
        e.c.raise(
            ALICE,
            &task.id,
            NewBlocker {
                blocked_by: bb(BLOCKED_ON_LEASE, "gpu/qwen"),
                ..q("one")
            },
        )
        .unwrap();
        let question = second_on.is_none();
        e.c.raise(
            ALICE,
            &task.id,
            NewBlocker {
                blocked_by: second_on,
                ..q("two")
            },
        )
        .unwrap();
        let for_user = BoardFilter {
            viewer: USER.into(),
            ..BoardFilter::default()
        };
        let v = e.c.board(&for_user);
        assert!(
            v.leases.len() == 1 && v.leases[0].blocks.len() == 1,
            "{name}"
        );
        e.c.release(BOB, "gpu/qwen").unwrap();
        if question {
            let c = &e.c.board(&for_user).needs_you;
            assert!(!c[0].resolved && c[0].gone == ["b1"], "{name}: {:?}", c[0]);
        }
        let ns = e.c.sweep_notices().unwrap();
        if question {
            assert!(
                ns.len() == 1
                    && ns[0].kind == NOTICE_ANSWERED
                    && ns[0].to == [ALICE]
                    && ns[0].text.contains("b1 (lease released)")
                    && ns[0].text.contains("1 more blockers are still open"),
                "{name}: {ns:?}"
            );
        }
        let got = e.c.get_task(&task.id).unwrap();
        let open: Vec<String> = got.open_blockers().into_iter().map(|b| b.id).collect();
        assert_eq!(
            (got.state.as_str(), open),
            (
                want_state,
                want_open.iter().map(|s| s.to_string()).collect::<Vec<_>>()
            ),
            "{name}"
        );
        let r = got.blockers[0].resolved.as_ref().unwrap();
        assert!(
            r.closed && r.by == "irori" && r.answer == "lease released",
            "{name}"
        );
        if question {
            assert!(
                got.blocked_by
                    .as_ref()
                    .is_some_and(|b| b.kind == BLOCKED_ON_PERSON)
                    && got.reason == "two",
                "{name}"
            );
            let v = e.c.board(&for_user);
            assert!(v.needs_you.len() == 1 && !v.needs_you[0].resolved, "{name}");
            assert_eq!(e.rows(kinds::BLOCKER_RESOLVED).len(), 1);
            e.c.sweep_notices().unwrap();
            assert_eq!(e.rows(kinds::BLOCKER_RESOLVED).len(), 1);
        }
    }
}

#[test]
fn test_raise_defaults_to_person() {
    let e = Env::new();
    let task = claimed(&e);
    let (got, b) = e.c.raise(ALICE, &task.id, q("Ship?")).unwrap();
    assert!(
        b.on.as_ref().is_some_and(|o| o.kind == BLOCKED_ON_PERSON)
            && got
                .blocked_by
                .as_ref()
                .is_some_and(|o| o.kind == BLOCKED_ON_PERSON)
    );
    let for_user = BoardFilter {
        viewer: USER.into(),
        ..BoardFilter::default()
    };
    let v = e.c.board(&for_user);
    assert!(v.needs_you.len() == 1 && v.blocked.is_empty());
    let t2 = claimed(&e);
    e.c.raise(
        ALICE,
        &t2.id,
        NewBlocker {
            blocked_by: bb(BLOCKED_ON_AGENT, BOB),
            ..q("Your API?")
        },
    )
    .unwrap();
    let v = e.c.board(&BoardFilter {
        viewer: BOB.into(),
        ..BoardFilter::default()
    });
    assert!(v.needs_you.len() == 1 && v.needs_you[0].task.id == t2.id);
    let t3 = claimed(&e);
    e.c.block(ALICE, &t3.id, "ci is red", None).unwrap();
    let v = e.c.board(&for_user);
    let mut lane: Vec<String> = v.blocked.iter().map(|c| c.task.id.clone()).collect();
    lane.sort();
    let mut want = vec![t2.id.clone(), t3.id.clone()];
    want.sort();
    assert!(v.needs_you.len() == 1 && lane == want);
}

#[test]
fn test_block_keeps_one_reason_item() {
    let e = Env::new();
    let task = claimed(&e);
    e.c.block(ALICE, &task.id, "waiting on the db", None)
        .unwrap();
    e.c.raise(
        ALICE,
        &task.id,
        NewBlocker {
            options: vec![opt("postgres")],
            ..q("Which db?")
        },
    )
    .unwrap();
    let got =
        e.c.block(ALICE, &task.id, "still waiting on the db", None)
            .unwrap();
    assert!(
        got.blockers.len() == 2
            && got.blockers[0].id == "b1"
            && got.blockers[0].question == "still waiting on the db"
    );
    assert!(got.blockers[1].question == "Which db?" && got.reason == "still waiting on the db");
    let long = "r".repeat(MAX_QUESTION + 50);
    let got = e.c.block(ALICE, &task.id, &long, None).unwrap();
    let b = &got.blockers[0];
    assert!(b.question.chars().count() == MAX_QUESTION && b.context == long && got.reason == long);
    let n =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "port".into(),
                block_reason: "a dead dev server has it".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    assert!(
        n.blockers.len() == 1
            && n.blockers[0].id == "b1"
            && n.blockers[0].question == "a dead dev server has it"
    );
}

#[test]
fn test_resolved_items_are_trimmed() {
    let e = Env::new();
    let task = claimed(&e);
    for i in 0..KEEP_RESOLVED + 3 {
        let (_, b) =
            e.c.raise(ALICE, &task.id, q(&format!("q{}", "x".repeat(i))))
                .unwrap();
        e.c.answer(USER, &task.id, &b.id, resolve_text("ok"))
            .unwrap();
    }
    let got = e.c.get_task(&task.id).unwrap();
    assert_eq!(got.blockers.len(), KEEP_RESOLVED);
    assert_eq!(got.blockers[0].id, "b4");
}

#[test]
fn test_watch_rules() {
    let cases = [
        ("an agent", BOB, false, None),
        ("a person can't", USER, false, Some(ErrorKind::Invalid)),
        ("irori can't", "irori", false, Some(ErrorKind::Invalid)),
        ("a done card can be watched", BOB, true, None),
    ];
    for (name, by, d, want_e) in cases {
        let e = Env::new();
        let task = claimed(&e);
        if d {
            e.c.set_task_state(ALICE, &task.id, COMPLETED, "").unwrap();
        }
        let r = e.c.watch(by, &task.id);
        if let Some(k) = want_e {
            assert!(r.as_ref().is_err_and(|x| x.is(k)), "{name}");
            continue;
        }
        let got = r.unwrap();
        e.c.watch(by, &task.id).unwrap();
        assert!(
            got.watchers == [by] && e.rows(kinds::TASK_WATCHED).len() == 1,
            "{name}"
        );
        let got = e.c.unwatch(by, &task.id).unwrap();
        e.c.unwatch(by, &task.id).unwrap();
        assert!(
            got.watchers.is_empty() && e.rows(kinds::TASK_WATCHED).len() == 2,
            "{name}"
        );
    }
}

#[test]
fn test_watch_notices() {
    type Do = Box<dyn Fn(&Env, &str)>;
    let cases: Vec<(&str, Do, Vec<&str>, Vec<String>)> = vec![
        (
            "steps and a tick",
            Box::new(|e: &Env, id: &str| {
                e.c.set_steps(ALICE, id, vec![step("build"), step("tag")])
                    .unwrap();
                e.c.set_step(ALICE, id, "s1", done(true)).unwrap();
            }),
            vec![BOB],
            vec![
                format!("{ALICE} set the steps (0/2 done)"),
                format!("{ALICE} ticked “build” (1/2)"),
                "steps 1/2".into(),
            ],
        ),
        (
            "a blocker raised and answered",
            Box::new(|e: &Env, id: &str| {
                e.c.raise(ALICE, id, q("Ship?")).unwrap();
                e.c.answer(USER, id, "b1", resolve_text("yes")).unwrap();
            }),
            vec![BOB],
            vec![
                format!("{ALICE} raised a blocker: Ship?"),
                format!("{USER} answered “Ship?”: yes"),
            ],
        ),
        (
            "a state change",
            Box::new(|e: &Env, id: &str| {
                e.c.set_task_state(ALICE, id, COMPLETED, "").unwrap();
            }),
            vec![BOB],
            vec![
                format!("{ALICE} set it to completed"),
                "Now: completed".into(),
            ],
        ),
        (
            "a PR link, not other links",
            Box::new(|e: &Env, id: &str| {
                e.c.add_link(ALICE, id, "https://linear.app/x/issue/ABC-1")
                    .unwrap();
                e.c.add_link(ALICE, id, "https://github.com/acme/app/pull/57")
                    .unwrap();
            }),
            vec![BOB],
            vec![format!("{ALICE} linked GitHub #57")],
        ),
        (
            "a recommendation tells the raiser too",
            Box::new(|e: &Env, id: &str| {
                e.c.raise(ALICE, id, q("Ship?")).unwrap();
                e.c.sweep_notices().unwrap();
                e.c.recommend(BOB, id, "b1", "Wait", "the temple is untested")
                    .unwrap();
            }),
            vec![ALICE],
            vec![format!("{BOB} recommended “Wait” for “Ship?”")],
        ),
        (
            "comments, joins and watches don't count",
            Box::new(|e: &Env, id: &str| {
                e.c.add_comment(ALICE, id, "hi", false).unwrap();
                e.c.watch(CAROL, id).unwrap();
            }),
            vec![],
            vec![],
        ),
    ];
    for (name, d, want_to, want_lines) in cases {
        let e = Env::new();
        let task = claimed(&e);
        e.c.watch(BOB, &task.id).unwrap();
        d(&e, &task.id);
        let ns = e.c.sweep_notices().unwrap();
        let watched: Vec<&Notice> = ns.iter().filter(|n| n.kind == NOTICE_WATCHED).collect();
        if want_to.is_empty() {
            assert!(watched.is_empty(), "{name}: {watched:?}");
            continue;
        }
        assert!(
            watched.len() == 1
                && watched[0].to == want_to
                && watched[0].by == "irori"
                && !watched[0].wake,
            "{name}: {watched:?}"
        );
        assert_eq!(watched[0].reference, format!("task:{}", task.id));
        for l in &want_lines {
            assert!(
                watched[0].text.contains(l.as_str()),
                "{name}: {:?} lacks {l:?}",
                watched[0].text
            );
        }
        assert!(!watched[0].text.contains("inear"), "{name}");
        assert!(e.c.sweep_notices().unwrap().is_empty(), "{name}");
    }
}

#[test]
fn test_watch_digest_text() {
    // cards_impl.go:1188-1275 verbatim.
    let e = Env::new();
    let task = claimed(&e);
    e.c.watch(BOB, &task.id).unwrap();
    e.c.set_steps(ALICE, &task.id, vec![step("build")]).unwrap();
    let ns = e.c.sweep_notices().unwrap();
    assert_eq!(
        ns[0].text,
        format!(
            "Update on \"ship it\" (#1, task:{id}), a card you watch or raised a blocker on:\n- {ALICE} set the steps (0/1 done)\nNow: working, steps 0/1.\nshow task:{id} for the card, its comments and links.",
            id = task.id
        )
    );
}

#[test]
fn test_watch_notice_skips_own_changes() {
    let e = Env::new();
    let task = claimed(&e);
    e.c.watch(ALICE, &task.id).unwrap();
    e.c.watch(BOB, &task.id).unwrap();
    e.c.set_steps(ALICE, &task.id, vec![step("a")]).unwrap();
    let ns = e.c.sweep_notices().unwrap();
    assert!(ns.len() == 1 && ns[0].to == [BOB], "{ns:?}");
    e.c.watch(CAROL, &task.id).unwrap();
    e.c.set_step(USER, &task.id, "s1", done(true)).unwrap();
    let ns = e.c.sweep_notices().unwrap();
    assert!(ns.len() == 1 && ns[0].to == [ALICE, BOB, CAROL], "{ns:?}");
    e.c.unwatch(BOB, &task.id).unwrap();
    e.c.set_step(ALICE, &task.id, "s1", done(false)).unwrap();
    assert!(e.reopen().sweep_notices().unwrap().is_empty());
    let ns = e.c.sweep_notices().unwrap();
    assert!(ns.len() == 1 && ns[0].to == [CAROL], "{ns:?}");
}

#[test]
fn test_cards_replay() {
    let e = Env::new();
    let task = claimed(&e);
    e.c.watch(BOB, &task.id).unwrap();
    e.c.set_steps(
        ALICE,
        &task.id,
        vec![st("", "a", true, false), st("", "b", false, true)],
    )
    .unwrap();
    e.c.set_step(ALICE, &task.id, "s2", done(true)).unwrap();
    e.c.block(ALICE, &task.id, "legacy reason", None).unwrap();
    e.c.raise(
        ALICE,
        &task.id,
        NewBlocker {
            context: "ctx".into(),
            options: vec![BlockerOption {
                why: "ready".into(),
                ..opt("yes")
            }],
            ..q("Ship?")
        },
    )
    .unwrap();
    e.c.recommend(BOB, &task.id, "b2", "no", "wait").unwrap();
    e.c.answer(USER, &task.id, "b1", resolve_text("fine"))
        .unwrap();
    e.clk.add(mins(1));
    e.c.add_link(ALICE, &task.id, "https://github.com/acme/app/pull/57")
        .unwrap();
    let want = e.c.get_task(&task.id).unwrap();
    let want_act = e.c.activity(&task.id).unwrap();
    let c2 = e.reopen();
    assert_eq!(c2.get_task(&task.id).unwrap(), want);
    assert_eq!(c2.activity(&task.id).unwrap(), want_act);
    assert_eq!(want_act[0].what, "linked GitHub #57");
    assert_eq!(want_act.last().unwrap().what, "put the card up");
    let (_, b) = c2.raise(ALICE, &task.id, q("third")).unwrap();
    assert_eq!(b.id, "b3");
}

#[test]
fn test_legacy_blocked_rows_fold() {
    let e = Env::new();
    let task = claimed(&e);
    for reason in ["old reason", "replaced reason"] {
        e.st
            .append(
                &format!("agent:{ALICE}"),
                kinds::TASK_STATUS,
                &gocompat::json::RawJson(format!(r#"{{"at":"2026-10-26T12:00:00Z","by":"{ALICE}","reason":"{reason}","state":"blocked","taskId":"{}"}}"#, task.id).into_bytes()),
            )
            .unwrap();
    }
    let c2 = e.reopen();
    let got = c2.get_task(&task.id).unwrap();
    assert!(got.state == BLOCKED && got.reason == "replaced reason" && got.blockers.len() == 1);
    assert!(
        got.blockers[0].id == "b1"
            && got.blockers[0].question == "replaced reason"
            && got.blockers[0].raised_by == ALICE
    );
    let (got, _) = c2
        .answer(USER, &task.id, "b1", resolve_text("done"))
        .unwrap();
    assert_eq!(got.state, WORKING);
}

#[test]
fn test_activity_cap() {
    let e = Env::new();
    let task = claimed(&e);
    for _ in 0..MAX_ACTIVITY + 5 {
        e.c.add_comment(ALICE, &task.id, "c", false).unwrap();
    }
    let acts = e.c.activity(&task.id).unwrap();
    assert_eq!(acts.len(), MAX_ACTIVITY);
    let pos = |a: &Activity| (a.hlc, a.device.clone(), a.seq);
    for (i, a) in acts.iter().enumerate() {
        assert!(
            a.kind == kinds::TASK_COMMENT
                && a.device == "dev-a"
                && a.what == "commented"
                && a.hlc != 0,
            "line {i}"
        );
        if i > 0 {
            assert!(
                pos(a) < pos(&acts[i - 1]),
                "lines {} and {i} out of order",
                i - 1
            );
        }
    }
    let (mid, low) = (acts[100].clone(), acts.last().unwrap().clone());
    e.remote(
        "dev-b",
        1,
        mid.hlc,
        "agent:dev-b/zed",
        kinds::TASK_COMMENT,
        &TaskCommentRow {
            task_id: task.id.clone(),
            comment_id: "c-b1".into(),
            by: "dev-b/zed".into(),
            text: "late".into(),
            at: fixed_now(),
            ..TaskCommentRow::default()
        },
    );
    let acts = e.c.activity(&task.id).unwrap();
    assert!(
        acts.len() == MAX_ACTIVITY
            && acts[100].device == "dev-b"
            && acts[100].by == "dev-b/zed"
            && acts[101] == mid
            && *acts.last().unwrap() != low
    );
    e.remote(
        "dev-c",
        1,
        acts.last().unwrap().hlc - 1,
        "agent:dev-c/yan",
        kinds::TASK_COMMENT,
        &TaskCommentRow {
            task_id: task.id.clone(),
            comment_id: "c-c1".into(),
            by: "dev-c/yan".into(),
            text: "older".into(),
            at: fixed_now(),
            ..TaskCommentRow::default()
        },
    );
    assert_eq!(e.c.activity(&task.id).unwrap(), acts);
    assert_eq!(e.reopen().activity(&task.id).unwrap(), acts);
    assert!(e.c.activity("nope").is_none());
}

#[test]
fn test_pr_link() {
    let cases = [
        (
            "https://github.com/acme/app/pull/57",
            Some(("acme/app", 57)),
        ),
        (
            "https://www.github.com/acme/app/pull/57/files",
            Some(("acme/app", 57)),
        ),
        ("https://github.com/acme/app/issues/57", None),
        ("https://evilgithub.com/acme/app/pull/57", None),
        ("src/pull/57", None),
    ];
    for (input, want) in cases {
        let l = parse_link(input).unwrap();
        assert_eq!(
            pr_link(&l),
            want.map(|(r, n)| (r.to_string(), n)),
            "{input}"
        );
    }
}

#[test]
fn test_merge_watch() {
    let task = Task {
        id: "T1".into(),
        title: "ship".into(),
        state: WORKING.into(),
        ..Task::default()
    };
    let a: Vec<String> = (0..6).map(|i| format!("a{i}")).collect();
    let b: Vec<String> = (0..6).map(|i| format!("b{i}")).collect();
    let got = merge_watch(&watch_text(&task, &a, 2), &watch_text(&task, &b, 0)).expect("merged");
    let mut lines = a[4..].to_vec();
    lines.extend(b.iter().cloned());
    assert_eq!(got, watch_text(&task, &lines, 6));
    assert!(merge_watch("hello", &got).is_none());
}

#[test]
fn test_watch_digest_comes_from_the_watchers_device() {
    const MAC: &str = "dev-b/mac-session";
    let owner: KeyFn = Arc::new(|k: &str| k == "user:dev-a" || k == "user:dev-b");
    let pc = Env::with("dev-a", Some(owner.clone()));
    let other = Env::with("dev-b", Some(owner));
    let sync = || {
        for (src, dst) in [(&pc, &other), (&other, &pc)] {
            let v = dst.st.vector().unwrap();
            for ev in src.st.missing(&v, store::MAX_MISSING_LIMIT).unwrap() {
                dst.st.insert(&ev).unwrap();
                dst.c.handle(&ev).unwrap();
            }
        }
    };
    let tk = pc.c.create_task(ALICE, nt("watched")).unwrap();
    sync();
    other.c.watch(MAC, &tk.id).unwrap();
    sync();
    pc.c.set_steps(ALICE, &tk.id, vec![step("one")]).unwrap();
    sync();
    let count = |e: &Env| {
        e.c.sweep_notices()
            .unwrap()
            .iter()
            .filter(|x| x.kind == NOTICE_WATCHED && x.to.iter().any(|t| t == MAC))
            .count()
    };
    assert_eq!(count(&pc), 0);
    assert_eq!(count(&other), 1);
}
