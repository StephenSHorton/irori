//! Port of I:internal/coord/blocker_test.go.
#![allow(clippy::type_complexity)]

mod common;

use std::sync::Arc;

use common::*;
use coord::rows::{BlockerRow, TaskCommentRow, TaskStateRow};
use coord::*;
use store::kinds;

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

fn claimed(e: &Env) -> Task {
    let t = e.c.create_task(ALICE, nt("ship it")).unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap()
}

fn opt(answer: &str) -> BlockerOption {
    BlockerOption {
        answer: answer.into(),
        ..BlockerOption::default()
    }
}

fn raised_on(e: &Env) -> Task {
    let task = claimed(e);
    e.c.raise(
        ALICE,
        &task.id,
        NewBlocker {
            question: "Which adapter?".into(),
            options: vec![opt("Wintun"), opt("TAP-Windows")],
            ..NewBlocker::default()
        },
    )
    .unwrap();
    task
}

fn blocker_of(c: &Coord, id: &str, bid: &str) -> (Task, Blocker) {
    let t = c.get_task(id).unwrap();
    let b = t
        .blockers
        .iter()
        .find(|b| b.id == bid)
        .cloned()
        .unwrap_or_else(|| panic!("no blocker {bid}: {:?}", t.blockers));
    (t, b)
}

fn reply(c: &Coord, by: &str, id: &str, bid: &str, text: &str, missing: &[&str]) {
    let (_, n) = c
        .comment_on(
            by,
            id,
            CommentInput {
                text: text.into(),
                blocker_id: bid.into(),
                missing: missing.iter().map(|s| s.to_string()).collect(),
                ..CommentInput::default()
            },
        )
        .unwrap();
    assert!(
        n.to.is_empty() && n.text.is_empty(),
        "a blocker reply routed a notice: {n:?}"
    );
}

fn text(t: &str) -> BlockerAnswer {
    BlockerAnswer {
        text: t.into(),
        ..BlockerAnswer::default()
    }
}

fn resolve(t: &str) -> BlockerAnswer {
    BlockerAnswer {
        text: t.into(),
        resolve: true,
        ..BlockerAnswer::default()
    }
}

