//! The window's services on a scratch Core (tests/common), and the TS
//! shapes in frontend/src/gen/bindings.ts checked against the real
//! JSON of the values the commands send.

use std::collections::BTreeSet;

use irori_window::agents::NamesThemesView;
use irori_window::attachments::{self, Request};
use irori_window::settings::NotifyOptions;

use crate::common;

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
    assert!(!a.resolved, "answered, not resolved: it waits on the agent");
    assert_eq!(r.w.card(&t.id).unwrap().blockers[0].state, "answered");
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
    assert!(a.resolved);
    assert_eq!(r.w.card(&t.id).unwrap().summary.state, coord::COMPLETED);
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

/// Card #483: a returned row kept before the fix carries the agent's key
/// in its body; the Notifications page says it by name, or "Its agent"
/// when only the key is known, and never shows the key.
#[test]
fn stored_returned_rows_never_show_a_key() {
    use coord::notifications::{self as history, PostedRow};
    let r = Rig::new();
    let co = r.w.core.coord();
    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "Spawned agents part 9".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let gone = "devzz/ffffffff-9999-4999-8999-999999999999";
    let named = "devzz/eeeeeeee-8888-4888-8888-888888888888";
    let tail = " went away, so it's open again. Assign it to an agent, or close it.";
    // (from, who the stored body says, what the page must say)
    let rows = [
        (r.key_a.clone(), r.key_a.clone(), "alpha"),
        (gone.to_string(), gone.to_string(), "Its agent"),
        // Nothing knows it any more, but the toast named it: keep that.
        (named.to_string(), "Bing Bong".to_string(), "Bing Bong"),
    ];
    for (i, (from, who, _)) in rows.iter().enumerate() {
        let mut row = PostedRow {
            kind: history::KIND_RETURNED.into(),
            task_id: t.id.clone(),
            instance: format!("dev1:{i}"),
            from: from.clone(),
            title: "Nobody is on “Spawned agents part 9” (#1)".into(),
            body: format!("{who}{tail}"),
            level: "warn".into(),
            at: gocompat::time::GoTime::from_system(std::time::SystemTime::now()),
            ..PostedRow::default()
        };
        row.key = history::key_of(&row);
        r.w.core
            .store()
            .append("irori", history::POSTED, &row)
            .unwrap();
    }
    wait("the rows are in the history", || {
        r.w.notifications().len() == rows.len()
    });
    let ns = r.w.notifications();
    for n in &ns {
        assert!(!n.body.contains('/'), "{n:?}");
        if let Some(f) = &n.from {
            assert!(!f.name.contains('/'), "{n:?}");
        }
    }
    for (i, (from, _, want)) in rows.iter().enumerate() {
        let n = ns
            .iter()
            .find(|n| n.key.ends_with(&format!("dev1:{i}")))
            .unwrap_or_else(|| panic!("no row for {from}: {ns:?}"));
        assert_eq!(n.body, format!("{want}{tail}"), "{n:?}");
        if *want == "Its agent" {
            assert!(n.from.is_none(), "an unnamed agent's key isn't sent: {n:?}");
        }
    }
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
    assert!(e.contains("no such space"), "{e}");
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

// Card #527: the window's appearance reaches the notifier, which keeps it
// in settings.json for the next start. The JSON is what card-look.ts sends.
#[test]
fn card_look_reaches_the_notifier() {
    let r = Rig::new();
    let look: irori_window::settings::CardLook = serde_json::from_str(
        r##"{"theme":"sakura","mode":"system","glass":"frost","alpha":0.62,"radius":14,
            "light":{"background":"#fbf7f8","tint":"#fdfbfc","onPrimary":"#fdfafb"},
            "dark":{"background":"#1d1519","primary":"#f496bb"}}"##,
    )
    .unwrap();
    assert_eq!(look.light.on_primary, "#fdfafb", "camelCase from the page");
    r.w.set_card_look(look).unwrap();
    let saved = notify::load_look(&r.data).expect("saved");
    assert_eq!(
        (
            saved.theme.as_str(),
            saved.mode.as_str(),
            saved.alpha,
            saved.radius
        ),
        ("sakura", "system", 0.62, 14.0)
    );
    assert_eq!(saved.light.on_primary, "#fdfafb");
    assert_eq!(saved.dark.primary, "#f496bb");
    // The notifications next to it are untouched.
    assert_eq!(notify::load(&r.data), notify::Options::default());
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
    // A weight (card #963), so Task, Weight and CardWeight are checked.
    co.set_weight(&a, &t.id, "heavy", "planning the split")
        .unwrap();
    let task = co.get_task(&t.id).unwrap();

    let j = |v: &dyn erased::Ser| v.json();
    check_shape(&src, "Task", &j(&task));
    check_shape(&src, "Blocker", &j(&task.blockers[0]));
    check_shape(&src, "BlockerOption", &j(&task.blockers[0].options[0]));
    check_shape(&src, "Link", &j(&task.links[0]));
    check_shape(&src, "Step", &j(&task.steps[0]));
    check_shape(&src, "Comment", &j(&co.comments(&t.id).unwrap()[0]));
    check_shape(&src, "Weight", &j(task.weight.as_ref().unwrap()));
    let card = r.w.card(&t.id).unwrap();
    let cj = j(&card);
    check_shape(&src, "CardWeight", &cj["weight"]);
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
    assert!(!a.resolved);
    assert_ne!(r.w.card(&t.id).unwrap().summary.state, coord::COMPLETED);
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
    let p = w
        .talk_post(&ch, "hello room", &[], true, None, &[])
        .unwrap();
    assert!(w.talk_post(&ch, "  ", &[], true, None, &[]).is_err());
    let from = w
        .talk_post(&ch, "about that", &[], false, Some(p.clone()), &[])
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
    let id = w
        .talk_post(&dm, "got a minute?", &[], true, None, &[])
        .unwrap();
    let ds = w.core.inbox().for_agent(&r.key_a, false, 10);
    assert!(ds.iter().any(|d| d.message_id == id), "{ds:?}");
    // The DM's agent shows as replying (card #645), by name.
    let rs = w.talk_replying();
    let row = rs
        .iter()
        .find(|r| r.conv_id == dm)
        .expect("replying in the DM");
    assert_eq!(row.agents.len(), 1);
    assert_eq!(row.agents[0].key, r.key_a);
    assert!(!row.agents[0].name.is_empty() && row.agents[0].name != r.key_a);
    assert!(src.contains("talkReplying"), "bindings are regenerated");

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
    let cid = w
        .talk_post(&t.id, "still slow", &[], false, None, &[])
        .unwrap();
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

    // Card #643: a picture alone in a channel and in a DM. The posts carry
    // it, and the route and the attachment calls find it on the post.
    {
        use base64::Engine as _;
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
        let up = |name: &str| irori_window::board::UploadFile {
            name: name.into(),
            data: base64::engine::general_purpose::STANDARD.encode(png),
        };
        let id = w
            .talk_post(&ch, "", &[], true, None, &[up("shot.png")])
            .unwrap();
        let ps = w.talk_posts(&ch, None, 50).unwrap();
        let p = ps.iter().find(|p| p.post_id == id).unwrap();
        assert_eq!(p.text, "");
        assert_eq!(p.attachments.len(), 1);
        assert_eq!(p.attachments[0].name, "shot.png");
        check_shape(&src, "TalkPost", &j(p));
        let sha = p.attachments[0].sha256.clone();
        let info = w.attachment_info(&sha).unwrap();
        assert_eq!((info.name.as_str(), info.here), ("shot.png", true));
        let ok = w.serve_attachment(&Request {
            method: "GET".into(),
            path: format!("{}{sha}", attachments::ROUTE),
            ..Request::default()
        });
        assert_eq!(ok.status, 200);
        assert_eq!(ok.body, png);
        assert_eq!(ok.get("Content-Type"), Some("image/png"));

        // A DM with an agent: the message carries it as well.
        let id = w
            .talk_post(&dm, "this one", &[], true, None, &[up("dm.png")])
            .unwrap();
        let ps = w.talk_posts(&dm, None, 50).unwrap();
        let p = ps.iter().find(|p| p.post_id == id).unwrap();
        assert_eq!(p.attachments.len(), 1);
        let ds = w.core.inbox().for_agent(&r.key_a, false, 10);
        let d = ds.iter().find(|d| d.message_id == id).unwrap();
        assert_eq!(d.message.attachments.len(), 1);
        assert!(w.attachment_info(&p.attachments[0].sha256).is_ok());

        // Nothing at all is still refused, and a file nobody posted isn't
        // served.
        assert!(w.talk_post(&ch, " ", &[], true, None, &[]).is_err());
        let none = w.serve_attachment(&Request {
            method: "GET".into(),
            path: format!("{}{}", attachments::ROUTE, "e".repeat(64)),
            ..Request::default()
        });
        assert_eq!(none.status, 404);
        assert!(w.attachment_info(&"e".repeat(64)).is_err());
    }

    // Card #538: nobody to DM yet, a person you share no space with is
    // refused, and an agent's DM is no person DM to leave. A row that isn't
    // waiting leaves `waiting` out, and one that is carries it.
    assert!(w.talk_dm_people().is_empty());
    let bob = irori_window::talk::TalkMemberRef {
        kind: "person".into(),
        key: "person:pBob".into(),
    };
    let e = w.talk_dm(&bob).unwrap_err();
    assert!(e.contains("don't share a space"), "{e}");
    assert!(w.talk_dm_leave(&dm).is_err());
    assert!(j(c).get("waiting").is_none());
    let mut waits = c.clone();
    waits.waiting = "them".into();
    assert_eq!(j(&waits)["waiting"], "them");
    check_shape(&src, "TalkConversation", &j(&waits));
    check_shape(
        &src,
        "TalkPerson",
        &j(&irori_window::talk::TalkPerson::default()),
    );

    // A channel in a shared space carries its spaceId (card #461); a
    // Personal one leaves it out.
    let space = w.core.create_board("one", "one").unwrap();
    let sc = w
        .talk_channel_create("general", None, Some(&space))
        .unwrap();
    let cs = w.talk_conversations().unwrap();
    let s = cs.iter().find(|c| c.conv_id == sc).expect("space channel");
    assert_eq!(s.space_id, space);
    let sj = j(s);
    assert_eq!(sj["spaceId"], space.as_str());
    check_shape(&src, "TalkConversation", &sj);
    let pc = cs.iter().find(|c| c.conv_id == ch).unwrap();
    assert!(j(pc).get("spaceId").is_none(), "Personal: no spaceId");
}

