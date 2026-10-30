//! Port of I:internal/coord/thread_test.go.
#![allow(clippy::type_complexity)]

mod common;

use common::*;
use coord::rows::{TaskCommentRow, TaskMemberRow};
use coord::*;
use store::kinds;

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

const RECIPIENT: &str = "dev-b/x";

#[test]
fn test_gone_merge_either_order() {
    let (sent, swept) = (plus(fixed_now(), mins(1)), plus(fixed_now(), mins(5)));
    let primary = TaskCommentRow {
        comment_id: "msg_1".into(),
        msg_id: "msg_1".into(),
        by: ALICE.into(),
        to: RECIPIENT.into(),
        text: "hello".into(),
        files: 1,
        at: sent,
        ..TaskCommentRow::default()
    };
    let gone = TaskCommentRow {
        comment_id: "msg_1".into(),
        msg_id: "msg_1".into(),
        by: ALICE.into(),
        to: RECIPIENT.into(),
        text: "hello (resent)".into(),
        gone: true,
        at: swept,
        ..TaskCommentRow::default()
    };
    let base = hlc_at(plus(fixed_now(), mins(60)));
    let fold = |gone_first: bool| -> (Comment, Task) {
        let e = Env::on("dev-c");
        let task = e.c.create_task(CAROL, nt("t")).unwrap();
        let mut p = primary.clone();
        let mut g = gone.clone();
        p.task_id = task.id.clone();
        g.task_id = task.id.clone();
        let put_primary = || {
            e.remote(
                "dev-a",
                1,
                base + 10,
                &format!("agent:{ALICE}"),
                kinds::TASK_COMMENT,
                &p,
            );
        };
        let put_gone = || {
            e.remote("dev-b", 1, base + 5, "irori", kinds::TASK_COMMENT, &g);
        };
        if gone_first {
            put_gone();
            put_primary();
        } else {
            put_primary();
            put_gone();
        }
        let cs = e.c.comments(&task.id).unwrap();
        assert_eq!(cs.len(), 1, "gone_first={gone_first}");
        let c2 = e.reopen();
        assert_eq!(
            c2.comments(&task.id).unwrap(),
            cs,
            "gone_first={gone_first}"
        );
        let got = e.c.get_task(&task.id).unwrap();
        assert_eq!(
            c2.get_task(&task.id).unwrap(),
            got,
            "gone_first={gone_first}"
        );
        let a = e.c.activity(&task.id).unwrap();
        assert!(a.len() == 1 && a[0].what == "put the card up", "{a:?}");
        let mut cm = cs[0].clone();
        cm.task_id.clear();
        (cm, got)
    };
    let (a, ta) = fold(false);
    let (b, tb) = fold(true);
    assert_eq!(a, b);
    assert!(
        a.gone
            && a.device == "dev-a"
            && a.hlc == base + 10
            && a.seq == 1
            && a.text == "hello"
            && a.at.equal(&sent)
            && a.files == 1
            && a.to == RECIPIENT,
        "{a:?}"
    );
    assert!(
        ta.comments == 1
            && tb.comments == 1
            && ta.updated_at.equal(&sent)
            && tb.updated_at.equal(&sent)
    );
}

#[test]
fn test_fill_merge_either_order() {
    let (sent, passed) = (plus(fixed_now(), mins(1)), plus(fixed_now(), mins(3)));
    let primary = TaskCommentRow {
        comment_id: "msg_1".into(),
        msg_id: "msg_1".into(),
        by: ALICE.into(),
        to: RECIPIENT.into(),
        held: true,
        files: 1,
        at: sent,
        ..TaskCommentRow::default()
    };
    let fill = TaskCommentRow {
        comment_id: "msg_1".into(),
        msg_id: "msg_1".into(),
        by: ALICE.into(),
        to: RECIPIENT.into(),
        text: "hello".into(),
        files: 1,
        fill: true,
        at: passed,
        ..TaskCommentRow::default()
    };
    let base = hlc_at(plus(fixed_now(), mins(60)));
    let fold = |fill_first: bool| -> Comment {
        let e = Env::on("dev-c");
        let task = e.c.create_task(CAROL, nt("t")).unwrap();
        let mut p = primary.clone();
        let mut f = fill.clone();
        p.task_id = task.id.clone();
        f.task_id = task.id.clone();
        let mut forged = f.clone();
        forged.text = "forged".into();
        e.remote("dev-z", 1, base + 1, "irori", kinds::TASK_COMMENT, &forged);
        let put_primary = || {
            e.remote(
                "dev-a",
                1,
                base + 10,
                &format!("agent:{ALICE}"),
                kinds::TASK_COMMENT,
                &p,
            );
        };
        let put_fill = || {
            e.remote("dev-b", 1, base + 20, "irori", kinds::TASK_COMMENT, &f);
        };
        if fill_first {
            put_fill();
            put_primary();
        } else {
            put_primary();
            put_fill();
        }
        let cs = e.c.comments(&task.id).unwrap();
        assert_eq!(cs.len(), 1, "fill_first={fill_first}");
        assert_eq!(
            e.reopen().comments(&task.id).unwrap(),
            cs,
            "fill_first={fill_first}"
        );
        let mut cm = cs[0].clone();
        cm.task_id.clear();
        cm
    };
    let (a, b) = (fold(false), fold(true));
    assert_eq!(a, b);
    assert!(
        !a.held
            && a.text == "hello"
            && a.device == "dev-a"
            && a.hlc == base + 10
            && a.at.equal(&sent)
            && a.files == 1
            && !a.gone,
        "{a:?}"
    );
}

