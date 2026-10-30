//! The capture and explore tools with capture on (I:internal/tools
//! capture_test.go and explore_test.go): timeline, why, search, show,
//! overview, commits and prs texts, their paging and caps, and which
//! messages each caller may read. Everything is synthetic: capture rows
//! are seeded into the log before capture replays it, and the repo is a
//! fresh temp repo.

mod common;

use common::*;
use tools::{Caller, MAX_RESULT};

fn none() -> Caller {
    Caller::default()
}

fn pr_entry(
    n: i64,
    url: &str,
    session: &str,
    role: ledger::Role,
    source: &str,
    at: gocompat::time::GoTime,
) -> ledger::Entry {
    ledger::Entry {
        key: format!("pr:acme/app#{n}"),
        repo: "acme/app".into(),
        number: n,
        url: url.into(),
        session: session.into(),
        role,
        source: source.into(),
        at,
    }
}

#[test]
fn test_capture_off() {
    let h = new_harness();
    h.fails(&call_a(), "timeline", "{}", &["capture is off"]);
    h.failsj(
        &call_a(),
        "why",
        json!({"path": "foo.go"}),
        &["capture is off"],
    );
}

#[test]
fn test_timeline() {
    let r = with_capture(|r| {
        r.append(
            capture::KIND_PR,
            &pr_entry(
                7,
                "https://github.com/acme/app/pull/7",
                KEY_A,
                ledger::Role::AUTHOR,
                "pr-link",
                plus(r.clk.now(), -90 * 60),
            ),
        );
    });
    let (a, b, c) = (call_a(), call_b(), call_c());
    r.send_to(&b, "alpha", "hello alpha from beta");
    r.clk.advance(std::time::Duration::from_secs(60));
    r.send_to(&a, "beta", "thanks beta");

    let out = r.ok(&a, "timeline", "{}");
    has(
        &out,
        &[
            &format!("Timeline of alpha ({KEY_A}), you,"),
            "untrusted data",
            "turn p1 (origin human), tools: Edit 2, Bash 1",
            r#"prompt: "add foo please""#,
            r#"reply: "added foo.go""#,
            "edited: ",
            "foo.go",
            &format!(
                r#"commit {} "add foo" on main, committed by alpha"#,
                &r.sha[..7]
            ),
            "files: foo.go [exact]",
            "PR acme/app#7 author (pr-link) https://github.com/acme/app/pull/7",
            &format!("message from beta ({KEY_B})"),
            r#"text: "hello alpha from beta""#,
            &format!("message sent to beta ({KEY_B})"),
            r#"text: "thanks beta""#,
        ],
    );
    has_not(&out, &["(more:"]);
    // Newest first: the reply message, then the first, then the PR, commit, turn.
    let commit = format!("commit {}", &r.sha[..7]);
    let mut last = 0;
    for w in [
        "thanks beta",
        "hello alpha",
        "PR acme/app#7",
        &commit,
        "turn p1",
    ] {
        let i = out.find(w).unwrap();
        assert!(i >= last, "{w:?} out of order:\n{out}");
        last = i;
    }

    // Another agent's timeline: by name, key and key prefix; message text
    // only for the messages' parties.
    for who in ["alpha", KEY_A, "aaaaaaaa-1111", "dev1/aaaaaaaa"] {
        let out = r.okj(&c, "timeline", json!({"agent": who}));
        has(
            &out,
            &[
                &format!("Timeline of alpha ({KEY_A})"),
                "turn p1",
                "[left out: only its sender and recipient see it]",
            ],
        );
        has_not(&out, &["hello alpha", "thanks beta", "), you,"]);
    }
    has(
        &r.okj(&b, "timeline", json!({"agent": "alpha"})),
        &["hello alpha from beta", "thanks beta"],
    );
    // beta has no turns but its messages show.
    has(
        &r.ok(&b, "timeline", "{}"),
        &["Timeline of beta", "message sent to alpha", "hello alpha"],
    );
}

#[test]
fn test_timeline_held_text() {
    let r = with_capture(|_| {});
    let secret = "grok held secret";
    let out = r.okj(
        &grok_caller(),
        "send",
        json!({"to": "alpha", "text": secret}),
    );
    has(&out, &["held"]);
    for c in [call_a(), call_b()] {
        has_not(
            &r.okj(&c, "timeline", json!({"agent": "alpha"})),
            &[secret, "grokker"],
        );
    }
    // Its sender sees its own message.
    has(
        &r.ok(&grok_caller(), "timeline", "{}"),
        &["message sent to alpha", secret],
    );
}

