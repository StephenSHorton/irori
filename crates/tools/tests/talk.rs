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
    fn share(&self, _: &str, _: &str, _: &str) -> Result<coord::Task, String> {
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
        &["no shared board"],
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
/// a DM and a thread are always Personal's.
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
    fn share(&self, _: &str, _: &str, _: &str) -> Result<coord::Task, String> {
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