/// Card #417: Settings › Merge desks sets a repo's desk once, as the
/// person. The desk never merges (card #418): no holder, check lines,
/// merge method, holds or pause.
#[test]
fn merge_desk_settings() {
    use irori_window::desk::DeskSettings;
    let r = Rig::new();
    let repo = "remote:github.com/example-org/example-app";
    assert!(r.w.desks().is_empty());
    let here = r.w.core.device_id().to_string();
    r.w.desk_set(DeskSettings {
        repo_key: repo.into(),
        enabled: true,
        takes_prs: true,
        poll_device: here.clone(),
        trusted: vec![" you ".into(), "".into()],
        ..DeskSettings::default()
    })
    .unwrap();
    let d = r.w.core.coord().desk(repo).expect("the desk");
    assert!(d.settings.by.starts_with("user:"));
    // The desk page lists it once the Core counts the repo as the
    // person's (card #625).
    r.w.core.coord().set_desk_repos([repo.to_string()]);
    let d = r.w.desks();
    assert_eq!(d.len(), 1);
    let d = &d[0];
    assert_eq!(d.settings.poll_device, here);
    assert_eq!(d.settings.trusted, vec!["you".to_string()]);
    assert!(d.poll.here, "this device polls it");
    assert!(d.prs.is_none(), "no poll in a rig without gh");
}

