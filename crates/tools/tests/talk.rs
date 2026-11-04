//! Talk's tools (card #389): talk_post, talk_read, talk_list, talk_join
//! and talk_leave over channels, DMs and threads, and the conversation
//! views the window shares.

mod common;

use common::*;
use coord::talk as ct;
use inbox::mode::WAKE;

const USER: &str = "user:dev1";

#[test]
fn channels_mentions_and_reading() {
    let h = new_harness();
    let (a, c) = (call_a(), call_c());
    has(
        &h.okj(&a, "talk_list", json!({})),
        &["no conversations yet"],
    );
    let ch =
        h.b.talk
            .create_channel(USER, "general", "anything goes")
            .unwrap();
    // Not in it yet: listed as one to join.
    has(
        &h.okj(&a, "talk_list", json!({})),
        &["Channels you aren't in", "#general"],
    );
    has(
        &h.okj(&a, "talk_join", json!({"conv": "#general"})),
        &["Joined #general"],
    );
    has(
        &h.okj(&a, "talk_join", json!({"conv": ch.id})),
        &["already in #general"],
    );

    // A mention wakes gamma through the mention notice and adds it.
    let out = h.okj(
        &a,
        "talk_post",
        json!({"conv": "#general", "text": "@gamma can you look at the ports?"}),
    );
    has(&out, &["Posted post_", "Mentioned: gamma (woke)."]);
    let ns = h.notices(KEY_C);
    assert_eq!(ns.len(), 1, "{ns:?}");
    assert_eq!(ns[0].mode, WAKE);
    assert_eq!(ns[0].message.reason, coord::NOTICE_MENTIONED);
    assert_eq!(ns[0].message.ref_, format!("talk:{}", ch.id));
    has(
        &ns[0].message.text,
        &[
            "alpha mentioned you in #general",
            "can you look at the ports",
            &format!("talk_post conv={}", ch.id),
        ],
    );
    assert!(h.b.talk.is_member(&ch.id, KEY_C));
    // Beta isn't mentioned: no notice, and it isn't in the channel.
    assert!(h.notices(KEY_B).is_empty());
    assert!(!h.b.talk.is_member(&ch.id, KEY_B));

    // Gamma reads it: one unread first, none after.
    has(
        &h.okj(&c, "talk_list", json!({})),
        &["#general (channel", "1 unread", "1 mention you"],
    );
    has(
        &h.okj(&c, "talk_read", json!({"conv": "#general"})),
        &["can you look at the ports", "alpha ("],
    );
    has_not(&h.okj(&c, "talk_list", json!({})), &["unread"]);
    has(
        &h.okj(&c, "talk_read", json!({"conv": "#general"})),
        &["Nothing new"],
    );

    // Pages: since a post.
    let first = h.b.talk.posts(&ch.id)[0].post_id.clone();
    h.okj(
        &c,
        "talk_post",
        json!({"conv": "#general", "text": "on it"}),
    );
    let out = h.okj(&a, "talk_read", json!({"conv": ch.id, "since": first}));
    has(&out, &["on it"]);
    has_not(&out, &["can you look"]);

    // Leave; joining a DM or a thread isn't a thing.
    has(
        &h.okj(&c, "talk_leave", json!({"conv": "#general"})),
        &["Left #general"],
    );
    h.failsj(
        &a,
        "talk_join",
        json!({"conv": "@beta"}),
        &["only channels"],
    );
    h.failsj(
        &a,
        "talk_post",
        json!({"conv": "#nope", "text": "x"}),
        &["no channel, thread or agent"],
    );
    h.failsj(
        &a,
        "talk_post",
        json!({"conv": "#general", "text": "  "}),
        &["empty post"],
    );
}