#[test]
fn test_refold_if_cut_drops_thread_entries() {
    let e = Env::new();
    let t = e.c.create_task(ALICE, nt("ship it")).unwrap();
    let task = e.c.claim_task(ALICE, &t.id).unwrap();
    let (mine, _) = e.c.add_comment(USER, &task.id, "from here", false).unwrap();
    const ZED: &str = "dev-b/zed";
    let actor = format!("agent:{ZED}");
    let at = hlc_at(plus(e.clk.now(), secs(1)));
    e.remote(
        "dev-b",
        1,
        at,
        &actor,
        kinds::TASK_COMMENT,
        &TaskCommentRow {
            task_id: task.id.clone(),
            comment_id: "c-b".into(),
            by: ZED.into(),
            text: "from b".into(),
            at: fixed_now(),
            ..TaskCommentRow::default()
        },
    );
    e.remote(
        "dev-b",
        2,
        at + 1,
        &actor,
        kinds::TASK_COMMENT,
        &TaskCommentRow {
            task_id: task.id.clone(),
            comment_id: "msg_b".into(),
            msg_id: "msg_b".into(),
            by: ZED.into(),
            to: ALICE.into(),
            text: "hi alice".into(),
            at: fixed_now(),
            ..TaskCommentRow::default()
        },
    );
    e.remote(
        "dev-b",
        3,
        at + 2,
        &actor,
        kinds::TASK_JOINED,
        &TaskMemberRow {
            task_id: task.id.clone(),
            who: ZED.into(),
            by: ZED.into(),
            at: fixed_now(),
            ..TaskMemberRow::default()
        },
    );
    assert_eq!(e.c.comments(&task.id).unwrap().len(), 3);
    let devs = || {
        let mut m = std::collections::BTreeMap::new();
        for a in e.c.activity(&task.id).unwrap() {
            *m.entry(a.device).or_insert(0) += 1;
        }
        m
    };
    assert_eq!(devs().get("dev-b"), Some(&2));
    let db = rusqlite::Connection::open(e._dir.path().join("irori.db")).unwrap();
    db.execute("DELETE FROM events WHERE device_id = 'dev-b'", [])
        .unwrap();
    drop(db);
    let vec = e.st.vector().unwrap();
    e.c.refold_if_cut(&vec).unwrap();
    let cs = e.c.comments(&task.id).unwrap();
    assert!(cs.len() == 1 && cs[0].id == mine.id, "{cs:?}");
    assert!(e.c.get_comment("msg_b").is_none());
    let d = devs();
    assert!(
        !d.contains_key("dev-b") && d.get("dev-a").is_some_and(|n| *n > 0),
        "{d:?}"
    );
    let got = e.c.get_task(&task.id).unwrap();
    assert!(got.comments == 1 && !got.holders.iter().any(|h| h == ZED));
}

#[test]
fn test_render_task_shows_messages() {
    let e = Env::new();
    let t = e.c.create_task(ALICE, nt("ship it")).unwrap();
    let task = e.c.claim_task(ALICE, &t.id).unwrap();
    let rec = |m: MessageEntry| {
        e.c.record_message(MessageEntry {
            task_id: task.id.clone(),
            ..m
        })
        .unwrap();
    };
    let msg = |id: &str, from: &str, to: &str, text: &str| MessageEntry {
        msg_id: id.into(),
        from: from.into(),
        to: to.into(),
        text: text.into(),
        ..MessageEntry::default()
    };
    rec(msg("msg_1", ALICE, BOB, "please review"));
    rec(msg("msg_2", BOB, CAROL, "carol?"));
    rec(MessageEntry {
        gone: true,
        writer: "irori".into(),
        ..msg("msg_2", BOB, CAROL, "carol?")
    });
    rec(MessageEntry {
        held: true,
        files: 2,
        ..msg("msg_3", CAROL, ALICE, "secret")
    });
    e.c.add_comment(USER, &task.id, "plain one", false).unwrap();
    let sh = search::Source::search_show(
        &e.c,
        &format!("task:{}", task.id),
        &search::ShowOptions::default(),
    )
    .unwrap();
    for s in [
        format!("{ALICE} -> {BOB}: please review
"),
        format!("{BOB} -> {CAROL}: carol? (recipient gone, left on the card)
"),
        format!(
            "{CAROL} -> {ALICE}: (not let through the wake policy yet; text not on the card) [2 files on msg:msg_3]
"
        ),
        format!("{USER}: plain one
"),
        "comments (4".to_string(),
    ] {
        assert!(sh.text.contains(&s), "show lacks {s:?}:
{}", sh.text);
    }
    assert!(
        !sh.text.contains("secret") && !sh.text.contains(&format!("{USER} -> ")),
        "show:
{}",
        sh.text
    );
}