/// Card #625: the desk page lists only the person's repos' desks, each
/// with only their PRs: a stranger's PR isn't shown, and a stray desk (a
/// repo nobody in their spaces opens PRs in) isn't listed.
#[test]
fn the_desk_page_shows_only_our_repos_and_prs() {
    use irori_window::desk::DeskSettings;
    let r = Rig::new();
    let repo = "remote:github.com/example-org/example-app";
    let upstream = "remote:github.com/blaizzy/mlx-video";
    let co = r.w.core.coord();
    // The person's agent linked a PR in example-app on a card: their repo,
    // on every desk tick. Nobody here opened one in mlx-video.
    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "the PR".into(),
                ..coord::NewTask::default()
            },
        )
        .unwrap();
    co.add_link(
        &r.key_a,
        &t.id,
        "https://github.com/example-org/example-app/pull/1",
    )
    .unwrap();
    for k in [repo, upstream] {
        r.w.desk_set(DeskSettings {
            repo_key: k.into(),
            enabled: true,
            takes_prs: true,
            poll_device: r.w.core.device_id().to_string(),
            ..DeskSettings::default()
        })
        .unwrap();
    }
    // The Core's desk pass judged both: only example-app is ours.
    co.set_desk_repos([repo.to_string()]);
    let pr = |n: i64, author: &str| coord::DeskPr {
        number: n,
        url: format!("https://github.com/example-org/example-app/pull/{n}"),
        title: format!("PR {n}"),
        author: author.into(),
        ..coord::DeskPr::default()
    };
    // "you" is the person's login, as their gh said it.
    co.record_desk_snapshot(repo, vec![pr(1, "you"), pr(2, "a-stranger")], false, "you")
        .unwrap();
    // mlx-video's desk is stray: not listed at all.
    assert!(co.desk(upstream).unwrap().stray);
    let d = r.w.desks();
    assert_eq!(
        d.iter()
            .map(|d| d.settings.repo_key.as_str())
            .collect::<Vec<_>>(),
        [repo]
    );
    let shown: Vec<i64> = d[0]
        .prs
        .as_ref()
        .unwrap()
        .prs
        .iter()
        .map(|p| p.number)
        .collect();
    assert_eq!(shown, [1]);
    assert_eq!(d[0].authors.iter().map(|a| a.pr).collect::<Vec<_>>(), [1]);
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

