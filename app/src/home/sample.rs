//! Sample needs for the headless scenes `home-needs` and `wind-down`: the
//! scratch data rarely has one of each row, so these scenes put one of
//! every kind on Home (the React preview's sample cards did the same).
//! Only the scenes call this; the live window never shows it.

use gocompat::time::GoTime;
use inbox::{Delivery, Sender, hold, st};
use irori_window::agents::AgentsSettings;
use irori_window::agents_trust::TrustNeed;
use irori_window::cards::{
    AgentRef, CardBlocker, CardDetail, CardOption, CardRef, CardSummary, GoneWait, WaitPr,
};
use irori_window::wind_down::{WindDownRow, WindDownStatus};
use rune::TextEdit;

use super::pure::answer_key;
use crate::logic::agent_status::now_ms;
use crate::ui::{Dialog, Route, State};

fn ago(min: i64) -> GoTime {
    GoTime::unix_milli(now_ms() - min * 60_000)
}

fn agent(name: &str) -> AgentRef {
    AgentRef {
        key: format!("sample/{name}"),
        name: name.into(),
        status: "idle".into(),
        ..AgentRef::default()
    }
}

fn card(id: &str, short: &str, title: &str, min: i64, f: impl FnOnce(&mut CardSummary)) -> CardSummary {
    let mut c = CardSummary {
        id: id.into(),
        short: short.into(),
        title: title.into(),
        state: "working".into(),
        scope: "mine".into(),
        scope_label: "My devices".into(),
        updated_at: ago(min),
        ..CardSummary::default()
    };
    f(&mut c);
    c
}

fn opt(answer: &str, why: &str, recommended: bool) -> CardOption {
    CardOption {
        answer: answer.into(),
        why: why.into(),
        recommended,
        ..CardOption::default()
    }
}

fn cref(id: &str, short: &str, title: &str) -> CardRef {
    CardRef {
        id: id.into(),
        short: short.into(),
        tag: String::new(),
        title: title.into(),
    }
}

fn held(id: &str, f: impl FnOnce(&mut Delivery)) -> Delivery {
    let mut d = Delivery {
        id: id.into(),
        state: st::HELD.into(),
        created_at: ago(30),
        ..Delivery::default()
    };
    d.message.from = Sender::new("sample/otter", "otter", "claude");
    f(&mut d);
    d
}

