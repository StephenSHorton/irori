//! Digest dedupe (I:internal/tools/notice.go) and the inbox half of the
//! card watcher (I:internal/notify/watch_test.go: the held, message and
//! reconcile tests; the Board/ask cards are coord's, ported at M6).

mod common;

use std::collections::HashSet;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use common::*;
use inbox::notice::{MERGED_NOTE, NOTICE_WATCHED, Notice, route_notice};
use inbox::watch::{
    CardAct, Level, Note, SeenStore, Sink, TAB_HELD, TAB_INBOX, Target, WAITING_ID, Watcher,
    held_id, message_id, preview, upper,
};
use inbox::{Address, Policy, Sender, addr, hold, mode, st, system_sender};

// ---- notice ----------------------------------------------------------------

/// A stand-in for coord.MergeWatch: digests are "Update on X:\n<lines>".
fn merge(earlier: &str, newer: &str) -> Option<String> {
    let (h1, l1) = earlier.split_once(":\n")?;
    let (h2, l2) = newer.split_once(":\n")?;
    if !h1.starts_with("Update on ") || h1 != h2 {
        return None;
    }
    Some(format!("{h2}:\n{l1}\n{l2}"))
}

fn watched(to: &[&str], text: &str) -> Notice {
    Notice {
        to: to.iter().map(|s| s.to_string()).collect(),
        kind: NOTICE_WATCHED.into(),
        text: text.into(),
        ref_: "task:T1".into(),
        ..Notice::default()
    }
}

#[test]
fn route_notice_merges_waiting_digests() {
    let h = Harness::new();
    h.abc();
    let sys = system_sender();
    let rs = route_notice(
        &h.ib,
        &sys,
        &watched(&[KEY_A, "", SYSTEM_KEY_ALIAS], "Update on T1:\n- a"),
        merge,
    );
    assert_eq!(rs.len(), 1, "empty and sender recipients are skipped");
    let first = rs[0].delivery.clone();
    assert!(rs[0].err.is_none());
    assert!(
        first.mode == mode::NEXT_TURN
            && first.message.notice
            && first.message.reason == NOTICE_WATCHED
            && first.message.ref_ == "task:T1"
    );
    let rs = route_notice(&h.ib, &sys, &watched(&[KEY_A], "Update on T1:\n- b"), merge);
    let second = rs[0].delivery.clone();
    assert_eq!(second.message.text, "Update on T1:\n- a\n- b");
    let old = h.want_state(&first.id, st::SKIPPED);
    assert_eq!(old.note, MERGED_NOTE);
    let open = h.ib.for_agent(KEY_A, false, 0);
    assert!(open.len() == 1 && open[0].id == second.id);

    // Once read (appended and delivered), a digest is not merged into.
    let w = h.append_for(KEY_A);
    h.ib.mark_delivered(KEY_A, &w.token).unwrap();
    let rs = route_notice(&h.ib, &sys, &watched(&[KEY_A], "Update on T1:\n- c"), merge);
    assert_eq!(rs[0].delivery.message.text, "Update on T1:\n- c");
    h.want_state(&second.id, st::DELIVERED);

    // Another card, another sender, or a wake-mode notice: no merge.
    let mut other = watched(&[KEY_B], "Update on T1:\n- x");
    route_notice(&h.ib, &sys, &other, merge);
    other.ref_ = "task:T2".into();
    let rs = route_notice(&h.ib, &sys, &other, merge);
    assert_eq!(rs[0].delivery.message.text, "Update on T1:\n- x");
    let rs = route_notice(
        &h.ib,
        &claude(KEY_C, "gamma"),
        &watched(&[KEY_B], "Update on T1:\n- y"),
        merge,
    );
    assert_eq!(rs[0].delivery.message.text, "Update on T1:\n- y");
    let mut wake = watched(&[KEY_B], "Update on T1:\n- z");
    wake.wake = true;
    let rs = route_notice(&h.ib, &sys, &wake, merge);
    assert!(
        rs[0].delivery.mode == mode::WAKE && rs[0].delivery.message.text == "Update on T1:\n- z"
    );
    assert_eq!(h.ib.for_agent(KEY_B, false, 0).len(), 4);
}