/// Card #454: "Start a thread" on a channel message. The thread records
/// the channel, and in a channel named after a repo with a checkout here
/// it is that repo's thread (so the agent Irori starts for it starts
/// there); the channel gets the one-line pointer. Two repos of that name,
/// or none, name no repo.
#[test]
fn a_thread_from_a_repo_channel_names_its_channel_and_repo() {
    let r = Rig::new();
    let w = &r.w;
    let d = tempfile::tempdir().unwrap();
    let seen = |key: &str, root: &std::path::Path| {
        w.core
            .capture()
            .emit_row(
                "irori",
                capture::KIND_REPO_SEEN,
                &capture::RepoSeen {
                    repo_key: key.into(),
                    root: root.to_string_lossy().into_owned(),
                    at: gocompat::time::GoTime::unix(1_790_000_000, 0),
                },
            )
            .unwrap();
    };
    let game = d.path().join("game-repo");
    std::fs::create_dir_all(&game).unwrap();
    seen("remote:github.com/t/game-repo", &game);
    let ch = w.talk_channel_create("Game-Repo", None, None).unwrap();
    let p = w
        .talk_post(&ch, "main is red after #212", &[], true, None, &[])
        .unwrap();
    assert_eq!(w.channel_repo(&ch), "remote:github.com/t/game-repo");
    let other = w.talk_channel_create("general", None, None).unwrap();
    assert_eq!(w.channel_repo(&other), "");
    assert_eq!(w.channel_repo(""), "");

    let res = w.talk_thread_from(&ch, &p, "Why is main red?").unwrap();
    let t = w.core.coord().get_task(&res.task.id).unwrap();
    let a = t.ask.clone().unwrap();
    assert!(a.thread, "{a:?}");
    assert_eq!(a.channel, ch);
    assert_eq!(a.repo_key, "remote:github.com/t/game-repo");
    assert_eq!(t.repo_key, "remote:github.com/t/game-repo");
    let posts = w.talk_posts(&ch, None, 0).unwrap();
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[1].text, format!("#{}", t.short));
    assert_eq!(posts[1].thread_from.as_deref(), Some(p.as_str()));
    assert!(w.talk_thread_from(&ch, "post_nope", "Why?").is_err());
    assert!(w.talk_thread_from(&t.id, &p, "Why?").is_err());

    // A same-named repo seen only on another device (no folder here)
    // doesn't count.
    seen(
        "remote:github.com/elsewhere/game-repo",
        &d.path().join("not-on-this-device"),
    );
    assert_eq!(w.channel_repo(&ch), "remote:github.com/t/game-repo");

    // A second repo by that name: no repo, the thread still starts.
    let fork = d.path().join("fork");
    std::fs::create_dir_all(&fork).unwrap();
    seen("remote:github.com/someone/game-repo", &fork);
    assert_eq!(w.channel_repo(&ch), "");
    let res = w.talk_thread_from(&ch, &p, "And now?").unwrap();
    let a = w.core.coord().get_task(&res.task.id).unwrap().ask.unwrap();
    assert_eq!((a.channel.as_str(), a.repo_key.as_str()), (ch.as_str(), ""));
}

/// Card #643: a file on a message an agent sent the person shows on the
/// DM mirrored from it, and the window resolves and serves it as
/// attachment_get would: from the message, not from any post's row.
#[test]
fn an_agents_message_to_the_person_serves_its_file() {
    let r = Rig::new();
    let w = &r.w;
    let me = w.core.user_sender().key;
    let dir = tempfile::tempdir().unwrap();
    let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
    let path = dir.path().join("graph.png");
    std::fs::write(&path, png).unwrap();
    let f = w
        .core
        .attach_files(&r.key_b, &[path.to_string_lossy().into_owned()])
        .unwrap()
        .remove(0);
    let sha = f.sha256.clone();
    // The person DMs beta; beta answers with the file.
    let dm = w
        .talk_dm(&irori_window::talk::TalkMemberRef {
            kind: "agent".into(),
            key: r.key_b.clone(),
        })
        .unwrap();
    assert_eq!(dm, coord::talk::dm_id(&r.key_b, &me));
    let asked = w
        .talk_post(&dm, "send me the graph", &[], true, None, &[])
        .unwrap();
    let from = inbox::Sender::new(&r.key_b, "beta", "agent");
    let to = inbox::Address::new(inbox::addr::REPLY, &asked);
    let (m, _) = w
        .core
        .inbox()
        .send_with(
            &from,
            &to,
            "the graph",
            inbox::SendOptions {
                attachments: vec![inbox::FileRef {
                    sha256: f.sha256,
                    name: f.name,
                    size: f.size,
                    mime: f.mime,
                    device: f.device,
                }],
                ..inbox::SendOptions::default()
            },
        )
        .unwrap();
    assert!(
        w.core.talk().attachments(&sha).is_empty(),
        "on the message alone, no post row"
    );

    let ps = w.talk_posts(&dm, None, 50).unwrap();
    let p = ps.iter().find(|p| p.post_id == m.id).expect("mirrored");
    assert_eq!(p.attachments.len(), 1);
    assert_eq!(p.attachments[0].sha256, sha);
    let info = w.attachment_info(&sha).unwrap();
    assert_eq!((info.name.as_str(), info.here), ("graph.png", true));
    let ok = w.serve_attachment(&Request {
        method: "GET".into(),
        path: format!("{}{sha}", attachments::ROUTE),
        ..Request::default()
    });
    assert_eq!(ok.status, 200);
    assert_eq!(ok.body, png);
}