/// Puts one of every Needs you row on Home.
pub fn needs(s: &mut State) {
    let oak = agent("oak");
    let decision = card("s-dec", "901", "Pick the sync format for shared boards", 4, |c| {
        c.open_blockers = 2;
        c.state = "input_required".into();
        c.blocker_question = "JSON lines or one CBOR blob per board?".into();
        c.working = vec![oak.clone()];
    });
    let check = card("s-check", "902", "Speed up the Board's first paint", 12, |c| {
        c.ready = true;
        c.open_blockers = 1;
    });
    let merge = card("s-merge", "903", "Fix the stale presence dot after sleep", 40, |c| {
        c.state = "in_review".into();
        c.merge_pr = Some(WaitPr {
            repo: "irori".into(),
            number: 152,
            url: "https://github.com/acme/irori/pull/152".into(),
            ..WaitPr::default()
        });
        c.wait_question = "CI is green; can you merge it?".into();
    });
    let gone = card("s-gone", "904", "Rotate the relay's TLS certificate", 90, |c| {
        c.state = "blocked".into();
        c.gone_wait = Some(GoneWait {
            agent: AgentRef {
                status: "gone".into(),
                ..agent("kestrel")
            },
            question: "Which DNS provider holds the zone?".into(),
            ..GoneWait::default()
        });
    });
    let trust = card("s-trust", "905", "Add the Android build to CI", 120, |c| {
        c.state = "submitted".into();
        c.cant_start = "not trusted: /Users/me/code/irori-android on this Mac".into();
    });
    let no_agent = card("s-noagent", "906", "Port the tray to Linux", 200, |c| {
        c.state = "submitted".into();
        c.no_agent = "No computer can start its agent: its repo is checked out nowhere you have Irori.".into();
    });
    let looped_a = card("s-loop-a", "907", "Ship the pairing rework", 300, |c| {
        c.state = "blocked".into();
        c.wait_loop = vec![
            cref("s-loop-a", "907", "Ship the pairing rework"),
            cref("s-loop-b", "908", "Invite flow copy"),
        ];
    });
    let looped_b = card("s-loop-b", "908", "Invite flow copy", 310, |c| {
        c.state = "blocked".into();
        c.wait_loop = looped_a.wait_loop.clone();
    });
    let detail = |c: &CardSummary, blockers: Vec<CardBlocker>| CardDetail {
        summary: c.clone(),
        blockers,
        ..CardDetail::default()
    };
    let q1 = CardBlocker {
        id: "b1".into(),
        question: "JSON lines or one CBOR blob per board?".into(),
        context: "JSON lines diff well in git; CBOR is a third the size on the relay.".into(),
        raised_by: oak.clone(),
        options: vec![
            opt("JSON lines", "Readable, and the export already writes it", false),
            opt("CBOR per board", "Smaller on the wire", true),
        ],
        state: "open".into(),
        ..CardBlocker::default()
    };
    let q2 = CardBlocker {
        id: "b2".into(),
        question: "Keep the old sync path for one release?".into(),
        raised_by: oak.clone(),
        state: "replied".into(),
        missing: vec!["how long older devices stay on 0.12".into()],
        ..CardBlocker::default()
    };
    let done = CardBlocker {
        id: "b3".into(),
        question: "Is it done?".into(),
        raised_by: agent("heron"),
        done_check: true,
        options: vec![opt("Done", "", true), opt("Not yet", "", false)],
        context: "First paint is 140 ms on the 900-card board, from 610 ms.".into(),
        state: "open".into(),
        ..CardBlocker::default()
    };
    s.home.details.insert(decision.id.clone(), detail(&decision, vec![q1, q2.clone()]));
    s.home.details.insert(check.id.clone(), detail(&check, vec![done]));
    s.home.trust.insert(
        trust.id.clone(),
        Some(TrustNeed {
            task_id: trust.id.clone(),
            folder: "/Users/me/code/irori-android".into(),
            device: "this computer".into(),
            here: true,
            what: "Claude Code will run agents in /Users/me/code/irori-android on this computer without asking you first. It's the trust Claude Code asks for the first time you open a folder.".into(),
            asked_by: Some("Lead-PC".into()),
        }),
    );
    // The replied item's box is open, with words typed.
    let mut words = TextEdit::new("Two releases: the PC stays on 0.12 until");
    words.focused = true;
    s.home.own.insert(answer_key(&decision.id, &q2.id), words);
    let perm = card("s-perm", "909", "Clean old worktrees off the Mac", 2, |c| {
        c.open_blockers = 1;
        c.state = "input_required".into();
        c.blocker_question = "Waiting for your permission: rm -rf .claude/worktrees/card-600-1".into();
        c.working = vec![AgentRef {
            spawned: true,
            assigned: false,
            status: "busy".into(),
            device: "Lead-PC".into(),
            where_: "Lead-PC".into(),
            ..agent("wren")
        }];
    });
    let mut cards = vec![perm, decision, check, merge, gone, trust, no_agent, looped_a, looped_b];
    cards.extend(s.data.cards.drain(..));
    s.data.cards = cards;
    s.home.held = vec![
        held("h1", |d| {
            d.hold = hold::UNOWNED.into();
            d.message.from = Sender::new("user:me", "Stephen", "user");
            d.repo_key = "github.com/acme/site".into();
            d.message.text = "Can someone bump the docs site to the new theme?".into();
        }),
        held("h2", |d| {
            d.hold = hold::NOT_ALLOWED.into();
            d.to = "user:me".into();
            d.message.from = Sender::new("person:ana", "Ana", "user");
            d.message.text = "Could your agent look at the flaky invite test on our board?".into();
            d.created_at = ago(55);
        }),
        held("h3", |d| {
            d.hold = hold::HOP_LIMIT.into();
            d.to = "sample/heron".into();
            d.message.hop = 8;
        }),
        held("h4", |d| {
            d.hold = hold::PAUSED.into();
            d.to = "sample/heron".into();
        }),
    ];
    s.home.held_loaded = true;
    s.home.spawn = Some(AgentsSettings {
        on: true,
        chosen: false,
        ..AgentsSettings::default()
    });
    s.home.wd = run();
    s.home.wd_seen.clear();
}

/// A finished Wind down run: five agents, three answered.
fn run() -> WindDownStatus {
    let row = |i: usize, state: &str, reply: &str| WindDownRow {
        key: format!("sample/wd-{i}"),
        name: format!("card-{}-1", 700 + i),
        device_id: "sample".into(),
        device: "this Mac".into(),
        delivery_id: format!("d{i}"),
        state: state.into(),
        reply: reply.into(),
        error: String::new(),
    };
    WindDownStatus {
        id: "wd_sample".into(),
        scope: "this".into(),
        pause_after: true,
        paused: true,
        finished: true,
        rows: vec![
            row(1, "done", "Wound down. Left #701 with a hand-off; no leases held."),
            row(2, "done", "Wound down. Pushed card-702-1 and left a hand-off."),
            row(3, "done", "Wound down. Nothing in hand."),
            row(4, "no_answer", ""),
            WindDownRow {
                delivery_id: String::new(),
                error: "no route to Lead-PC".into(),
                ..row(5, "no_answer", "")
            },
        ],
    }
}

/// The scene `home-needs`: Home with one of every Needs you row.
pub fn needs_scene(s: &mut State) {
    s.close_card();
    s.go(Route::Home);
    needs(s);
}

/// The scene `wind-down`: the dialog over a finished run.
pub fn wind_down_scene(s: &mut State) {
    needs_scene(s);
    s.open_dialog(Dialog::WindDown);
}