#[test]
fn test_blocker_states() {
    let e = Env::new();
    let task = raised_on(&e);
    let ans = |a: BlockerAnswer| {
        e.c.answer(USER, &task.id, "b1", a).unwrap();
    };
    type Do<'a> = Box<dyn Fn() + 'a>;
    let steps: Vec<(&str, Do, &str, &str, Vec<&str>, &str)> = vec![
        ("raised", Box::new(|| {}), BLOCKER_OPEN, "", vec![], ""),
        (
            "an agent adds context",
            Box::new(|| {
                reply(
                    &e.c,
                    CAROL,
                    &task.id,
                    "b1",
                    "Radmin isn't open source.",
                    &["ignored before an answer"],
                )
            }),
            BLOCKER_OPEN,
            "",
            vec![],
            "Radmin isn't open source.",
        ),
        (
            "the person asks back",
            Box::new(|| ans(text("which is signed?"))),
            BLOCKER_ANSWERED,
            ALICE,
            vec![],
            "which is signed?",
        ),
        (
            "the person picks",
            Box::new(|| {
                ans(BlockerAnswer {
                    pick: 2,
                    ..BlockerAnswer::default()
                })
            }),
            BLOCKER_ANSWERED,
            ALICE,
            vec![],
            "TAP-Windows",
        ),
        (
            "the raiser replies",
            Box::new(|| {
                reply(
                    &e.c,
                    ALICE,
                    &task.id,
                    "b1",
                    "TAP needs a port.",
                    &["which port"],
                )
            }),
            BLOCKER_REPLIED,
            "",
            vec!["which port"],
            "TAP needs a port.",
        ),
        (
            "the person again",
            Box::new(|| ans(text("4242"))),
            BLOCKER_ANSWERED,
            ALICE,
            vec![],
            "4242",
        ),
    ];
    for (name, d, status, sent_to, missing, last) in steps {
        d();
        let (got, b) = blocker_of(&e.c, &task.id, "b1");
        assert!(
            b.status == status
                && b.sent_to == sent_to
                && b.missing == missing
                && b.open()
                && got.state == BLOCKED,
            "{name}: {b:?}"
        );
        if !last.is_empty() {
            let cs = e.c.comments(&task.id).unwrap();
            let l = cs.last().unwrap();
            assert!(l.text == last && l.blocker_id == "b1", "{name}");
        }
    }
    let cs = e.c.comments(&task.id).unwrap();
    assert!(
        cs[2].pick == 2 && cs[2].to == ALICE && cs[2].by == USER,
        "{:?}",
        cs[2]
    );
    let acts = e.c.activity(&task.id).unwrap();
    assert!(
        acts[0].what == "answered blocker b1"
            && acts[0].blocker_id == "b1"
            && acts[1].what == "replied on blocker b1"
    );
    let sh = search::Source::search_show(
        &e.c,
        &format!("task:{}", task.id),
        &search::ShowOptions::default(),
    )
    .unwrap();
    for s in [
        format!("(answered, waiting on {ALICE}"),
        format!("{USER} on b1: TAP-Windows (option 2)"),
        format!("{ALICE} on b1: TAP needs a port. [missing: which port]"),
    ] {
        assert!(
            sh.text.contains(&s),
            "show lacks {s:?}:
{}",
            sh.text
        );
    }
    let (got, n) =
        e.c.answer(ALICE, &task.id, "b1", resolve("using port 4242"))
            .unwrap();
    let r = got.blockers[0].resolved.clone().unwrap();
    assert!(
        r.answer == "4242"
            && r.note == "using port 4242"
            && r.option == -1
            && r.by == ALICE
            && !r.closed
            && got.state == WORKING,
        "{r:?}"
    );
    assert!(
        n.text.contains("resolved blocker b1")
            && n.text.contains("using port 4242")
            && n.text.contains("working again")
    );
    assert_eq!(
        n.text,
        format!(
            "{ALICE} resolved blocker b1 on \"ship it\" (#1, task:{id}): using port 4242\nQuestion: Which adapter?\nThe person's answer: 4242\nNo blockers remain: it is working again.\nshow task:{id} for the card, its comments and links.",
            id = task.id
        )
    );
    assert_eq!(
        e.c.activity(&task.id).unwrap()[0].what,
        "resolved “Which adapter?”: using port 4242"
    );
    assert_eq!(
        e.c.list_tasks(&TaskFilter::default()),
        e.reopen().list_tasks(&TaskFilter::default())
    );
    let e2 = Env::new();
    let t2 = raised_on(&e2);
    e2.c.answer(
        USER,
        &t2.id,
        "b1",
        BlockerAnswer {
            pick: 1,
            ..BlockerAnswer::default()
        },
    )
    .unwrap();
    let (got, _) = e2.c.answer(ALICE, &t2.id, "b1", resolve("on it")).unwrap();
    let r = got.blockers[0].resolved.clone().unwrap();
    assert!(r.answer == "Wintun" && r.option == 0 && r.note == "on it");
}

fn two_owners() -> Env {
    let owner: KeyFn = Arc::new(|k: &str| k == USER || k == "user:dev-b");
    Env::with("dev-a", Some(owner))
}

#[test]
fn test_blocker_spoof_ignored() {
    let e = two_owners();
    let other = "user:dev-b";
    let task = raised_on(&e);
    let at = hlc_at(plus(fixed_now(), mins(1)));
    let row = |id: &str, by: &str, text: &str| TaskCommentRow {
        task_id: task.id.clone(),
        comment_id: id.into(),
        by: by.into(),
        text: text.into(),
        blocker_id: "b1".into(),
        at: fixed_now(),
        ..TaskCommentRow::default()
    };
    let check = |c: &Coord, want: &str, comments: usize, ignored: usize| {
        let (_, b) = blocker_of(c, &task.id, "b1");
        assert_eq!(b.status, want);
        assert_eq!(c.comments(&task.id).unwrap().len(), comments);
        assert_eq!(c.ignored_rows(), ignored);
    };
    e.remote(
        "dev-b",
        1,
        at + 1,
        "agent:dev-b/x",
        kinds::TASK_COMMENT,
        &row("c1", other, "as the person"),
    );
    check(&e.c, BLOCKER_OPEN, 1, 0);
    e.remote(
        "dev-b",
        2,
        at + 2,
        "user",
        kinds::TASK_COMMENT,
        &row("c2", other, "yes"),
    );
    check(&e.c, BLOCKER_ANSWERED, 2, 0);
    e.remote(
        "dev-b",
        3,
        at + 3,
        "agent:dev-b/x",
        kinds::TASK_COMMENT,
        &row("c3", ALICE, "spoofed reply"),
    );
    check(&e.c, BLOCKER_ANSWERED, 2, 1);
    check(&e.reopen(), BLOCKER_ANSWERED, 2, 1);
}