/// Huddles (#654) through the window: nothing on at first, and a snapshot
/// named for the person: their own devices as "you" with the device's name,
/// someone nothing names as "another person" (never a key), a channel by
/// its name; the TS shape of what call:state sends.
#[test]
fn huddles_through_the_window() {
    let src = std::fs::read_to_string(manifest_dir().join("../../frontend/src/gen/bindings.ts"))
        .unwrap()
        .replace("\r\n", "\n");
    let r = Rig::new();
    let w = &r.w;
    assert_eq!(
        w.call_state(),
        irori_window::call::CallView {
            transcribe_on: true,
            ..Default::default()
        }
    );
    assert!(w.call_start("  ").is_err());

    let ch = w.talk_channel_create("standup", None, None).unwrap();
    let me = w.core.chain().person_id();
    let here = w.core.device_id().to_string();
    let label = w
        .core
        .device_labels()
        .get(&here)
        .cloned()
        .unwrap_or_default();
    let stranger = "p0123456789abcdef".to_string();
    let peer = |device: &str, person: &str| irori_core::CallPeer {
        device_id: device.into(),
        person_id: person.into(),
        phase: "live".into(),
        path: "direct".into(),
        rtt_ms: 26.0,
        transcribing: Some(true),
        ..Default::default()
    };
    let caption = |device: &str, person: &str, mine: bool, text: &str| irori_core::Caption {
        device_id: device.into(),
        person_id: person.into(),
        mine,
        seq: 0,
        start_ms: 1_000,
        end_ms: 2_400,
        text: text.into(),
    };
    // Transcribing needs the speech model here (and an engine): without
    // it the device can't, whatever the call says.
    let only_mine = |can_transcribe: bool| irori_core::CallSnapshot {
        mine: Some(irori_core::MyCall {
            call_id: "call-0".into(),
            conv_id: ch.clone(),
            can_transcribe,
            ..Default::default()
        }),
        ..Default::default()
    };
    let v = w.call_view(only_mine(true));
    assert!(!v.can_transcribe && !v.mine.as_ref().unwrap().can_transcribe);
    let model = voice::model_path(w.core.data_dir(), &voice::MODEL);
    std::fs::create_dir_all(model.parent().unwrap()).unwrap();
    std::fs::File::create(&model)
        .unwrap()
        .set_len(voice::MODEL.size)
        .unwrap();
    let engine = !matches!(voice::status(), voice::Status::Unsupported(_));
    let v = w.call_view(only_mine(false));
    assert_eq!(v.can_transcribe, engine);
    assert!(!v.mine.as_ref().unwrap().can_transcribe, "no transcriber");
    assert_eq!(w.call_state().can_transcribe, engine);

    let v = w.call_view(irori_core::CallSnapshot {
        mine: Some(irori_core::MyCall {
            call_id: "call-1".into(),
            conv_id: ch.clone(),
            joined_at_ms: 5,
            peers: vec![peer(&here, &me), peer("dev-x", &stranger)],
            error: Some("  ".into()),
            transcribing: true,
            can_transcribe: true,
            captions: vec![
                caption(&here, &me, true, "I'll take the PC build"),
                caption("dev-y", &me, false, "from my phone"),
                caption("dev-x", &stranger, false, "thanks"),
                caption("dev-z", "pfedcba9876543210", false, "gone now"),
            ],
            ..Default::default()
        }),
        ringing: vec![irori_core::Ringing {
            call_id: "call-2".into(),
            conv_id: ch.clone(),
            by_person: stranger.clone(),
            ..Default::default()
        }],
        live: vec![irori_core::LiveCall {
            call_id: "call-1".into(),
            conv_id: ch.clone(),
            devices: vec![
                (here.clone(), me.clone()),
                ("dev-y".into(), me.clone()),
                ("dev-x".into(), stranger.clone()),
            ],
            ..Default::default()
        }],
    });
    let m = v.mine.as_ref().unwrap();
    assert_eq!(
        (m.conv_kind.as_str(), m.conv_name.as_str()),
        ("channel", "standup")
    );
    assert_eq!(m.device, label);
    assert_eq!(m.peers[0].name, "you");
    assert_eq!(m.peers[0].device, label);
    assert_eq!(m.peers[1].name, "another person");
    assert_eq!(
        m.peers[1].device, "",
        "another person's device has no name here"
    );
    assert_eq!(m.error, None, "a blank error is none");
    assert!(m.transcribing && m.transcribe_on && v.transcribe_on);
    assert_eq!((m.can_transcribe, v.can_transcribe), (engine, engine));
    assert_eq!(m.peers[1].transcribing, Some(true));
    let said: Vec<(&str, bool, &str)> = m
        .captions
        .iter()
        .map(|c| (c.name.as_str(), c.mine, c.text.as_str()))
        .collect();
    assert_eq!(
        said,
        [
            ("you", true, "I'll take the PC build"),
            ("you", true, "from my phone"),
            ("another person", false, "thanks"),
            ("another person", false, "gone now"),
        ]
    );
    assert_eq!((m.captions[0].at_ms, m.captions[0].end_ms), (1_000, 2_400));
    assert_eq!(v.ringing[0].by, "another person");
    assert_eq!(v.ringing[0].conv_name, "standup");
    assert_eq!(v.live[0].people, ["you", "another person"]);
    let json = serde_json::to_string(&v).unwrap();
    assert!(!json.contains(&stranger) && !json.contains(&me), "{json}");
    check_shape(&src, "CallView", &serde_json::to_value(&v).unwrap());
    check_shape(&src, "CallMineView", &serde_json::to_value(m).unwrap());
    check_shape(
        &src,
        "CallPeerView",
        &serde_json::to_value(&m.peers[0]).unwrap(),
    );
    check_shape(
        &src,
        "CallCaptionView",
        &serde_json::to_value(&m.captions[0]).unwrap(),
    );
    assert!(!json.contains("dev-x") && !json.contains("pfedcba9876543210"));

    // The switch round-trips, and the picture carries it.
    assert!(w.call_transcribe());
    w.call_transcribe_set(false).unwrap();
    assert!(!w.call_transcribe());
    assert!(!w.call_state().transcribe_on);
    w.call_transcribe_set(true).unwrap();
    assert!(w.call_transcribe());
}

