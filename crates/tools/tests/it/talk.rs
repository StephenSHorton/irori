//! Talk's tools (card #389): talk_post, talk_read, talk_list, talk_join
//! and talk_leave over channels, DMs and threads, and the conversation
//! views the window shares.

use crate::common;

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
fn a_conversation_carries_its_last_line_and_who_wrote_it() {
    // Card #486: Talk's list shows the newest post, not the section's name.
    let h = new_harness();
    let a = call_a();
    h.send_to(&a, KEY_C, "first\n\tthing");
    h.send_to(&a, KEY_C, "the ports clash\non 8080");
    let long = "x".repeat(tools::talk::LAST_TEXT + 40);
    h.send_to(&a, KEY_B, &long);
    let cs = tools::talk::conversations(&*h.b, USER).unwrap();
    let ac = cs
        .iter()
        .find(|x| x.conv_id == ct::dm_id(KEY_A, KEY_C))
        .unwrap();
    assert_eq!(ac.last_text, "the ports clash on 8080");
    assert_eq!(ac.last_by, KEY_A);
    let ab = cs
        .iter()
        .find(|x| x.conv_id == ct::dm_id(KEY_A, KEY_B))
        .unwrap();
    assert_eq!(ab.last_text.chars().count(), tools::talk::LAST_TEXT);
    assert!(ab.last_text.ends_with('…'));
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
        &[],
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

/// Two shared spaces, mlg and wind, as the tools see them (card #461).
struct TwoSpaces;

impl tools::SharedBoards for TwoSpaces {
    fn boards(&self) -> Vec<tools::BoardInfo> {
        let b = |id: &str, tag: &str| tools::BoardInfo {
            board_id: id.into(),
            tag: tag.into(),
            name: tag.into(),
            ..Default::default()
        };
        vec![b("sp_mlg", "mlg"), b("sp_wind", "wind")]
    }
    fn board_of(&self, _task_id: &str) -> Option<tools::BoardInfo> {
        None
    }
    fn board_coord(
        &self,
        _board_id: &str,
        _keys: &[String],
    ) -> Result<(std::sync::Arc<coord::Coord>, Vec<String>), coord::Error> {
        Err(coord::Error::raw(coord::ErrorKind::NotFound, "no board"))
    }
    fn share(&self, _: &str, _: &str, _: &str, _: &str) -> Result<coord::Task, String> {
        Err("no".into())
    }
    fn withdraw(&self, _: &str) -> Result<(), String> {
        Err("no".into())
    }
}

/// The spaces' logs: one store, each row carrying its spaceId.
struct SpaceRows(std::sync::Arc<store::Store>);

impl ct::SpaceLogs for SpaceRows {
    fn append(
        &self,
        _space: &str,
        _by: &str,
        kind: &str,
        payload: serde_json::Value,
    ) -> Result<store::Event, String> {
        self.0
            .append("irori", kind, &payload)
            .map_err(|e| e.to_string())
    }
    fn rows(&self) -> Vec<store::Event> {
        Vec::new()
    }
}

#[test]
fn a_channel_name_resolves_in_its_space_only() {
    let h = new_harness_setup(None, |b, _| {
        let _ = b.shared.set(std::sync::Arc::new(TwoSpaces));
        b.talk
            .set_spaces(std::sync::Arc::new(SpaceRows(b.st.clone())));
    });
    let a = call_a();
    let t = &h.b.talk;
    let mine = t.create_channel(USER, "general", "").unwrap();
    let mlg = t.create_channel_in(USER, "general", "", "sp_mlg").unwrap();
    let wind = t.create_channel_in(USER, "general", "", "sp_wind").unwrap();
    let net = t.create_channel_in(USER, "netcode", "", "sp_mlg").unwrap();

    // No space is Personal; space picks that space's #general.
    has(
        &h.okj(&a, "talk_join", json!({"conv": "#general"})),
        &[&format!("Joined #general ({})", mine.id)],
    );
    has(
        &h.okj(&a, "talk_join", json!({"conv": "#general", "space": "mlg"})),
        &[&format!("Joined #general ({})", mlg.id)],
    );
    has(
        &h.okj(
            &a,
            "talk_join",
            json!({"conv": "general", "space": "sp_wind"}),
        ),
        &[&format!("Joined #general ({})", wind.id)],
    );
    has(
        &h.okj(
            &a,
            "talk_post",
            json!({"conv": "#general", "space": "MLG", "text": "in mlg"}),
        ),
        &[&format!("to {}", mlg.id)],
    );
    assert_eq!(t.posts(&mlg.id).len(), 1);
    assert!(t.posts(&mine.id).is_empty() && t.posts(&wind.id).is_empty());
    has(
        &h.okj(
            &a,
            "talk_read",
            json!({"conv": "#general", "space": "personal"}),
        ),
        &["No posts in"],
    );

    // A name only another space has is refused with where it is, never
    // followed there.
    let out = h.failsj(
        &a,
        "talk_post",
        json!({"conv": "#netcode", "text": "x"}),
        &[
            "no channel #netcode in Personal",
            "space mlg",
            &net.id,
            "pass space=mlg",
        ],
    );
    has_not(&out, &["Posted"]);
    h.failsj(
        &a,
        "talk_join",
        json!({"conv": "#netcode", "space": "wind"}),
        &["no channel #netcode in space wind", "pass space=mlg"],
    );
    assert!(t.posts(&net.id).is_empty());
    assert!(!t.is_member(&net.id, KEY_A));
    // Its id works from anywhere; an unknown space is an error.
    has(
        &h.okj(&a, "talk_join", json!({"conv": net.id})),
        &["Joined #netcode"],
    );
    h.failsj(
        &a,
        "talk_read",
        json!({"conv": "#general", "space": "nope"}),
        &["in a shared space"],
    );

    // talk_list tells the three #generals apart.
    has(
        &h.okj(&a, "talk_list", json!({})),
        &[
            &format!("#general (channel, conv={})", mine.id),
            &format!("#general (channel in space mlg, conv={})", mlg.id),
            &format!("#general (channel in space wind, conv={})", wind.id),
        ],
    );
}

/// Each channel in the list carries its space (card #461), so the window's
/// switcher can filter Talk: empty for Personal, the space's id otherwise;
/// a DM and a private log's thread are Personal's.
#[test]
fn conversations_carry_their_space() {
    let h = new_harness_setup(None, |b, _| {
        let _ = b.shared.set(std::sync::Arc::new(TwoSpaces));
        b.talk
            .set_spaces(std::sync::Arc::new(SpaceRows(b.st.clone())));
    });
    let a = call_a();
    let t = &h.b.talk;
    let mine = t.create_channel(USER, "general", "").unwrap();
    let mlg = t.create_channel_in(USER, "general", "", "sp_mlg").unwrap();
    let wind = t.create_channel_in(USER, "general", "", "sp_wind").unwrap();
    h.okj(&a, "talk_post", json!({"conv": "person", "text": "hello"}));
    let dm = ct::dm_id(KEY_A, USER);
    let (th, _) =
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

    let cs = tools::talk::conversations(&*h.b, USER).unwrap();
    let space = |id: &str| {
        cs.iter()
            .find(|x| x.conv_id == id)
            .unwrap_or_else(|| panic!("{id} listed: {cs:?}"))
            .space_id
            .as_str()
    };
    assert_eq!(space(&mine.id), "");
    assert_eq!(space(&mlg.id), "sp_mlg");
    assert_eq!(space(&wind.id), "sp_wind");
    assert_eq!(space(&dm), "");
    assert_eq!(space(&th.id), "");
}

/// Two spaces, one tagged `personal` and one sharing its tag `art` with
/// another board, as the tools see them (card #461).
struct OddSpaces;

impl tools::SharedBoards for OddSpaces {
    fn boards(&self) -> Vec<tools::BoardInfo> {
        let b = |id: &str, tag: &str, space_board: &str| tools::BoardInfo {
            board_id: id.into(),
            tag: tag.into(),
            name: tag.into(),
            space_board: space_board.into(),
            ..Default::default()
        };
        vec![
            b("sp_p", "personal", ""),
            b("sp_art", "art", ""),
            b("sp_p", "art", "bd_p_art"),
        ]
    }
    fn board_of(&self, _task_id: &str) -> Option<tools::BoardInfo> {
        None
    }
    fn board_coord(
        &self,
        _board_id: &str,
        _keys: &[String],
    ) -> Result<(std::sync::Arc<coord::Coord>, Vec<String>), coord::Error> {
        Err(coord::Error::raw(coord::ErrorKind::NotFound, "no board"))
    }
    fn share(&self, _: &str, _: &str, _: &str, _: &str) -> Result<coord::Task, String> {
        Err("no".into())
    }
    fn withdraw(&self, _: &str) -> Result<(), String> {
        Err("no".into())
    }
}

/// The refusal for a name in another space only offers a `space` that
/// names that space, leaves out spaces this person no longer has, and a
/// name two Cores made at once in one space is refused with both ids.
#[test]
fn a_channel_name_refusal_names_only_what_resolves() {
    let h = new_harness_setup(None, |b, _| {
        let _ = b.shared.set(std::sync::Arc::new(OddSpaces));
        b.talk
            .set_spaces(std::sync::Arc::new(SpaceRows(b.st.clone())));
    });
    let a = call_a();
    let t = &h.b.talk;
    let p = t.create_channel_in(USER, "netcode", "", "sp_p").unwrap();
    let art = t.create_channel_in(USER, "ops", "", "sp_art").unwrap();
    // A space that is gone from this person's boards (left, or not ready).
    let gone = t.create_channel_in(USER, "lore", "", "sp_gone").unwrap();

    // A space tagged personal is named by its id: space=personal is Personal.
    let out = h.failsj(
        &a,
        "talk_post",
        json!({"conv": "#netcode", "text": "x"}),
        &["no channel #netcode in Personal", &p.id, "pass space=sp_p"],
    );
    has_not(&out, &["pass space=personal"]);
    has(
        &h.okj(
            &a,
            "talk_join",
            json!({"conv": "#netcode", "space": "sp_p"}),
        ),
        &[&format!("Joined #netcode ({})", p.id)],
    );
    // A tag two boards share is named by the space's id too.
    let out = h.failsj(
        &a,
        "talk_join",
        json!({"conv": "#ops"}),
        &["no channel #ops in Personal", "pass space=sp_art"],
    );
    has_not(&out, &["pass space=art"]);
    has(
        &h.okj(&a, "talk_join", json!({"conv": "#ops", "space": "sp_art"})),
        &[&format!("Joined #ops ({})", art.id)],
    );
    // A gone space isn't offered: the name is just not found.
    let out = h.failsj(&a, "talk_read", json!({"conv": "#lore"}), &["#lore"]);
    has_not(&out, &["sp_gone", &gone.id]);

    // Another Core of the space made #ops too before it heard of ours.
    let twin = h
        .b
        .st
        .append(
            "irori",
            store::kinds::TALK_CHANNEL_CREATED,
            &json!({"channelId": "ch_twin", "spaceId": "sp_art", "name": "ops", "by": "user:dev2"}),
        )
        .unwrap();
    assert!(t.apply(&twin));
    h.failsj(
        &a,
        "talk_post",
        json!({"conv": "#ops", "space": "sp_art", "text": "x"}),
        &["more than one #ops", &art.id, "ch_twin"],
    );
    assert!(t.posts(&art.id).is_empty() && t.posts("ch_twin").is_empty());
}

/// Space mlg with another person, Bob, and his agent bobbot on it, as the
/// tools see them (card #461).
struct BobsSpace;

impl tools::SharedBoards for BobsSpace {
    fn boards(&self) -> Vec<tools::BoardInfo> {
        vec![tools::BoardInfo {
            board_id: "sp_mlg".into(),
            tag: "mlg".into(),
            name: "mlg".into(),
            ..Default::default()
        }]
    }
    fn board_of(&self, _task_id: &str) -> Option<tools::BoardInfo> {
        None
    }
    fn board_coord(
        &self,
        _board_id: &str,
        _keys: &[String],
    ) -> Result<(std::sync::Arc<coord::Coord>, Vec<String>), coord::Error> {
        Err(coord::Error::raw(coord::ErrorKind::NotFound, "no board"))
    }
    fn share(&self, _: &str, _: &str, _: &str, _: &str) -> Result<coord::Task, String> {
        Err("no".into())
    }
    fn withdraw(&self, _: &str) -> Result<(), String> {
        Err("no".into())
    }
    fn board_mentionables(&self, board_id: &str) -> Vec<coord::roster::AgentCard> {
        if board_id != "sp_mlg" {
            return Vec::new();
        }
        let c = |key: &str, name: &str| coord::roster::AgentCard {
            key: key.into(),
            name: name.into(),
            status: "idle".into(),
            ..Default::default()
        };
        // Bob's other agent shares a name with this person's beta.
        vec![
            c("person:pBob", "Bob"),
            c("agent:cBob", "bobbot"),
            c("agent:cBeta", "beta"),
        ]
    }
}

/// In a space's channel @name reaches the space's other people and their
/// agents by board key (card #461): nothing is routed here for them (their
/// own Core wakes them from the row), the result says so, Bob himself
/// doesn't join, and a Personal channel never resolves them.
#[test]
fn a_space_channel_mention_names_the_other_persons_agent_by_board_key() {
    let h = new_harness_setup(None, |b, _| {
        let _ = b.shared.set(std::sync::Arc::new(BobsSpace));
        b.talk
            .set_spaces(std::sync::Arc::new(SpaceRows(b.st.clone())));
    });
    let a = call_a();
    let t = &h.b.talk;
    let mlg = t.create_channel_in(USER, "netcode", "", "sp_mlg").unwrap();
    let mine = t.create_channel(USER, "general", "").unwrap();
    h.okj(&a, "talk_join", json!({"conv": mlg.id}));
    let out = h.okj(
        &a,
        "talk_post",
        json!({"conv": mlg.id, "text": "@bobbot @Bob is rollback ready?"}),
    );
    has(&out, &["bobbot (in the space)", "Bob (in the space)"]);
    has_not(&out, &["Not delivered"]);
    let ps = t.posts(&mlg.id);
    let last = ps.last().unwrap();
    assert!(
        last.mentions.contains(&"agent:cBob".to_string()),
        "{last:?}"
    );
    assert!(
        last.mentions.contains(&"person:pBob".to_string()),
        "{last:?}"
    );
    // Bob's agent joins on the space; Bob joins himself, if he will.
    assert!(t.is_member(&mlg.id, "agent:cBob"));
    assert!(!t.is_member(&mlg.id, "person:pBob"));
    assert!(h.notices("agent:cBob").is_empty(), "nothing routed here");
    // A later post doesn't give Bob's agent a reading here either.
    h.okj(&a, "talk_post", json!({"conv": mlg.id, "text": "anyone?"}));
    assert!(h.notices("agent:cBob").is_empty(), "no reading here");

    // A Personal channel never names them.
    h.okj(&a, "talk_join", json!({"conv": mine.id}));
    let out = h.okj(
        &a,
        "talk_post",
        json!({"conv": mine.id, "text": "@bobbot hello"}),
    );
    has(&out, &["Not delivered", "bobbot"]);
}

/// A reply to a space channel's mention bridged from another person goes
/// in the channel and answers the delivery (card #461).
#[test]
fn a_reply_to_a_bridged_talk_mention_posts_in_the_channel() {
    let h = new_harness_setup(None, |b, _| {
        let _ = b.shared.set(std::sync::Arc::new(BobsSpace));
        b.talk
            .set_spaces(std::sync::Arc::new(SpaceRows(b.st.clone())));
    });
    let t = &h.b.talk;
    let mlg = t.create_channel_in(USER, "netcode", "", "sp_mlg").unwrap();
    let ds =
        h.b.ib
            .bridge(&inbox::BridgeRow {
                from: inbox::BoardFrom {
                    board_id: "sp_mlg".into(),
                    tag: "mlg".into(),
                    person_id: "pBob".into(),
                    card_id: "cBob".into(),
                    row: "devB/9".into(),
                    kind: inbox::row_kind::COMMENT.into(),
                    wake: true,
                },
                name: "Bob's agent bobbot".into(),
                to: vec![KEY_A.into()],
                text: "is rollback ready?".into(),
                ref_: format!("talk:{}", mlg.id),
                hop: 1,
                attachments: vec![],
            })
            .unwrap();
    assert_eq!(ds.len(), 1, "{ds:?}");
    let did = ds[0].id.clone();
    let out = h.okj(
        &call_a(),
        "reply",
        json!({"delivery_id": did, "text": "it is ready"}),
    );
    has(&out, &["Replied in #netcode", "is now answered"]);
    let ps = t.posts(&mlg.id);
    assert!(
        ps.iter().any(|p| p.text == "it is ready" && p.by == KEY_A),
        "{ps:?}"
    );
    assert_eq!(
        h.b.ib.get(&did).unwrap().state,
        inbox::st::ANSWERED,
        "answered"
    );
}

/// An @name this person's agent and Bob's agent both answer to in a space
/// channel names this person's own (card #461), as before the space's
/// agents could be named; Bob's is named by its key, as the picker passes
/// it.
#[test]
fn a_space_channel_name_tie_names_this_persons_own_agent() {
    let h = new_harness_setup(None, |b, _| {
        let _ = b.shared.set(std::sync::Arc::new(BobsSpace));
        b.talk
            .set_spaces(std::sync::Arc::new(SpaceRows(b.st.clone())));
    });
    let a = call_a();
    let t = &h.b.talk;
    let mlg = t.create_channel_in(USER, "netcode", "", "sp_mlg").unwrap();
    h.okj(&a, "talk_join", json!({"conv": mlg.id}));
    let out = h.okj(
        &a,
        "talk_post",
        json!({"conv": mlg.id, "text": "@beta is rollback ready?"}),
    );
    has_not(&out, &["Not delivered", "ambiguous"]);
    let last = t.posts(&mlg.id).pop().unwrap();
    assert_eq!(last.mentions, vec![KEY_B.to_string()], "{last:?}");
    assert!(!h.notices(KEY_B).is_empty(), "this person's beta hears it");

    let out = h.okj(
        &a,
        "talk_post",
        json!({"conv": mlg.id, "text": "@beta over to you", "mention": ["agent:cBeta"]}),
    );
    has(&out, &["(in the space)"]);
    let last = t.posts(&mlg.id).pop().unwrap();
    assert_eq!(last.mentions, vec!["agent:cBeta".to_string()], "{last:?}");
}

/// A blocker row bridged from a shared board (card #478): Spencer's
/// agent raised it on a card two of my agents hold and watch.
fn spencer_blocker(h: &Harness, row: &str) -> Vec<inbox::Delivery> {
    spencer_row(
        h,
        row,
        inbox::row_kind::BLOCKER,
        &[KEY_A, KEY_B],
        "spencer's agent BillingEng1 raised a blocker on \"ENG-6452\": rehearse first?",
    )
}

fn spencer_row(
    h: &Harness,
    row: &str,
    kind: &str,
    to: &[&str],
    text: &str,
) -> Vec<inbox::Delivery> {
    h.b.ib
        .bridge(&inbox::BridgeRow {
            from: inbox::BoardFrom {
                board_id: "b3f5".into(),
                tag: "billing".into(),
                person_id: "pSpencer".into(),
                card_id: "cBilling".into(),
                row: row.into(),
                kind: kind.into(),
                wake: false,
            },
            name: "spencer's agent BillingEng1".into(),
            to: to.iter().map(|k| k.to_string()).collect(),
            text: text.into(),
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
    let to_person: Vec<_> =
        h.b.ib
            .list(usize::MAX)
            .into_iter()
            .filter(|d| d.to.starts_with("user") && d.message.from.key.starts_with("board:"))
            .collect();
    assert!(to_person.is_empty(), "{to_person:?}");
    let cs = tools::talk::conversations(&*h.b, USER).unwrap();
    assert!(board_dms(&cs).is_empty(), "{:?}", board_dms(&cs));
}

#[test]
fn board_work_for_an_agent_that_has_gone_still_reaches_the_person() {
    // Card #478's review: only comments and blockers, which every holder
    // and watcher hears, close on the shared card. Work assigned to one
    // agent that has gone still reaches the person, as before, and it is
    // still no DM.
    let h = new_harness();
    let ds = spencer_row(
        &h,
        "devS/9",
        inbox::row_kind::ASSIGN,
        &[KEY_A],
        "spencer's agent BillingEng1 assigned you \"ENG-6569\".",
    );
    h.b.roster.remove(KEY_A);
    for _ in 0..4 {
        h.b.ib.sweep().unwrap();
        h.clk.advance(std::time::Duration::from_secs(10 * 60));
    }
    h.b.ib.sweep().unwrap();
    let d = h.b.ib.get(&ds[0].id).unwrap();
    assert_eq!(d.state, inbox::st::MOVED, "{d:?}");
    let to_person =
        h.b.ib
            .list(usize::MAX)
            .into_iter()
            .filter(|x| x.to.starts_with("user") && x.message.id == ds[0].message.id)
            .count();
    assert_eq!(to_person, 1);
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
    let empty =
        h.b.talk
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
        (
            m.kind.as_str(),
            m.name.as_str(),
            m.status.as_str(),
            m.owner.as_str()
        ),
        ("agent", "BillingEng1 · Spencer", "idle", "Spencer")
    );
    let c = get(&unknown).expect("unknown agent's DM");
    assert_eq!(c.name, tools::talk::UNKNOWN_AGENT);
    assert!(c.members.iter().all(|m| m.status.is_empty()));
    let c = get(&person).expect("person's DM");
    assert_eq!(c.name, tools::talk::UNKNOWN_PERSON);
    assert!(
        c.members.iter().all(|m| m.kind == "person"),
        "{:?}",
        c.members
    );
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

/// The person's Wind down reaches every agent but opens no DM, and a
/// "wound down" sent back as a message (as before card #307) stays out of
/// the DMs too, so nothing waits unread (card #484). An ordinary message
/// to the same agent still shows.
#[test]
fn a_wind_down_and_its_replies_open_no_dm() {
    let h = new_harness();
    let me = inbox::Sender::new(USER, "you", "user");
    let to = |k: &str| inbox::Address::new(inbox::addr::AGENT, k);
    let wd = || inbox::SendOptions {
        mode: WAKE.into(),
        reason: inbox::REASON_WIND_DOWN.into(),
        ..inbox::SendOptions::default()
    };
    let (ma, _) =
        h.b.ib
            .send_with(&me, &to(KEY_A), "Wind down", wd())
            .unwrap();
    h.b.ib
        .send_with(&me, &to(KEY_B), "Wind down", wd())
        .unwrap();
    // Alpha answers the old way: a message back, replying to it.
    h.b.ib
        .send_with(
            &call_a().sender,
            &inbox::Address::new(inbox::addr::REPLY, &ma.id),
            "Wound down. No cards held.",
            inbox::SendOptions::default(),
        )
        .unwrap();
    let mine_a = ct::dm_id(KEY_A, USER);
    let mine_b = ct::dm_id(KEY_B, USER);
    let cs = tools::talk::conversations(&*h.b, USER).unwrap();
    assert!(
        !cs.iter()
            .any(|c| c.conv_id == mine_a || c.conv_id == mine_b),
        "{cs:?}"
    );
    assert!(cs.iter().all(|c| c.unread == 0), "{cs:?}");
    assert!(tools::talk::posts(&*h.b, USER, &mine_a).unwrap().is_empty());

    // An ordinary message to alpha is a DM again, with only that in it.
    h.b.ib
        .send_with(&me, &to(KEY_A), "how's #3?", inbox::SendOptions::default())
        .unwrap();
    let ps = tools::talk::posts(&*h.b, USER, &mine_a).unwrap();
    let texts: Vec<&str> = ps.iter().map(|p| p.text.as_str()).collect();
    assert_eq!(texts, ["how's #3?"]);
}

/// Card #454 (spawned-agents.md §4 "Channels"): an agent Irori started
/// posts in a channel without joining it, so it gets no next-turn
/// readings; any other agent's post joins it, as before.
#[test]
fn a_spawned_agent_posts_without_joining() {
    let h = new_harness();
    let (a, c) = (call_a(), call_c());
    let ch = h.b.talk.create_channel(USER, "game-repo", "").unwrap();
    let t =
        h.b.co
            .create_task(
                USER,
                coord::NewTask {
                    title: "Fix the save test".into(),
                    ..coord::NewTask::default()
                },
            )
            .unwrap();
    let g =
        h.b.co
            .request_spawn(
                "irori",
                coord::SpawnAsk {
                    task_id: t.id.clone(),
                    device: "dev1".into(),
                    folder: "/repo".into(),
                    name: "otter".into(),
                    permission_mode: "default".into(),
                    ..coord::SpawnAsk::default()
                },
            )
            .unwrap()
            .unwrap();
    assert!(h.b.co.bind_spawn(&t.id, g, KEY_A).unwrap());
    assert!(h.b.co.is_spawned(KEY_A));
    has(
        &h.okj(
            &a,
            "talk_post",
            json!({"conv": "#game-repo", "text": "main is green again"}),
        ),
        &["Posted post_"],
    );
    assert_eq!(h.b.talk.posts(&ch.id).len(), 1);
    assert!(!h.b.talk.is_member(&ch.id, KEY_A), "spawned: not a member");
    h.okj(
        &c,
        "talk_post",
        json!({"conv": "#game-repo", "text": "thanks"}),
    );
    assert!(h.b.talk.is_member(&ch.id, KEY_C), "a terminal agent joins");
}

/// Card #538: agents don't DM other people, and people don't DM other
/// people's agents.
#[test]
fn a_dm_crosses_people_only_person_to_person() {
    let h = new_harness_setup(None, |_, _| {});
    let user = inbox::Sender::new(USER, "you", "user");
    let agent = inbox::Sender::new(KEY_A, "agent", "claude");
    let post = |from: &inbox::Sender, key: &str| {
        let other = ct::Member::of(key);
        let id = ct::dm_id(&from.key, key);
        tools::talk::post(
            &*h.b,
            from,
            &tools::talk::Target::Dm(id, Some(other)),
            "hi",
            &[],
            false,
            None,
            &[],
        )
        .map(|_| ())
        .unwrap_err()
    };
    assert!(post(&agent, "person:p1").contains("agents don't DM"));
    assert!(post(&agent, "board:b1/person:p1").contains("agents don't DM"));
    assert!(post(&user, "agent:c1").contains("other people's agents"));
    assert!(post(&user, "board:b1/c1").contains("other people's agents"));
    assert!(post(&agent, "agent:c1").contains("other people's agents"));
}

/// Card #538: what a person DM needs of the Core, faked: this person is
/// `me`, and the DM's rows come back stamped as a hidden space's do.
struct PairLogs(std::sync::Arc<store::Store>);

impl ct::SpaceLogs for PairLogs {
    fn append(
        &self,
        space: &str,
        _by: &str,
        kind: &str,
        payload: serde_json::Value,
    ) -> Result<store::Event, String> {
        let conv = payload["convId"].as_str().unwrap_or("").to_string();
        let mut ev = self
            .0
            .append("irori", kind, &payload)
            .map_err(|e| e.to_string())?;
        let mut v: serde_json::Value = serde_json::from_slice(&ev.payload).unwrap();
        v["spaceId"] = space.into();
        v["dmConv"] = conv.into();
        ev.payload = serde_json::to_vec(&v).unwrap();
        Ok(ev)
    }
    fn rows(&self) -> Vec<store::Event> {
        Vec::new()
    }
    fn person_id(&self) -> Option<String> {
        Some("me".into())
    }
    fn dm_for(&self, _other: &str) -> Result<String, String> {
        Ok("sp_dm".into())
    }
}

/// The people this person shares a space with, and what a DM waits on.
#[derive(Default)]
struct Shared {
    people: std::sync::Mutex<Vec<tools::PersonInfo>>,
    waiting: std::sync::Mutex<&'static str>,
}

impl tools::SharedBoards for Shared {
    fn boards(&self) -> Vec<tools::BoardInfo> {
        Vec::new()
    }
    fn board_of(&self, _task_id: &str) -> Option<tools::BoardInfo> {
        None
    }
    fn board_coord(
        &self,
        _board_id: &str,
        _keys: &[String],
    ) -> Result<(std::sync::Arc<coord::Coord>, Vec<String>), coord::Error> {
        Err(coord::Error::raw(coord::ErrorKind::NotFound, "no board"))
    }
    fn share(&self, _: &str, _: &str, _: &str, _: &str) -> Result<coord::Task, String> {
        Err("no".into())
    }
    fn withdraw(&self, _: &str) -> Result<(), String> {
        Err("no".into())
    }
    fn dm_people(&self) -> Vec<tools::PersonInfo> {
        self.people.lock().unwrap().clone()
    }
    fn dm_waiting(&self, other: &str) -> &'static str {
        assert_eq!(other, BOB);
        *self.waiting.lock().unwrap()
    }
}

const BOB: &str = "person:pBob";

fn someone(key: &str, name: &str) -> tools::PersonInfo {
    tools::PersonInfo {
        key: key.into(),
        name: name.into(),
    }
}

/// Bob on a shared space, and Spencer's agent BillingEng1 on one too.
fn bobs_harness() -> (Harness, std::sync::Arc<Shared>) {
    let shared = std::sync::Arc::new(Shared::default());
    shared.people.lock().unwrap().push(someone(BOB, "Bob"));
    let held = shared.clone();
    let h = new_harness_setup(None, move |b, _| {
        let _ = b.shared.set(held);
        b.talk
            .set_spaces(std::sync::Arc::new(PairLogs(b.st.clone())));
        b.known.lock().unwrap().insert(BOB.into(), "Bob".into());
    });
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
    (h, shared)
}

fn bob_dm() -> tools::talk::Target {
    tools::talk::Target::Dm(ct::pair_dm_id("person:me", BOB), Some(ct::Member::of(BOB)))
}

/// Card #538: the person reaches another person by key or by name, as one
/// pair id both sides compute. What can't be told apart is refused with
/// the keys that can.
#[test]
fn a_person_resolves_by_key_or_by_name_to_the_pair_dm() {
    let (h, shared) = bobs_harness();
    let go = |conv: &str| tools::talk::resolve(&*h.b, USER, conv, "");
    for conv in [BOB, "@person:pBob", "Bob", "bob", "@Bob"] {
        assert_eq!(go(conv), Ok(bob_dm()), "{conv}");
    }
    assert!(go("person:me").unwrap_err().contains("that's you"));
    assert!(go("Carol").unwrap_err().contains("is no channel"));

    // A key needs no shared space: the first post says so if there's none.
    assert!(matches!(
        go("person:pGone"),
        Ok(tools::talk::Target::Dm(..))
    ));

    // A board's key from a notice isn't a person DM.
    let e = go("board:b1/person:pBob").unwrap_err();
    assert!(e.contains("is no channel"), "{e}");

    // Two people of one name.
    let mut more = vec![someone("person:pBob2", "BOB")];
    shared.people.lock().unwrap().append(&mut more);
    let e = go("Bob").unwrap_err();
    assert!(e.contains("more than one person"), "{e}");
    assert!(e.contains(BOB) && e.contains("person:pBob2"), "{e}");
    assert_eq!(go(BOB), Ok(bob_dm()), "the key still names one");

    // One of the person's agents and another person of one name.
    h.b.roster
        .put(card(KEY_A, "alpha", "", "bypassPermissions"));
    let mut more = vec![someone("person:pAl", "alpha")];
    shared.people.lock().unwrap().append(&mut more);
    let e = go("alpha").unwrap_err();
    assert!(e.contains("more than one agent or person"), "{e}");
    assert!(e.contains(KEY_A) && e.contains("person:pAl"), "{e}");
}

/// Card #538: an agent is told no in words, by key or by name, reading or
/// writing, and nothing is written. Its own person is still its person,
/// and an agent's name still beats a person's for an agent.
#[test]
fn an_agent_cant_dm_a_person_and_nobody_dms_their_agent() {
    let (h, shared) = bobs_harness();
    let a = call_a();
    for conv in [BOB, "Bob", "@bob", "board:b1/person:pBob"] {
        h.failsj(
            &a,
            "talk_post",
            json!({"conv": conv, "text": "hi"}),
            &["agents don't DM other people"],
        );
        h.failsj(
            &a,
            "talk_read",
            json!({"conv": conv}),
            &["agents don't DM other people"],
        );
    }
    for conv in ["agent:cBilling", "board:b1/cBilling", "BillingEng1"] {
        h.failsj(
            &a,
            "talk_post",
            json!({"conv": conv, "text": "hi"}),
            &["other people's agents"],
        );
        let e = tools::talk::resolve(&*h.b, USER, conv, "").unwrap_err();
        assert!(e.contains("other people's agents"), "{e}");
    }
    assert!(h.b.talk.dms().is_empty(), "nothing opened");
    assert_eq!(h.b.st_rows("talk.post"), 0);

    // Two of one name is still a refusal, not a pick.
    let mut more = vec![someone("person:pBob2", "Bob")];
    shared.people.lock().unwrap().append(&mut more);
    h.failsj(
        &a,
        "talk_post",
        json!({"conv": "Bob", "text": "hi"}),
        &["agents don't DM other people"],
    );

    // Its own person by their person key is the DM with them.
    let own = tools::talk::resolve(&*h.b, KEY_A, "person:me", "").unwrap();
    assert_eq!(own.id(), ct::dm_id(KEY_A, "user"));

    // Beta is an agent here; a person called beta doesn't change that.
    let mut more = vec![someone("person:pBeta", "beta")];
    shared.people.lock().unwrap().append(&mut more);
    h.b.roster.put(card(KEY_B, "beta", "", "bypassPermissions"));
    let t = tools::talk::resolve(&*h.b, KEY_A, "beta", "").unwrap();
    assert_eq!(t.id(), ct::dm_id(KEY_A, KEY_B));
}

/// Card #538: nobody is woken through a DM with a person, and the post
/// says nothing of who wasn't: no mentions and no misses.
#[test]
fn a_person_dm_clears_its_mentions() {
    let (h, _) = bobs_harness();
    h.b.roster
        .put(card(KEY_A, "alpha", "", "bypassPermissions"));
    let user = inbox::Sender::new(USER, "you", "user");
    let out = tools::talk::post(
        &*h.b,
        &user,
        &bob_dm(),
        "@alpha @nobody have a look",
        &["alpha".to_string()],
        true,
        None,
        &[],
    )
    .unwrap();
    assert_eq!(out.post.conv_id, ct::pair_dm_id("person:me", BOB));
    assert!(out.post.mentions.is_empty(), "{:?}", out.post);
    assert!(out.mentions.is_empty() && out.misses.is_empty() && out.told.is_empty());
    assert!(out.sent.is_none());
    assert!(h.notices(KEY_A).is_empty(), "alpha isn't woken");
}

/// Card #538: a person DM's row names the other person and says what it is
/// waiting on, for the person only: an agent never sees one.
#[test]
fn a_person_dm_row_names_them_and_says_what_it_waits_on() {
    let (h, shared) = bobs_harness();
    let dm = tools::talk::open_dm(&*h.b, USER, BOB).unwrap();
    assert_eq!(dm, ct::pair_dm_id("person:me", BOB));
    assert_eq!(h.b.st_rows("talk.member"), 0, "opening one writes nothing");
    let row = || {
        tools::talk::conversations(&*h.b, USER)
            .unwrap()
            .into_iter()
            .find(|c| c.conv_id == dm)
            .expect("the DM is listed")
    };
    let c = row();
    assert_eq!((c.kind.as_str(), c.name.as_str()), ("dm", "Bob"));
    assert!(c.joined && !c.read_only && c.space_id.is_empty());
    assert!(c.waiting.is_empty(), "nothing is on its way");
    let bob = c.members.iter().find(|m| m.key == BOB).unwrap();
    assert_eq!((bob.kind.as_str(), bob.name.as_str()), ("person", "Bob"));

    for w in [
        tools::talk::WAITING_FOR_THEM,
        tools::talk::WAITING_FOR_UPDATE,
    ] {
        *shared.waiting.lock().unwrap() = w;
        assert_eq!(row().waiting, w);
    }

    let a = call_a();
    let listed = tools::talk::conversations(&*h.b, KEY_A).unwrap();
    assert!(listed.iter().all(|c| c.conv_id != dm), "{listed:?}");
    h.failsj(&a, "talk_read", json!({"conv": dm}), &["isn't yours"]);
    has_not(&h.okj(&a, "talk_list", json!({})), &["Bob", &dm]);

    // An agent's DM waits on nothing.
    let own = tools::talk::open_dm(&*h.b, USER, KEY_A).unwrap();
    let cs = tools::talk::conversations(&*h.b, USER).unwrap();
    let c = cs.iter().find(|c| c.conv_id == own).unwrap();
    assert!(c.waiting.is_empty(), "{c:?}");
}

/// Card #538: the window opens a DM through the same refusals: only
/// someone this person shares a space or a DM with, never their agent.
#[test]
fn opening_a_dm_checks_who_with() {
    let (h, shared) = bobs_harness();
    let open = |me: &str, key: &str| tools::talk::open_dm(&*h.b, me, key);
    let dm = open(USER, BOB).unwrap();
    let e = open(USER, "person:pCarol").unwrap_err();
    assert!(e.contains("don't share a space"), "{e}");
    assert!(open(USER, "person:me").unwrap_err().contains("that's you"));
    let e = open(USER, "agent:cBilling").unwrap_err();
    assert!(e.contains("other people's agents"), "{e}");
    let e = open(KEY_A, BOB).unwrap_err();
    assert!(e.contains("agents don't DM"), "{e}");

    // The DM stays yours after the last space you shared goes.
    shared.people.lock().unwrap().clear();
    assert_eq!(open(USER, BOB).unwrap(), dm);

    // The person's own agent, as before.
    let own = open(USER, KEY_A).unwrap();
    assert_eq!(own, ct::dm_id(USER, KEY_A));
    assert!(h.b.talk.is_member(&own, KEY_A));
}

/// Card #643: talk_post takes files like board_comment does. In a channel
/// the post carries them and talk_read shows them; files alone make a
/// post; a DM to an agent sends them on the message; a thread passes them
/// to its comment. A mention notice of a files-only post names them.
#[test]
fn posts_carry_files() {
    let h = new_harness();
    let (a, b, c) = (call_a(), call_b(), call_c());
    // No attacher: refused, nothing posted.
    let ch = h.b.talk.create_channel(USER, "general", "").unwrap();
    h.failsj(
        &a,
        "talk_post",
        json!({"conv": "#general", "text": "see", "attach": ["/tmp/shot.png"]}),
        &["attachments are off"],
    );
    assert!(h.b.talk.posts(&ch.id).is_empty());
    assert!(h.b.attacher.set(std::sync::Arc::new(NamedFiles)).is_ok());

    // In a channel: the post carries the file, talk_read shows it.
    let out = h.okj(
        &a,
        "talk_post",
        json!({"conv": "#general", "text": "the red build", "attach": ["/tmp/shot.png"]}),
    );
    has(&out, &["Posted post_", " with shot.png", "sha256="]);
    let ps = h.b.talk.posts(&ch.id);
    assert_eq!(ps.len(), 1);
    assert_eq!(ps[0].files.len(), 1);
    assert_eq!(ps[0].files[0].name, "shot.png");
    assert_eq!(ps[0].files[0].sha256, "a".repeat(64));
    has(
        &h.okj(&c, "talk_read", json!({"conv": "#general"})),
        &["the red build", "    file: shot.png", "(attachment_get)"],
    );

    // Files alone: an empty text is fine with a file, never without.
    h.clk.advance(std::time::Duration::from_secs(5));
    let out = h.okj(
        &a,
        "talk_post",
        json!({"conv": "#general", "text": "", "attach": ["/tmp/log.txt"], "mention": ["gamma"]}),
    );
    has(&out, &["Posted post_", " with log.txt"]);
    let ps = h.b.talk.posts(&ch.id);
    assert_eq!(ps.len(), 2);
    assert_eq!(ps[1].text, "");
    // The woken agent's notice names the file it can't otherwise see.
    let ns = h.notices(KEY_C);
    let n = ns.last().expect("gamma woken");
    has(&n.message.text, &["file: log.txt", "(attachment_get)"]);
    // The list's last line names the files of a post with no text.
    let cs = tools::talk::conversations(&*h.b, USER).unwrap();
    let x = cs.iter().find(|x| x.conv_id == ch.id).unwrap();
    assert_eq!(x.last_text, "log.txt");
    h.failsj(
        &a,
        "talk_post",
        json!({"conv": "#general", "text": "  "}),
        &["empty post"],
    );

    // A DM to an agent: the message carries the file, the post too, once.
    let out = h.okj(
        &a,
        "talk_post",
        json!({"conv": "@beta", "text": "", "attach": ["/tmp/trace.json"]}),
    );
    has(&out, &["Posted msg_", " with trace.json"]);
    let ds = h.b.ib.for_agent(KEY_B, false, 10);
    assert_eq!(ds.len(), 1, "{ds:?}");
    assert_eq!(ds[0].message.attachments.len(), 1);
    assert_eq!(ds[0].message.attachments[0].name, "trace.json");
    let dm = ct::dm_id(KEY_A, KEY_B);
    let ps = tools::talk::posts(&*h.b, KEY_B, &dm).unwrap();
    assert_eq!(ps.len(), 1, "{ps:?}");
    assert_eq!(ps[0].files.len(), 1);
    has(
        &h.okj(&b, "talk_read", json!({"conv": dm})),
        &["file: trace.json"],
    );

    // A thread: the comment carries the file, and reading it shows it.
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
        json!({"conv": r, "text": "flame graph", "attach": ["/tmp/flame.svg"]}),
    );
    has(&out, &["Commented comment:", "flame.svg"]);
    let cs = h.b.co.comments(&t.id).unwrap();
    let cm = cs.iter().find(|x| x.text == "flame graph").unwrap();
    assert_eq!(cm.attachments.len(), 1);
    assert_eq!(cm.attachments[0].name, "flame.svg");
    has(
        &h.okj(&a, "talk_read", json!({"conv": format!("task:{}", t.id)})),
        &["flame graph", "file: flame.svg"],
    );
}