const SYSTEM_KEY_ALIAS: &str = inbox::SYSTEM_KEY;

#[test]
fn route_notice_reports_failures() {
    let h = Harness::new();
    h.abc();
    let n = Notice {
        to: vec!["nobody".into(), KEY_B.into()],
        kind: "assigned".into(),
        text: "you have a card".into(),
        ref_: "task:T1".into(),
        ..Notice::default()
    };
    let rs = route_notice(&h.ib, &claude(KEY_A, "alpha"), &n, merge);
    assert!(
        rs[0]
            .err
            .as_ref()
            .is_some_and(|e| e.is(inbox::ErrorKind::UnknownAgent))
    );
    assert!(rs[1].err.is_none() && rs[1].delivery.to == KEY_B);
    assert!(
        inbox::notice::notice_err(&rs)
            .unwrap()
            .starts_with("notice to nobody: inbox: no agent")
    );
}

// ---- watch -----------------------------------------------------------------

/// The fake toaster: posts, clears, what is live.
#[derive(Default)]
struct FakeSink {
    posts: Vec<Note>,
    live: BTreeMap<String, Note>,
    cleared: Vec<String>,
}

impl Sink for FakeSink {
    fn post(&mut self, note: Note) {
        self.live.insert(note.id.clone(), note.clone());
        self.posts.push(note);
    }
    fn clear(&mut self, id: &str) {
        self.live.remove(id);
        self.cleared.push(id.into());
    }
    fn live(&self, id: &str) -> bool {
        self.live.contains_key(id)
    }
}

impl FakeSink {
    #[track_caller]
    fn card(&self, id: &str) -> Note {
        self.live
            .get(id)
            .cloned()
            .unwrap_or_else(|| panic!("card {id} not up"))
    }
    fn posts_of(&self, id: &str) -> Vec<Note> {
        self.posts.iter().filter(|n| n.id == id).cloned().collect()
    }
    /// A button press takes the card down, then runs its action.
    #[track_caller]
    fn press(&mut self, id: &str, label: &str) -> CardAct {
        let c = self.card(id);
        self.live.remove(id);
        c.actions
            .iter()
            .find(|a| a.label == label)
            .and_then(|a| a.run.clone())
            .unwrap_or_else(|| panic!("no {label} on {id}"))
    }
}

struct Rig {
    h: Harness,
    w: Watcher,
    sink: FakeSink,
    sub: inbox::Subscription,
}

const ALPHA: (&str, &str) = (KEY_A, "alpha");