/// A huddle's transcript (#656) through the window: every line named, the
/// person's as "you", with the huddle's channel, start and length; its end
/// post says it has one; an unknown huddle is refused.
#[test]
fn a_huddles_transcript_through_the_window() {
    let src = std::fs::read_to_string(manifest_dir().join("../../frontend/src/gen/bindings.ts"))
        .unwrap()
        .replace("\r\n", "\n");
    let r = Rig::new();
    let w = &r.w;
    let ch = w.talk_channel_create("standup", None, None).unwrap();
    let me = w.core.user_sender().key;
    let start = coord::talk::CallMark {
        call_id: "call_t1".into(),
        op: coord::talk::CALL_START.into(),
        ..Default::default()
    };
    let started = w.core.post_call_mark(&ch, "huddle started", start).unwrap();
    let t0 = started.at.unix_milli_value();
    let talk = w.core.talk();
    talk.write_segment(&ch, "call_t1", 1, t0 + 5_000, t0 + 6_000, "second", &me)
        .unwrap();
    talk.write_segment(&ch, "call_t1", 0, t0 + 1_000, t0 + 2_000, "first", &me)
        .unwrap();
    let t = w.talk_call_transcript("call_t1").unwrap();
    assert_eq!(
        (
            t.conv_id.as_str(),
            t.conv_name.as_str(),
            t.conv_kind.as_str()
        ),
        (ch.as_str(), "standup", "channel")
    );
    assert_eq!(t.started_at_ms, Some(t0));
    assert!(!t.ended);
    let lines: Vec<(&str, bool, i64, &str)> = t
        .lines
        .iter()
        .map(|l| (l.name.as_str(), l.mine, l.at_ms, l.text.as_str()))
        .collect();
    assert_eq!(
        lines,
        [
            ("you", true, t0 + 1_000, "first"),
            ("you", true, t0 + 5_000, "second"),
        ]
    );

    let end = coord::talk::CallMark {
        call_id: "call_t1".into(),
        op: coord::talk::CALL_END.into(),
        duration_ms: 60_000,
        people: vec![me.clone()],
        transcript: true,
    };
    w.core.post_call_mark(&ch, "huddle ended", end).unwrap();
    let t = w.talk_call_transcript(" call_t1 ").unwrap();
    assert!(t.ended);
    assert_eq!(t.duration_ms, 60_000);
    let json = serde_json::to_string(&t).unwrap();
    assert!(!json.contains(&me), "{json}");
    check_shape(&src, "TalkTranscript", &serde_json::to_value(&t).unwrap());
    check_shape(
        &src,
        "TalkTranscriptLine",
        &serde_json::to_value(&t.lines[0]).unwrap(),
    );

    let posts = w.talk_posts(&ch, None, 50).unwrap();
    let marks: Vec<(String, bool)> = posts
        .iter()
        .filter_map(|p| p.call.as_ref())
        .map(|c| (c.op.clone(), c.transcript))
        .collect();
    assert_eq!(
        marks,
        [("start".to_string(), false), ("end".to_string(), true)]
    );

    assert!(w.talk_call_transcript("call_nope").is_err());
    assert!(w.talk_call_transcript("  ").is_err());
}

