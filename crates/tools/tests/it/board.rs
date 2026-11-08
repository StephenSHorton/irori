//! The board tools (I:internal/tools board_test.go, blocker_rich_test.go,
//! room_test.go, owner_test.go): cards, comments and who hears them,
//! blockers with options, links and files, short refs and the clash
//! refusal, split, the ask tools and the owner rule. board_test.go's
//! TestWatchDigestsMergeUntilRead is in ported.rs.

use crate::common;

use std::sync::Arc;

use common::*;
use inbox::mode::{NEXT_TURN, WAKE};
use search::Source;

#[test]
fn test_board_create_and_assign() {
    let h = new_harness();
    let (a, b, c) = (call_a(), call_b(), call_c());
    h.failsj(&a, "board_create", json!({"objective": "x"}), &["title"]);
    h.failsj(
        &a,
        "board_create",
        json!({"title": "x", "assign_to": "nobody"}),
        &["agents tool"],
    );
    h.failsj(
        &a,
        "board_create",
        json!({"title": "x", "links": ["ftp://nope"]}),
        &["http(s)"],
    );
    h.failsj(
        &a,
        "board_create",
        json!({"title": "x", "priority": 1}),
        &[r#"unknown argument "priority""#],
    );

    let out = h.okj(
        &a,
        "board_create",
        json!({"title": "Fix\nauth", "objective": "tokens expire early",
            "links": ["https://linear.app/acme/issue/ABC-123/fix-auth"], "assign_to": "beta"}),
    );
    has(
        &out,
        &[
            "Created #1 (task:",
            "Assigned to beta",
            // beta is idle, so the hand-off wakes it (card #85).
            "Woke: beta",
            r#""Fix auth""#,
            "links: Linear ABC-123",
            "beta (dev1/bbbbbbbb-2222-4222-8222-222222222222) (offered, not taken up yet)",
        ],
    );
    let id = created_id(&out);
    let ns = h.notices(KEY_B);
    assert!(
        ns.len() == 1
            && ns[0].mode == inbox::mode::WAKE
            && ns[0].message.ref_ == format!("task:{id}")
            && ns[0].message.from.key == KEY_A
            && ns[0].message.reason == coord::NOTICE_ASSIGNED,
        "assign notice {ns:?}"
    );
    has(
        &ns[0].message.text,
        &["tokens expire early", &format!("(#1, task:{id})")],
    );

    // The card is in beta's Needs you until it takes it up, then Working.
    has(
        &h.ok(&b, "board", "{}"),
        &["Needs you (1)", &format!("task:{id}")],
    );
    has(
        &h.okj(&b, "board_join", json!({"task_id": format!("task:{id}")})),
        &["You hold #1 now", "[working]"],
    );
    has(&h.ok(&b, "board", "{}"), &["Working (1)"]);
    h.failsj(&c, "board_join", json!({"task_id": id}), &["held by"]);

    // board_assign hands it on. gamma is in another repo, but a card that
    // needs it shows on its own board wherever the card lives. gamma is
    // busy, so it hears at its next turn (card #85).
    h.b.roster.set_status(KEY_C, "busy");
    has(
        &h.okj(&b, "board_assign", json!({"task_id": id, "to": "GAMMA"})),
        &[
            &format!("Assigned {} to gamma", h.short(&id)),
            "Told at their next turn: gamma",
        ],
    );
    assert_eq!(
        h.notices(KEY_C).len(),
        1,
        "gamma notices {:?}",
        h.notices(KEY_C)
    );
    let out = h.ok(&c, "board", "{}");
    has(
        &out,
        &[
            "Board for remote:github.com/acme/other: 1 open card",
            "Needs you (1)",
            &format!("task:{id}"),
            "repo remote:github.com/acme/app",
        ],
    );
    has_not(&out, &["Nothing live here"]);
    has(
        &h.okj(&c, "board", json!({"repo": "*"})),
        &["Board for every repo", "Needs you (1)"],
    );
    // The other cards of acme/app stay off gamma's board.
    has_not(&h.ok(&c, "board", "{}"), &["Working ("]);

    // A Grok member's card has no repo; assigned to gamma, it still needs it.
    let out = h.okj(
        &grok_caller(),
        "board_create",
        json!({"title": "grok asks", "assign_to": "gamma"}),
    );
    let gid = format!("task:{}", created_id(&out));
    has(&h.ok(&c, "board", "{}"), &["Needs you (2)", &gid]);
    h.failsj(&a, "board_assign", json!({"task_id": id}), &["give to"]);
}

/// A comment reaches the holders at their next turn unless the author
/// chose to wake them (decision 18).
#[test]
fn test_board_comment_delivery() {
    for (name, wake, want_mode) in [
        ("default", None, NEXT_TURN),
        ("wake false", Some(false), NEXT_TURN),
        ("wake true", Some(true), WAKE),
    ] {
        let h = new_harness();
        let (a, b) = (call_a(), call_b());
        // beta is mid-turn, so the new card isn't offered to it (#82):
        // the wakes below are the comment's alone.
        h.b.roster.set_status(KEY_B, "working");
        let id = h.new_card(&a, json!({"title": "lexer"}));
        h.okj(&b, "board_join", json!({"task_id": id}));
        let mut args = json!({"task_id": id, "text": "the lexer drops tabs"});
        if let Some(w) = wake {
            args["wake"] = json!(w);
        }
        let out = h.okj(&a, "board_comment", args);
        has(
            &out,
            &["Commented comment:", &format!("on {}.", h.short(&id))],
        );
        if want_mode == WAKE {
            has(&out, &["Woke: beta"]);
        } else {
            has(&out, &["Told at their next turn: beta"]);
        }
        let cid = field(&out, "Commented");
        let cid = cid.strip_prefix("comment:").unwrap_or(&cid);
        let ns = h.notices(KEY_B);
        assert!(
            ns.len() == 1
                && ns[0].mode == want_mode
                && ns[0].message.ref_ == format!("comment:{cid}"),
            "{name}: notices {ns:?}"
        );
        let w = h.b.ib.next_wake(KEY_B, inbox::path::MOD).unwrap();
        assert_eq!(w.is_some(), want_mode == WAKE, "{name}: wake {w:?}");
        if want_mode == NEXT_TURN {
            let ap = h.b.ib.next_append(KEY_B).unwrap();
            assert!(
                ap.as_ref().is_some_and(|a| a.mode == NEXT_TURN),
                "{name}: append {ap:?}"
            );
        }
        // The commenter isn't told about its own comment.
        assert!(
            h.notices(KEY_A).is_empty(),
            "{name}: alpha got {:?}",
            h.notices(KEY_A)
        );
        has(
            &h.okj(&call_c(), "board", json!({"repo": "*"})),
            &[&format!("last: comment:{cid}"), "the lexer drops tabs"],
        );
    }
}

#[test]
fn test_board_comment_nobody_to_tell() {
    let h = new_harness();
    let a = call_a();
    let id = h.new_card(&a, json!({"title": "solo"}));
    h.okj(&a, "board_join", json!({"task_id": id}));
    has(
        &h.okj(
            &a,
            "board_comment",
            json!({"task_id": id, "text": "note to self"}),
        ),
        &["Nobody else holds it"],
    );
    h.failsj(
        &a,
        "board_comment",
        json!({"task_id": id}),
        &["comment text"],
    );
    h.failsj(&a, "board_comment", json!({"text": "x"}), &["give task_id"]);
    h.failsj(
        &a,
        "board_comment",
        json!({"task_id": "nope", "text": "x"}),
        &["board and leases tools"],
    );
}

#[test]
fn test_board_block_on() {
    let res = "path:remote:github.com/acme/app:src/api/**";
    let cases: Vec<(&str, String, Vec<String>)> = vec![
        (
            "no blocker",
            "".into(),
            vec!["Blocked #1 ".into(), "Anyone may board_unblock".into()],
        ),
        (
            "lease",
            format!("lease {res}"),
            vec![
                "unblocks by itself when that lease".into(),
                format!("(on lease {res})"),
            ],
        ),
        (
            "lease colon",
            format!("lease:{res}"),
            vec![format!("(on lease {res})")],
        ),
        ("person", "person".into(), vec!["(on person )".into()]),
        (
            "agent by name",
            "agent gamma".into(),
            vec![format!("(on agent {KEY_C})")],
        ),
        (
            "external",
            "external CI is red".into(),
            vec!["(on external CI is red)".into()],
        ),
        (
            "bad kind",
            "bogus x".into(),
            vec!["Error: ".into(), "on starts with".into()],
        ),
        (
            "lease without resource",
            "lease".into(),
            vec!["Error: ".into(), "needs the resource".into()],
        ),
        (
            "lease nobody holds",
            "lease gpu/qwen".into(),
            vec!["Error: ".into()],
        ),
        (
            "task without id",
            "task".into(),
            vec!["Error: ".into(), "needs its short ref or id".into()],
        ),
        (
            "task nobody has",
            "task #7".into(),
            vec!["Error: ".into(), "no card is".into()],
        ),
        (
            "named person",
            "person bob".into(),
            vec!["Error: ".into(), "takes no name".into()],
        ),
        (
            "unknown agent",
            "agent nobody".into(),
            vec!["Error: ".into(), "agents tool".into()],
        ),
    ];
    for (name, on, want) in cases {
        let h = new_harness();
        let a = call_a();
        h.okj(&call_c(), "lease", json!({"resource": res}));
        let id = h.new_card(&a, json!({"title": "api"}));
        h.okj(&a, "board_join", json!({"task_id": id}));
        let out = h.callj(
            &a,
            "board_block",
            json!({"task_id": id, "reason": "waiting", "on": on}),
        );
        assert_eq!(
            want[0].starts_with("Error: "),
            out.starts_with("Error: "),
            "{name}: got {out:?}"
        );
        for w in &want {
            assert!(
                out.contains(w.as_str()),
                "{name}: {out:?} should contain {w:?}"
            );
        }
    }
}

#[test]
fn test_board_block_unblock_status() {
    let h = new_harness();
    let (a, b, c) = (call_a(), call_b(), call_c());
    let id = h.new_card(
        &a,
        json!({"title": "migrate", "done_when": "tests pass", "shared": true}),
    );
    let r = h.short(&id);
    has(
        &h.okj(&a, "board_join", json!({"task_id": id})),
        &[&format!("You hold {r}")],
    );
    has(
        &h.okj(&b, "board_join", json!({"task_id": id})),
        &["with the others on it", "shared"],
    );
    h.failsj(
        &a,
        "board_block",
        json!({"task_id": id}),
        &["give question"],
    );
    has(
        &h.okj(
            &a,
            "board_block",
            json!({"task_id": id, "reason": "need the schema", "on": "person"}),
        ),
        &[r#"blocked [b1]: "need the schema""#],
    );
    has(&h.okj(&c, "board", json!({"repo": "*"})), &["Blocked (1)"]);

    // Anyone may unblock; the holders other than gamma hear at their next turn.
    let out = h.okj(
        &c,
        "board_unblock",
        json!({"task_id": id, "note": "schema is in"}),
    );
    has(
        &out,
        &[
            &format!("Unblocked {r};"),
            "working again",
            "Told at their next turn:",
            "alpha",
            "beta",
        ],
    );
    for k in [KEY_A, KEY_B] {
        let ns = h.notices(k);
        assert!(
            ns.len() == 1 && ns[0].mode == NEXT_TURN && ns[0].message.ref_ == format!("task:{id}"),
            "{k} notices {ns:?}"
        );
    }
    h.failsj(&c, "board_unblock", json!({"task_id": id}), &["blocked"]);

    h.failsj(
        &a,
        "board_status",
        json!({"task_id": id, "state": "blocked"}),
        &["use board_block"],
    );
    h.failsj(
        &a,
        "board_status",
        json!({"task_id": id, "state": "submitted"}),
        &["give state"],
    );
    h.failsj(
        &a,
        "board_status",
        json!({"task_id": id, "state": "done"}),
        &["give state"],
    );
    has(
        &h.okj(&b, "board_leave", json!({"task_id": id})),
        &[&format!("You left {r};")],
    );
    h.failsj(
        &b,
        "board_leave",
        json!({"task_id": id}),
        &["isn't on task"],
    );
    has(
        &h.okj(
            &a,
            "board_status",
            json!({"task_id": id, "state": "completed", "note": "shipped"}),
        ),
        &["is completed now", "folds into Done"],
    );

    has(
        &h.ok(&a, "board", "{}"),
        &["0 open cards, 1 done", "Nothing live here"],
    );
    has(
        &h.okj(&a, "board", json!({"done": true})),
        &["Done, newest first (1)", &format!("task:{id}")],
    );
    h.new_card(&a, json!({"title": "next"}));
    has(
        &h.ok(&a, "board", "{}"),
        &["Open (1)", "1 done cards are collapsed"],
    );
}

#[test]
fn test_board_link() {
    let h = new_harness();
    let b = call_b();
    let id = h.new_card(&call_a(), json!({"title": "auth"}));
    let r = h.short(&id);
    for (link, want) in [
        ("https://github.com/acme/app/issues/57", "GitHub #57"),
        (
            "https://www.notion.so/acme/Board-design-0123456789abcdef0123456789abcdef",
            "Notion Board design",
        ),
        (r"docs\design\board.md", ".../design/board.md"),
    ] {
        has(
            &h.okj(
                &b,
                "board_link",
                json!({"task_id": id, "url_or_path": link}),
            ),
            &[&format!("Linked {r}"), want],
        );
    }
    h.failsj(&b, "board_link", json!({"task_id": id}), &["url_or_path"]);
    h.failsj(
        &b,
        "board_link",
        json!({"task_id": id, "url_or_path": "../etc/passwd"}),
        &["leaves the repo"],
    );
    h.failsj(
        &b,
        "board_link",
        json!({"task_id": id, "url_or_path": "x", "label": "y"}),
        &[r#"unknown argument "label""#],
    );
}

/// board_edit: the holder renames a card when its purpose changes (card #74).
#[test]
fn test_board_edit() {
    let h = new_harness();
    let (a, b) = (call_a(), call_b());
    let co = h.b.co.clone();
    let id = h.new_card(&a, json!({"title": "Paste screenshots into an Ask"}));
    let r = h.short(&id);
    h.failsj(
        &a,
        "board_edit",
        json!({"task_id": id}),
        &["title, objective, done_when or repo"],
    );
    h.failsj(
        &a,
        "board_edit",
        json!({"task_id": id, "title": "  "}),
        &["needs a title"],
    );
    // Any agent may edit any card (card #320).
    has(
        &h.okj(
            &b,
            "board_edit",
            json!({"task_id": id, "title": "mine now"}),
        ),
        &[&format!("Edited {r}")],
    );
    has(
        &h.okj(
            &a,
            "board_edit",
            json!({"task_id": id, "title": "Ask box: thumbnails and file icons", "done_when": "pictures show as thumbnails"}),
        ),
        &[&format!("Edited {r}"), "board_steps", "Ask box: thumbnails and file icons"],
    );
    let t = co.get_task(&id).unwrap();
    assert_eq!(t.title, "Ask box: thumbnails and file icons");
    assert_eq!(t.done_criteria, "pictures show as thumbnails");
    let log = format!("{:?}", co.activity(&id).unwrap());
    has(&log, &["renamed it from", "Paste screenshots into an Ask"]);
    // Empty clears the objective; title stays.
    h.okj(&a, "board_edit", json!({"task_id": id, "objective": "x"}));
    h.okj(&a, "board_edit", json!({"task_id": id, "objective": ""}));
    let t = co.get_task(&id).unwrap();
    assert!(
        t.objective.is_empty() && t.title.starts_with("Ask box"),
        "{t:?}"
    );

    // The person renames it by hand (card #779): an agent's rename alone
    // is refused and says why; with other changes those land without it.
    co.rename_task("user:dev1", &id, "The person's name")
        .unwrap();
    h.failsj(
        &b,
        "board_edit",
        json!({"task_id": id, "title": "agent name"}),
        &["the person named", "leave its title"],
    );
    h.failsj(
        &b,
        "board_edit",
        json!({"task_id": id, "title": "agent name", "done_when": "thumbnails show"}),
        &["the person named"],
    );
    h.okj(
        &b,
        "board_edit",
        json!({"task_id": id, "done_when": "thumbnails show"}),
    );
    let t = co.get_task(&id).unwrap();
    assert_eq!(t.title, "The person's name");
    assert_eq!(t.done_criteria, "thumbnails show");

    // A card that was an ask: once closed, a later holder may rename it.
    let (ask, _) = co
        .post_ask(
            "user:dev1",
            coord::NewAsk {
                text: "screenshots?".into(),
                ..Default::default()
            },
            &[],
        )
        .unwrap();
    let ra = h.short(&ask.id);
    h.okj(&a, "ask_claim", json!({"task_id": ra}));
    h.failsj(
        &a,
        "board_edit",
        json!({"task_id": ra, "title": "x"}),
        &["ask_close"],
    );
    h.okj(
        &a,
        "ask_close",
        json!({"task_id": ra, "outcome": "card", "note": "on it", "title": "Paste screenshots"}),
    );
    h.okj(&a, "board_assign", json!({"task_id": ra, "to": "beta"}));
    has(
        &h.okj(
            &b,
            "board_edit",
            json!({"task_id": ra, "title": "Thumbnails in the Ask box"}),
        ),
        &[&format!("Edited {ra}")],
    );
    assert_eq!(
        co.get_task(&ask.id).unwrap().title,
        "Thumbnails in the Ask box"
    );
}

/// Leases that block something show on the Board.
#[test]
fn test_board_shows_blocking_lease() {
    let h = new_harness();
    let res = "path:remote:github.com/acme/app:src/**";
    h.okj(&call_b(), "lease", json!({"resource": res}));
    h.failsj(&call_a(), "lease", json!({"resource": res}), &["steal"]);
    let out = h.ok(&call_a(), "board", "{}");
    has(
        &out,
        &[
            "Leases in the way (1)",
            &format!("{res} held by beta"),
            "refused to alpha",
        ],
    );
    let v = h.b.co.board(&coord::BoardFilter::default());
    assert_eq!(v.leases.len(), 1, "leases {:?}", v.leases);
}

/// The cards: steps, blockers with recommended answers, watchers.
#[test]
fn test_board_cards() {
    let h = new_harness();
    let (a, b, c) = (call_a(), call_b(), call_c());
    let id = h.new_card(&a, json!({"title": "Ship v0.6.0"}));
    let r = h.short(&id);
    h.okj(&a, "board_join", json!({"task_id": id}));

    // Steps: strings or objects; the result lists their ids.
    let out = h.okj(
        &a,
        "board_steps",
        json!({"task_id": id, "steps": [{"text": "notes", "done": true}, {"text": "co-op check", "doing": true}, "tag"]}),
    );
    has(
        &out,
        &[
            "3 steps, 1 done",
            r#"[x] s1 "notes""#,
            r#"[>] s2 "co-op check""#,
            r#"[ ] s3 "tag""#,
        ],
    );
    h.failsj(
        &a,
        "board_steps",
        json!({"task_id": id, "steps": [{"text": "a", "doing": true}, {"text": "b", "doing": true}]}),
        &["only one step"],
    );
    h.failsj(
        &a,
        "board_step",
        json!({"task_id": id, "step_id": "s2"}),
        &["give done"],
    );
    h.failsj(
        &a,
        "board_step",
        json!({"task_id": id, "step_id": "s9", "done": true}),
        &["no step"],
    );
    has(
        &h.okj(
            &a,
            "board_step",
            json!({"task_id": id, "step_id": "s2", "done": true}),
        ),
        &["steps 2/3 done"],
    );

    // Watching.
    has(
        &h.okj(&b, "board_watch", json!({"task_id": id})),
        &[&format!("You watch {r}"), "watched by beta"],
    );

    // A blocker with context and options; beta recommends, gamma (not on it) can't.
    h.failsj(
        &a,
        "board_block",
        json!({"task_id": id, "question": "a", "reason": "b"}),
        &["not both"],
    );
    let out = h.okj(
        &a,
        "board_block",
        json!({"task_id": id, "question": "Ship today, or wait for the check?",
            "context": "Every automated check passes; the co-op check needs both PCs.",
            "options": [{"answer": "Wait, ship tonight", "why": "the temple is the headline"}]}),
    );
    has(
        &out,
        &[
            &format!("Blocked {r} on blocker b1"),
            r#"blocked [b1]: "Ship today, or wait for the check?""#,
            // A lone option is the recommendation.
            r#"1. "Wait, ship tonight" (alpha) [recommended]: "the temple is the headline""#,
        ],
    );
    has_not(&out, &["board_recommend so"]); // it gave an option
    has(
        &h.okj(
            &b,
            "board_recommend",
            json!({"task_id": id, "blocker_id": "b1", "answer": "Ship now, mark experimental", "why": "fixes today"}),
        ),
        &["Recommended on blocker b1", r#"2. "Ship now, mark experimental" (beta)"#],
    );
    h.failsj(
        &b,
        "board_recommend",
        json!({"task_id": id, "blocker_id": "b9", "answer": "x"}),
        &["no blocker"],
    );
    let out = h.okj(&c, "board", json!({"repo": "*"}));
    has(
        &out,
        &[
            "steps 2/3",
            "watched by beta",
            "[b1]",
            "Wait, ship tonight",
            "Ship now, mark experimental",
        ],
    );

    // The watcher's digest goes out on the sweep; the raiser hears of
    // beta's recommendation.
    for n in h.b.co.sweep_notices().unwrap() {
        tools::notice::route_notice(&h.b.ib, &inbox::system_sender(), &n);
    }
    let got = h.notices(KEY_B);
    assert!(
        got.len() == 1 && got[0].mode == NEXT_TURN,
        "beta's watch notices {got:?}"
    );
    has(&got[0].message.text, &["raised a blocker: Ship today"]);
    has_not(&got[0].message.text, &["ticked", "recommended"]); // before it watched; its own
    let got = h.notices(KEY_A);
    assert_eq!(got.len(), 1, "alpha's notices {got:?}");
    has(
        &got[0].message.text,
        &["recommended “Ship now, mark experimental”"],
    );

    // gamma answers b1 by picking beta's option: alpha (the raiser) hears.
    h.failsj(
        &c,
        "board_unblock",
        json!({"task_id": id, "blocker_id": "b1"}),
        &["give pick"],
    );
    h.failsj(
        &c,
        "board_unblock",
        json!({"task_id": id, "pick": 1}),
        &["pick needs blocker_id"],
    );
    let out = h.okj(
        &c,
        "board_unblock",
        json!({"task_id": id, "blocker_id": "b1", "pick": 2}),
    );
    has(
        &out,
        &[
            "Resolved blocker b1",
            "working again",
            "Told at their next turn: alpha",
        ],
    );
    let texts: Vec<String> = h
        .notices(KEY_A)
        .into_iter()
        .map(|d| d.message.text)
        .collect();
    has(
        &texts.join(" | "),
        &["Answer: Ship now, mark experimental", "That is the answer"],
    );
    has(
        &h.okj(&b, "board_unwatch", json!({"task_id": id})),
        &["no longer watch"],
    );
}

/// A reply on a blocker (board_comment blocker_id) routes no notice and
/// leaves the item open: answered by the person, then replied with what is
/// still missing.
#[test]
fn test_board_comment_on_blocker() {
    let h = new_harness();
    let (a, b, c) = (call_a(), call_b(), call_c());
    let id = h.new_card(&a, json!({"title": "VPN adapter", "shared": true}));
    h.okj(&a, "board_join", json!({"task_id": id}));
    h.okj(&b, "board_join", json!({"task_id": id}));
    h.okj(
        &a,
        "board_block",
        json!({"task_id": id, "question": "Which adapter?",
            "options": [{"answer": "Wintun"}, {"answer": "TAP-Windows"}]}),
    );
    let before: Vec<usize> = [KEY_A, KEY_B, KEY_C]
        .iter()
        .map(|k| h.notices(k).len())
        .collect();

    // Before the person answers, a reply only adds context: still open.
    let out = h.okj(
        &b,
        "board_comment",
        json!({"task_id": id, "blocker_id": "b1", "text": "Wintun is signed."}),
    );
    has(
        &out,
        &[
            &format!("Replied on blocker b1 of {}", h.short(&id)),
            "it stays open and goes back to the person",
            "board_unblock blocker_id=b1 note=",
        ],
    );
    has_not(&out, &["Nobody else holds it", "Told"]);
    has_not(&h.ok(&a, "board", "{}"), &["answered", "replied"]);

    h.b.co
        .answer(
            "user:dev1",
            &id,
            "b1",
            coord::BlockerAnswer {
                text: "which one needs a port?".into(),
                ..Default::default()
            },
        )
        .unwrap();
    has(
        &h.ok(&a, "board", "{}"),
        &[
            "Needs you (1)",
            "[b1]",
            "answered (waiting on alpha: resolve or reply)",
        ],
    );
    // Card #204: a plain reply after the answer acts on it; still the agent's.
    has(
        &h.okj(
            &a,
            "board_comment",
            json!({"task_id": id, "blocker_id": "b1", "text": "Wintun; wiring it now."}),
        ),
        &[
            "waits on you, not them",
            "board_unblock blocker_id=b1 note=",
        ],
    );
    has(
        &h.ok(&a, "board", "{}"),
        &["answered (waiting on alpha: resolve or reply)"],
    );

    h.failsj(
        &a,
        "board_comment",
        json!({"task_id": id, "blocker_id": "b1", "text": "x", "missing": ["1", "2", "3", "4", "5", "6", "7"]}),
        &["at most 6"],
    );
    h.failsj(
        &a,
        "board_comment",
        json!({"task_id": id, "text": "x", "missing": ["a"]}),
        &["needs blocker_id"],
    );
    h.failsj(
        &a,
        "board_comment",
        json!({"task_id": id, "blocker_id": "b1", "text": "x", "missing": [" "]}),
        &["missing line"],
    );
    h.failsj(
        &a,
        "board_comment",
        json!({"task_id": id, "blocker_id": "b9", "text": "x"}),
        &["no blocker"],
    );
    has(
        &h.okj(
            &a,
            "board_comment",
            json!({"task_id": id, "blocker_id": "b1", "text": "TAP needs one.", "missing": ["the port number"]}),
        ),
        &["Replied on blocker b1"],
    );
    let out = h.okj(&c, "board", json!({"repo": "*"}));
    has(
        &out,
        &[r#"replied (with the person), missing: "the port number""#],
    );
    has_not(&out, &["answered ("]);
    for (i, k) in [KEY_A, KEY_B, KEY_C].iter().enumerate() {
        let got = h.notices(k).len();
        assert_eq!(
            got, before[i],
            "{k} got {got} notices, had {}: a blocker reply routed one",
            before[i]
        );
    }
    let task = h.b.co.get_task(&id).unwrap();
    let bl = &task.blockers[0];
    assert!(
        bl.status == coord::BLOCKER_REPLIED && bl.open() && task.state == coord::BLOCKED,
        "task {} blocker {bl:?}",
        task.state
    );
}

/// Once the person has answered, board_unblock resolves with a note: their
/// answer stands and the note shows beside it.
#[test]
fn test_board_unblock_resolves_with_note() {
    let h = new_harness();
    let (a, b, c) = (call_a(), call_b(), call_c());
    let id = h.new_card(&a, json!({"title": "VPN adapter", "shared": true}));
    h.okj(&a, "board_join", json!({"task_id": id}));
    h.okj(&b, "board_join", json!({"task_id": id}));
    h.okj(
        &a,
        "board_block",
        json!({"task_id": id, "question": "Which adapter?", "options": [{"answer": "Wintun"}]}),
    );
    h.okj(
        &a,
        "board_block",
        json!({"task_id": id, "question": "Which port?"}),
    );
    let r = h.short(&id);

    // Nobody answered b2: a note is the answer, as before.
    let out = h.okj(
        &c,
        "board_unblock",
        json!({"task_id": id, "blocker_id": "b2", "note": "4242"}),
    );
    has(
        &out,
        &[&format!("Resolved blocker b2 on {r}; 1 still open.")],
    );
    has_not(&out, &["with your note"]);

    h.b.co
        .answer(
            "user:dev1",
            &id,
            "b1",
            coord::BlockerAnswer {
                pick: 1,
                ..Default::default()
            },
        )
        .unwrap();
    h.failsj(
        &c,
        "board_unblock",
        json!({"task_id": id, "blocker_id": "b1", "pick": 1}),
        &["already answered", "give note"],
    );
    h.failsj(
        &c,
        "board_unblock",
        json!({"task_id": id, "blocker_id": "b1"}),
        &["give pick"],
    );
    h.failsj(
        &a,
        "board_unblock",
        json!({"task_id": id, "blocker_id": "b1", "note": "a\nb"}),
        &["one line"],
    );
    let before = h.notices(KEY_B).len();
    let out = h.okj(
        &a,
        "board_unblock",
        json!({"task_id": id, "blocker_id": "b1", "note": "Shipping Wintun tonight."}),
    );
    has(
        &out,
        &[
            &format!("Resolved blocker b1 on {r} with your note; it is working again."),
            "Told at their next turn: beta",
        ],
    );
    let ns = h.notices(KEY_B);
    assert_eq!(ns.len(), before + 1, "beta notices {ns:?}");
    let texts: Vec<String> = ns.into_iter().map(|d| d.message.text).collect();
    has(
        &texts.join(" | "),
        &[
            "resolved blocker b1",
            "Shipping Wintun tonight.",
            "The person's answer: Wintun",
        ],
    );
    let task = h.b.co.get_task(&id).unwrap();
    let res = task.blockers[0].resolved.as_ref();
    assert!(
        res.is_some_and(|r| r.answer == "Wintun"
            && r.note == "Shipping Wintun tonight."
            && r.option == 0),
        "resolution {res:?}"
    );
    h.failsj(
        &b,
        "board_comment",
        json!({"task_id": id, "blocker_id": "b1", "text": "late"}),
        &["already resolved"],
    );
}

/// Waiting on the person is one blocker per ask: board_status
/// input_required is refused on a card with no open blocker, and is a
/// no-op (it would close them) on one that has them.
#[test]
fn test_board_input_required_needs_blockers() {
    let h = new_harness();
    let a = call_a();
    let id = h.new_card(&a, json!({"title": "TRELLIS.2 test"}));
    h.okj(&a, "board_join", json!({"task_id": id}));
    let note = "request access to dinov3 on huggingface and run hf auth login? may I push mokei-ui? where should the out/ backup go?";
    h.failsj(
        &a,
        "board_status",
        json!({"task_id": id, "state": "input_required", "note": note}),
        &[
            "your note asks more than one thing",
            "its own board_block",
            "context for a newcomer",
            "recommended: true",
        ],
    );
    h.failsj(
        &a,
        "board_status",
        json!({"task_id": id, "state": "input_required"}),
        &["as a blocker", "board_block"],
    );
    let st = h.b.co.get_task(&id).unwrap().state;
    assert_eq!(
        st,
        coord::WORKING,
        "a refused input_required moved the card"
    );

    // Three asks, three blockers; the push one with options and a recommendation.
    h.okj(
        &a,
        "board_block",
        json!({"task_id": id, "question": "Request access to facebook/dinov3-vitl16-pretrain-lvd1689m",
            "context": "Meta approves by hand on huggingface.co; then run hf auth login on the Mac."}),
    );
    let out = h.okj(
        &a,
        "board_block",
        json!({"task_id": id, "question": "May I push mokei-ui?", "options": [
            {"answer": "Push to main", "why": "checks pass", "recommended": true},
            {"answer": "Open a PR first"},
            {"answer": "Hold"}]}),
    );
    has(
        &out,
        &[
            "on blocker b2",
            r#"1. "Push to main" (alpha) [recommended]: "checks pass""#,
            r#"2. "Open a PR first" (alpha)"#,
        ],
    );
    has_not(&out, &["board_recommend", "more than one thing"]);
    h.failsj(
        &a,
        "board_block",
        json!({"task_id": id, "question": "Where should the out/ backup go?",
            "options": [{"answer": "D:", "recommended": true}, {"answer": "NAS", "recommended": true}]}),
        &["at most one"],
    );
    has(
        &h.okj(
            &a,
            "board_block",
            json!({"task_id": id, "question": "Where should the out/ backup go?",
                "options": [{"answer": "D:/backup"}, {"answer": "The NAS"}]}),
        ),
        &["on blocker b3", "None of its options is marked recommended"],
    );
    has(
        &h.okj(
            &a,
            "board_block",
            json!({"task_id": id, "question": "Ship? And tag?"}),
        ),
        &["asks more than one thing"],
    );

    // With open blockers, input_required changes nothing.
    let out = h.okj(
        &a,
        "board_status",
        json!({"task_id": id, "state": "input_required", "note": "see above"}),
    );
    has(
        &out,
        &[
            "already waits on 4 open blockers (b1, b2, b3, b4)",
            "stays blocked",
            "note was not saved",
        ],
    );
    let tk = h.b.co.get_task(&id).unwrap();
    assert!(
        tk.state == coord::BLOCKED && tk.open_blockers().len() == 4,
        "input_required touched the blockers: {} {}",
        tk.state,
        tk.open_blockers().len()
    );
}

/// board_create takes the card's steps, the same shape board_steps takes.
#[test]
fn test_board_create_with_steps() {
    let h = new_harness();
    let a = call_a();
    h.failsj(
        &a,
        "board_create",
        json!({"title": "x", "steps": [{"text": "a", "doing": true}, {"text": "b", "doing": true}]}),
        &["only one step"],
    );
    let out = h.okj(
        &a,
        "board_create",
        json!({"title": "Release", "steps": [{"text": "notes", "done": true}, {"text": "tag", "doing": true}, "announce"]}),
    );
    has(
        &out,
        &[
            "Created #",
            "Tick its steps with board_step",
            "steps 1/3",
            r#"now s2 "tag""#,
        ],
    );
    has_not(&out, &["Give it steps"]);
    has(
        &h.okj(&a, "board_create", json!({"title": "No plan yet"})),
        &["Give it steps with board_steps"],
    );
}

/// Every tool that takes a card takes its short ref (#2, 2, task:#2) as
/// well as its id, and prints the short ref back.
#[test]
fn test_short_refs_in_the_board_tools() {
    let r = with_capture(|_| {});
    let (a, b) = (call_a(), call_b());
    let one = r.new_card(&a, json!({"title": "First card"}));
    let two = r.new_card(&a, json!({"title": "Second card"}));
    has(
        &r.okj(&a, "board", json!({})),
        &[&format!("- #1 task:{one} ["), &format!("- #2 task:{two} [")],
    );
    has(
        &r.okj(&a, "board_join", json!({"task_id": "#2"})),
        &["You hold #2 now"],
    );
    has(
        &r.okj(
            &a,
            "board_comment",
            json!({"task_id": "task:#2", "text": "on it"}),
        ),
        &[" on #2."],
    );
    has(
        &r.okj(
            &a,
            "board_steps",
            json!({"task_id": "2", "steps": ["a", "b"]}),
        ),
        &["#2 has 2 steps"],
    );
    has(
        &r.okj(
            &a,
            "board_block",
            json!({"task_id": "#2", "question": "Wait for the first?", "on": "task #1"}),
        ),
        &["Blocked #2"],
    );
    let x = r.b.co.get_task(&two).unwrap();
    assert!(
        x.blocked_by.as_ref().is_some_and(|bb| bb.reference == one),
        "on task #1 stored {:?}",
        x.blocked_by
    );
    r.failsj(
        &a,
        "board_block",
        json!({"task_id": "#1", "question": "Wait?", "on": "task #9"}),
        &["no card is"],
    );
    r.failsj(
        &a,
        "board_join",
        json!({"task_id": "#9"}),
        &["no such task"],
    );
    has(
        &r.okj(&a, "board_join", json!({"task_id": one})),
        &["You hold #1 now"],
    );
    has(
        &r.okj(&b, "show", json!({"ref": "#2"})),
        &[&format!("task:{two}"), "Task #2 ", "Second card"],
    );
    has(
        &r.okj(&b, "show", json!({"ref": "task:#2"})),
        &[&format!("task:{two}"), "Task #2 "],
    );
    r.failsj(&b, "show", json!({"ref": "#9"}), &["no card is #9"]);
    let out = r.okj(&b, "search", json!({"query": "#2"}));
    let i = out
        .find("\n- ")
        .unwrap_or_else(|| panic!("no hits:\n{out}"));
    assert!(
        out[i + 1..].starts_with(&format!("- #2 task:{two} ")),
        "#2 is not the first hit:\n{out}"
    );
    has(
        &r.okj(&b, "search", json!({"query": "first card"})),
        &[&format!("- #1 task:{one}")],
    );
}

/// A number two cards claimed on devices that were apart is refused bare
/// by every tool, with both cards named, so a #1 an agent saw before the
/// clash settled never acts on the other card; the lettered refs work.
#[test]
fn test_clashed_short_ref_is_refused() {
    let h = with_capture(|_| {}); // show reads through capture
    let a = call_a();
    let mine = h.new_card(&a, json!({"title": "Mine"}));
    // Another of the person's devices put up its own #1 a second earlier,
    // out of reach; its row arrives now (Go rebuilds the coord over the
    // store; here the live coord takes it as a followed row).
    let at = plus(h.clk.now(), -1);
    let payload = json!({"taskId": "01THEIRS", "num": 1, "title": "Theirs", "by": "dev9/s",
        "at": at.format_rfc3339_nano()});
    let ev = store::Event {
        device_id: "dev9".into(),
        seq: 1,
        hlc: hlc::Timestamp::from_unix_ms(at.unix_milli_value()),
        actor: "agent:dev9/s".into(),
        kind: "task.created".into(),
        payload: serde_json::to_vec(&payload).unwrap(),
        sig: Vec::new(),
    };
    h.b.st.insert(&ev).unwrap();
    let co = h.b.co.clone();
    co.handle(&ev).unwrap();
    has(
        &h.okj(&a, "board", json!({})),
        &[&format!("- #1b task:{mine} [")],
    );
    assert_eq!(co.get_task("01THEIRS").unwrap().short_ref(), "#1a");
    let named = format!(r#"#1b "Mine" (task:{mine})"#);
    for (name, args) in [
        (
            "board_comment",
            json!({"task_id": "#1", "text": "meant mine"}),
        ),
        ("board_status", json!({"task_id": "1", "state": "done"})),
        ("board_join", json!({"task_id": "task:#1"})),
        (
            "board_block",
            json!({"task_id": mine, "question": "Wait?", "on": "task #1"}),
        ),
        ("show", json!({"ref": "#1"})),
    ] {
        h.failsj(
            &a,
            name,
            args,
            &[
                "each claimed #1",
                r#"#1a "Theirs" (task:01THEIRS)"#,
                &named,
                "lettered ref",
            ],
        );
    }
    // show task:#1 goes through the search sources; coord's says the same.
    match co.search_show("task:#1", &search::ShowOptions::default()) {
        Err(e) => assert!(e.to_string().contains("#1a"), "show task:#1: {e}"),
        Ok(sh) => panic!("show task:#1 answered {sh:?}"),
    }
    let x = co.get_task("01THEIRS").unwrap();
    assert!(
        x.comments == 0 && x.state == coord::SUBMITTED,
        "the other card was acted on: {x:?}"
    );
    has(
        &h.okj(
            &a,
            "board_comment",
            json!({"task_id": "#1b", "text": "meant mine"}),
        ),
        &[" on #1b."],
    );
    let sh = co
        .search_show("task:#1a", &search::ShowOptions::default())
        .unwrap();
    assert!(sh.text.contains("it was #1"), "show #1a: {sh:?}");
}

/// One heavily blocked card must not push every other card off the board:
/// a list line shows the first open item and counts the rest.
#[test]
fn test_board_blocked_card_fits_the_list() {
    let h = new_harness();
    let a = call_a();
    h.new_card(&a, json!({"title": "Small card"}));
    let big = h.new_card(&a, json!({"title": "Big card"}));
    h.okj(&a, "board_join", json!({"task_id": big}));
    let long = "x".repeat(150);
    for i in 0..coord::MAX_OPEN_BLOCKERS {
        let ci = (b'a' + i as u8) as char;
        let opts: Vec<J> = (0..coord::MAX_OPTIONS)
            .map(|j| {
                let cj = (b'a' + j as u8) as char;
                json!({"answer": format!("{long}{cj}"), "why": format!("{long}{ci}")})
            })
            .collect();
        h.okj(
            &a,
            "board_block",
            json!({"task_id": big, "question": format!("{ci}{}", "q".repeat(190)), "options": opts}),
        );
    }
    let out = h.okj(&a, "board", json!({"repo": "*"}));
    has(
        &out,
        &["Small card", "Big card", "[b1]", "7 more open items"],
    );
    has_not(&out, &["not shown", "[b2]"]);
}

/// board_split turns a card into a parent: open steps move to subtasks, an
/// unplaced one is named, an assignee is told, and the board shows each
/// subtask with its parent and the parent with its subtask count.
#[test]
fn test_board_split() {
    let h = new_harness();
    let a = call_a();
    let id = h.new_card(
        &a,
        json!({"title": "Big job", "steps": [{"text": "plan", "done": true}, "backend", "window"]}),
    );
    let r = h.short(&id);
    h.failsj(
        &a,
        "board_split",
        json!({"task_id": r, "subtasks": [{"title": "Backend", "steps": ["s2"]}]}),
        &[r#"not placed: s3 "window""#],
    );
    let out = h.okj(
        &a,
        "board_split",
        json!({"task_id": r, "subtasks": [
            {"title": "Backend", "steps": ["s2", "migrations"], "assign_to": "beta"},
            {"title": "Window", "steps": ["s3"]}]}),
    );
    has(
        &out,
        &[
            &format!("{r} is a parent now"),
            "Created #",
            "Assigned to beta",
            "0/2 subtasks",
            "subtasks: #",
        ],
    );
    assert!(!h.notices(KEY_B).is_empty(), "the assignee heard nothing");
    let p = h.b.co.get_task(&id).unwrap();
    assert!(p.subtasks.len() == 2 && p.steps.len() == 1, "parent {p:?}");
    let board = h.okj(&a, "board", json!({"repo": "*"}));
    has(&board, &[&format!("part of {r}"), "0/2 subtasks"]);
    // A subtask of a subtask is refused, and the parent's steps can't be set.
    h.failsj(
        &a,
        "board_create",
        json!({"title": "deeper", "parent": h.short(&p.subtasks[0])}),
        &["one level"],
    );
    h.failsj(
        &a,
        "board_steps",
        json!({"task_id": r, "steps": [{"text": "x"}]}),
        &["its subtasks"],
    );
    has(
        &h.okj(&a, "board_create", json!({"title": "Docs", "parent": r})),
        &[&format!("a subtask of {r}")],
    );
}

// ---- blocker_rich_test.go ----

/// board_block carries links and files on the blocker, and a link on each
/// option; show and the board list them for agents.
#[test]
fn test_board_block_links_and_attach() {
    let h = new_harness();
    assert!(h.b.attacher.set(Arc::new(NamedFiles)).is_ok());
    let a = call_a();
    let id = h.new_card(&a, json!({"title": "Train the probe"}));
    h.okj(&a, "board_join", json!({"task_id": id}));

    h.failsj(
        &a,
        "board_block",
        json!({"task_id": id, "question": "q?", "links": ["ftp://nope"]}),
        &["http(s)"],
    );
    h.failsj(
        &a,
        "board_block",
        json!({"task_id": id, "question": "q?", "options": [{"answer": "a", "link": "data://x"}]}),
        &["http(s)"],
    );
    h.failsj(
        &a,
        "board_block",
        json!({"task_id": id, "question": "q?", "links": ["a", "b", "c", "d", "e", "f", "g", "h", "i"]}),
        &["at most 8"],
    );

    let url = "https://huggingface.co/facebook/dinov3-vitl16-pretrain-lvd1689m";
    let out = h.okj(
        &a,
        "board_block",
        json!({"task_id": id, "question": "Request DINOv3 access?",
            "context": format!("Open {url} and click Request access, then type `! hf auth login` in the Mac session."),
            "links": [url, "docs/models.md"],
            "attach": ["shots/gate.png"],
            "options": [
                {"answer": "I requested it", "recommended": true, "link": "https://huggingface.co/settings/gated-repos"},
                {"answer": "Use DINOv2 instead"}]}),
    );
    has(
        &out,
        &[
            "Blocked #1 on blocker b1 with gate.png",
            "and links huggingface.co, docs/models.md.",
        ],
    );

    let tk = h.b.co.get_task(&id).unwrap();
    let bl = &tk.open_blockers()[0];
    assert!(
        bl.links.len() == 2
            && bl.links[0].reference == url
            && bl.attachments.len() == 1
            && bl.attachments[0].name == "gate.png"
            && bl.options[0]
                .link
                .as_ref()
                .is_some_and(|l| l.reference == "https://huggingface.co/settings/gated-repos")
            && bl.options[1].link.is_none(),
        "blocker {bl:?}"
    );
    for o in [out, h.ok(&call_b(), "board", "{}")] {
        has(
            &o,
            &[
                &format!("links: {url}, docs/models.md"),
                "file: gate.png",
                "(attachment_get)",
                r#""I requested it" (alpha) [recommended] link: https://huggingface.co/settings/gated-repos"#,
            ],
        );
    }

    // Links alone still raise an item (not the reason-only form).
    let id2 = h.new_card(&a, json!({"title": "Second"}));
    h.okj(&a, "board_join", json!({"task_id": id2}));
    has(
        &h.okj(
            &a,
            "board_block",
            json!({"task_id": id2, "reason": "waiting on the vendor", "links": ["https://example.com/status"]}),
        ),
        &["with links example.com"],
    );
    let tk = h.b.co.get_task(&id2).unwrap();
    assert!(
        tk.open_blockers().len() == 1 && tk.open_blockers()[0].links.len() == 1,
        "second {:?}",
        tk.blockers
    );
}

// ---- room_test.go ----

/// The ask tools (board.md "Asks"): the room hears, one takes it, it ends
/// with what it became.
#[test]
fn test_ask_tools() {
    let h = new_harness();
    let (a, b, c) = (call_a(), call_b(), call_c());
    let co = h.b.co.clone();
    let agents: Vec<coord::roster::AgentCard> =
        h.b.roster
            .cards()
            .into_iter()
            .map(|c| coord::roster::AgentCard {
                key: c.key,
                device_id: c.device_id,
                agent: c.agent,
                session_id: c.session_id,
                name: c.name,
                status: c.status,
                repo_key: c.repo_key,
                permission_mode: c.permission_mode,
                cwd: c.cwd,
                ..Default::default()
            })
            .collect();
    let new_ask = |text: &str| coord::NewAsk {
        text: text.into(),
        ..Default::default()
    };
    let (ask, n) = co
        .post_ask("user:dev1", new_ask("The login page feels slow"), &agents)
        .unwrap();
    assert_eq!(n.to.len(), 1, "told {:?}", n.to);
    let r = h.short(&ask.id);
    // Handed to alpha, which holds it as it wakes.
    assert_eq!(n.to, [call_a().sender.key.clone()]);
    has(&h.ok(&a, "board", "{}"), &["Asks you hold", &r, "[ask]"]);

    has(
        &h.okj(&a, "ask_claim", json!({"task_id": r})),
        &[&format!("You have {r}"), "ask_close"],
    );
    has(
        &h.okj(&b, "ask_claim", json!({"task_id": r})),
        &["alpha", &format!("took {r}")],
    );
    h.failsj(&b, "board_join", json!({"task_id": r}), &["ask_claim"]);
    h.failsj(
        &b,
        "board_assign",
        json!({"task_id": r, "to": "beta"}),
        &["ask_claim"],
    );
    h.failsj(
        &a,
        "board_status",
        json!({"task_id": r, "state": "completed"}),
        &["ask_close"],
    );
    has(&h.ok(&a, "board", "{}"), &["Asks you hold", &r]);

    has(
        &h.okj(
            &b,
            "board_comment",
            json!({"task_id": r, "text": "the API is slow too"}),
        ),
        &["You watch it now"],
    );
    let got = co.get_task(&ask.id).unwrap();
    assert!(
        got.watchers.len() == 1 && got.watchers[0] == KEY_B,
        "watchers {:?}",
        got.watchers
    );
    h.failsj(
        &b,
        "ask_close",
        json!({"task_id": r, "outcome": "none", "note": "x"}),
        &["holds"],
    );
    h.failsj(
        &a,
        "ask_close",
        json!({"task_id": r, "outcome": "card", "note": "x"}),
        &["title"],
    );
    h.failsj(
        &a,
        "ask_close",
        json!({"task_id": r, "outcome": "maybe", "note": "x"}),
        &["outcome"],
    );
    let out = h.okj(
        &a,
        "ask_close",
        json!({"task_id": r, "outcome": "card", "note": "a real fix", "title": "Speed up login",
            "steps": [{"text": "profile"}, {"text": "fix"}]}),
    );
    has(
        &out,
        &[
            &format!("{r} is a card now"),
            r#""Speed up login""#,
            "steps 0/2",
        ],
    );
    let got = co.get_task(&ask.id).unwrap();
    assert!(
        got.holder == KEY_A
            && got.state == coord::WORKING
            && !got.ask.as_ref().is_some_and(|x| x.is_open()),
        "after the card outcome {got:?}"
    );
    assert!(
        h.okj(&a, "board", json!({"repo": "*"}))
            .contains("Working (1)"),
        "the card isn't an ordinary working card"
    );

    // edit and none.
    let other = h.new_card(&c, json!({"title": "Ship docs"}));
    let (two, _) = co
        .post_ask("user:dev1", new_ask("docs should say Windows"), &[])
        .unwrap();
    let r2 = h.short(&two.id);
    h.okj(&c, "ask_claim", json!({"task_id": r2}));
    has(
        &h.okj(
            &c,
            "ask_close",
            json!({"task_id": r2, "outcome": "edit", "task": h.short(&other), "note": "added it", "objective": "with Windows"}),
        ),
        &[&format!("Changed {}", h.short(&other))],
    );
    let o = co.get_task(&other).unwrap();
    assert_eq!(o.objective, "with Windows", "edited card {o:?}");
    let (three, _) = co
        .post_ask("user:dev1", new_ask("never mind"), &[])
        .unwrap();
    let r3 = h.short(&three.id);
    h.okj(&c, "ask_claim", json!({"task_id": r3}));
    has(
        &h.okj(&c, "board_leave", json!({"task_id": r3})),
        &[&format!("gave the ask {r3} back")],
    );
    h.okj(&b, "ask_claim", json!({"task_id": r3}));
    has(
        &h.okj(
            &b,
            "ask_close",
            json!({"task_id": r3, "outcome": "none", "note": "the person sorted it"}),
        ),
        &[&format!("Closed {r3}")],
    );

    // answered: the ask completes, and the taker keeps it.
    let (four, _) = co
        .post_ask("user:dev1", new_ask("which port?"), &[])
        .unwrap();
    let r4 = h.short(&four.id);
    h.okj(&b, "ask_claim", json!({"task_id": r4}));
    has(
        &h.okj(
            &b,
            "ask_close",
            json!({"task_id": r4, "outcome": "answered", "note": "8080"}),
        ),
        &[&format!("Answered {r4}"), "reopens"],
    );
    let got = co.get_task(&four.id).unwrap();
    assert!(
        got.state == coord::COMPLETED
            && got
                .ask
                .as_ref()
                .and_then(|x| x.outcome.as_ref())
                .is_some_and(|o| o.kind == coord::OUTCOME_ANSWERED),
        "after the answered outcome {got:?}"
    );
}

// ---- owner_test.go ----

/// The tools use coord's owner rule for "the person" (v0.4 §2): the
/// person's own key passes as an assignee and shows as the person; a
/// made-up "user:" key is just an unknown name. Go tests the unexported
/// assignee and names.short; here the same rule through board_create and
/// board_assign.
#[test]
fn test_tools_use_the_owner_rule() {
    let h = new_harness();
    let a = call_a();
    let out = h.okj(
        &a,
        "board_create",
        json!({"title": "for you", "assign_to": "user:dev1"}),
    );
    has(&out, &["Assigned to the person"]);
    let id = created_id(&out);
    let t = h.b.co.get_task(&id).unwrap();
    assert_eq!(t.assigned, vec!["user:dev1".to_string()], "{t:?}");
    h.failsj(
        &a,
        "board_create",
        json!({"title": "for nobody", "assign_to": "user:made-up"}),
        &["agents tool"],
    );
    let out = h.callj(
        &a,
        "board_assign",
        json!({"task_id": id, "to": "user:made-up"}),
    );
    assert!(out.starts_with("Error: "), "{out}");
    has_not(&out, &["the person"]);
}

/// Card #50: the person answers a blocker with a question. The raiser
/// answers it on the blocker, then revises the blocker (board_block
/// blocker_id) with options that fit, instead of leaving the same ones up.
#[test]
fn test_board_block_revises() {
    let h = new_harness();
    let a = call_a();
    let id = h.new_card(&a, json!({"title": "VPN adapter", "shared": true}));
    h.okj(&a, "board_join", json!({"task_id": id}));
    h.okj(
        &a,
        "board_block",
        json!({"task_id": id, "question": "Which adapter?",
            "options": [{"answer": "Wintun"}, {"answer": "TAP-Windows"}]}),
    );
    h.b.co
        .answer(
            "user:dev1",
            &id,
            "b1",
            coord::BlockerAnswer {
                text: "which one is signed?".into(),
                ..Default::default()
            },
        )
        .unwrap();
    // The reply's result says to revise or resolve, not to wait.
    has(
        &h.okj(
            &a,
            "board_comment",
            json!({"task_id": id, "blocker_id": "b1", "text": "Wintun is signed, TAP is not."}),
        ),
        &[
            "revise it now with board_block blocker_id=b1",
            "don't leave the same options up",
        ],
    );
    // The same question and options again: refused.
    h.failsj(
        &a,
        "board_block",
        json!({"task_id": id, "blocker_id": "b1",
            "options": [{"answer": "Wintun"}, {"answer": "TAP-Windows"}]}),
        &["same question with the same options", "board_unblock"],
    );
    h.failsj(
        &a,
        "board_block",
        json!({"task_id": id, "blocker_id": "b1", "question": "Ship Wintun?", "on": "person"}),
        &["revises that blocker"],
    );
    h.failsj(
        &a,
        "board_block",
        json!({"task_id": id, "blocker_id": "b9", "question": "Ship Wintun?"}),
        &["no blocker"],
    );
    let before = h.notices(KEY_A).len();
    let out = h.okj(
        &a,
        "board_block",
        json!({"task_id": id, "blocker_id": "b1", "question": "Ship with Wintun, the signed one?",
            "options": [{"answer": "Yes", "recommended": true}, {"answer": "No, wait"}]}),
    );
    has(
        &out,
        &[
            "Revised blocker b1 on",
            "\"Ship with Wintun, the signed one?\"",
            "with 2 options",
            "waits on them again",
        ],
    );
    has_not(&out, &["Blocked "]);
    assert_eq!(h.notices(KEY_A).len(), before, "a revision routed a notice");
    let task = h.b.co.get_task(&id).unwrap();
    assert_eq!(task.blockers.len(), 1);
    let bl = &task.blockers[0];
    let answers: Vec<&str> = bl.options.iter().map(|o| o.answer.as_str()).collect();
    assert_eq!(answers, ["Yes", "No, wait"]);
    assert!(bl.status == coord::BLOCKER_OPEN && bl.open() && task.state == coord::BLOCKED);
    has(
        &h.ok(&a, "board", "{}"),
        &["Ship with Wintun, the signed one?", "Yes"],
    );
    // Now a plain question, no options.
    has(
        &h.okj(
            &a,
            "board_block",
            json!({"task_id": id, "blocker_id": "b1", "question": "Anything else before I ship?"}),
        ),
        &["with no options"],
    );
    assert!(h.b.co.get_task(&id).unwrap().blockers[0].options.is_empty());
}

/// The blocker tools tell an agent to answer a question on a blocker and
/// then resolve or revise it, never to re-present the same options.
#[test]
fn test_blocker_tool_descriptions() {
    let h = new_harness();
    let specs: serde_json::Value = serde_json::from_slice(&h.set.specs_json("claude")).unwrap();
    let spec = |n: &str| {
        specs
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == n)
            .unwrap_or_else(|| panic!("no {n}"))
            .clone()
    };
    let block = spec("board_block");
    has(
        block["description"].as_str().unwrap(),
        &[
            "If they asked a question, answer it",
            "Never re-offer the same options",
        ],
    );
    has(
        block["inputSchema"]["properties"]["blocker_id"]["description"]
            .as_str()
            .unwrap(),
        &[
            "revise this open blocker",
            "Any agent may; the card's holders hear it",
        ],
    );
    let comment = spec("board_comment");
    has(
        comment["inputSchema"]["properties"]["blocker_id"]["description"]
            .as_str()
            .unwrap(),
        &["answer the person's question", "board_block blocker_id"],
    );
    has(
        spec("board_unblock")["description"].as_str().unwrap(),
        &["revise with board_block blocker_id"],
    );
    has(
        tools::BOARD_INSTRUCTIONS,
        &[
            "reply with a question",
            "never offer the same options again",
            // Card #204: resolve when acting on the answer; missing asks back.
            "in the same turn you act on the answer",
            "give missing or revise it",
        ],
    );
    has(
        comment["description"].as_str().unwrap(),
        &["waits on you, not them", "To ask them back, give missing"],
    );
}

/// #91: skipping the delivery of an ask Irori handed you passes it on at
/// once, to the next idle agent.
#[test]
fn test_skipping_a_handed_ask_passes_it_on() {
    let h = new_harness();
    let a = call_a();
    let co = h.b.co.clone();
    let agents: Vec<coord::roster::AgentCard> =
        h.b.roster
            .cards()
            .into_iter()
            .map(|c| coord::roster::AgentCard {
                key: c.key,
                device_id: c.device_id,
                status: c.status,
                repo_key: c.repo_key,
                ..Default::default()
            })
            .collect();
    let (ask, n) = co
        .post_ask(
            "user:dev1",
            coord::NewAsk {
                text: "Is the nightly build green?".into(),
                ..Default::default()
            },
            &agents,
        )
        .unwrap();
    assert_eq!(n.to, [KEY_A]);
    tools::notice::route_notice(&h.b.ib, &inbox::system_sender(), &n);
    let d = h.notices(KEY_A);
    assert_eq!(d.len(), 1, "{d:?}");
    let r = h.short(&ask.id);
    has(
        &h.okj(&a, "skip", json!({"delivery_id": d[0].id})),
        &["gave the ask", &r, "next idle agent"],
    );
    let t = co.get_task(&ask.id).unwrap();
    assert!(t.ask.as_ref().unwrap().taker.is_empty(), "{t:?}");
    let ns = co.sweep_asks(&agents).unwrap();
    assert!(
        ns.len() == 1 && ns[0].to.len() == 1 && ns[0].to[0] != KEY_A,
        "{ns:?}"
    );
}

/// Step owners (card #86): board_steps and board_step name the agent doing
/// a step, which wakes it and makes it a co-holder until the step is done.
#[test]
fn test_step_owners() {
    let h = new_harness();
    let (a, b) = (call_a(), call_b());
    let out = h.okj(&a, "board_create", json!({"title": "two halves"}));
    let id = created_id(&out);
    h.okj(&a, "board_join", json!({"task_id": id}));
    let out = h.okj(
        &a,
        "board_steps",
        json!({"task_id": id, "steps": [
            {"text": "server half"},
            {"text": "window half", "owner": "beta"},
        ]}),
    );
    has(&out, &["Woke: beta", "s2 \"window half\" (owner: beta"]);
    has(
        &h.ok(&a, "board", "{}"),
        &["with beta (dev1/bbbbbbbb-2222-4222-8222-222222222222) (s2) on steps"],
    );
    // beta ticks its own step.
    h.okj(
        &b,
        "board_step",
        json!({"task_id": id, "step_id": "s2", "done": true}),
    );
    let out = h.ok(&a, "board", "{}");
    assert!(!out.contains("on steps"), "done drops the co-holder: {out}");
    // board_step owner= gives a step out; empty takes it back.
    has(
        &h.okj(
            &a,
            "board_step",
            json!({"task_id": id, "step_id": "s1", "owner": "beta"}),
        ),
        &["Woke: beta", "with beta"],
    );
    let out = h.okj(
        &a,
        "board_step",
        json!({"task_id": id, "step_id": "s1", "owner": ""}),
    );
    assert!(!out.contains("on steps"), "{out}");
}

/// #82: an unheld card an agent posts goes to one free agent, which takes
/// it with board_join; a busy agent is told no; skipping the delivery
/// passes it on.
#[test]
fn test_board_create_hands_an_unheld_card_to_one_free_agent() {
    let h = new_harness();
    let (a, b, c) = (call_a(), call_b(), call_c());
    // gamma works another repo; grokker is busy.
    h.b.roster.set_status(KEY_G, "working");
    let out = h.okj(&a, "board_create", json!({"title": "port the lexer"}));
    has(&out, &["Irori handed it to beta", "Woke: beta"]);
    let id = created_id(&out);
    let ns = h.all_notices(KEY_B);
    assert!(
        ns.len() == 1
            && ns[0].message.reason == coord::NOTICE_OFFER
            && ns[0].mode == WAKE
            && ns[0].message.text.contains("board_join"),
        "{ns:?}"
    );
    assert!(
        h.all_notices(KEY_A).is_empty(),
        "the poster isn't handed its own card"
    );
    assert!(h.all_notices(KEY_G).is_empty(), "a busy agent isn't woken");
    assert!(
        h.all_notices(KEY_C).is_empty(),
        "another repo's agent isn't woken"
    );
    let w = h.b.ib.next_wake(KEY_B, inbox::path::MOD).unwrap().unwrap();
    assert!(w.prompt.contains("offer=\"true\""), "{}", w.prompt);
    // beta takes it; gamma is told it is held.
    h.okj(&b, "board_join", json!({"task_id": id}));
    h.failsj(&c, "board_join", json!({"task_id": id}), &["held by"]);
    // A second card while beta works the first: it goes to grokker, now
    // idle, and beta is told no when it tries to take it.
    h.b.roster.set_status(KEY_G, "idle");
    let out = h.okj(&a, "board_create", json!({"title": "and the parser"}));
    has(&out, &["Irori handed it to grokker"]);
    let id2 = created_id(&out);
    h.failsj(
        &b,
        "board_join",
        json!({"task_id": id2}),
        &["one card at a time", "board_leave"],
    );
    // grokker skips the delivery: it gives the card back.
    let d = h.all_notices(KEY_G);
    assert_eq!(d.len(), 1, "{d:?}");
    let g = grok_caller();
    has(
        &h.okj(&g, "skip", json!({"delivery_id": d[0].id})),
        &["gave the card", "next free agent"],
    );
    let t = h.b.co.get_task(&id2).unwrap();
    let o = t.live_offer().unwrap();
    assert!(o.to.is_empty() && o.reason == coord::OFFER_PASSED, "{o:?}");
}

#[test]
fn test_board_status_completed_waits_on_the_persons_done() {
    DONE_CHECK.with(|c| c.set(true));
    let h = new_harness();
    let a = call_a();
    let id = h.new_card(&a, json!({"title": "lexer"}));
    h.okj(&a, "board_join", json!({"task_id": id}));
    // Card #140: with the done check on, completed needs proof.
    h.failsj(
        &a,
        "board_status",
        json!({"task_id": id, "state": "completed", "note": "tabs kept"}),
        &["completed needs proof", "what you saw in the real app"],
    );
    assert_eq!(h.b.co.get_task(&id).unwrap().state, coord::WORKING);
    let out = h.okj(
        &a,
        "board_status",
        json!({"task_id": id, "state": "completed", "proof": "tabs kept\nChecked: cargo test."}),
    );
    has(
        &out,
        &[
            "waits on the person to confirm it's done",
            "done check b1",
            "set completed again",
        ],
    );
    has_not(&out, &["folds into Done", "is completed now"]);
    let check = &h.b.co.get_task(&id).unwrap().open_blockers()[0];
    assert!(check.done_check && check.context == "tabs kept\nChecked: cargo test.");
    let board = h.okj(&a, "board", json!({}));
    has(&board, &["Is this done?"]);
}

/// Card #354: a card whose done-when leaves a choice to the person asks it
/// as a blocker; a done check there would settle nothing.
#[test]
fn test_board_status_completed_asks_a_choice_as_a_blocker() {
    DONE_CHECK.with(|c| c.set(true));
    let h = new_harness();
    let a = call_a();
    let id = h.new_card(
        &a,
        json!({"title": "findings", "done_when": "Findings listed; Stephen picks what becomes cards."}),
    );
    h.okj(&a, "board_join", json!({"task_id": id}));
    let done = json!({"task_id": id, "state": "completed", "proof": "Six findings on the card."});
    h.failsj(
        &a,
        "board_status",
        done.clone(),
        &[
            "has the person pick",
            "Stephen picks what becomes cards",
            "board_block",
            "board_edit",
        ],
    );
    let t = h.b.co.get_task(&id).unwrap();
    assert_eq!(t.state, coord::WORKING);
    assert!(t.blockers.is_empty(), "{:?}", t.blockers);
    // Asked as a blocker, answered and acted on: now it completes.
    h.okj(
        &a,
        "board_block",
        json!({"task_id": id, "question": "Which findings become cards?",
               "options": [{"answer": "All six", "recommended": true}, {"answer": "None"}]}),
    );
    h.okj(
        &a,
        "board_unblock",
        json!({"task_id": id, "blocker_id": "b1", "note": "made six cards"}),
    );
    let out = h.okj(&a, "board_status", done);
    has(&out, &["waits on the person to confirm it's done"]);

    // A feature the person uses is no choice left, and board_edit lifts it.
    for (when, ok) in [
        ("the person can pick a theme in settings", true),
        ("the app lets Stephen choose a voice", true),
        ("tests pass", true),
        ("Stephen decides which to keep", false),
    ] {
        let id = h.new_card(&a, json!({"title": "x", "done_when": when}));
        let b = call_b();
        h.okj(&b, "board_join", json!({"task_id": id}));
        let out = h.callj(
            &b,
            "board_status",
            json!({"task_id": id, "state": "completed", "proof": "p"}),
        );
        assert_eq!(!out.contains("has the person"), ok, "{when}: {out}");
        if !ok {
            h.okj(
                &b,
                "board_edit",
                json!({"task_id": id, "done_when": "kept the two Stephen chose"}),
            );
            let out = h.okj(
                &b,
                "board_status",
                json!({"task_id": id, "state": "completed", "proof": "p"}),
            );
            has(&out, &["waits on the person"]);
        }
        h.okj(&b, "board_leave", json!({"task_id": id}));
    }
}

/// Card #140: completed's screenshots ride on the done check beside the
/// proof; proof and attach belong to completed only.
#[test]
fn test_board_status_completed_attaches_proof_files() {
    DONE_CHECK.with(|c| c.set(true));
    let h = new_harness();
    let _ = h.b.attacher.set(Arc::new(NamedFiles));
    let a = call_a();
    let id = h.new_card(&a, json!({"title": "panel"}));
    h.okj(&a, "board_join", json!({"task_id": id}));
    h.failsj(
        &a,
        "board_status",
        json!({"task_id": id, "state": "working", "proof": "x"}),
        &["proof and attach go with completed"],
    );
    let out = h.okj(
        &a,
        "board_status",
        json!({"task_id": id, "state": "completed", "proof": "Fits now.",
               "attach": ["shots/narrow.png", "shots/wide.png"]}),
    );
    has(&out, &["done check b1 with narrow.png", "wide.png"]);
    // Any agent may change the card (card #320), and needs proof too.
    let other = h.new_card(&a, json!({"title": "not b's"}));
    let out = h.callj(
        &call_b(),
        "board_status",
        json!({"task_id": other, "state": "completed"}),
    );
    has(&out, &["needs proof"]);
    let check = &h.b.co.get_task(&id).unwrap().open_blockers()[0];
    let names: Vec<&str> = check.attachments.iter().map(|f| f.name.as_str()).collect();
    assert!(
        check.done_check && check.context == "Fits now.",
        "{check:?}"
    );
    assert_eq!(names, vec!["narrow.png", "wide.png"]);
}

/// Without the done check, completed still closes with a plain note.
#[test]
fn test_board_status_completed_without_done_check_needs_no_proof() {
    let h = new_harness();
    let a = call_a();
    let id = h.new_card(&a, json!({"title": "lexer"}));
    h.okj(&a, "board_join", json!({"task_id": id}));
    let out = h.okj(
        &a,
        "board_status",
        json!({"task_id": id, "state": "completed", "note": "done"}),
    );
    has(&out, &["is completed now"]);
}

/// Card #137: a card says where its work is and where its holder left
/// off, and whoever picks it up hears both.
#[test]
fn test_board_join_hands_over_where_and_left_off() {
    // alpha works in a git worktree on feat/x, one commit, not pushed.
    let repo = tempfile::tempdir().unwrap();
    let git = repo.path().join(".git");
    std::fs::create_dir_all(git.join("refs/heads/feat")).unwrap();
    std::fs::write(git.join("HEAD"), "ref: refs/heads/feat/x\n").unwrap();
    let sha = "abcdef0123456789abcdef0123456789abcdef01";
    std::fs::write(git.join("refs/heads/feat/x"), format!("{sha}\n")).unwrap();
    let root = repo.path().to_string_lossy().into_owned();

    let h = new_harness();
    let mut ca = card(
        KEY_A,
        "alpha",
        "remote:github.com/acme/app",
        inbox::BYPASS_MODE,
    );
    ca.cwd = root.clone();
    h.b.roster.put(ca);
    let (a, b) = (call_a(), call_b());
    let id = h.new_card(
        &a,
        json!({"title": "lexer", "steps": [{"text": "tokens"}, {"text": "errors"}]}),
    );
    h.okj(&a, "board_join", json!({"task_id": id}));

    // A tick without left_off asks for one.
    let out = h.okj(
        &a,
        "board_step",
        json!({"task_id": id, "step_id": "s1", "done": true}),
    );
    has(
        &out,
        &["Say where you left off", "left_off=", "push your branch"],
    );
    has(
        &out,
        &[&format!(
            "where: branch feat/x @ abcdef0 (not pushed) in {root}"
        )],
    );
    // With it, the card says it.
    let out = h.okj(
        &a,
        "board_step",
        json!({"task_id": id, "step_id": "s2", "doing": true, "left_off": "tokens done; next: error recovery"}),
    );
    has_not(&out, &["Say where you left off"]);
    has(&out, &["left off: \"tokens done; next: error recovery\""]);

    // alpha stops; beta picks it up and hears where it is.
    h.okj(&a, "board_leave", json!({"task_id": id}));
    let out = h.okj(&b, "board_join", json!({"task_id": id}));
    has(
        &out,
        &[
            "Picking up where it was:",
            &format!(
                "Where: branch feat/x @ abcdef0 (not pushed) in {root}; not pushed, so the last commits may be only on that machine"
            ),
            "Left off: tokens done; next: error recovery",
            "Work from that branch",
        ],
    );
}

/// Card #400: board_create with waits_on makes the card blocked from the
/// start, so Irori never hands it out until the work it waits on is done;
/// then it goes out once.
#[test]
fn test_board_create_waits_on_a_card() {
    let h = new_harness();
    let (a, b) = (call_a(), call_b());
    let co = h.b.co.clone();
    let agents = || -> Vec<coord::roster::AgentCard> {
        h.b.roster
            .cards()
            .into_iter()
            .map(|c| coord::roster::AgentCard {
                key: c.key,
                device_id: c.device_id,
                status: c.status,
                repo_key: c.repo_key,
                ..Default::default()
            })
            .collect()
    };
    h.b.roster.set_status(KEY_G, "working");
    let first = created_id(&h.okj(&a, "board_create", json!({"title": "the lexer"})));
    h.okj(&b, "board_join", json!({"task_id": first}));
    let r = h.short(&first);

    h.failsj(
        &a,
        "board_create",
        json!({"title": "x", "waits_on": "#999"}),
        &["no card"],
    );
    h.failsj(
        &a,
        "board_create",
        json!({"title": "x", "waits_on": "person"}),
        &["waits_on takes a card"],
    );
    let out = h.okj(
        &a,
        "board_create",
        json!({"title": "the parser", "waits_on": r}),
    );
    has(
        &out,
        &[
            "It starts blocked (Waits on",
            &r,
            "once that is done",
            "[blocked]",
        ],
    );
    has_not(&out, &["Irori handed it"]);
    let id = created_id(&out);
    let t = co.get_task(&id).unwrap();
    assert_eq!(t.state, coord::BLOCKED);
    assert!(t.offer.is_none(), "no task.offered row: {:?}", t.offer);
    let bl = t.open_blockers();
    assert!(
        bl.len() == 1
            && bl[0]
                .on
                .as_ref()
                .is_some_and(|o| o.kind == coord::BLOCKED_ON_TASK && o.reference == first),
        "{bl:?}"
    );
    // "task #N" works too.
    let out = h.okj(
        &a,
        "board_create",
        json!({"title": "the printer", "waits_on": format!("task {r}")}),
    );
    has(&out, &["It starts blocked"]);

    // Sweeps while the lexer is open hand out nothing.
    co.sweep_notices().unwrap();
    co.sweep_asks(&agents()).unwrap();
    assert!(co.get_task(&id).unwrap().offer.is_none());

    // The lexer is done: the parser unblocks and goes out once.
    h.okj(
        &b,
        "board_status",
        json!({"task_id": first, "state": "completed", "proof": "done"}),
    );
    co.sweep_notices().unwrap();
    let t = co.get_task(&id).unwrap();
    assert_ne!(t.state, coord::BLOCKED, "{t:?}");
    h.b.roster.set_status(KEY_G, "idle");
    let ns = co.sweep_asks(&agents()).unwrap();
    let offers: Vec<_> = ns
        .iter()
        .filter(|n| n.kind == coord::NOTICE_OFFER && n.task_id == id)
        .collect();
    assert_eq!(offers.len(), 1, "{ns:?}");
    let ns = co.sweep_asks(&agents()).unwrap();
    assert!(
        !ns.iter()
            .any(|n| n.kind == coord::NOTICE_OFFER && n.task_id == id),
        "handed out twice: {ns:?}"
    );
}

/// Card #407: an agent's face through the avatar tool. A bad face is
/// refused with why; the test backend keeps none, so setting one says so.
#[test]
fn the_avatar_tool_checks_a_face_before_keeping_it() {
    let h = new_harness();
    let bad = h.call(&call_a(), "avatar", r#"{"avatar":{"version":1,"id":"x","name":"X","body":{"path":"<svg/>"},"eyes":{"x":50,"y":50,"spacing":15,"scale":1}}}"#);
    assert!(
        bad.contains("isn't valid") && bad.contains("body.path"),
        "{bad}"
    );
    let none = h.call(&call_a(), "avatar", "{}");
    assert!(none.contains("no face of your own"), "{none}");
}

/// Card #632: Irori keeps no "main is red" of its own. Agents see a red
/// main by running the tests in the repo, so on = main-red is refused and
/// makes no card, and the board says nothing about it.
#[test]
fn test_board_block_main_red_is_gone() {
    let h = new_harness();
    let a = call_a();
    let x = h.new_card(&a, json!({"title": "Talk 3"}));
    h.okj(&a, "board_join", json!({"task_id": x}));

    let out = h.callj(
        &a,
        "board_block",
        json!({"task_id": x, "question": "attach_exe fails on clean main", "on": "main-red attach_exe 3573cf11"}),
    );
    has(
        &out,
        &["Error: ", "lease, task, person, agent, external or version"],
    );

    let board = h.okj(&a, "board", json!({}));
    has_not(&board, &["Main is red", "fails on clean main", "#2 "]);
}

/// Card #485: board_block's on = version names the least app version and
/// the devices (none: this one); a malformed version or an unknown device
/// is refused with what to give.
#[test]
fn test_board_block_on_a_version() {
    let h = new_harness();
    let a = call_a();
    let x = h.new_card(&a, json!({"title": "Live check of part 2"}));
    h.okj(&a, "board_join", json!({"task_id": x}));
    let out = h.callj(
        &a,
        "board_block",
        json!({"task_id": x, "question": "Update to v0.11.0", "on": "version soon"}),
    );
    has(&out, &["Error: ", "is not a version"]);
    let out = h.callj(
        &a,
        "board_block",
        json!({"task_id": x, "question": "Update the PC", "on": "version 0.11.0 the-pc"}),
    );
    has(&out, &["Error: ", "no device is called \"the-pc\""]);
    h.okj(
        &a,
        "board_block",
        json!({"task_id": x, "question": "Update to v0.11.0", "on": "version v0.11.0"}),
    );
    let board = h.okj(&a, "board", json!({}));
    has(&board, &["Update to v0.11.0"]);
}

/// Card #600: board_edit sets a card's repo, in the checkout's spelling,
/// and tells the agent holding it whether it can work it where it is.
#[test]
fn test_board_edit_sets_a_cards_repo() {
    let h = new_harness();
    let (a, c) = (call_a(), call_c());
    let co = h.b.co.clone();
    let id = h.new_card(&grok_caller(), json!({"title": "Sort the board by age"}));
    assert!(co.get_task(&id).unwrap().repo_key.is_empty());
    let r = h.short(&id);
    h.failsj(
        &c,
        "board_edit",
        json!({"task_id": id, "repo": "not a key"}),
        &["bad repoKey"],
    );
    // Gamma works in acme/other; the card is acme/app's: it hands it on.
    co.claim_task(KEY_C, &id).unwrap();
    let out = h.okj(
        &c,
        "board_edit",
        json!({"task_id": id, "repo": "remote:github.com/Acme/App"}),
    );
    has(
        &out,
        &[
            &format!("Edited {r}"),
            "You are in another repo",
            &format!("board_leave {r}"),
            "a checkout of app",
        ],
    );
    let t = co.get_task(&id).unwrap();
    assert_eq!(t.repo_key, "remote:github.com/acme/app");
    let log = format!("{:?}", co.activity(&id).unwrap());
    has(&log, &["set its repo to app"]);
    // The same repo again is no change, and no error.
    h.okj(
        &c,
        "board_edit",
        json!({"task_id": id, "repo": "remote:github.com/acme/app"}),
    );

    // Alpha works in acme/app: it works it here.
    let id2 = h.new_card(&grok_caller(), json!({"title": "Thumbnails"}));
    co.claim_task(KEY_A, &id2).unwrap();
    let out = h.okj(
        &a,
        "board_edit",
        json!({"task_id": id2, "repo": "remote:github.com/acme/app"}),
    );
    has(&out, &["You are in app: work it here, in a worktree."]);
    assert!(!out.contains("board_leave"), "{out}");
}

/// Card #600: a card made with no repo takes its parent's, else its
/// thread's, else the caller's; with none of those it has none.
#[test]
fn test_board_create_repo_defaults() {
    let h = new_harness();
    let (a, c) = (call_a(), call_c());
    let co = h.b.co.clone();
    let repo = |id: &str| co.get_task(id).unwrap().repo_key;
    // The caller's own.
    let mine = h.new_card(&c, json!({"title": "In other"}));
    assert_eq!(repo(&mine), "remote:github.com/acme/other");
    // A caller with no repo: none, and its agent works it out.
    let none = h.new_card(&grok_caller(), json!({"title": "Nowhere"}));
    assert_eq!(repo(&none), "");
    // A subtask: its parent's, not the caller's.
    let sub = h.new_card(&a, json!({"title": "A part", "parent": h.short(&mine)}));
    assert_eq!(repo(&sub), "remote:github.com/acme/other");
    // From a thread that names a repo: the thread's.
    let (th, _) =
        h.b.co
            .post_ask(
                "user:dev1",
                coord::NewAsk {
                    text: "the parser drops tabs".into(),
                    repo_key: "remote:github.com/acme/parser".into(),
                    thread: true,
                    ..Default::default()
                },
                &[],
            )
            .unwrap();
    let tr = h.short(&th.id);
    h.okj(
        &a,
        "board_comment",
        json!({"task_id": tr, "text": "I'll make a card"}),
    );
    let made = h.new_card(&a, json!({"title": "Keep tabs", "from": tr}));
    assert_eq!(repo(&made), "remote:github.com/acme/parser");
    // A repo given wins.
    let given = h.new_card(
        &a,
        json!({"title": "Given", "from": tr, "repo": "remote:github.com/acme/app"}),
    );
    assert_eq!(repo(&given), "remote:github.com/acme/app");
}