#[test]
fn test_timeline_paging() {
    let r = with_capture(|r| {
        let base = plus(r.clk.now(), -3600);
        for i in 2..=4 {
            r.turn(
                KEY_A,
                &format!("p{i}"),
                &format!("prompt {i}"),
                "",
                plus(base, i * 60),
                &[],
            );
        }
    });
    let a = call_a();
    let out = r.okj(&a, "timeline", json!({"limit": 2}));
    has(&out, &["turn p4", "turn p3", "(more: before="]);
    has_not(&out, &["turn p2", "turn p1"]);
    let before = more_before(&out);
    let out = r.okj(&a, "timeline", json!({"limit": 2, "before": before}));
    has(
        &out,
        &["turn p2", "turn p1", &format!("commit {}", &r.sha[..7])],
    );
    has_not(&out, &["turn p3", "turn p4", "(more:"]);
}

#[test]
fn test_timeline_capped() {
    let r = with_capture(|r| {
        let base = plus(r.clk.now(), -3600);
        let long = "a long prompt line ".repeat(40);
        for i in 0..50 {
            r.turn(KEY_A, &format!("q{i:02}"), &long, &long, plus(base, i), &[]);
        }
    });
    let a = call_a();
    let mut out = r.okj(&a, "timeline", json!({"limit": 50}));
    assert!(out.len() <= MAX_RESULT, "{} bytes", out.len());
    has(&out, &["turn q49", "(more: before="]);
    // Paging by the hints reaches every turn once, each page under the cap.
    let mut seen: std::collections::BTreeMap<String, usize> = Default::default();
    let mut page = 0;
    loop {
        assert!(page <= 20, "paging doesn't end");
        page += 1;
        has_not(&out, &["[truncated"]);
        for l in out.lines() {
            if let Some((_, rest)) = l.split_once(" turn ") {
                let id = rest.split(' ').next().unwrap();
                *seen.entry(id.to_string()).or_default() += 1;
            }
        }
        if !out.contains("(more: before=") {
            break;
        }
        out = r.okj(
            &a,
            "timeline",
            json!({"limit": 50, "before": more_before(&out)}),
        );
    }
    assert_eq!(seen.len(), 51, "saw {} turns: {seen:?}", seen.len());
    for (id, n) in &seen {
        assert_eq!(*n, 1, "turn {id} shown {n} times");
    }
}

#[test]
fn test_timeline_errors() {
    let r = with_capture(|_| {});
    let a = call_a();
    r.b.roster.put(card(
        "dev1/aaaaaaaa-9999-4999-8999-999999999999",
        "alpha2",
        "",
        "",
    ));
    r.failsj(
        &a,
        "timeline",
        json!({"agent": "nobody-here"}),
        &["no agent called", "agents tool"],
    );
    r.failsj(
        &a,
        "timeline",
        json!({"agent": "aaaaaaaa"}),
        &["matches 2 agents", "use a key"],
    );
    r.failsj(
        &a,
        "timeline",
        json!({"before": "yesterday"}),
        &["RFC 3339"],
    );
    r.failsj(
        &a,
        "timeline",
        json!({"limit": "ten"}),
        &["limit should be a whole number"],
    );
    r.failsj(
        &a,
        "timeline",
        json!({"who": "alpha"}),
        &["unknown argument"],
    );
    r.fails(&none(), "timeline", "{}", &["join first"]);
    // A gone agent's key still works.
    has(
        &r.okj(&a, "timeline", json!({"agent": "dev1/zzzzzzzz-gone"})),
        &["nothing recorded yet"],
    );
}