/// Card #702: Settings › Agents' theme switches through the window: every
/// theme on at first, each with its count, a switch round-trips, an unknown
/// theme is refused, and the view is bindings.ts's.
#[test]
fn the_names_themes_through_the_window() {
    let src = std::fs::read_to_string(manifest_dir().join("../../frontend/src/gen/bindings.ts"))
        .unwrap()
        .replace("\r\n", "\n");
    let r = Rig::new();
    let w = &r.w;
    let v = w.names_themes();
    let ids: Vec<&str> = v.themes.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        ["nature", "produce", "movies", "anime", "presidents", "myth"]
    );
    assert!(v.themes.iter().all(|t| t.on), "all on (decision 52)");
    assert_eq!(
        v.themes.iter().map(|t| t.words as usize).sum::<usize>(),
        coord::names::WORDS.len()
    );
    assert!(v.themes.iter().all(|t| !t.sample.is_empty()));
    check_shape(&src, "NamesThemesView", &serde_json::to_value(&v).unwrap());
    check_shape(
        &src,
        "NamesThemeChoice",
        &serde_json::to_value(&v.themes[0]).unwrap(),
    );

    let on = |v: &NamesThemesView, id: &str| v.themes.iter().find(|t| t.id == id).unwrap().on;
    let v = w.names_theme_set(" movies ", false).unwrap();
    assert!(!on(&v, "movies") && on(&v, "anime"));
    assert!(!on(&w.names_themes(), "movies"));
    assert!(w.names_theme_set("westerns", false).is_err());
    assert!(on(&w.names_theme_set("movies", true).unwrap(), "movies"));
}