#[test]
fn test_blocker_comment_before_raise() {
    let e = two_owners();
    let task = claimed(&e);
    let at = hlc_at(plus(fixed_now(), mins(1)));
    e.remote(
        "dev-b",
        1,
        at + 2,
        "user",
        kinds::TASK_COMMENT,
        &TaskCommentRow {
            task_id: task.id.clone(),
            comment_id: "c1".into(),
            by: "user:dev-b".into(),
            to: "dev-c/raiser".into(),
            text: "go".into(),
            blocker_id: "b1".into(),
            at: fixed_now(),
            ..TaskCommentRow::default()
        },
    );
    let got = e.c.get_task(&task.id).unwrap();
    assert!(got.blockers.is_empty() && got.comments == 1);
    e.remote(
        "dev-c",
        1,
        at + 1,
        "agent:dev-c/raiser",
        kinds::TASK_STATUS,
        &TaskStateRow {
            task_id: task.id.clone(),
            state: BLOCKED.into(),
            reason: "Ship?".into(),
            by: "dev-c/raiser".into(),
            at: fixed_now(),
            blocker: Some(BlockerRow {
                id: "b1".into(),
                question: "Ship?".into(),
                on: Some(BlockedBy {
                    kind: BLOCKED_ON_PERSON.into(),
                    reference: String::new(),
                }),
                ..BlockerRow::default()
            }),
            ..TaskStateRow::default()
        },
    );
    for c in [&e.c, &e.reopen()] {
        let (_, b) = blocker_of(c, &task.id, "b1");
        assert!(
            b.status == BLOCKER_ANSWERED && b.sent_to == "dev-c/raiser",
            "{b:?}"
        );
    }
}

#[test]
fn test_resolve_needs_note_after_answer() {
    let e = Env::new();
    let task = raised_on(&e);
    let (got, _) =
        e.c.answer(
            CAROL,
            &task.id,
            "b1",
            BlockerAnswer {
                pick: 1,
                resolve: true,
                ..BlockerAnswer::default()
            },
        )
        .unwrap();
    let r = got.blockers[0].resolved.clone().unwrap();
    assert!(r.answer == "Wintun" && r.note.is_empty() && r.option == 0);
    let e = Env::new();
    let task = raised_on(&e);
    e.c.answer(USER, &task.id, "b1", text("TAP, on 4242"))
        .unwrap();
    for a in [
        BlockerAnswer {
            pick: 1,
            resolve: true,
            ..BlockerAnswer::default()
        },
        resolve("  "),
        resolve("two\nlines"),
        resolve(&"n".repeat(MAX_ANSWER + 1)),
        text("an agent can't answer"),
    ] {
        want_err(e.c.answer(ALICE, &task.id, "b1", a), ErrorKind::Invalid);
    }
    let (_, b) = blocker_of(&e.c, &task.id, "b1");
    assert!(b.open() && b.status == BLOCKER_ANSWERED);
    let (got, _) =
        e.c.answer(
            USER,
            &task.id,
            "b1",
            BlockerAnswer {
                resolve: true,
                ..BlockerAnswer::default()
            },
        )
        .unwrap();
    let r = got.blockers[0].resolved.clone().unwrap();
    assert!(
        r.answer == "TAP, on 4242" && r.note.is_empty() && r.by == USER && got.state == WORKING
    );
    let ci = |by: &str, t: &str, missing: usize| {
        e.c.comment_on(
            by,
            &task.id,
            CommentInput {
                text: t.into(),
                blocker_id: "b1".into(),
                missing: vec!["m".into(); missing],
                ..CommentInput::default()
            },
        )
    };
    want_err(ci(ALICE, "late", 0), ErrorKind::BadState);
    want_err(ci(USER, "mine", 0), ErrorKind::Invalid);
    want_err(ci(ALICE, "x", MAX_MISSING + 1), ErrorKind::Invalid);
}

