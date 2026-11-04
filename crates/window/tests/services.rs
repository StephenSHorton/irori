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

    let a = r.w.answer(&t.id, &bl.id, 0, "", false, &[]).unwrap();
    assert_eq!(a.told.to, vec![r.key_a.clone()]);
    assert_eq!(a.card.blockers[0].state, "answered");
    assert_eq!(r.w.cards()[0].waiting, 1);

    let c =
        r.w.comment(&t.id, "looks right", "next_turn", &[], &[])
            .unwrap();
    assert_eq!(c.comment.text, "looks right");
    assert_eq!(c.answers.len(), 1);
    assert_eq!(c.told.mode, "next_turn");
    assert!(r.w.comment(&t.id, "x", "loud", &[], &[]).is_err());

    r.w.pin(&t.id, true).unwrap();
    assert!(r.w.cards()[0].pinned);
    assert_eq!(
        r.w.card("nope").unwrap_err(),
        "coord: no such task or lease: task \"nope\""
    );

    // The agent says it's done: the card waits on the person's Done
    // (card #112), shown as ready.
    co.set_task_state(&r.key_a, &t.id, coord::COMPLETED, "done")
        .unwrap();
    let cards = r.w.cards();
    assert_eq!(cards.len(), 1);
    assert!(cards[0].ready);
    let d = r.w.card(&t.id).unwrap();
    let check = d
        .blockers
        .iter()
        .find(|b| b.done_check)
        .expect("a done check");
    assert_eq!(check.question, coord::DONE_QUESTION);
    assert_eq!(check.context, "done");
    let check_id = check.id.clone();
    // Done closes it, as the person.
    let a = r.w.answer(&t.id, &check_id, 0, "", false, &[]).unwrap();
    assert_eq!(a.card.summary.state, coord::COMPLETED);
    // In one step (card #315): the check reads answered Done.
    let ct = co.get_task(&t.id).unwrap();
    let res = ct
        .blockers
        .iter()
        .find(|b| b.id == check_id)
        .and_then(|b| b.resolved.clone())
        .expect("resolved");
    assert_eq!(res.answer, coord::DONE_ANSWER);
    assert!(!res.closed);
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
        r.w.ask("Who can look at the tray?", "", false, &[], "", &[])
            .unwrap();
    assert!(res.task.ask.is_some());
    let cards = r.w.cards();
    // Handed to one idle agent (#91), or open when none is.
    let state = cards[0].ask.as_ref().unwrap().state.clone();
    assert!(state == "handed" || state == "unclaimed", "{state}");
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
            &[],
        )
        .unwrap_err();
    assert!(e.contains("files"), "{e}");
    let e =
        r.w.ask("Anyone?", "", false, &[], "no-such-board", &[])
            .unwrap_err();
    assert!(e.contains("no such board"), "{e}");
    let res =
        r.w.ask("See the picture", "", false, &[f], "", &[])
            .unwrap();
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
    let m = r.w.inbox_send(&r.key_b, "", "hello beta", &[]).unwrap();
    assert_eq!(m.message.text, "hello beta");
    assert_eq!(
        r.w.inbox_send("", "", "x", &[]).unwrap_err(),
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
    r.w.inbox_reply(&d.id, "an answer", &[]).unwrap();
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
    let keys: BTreeSet<String> = r.w.agents().into_iter().map(|a| a.card.key).collect();
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
    // A rename (card #184) is what the Devices list calls it.
    let me = r.w.core.device_id().to_string();
    assert_eq!(r.w.rename_device(&me, "MacBook").unwrap(), "MacBook");
    assert_eq!(r.w.devices()[0].name, "MacBook");
    assert!(r.w.rename_device("not-mine", "PC").is_err());
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
    let _ = r.w.comment(&t.id, "a comment", "none", &[], &[]).unwrap();
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
    // A wait on another card carries what it waits on (card #481).
    let other = co
        .create_task(
            &a,
            coord::NewTask {
                title: "Other".into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.raise(
        &a,
        &t.id,
        coord::NewBlocker {
            question: "after it".into(),
            blocked_by: Some(coord::BlockedBy {
                kind: coord::BLOCKED_ON_TASK.into(),
                reference: other.id.clone(),
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let wj = j(&r.w.card(&t.id).unwrap());
    let wait = wj["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b.get("waitsOn").is_some())
        .expect("a wait item");
    check_shape(&src, "CardBlocker", wait);
    check_shape(&src, "CardWaitOn", &wait["waitsOn"]);
    let mut summary = cj.clone();
    for k in [
        "goal",
        "doneWhen",
        "note",
        "steps",
        "blockers",
        "reviews",
        "prs",
        "links",
        "recent",
        "comments",
        "thread",
        "threadEarlier",
        "historyTrimmed",
        "subtaskList",
        "waits",
    ] {
        summary.as_object_mut().unwrap().remove(k);
    }
    check_shape(&src, "CardSummary", &summary);

    let m = r.w.inbox_send(&r.key_b, "", "hi", &[]).unwrap();
    check_shape(&src, "SendResult", &j(&m));
    check_shape(&src, "Message", &j(&m.message));
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

#[test]
fn not_done_goes_back_to_the_agent() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "Ship it".into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &t.id).unwrap();
    co.set_task_state(&r.key_a, &t.id, coord::COMPLETED, "")
        .unwrap();
    let d = r.w.card(&t.id).unwrap();
    let check = d.blockers.iter().find(|b| b.done_check).unwrap().id.clone();
    // The person says why it isn't done: it stays open and the agent hears.
    let a =
        r.w.answer(&t.id, &check, -1, "the tests still fail", false, &[])
            .unwrap();
    assert_eq!(a.told.to, vec![r.key_a.clone()]);
    assert_ne!(a.card.summary.state, coord::COMPLETED);
    let cards = r.w.cards();
    assert_eq!(cards.len(), 1);
    assert!(!cards[0].ready, "answered: it is back with the agent");
    // Card #476: the agent answers on the check, then asks again with new
    // proof. The new check opens with that round: his reason, its answer,
    // the proof he sent back.
    co.comment_on(
        &r.key_a,
        &t.id,
        coord::CommentInput {
            text: "fixed the flaky login test".into(),
            blocker_id: check.clone(),
            ..Default::default()
        },
    )
    .unwrap();
    co.set_task_state(&r.key_a, &t.id, coord::COMPLETED, "tests green on CI")
        .unwrap();
    let d = r.w.card(&t.id).unwrap();
    let now = d
        .blockers
        .iter()
        .find(|b| b.done_check && b.resolved.is_none())
        .unwrap();
    let p = now.previous.as_ref().expect("the round he sent back");
    assert_eq!(p.id, check);
    assert_eq!(p.round, 2);
    assert!(!p.same_proof);
    let said: Vec<(&str, &str)> = p
        .thread
        .iter()
        .filter(|it| it.kind == "comment")
        .map(|it| (it.who.status.as_str(), it.text.as_str()))
        .collect();
    assert_eq!(said.len(), 2, "{said:?}");
    assert_eq!(said[0], ("person", "the tests still fail"));
    assert_eq!(said[1].1, "fixed the flaky login test");
    // The old check keeps its thread too, folded under Blockers.
    let old = d.blockers.iter().find(|b| b.id == check).unwrap();
    assert!(old.resolved.is_some());
    assert!(old.previous.is_none());
}

/// Talk (card #389) through the window: a channel, a post, a DM with an
/// agent (opened with talk_dm), a thread's reply through the comment box's
/// call, reads, and the TS shapes of what talk_conversations and
/// talk_posts send.
#[test]
fn talk_through_the_window() {
    let src = std::fs::read_to_string(manifest_dir().join("../../frontend/src/gen/bindings.ts"))
        .unwrap()
        .replace("\r\n", "\n");
    let r = Rig::new();
    let w = &r.w;
    let ch = w
        .talk_channel_create("general", Some("the room"), None)
        .unwrap();
    let p = w.talk_post(&ch, "hello room", true, None).unwrap();
    assert!(w.talk_post(&ch, "  ", true, None).is_err());
    let from = w
        .talk_post(&ch, "about that", false, Some(p.clone()))
        .unwrap();
    let posts = w.talk_posts(&ch, None, 0).unwrap();
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[1].post_id, from);
    assert_eq!(posts[1].thread_from.as_deref(), Some(p.as_str()));
    assert!(!posts[1].agents_now);
    assert_eq!(w.talk_posts(&ch, Some(&from), 10).unwrap().len(), 1);

    let agent = irori_window::talk::TalkMemberRef {
        kind: "agent".into(),
        key: r.key_a.clone(),
    };
    let dm = w.talk_dm(&agent).unwrap();
    assert_eq!(w.talk_dm(&agent).unwrap(), dm, "one DM per pair");
    let id = w.talk_post(&dm, "got a minute?", true, None).unwrap();
    let ds = w.core.inbox().for_agent(&r.key_a, false, 10);
    assert!(ds.iter().any(|d| d.message_id == id), "{ds:?}");

    // A thread's reply is a comment on it.
    let (t, _) = w
        .core
        .coord()
        .post_ask(
            &w.core.user_sender().key,
            coord::NewAsk {
                text: "why is it slow?".into(),
                thread: true,
                ..Default::default()
            },
            &[],
        )
        .unwrap();
    let cid = w.talk_post(&t.id, "still slow", false, None).unwrap();
    let tp = w.talk_posts(&t.id, None, 50).unwrap();
    assert_eq!(tp.len(), 2);
    assert_eq!(tp[1].post_id, cid);
    assert_eq!(w.core.coord().comments(&t.id).unwrap().len(), 1);

    let cs = w.talk_conversations().unwrap();
    for (id, kind) in [(&ch, "channel"), (&dm, "dm"), (&t.id, "thread")] {
        let c = cs.iter().find(|c| &c.conv_id == id).expect(kind);
        assert_eq!(c.kind, kind);
    }
    let c = cs.iter().find(|c| c.conv_id == ch).unwrap();
    assert_eq!(c.topic.as_deref(), Some("the room"));
    assert_eq!(c.unread, 0, "the person's own posts");
    w.talk_read(&ch, &from).unwrap();
    w.talk_leave(&ch, &agent).unwrap();
    w.talk_join(&ch, &agent).unwrap();
    assert!(w.core.talk().is_member(&ch, &r.key_a));

    let j = |v: &dyn erased::Ser| v.json();
    check_shape(&src, "TalkConversation", &j(c));
    check_shape(&src, "TalkMember", &j(&c.members[0]));
    check_shape(&src, "TalkPost", &j(&posts[1]));
}

/// Card #417: Settings › Merge desks sets a repo's desk once, and the desk
/// page's holds, pause and Outside PR OK go in as the person.
#[test]
fn merge_desk_settings_holds_pause_and_ok() {
    use irori_window::desk::{DeskCheckLine, DeskSettings};
    let r = Rig::new();
    let repo = "remote:github.com/example-org/example-app";
    assert!(r.w.desks().is_empty());
    // No desk yet: nothing to hold.
    assert!(r.w.desk_hold(repo, 3, "", false).is_err());
    let here = r.w.core.device_id().to_string();
    r.w.desk_set(DeskSettings {
        repo_key: repo.into(),
        enabled: true,
        takes_prs: true,
        holder: r.key_a.clone(),
        poll_device: here.clone(),
        trusted: vec![" you ".into(), "".into()],
        checks: vec![DeskCheckLine {
            command: " cargo test --workspace ".into(),
            paths: vec!["".into()],
            ..DeskCheckLine::default()
        }],
        ..DeskSettings::default()
    })
    .unwrap();
    r.w.desk_hold(repo, 3, "after the playtest", false).unwrap();
    r.w.desk_pause(repo, true).unwrap();
    r.w.desk_ok(repo, 4, "3f2a9c1e0b7d").unwrap();
    let d = r.w.desks();
    assert_eq!(d.len(), 1);
    let d = &d[0];
    assert_eq!(d.settings.holder, r.key_a);
    assert_eq!(d.settings.poll_device, here);
    assert_eq!(d.settings.trusted, vec!["you".to_string()]);
    assert_eq!(d.settings.checks[0].command, "cargo test --workspace");
    assert!(d.settings.checks[0].paths.is_empty());
    assert_eq!(d.settings.merge_method, "squash");
    assert!(d.settings.by.starts_with("user:"));
    assert_eq!(
        (d.holds[0].pr, d.holds[0].note.as_str()),
        (3, "after the playtest")
    );
    assert!(d.paused.is_some());
    assert_eq!(d.oks[0].head_sha, "3f2a9c1e0b7d");
    assert!(d.poll.here, "this device polls it");
    assert!(d.prs.is_none(), "no poll in a rig without gh");

    r.w.desk_hold(repo, 3, "", true).unwrap();
    r.w.desk_pause(repo, false).unwrap();
    let d = &r.w.desks()[0];
    assert!(d.holds.is_empty());
    assert!(d.paused.is_none());
}

/// Card #452: "Start an agent with this" on the person's comment puts a
/// card nobody is on back on the Board; a card with an agent on it is
/// refused (spawned-agents §4).
#[test]
fn start_agent_with_a_comment() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let t = co
        .create_task(
            &r.w.core.user_sender().key,
            coord::NewTask {
                title: "Save slots".into(),
                ..coord::NewTask::default()
            },
        )
        .unwrap();
    co.set_task_state(&r.w.core.user_sender().key, &t.id, "input_required", "")
        .ok();
    r.w.start_agent(&t.id, "keep the last three").unwrap();
    assert_eq!(co.get_task(&t.id).unwrap().state, coord::SUBMITTED);

    co.claim_task(&r.key_a, &t.id).unwrap();
    let e = r.w.start_agent(&t.id, "again").unwrap_err();
    assert!(e.contains("has an agent on it"), "{e}");
    assert!(r.w.start_agent("no-such-card", "").is_err());
}

/// Card #476: a Not done after the agent has gone still goes on the done
/// check's thread (it used to resolve the check with no comment), so the
/// next agent finds his reason there and answers it.
#[test]
fn a_late_not_done_stays_on_the_checks_thread() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let ghost = format!("{}/gone-session", r.w.core.device_id());
    let t = co
        .create_task(
            &ghost,
            coord::NewTask {
                title: "Ship it".into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.claim_task(&ghost, &t.id).unwrap();
    co.set_task_state(&ghost, &t.id, coord::COMPLETED, "merged")
        .unwrap();
    let check = co
        .get_task(&t.id)
        .unwrap()
        .blockers
        .iter()
        .find(|b| b.done_check)
        .unwrap()
        .id
        .clone();
    r.w.answer(&t.id, &check, -1, "the docs still say v1", false, &[])
        .unwrap();
    let d = r.w.card(&t.id).unwrap();
    let b = d.blockers.iter().find(|b| b.id == check).unwrap();
    assert!(
        b.thread
            .iter()
            .any(|it| it.who.status == "person" && it.text == "the docs still say v1"),
        "{:?}",
        b.thread
    );
    let sb = co.sent_back(&t.id).expect("sent back");
    assert_eq!(sb.id, check);
    assert!(!sb.answered);
}