/// Card #779: the person renames a card and a thread from its page. The
/// board and the Threads list show the new names, an agent's rename after
/// it is refused, and both stay after a restart.
#[test]
fn rename_by_hand() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "lobby join".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let (th, _) = co
        .post_ask(
            &r.w.core.user_sender().key,
            coord::NewAsk {
                text: "can I rename threads?".into(),
                thread: true,
                ..Default::default()
            },
            &[],
        )
        .unwrap();
    let got = r.w.rename_card(&t.id, " Join the lobby ").unwrap();
    assert_eq!(got.title, "Join the lobby");
    assert!(got.title_by_person);
    r.w.rename_card(&th.id, "Renaming threads").unwrap();
    assert!(
        r.w.rename_card(&t.id, "  ")
            .unwrap_err()
            .contains("needs a title")
    );

    let on_board = |w: &irori_window::Window| {
        w.cards()
            .iter()
            .find(|s| s.id == t.id)
            .map(|s| s.title.clone())
    };
    let in_threads = |w: &irori_window::Window| {
        w.threads("")
            .iter()
            .find(|s| s.id == th.id)
            .map(|s| s.title.clone())
    };
    assert_eq!(on_board(&r.w).as_deref(), Some("Join the lobby"));
    assert_eq!(in_threads(&r.w).as_deref(), Some("Renaming threads"));
    assert_eq!(r.w.card(&t.id).unwrap().summary.title, "Join the lobby");

    // The agent that holds it can't name it back.
    let e = co
        .edit_task(
            &r.key_a,
            &t.id,
            coord::TaskEdit {
                title: Some("lobby join".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(e.to_string().contains("the person named"), "{e}");

    // A restart over the same data dir keeps both names.
    r.w.core.close().unwrap();
    let data = r.data.clone();
    let again = Rig::with(|o| o.data_dir = data);
    assert_eq!(on_board(&again.w).as_deref(), Some("Join the lobby"));
    assert_eq!(in_threads(&again.w).as_deref(), Some("Renaming threads"));
    assert!(
        again
            .w
            .core
            .coord()
            .get_task(&t.id)
            .unwrap()
            .title_by_person
    );
}

/// Card #782: Irori names the person's thread. Its line on the thread
/// carries Undo, and Undo gives the old name back as the person's own:
/// the line loses its Undo, and Irori can't rename it again.
#[test]
fn undo_irori_naming_a_thread() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let (th, _) = co
        .post_ask(
            &r.w.core.user_sender().key,
            coord::NewAsk {
                text: "hey so the login page is broken again on safari".into(),
                thread: true,
                ..Default::default()
            },
            &[],
        )
        .unwrap();
    co.rename_as_irori(&th.id, "Login fails on Safari", &th.title)
        .unwrap();
    let d = r.w.card(&th.id).unwrap();
    assert_eq!(d.summary.title, "Login fails on Safari");
    let undo: Vec<_> = d
        .thread
        .iter()
        .filter(|it| !it.undo_title.is_empty())
        .collect();
    assert_eq!(undo.len(), 1, "one Undo, on Irori's line");
    assert_eq!(undo[0].who.status, "irori");
    assert!(
        undo[0].text.starts_with("Irori renamed it from"),
        "{}",
        undo[0].text
    );
    assert_eq!(undo[0].undo_title, th.title);

    let back = r.w.undo_title(&th.id).unwrap();
    assert_eq!(back.title, th.title);
    assert!(back.title_by_person);
    let d = r.w.card(&th.id).unwrap();
    assert!(d.thread.iter().all(|it| it.undo_title.is_empty()));
    assert!(r.w.undo_title(&th.id).is_err(), "nothing left to undo");
    assert!(
        co.rename_as_irori(&th.id, "Login fails on Safari", &th.title)
            .is_err()
    );
    // The switch reads and turns off.
    assert!(r.w.assistant_names());
    r.w.assistant_names_set(false).unwrap();
    assert!(!r.w.assistant_names());
}

/// Card #786: Irori's record is a thread line with Undo, keyed on the
/// record; the switch reads and turns off.
#[test]
fn irori_did_lines_on_the_thread_and_the_switch() {
    let r = Rig::new();
    assert!(r.w.assistant_tidy(), "on unless turned off");
    r.w.assistant_tidy_set(false).unwrap();
    assert!(!r.w.assistant_tidy());
    r.w.assistant_tidy_set(true).unwrap();

    let co = r.w.core.coord();
    let me = r.w.core.user_sender().key;
    let t = co
        .create_task(
            &me,
            coord::NewTask {
                title: "ship".into(),
                ..coord::NewTask::default()
            },
        )
        .unwrap();
    co.add_link(&me, &t.id, "https://github.com/a/b/pull/7")
        .unwrap();
    co.record_pr_state(
        &t.id,
        coord::PrCheck {
            pr: "https://github.com/a/b/pull/7".into(),
            state: coord::PR_MERGED.into(),
            ..coord::PrCheck::default()
        },
    )
    .unwrap();
    // The Core's listener may get there first; either way, one question.
    let _ = co.flag_merged(&t.id).unwrap();
    let d = r.w.card(&t.id).unwrap();
    let line = d
        .thread
        .iter()
        .find(|it| !it.undo_did.is_empty())
        .expect("a line with Undo");
    assert_eq!(line.who.status, "irori");
    assert!(line.text.contains("Irori asked if this is done"));
    let did = line.undo_did.clone();
    r.w.undo_did(&t.id, &did).unwrap();
    let d = r.w.card(&t.id).unwrap();
    assert!(d.thread.iter().all(|it| it.undo_did.is_empty()));
    assert!(d.thread.iter().any(|it| it.text.contains("(undone)")));
}

/// Card #786 job (b): the switch reads and turns off, and a link is a
/// line on each card with Undo.
#[test]
fn a_duplicate_link_is_a_thread_line_on_both_cards() {
    let r = Rig::new();
    assert!(r.w.assistant_dupes(), "on unless turned off");
    r.w.assistant_dupes_set(false).unwrap();
    assert!(!r.w.assistant_dupes());
    r.w.assistant_dupes_set(true).unwrap();

    let co = r.w.core.coord();
    let me = r.w.core.user_sender().key;
    let make = |t: &str| {
        co.create_task(
            &me,
            coord::NewTask {
                title: t.into(),
                ..coord::NewTask::default()
            },
        )
        .unwrap()
    };
    let (a, b) = (make("Login crash"), make("Safari login crash"));
    assert!(co.link_duplicate(&a.id, &b.id).unwrap());
    for (id, other) in [(&a.id, &b), (&b.id, &a)] {
        let d = r.w.card(id).unwrap();
        let line = d
            .thread
            .iter()
            .find(|it| !it.undo_did.is_empty())
            .expect("a line with Undo");
        assert_eq!(line.who.status, "irori");
        assert!(line.text.contains(&other.short_ref()), "{}", line.text);
    }
    let did =
        r.w.card(&a.id)
            .unwrap()
            .thread
            .iter()
            .find(|it| !it.undo_did.is_empty())
            .unwrap()
            .undo_did
            .clone();
    r.w.undo_did(&a.id, &did).unwrap();
    for id in [&a.id, &b.id] {
        let d = r.w.card(id).unwrap();
        assert!(d.thread.iter().all(|it| it.undo_did.is_empty()));
        assert!(d.thread.iter().any(|it| it.text.contains("(undone)")));
    }
}