impl Rig {
    fn new() -> Rig {
        let h = Harness::new();
        h.ros
            .put(&[card(KEY_A, "alpha", "", ""), card(KEY_B, "beta", "", "")]);
        let names: BTreeMap<String, String> = [(KEY_A, "alpha"), (KEY_B, "beta")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let w = Watcher::new(
            USER,
            None,
            Some(Arc::new(move |k: &str| {
                names.get(k).cloned().unwrap_or_default()
            })),
        );
        let sub = h.ib.subscribe();
        let mut r = Rig {
            h,
            w,
            sink: FakeSink::default(),
            sub,
        };
        r.w.start(&*r.h.ib, &mut r.sink);
        r
    }

    /// Delivers every buffered change to the watcher, as Watch's loop does.
    fn pump(&mut self) {
        while let Some(c) = self.sub.try_recv() {
            let drained = self.sub.is_empty();
            self.w.on_change(&c, drained, &*self.h.ib, &mut self.sink);
        }
    }

    fn press(&mut self, act: &CardAct) -> Option<Target> {
        let t = self.w.press(act, &*self.h.ib, &mut self.sink);
        self.pump();
        t
    }

    fn held_message(&mut self, text: &str) -> inbox::Delivery {
        self.h
            .ib
            .set_policy(
                Policy {
                    block: vec![KEY_A.into()],
                    ..Policy::default()
                },
                USER,
            )
            .unwrap();
        let (_, d) = self
            .h
            .ib
            .send(&claude(ALPHA.0, ALPHA.1), &to(KEY_B), text, "")
            .unwrap();
        assert_eq!(d.state, st::HELD);
        self.pump();
        d
    }
}

#[test]
fn watch_held_release_acts_as_person() {
    let mut r = Rig::new();
    let d = r.held_message("please run the migration");
    let id = held_id(KEY_A);
    let c = r.sink.card(&id);
    assert!(
        c.title == "alpha sent beta a message Irori held"
            && c.body.contains("block list")
            && c.body.contains("migration"),
        "{c:?}"
    );
    assert!(c.body.starts_with("The sender is on your block list: "));
    assert!(c.sticky && c.level == Level::Warn && !c.silent);
    let labels: Vec<&str> = c.actions.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(labels, ["Release", "Dismiss"]);
    let act = r.sink.press(&id, "Release");
    r.press(&act);
    assert_eq!(r.h.state(&d.id).state, st::QUEUED);
    let evs =
        r.h.st
            .list(&store::Filter {
                limit: 1,
                desc: true,
                ..Default::default()
            })
            .unwrap();
    assert!(evs[0].actor == "user" && String::from_utf8_lossy(&evs[0].payload).contains(&d.id));
    assert!(!r.sink.live(&id));
}

#[test]
fn watch_held_cleared_when_resolved_elsewhere() {
    let mut r = Rig::new();
    let d = r.held_message("one");
    let id = held_id(KEY_A);
    assert!(r.sink.live(&id));
    r.h.ib.dismiss(&d.id, USER).unwrap();
    r.pump();
    assert!(!r.sink.live(&id));
    let d2 = r.held_message("two");
    assert_eq!(
        r.sink.card(&id).title,
        "alpha sent beta a message Irori held"
    );
    let act = r.sink.press(&id, "Dismiss");
    r.press(&act);
    assert_eq!(r.h.state(&d2.id).state, st::DISMISSED);
}

#[test]
fn watch_held_coalesces_per_sender() {
    let mut r = Rig::new();
    let id = held_id(KEY_A);
    let d1 = r.held_message("one");
    let d2 = r.held_message("two");
    let d3 = r.held_message("three");
    let c = r.sink.card(&id);
    assert_eq!(c.title, "alpha sent beta 3 messages Irori held");
    assert!(
        c.body.contains("three")
            && c.actions[0].label == "Release all"
            && c.actions[1].label == "Dismiss all"
    );
    let posts = r.sink.posts_of(&id);
    assert!(!posts[0].silent, "the first post made no sound");
    assert!(
        posts[1..].iter().all(|p| p.silent),
        "an update of a card that is up made a sound"
    );
    assert_eq!(
        r.sink
            .live
            .keys()
            .filter(|k| k.starts_with("held:"))
            .count(),
        1
    );
    r.h.ib.dismiss(&d1.id, USER).unwrap();
    r.pump();
    assert_eq!(
        r.sink.card(&id).title,
        "alpha sent beta 2 messages Irori held"
    );
    r.h.ib
        .set_policy(
            Policy {
                block: vec![KEY_A.into(), KEY_B.into()],
                ..Policy::default()
            },
            USER,
        )
        .unwrap();
    r.h.ib
        .send(&claude(KEY_B, "beta"), &to(KEY_A), "from beta", "")
        .unwrap();
    r.pump();
    assert_eq!(
        r.sink.card(&held_id(KEY_B)).title,
        "beta sent alpha a message Irori held"
    );
    let act = r.sink.press(&id, "Release all");
    r.press(&act);
    for d in [&d2, &d3] {
        assert_eq!(r.h.state(&d.id).state, st::QUEUED);
    }
    assert!(!r.sink.live(&id));
    assert!(
        r.sink.live(&held_id(KEY_B)),
        "releasing alpha's took beta's card down"
    );
}

#[test]
fn watch_paused_holds_stay_quiet() {
    let mut r = Rig::new();
    r.h.ib
        .set_policy(
            Policy {
                paused: true,
                ..Policy::default()
            },
            USER,
        )
        .unwrap();
    let (_, d) =
        r.h.ib
            .send(&claude(KEY_A, "alpha"), &to(KEY_B), "x", "")
            .unwrap();
    assert_eq!(d.hold, hold::PAUSED);
    r.pump();
    assert!(r.sink.posts.is_empty());
}

#[test]
fn watch_held_opens_the_held_list() {
    let mut r = Rig::new();
    r.held_message("please run the migration");
    let c = r.sink.card(&held_id(KEY_A));
    let t = r.press(&c.on_click.unwrap()).unwrap();
    assert!(t.tab == TAB_HELD && t.ref_ == KEY_A);
}

#[test]
fn watch_message_to_person() {
    let mut r = Rig::new();
    let you = Sender::new(USER, "you", "user");
    let (m, _) =
        r.h.ib
            .send(&you, &to(KEY_A), "how is it going?", "")
            .unwrap();
    let (_, back) =
        r.h.ib
            .send(
                &claude(KEY_A, "alpha"),
                &Address::new(addr::REPLY, &m.id),
                "done, tests pass",
                "",
            )
            .unwrap();
    assert_eq!(back.to, USER);
    r.pump();
    let id = message_id(&back.id);
    let c = r.sink.card(&id);
    assert!(
        c.title == "alpha sent you a message"
            && c.body == "done, tests pass"
            && c.actions.len() == 1
            && c.actions[0].label == "Open"
    );
    let t = r.press(&c.on_click.unwrap()).unwrap();
    assert!(t.tab == TAB_INBOX && t.ref_ == back.id);
    // Read by the person: cleared.
    r.h.ib.skip(USER, &back.id, "").unwrap();
    r.pump();
    assert!(!r.sink.live(&id));
}

#[test]
fn watch_catches_up_after_dropped_changes() {
    let mut r = Rig::new();
    let d = r.held_message("first");
    let id = held_id(KEY_A);
    // The watcher stalls: more changes than the 64-slot queue, unread.
    r.h.ib
        .set_policy(
            Policy {
                block: vec![KEY_A.into()],
                ..Policy::default()
            },
            USER,
        )
        .unwrap();
    r.h.ib
        .send(&claude(KEY_A, "alpha"), &to(KEY_B), "second", "")
        .unwrap();
    for _ in 0..80 {
        r.h.ib
            .set_policy(
                Policy {
                    block: vec![KEY_A.into()],
                    ..Policy::default()
                },
                USER,
            )
            .unwrap();
    }
    r.h.ib.dismiss(&d.id, USER).unwrap();
    let (_, d3) =
        r.h.ib
            .send(&claude(KEY_A, "alpha"), &to(KEY_B), "third", "")
            .unwrap();
    assert_eq!(d3.state, st::HELD);
    assert_eq!(r.sub.len(), inbox::SUB_BUFFER, "the queue dropped changes");
    r.pump();
    let c = r.sink.card(&id);
    assert!(
        c.title == "alpha sent beta 2 messages Irori held" && c.body.contains("third"),
        "{c:?}"
    );
}

#[test]
fn watch_held_card_comes_back_after_dismiss() {
    let mut r = Rig::new();
    let d = r.held_message("please run the migration");
    let id = held_id(KEY_A);
    let c = r.sink.card(&id);
    r.sink.live.remove(&id);
    r.press(
        &c.on_dismiss
            .clone()
            .expect("a held card has no dismiss handler"),
    );
    assert_eq!(r.h.state(&d.id).state, st::HELD);
    let you = Sender::new(USER, "you", "user");
    r.h.ib.send(&you, &to(KEY_B), "any change", "").unwrap();
    r.pump();
    assert_eq!(
        r.sink.card(&id).title,
        "alpha sent beta a message Irori held"
    );
    assert!(
        r.sink.posts_of(&id).last().unwrap().silent,
        "back after its x made a sound"
    );
}

#[test]
fn watch_posts_waiting_messages_at_start() {
    let h = Harness::new();
    h.ros
        .put(&[card(KEY_A, "alpha", "", ""), card(KEY_B, "beta", "", "")]);
    h.ib.set_policy(
        Policy {
            block: vec![KEY_A.into()],
            ..Policy::default()
        },
        USER,
    )
    .unwrap();
    let (_, held) =
        h.ib.send(&claude(KEY_A, "alpha"), &to(KEY_B), "held", "")
            .unwrap();
    assert_eq!(held.state, st::HELD);
    let you = Sender::new(USER, "you", "user");
    let (m, _) = h.ib.send(&you, &to(KEY_B), "status?", "").unwrap();
    let (_, back) =
        h.ib.send(
            &claude(KEY_B, "beta"),
            &Address::new(addr::REPLY, &m.id),
            "all green",
            "",
        )
        .unwrap();
    assert_eq!(back.to, USER);
    let mut sink = FakeSink::default();
    let mut w = Watcher::new(USER, None, None);
    w.start(&*h.ib, &mut sink);
    let ids: BTreeSet<String> = sink.posts.iter().map(|n| n.id.clone()).collect();
    assert!(
        ids.contains(&held_id(KEY_A)) && ids.contains(&message_id(&back.id)),
        "{ids:?}"
    );
}

#[test]
fn watch_held_release_failure_keeps_the_card() {
    let mut r = Rig::new();
    let (_, d) =
        r.h.ib
            .send(
                &claude(KEY_A, "alpha"),
                &Address::new(addr::REPO, "acme/nobody"),
                "deploy it",
                "",
            )
            .unwrap();
    assert!(d.state == st::HELD && d.hold == hold::UNOWNED);
    r.pump();
    let id = held_id(KEY_A);
    let act = r.sink.press(&id, "Release");
    r.press(&act);
    let c = r.sink.card(&id);
    assert!(
        c.body.contains("Couldn't release that message") && c.body.contains("deploy it"),
        "{}",
        c.body
    );
    assert!(c.level == Level::Error && c.actions.len() == 2 && c.actions[0].label == "Release");
    assert!(
        r.sink.posts_of(&id).last().unwrap().silent,
        "the failed card made a sound"
    );
    assert_eq!(r.h.state(&d.id).state, st::HELD);
    r.h.ib.dismiss(&d.id, USER).unwrap();
    r.pump();
    assert!(!r.sink.live(&id));
    let d2 = r.held_message("two");
    assert_eq!(
        r.sink.card(&id).title,
        "alpha sent beta a message Irori held"
    );
    let c = r.sink.card(&id);
    r.h.ib.release(&d2.id, USER).unwrap();
    let before = r.sink.posts.len();
    let dismiss = c
        .actions
        .iter()
        .find(|a| a.label == "Dismiss")
        .unwrap()
        .run
        .clone()
        .unwrap();
    r.press(&dismiss);
    assert!(
        r.sink.posts[before..]
            .iter()
            .all(|n| !n.body.contains("Couldn't")),
        "an error card after a resolved press"
    );
}

#[test]
fn preview_and_upper() {
    assert_eq!(preview("  a\n b\tc  "), "a b c");
    let long = "x".repeat(200);
    let p = preview(&long);
    assert!(p.chars().count() == 160 && p.ends_with('…'));
    assert_eq!(upper("éa"), "Éa");
    assert_eq!(upper(""), "");
}

#[test]
fn watch_held_board_row_shows_the_other_person() {
    // v0.5 §7.2: under Ask me a bridged wake request is held, and the
    // person sees a card naming who asked, with Release and Dismiss.
    let mut r = Rig::new();
    r.h.ib
        .set_receive(
            inbox::ReceiveSetting {
                preset: inbox::receive::ASK.into(),
                ..inbox::ReceiveSetting::default()
            },
            USER,
        )
        .unwrap();
    let ds =
        r.h.ib
            .bridge(&inbox::BridgeRow {
                from: inbox::BoardFrom {
                    board_id: "b0".into(),
                    tag: "mlg".into(),
                    person_id: "pBlake".into(),
                    card_id: "cBlake".into(),
                    row: "devB/1".into(),
                    kind: inbox::row_kind::COMMENT.into(),
                    wake: true,
                },
                name: "blake's agent mlg-lead".into(),
                to: vec![KEY_B.into()],
                text: "can you take the loader?".into(),
                ref_: "task:T1".into(),
                hop: 1,
                attachments: vec![],
            })
            .unwrap();
    assert_eq!(ds[0].hold, hold::MEMBER_WAKE);
    r.pump();
    let key = inbox::board_sender_key("b0", "cBlake", "pBlake");
    let id = held_id(&key);
    let c = r.sink.card(&id);
    assert!(
        c.title.contains("blake's agent mlg-lead")
            && c.body
                .contains("your receive setting for that board asks you first")
            && c.body.contains("take the loader"),
        "{c:?}"
    );
    let act = r.sink.press(&id, "Release");
    r.press(&act);
    let d = r.h.state(&ds[0].id);
    assert_eq!(
        (d.state.as_str(), d.mode.as_str()),
        (st::QUEUED, mode::WAKE)
    );
}

/// Card #265: a notice carrying the person's long text reaches the agent
/// clipped to MAX_TEXT, with a pointer to show for all of it.
#[test]
fn route_notice_clips_long_text_with_show() {
    let h = Harness::new();
    h.abc();
    let n = Notice {
        to: vec![KEY_A.into()],
        kind: "comment".into(),
        text: format!("Stephen commented:\n{}", "p".repeat(20_000)),
        ref_: "comment:C1".into(),
        ..Notice::default()
    };
    let rs = route_notice(&h.ib, &system_sender(), &n, |_, _| None);
    assert!(rs[0].err.is_none(), "{:?}", rs[0].err);
    let text = &rs[0].delivery.message.text;
    assert!(text.chars().count() <= inbox::MAX_TEXT);
    assert!(text.starts_with("Stephen commented:\npppp"));
    assert!(
        text.ends_with("(clipped; show comment:C1 for all of it)"),
        "{text:?}"
    );
}

// ---- restarts (#364) ---------------------------------------------------------

/// What a launch remembers of earlier runs, in memory.
#[derive(Default)]
struct MemSeen(Mutex<HashSet<String>>);

impl SeenStore for MemSeen {
    fn load(&self) -> HashSet<String> {
        self.0.lock().unwrap().clone()
    }
    fn save(&self, ids: &HashSet<String>) {
        *self.0.lock().unwrap() = ids.clone();
    }
}

/// beta answers two of the person's messages; returns their deliveries.
fn two_replies(h: &Harness) -> (inbox::Delivery, inbox::Delivery) {
    let you = Sender::new(USER, "you", "user");
    let mut back = Vec::new();
    for (ask, answer) in [("status?", "all green"), ("and the soak?", "still running")] {
        let (m, _) = h.ib.send(&you, &to(KEY_B), ask, "").unwrap();
        let (_, d) =
            h.ib.send(
                &claude(KEY_B, "beta"),
                &Address::new(addr::REPLY, &m.id),
                answer,
                "",
            )
            .unwrap();
        assert_eq!(d.to, USER);
        back.push(d);
    }
    let b = back.pop().unwrap();
    (back.pop().unwrap(), b)
}

fn launch(h: &Harness, seen: &Arc<MemSeen>) -> (Watcher, FakeSink) {
    let mut sink = FakeSink::default();
    let mut w = Watcher::new(USER, None, None).with_seen(seen.clone());
    w.start(&*h.ib, &mut sink);
    (w, sink)
}

#[test]
fn watch_start_shows_one_quiet_card_for_what_waited() {
    let h = Harness::new();
    h.ros
        .put(&[card(KEY_A, "alpha", "", ""), card(KEY_B, "beta", "", "")]);
    h.ib.set_policy(
        Policy {
            block: vec![KEY_A.into()],
            ..Policy::default()
        },
        USER,
    )
    .unwrap();
    h.ib.send(&claude(KEY_A, "alpha"), &to(KEY_B), "held", "")
        .unwrap();
    let (d1, d2) = two_replies(&h);
    let seen = Arc::new(MemSeen::default());

    let (mut w, mut sink) = launch(&h, &seen);
    let msgs: Vec<&Note> = sink
        .posts
        .iter()
        .filter(|n| n.id.starts_with("msg:"))
        .collect();
    assert_eq!(msgs.len(), 1, "one card for the waiting messages: {msgs:?}");
    let c = msgs[0];
    assert!(
        c.id == WAITING_ID
            && c.silent
            && c.title == "2 messages waited for you"
            && c.body == "From beta",
        "{c:?}"
    );
    // It stands for both, and opens Talk's list, where they are (#482).
    let mut ids = c.ids.clone();
    ids.sort();
    let mut both = vec![d1.id.clone(), d2.id.clone()];
    both.sort();
    assert_eq!(ids, both);
    assert_eq!(
        c.on_click,
        Some(CardAct::Open(Target {
            tab: inbox::watch::TAB_TALK.into(),
            ref_: String::new(),
        }))
    );
    assert!(
        sink.live(&held_id(KEY_A)),
        "a held message still gets its card"
    );

    // Read one: the card stays; read both: it goes.
    h.ib.skip(USER, &d1.id, "").unwrap();
    w.reconcile(&*h.ib, &mut sink, true);
    assert!(sink.live(WAITING_ID));
    // Restart before the last is read: nothing new to show.
    let (_, again) = launch(&h, &seen);
    assert!(
        again.posts.iter().all(|n| !n.id.starts_with("msg:")),
        "a restart toasted seen messages again: {:?}",
        again.posts
    );
    h.ib.skip(USER, &d2.id, "").unwrap();
    w.reconcile(&*h.ib, &mut sink, true);
    assert!(!sink.live(WAITING_ID));
    assert!(
        seen.load().is_empty(),
        "read messages stay in the seen file"
    );
}

#[test]
fn watch_restart_does_not_toast_a_message_again() {
    let h = Harness::new();
    h.ros
        .put(&[card(KEY_A, "alpha", "", ""), card(KEY_B, "beta", "", "")]);
    let seen = Arc::new(MemSeen::default());
    let (mut w, mut sink) = launch(&h, &seen);
    let sub = h.ib.subscribe();
    let (d1, _) = two_replies(&h);
    while let Some(c) = sub.try_recv() {
        w.on_change(&c, sub.is_empty(), &*h.ib, &mut sink);
    }
    let id = message_id(&d1.id);
    assert!(!sink.posts_of(&id).is_empty() && !sink.posts_of(&id)[0].silent);
    // Closed with its x, or not: either way the next launch shows nothing.
    sink.live.remove(&id);
    let (_, again) = launch(&h, &seen);
    assert!(again.posts.is_empty(), "{:?}", again.posts);
    // No store (as before): the restart shows the waiting card.
    let mut plain = FakeSink::default();
    Watcher::new(USER, None, None).start(&*h.ib, &mut plain);
    assert_eq!(plain.posts.len(), 1);
    assert_eq!(plain.posts[0].id, WAITING_ID);
}