#[test]
fn dms_wake_and_show_once() {
    let h = new_harness();
    let (a, b) = (call_a(), call_b());
    let out = h.okj(
        &a,
        "talk_post",
        json!({"conv": "@beta", "text": "got a minute?"}),
    );
    has(&out, &["Posted msg_"]);
    let dm = ct::dm_id(KEY_A, KEY_B);
    // Beta is woken with it, as a message about the DM.
    let ds = h.b.ib.for_agent(KEY_B, false, 10);
    assert_eq!(ds.len(), 1, "{ds:?}");
    assert_eq!(ds[0].mode, WAKE);
    assert_eq!(ds[0].message.ref_, format!("talk:{dm}"));
    assert_eq!(h.b.talk.members(&dm).len(), 2);

    // Beta answers with reply: the DM shows the post once and the reply.
    h.clk.advance(std::time::Duration::from_secs(5));
    h.okj(
        &b,
        "reply",
        json!({"delivery_id": ds[0].id, "text": "sure, what's up"}),
    );
    let ps = tools::talk::posts(&*h.b, KEY_A, &dm).unwrap();
    let texts: Vec<&str> = ps.iter().map(|p| p.text.as_str()).collect();
    assert_eq!(texts, ["got a minute?", "sure, what's up"]);
    has(
        &h.okj(&a, "talk_read", json!({"conv": "@beta"})),
        &["got a minute?", "sure, what's up"],
    );
    // Gamma may not read it; the person may.
    h.failsj(
        &call_c(),
        "talk_read",
        json!({"conv": dm}),
        &["isn't yours"],
    );
    assert_eq!(tools::talk::posts(&*h.b, USER, &dm).unwrap().len(), 2);

    // To the person: a post, no message.
    let before = h.b.ib.list(100).len();
    h.okj(
        &a,
        "talk_post",
        json!({"conv": "person", "text": "done with #3"}),
    );
    assert_eq!(h.b.ib.list(100).len(), before);
    let mine = ct::dm_id(KEY_A, USER);
    let cs = tools::talk::conversations(&*h.b, USER).unwrap();
    let p = cs
        .iter()
        .find(|x| x.conv_id == mine)
        .expect("the person's DM");
    assert_eq!((p.kind.as_str(), p.unread, p.read_only), ("dm", 1, false));
    assert_eq!(p.name, "alpha");
    // Alpha and beta's DM is the person's to read only.
    let ab = cs.iter().find(|x| x.conv_id == dm).expect("agents' DM");
    assert!(ab.read_only);
    assert_eq!(ab.unread, 0);
}

#[test]
fn agents_messages_mirror_as_dms() {
    let h = new_harness();
    let (a, c) = (call_a(), call_c());
    h.send_to(&a, KEY_C, "the ports clash on 8080");
    let dm = ct::dm_id(KEY_A, KEY_C);
    // No talk.post rows: it's read from the message.
    assert!(h.b.talk.posts(&dm).is_empty());
    let cs = tools::talk::conversations(&*h.b, USER).unwrap();
    let x = cs.iter().find(|x| x.conv_id == dm).expect("mirrored");
    assert!(x.read_only);
    assert_eq!(x.members.len(), 2);
    has(
        &h.okj(&c, "talk_read", json!({"conv": "@alpha"})),
        &["the ports clash on 8080"],
    );
    // Gamma's own list counts it.
    let cs = tools::talk::conversations(&*h.b, KEY_C).unwrap();
    assert!(cs.iter().any(|x| x.conv_id == dm && !x.read_only));
}

#[test]
fn threads_read_and_post_as_comments() {
    let h = new_harness();
    let a = call_a();
    let (t, _) =
        h.b.co
            .post_ask(
                USER,
                coord::NewAsk {
                    text: "why is the build slow?".into(),
                    thread: true,
                    ..Default::default()
                },
                &[],
            )
            .unwrap();
    let r = h.short(&t.id);
    let out = h.okj(
        &a,
        "talk_post",
        json!({"conv": r, "text": "the cache is cold"}),
    );
    has(&out, &["Commented comment:"]);
    assert!(
        h.b.talk.posts(&t.id).is_empty(),
        "no talk.post rows for a thread"
    );
    let out = h.okj(&a, "talk_read", json!({"conv": format!("task:{}", t.id)}));
    has(&out, &["why is the build slow?", "the cache is cold"]);
    let cs = tools::talk::conversations(&*h.b, USER).unwrap();
    let x = cs.iter().find(|x| x.conv_id == t.id).expect("the thread");
    assert_eq!(x.kind, "thread");
    assert_eq!(x.unread, 1, "alpha's answer");
    // Alpha answered, so it's on the thread's list too.
    assert!(
        tools::talk::conversations(&*h.b, KEY_A)
            .unwrap()
            .iter()
            .any(|x| x.conv_id == t.id)
    );
    // Joining a thread isn't Talk's.
    h.failsj(&a, "talk_join", json!({"conv": r}), &["only channels"]);
}