#[test]
fn test_answer_notice_recipients() {
    let e = Env::new();
    let task = e.c.create_task(ALICE, nt("db")).unwrap();
    e.c.claim_task(BOB, &task.id).unwrap();
    e.c.raise(
        ALICE,
        &task.id,
        NewBlocker {
            question: "which db?".into(),
            ..NewBlocker::default()
        },
    )
    .unwrap();
    let (got, n) =
        e.c.answer(
            USER,
            &task.id,
            "b1",
            BlockerAnswer {
                wake: true,
                ..text("postgres")
            },
        )
        .unwrap();
    assert!(got.state == BLOCKED && n.kind == NOTICE_ANSWERED && n.to == [ALICE, BOB] && n.wake);
    for s in [
        "Answer: postgres".to_string(),
        "It is not resolved yet.".into(),
        format!("board_unblock task_id={} blocker_id=b1 note=", task.id),
        format!("board_comment task_id={} blocker_id=b1", task.id),
    ] {
        assert!(n.text.contains(&s), "{s:?}: {}", n.text);
    }
    assert_eq!(
        n.text,
        format!(
            "{USER} answered the blocker on \"db\" (#1, task:{id}, blocker b1).\nQuestion: which db?\nAnswer: postgres\nIt is not resolved yet. When you have what you need: board_unblock task_id={id} blocker_id=b1 note=\"<one line on what you will do now>\". If the answer doesn't settle it: board_comment task_id={id} blocker_id=b1 with what is still missing.\nshow task:{id} for the card, its comments and links.",
            id = task.id
        )
    );
    let (got, n) =
        e.c.answer(ALICE, &task.id, "b1", resolve("migrating"))
            .unwrap();
    assert!(
        got.state == WORKING
            && n.to == [BOB]
            && n.text.contains(&format!("{ALICE} resolved blocker b1"))
    );
}

#[test]
fn test_needs_you_by_blocker_state() {
    let e = Env::new();
    let task = raised_on(&e);
    let needs = |viewer: &str| {
        e.c.board(&BoardFilter {
            viewer: viewer.into(),
            ..BoardFilter::default()
        })
        .needs_you
        .iter()
        .any(|c| c.task.id == task.id)
    };
    let check = |when: &str, person: bool, raiser: bool| {
        assert!(
            needs(USER) == person && needs(ALICE) == raiser && !needs(BOB),
            "{when}"
        );
    };
    check("open", true, false);
    e.c.answer(USER, &task.id, "b1", text("which one is signed?"))
        .unwrap();
    check("answered", false, true);
    reply(&e.c, ALICE, &task.id, "b1", "Wintun is.", &[]);
    check("replied", true, false);
}

#[test]
fn test_old_peer_resolve() {
    let e = Env::new();
    let task = raised_on(&e);
    e.c.answer(USER, &task.id, "b1", text("yes")).unwrap();
    let raw = format!(
        r#"{{"taskId":"{}","blockerId":"b1","answer":"fine","option":-1,"by":"user:dev-b","restored":"working","at":"2026-10-26T12:00:00Z"}}"#,
        task.id
    );
    e.remote(
        "dev-b",
        1,
        hlc_at(plus(fixed_now(), mins(1))),
        "user",
        kinds::BLOCKER_RESOLVED,
        &gocompat::json::RawJson(raw.into_bytes()),
    );
    for c in [&e.c, &e.reopen()] {
        let (got, b) = blocker_of(c, &task.id, "b1");
        let r = b.resolved.unwrap();
        assert!(r.answer == "fine" && r.note.is_empty() && got.state == WORKING);
    }
    assert_eq!(
        e.c.activity(&task.id).unwrap()[0].what,
        "answered “Which adapter?”: fine"
    );
}