#[test]
fn test_why() {
    let r = with_capture(|_| {});
    let (a, b, g) = (call_a(), call_b(), grok_caller());
    let sha7 = &r.sha[..7];
    // A committed line: blame -> commit -> session and turn.
    let out = r.okj(&a, "why", json!({"path": "foo.go", "line": 3}));
    has(
        &out,
        &[
            &format!("Why foo.go:3 in {}", r.repo_key),
            &format!(r#"Last changed by commit {sha7} "add foo", by Test"#),
            &format!("Sessions behind it: alpha ({KEY_A})"),
            &format!("Committed by alpha ({KEY_A}) in turn p1"),
            "Turns:",
            r#"prompt: "add foo please""#,
            "Edits, newest first:",
            "Write +5/-0 (exact) by alpha",
        ],
    );
    has_not(&out, &["ecision"]);
    // Absolute path, from Grok with no cwd.
    r.grok_no_cwd();
    let foo = join(&r.dir, "foo.go");
    has(
        &r.okj(&g, "why", json!({"path": foo})),
        &["Why foo.go in", "No line given"],
    );
    r.failsj(
        &g,
        "why",
        json!({"path": "foo.go"}),
        &["no working directory for you", "repo"],
    );
    has(
        &r.okj(&g, "why", json!({"path": "foo.go", "repo": r.repo_key})),
        &[&format!("Why foo.go in {}", r.repo_key)],
    );

    // An uncommitted line: the edits since the last commit.
    write_file(&foo, &format!("{FOO_GO}\nfunc C() {{}}\n"));
    let out = r.okj(&a, "why", json!({"path": "foo.go", "line": 7}));
    has(&out, &["isn't committed yet"]);

    // Decision records left the agent tools (decision 27): one already in
    // the log, anchored right there, is not shown.
    r.b.cp
        .get()
        .unwrap()
        .record_decision(&capture::DecisionInput {
            statement: "keep A tiny".into(),
            path: "foo.go".into(),
            lines: "3".into(),
            cwd: r.dir.clone(),
            ..Default::default()
        })
        .unwrap();
    let out = r.okj(&b, "why", json!({"path": foo, "line": 3}));
    has(&out, &[&format!("Last changed by commit {sha7}")]);
    has_not(&out, &["ecision", "keep A tiny"]);

    r.fails(&a, "why", "{}", &["give path"]);
    r.failsj(
        &a,
        "why",
        json!({"path": "foo.go", "line": -1}),
        &["1-based"],
    );
    let outside = tempfile::tempdir().unwrap();
    let x = join(&canon(outside.path()), "x.txt");
    write_file(&x, "x\n");
    r.failsj(
        &a,
        "why",
        json!({"path": x}),
        &["not inside a known git repo", "absolute path"],
    );
}

/// why stays under the result cap, without the generic clip, when the
/// turns and edits behind a line are big enough to fill it.
#[test]
fn test_why_caps_big_turns() {
    let big = "漢".repeat(300); // 3 bytes a rune, past the excerpt cap
    let r = with_capture(|r| {
        let t0 = plus(r.clk.now(), -3600);
        let mut edits = vec![r.edit_id.clone()];
        let mut turns = vec!["p1".to_string()];
        for i in 2..=10 {
            let (id, eid) = (format!("p{i}"), format!("e-foo{i}"));
            let at = plus(t0, i * 60);
            r.turn(KEY_A, &id, &big, &big, at, &["foo.go"]);
            r.append(
                capture::KIND_EDIT,
                &capture::EditRow {
                    id: eid.clone(),
                    key: KEY_A.into(),
                    turn_id: id.clone(),
                    tool_use_id: format!("w{id}"),
                    tool: "Edit".into(),
                    repo_key: r.repo_key.clone(),
                    root: r.dir.clone(),
                    path: "foo.go".into(),
                    adds: 1,
                    fidelity: capture::transcript::EXACT.into(),
                    at: plus(at, 1),
                    ..Default::default()
                },
            );
            edits.push(eid);
            turns.push(id);
        }
        let turns: Vec<&str> = turns.iter().map(|s| s.as_str()).collect();
        r.link(&edits, &turns, plus(t0, 11 * 60));
    });
    let out = r.okj(&call_a(), "why", json!({"path": "foo.go", "line": 3}));
    has(&out, &["Turns:", "not shown"]);
    has_not(&out, &["[truncated:"]);
    assert!(
        out.len() <= MAX_RESULT,
        "why is {} bytes, over {MAX_RESULT}",
        out.len()
    );
}

// ---- explore_test.go ----

/// search and show read as the caller: a message is found and shown only
/// by its sender and recipient, never by a third agent, whatever it passes.
#[test]
fn test_search_show_keep_messages_with_their_parties() {
    let r = with_capture(|_| {});
    let secret = "pangolin migration plan";
    let (msg_id, did) = r.send_to(&call_a(), "beta", secret);
    let d = r.b.ib.get(&did).unwrap();
    assert!(
        d.state != inbox::st::HELD && d.state != inbox::st::DISMISSED,
        "delivery {did} is {:?}: the test needs a searchable message",
        d.state
    );
    let rf = format!("msg:{msg_id}");
    for (name, caller, party) in [
        ("sender", call_a(), true),
        ("recipient", call_b(), true),
        ("other agent", call_c(), false),
        ("grok member", grok_caller(), false),
    ] {
        let found = r.okj(&caller, "search", json!({"query": "pangolin"}));
        let shown = r.callj(&caller, "show", json!({"ref": rf}));
        if party {
            has(&found, &[&rf]);
            has(&shown, &[&rf, secret]);
            continue;
        }
        assert!(
            !found.contains(&rf) && !found.contains(secret),
            "{name}: {found}"
        );
        has_not(&shown, &[secret]);
        has(&shown, &["Error: ", "nothing you may read"]);
        // Kinds filtered to messages: still nothing.
        has_not(
            &r.okj(
                &caller,
                "search",
                json!({"query": "pangolin", "kinds": ["msg"]}),
            ),
            &[&rf],
        );
    }
    // A message about a card is on its thread, which anyone reading the
    // card sees (the sender opted in with task_id); the message itself
    // stays with its parties.
    let id = r.new_card(&call_a(), json!({"title": "burrow survey"}));
    let out = r.okj(
        &call_a(),
        "send",
        json!({"to": "beta", "text": "aardvark tunnel notes", "task_id": id}),
    );
    let ref2 = format!("msg:{}", field(&out, "message"));
    has(
        &r.okj(&call_c(), "show", json!({"ref": format!("task:{id}")})),
        &[&format!("{KEY_A} -> {KEY_B}: aardvark tunnel notes")],
    );
    let shown = r.callj(&call_c(), "show", json!({"ref": ref2}));
    has(&shown, &["Error: ", "nothing you may read"]);
    has_not(&shown, &["aardvark"]);
    has(
        &r.okj(&call_b(), "show", json!({"ref": ref2})),
        &["aardvark tunnel notes"],
    );

    // The viewer is never an argument.
    r.failsj(
        &call_c(),
        "search",
        json!({"query": "pangolin", "viewer": KEY_A}),
        &[r#"unknown argument "viewer""#],
    );
    r.failsj(
        &call_c(),
        "show",
        json!({"ref": rf, "viewer": ""}),
        &[r#"unknown argument "viewer""#],
    );
}

#[test]
fn test_search_paging_and_errors() {
    let r = with_capture(|_| {});
    let b = call_b();
    for i in 0..12 {
        r.new_card(&call_a(), json!({"title": format!("zebra crossing {i}")}));
    }
    let out = r.okj(&b, "search", json!({"query": "zebra", "limit": 5}));
    has(
        &out,
        &[
            "5 hits for \"zebra\"",
            "task:",
            "(more: offset=5;",
            "show <ref>",
        ],
    );
    let out = r.okj(
        &b,
        "search",
        json!({"query": "zebra", "limit": 5, "offset": 10}),
    );
    has(&out, &["2 hits", "(from offset 10)"]);
    has_not(&out, &["(more:"]);
    has(
        &r.okj(&b, "search", json!({"query": "zebra", "offset": 20})),
        &["No more hits"],
    );
    has(
        &r.okj(&b, "search", json!({"query": "okapi"})),
        &["No hits for \"okapi\""],
    );
    has(
        &r.okj(&b, "search", json!({"query": "zebra 3", "kinds": ["task"]})),
        &["task:"],
    );
    r.failsj(&b, "search", json!({"kinds": ["agent"]}), &["kinds are"]);
    r.failsj(&b, "search", json!({"kinds": ["bogus"]}), &["kinds are"]);
    r.failsj(&b, "search", json!({"agent": "nobody"}), &["agents tool"]);
    r.failsj(&b, "search", json!({"offset": -1}), &["offset"]);
    // An empty query lists the newest.
    has(&r.ok(&b, "search", "{}"), &["newest items"]);
}

#[test]
fn test_show() {
    let r = with_capture(|_| {});
    let c = call_c();
    let id = r.new_card(
        &call_a(),
        json!({"title": "wombat burrow", "objective": "dig it"}),
    );
    r.okj(
        &call_b(),
        "board_comment",
        json!({"task_id": id, "text": "mind the roots"}),
    );
    let cases: Vec<(String, Vec<String>)> = vec![
        (
            format!("task:{id}"),
            vec![
                format!("task:{id}"),
                "wombat burrow".into(),
                "dig it".into(),
                "mind the roots".into(),
            ],
        ),
        (
            format!("commit:{}", r.sha),
            vec!["commit:".into(), "add foo".into()],
        ),
        (
            format!("edit:{}", r.edit_id),
            vec![format!("edit:{}", r.edit_id), "foo.go".into()],
        ),
        (format!("agent:{KEY_A}"), vec![format!("agent:{KEY_A}")]),
    ];
    for (rf, want) in &cases {
        let want: Vec<&str> = want.iter().map(|s| s.as_str()).collect();
        has(&r.okj(&c, "show", json!({"ref": rf})), &want);
    }
    r.fails(&c, "show", "{}", &["give ref"]);
    r.failsj(&c, "show", json!({"ref": "nonsense"}), &["Copy a ref"]);
    r.failsj(
        &c,
        "show",
        json!({"ref": "task:nope"}),
        &["nothing you may read"],
    );
    r.failsj(
        &c,
        "show",
        json!({"ref": format!("task:{id}"), "offset": -2}),
        &["offset"],
    );
}

#[test]
fn test_show_pages() {
    let r = with_capture(|_| {});
    let long = "aardvark ".repeat(600); // 5400 bytes, past one page
    let (msg_id, _) = r.send_to(&call_a(), "beta", &long);
    let rf = format!("msg:{msg_id}");
    let out = r.okj(&call_b(), "show", json!({"ref": rf}));
    has(&out, &["(more: offset="]);
    let next = out.split_once("(more: offset=").unwrap().1;
    let next = next.split(';').next().unwrap();
    let off: i64 = next
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("next offset {next:?}"));
    assert!(off > 0, "next offset {next:?}");
    let out = r.okj(&call_b(), "show", json!({"ref": rf, "offset": off}));
    has(&out, &[&format!("(from offset {off})"), "aardvark"]);
    has_not(&out, &["(more:"]);
}

#[test]
fn test_overview() {
    let r = with_capture(|_| {});
    let (a, b) = (call_a(), call_b());
    r.send_to(&b, "alpha", "please look at the parser");
    let blocked = r.new_card(&b, json!({"title": "flaky ci", "repo": r.repo_key}));
    r.okj(&b, "board_join", json!({"task_id": blocked}));
    r.okj(
        &b,
        "board_block",
        json!({"task_id": blocked, "reason": "runner is down", "on": "external CI"}),
    );
    let mine = r.new_card(&a, json!({"title": "parser rewrite"}));
    r.okj(&a, "board_join", json!({"task_id": mine}));
    let out = r.ok(&a, "overview", "{}");
    has(
        &out,
        &[
            "Overview for alpha",
            &format!("repo {}", r.repo_key),
            "Agents live",
            "alpha (you)",
            "Inbox: 1 open",
            "from beta",
            "Blocked:",
            &format!("task:{blocked}"),
            "runner is down",
            "On the Board:",
            &format!("task:{mine}"),
            "Recent commits:",
            &format!("commit:{}", r.sha),
            "show <ref> for any line",
        ],
    );
    r.failsj(&a, "overview", json!({"x": 1}), &["unknown argument"]);

    // A card from another repo, or from a Grok member (no repo), that is
    // assigned to alpha needs it, so its overview shows it.
    let other = r.new_card(
        &call_c(),
        json!({"title": "other repo ask", "assign_to": "alpha"}),
    );
    let grok = r.new_card(
        &grok_caller(),
        json!({"title": "grok ask", "assign_to": "alpha"}),
    );
    let (to, tg) = (format!("task:{other}"), format!("task:{grok}"));
    has(&r.ok(&a, "overview", "{}"), &[&to, &tg]);
    has_not(&r.ok(&b, "overview", "{}"), &[&to, &tg]);
}

#[test]
fn test_agents_doing() {
    let r = with_capture(|_| {});
    let id = r.new_card(&call_a(), json!({"title": "tidy the burrow"}));
    r.okj(&call_b(), "board_join", json!({"task_id": id}));
    let out = r.ok(&call_c(), "agents", "{}");
    has(
        &out,
        &[&format!(
            r#"doing: "on the Board: #1 tidy the burrow" (task:{id})"#
        )],
    );
    // alpha holds nothing: its last reply.
    has(&out, &[r#"doing: "added foo.go" (turn:"#]);
    // With capture off the list still works, without doing lines.
    let h = new_harness();
    has_not(&h.ok(&call_c(), "agents", "{}"), &["doing:"]);
}

#[test]
fn test_commits() {
    let r = with_capture(|_| {});
    let a = call_a();
    let commit = format!("commit:{}", r.sha);
    let in_repo = format!("1 commits in {}", r.repo_key);
    let cases: Vec<(&str, J, Vec<&str>)> = vec![
        (
            "own repo",
            json!({}),
            vec![
                &in_repo,
                &commit,
                r#""add foo""#,
                "on main",
                "committed by alpha",
            ],
        ),
        (
            "path",
            json!({"path": "./foo.go"}),
            vec!["touching foo.go", &commit],
        ),
        (
            "other path",
            json!({"path": "bar"}),
            vec!["No commits seen"],
        ),
        (
            "other repo",
            json!({"repo": "remote:github.com/x/y"}),
            vec!["No commits seen in remote:github.com/x/y"],
        ),
    ];
    for (name, args, want) in cases {
        let out = r.okj(&a, "commits", args);
        for w in want {
            assert!(
                out.contains(w),
                "{name}: output should contain {w:?}:\n{out}"
            );
        }
    }
    r.grok_no_cwd();
    r.fails(&grok_caller(), "commits", "{}", &["give repo"]);
    has(
        &new_harness().call(&a, "commits", "{}"),
        &["capture is off"],
    );
}

#[test]
fn test_prs() {
    let r = with_capture(|r| {
        let at = r.clk.now();
        r.append(
            capture::KIND_PR,
            &pr_entry(
                57,
                "https://github.com/acme/app/pull/57",
                KEY_B,
                ledger::Role::AUTHOR,
                "gh",
                at,
            ),
        );
        r.append(
            capture::KIND_PR,
            &pr_entry(57, "", KEY_A, ledger::Role::REVIEWER, "gh", at),
        );
    });
    let b = call_b();
    for (name, repo, want) in [
        (
            "repoKey",
            "remote:github.com/acme/app",
            vec![
                "1 pull requests in acme/app",
                "pr:acme/app#57",
                "author beta",
                "reviewer alpha",
                "pull/57",
            ],
        ),
        ("owner/name", "acme/app", vec!["pr:acme/app#57"]),
        ("none", "acme/other", vec!["No pull requests"]),
        ("beta's own repo", "", vec!["pr:acme/app#57"]),
    ] {
        let args = if repo.is_empty() {
            json!({})
        } else {
            json!({"repo": repo})
        };
        let out = r.okj(&b, "prs", args);
        for w in want {
            assert!(
                out.contains(w),
                "{name}: output should contain {w:?}:\n{out}"
            );
        }
    }
    r.failsj(&b, "prs", json!({"repo": "root:abc"}), &["GitHub repo"]);
}

/// A wake puts the fenced message into the recipient's turn prompt. That
/// turn is still searchable and showable, but the message body in it stays
/// with its sender and recipients.
#[test]
fn test_search_show_hide_messages_in_wake_prompts() {
    let secret = "pangolin secret plan";
    let prompt = format!(
        "[irori-wake:w1] Irori delivered 1 message from other agents. They came through Irori.\n\n<irori-message from=\"alpha\" key=\"{KEY_A}\" delivery=\"dlv_1\" trust=\"untrusted\">\n{secret}\n</irori-message>\n"
    );
    let r = with_capture(|r| {
        r.turn(KEY_B, "pw", &prompt, "on it", plus(r.clk.now(), -3600), &[]);
    });
    let rf = format!("turn:t-{KEY_B}pw");
    for (name, caller, sees) in [
        ("recipient", call_b(), true),
        ("sender", call_a(), true),
        ("other agent", call_c(), false),
        ("grok member", grok_caller(), false),
    ] {
        let found = r.okj(&caller, "search", json!({"query": "pangolin"}));
        assert!(
            !found.contains(&rf) && !found.contains("secret plan"),
            "{name}: {found}"
        );
        let shown = r.okj(&caller, "show", json!({"ref": rf}));
        has(&shown, &[&rf, "on it"]);
        if sees {
            assert!(shown.contains(secret), "{name}: {shown}");
        } else {
            assert!(
                !shown.contains(secret) && shown.contains("hidden"),
                "{name}: {shown}"
            );
        }
        has_not(
            &r.okj(&caller, "show", json!({"ref": format!("agent:{KEY_B}")})),
            &["secret plan"],
        );
    }
}