/// The next-turn reading (card #391): every agent in a channel but the
/// poster and the woken hears what it hasn't read there, at most five
/// lines and a count, a reply optional; a newer reading replaces one still
/// waiting. With "Agents answer now" off, even a mentioned agent isn't
/// woken.
#[test]
fn channel_members_read_at_their_next_turn() {
    use inbox::mode::NEXT_TURN;
    let h = new_harness();
    let (a, c) = (call_a(), call_c());
    let ch = h.b.talk.create_channel(USER, "general", "").unwrap();
    for k in [&a, &call_b(), &c] {
        h.okj(k, "talk_join", json!({"conv": "#general"}));
    }
    let waiting = |key: &str| -> Vec<inbox::Delivery> {
        h.notices(key)
            .into_iter()
            .filter(|d| d.message.reason == coord::NOTICE_READING && d.state == inbox::st::QUEUED)
            .collect()
    };
    for i in 1..=7 {
        h.okj(
            &a,
            "talk_post",
            json!({"conv": "#general", "text": format!("line {i}")}),
        );
    }
    assert!(waiting(KEY_A).is_empty(), "not the poster");
    for key in [KEY_B, KEY_C] {
        let ds = waiting(key);
        assert_eq!(ds.len(), 1, "one reading, the newest: {ds:?}");
        assert_eq!(ds[0].mode, NEXT_TURN);
        assert_eq!(ds[0].message.ref_, format!("talk:{}", ch.id));
        has(
            &ds[0].message.text,
            &[
                "Since you last read #general",
                "7 new posts, the last 5 below",
                "line 3",
                "line 7",
                "A reply is optional",
            ],
        );
        has_not(&ds[0].message.text, &["line 2"]);
    }

    // Gamma reads the channel: the next reading starts after its cursor.
    h.okj(&c, "talk_read", json!({"conv": "#general"}));
    h.okj(
        &a,
        "talk_post",
        json!({"conv": "#general", "text": "line 8"}),
    );
    let ds = waiting(KEY_C);
    assert_eq!(ds.len(), 1, "{ds:?}");
    has(&ds[0].message.text, &["1 new post.", "line 8"]);

    // The person, with Agents answer now off, names gamma: no wake.
    let me = inbox::Sender::new(USER, "you", "user");
    let p = tools::talk::post(
        &*h.b,
        &me,
        &tools::talk::Target::Channel(ch.id.clone()),
        "@gamma no rush on this",
        &[],
        false,
        None,
    )
    .unwrap();
    assert!(p.post.mentions.contains(&KEY_C.to_string()), "{:?}", p.post);
    assert!(!p.post.agents_now);
    let ns = h.notices(KEY_C);
    assert!(ns.iter().all(|d| d.mode != WAKE), "{ns:?}");
    let ds = waiting(KEY_C);
    assert_eq!(ds.len(), 1, "{ds:?}");
    has(&ds[0].message.text, &["no rush on this", "was off"]);
    // It quotes an agent's post too, so it is Irori's notice, never the
    // person's: no "user" trust over an agent's words.
    assert_eq!(ds[0].message.from.key, inbox::SYSTEM_KEY, "{ds:?}");
    assert_eq!(ds[0].message.trust, inbox::trust::UNTRUSTED, "{ds:?}");
}

/// A blocker row bridged from a shared board (card #478): Spencer's
/// agent raised it on a card two of my agents hold and watch.
fn spencer_blocker(h: &Harness, row: &str) -> Vec<inbox::Delivery> {
    h.b.ib
        .bridge(&inbox::BridgeRow {
            from: inbox::BoardFrom {
                board_id: "b3f5".into(),
                tag: "billing".into(),
                person_id: "pSpencer".into(),
                card_id: "cBilling".into(),
                row: row.into(),
                kind: inbox::row_kind::BLOCKER.into(),
                wake: false,
            },
            name: "spencer's agent BillingEng1".into(),
            to: vec![KEY_A.into(), KEY_B.into()],
            text: "spencer's agent BillingEng1 raised a blocker on \"ENG-6452\": rehearse first?"
                .into(),
            ref_: "task:T1".into(),
            hop: 1,
            attachments: vec![],
        })
        .unwrap()
}

fn board_dms(cs: &[tools::talk::Conversation]) -> Vec<String> {
    cs.iter()
        .filter(|c| c.members.iter().any(|m| m.key.starts_with("board:")))
        .map(|c| format!("{} {:?}", c.name, c.members))
        .collect()
}