#[test]
fn test_reblock_asks_afresh() {
    let person = || {
        Some(BlockedBy {
            kind: BLOCKED_ON_PERSON.into(),
            reference: String::new(),
        })
    };
    let e = Env::new();
    let task = claimed(&e);
    let block = |e: &Env, id: &str, reason: &str| {
        e.c.block(ALICE, id, reason, person()).unwrap();
    };
    block(&e, &task.id, "Which DB?");
    e.c.answer(USER, &task.id, "b1", text("Postgres")).unwrap();
    let (_, b) = blocker_of(&e.c, &task.id, "b1");
    assert!(b.status == BLOCKER_ANSWERED && b.sent_to == ALICE);
    block(&e, &task.id, "Which region?");
    for c in [&e.c, &e.reopen()] {
        let (got, b) = blocker_of(c, &task.id, "b1");
        assert!(
            got.blockers.len() == 1
                && b.question == "Which region?"
                && b.status == BLOCKER_OPEN
                && b.sent_to.is_empty()
                && b.open(),
            "{:?}",
            got.blockers
        );
        for v in [USER, ALICE] {
            let inn = c
                .board(&BoardFilter {
                    viewer: v.into(),
                    ..BoardFilter::default()
                })
                .needs_you
                .iter()
                .any(|cd| cd.task.id == task.id);
            assert_eq!(inn, v == USER, "{v}");
        }
    }
    e.c.answer(USER, &task.id, "b1", text("eu-west")).unwrap();
    let (got, _) =
        e.c.answer(ALICE, &task.id, "b1", resolve("deploying to eu-west"))
            .unwrap();
    let r = got.blockers[0].resolved.clone().unwrap();
    assert!(r.answer == "eu-west" && r.note == "deploying to eu-west" && got.state == WORKING);
    assert_eq!(
        e.c.list_tasks(&TaskFilter::default()),
        e.reopen().list_tasks(&TaskFilter::default())
    );
    let e = Env::new();
    let task = claimed(&e);
    block(&e, &task.id, "Which DB?");
    e.c.answer(USER, &task.id, "b1", text("Postgres")).unwrap();
    block(&e, &task.id, "Which region?");
    let (got, _) =
        e.c.answer(ALICE, &task.id, "b1", resolve("eu-west, it's closest"))
            .unwrap();
    let r = got.blockers[0].resolved.clone().unwrap();
    assert!(r.answer == "eu-west, it's closest" && r.note.is_empty());
}

#[test]
fn test_reblock_from_old_peer() {
    let e = Env::new();
    let task = claimed(&e);
    e.c.block(ALICE, &task.id, "Which DB?", None).unwrap();
    e.c.answer(USER, &task.id, "b1", text("Postgres")).unwrap();
    let at = hlc_at(plus(fixed_now(), mins(1)));
    let row = |by: &str, reason: &str| TaskStateRow {
        task_id: task.id.clone(),
        state: BLOCKED.into(),
        reason: reason.into(),
        by: by.into(),
        at: fixed_now(),
        ..TaskStateRow::default()
    };
    e.remote(
        "dev-b",
        1,
        at + 2,
        "agent:dev-b/x",
        kinds::TASK_STATUS,
        &row("dev-b/x", "Which region?"),
    );
    e.remote(
        "dev-c",
        1,
        at + 1,
        "agent:dev-c/y",
        kinds::TASK_STATUS,
        &row("dev-c/y", "Which zone?"),
    );
    for c in [&e.c, &e.reopen()] {
        let (got, b) = blocker_of(c, &task.id, "b1");
        assert!(
            got.blockers.len() == 1
                && b.question == "Which region?"
                && b.status == BLOCKER_OPEN
                && b.sent_to.is_empty()
                && b.raised_by == "dev-b/x",
            "{:?}",
            got.blockers
        );
    }
}

