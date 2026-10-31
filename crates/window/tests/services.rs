//! The window's services on a scratch Core (tests/common), and the TS
//! shapes in frontend/src/gen/bindings.ts checked against the real
//! JSON of the values the commands send.

use std::collections::BTreeSet;

use irori_window::attachments::{self, Request};
use irori_window::settings::NotifyOptions;

mod common;

use common::*;

#[test]
fn board_cards_and_answers() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "Port the window".into(),
                objective: "Every command".into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &t.id).unwrap();
    let (_, bl) = co
        .raise(
            &r.key_a,
            &t.id,
            coord::NewBlocker {
                question: "Which crate?".into(),
                options: vec![coord::BlockerOption {
                    answer: "window".into(),
                    recommended: true,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();

    let cards = r.w.cards();
    assert_eq!(cards.len(), 1);
    let s = &cards[0];
    assert_eq!(
        (s.id.as_str(), s.title.as_str()),
        (t.id.as_str(), "Port the window")
    );
    assert_eq!(
        (s.open_blockers, s.blocker_question.as_str()),
        (1, "Which crate?")
    );
    assert_eq!(s.working.len(), 1);
    assert_eq!(s.working[0].name, "alpha");
    assert_eq!(s.holder.state, irori_window::cards::Holding::Idle);
    assert_eq!(r.w.decide_count(), 1);

    let d = r.w.card(&t.id).unwrap();
    assert_eq!(d.blockers.len(), 1);
    assert_eq!(d.blockers[0].options[0].answer, "window");
    assert!(d.blockers[0].options[0].recommended);
    assert!(d.thread.iter().any(|i| i.kind == "event"));
    // The JSON is the summary flattened, every list present.
    let j = serde_json::to_value(&d).unwrap();
    for k in [
        "id",
        "title",
        "working",
        "goal",
        "steps",
        "blockers",
        "prs",
        "thread",
        "subtaskList",
    ] {
        assert!(j.get(k).is_some(), "CardDetail has no {k}");
    }

    let a = r.w.answer(&t.id, &bl.id, 0, "", false).unwrap();
    assert_eq!(a.told.to, vec![r.key_a.clone()]);
    assert_eq!(a.card.blockers[0].state, "answered");
    assert_eq!(r.w.cards()[0].waiting, 1);

    let c = r.w.comment(&t.id, "looks right", "next_turn").unwrap();
    assert_eq!(c.comment.text, "looks right");
    assert_eq!(c.answers.len(), 1);
    assert_eq!(c.told.mode, "next_turn");
    assert!(r.w.comment(&t.id, "x", "loud").is_err());

    r.w.pin(&t.id, true).unwrap();
    assert!(r.w.cards()[0].pinned);
    assert_eq!(
        r.w.card("nope").unwrap_err(),
        "coord: no such task or lease: task \"nope\""
    );

    co.set_task_state(&r.key_a, &t.id, coord::COMPLETED, "done")
        .unwrap();
    assert!(r.w.cards().is_empty());
    let done = r.w.done_cards(0, "").unwrap();
    assert_eq!(done.cards.len(), 1);
    assert_eq!(done.next, "");
    let reopened = r.w.reopen(&t.id).unwrap();
    assert_eq!(reopened.task.id, t.id);
    assert_eq!(r.w.cards().len(), 1);
}

#[test]
fn done_cards_page() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let me = r.w.core.user_sender().key;
    for i in 0..3 {
        let t = co
            .create_task(
                &me,
                coord::NewTask {
                    title: format!("card {i}"),
                    objective: "o".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        co.set_task_state(&me, &t.id, coord::CANCELED, "").unwrap();
    }
    let p1 = r.w.done_cards(2, "").unwrap();
    assert_eq!(p1.cards.len(), 2);
    assert!(!p1.next.is_empty());
    let p2 = r.w.done_cards(2, &p1.next).unwrap();
    assert_eq!(p2.cards.len(), 1);
    assert_eq!(p2.next, "");
    let all: BTreeSet<String> = p1
        .cards
        .iter()
        .chain(&p2.cards)
        .map(|c| c.id.clone())
        .collect();
    assert_eq!(all.len(), 3);
    assert!(r.w.done_cards(2, "garbage").unwrap_err().contains("cursor"));
}

#[test]
fn asks_and_repos() {
    let r = Rig::new();
    let res =
        r.w.ask("Who can look at the tray?", "", false, &[], "")
            .unwrap();
    assert!(res.task.ask.is_some());
    let cards = r.w.cards();
    assert_eq!(cards[0].ask.as_ref().unwrap().state, "open");
    let closed = r.w.close_ask(&res.task.id, "never mind").unwrap();
    assert!(closed.ask.unwrap().outcome.is_some());
    // The sessions' cwd is a plain folder: no repo.
    assert!(r.w.ask_repos().is_empty());
}

#[test]
fn ask_with_a_file_lands_on_the_card() {
    use base64::Engine as _;
    let r = Rig::new();
    let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
    let f = irori_window::board::UploadFile {
        name: "shot.png".into(),
        data: base64::engine::general_purpose::STANDARD.encode(png),
    };
    // A board ask takes no files (its rows may name only attachments
    // offered on the board), nor a board this device doesn't hold.
    let e =
        r.w.ask(
            "See the picture",
            "",
            false,
            std::slice::from_ref(&f),
            "no-such-board",
        )
        .unwrap_err();
    assert!(e.contains("files"), "{e}");
    let e =
        r.w.ask("Anyone?", "", false, &[], "no-such-board")
            .unwrap_err();
    assert!(e.contains("no such board"), "{e}");
    let res = r.w.ask("See the picture", "", false, &[f], "").unwrap();
    let d = r.w.card(&res.task.id).unwrap();
    let files: Vec<_> = d
        .comments
        .iter()
        .flat_map(|c| c.attachments.clone())
        .collect();
    assert_eq!(files.len(), 1, "the ask's first comment carries the file");
    let sha = files[0].sha256.clone();

    let info = r.w.attachment_info(&sha).unwrap();
    assert_eq!(
        (info.name.as_str(), info.here, info.openable),
        ("shot.png", true, true)
    );
    assert!(info.url.ends_with(&format!("/irori/attachment/{sha}")));

    let get = |range: &str, inm: &str| {
        r.w.serve_attachment(&Request {
            method: "GET".into(),
            path: format!("{}{sha}", attachments::ROUTE),
            range: range.into(),
            if_none_match: inm.into(),
        })
    };
    let ok = get("", "");
    assert_eq!(ok.status, 200);
    assert_eq!(ok.body, png);
    assert_eq!(ok.get("Content-Type"), Some("image/png"));
    assert_eq!(
        ok.get("Content-Disposition"),
        Some("inline; filename=shot.png")
    );
    assert_eq!(ok.get("X-Content-Type-Options"), Some("nosniff"));
    assert_eq!(
        ok.get("Content-Security-Policy"),
        Some("default-src 'none'; sandbox")
    );
    assert_eq!(ok.get("ETag"), Some(format!("\"{sha}\"").as_str()));
    assert_eq!(get("bytes=0-3", "").status, 206);
    assert_eq!(get("", &format!("\"{sha}\"")).status, 304);
    let none = r.w.serve_attachment(&Request {
        method: "GET".into(),
        path: format!("{}{}", attachments::ROUTE, "0".repeat(64)),
        ..Request::default()
    });
    assert_eq!(none.status, 404);
    assert_eq!(none.get("Cache-Control"), Some("no-store"));
    let post = r.w.serve_attachment(&Request {
        method: "POST".into(),
        path: format!("{}{sha}", attachments::ROUTE),
        ..Request::default()
    });
    assert_eq!((post.status, post.get("Allow")), (405, Some("GET")));

    let saved = r.w.attachment_save(&sha).unwrap();
    assert!(saved.ends_with("shot.png"), "{saved}");
    assert_eq!(std::fs::read(&saved).unwrap(), png);
    // Same bytes: the same path, not "shot (2).png".
    assert_eq!(r.w.attachment_save(&sha).unwrap(), saved);
    assert_eq!(
        r.w.attachment_open(&sha).unwrap_err(),
        "this Irori has no window to open files from"
    );
}

#[test]
fn inbox_send_reply_and_policy() {
    let r = Rig::new();
    let m = r.w.inbox_send(&r.key_b, "", "hello beta").unwrap();
    assert_eq!(m.text, "hello beta");
    assert_eq!(
        r.w.inbox_send("", "", "x").unwrap_err(),
        "pick an agent or a repo to send to"
    );
    let list = r.w.inbox_list(0);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].to, r.key_b);
    assert_eq!(r.w.inbox_counts().unread, 1);

    // Beta answers the person; the person replies to that.
    let ib = r.w.core.inbox();
    let beta = inbox::Sender::new(&r.key_b, "beta", "agent");
    let (_, d) = ib.reply(&beta, &list[0].id, "a question back").unwrap();
    assert_eq!(d.to, r.w.core.user_sender().key);
    r.w.inbox_reply(&d.id, "an answer").unwrap();
    assert_eq!(r.w.inbox_get(&d.id).unwrap().state, "answered");
    assert_eq!(
        r.w.inbox_get("nope").unwrap_err(),
        "inbox: no such delivery or message: delivery \"nope\""
    );

    r.w.inbox_set_paused(true).unwrap();
    assert!(r.w.inbox_policy().paused);
    assert!(r.w.inbox_counts().paused);
    r.w.inbox_set_paused(false).unwrap();
    assert!(!r.w.inbox_policy().paused);
    assert!(r.w.inbox_held().is_empty());
    assert!(r.w.inbox_held_history(0).is_empty());
}

#[test]
fn roster_and_devices() {
    let r = Rig::new();
    let keys: BTreeSet<String> = r.w.agents().into_iter().map(|a| a.key).collect();
    assert!(keys.contains(&r.key_a) && keys.contains(&r.key_b));
    assert!(!r.w.recent(5).unwrap().is_empty());
    assert!(r.w.gone(0).unwrap().is_empty());
    assert_eq!(
        r.w.revive("not-a-session!").unwrap_err(),
        "not a session id"
    );
    assert_eq!(r.w.revive(SESS_A).unwrap_err(), "alpha is still running");

    let ds = r.w.devices();
    assert_eq!(ds.len(), 1);
    assert!(ds[0].this);
    assert_eq!(ds[0].state, "this");
    assert_eq!(ds[0].device_id, r.w.core.device_id());
    // No home server in this Core (Options::no_home): the Core's answer.
    let hs = r.w.home_server();
    assert!(!hs.can && !hs.on && !hs.running && !hs.suggest);
    assert!(r.w.set_home_server_awake(false).is_err());
    // D4: no TURN relay.
    assert_eq!(r.w.relay().state, "none");
    // Pairing is the Core's Pairer: Go's texts, nothing open yet.
    assert!(r.w.seams.pairing.status().code.is_empty());
    assert_eq!(
        r.w.seams
            .pairing
            .join("ABCD-EFGH", "no port here")
            .unwrap_err(),
        "the home server address is a host and port, like 203.0.113.7:24817"
    );
    // Remove device is the mesh Node's revoke: an unknown device is refused.
    let e = r.w.remove_device("dev-x").unwrap_err();
    assert!(!e.is_empty(), "{e}");
    assert!(r.w.boards_list().is_empty());
    assert_eq!(r.w.update_status().phase, "idle");
}

#[test]
fn settings_survive_a_restart() {
    let r = Rig::new();
    assert_eq!(r.w.get_notifications(), NotifyOptions::default());
    let got =
        r.w.set_notifications(NotifyOptions {
            sound: false,
            volume: 0.25,
            corner: "bottom-left".into(),
            ..NotifyOptions::default()
        })
        .unwrap();
    assert_eq!((got.sound, got.corner.as_str()), (false, "bottom-left"));
    // The one settings.json writer is the notify crate's.
    let again = notify::load(&r.data);
    assert_eq!(
        (again.sound, again.volume, again.corner.as_str()),
        (false, 0.25, "bottom-left")
    );
    let fresh = irori_window::settings::NotifierSeam::headless(&r.data);
    assert_eq!(irori_window::seams::Notifier::options(&fresh), got);
    // The default seam never shows a card.
    assert_eq!(
        r.w.test_notification().unwrap_err(),
        "this Irori shows no notifications"
    );
    assert!(!r.w.get_launch_at_login().unwrap());
    assert!(r.w.set_launch_at_login(true).is_err());
}

// ---- shapes ---------------------------------------------------------------------

/// The fields of `name` in the generated bindings: (field, required).
fn ts_fields(src: &str, name: &str) -> Vec<(String, bool)> {
    let head = format!("export type {name} = {{\n");
    let start = src
        .find(&head)
        .unwrap_or_else(|| panic!("no type {name} in bindings.ts"));
    let body = &src[start + head.len()..];
    let end = body.find("\n}").unwrap();
    body[..end]
        .lines()
        .filter_map(|l| {
            let (f, _) = l.trim().split_once(':')?;
            let name = f.trim_end_matches('?');
            // Doc comment lines are not fields.
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric()) {
                return None;
            }
            Some((name.to_string(), !f.ends_with('?')))
        })
        .collect()
}

/// Every key the value sends is in the TS type, and every field the type
/// requires is sent.
fn check_shape(src: &str, name: &str, v: &serde_json::Value) {
    let obj = v
        .as_object()
        .unwrap_or_else(|| panic!("{name}: not an object: {v}"));
    let fields = ts_fields(src, name);
    let known: BTreeSet<&str> = fields.iter().map(|(f, _)| f.as_str()).collect();
    for k in obj.keys() {
        assert!(
            known.contains(k.as_str()),
            "{name}: the JSON has {k}, the TS type doesn't"
        );
    }
    for (f, req) in &fields {
        assert!(
            !req || obj.contains_key(f),
            "{name}: the TS type requires {f}, the JSON has none"
        );
    }
}

#[test]
fn ts_shapes_match_the_json() {
    let src = std::fs::read_to_string(manifest_dir().join("../../frontend/src/gen/bindings.ts"))
        .unwrap()
        .replace("\r\n", "\n");
    let r = Rig::new();
    let co = r.w.core.coord();
    let a = r.key_a.clone();
    let t = co
        .create_task(
            &a,
            coord::NewTask {
                title: "Shapes".into(),
                objective: "o".into(),
                done_criteria: "d".into(),
                links: vec!["https://github.com/a/b/pull/3".into()],
                steps: vec![coord::Step {
                    text: "one".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();
    co.claim_task(&a, &t.id).unwrap();
    co.raise(
        &a,
        &t.id,
        coord::NewBlocker {
            question: "Q?".into(),
            options: vec![coord::BlockerOption {
                answer: "yes".into(),
                why: "because".into(),
                recommended: true,
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let _ = r.w.comment(&t.id, "a comment", "none").unwrap();
    let task = co.get_task(&t.id).unwrap();

    let j = |v: &dyn erased::Ser| v.json();
    check_shape(&src, "Task", &j(&task));
    check_shape(&src, "Blocker", &j(&task.blockers[0]));
    check_shape(&src, "BlockerOption", &j(&task.blockers[0].options[0]));
    check_shape(&src, "Link", &j(&task.links[0]));
    check_shape(&src, "Step", &j(&task.steps[0]));
    check_shape(&src, "Comment", &j(&co.comments(&t.id).unwrap()[0]));
    let card = r.w.card(&t.id).unwrap();
    let cj = j(&card);
    check_shape(&src, "CardBlocker", &cj["blockers"][0]);
    check_shape(&src, "CardOption", &cj["blockers"][0]["options"][0]);
    check_shape(&src, "AgentRef", &cj["working"][0]);
    check_shape(&src, "CardThreadItem", &cj["thread"][0]);
    check_shape(&src, "CardPR", &cj["prs"][0]);
    let mut summary = cj.clone();
    for k in [
        "goal",
        "doneWhen",
        "note",
        "steps",
        "blockers",
        "prs",
        "links",
        "recent",
        "comments",
        "thread",
        "threadEarlier",
        "historyTrimmed",
        "subtaskList",
    ] {
        summary.as_object_mut().unwrap().remove(k);
    }
    check_shape(&src, "CardSummary", &summary);

    let m = r.w.inbox_send(&r.key_b, "", "hi").unwrap();
    check_shape(&src, "Message", &j(&m));
    let d = &r.w.inbox_list(0)[0];
    check_shape(&src, "Delivery", &j(d));
    check_shape(&src, "Sender", &j(&d.message.from));
    check_shape(&src, "Address", &j(&d.message.to));
    check_shape(&src, "Policy", &j(&r.w.inbox_policy()));
    check_shape(&src, "InboxCounts", &j(&r.w.inbox_counts()));

    let agents = r.w.agents();
    check_shape(&src, "AgentCard", &j(&agents[0]));
    check_shape(&src, "Identity", &j(r.w.core.device()));
    let ev = &r.w.recent(1).unwrap()[0];
    check_shape(&src, "Event", &j(ev));
    check_shape(&src, "DeviceView", &j(&r.w.devices()[0]));
    check_shape(&src, "HomeServerView", &j(&r.w.home_server()));
    check_shape(&src, "NotifyOptions", &j(&r.w.get_notifications()));
}

/// Serializing through a trait object, so the shape list reads as one.
mod erased {
    pub trait Ser {
        fn json(&self) -> serde_json::Value;
    }
    impl<T: serde::Serialize> Ser for T {
        fn json(&self) -> serde_json::Value {
            serde_json::to_value(self).unwrap()
        }
    }
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}

/// Card #57: the person closes any card by hand (Done or Canceled, with a
/// note) and deletes any card; Closed lists done, canceled and failed.
#[test]
fn close_and_delete_by_hand() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let new = |title: &str| {
        co.create_task(
            &r.key_a,
            coord::NewTask {
                title: title.into(),
                ..Default::default()
            },
        )
        .unwrap()
    };
    let (a, b, c, d) = (new("a"), new("b"), new("c"), new("d"));
    co.claim_task(&r.key_a, &a.id).unwrap();
    // An agent's card the person never touched: they may still close it.
    let t = r.w.close_card(&a.id, coord::COMPLETED, "shipped").unwrap();
    assert_eq!(
        (t.state.as_str(), t.note.as_str()),
        (coord::COMPLETED, "shipped")
    );
    r.w.close_card(&b.id, coord::CANCELED, "").unwrap();
    co.set_task_state(&r.key_a, &c.id, coord::FAILED, "broke")
        .unwrap();
    assert!(
        r.w.close_card(&d.id, "working", "")
            .unwrap_err()
            .contains("completed or canceled")
    );
    let closed: Vec<String> =
        r.w.done_cards(0, "")
            .unwrap()
            .cards
            .iter()
            .map(|s| s.state.clone())
            .collect();
    let mut want = vec![coord::COMPLETED, coord::CANCELED, coord::FAILED];
    let mut got = closed.clone();
    got.sort();
    want.sort();
    assert_eq!(got, want);
    // Delete: open or closed, the card leaves every list.
    r.w.delete_card(&d.id).unwrap();
    r.w.delete_card(&a.id).unwrap();
    assert!(r.w.cards().iter().all(|s| s.id != d.id));
    assert!(
        r.w.done_cards(0, "")
            .unwrap()
            .cards
            .iter()
            .all(|s| s.id != a.id)
    );
    assert!(r.w.card(&d.id).is_err());
    assert!(r.w.delete_card(&d.id).is_err());
}

/// Card #57 on a shared board: Close acts in the board's log as the
/// person; Delete is refused (a shared card is withdrawn, §5.3).
#[test]
fn close_and_delete_on_a_shared_board() {
    let r = Rig::new();
    let c = &r.w.core;
    let b = c.create_board("one", "one").unwrap();
    let co = c.board_coord(&b).unwrap();
    let me = c.board_key(&b, &c.user_sender().key).unwrap();
    let t = co
        .create_task(
            &me,
            coord::NewTask {
                title: "shared work".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let e = r.w.delete_card(&t.id).unwrap_err();
    assert!(e.contains("withdrawn, not deleted"), "{e}");
    assert!(co.get_task(&t.id).is_some());
    let closed =
        r.w.close_card(&t.id, coord::COMPLETED, "done here")
            .unwrap();
    assert_eq!(closed.state, coord::COMPLETED);
    assert_eq!(co.get_task(&t.id).unwrap().state, coord::COMPLETED);
    assert!(
        c.coord().get_task(&t.id).is_none(),
        "it went to the private log"
    );
}

/// Card #66 s4: the Boards dialog dedicates one of the person's agents as
/// their reviewer on a board, for the whole board or one repo, and takes
/// it off again (design §10).
#[test]
fn boards_set_reviewer() {
    let r = Rig::new();
    let b = r.w.core.create_board("Reviews", "rev").unwrap();
    let s = r.w.boards_settings(&b).unwrap();
    assert!(s.reviewers.is_empty());
    assert!(
        s.agents.iter().any(|a| a.agent == r.key_a),
        "the person's agents to pick from: {:?}",
        s.agents
    );
    let s = r.w.boards_set_reviewer(&b, &r.key_a, "").unwrap();
    assert_eq!(s.reviewers.len(), 1);
    assert_eq!(
        (s.reviewers[0].agent.as_str(), s.reviewers[0].repo.as_str()),
        (r.key_a.as_str(), "")
    );
    let s =
        r.w.boards_set_reviewer(&b, &r.key_b, "remote:github.com/example-org/example-app")
            .unwrap();
    assert_eq!(s.reviewers.len(), 2);
    let s = r.w.boards_set_reviewer(&b, "", "").unwrap();
    assert_eq!(s.reviewers.len(), 1);
    assert_eq!(s.reviewers[0].agent, r.key_b);
    assert!(
        r.w.boards_set_reviewer("no-such-board", &r.key_a, "")
            .is_err()
    );
}