#[test]
fn board_notices_never_show_as_dms() {
    // Card #478: each copy of one Board notice showed as a DM from a
    // board:<board>/<card> key, ten times over, once the agents it went
    // to had gone and the inbox handed every copy to the person.
    let h = new_harness();
    let ds = spencer_blocker(&h, "devS/7");
    assert_eq!(ds.len(), 2, "{ds:?}");
    for reader in [USER, KEY_A, KEY_B] {
        let cs = tools::talk::conversations(&*h.b, reader).unwrap();
        assert!(board_dms(&cs).is_empty(), "{reader}: {:?}", board_dms(&cs));
    }

    // Both agents end; past the grace the sweep closes each copy on the
    // card instead of moving it to the person.
    h.b.roster.remove(KEY_A);
    h.b.roster.remove(KEY_B);
    for _ in 0..4 {
        h.b.ib.sweep().unwrap();
        h.clk.advance(std::time::Duration::from_secs(10 * 60));
    }
    h.b.ib.sweep().unwrap();
    for d in &ds {
        let d = h.b.ib.get(&d.id).unwrap();
        assert_eq!(
            (d.state.as_str(), d.note.as_str()),
            (inbox::st::SKIPPED, "recipient gone; on task:T1"),
        );
    }
    let to_person: Vec<_> = h
        .b
        .ib
        .list(usize::MAX)
        .into_iter()
        .filter(|d| d.to.starts_with("user") && d.message.from.key.starts_with("board:"))
        .collect();
    assert!(to_person.is_empty(), "{to_person:?}");
    let cs = tools::talk::conversations(&*h.b, USER).unwrap();
    assert!(board_dms(&cs).is_empty(), "{:?}", board_dms(&cs));
}

#[test]
fn other_members_agents_go_by_name_never_their_key() {
    // Card #478: a DM with another member's agent named it
    // "board:b3f5.../f052..."; it is "BillingEng1 · Spencer".
    let h = new_harness();
    h.b.board_agents.lock().unwrap().push(tools::BoardAgent {
        board: "billing".into(),
        person: "Spencer".into(),
        card: presence::AgentCard {
            key: "agent:cBilling".into(),
            name: "BillingEng1".into(),
            status: "idle".into(),
            ..presence::AgentCard::default()
        },
    });
    let me = ct::Member::of(USER);
    let say = |other: &str, text: &str| {
        let o = ct::Member::of(other);
        let id = h.b.talk.open_dm(&me, &o, USER).unwrap();
        h.b.talk
            .post(ct::NewPost {
                conv_id: id.clone(),
                text: text.into(),
                by: USER.into(),
                ..ct::NewPost::default()
            })
            .unwrap();
        id
    };
    let named = say("board:b3f5/cBilling", "hi");
    let unknown = say("board:b3f5/cGoneLongAgo", "hi");
    let person = say("board:b3f5/person:pSpencer", "hi");
    // Opened from a notice and never written in: not listed.
    let empty = h
        .b
        .talk
        .open_dm(&me, &ct::Member::of("board:b3f5/cOther"), USER)
        .unwrap();

    let cs = tools::talk::conversations(&*h.b, USER).unwrap();
    let get = |id: &str| cs.iter().find(|c| c.conv_id == id);
    let c = get(&named).expect("named DM");
    assert_eq!(c.name, "BillingEng1 · Spencer");
    let m = c
        .members
        .iter()
        .find(|m| m.key.starts_with("board:"))
        .unwrap();
    assert_eq!(
        (m.kind.as_str(), m.name.as_str(), m.status.as_str(), m.owner.as_str()),
        ("agent", "BillingEng1 · Spencer", "idle", "Spencer")
    );
    let c = get(&unknown).expect("unknown agent's DM");
    assert_eq!(c.name, tools::talk::UNKNOWN_AGENT);
    assert!(c.members.iter().all(|m| m.status.is_empty()));
    let c = get(&person).expect("person's DM");
    assert_eq!(c.name, tools::talk::UNKNOWN_PERSON);
    assert!(c.members.iter().all(|m| m.kind == "person"), "{:?}", c.members);
    assert!(get(&empty).is_none());
    // No name anywhere is a key.
    for c in &cs {
        for n in std::iter::once(&c.name).chain(c.members.iter().map(|m| &m.name)) {
            assert!(
                !n.contains("board:") && !n.contains("agent:") && !n.contains("person:"),
                "{n}"
            );
        }
    }
}