#[test]
fn test_blocker_links_and_files() {
    let e = Env::new();
    let task = claimed(&e);
    let sha = "ab".repeat(32);
    let file = FileRef {
        sha256: sha.clone(),
        name: "shot.png".into(),
        size: 12,
        mime: "image/png".into(),
        device: "dev-a".into(),
    };
    let links: Vec<String> = vec![
        "https://huggingface.co/facebook/dinov3-vitl16-pretrain-lvd1689m".into(),
        "docs/models.md".into(),
        " ".into(),
        "docs/models.md".into(),
    ];
    let question = "Request DINOv3 access?";
    let nb = NewBlocker {
        question: question.into(),
        context: "open huggingface.co/facebook/dinov3 and click Request access".into(),
        links: links.clone(),
        files: vec![file.clone()],
        options: vec![
            BlockerOption {
                link: Some(Link {
                    reference: "https://huggingface.co/settings/gated-repos".into(),
                    ..Link::default()
                }),
                recommended: true,
                ..opt("I requested it")
            },
            opt("Use DINOv2 instead"),
        ],
        ..NewBlocker::default()
    };
    let (_, b) = e.c.raise(ALICE, &task.id, nb).unwrap();
    let check = |c: &Coord, want_links: usize| {
        let (_, got) = blocker_of(c, &task.id, &b.id);
        assert!(
            got.links.len() == want_links
                && got.links[0].kind == LINK_URL
                && got.links[0].label == "huggingface.co",
            "{:?}",
            got.links
        );
        assert!(got.links[1].kind == LINK_PATH && got.links[1].reference == "docs/models.md");
        assert_eq!(got.attachments, std::slice::from_ref(&file));
        let l = got.options[0].link.as_ref().unwrap();
        assert!(l.kind == LINK_URL && l.label == "huggingface.co" && got.options[1].link.is_none());
        assert_eq!(c.comment_attachment(&sha), Some(file.clone()));
    };
    check(&e.c, 2);
    check(&e.reopen(), 2);
    let n = e.rows(kinds::TASK_STATUS).len();
    let (_, again) =
        e.c.raise(
            ALICE,
            &task.id,
            NewBlocker {
                question: question.into(),
                links: links[..1].to_vec(),
                ..NewBlocker::default()
            },
        )
        .unwrap();
    assert_eq!(again.id, b.id);
    assert_eq!(e.rows(kinds::TASK_STATUS).len(), n);
    let (_, again) =
        e.c.raise(
            ALICE,
            &task.id,
            NewBlocker {
                question: question.into(),
                links: vec!["https://example.com/x".into()],
                ..NewBlocker::default()
            },
        )
        .unwrap();
    assert!(
        again.id == b.id
            && again.links.len() == 3
            && again.links[2].reference == "https://example.com/x"
    );
    assert_eq!(e.c.get_task(&task.id).unwrap().open_blockers().len(), 1);
    check(&e.reopen(), 3);
    for bad in [
        NewBlocker {
            question: "q1".into(),
            links: vec!["javascript:alert(1)//x://y".into()],
            ..NewBlocker::default()
        },
        NewBlocker {
            question: "q2".into(),
            links: vec!["ftp://example.com/x".into()],
            ..NewBlocker::default()
        },
        NewBlocker {
            question: "q3".into(),
            options: vec![BlockerOption {
                link: Some(Link {
                    reference: "file:///etc/passwd".into(),
                    ..Link::default()
                }),
                ..opt("a")
            }],
            ..NewBlocker::default()
        },
        NewBlocker {
            question: "q4".into(),
            links: vec!["a".into()],
            files: vec![file.clone(); MAX_ATTACHMENTS + 1],
            ..NewBlocker::default()
        },
    ] {
        assert!(e.c.raise(ALICE, &task.id, bad).is_err());
    }
    let many: Vec<String> = (0..=MAX_BLOCKER_LINKS)
        .map(|i| format!("docs/{}", "x".repeat(i + 1)))
        .collect();
    assert!(
        e.c.raise(
            ALICE,
            &task.id,
            NewBlocker {
                question: "q5".into(),
                links: many.clone(),
                ..NewBlocker::default()
            }
        )
        .is_err()
    );
    let (_, full) =
        e.c.raise(
            ALICE,
            &task.id,
            NewBlocker {
                question: question.into(),
                links: many[..MAX_BLOCKER_LINKS - 3].to_vec(),
                ..NewBlocker::default()
            },
        )
        .unwrap();
    assert_eq!(full.links.len(), MAX_BLOCKER_LINKS);
    let n = e.rows(kinds::TASK_STATUS).len();
    want_err(
        e.c.raise(
            ALICE,
            &task.id,
            NewBlocker {
                question: question.into(),
                links: many[MAX_BLOCKER_LINKS - 3..].to_vec(),
                ..NewBlocker::default()
            },
        ),
        ErrorKind::Invalid,
    );
    assert_eq!(e.rows(kinds::TASK_STATUS).len(), n);
    e.c.raise(
        ALICE,
        &task.id,
        NewBlocker {
            question: question.into(),
            links: many[..2].to_vec(),
            ..NewBlocker::default()
        },
    )
    .unwrap();
}
