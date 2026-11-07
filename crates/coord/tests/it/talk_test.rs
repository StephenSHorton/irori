//! Talk's fold (card #389): channels, members, posts, DMs and read
//! cursors over `talk.*` rows, written through the store and folded again
//! from it.

use crate::common;

use common::*;
use coord::talk::{self, Member, NewPost, Talk};
use coord::{Error, ErrorKind};
use store::kinds;

fn talk(e: &Env) -> Talk {
    let t = Talk::new(Some(e.st.clone()), Some(e.clk.func()));
    t.load().unwrap();
    t
}

fn post(t: &Talk, conv: &str, by: &str, text: &str) -> talk::Post {
    t.post(NewPost {
        conv_id: conv.into(),
        text: text.into(),
        by: by.into(),
        agents_now: true,
        ..NewPost::default()
    })
    .unwrap()
}

fn want(r: coord::Result<impl std::fmt::Debug>, kind: ErrorKind) -> Error {
    let e = r.expect_err("want an error");
    assert!(e.is(kind), "{e:?}");
    e
}

#[test]
fn channel_posts_and_unread() {
    let e = Env::new();
    let t = talk(&e);
    let c = t.create_channel(USER, "  #general ", "what's up").unwrap();
    assert!(c.id.starts_with(talk::CHANNEL_PREFIX));
    assert_eq!(c.name, "general");
    assert_eq!(c.topic, "what's up");
    assert_eq!(c.space_id, "", "empty is Personal");
    assert_eq!(t.members(&c.id), vec![Member::of(USER)], "the maker joins");
    want(t.create_channel(ALICE, "GENERAL", ""), ErrorKind::Invalid);
    want(t.create_channel(ALICE, "  ", ""), ErrorKind::Invalid);

    let p1 = post(&t, &c.id, USER, "  hello room ");
    assert_eq!(p1.text, "hello room");
    e.clk.add(secs(1));
    // Posting joins the writer.
    let p2 = t
        .post(NewPost {
            conv_id: c.id.clone(),
            text: "hi @bob".into(),
            mentions: vec![BOB.into(), ALICE.into()],
            by: ALICE.into(),
            ..NewPost::default()
        })
        .unwrap();
    assert_eq!(p2.mentions, vec![BOB.to_string()], "not the writer");
    assert!(!p2.agents_now);
    assert!(t.is_member(&c.id, ALICE));
    e.clk.add(secs(1));
    let p3 = post(&t, &c.id, BOB, "hello");

    let ps = t.posts(&c.id);
    let ids: Vec<&str> = ps.iter().map(|p| p.post_id.as_str()).collect();
    assert_eq!(ids, [&p1.post_id, &p2.post_id, &p3.post_id]);

    // The person reads others' posts; Bob has one mention.
    assert_eq!(
        talk::unread(&ps, t.cursor(USER, &c.id).as_deref(), USER),
        (2, 0)
    );
    assert_eq!(talk::unread(&ps, None, BOB), (2, 1));
    // Any of the person's devices reads as the person.
    t.mark_read(&c.id, &p2.post_id, "user:dev-b").unwrap();
    assert_eq!(t.cursor(USER, &c.id).as_deref(), Some(p2.post_id.as_str()));
    assert_eq!(
        talk::unread(&ps, t.cursor(USER, &c.id).as_deref(), USER),
        (1, 0)
    );
    let since = talk::since(&ps, t.cursor(USER, &c.id).as_deref());
    assert_eq!(since.len(), 1);
    assert_eq!(since[0].post_id, p3.post_id);

    // Bad posts.
    want(
        t.post(NewPost {
            conv_id: c.id.clone(),
            text: "   ".into(),
            by: USER.into(),
            ..NewPost::default()
        }),
        ErrorKind::Invalid,
    );
    want(
        t.post(NewPost {
            conv_id: c.id.clone(),
            text: "x".repeat(coord::MAX_COMMENT + 1),
            by: ALICE.into(),
            ..NewPost::default()
        }),
        ErrorKind::Invalid,
    );
    want(
        t.post(NewPost {
            conv_id: "ch_nope".into(),
            text: "x".into(),
            by: USER.into(),
            ..NewPost::default()
        }),
        ErrorKind::NotFound,
    );
    want(
        t.post(NewPost {
            conv_id: c.id.clone(),
            text: "x".into(),
            by: USER.into(),
            thread_from: Some("post_nope".into()),
            ..NewPost::default()
        }),
        ErrorKind::NotFound,
    );
    let from = t
        .post(NewPost {
            conv_id: c.id.clone(),
            text: "about that".into(),
            by: USER.into(),
            thread_from: Some(p3.post_id.clone()),
            ..NewPost::default()
        })
        .unwrap();
    assert_eq!(from.thread_from.as_deref(), Some(p3.post_id.as_str()));

    // Rows: actors are the person's (user) and the agent's.
    let rows = e.rows(kinds::TALK_POST);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].actor, "user");
    assert_eq!(rows[1].actor, format!("agent:{ALICE}"));
    assert_eq!(e.rows(kinds::TALK_READ).len(), 1);
}

#[test]
fn reload_folds_the_same() {
    let e = Env::new();
    let t = talk(&e);
    let c = t.create_channel(USER, "dev", "").unwrap();
    let p = post(&t, &c.id, ALICE, "one");
    post(&t, &c.id, USER, "two");
    t.mark_read(&c.id, &p.post_id, USER).unwrap();
    t.leave(&c.id, &Member::of(ALICE), ALICE).unwrap();
    let dm = t
        .open_dm(&Member::of(USER), &Member::of(BOB), USER)
        .unwrap();
    post(&t, &dm, BOB, "psst");

    let t2 = talk(&e);
    assert_eq!(t2.channels(), t.channels());
    assert_eq!(t2.posts(&c.id), t.posts(&c.id));
    assert_eq!(t2.posts(&dm), t.posts(&dm));
    assert_eq!(t2.members(&c.id), vec![Member::of(USER)]);
    assert_eq!(t2.cursor(USER, &c.id), Some(p.post_id.clone()));
    assert_eq!(t2.dms(), t.dms());
    // Folding a row again changes nothing.
    for ev in e.rows("talk.") {
        assert!(!t2.apply(&ev), "{} folded twice", ev.kind);
    }
}

#[test]
fn dms_are_one_per_pair() {
    let e = Env::new();
    let t = talk(&e);
    let dm = t
        .open_dm(&Member::of(ALICE), &Member::of(USER), ALICE)
        .unwrap();
    assert_eq!(dm, talk::dm_id(USER, ALICE));
    assert_eq!(
        dm,
        talk::dm_id(ALICE, "user:dev-b"),
        "the person on any device"
    );
    // Opening it again writes nothing.
    let n = e.rows(kinds::TALK_MEMBER).len();
    assert_eq!(
        t.open_dm(&Member::of("user:dev-b"), &Member::of(ALICE), USER)
            .unwrap(),
        dm
    );
    assert_eq!(e.rows(kinds::TALK_MEMBER).len(), n);
    assert_eq!(t.members(&dm).len(), 2);
    assert_eq!(t.members(&dm)[1].kind, talk::PERSON);
    want(
        t.open_dm(&Member::of(USER), &Member::of("user:dev-b"), USER),
        ErrorKind::Invalid,
    );

    post(&t, &dm, ALICE, "hi");
    post(&t, &dm, "user:dev-b", "hello");
    want(
        t.post(NewPost {
            conv_id: dm.clone(),
            text: "let me in".into(),
            by: BOB.into(),
            ..NewPost::default()
        }),
        ErrorKind::NotAllowed,
    );
    // A thread id isn't Talk's to write.
    want(
        t.post(NewPost {
            conv_id: "01JTASK".into(),
            text: "x".into(),
            by: USER.into(),
            ..NewPost::default()
        }),
        ErrorKind::Invalid,
    );
    let dms = t.dms();
    assert_eq!(dms.len(), 1);
    assert_eq!(dms[0].0, dm);
}

/// A row another device wrote, as Follow hands it over (not stored).
fn row(
    dev: &str,
    seq: i64,
    at: u64,
    actor: &str,
    kind: &str,
    p: &serde_json::Value,
) -> store::Event {
    store::Event {
        device_id: dev.into(),
        seq,
        hlc: hlc::Timestamp(at),
        actor: actor.into(),
        kind: kind.into(),
        payload: gocompat::json::marshal(p).unwrap(),
        sig: Vec::new(),
    }
}

#[test]
fn rows_from_other_devices_fold_in_log_order() {
    let t = Talk::new(None, None);
    let t0 = hlc_at(fixed_now());
    // An edit that arrives before its channel still wins over it.
    let edit = row(
        "dev-b",
        2,
        t0 + 20,
        "user",
        kinds::TALK_CHANNEL_EDITED,
        &serde_json::json!({"channelId": "ch_1", "name": "renamed", "archived": true}),
    );
    assert!(!t.apply(&edit), "no channel yet");
    let made = row(
        "dev-b",
        1,
        t0 + 10,
        "user",
        kinds::TALK_CHANNEL_CREATED,
        &serde_json::json!({"channelId": "ch_1", "spaceId": "sp_1", "name": "first", "by": "user:dev-b"}),
    );
    assert!(t.apply(&made));
    let c = t.channel("ch_1").unwrap();
    assert_eq!((c.name.as_str(), c.archived), ("renamed", true));
    assert_eq!(c.space_id, "sp_1", "another space's id is kept");
    assert!(t.channel_named_in("renamed", "sp_1").is_none(), "archived");

    // Posts sort by their rows, not by arrival.
    for (seq, at, id) in [(4, 40, "p2"), (3, 30, "p1"), (5, 50, "p3")] {
        let ev = row(
            "dev-b",
            seq,
            t0 + at,
            &format!("agent:{ALICE}"),
            kinds::TALK_POST,
            &serde_json::json!({"postId": id, "convId": "ch_1", "text": id, "mentions": [], "agentsNow": true, "by": ALICE, "at": fixed_now()}),
        );
        assert!(t.apply(&ev));
        assert!(!t.apply(&ev), "once");
    }
    let ids: Vec<String> = t.posts("ch_1").into_iter().map(|p| p.post_id).collect();
    assert_eq!(ids, ["p1", "p2", "p3"]);

    // Join then an older leave: the join stands.
    let join = row(
        "dev-b",
        6,
        t0 + 60,
        "user",
        kinds::TALK_MEMBER,
        &serde_json::json!({"convId": "ch_1", "member": {"kind": "agent", "key": BOB}, "op": "join", "by": "user:dev-b"}),
    );
    let old_leave = row(
        "dev-c",
        1,
        t0 + 55,
        "user",
        kinds::TALK_MEMBER,
        &serde_json::json!({"convId": "ch_1", "member": {"kind": "agent", "key": BOB}, "op": "leave", "by": "user:dev-c"}),
    );
    assert!(t.apply(&join));
    assert!(!t.apply(&old_leave));
    assert!(t.is_member("ch_1", BOB));

    // A card made from a post (nothing writes it in Talk 1) shows on it.
    let card = row(
        "dev-b",
        7,
        t0 + 70,
        "user",
        kinds::TALK_CARD,
        &serde_json::json!({"postId": "p2", "taskId": "T1"}),
    );
    assert!(t.apply(&card));
    assert_eq!(t.posts("ch_1")[1].card.as_deref(), Some("T1"));
    assert_eq!(t.card_of("p2").as_deref(), Some("T1"));

    // A row that doesn't decode is dropped.
    let bad = row(
        "dev-b",
        8,
        t0 + 80,
        "user",
        kinds::TALK_POST,
        &serde_json::json!({"postId": 5}),
    );
    assert!(!t.apply(&bad));
}

#[test]
fn archived_channels_refuse_posts() {
    let e = Env::new();
    let t = talk(&e);
    let c = t.create_channel(USER, "old", "").unwrap();
    e.write_raw(
        "user",
        kinds::TALK_CHANNEL_EDITED,
        &serde_json::json!({"channelId": c.id, "archived": true}),
    );
    let t = talk(&e);
    want(
        t.post(NewPost {
            conv_id: c.id.clone(),
            text: "anyone?".into(),
            by: USER.into(),
            ..NewPost::default()
        }),
        ErrorKind::Invalid,
    );
    // Its name is free again.
    t.create_channel(USER, "old", "").unwrap();
}

#[test]
fn pages_run_oldest_first() {
    let e = Env::new();
    let t = talk(&e);
    let c = t.create_channel(USER, "busy", "").unwrap();
    let mut ids = Vec::new();
    for i in 0..7 {
        e.clk.add(secs(1));
        ids.push(post(&t, &c.id, ALICE, &format!("n{i}")).post_id);
    }
    let ps = t.posts(&c.id);
    let last3: Vec<String> = talk::page(&ps, None, 3)
        .into_iter()
        .map(|p| p.post_id)
        .collect();
    assert_eq!(last3, ids[4..]);
    let before: Vec<String> = talk::page(&ps, Some(&ids[4]), 3)
        .into_iter()
        .map(|p| p.post_id)
        .collect();
    assert_eq!(before, ids[1..4]);
    assert_eq!(talk::page(&ps, Some(&ids[1]), 3).len(), 1);
    assert_eq!(talk::page(&ps, Some(&ids[0]), 3).len(), 0);
    assert_eq!(talk::page(&ps, Some("unknown"), 2).len(), 2);
    assert_eq!(talk::since(&ps, Some("unknown")).len(), 7);
}

// Card #390, "Make a card" (design §4): the card made from a message is
// linked to it, the post carries it, and it survives a reload.
#[test]
fn a_card_made_from_a_post_is_linked() {
    let e = Env::new();
    let t = talk(&e);
    let c = t.create_channel(USER, "releases", "").unwrap();
    let p = post(&t, &c.id, USER, "Tag 0.10.0 once #96 merges");
    assert_eq!(t.card_of(&p.post_id), None);
    assert!(t.link_card(&p.post_id, "task-1", USER).unwrap());
    assert_eq!(t.card_of(&p.post_id).as_deref(), Some("task-1"));
    assert_eq!(t.posts(&c.id)[0].card.as_deref(), Some("task-1"));
    // The same link again writes nothing.
    assert!(!t.link_card(&p.post_id, "task-1", USER).unwrap());
    want(
        t.link_card("no-such-post", "task-1", USER),
        ErrorKind::NotFound,
    );
    want(t.link_card(&p.post_id, " ", USER), ErrorKind::Invalid);
    assert_eq!(talk(&e).card_of(&p.post_id).as_deref(), Some("task-1"));
}

/// Card #395: a mention or a DM notifies the person; a plain channel post,
/// their own post or one they've read doesn't.
#[test]
fn only_mentions_and_dms_notify() {
    let e = Env::new();
    let t = talk(&e);
    let reader = "user:phone";
    let c = t.create_channel(USER, "suzuri", "").unwrap();
    let plain = post(&t, &c.id, ALICE, "the build is green");
    assert!(
        t.notifies(&c.id, &plain.post_id, reader).is_none(),
        "a plain channel post"
    );
    let tagged = t
        .post(NewPost {
            conv_id: c.id.clone(),
            text: "@you anything blocking?".into(),
            mentions: vec!["user".into()],
            by: ALICE.into(),
            ..NewPost::default()
        })
        .unwrap();
    assert_eq!(
        t.notifies(&c.id, &tagged.post_id, reader).map(|p| p.text),
        Some("@you anything blocking?".into()),
        "a mention of the person, read on any of their devices"
    );
    let own = t
        .post(NewPost {
            conv_id: c.id.clone(),
            text: "note to self @you".into(),
            mentions: vec!["user".into()],
            by: USER.into(),
            ..NewPost::default()
        })
        .unwrap();
    assert!(
        t.notifies(&c.id, &own.post_id, reader).is_none(),
        "their own post"
    );

    let dm = t
        .open_dm(&Member::of(USER), &Member::of(ALICE), USER)
        .unwrap();
    let said = post(&t, &dm, ALICE, "done with #330");
    assert!(
        t.notifies(&dm, &said.post_id, reader).is_some(),
        "a DM to the person"
    );
    t.mark_read(&dm, &said.post_id, USER).unwrap();
    assert!(
        t.notifies(&dm, &said.post_id, reader).is_none(),
        "read already"
    );

    let theirs = t
        .open_dm(&Member::of(ALICE), &Member::of(BOB), ALICE)
        .unwrap();
    let between = post(&t, &theirs, ALICE, "hi bob");
    assert!(
        t.notifies(&theirs, &between.post_id, reader).is_none(),
        "two agents' DM"
    );
}

// ---- a space's channels (card #392) ----------------------------------------

/// A space log that takes rows only while `ready` (the Core's version
/// gate), keeping keys as written.
struct FakeSpace {
    st: std::sync::Arc<store::Store>,
    ready: std::sync::atomic::AtomicBool,
}

impl talk::SpaceLogs for FakeSpace {
    fn append(
        &self,
        space: &str,
        _by: &str,
        kind: &str,
        payload: serde_json::Value,
    ) -> Result<store::Event, String> {
        assert_eq!(space, "s1");
        if !self.ready.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("every device in the space must run an Irori with spaces first; waiting on Bob/abc123".into());
        }
        self.st
            .append("irori", kind, &payload)
            .map_err(|e| e.to_string())
    }

    fn rows(&self) -> Vec<store::Event> {
        self.st
            .list(&store::Filter {
                kinds: vec!["talk.".into(), kinds::CALL_SEGMENT.into()],
                ..Default::default()
            })
            .unwrap()
    }
}

fn kinds_in(st: &store::Store) -> Vec<String> {
    st.list(&store::Filter {
        kinds: vec!["talk.".into(), kinds::CALL_SEGMENT.into()],
        ..Default::default()
    })
    .unwrap()
    .into_iter()
    .map(|e| e.kind)
    .collect()
}

#[test]
fn a_space_channel_goes_in_the_space_log_behind_the_gate() {
    let (e, sp) = (Env::new(), Env::on("dev-a"));
    let space = std::sync::Arc::new(FakeSpace {
        st: sp.st.clone(),
        ready: false.into(),
    });
    let t = talk(&e);
    t.set_spaces(space.clone());

    // Not ready: refused with the devices it waits on, and nothing written.
    let err = want(
        t.create_channel_in(USER, "netcode", "", "s1"),
        ErrorKind::NotAllowed,
    );
    assert!(err.to_string().contains("waiting on Bob/abc123"), "{err}");
    assert!(t.channel_named_in("netcode", "s1").is_none());
    assert!(kinds_in(&sp.st).is_empty());
    // Personal channels and DMs with an agent are never gated.
    let mine = t.create_channel(USER, "notes", "").unwrap();
    post(&t, &mine.id, USER, "personal");
    let dm = t
        .open_dm(&Member::of(USER), &Member::of(ALICE), USER)
        .unwrap();
    post(&t, &dm, USER, "a dm");

    space.ready.store(true, std::sync::atomic::Ordering::SeqCst);
    let c = t
        .create_channel_in(USER, "netcode", "rollback", "s1")
        .unwrap();
    assert_eq!(c.space_id, "s1");
    let p = post(&t, &c.id, USER, "hello space");
    t.mark_read(&c.id, &p.post_id, USER).unwrap();
    // The channel, its member and post rows are the space's; the read
    // cursor stays in the private log.
    assert_eq!(
        kinds_in(&sp.st),
        [
            kinds::TALK_CHANNEL_CREATED,
            kinds::TALK_MEMBER,
            kinds::TALK_POST
        ]
    );
    let private = kinds_in(&e.st);
    assert!(private.contains(&kinds::TALK_READ.to_string()));
    let created = private
        .iter()
        .filter(|k| *k == kinds::TALK_CHANNEL_CREATED)
        .count();
    assert_eq!(created, 1, "only the Personal channel's");

    // A later post waits again while a device isn't ready.
    space
        .ready
        .store(false, std::sync::atomic::Ordering::SeqCst);
    want(
        t.post(NewPost {
            conv_id: c.id.clone(),
            text: "too soon".into(),
            by: USER.into(),
            ..NewPost::default()
        }),
        ErrorKind::NotAllowed,
    );
    assert_eq!(t.posts(&c.id).len(), 1);

    // A fresh fold (a start, or a resync of the private log) reads the
    // space's rows too.
    let again = Talk::new(Some(e.st.clone()), Some(e.clk.func()));
    again.set_spaces(space);
    again.load().unwrap();
    let c2 = again.channel(&c.id).unwrap();
    assert_eq!(c2.space_id, "s1");
    assert_eq!(again.posts(&c.id)[0].text, "hello space");
    assert_eq!(again.cursor(USER, &c.id), Some(p.post_id));
    assert_eq!(again.channels().len(), 2);
}

#[test]
fn a_space_channel_without_spaces_is_refused() {
    let e = Env::new();
    let t = talk(&e);
    want(
        t.create_channel_in(USER, "x", "", "s1"),
        ErrorKind::NotAllowed,
    );
}

/// Card #461: a space forgotten or left here takes its channels, members
/// and posts out of the fold, Personal stays, and the space's rows fold
/// back in whole when it returns (the post ids left the dedupe set).
#[test]
fn dropping_a_space_takes_its_channels_out() {
    let (e, sp) = (Env::new(), Env::on("dev-a"));
    let space = std::sync::Arc::new(FakeSpace {
        st: sp.st.clone(),
        ready: true.into(),
    });
    let t = talk(&e);
    t.set_spaces(space.clone());
    let mine = t.create_channel(USER, "notes", "").unwrap();
    post(&t, &mine.id, USER, "personal");
    let c = t.create_channel_in(USER, "netcode", "", "s1").unwrap();
    let p = post(&t, &c.id, USER, "hello space");
    assert!(!t.members(&c.id).is_empty());

    assert!(t.drop_space("s1"));
    assert!(t.channel(&c.id).is_none());
    assert!(t.posts(&c.id).is_empty());
    assert!(t.members(&c.id).is_empty());
    assert!(!t.has_post(&p.post_id));
    assert!(t.channel(&mine.id).is_some(), "the Personal channel went");
    assert_eq!(t.posts(&mine.id).len(), 1);
    assert!(!t.drop_space("s1"), "nothing left to drop");
    assert!(
        !t.drop_space(talk::PERSONAL_SPACE),
        "Personal is never dropped"
    );
    assert!(t.channel(&mine.id).is_some());

    // The space comes back (a re-invite): its rows fold in again.
    for ev in talk::SpaceLogs::rows(&*space) {
        t.apply(&ev);
    }
    assert_eq!(t.channel(&c.id).unwrap().space_id, "s1");
    let ps = t.posts(&c.id);
    assert_eq!(ps.len(), 1);
    assert_eq!(ps[0].text, "hello space");
}

/// Any number of spaces over one log: each row carries its spaceId.
struct ManySpaces {
    st: std::sync::Arc<store::Store>,
}

impl talk::SpaceLogs for ManySpaces {
    fn append(
        &self,
        _space: &str,
        _by: &str,
        kind: &str,
        payload: serde_json::Value,
    ) -> Result<store::Event, String> {
        self.st
            .append("irori", kind, &payload)
            .map_err(|e| e.to_string())
    }

    fn rows(&self) -> Vec<store::Event> {
        self.st
            .list(&store::Filter {
                kinds: vec!["talk.".into()],
                ..Default::default()
            })
            .unwrap()
    }
}

#[test]
fn channel_names_are_unique_per_space() {
    let (e, sp) = (Env::new(), Env::on("dev-a"));
    let t = talk(&e);
    t.set_spaces(std::sync::Arc::new(ManySpaces { st: sp.st.clone() }));

    // The same name in two spaces and in Personal (card #461).
    let s1 = t.create_channel_in(USER, "general", "", "s1").unwrap();
    let s2 = t.create_channel_in(USER, "#General", "", "s2").unwrap();
    let mine = t.create_channel(USER, "general", "").unwrap();
    assert_ne!(s1.id, s2.id);
    // Twice in one space is refused, any case.
    let err = want(
        t.create_channel_in(USER, "GENERAL", "", "s1"),
        ErrorKind::Invalid,
    );
    assert!(err.to_string().contains("in this space"), "{err}");
    want(t.create_channel(USER, "#general", ""), ErrorKind::Invalid);

    // Each lookup stays in its space; no space is Personal.
    let id = |c: Option<talk::Channel>| c.map(|c| c.id);
    assert_eq!(
        id(t.channel_named_in("#general", "s1")),
        Some(s1.id.clone())
    );
    assert_eq!(
        id(t.channel_named_in("general", " s2 ")),
        Some(s2.id.clone())
    );
    assert_eq!(id(t.channel_named_in("general", "")), Some(mine.id.clone()));
    assert_eq!(id(t.channel_named("general")), Some(mine.id.clone()));
    assert!(t.channel_named_in("general", "s3").is_none());
    let only = t.create_channel_in(USER, "netcode", "", "s1").unwrap();
    assert!(
        t.channel_named("netcode").is_none(),
        "s1's isn't Personal's"
    );
    assert_eq!(id(t.channel_named_in("netcode", "s1")), Some(only.id));
    // Where a name lives, every space.
    let all: Vec<String> = t
        .channels_named("general")
        .into_iter()
        .map(|c| c.space_id)
        .collect();
    assert_eq!(all, ["", "s1", "s2"]);
}

#[test]
fn another_person_is_a_person() {
    assert_eq!(Member::of("person:abc").kind, talk::PERSON);
    assert_eq!(talk::canon("person:abc"), "person:abc", "not this person");
    assert_eq!(Member::of("dev-a/x").kind, talk::AGENT);
}

/// A space log as the Core keeps it: rows go in under board keys (this
/// device's `user:` key is `person:stephen`), and come back with this
/// device's keys.
struct KeyedSpace {
    st: std::sync::Arc<store::Store>,
}

fn keys_in_payload(v: &mut serde_json::Value, f: impl Fn(&str) -> String) {
    if let Some(serde_json::Value::String(by)) = v.get_mut("by") {
        *by = f(by);
    }
    if let Some(serde_json::Value::String(k)) = v.get_mut("member").and_then(|m| m.get_mut("key")) {
        *k = f(k);
    }
}

fn to_board(k: &str) -> String {
    if k == USER {
        "person:stephen".into()
    } else {
        k.into()
    }
}

fn from_board(k: &str) -> String {
    if k == "person:stephen" {
        USER.into()
    } else {
        k.into()
    }
}

fn back(mut ev: store::Event) -> store::Event {
    let mut v: serde_json::Value = serde_json::from_slice(&ev.payload).unwrap();
    keys_in_payload(&mut v, from_board);
    ev.payload = serde_json::to_vec(&v).unwrap();
    ev
}

impl talk::SpaceLogs for KeyedSpace {
    fn append(
        &self,
        _space: &str,
        by: &str,
        kind: &str,
        mut payload: serde_json::Value,
    ) -> Result<store::Event, String> {
        keys_in_payload(&mut payload, to_board);
        let ev = self
            .st
            .append(&to_board(by), kind, &payload)
            .map_err(|e| e.to_string())?;
        Ok(back(ev))
    }

    fn rows(&self) -> Vec<store::Event> {
        self.st
            .list(&store::Filter {
                kinds: vec!["talk.".into()],
                ..Default::default()
            })
            .unwrap()
            .into_iter()
            .map(back)
            .collect()
    }
}

/// Card #392 (nutmeg's check, ported): a space's rows go in its log by
/// board keys, and two people in a space stay two members; only `user:`
/// keys fold into this person.
#[test]
fn a_space_talk_writes_by_board_keys() {
    let (e, sp) = (Env::new(), Env::on("dev-a"));
    let t = talk(&e);
    t.set_spaces(std::sync::Arc::new(KeyedSpace { st: sp.st.clone() }));
    let c = t.create_channel_in(USER, "general", "", "space1").unwrap();
    assert_eq!(c.space_id, "space1");
    assert_eq!(c.by, USER, "this person, as this device reads it");
    // Blake's join arrives from the space's log.
    let ev = sp
        .st
        .append(
            "person:blake",
            kinds::TALK_MEMBER,
            &serde_json::json!({
                "convId": c.id, "member": {"kind": "person", "key": "person:blake"},
                "op": "join", "by": "person:blake"
            }),
        )
        .unwrap();
    assert!(t.apply(&back(ev)));
    let ms = t.members(&c.id);
    assert_eq!(ms.len(), 2, "two people stay two: {ms:?}");
    assert!(ms.iter().all(|m| m.kind == talk::PERSON), "{ms:?}");
    assert_ne!(talk::canon("person:stephen"), talk::canon("person:blake"));
    assert_ne!(talk::canon("person:blake"), talk::canon(USER));
    assert!(t.is_member(&c.id, USER) && t.is_member(&c.id, "person:blake"));
    // The rows sit in the space's log under the board actor and key.
    let rows = sp
        .st
        .list(&store::Filter {
            kinds: vec![
                kinds::TALK_CHANNEL_CREATED.into(),
                kinds::TALK_MEMBER.into(),
            ],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(rows[0].actor, "person:stephen");
    let created: serde_json::Value = serde_json::from_slice(&rows[0].payload).unwrap();
    assert_eq!(created["by"], "person:stephen");
    let joined: serde_json::Value = serde_json::from_slice(&rows[1].payload).unwrap();
    assert_eq!(rows[1].actor, "person:stephen");
    assert_eq!(joined["member"]["key"], "person:stephen");
}

/// A row a space's log carried (tagged with its spaceId by the Core) only
/// touches that space's channels: never a Personal channel, a DM or another
/// space's channel, and a channel never moves space (card #461).
#[test]
fn a_space_row_only_touches_its_own_spaces_channels() {
    let t = Talk::new(None, None);
    let t0 = hlc_at(fixed_now());
    let ev = |seq: i64, kind: &str, p: serde_json::Value| {
        row("dev-b", seq, t0 + seq as u64, "user", kind, &p)
    };
    let made = |seq, id: &str, space: Option<&str>| {
        let mut p = serde_json::json!({"channelId": id, "name": id, "by": "user:dev-b"});
        if let Some(sp) = space {
            p["spaceId"] = sp.into();
        }
        ev(seq, kinds::TALK_CHANNEL_CREATED, p)
    };
    let post = |seq, conv: &str, space: &str| {
        ev(
            seq,
            kinds::TALK_POST,
            serde_json::json!({"postId": format!("p{seq}"), "convId": conv, "by": "user:dev-b", "text": "hi", "spaceId": space}),
        )
    };
    assert!(t.apply(&made(10, "ch_mine", None)));
    assert!(t.apply(&made(11, "ch_s1", Some("s1"))));

    assert!(t.apply(&post(20, "ch_s1", "s1")), "its own space's channel");
    assert!(!t.apply(&post(21, "ch_mine", "s1")), "a Personal channel");
    assert!(
        !t.apply(&post(22, "ch_s1", "s2")),
        "another space's channel"
    );
    // Card #538: a DM only through the stamp a hidden space's row gets
    // from the Core, and only the DM the stamp names.
    assert!(!t.apply(&post(23, "dm:a|b", "s1")), "a DM");
    let stamped = |seq, conv: &str, stamp: &str| {
        ev(
            seq,
            kinds::TALK_POST,
            serde_json::json!({
                "postId": format!("p{seq}"), "convId": conv, "by": "user:dev-b",
                "text": "hi", "spaceId": "s1", "dmConv": stamp
            }),
        )
    };
    assert!(
        !t.apply(&stamped(24, "dm:a|b", "dm:c|d")),
        "a stamp for another DM"
    );
    assert!(t.posts("ch_mine").is_empty() && t.posts("dm:a|b").is_empty());
    assert!(t.apply(&stamped(25, "dm:a|b", "dm:a|b")), "its own stamp");
    assert_eq!(t.posts("dm:a|b").len(), 1);
    assert_eq!(t.posts("ch_s1").len(), 1);

    let join = ev(
        30,
        kinds::TALK_MEMBER,
        serde_json::json!({"convId": "ch_mine", "member": {"kind": "agent", "key": BOB}, "op": "join", "by": "user:dev-b", "spaceId": "s1"}),
    );
    assert!(!t.apply(&join), "a member into a Personal channel");
    assert!(!t.is_member("ch_mine", BOB));
    let edit = ev(
        31,
        kinds::TALK_CHANNEL_EDITED,
        serde_json::json!({"channelId": "ch_mine", "name": "taken", "spaceId": "s1"}),
    );
    assert!(!t.apply(&edit));
    // An older copy of ch_mine "made in s1" doesn't move it there.
    assert!(!t.apply(&made(1, "ch_mine", Some("s1"))));
    let c = t.channel("ch_mine").unwrap();
    assert_eq!((c.name.as_str(), c.space_id.as_str()), ("ch_mine", ""));
    // A card link from s1 onto ch_mine's post doesn't land either.
    let mine = row(
        "dev-a",
        40,
        t0 + 40,
        "user",
        kinds::TALK_POST,
        &serde_json::json!({"postId": "pm", "convId": "ch_mine", "by": USER, "text": "mine"}),
    );
    assert!(t.apply(&mine));
    let card = ev(
        41,
        kinds::TALK_CARD,
        serde_json::json!({"postId": "pm", "taskId": "t1", "spaceId": "s1"}),
    );
    assert!(!t.apply(&card));
    assert!(t.posts("ch_mine")[0].card.is_none());
}

/// Two people make a card from one post while apart (card #558): the later
/// row is the link on every device, whichever order the rows arrive in.
#[test]
fn a_posts_card_link_follows_row_order_not_arrival() {
    let e = Env::new();
    let t = talk(&e);
    let c = t.create_channel(USER, "general", "").unwrap();
    let p = post(&t, &c.id, USER, "worth a card");
    let link = |dev: &str, hlc: u64, task: &str| store::Event {
        device_id: dev.into(),
        seq: 1,
        hlc: hlc::Timestamp(hlc),
        actor: USER.into(),
        kind: kinds::TALK_CARD.into(),
        payload: serde_json::to_vec(
            &serde_json::json!({"postId": p.post_id, "taskId": task, "by": USER}),
        )
        .unwrap(),
        sig: Vec::new(),
    };
    let t0 = hlc_at(fixed_now()) + (1 << 30);
    let (early, late) = (
        link("dev-b", t0, "t-early"),
        link("dev-c", t0 + 1, "t-late"),
    );
    assert!(t.apply(&late));
    assert!(!t.apply(&early), "an earlier row doesn't take the link");
    assert_eq!(t.posts(&c.id)[0].card.as_deref(), Some("t-late"));

    let other = talk(&e);
    assert!(other.apply(&early));
    assert!(other.apply(&late));
    assert_eq!(other.posts(&c.id)[0].card.as_deref(), Some("t-late"));
}

/// Space logs for person DMs (card #538): the Core's side, as far as Talk
/// sees it. `dm_for` picks `space`; each row comes back with the spaceId
/// and dmConv the Core stamps, keys as written.
struct PairSpaces {
    st: std::sync::Arc<store::Store>,
    space: std::sync::Mutex<Result<String, String>>,
    asked: std::sync::atomic::AtomicUsize,
    route: Option<String>,
    // Last: holds the dir `st` lives in; fields drop in order, and Windows
    // can't delete an open file.
    _env: Env,
}

impl PairSpaces {
    fn new(space: Result<String, String>, route: Option<String>) -> PairSpaces {
        let env = Env::on("dev-a");
        PairSpaces {
            st: env.st.clone(),
            space: std::sync::Mutex::new(space),
            asked: 0.into(),
            route,
            _env: env,
        }
    }
}

const ME: &str = "person:me";
const THEM: &str = "person:them";

impl PairSpaces {
    fn stamped(&self, mut ev: store::Event, space: &str) -> store::Event {
        let mut v: serde_json::Value = serde_json::from_slice(&ev.payload).unwrap();
        v["spaceId"] = space.into();
        v["dmConv"] = talk::pair_dm_id(ME, THEM).into();
        ev.payload = serde_json::to_vec(&v).unwrap();
        ev
    }
}

impl talk::SpaceLogs for PairSpaces {
    fn append(
        &self,
        space: &str,
        _by: &str,
        kind: &str,
        payload: serde_json::Value,
    ) -> Result<store::Event, String> {
        let ev = self
            .st
            .append("irori", kind, &payload)
            .map_err(|e| e.to_string())?;
        Ok(self.stamped(ev, space))
    }

    fn rows(&self) -> Vec<store::Event> {
        Vec::new()
    }

    fn person_id(&self) -> Option<String> {
        Some("me".into())
    }

    fn dm_for(&self, other: &str) -> Result<String, String> {
        assert_eq!(other, THEM);
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.space.lock().unwrap().clone()
    }

    fn dm_route(&self, conv: &str) -> Option<String> {
        self.route
            .clone()
            .filter(|k| talk::pair_dm_id(ME, k) == conv)
    }
}

fn pair_talk(e: &Env, space: Result<&str, &str>) -> (Talk, std::sync::Arc<PairSpaces>) {
    let t = talk(e);
    let sp = std::sync::Arc::new(PairSpaces::new(
        space.map(str::to_string).map_err(str::to_string),
        None,
    ));
    t.set_spaces(sp.clone());
    (t, sp)
}

/// Card #538: opening a DM with a person writes nothing and lists a draft;
/// the first post goes to the space `dm_for` picks, with both joins first.
#[test]
fn a_person_dm_opens_as_a_draft_and_its_first_post_finds_a_space() {
    let e = Env::new();
    let (t, sp) = pair_talk(&e, Ok("s1"));
    let (me, them) = (Member::of(USER), Member::of(THEM));
    let dm = t.open_dm(&me, &them, USER).unwrap();
    assert_eq!(dm, talk::pair_dm_id(ME, THEM));
    assert_eq!(t.open_dm(&them, &me, USER).unwrap(), dm, "either way round");

    assert!(kinds_in(&sp.st).is_empty() && kinds_in(&e.st).is_empty());
    assert_eq!(sp.asked.load(std::sync::atomic::Ordering::SeqCst), 0);
    let listed = t.dms();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].0, dm);
    assert!(t.is_member(&dm, USER) && t.is_member(&dm, THEM));
    t.load().unwrap();
    assert_eq!(t.dms().len(), 1, "a refold keeps a draft");

    let p = post(&t, &dm, USER, "hello");
    assert_eq!(p.space, "s1");
    assert_eq!(
        kinds_in(&sp.st),
        [kinds::TALK_MEMBER, kinds::TALK_MEMBER, kinds::TALK_POST]
    );
    assert!(kinds_in(&e.st).is_empty(), "nothing in the private log");
    let ms = t.members(&dm);
    assert_eq!(ms.len(), 2, "{ms:?}");
    assert!(ms.iter().all(|m| m.kind == talk::PERSON), "{ms:?}");

    // The next post in that log writes only the post.
    post(&t, &dm, USER, "again");
    assert_eq!(kinds_in(&sp.st).len(), 4);
    // A second log of the same DM gets its own joins.
    *sp.space.lock().unwrap() = Ok("s2".into());
    assert_eq!(post(&t, &dm, USER, "other log").space, "s2");
    assert_eq!(kinds_in(&sp.st).len(), 7);
}

/// Card #538: agents don't DM other people, a person DM has no member
/// row to leave by, and mentions never go in its rows.
#[test]
fn a_person_dm_takes_only_its_two_people() {
    let e = Env::new();
    let (t, sp) = pair_talk(&e, Ok("s1"));
    let them = Member::of(THEM);
    want(
        t.open_dm(&Member::of(ALICE), &them, ALICE),
        ErrorKind::NotAllowed,
    );
    let dm = t.open_dm(&Member::of(USER), &them, USER).unwrap();
    want(
        t.post(NewPost {
            conv_id: dm.clone(),
            text: "from an agent".into(),
            by: ALICE.into(),
            ..NewPost::default()
        }),
        ErrorKind::NotAllowed,
    );
    want(t.leave(&dm, &them, USER), ErrorKind::NotAllowed);
    assert!(kinds_in(&sp.st).is_empty());

    let p = t
        .post(NewPost {
            conv_id: dm.clone(),
            text: "hi @alpha".into(),
            mentions: vec![ALICE.into()],
            by: USER.into(),
            ..NewPost::default()
        })
        .unwrap();
    assert!(p.mentions.is_empty(), "{:?}", p.mentions);

    // An older DM with a board key stays what it was.
    let board = Member::of("board:b3f5/person:pSpencer");
    let old = t.open_dm(&Member::of(USER), &board, USER).unwrap();
    assert_eq!(old, talk::dm_id(USER, &board.key));
    assert!(kinds_in(&e.st).contains(&kinds::TALK_MEMBER.to_string()));
}

/// Card #538: with no shared space, `dm_for` refuses and nothing is written;
/// the draft stays, so the person can try again.
#[test]
fn a_person_dm_with_no_shared_space_is_refused_and_stays_a_draft() {
    let e = Env::new();
    let (t, sp) = pair_talk(&e, Err("you share no space with them"));
    let dm = t
        .open_dm(&Member::of(USER), &Member::of(THEM), USER)
        .unwrap();
    let err = want(
        t.post(NewPost {
            conv_id: dm.clone(),
            text: "hi".into(),
            by: USER.into(),
            ..NewPost::default()
        }),
        ErrorKind::NotAllowed,
    );
    assert!(err.to_string().contains("share no space"), "{err}");
    assert!(kinds_in(&sp.st).is_empty() && t.posts(&dm).is_empty());
    assert_eq!(t.dms().len(), 1);
}

/// Card #538: both people wrote first, so one DM spans two logs. Losing
/// one takes only its posts; losing the last takes the DM.
#[test]
fn dropping_a_person_dm_log_keeps_what_the_other_holds() {
    let e = Env::new();
    let (t, sp) = pair_talk(&e, Ok("s1"));
    let dm = t
        .open_dm(&Member::of(USER), &Member::of(THEM), USER)
        .unwrap();
    let first = post(&t, &dm, USER, "in s1");
    *sp.space.lock().unwrap() = Ok("s2".into());
    let second = post(&t, &dm, USER, "in s2");

    assert!(t.drop_space("s1"));
    let left: Vec<String> = t.posts(&dm).into_iter().map(|p| p.post_id).collect();
    assert_eq!(left, std::slice::from_ref(&second.post_id));
    assert!(!t.has_post(&first.post_id), "a re-join folds it back");
    assert_eq!(t.dms().len(), 1, "the other log still holds the DM");
    assert_eq!(t.members(&dm).len(), 2);

    assert!(t.drop_space("s2"));
    assert!(t.posts(&dm).is_empty());
    assert!(t.dms().is_empty(), "its last log took it");
    assert!(!t.drop_space("s2"));
}

/// Card #538: a device holding the space with no row folded yet still
/// knows who the DM is with.
#[test]
fn a_person_dm_is_found_by_its_space_before_any_row_folds() {
    let e = Env::new();
    let t = talk(&e);
    t.set_spaces(std::sync::Arc::new(PairSpaces::new(
        Ok("s1".into()),
        Some(THEM.into()),
    )));
    let dm = talk::pair_dm_id(ME, THEM);
    assert_eq!(t.pair_other(&dm).as_deref(), Some(THEM));
    assert_eq!(post(&t, &dm, USER, "hi").space, "s1");
    assert_eq!(t.pair_other(&talk::dm_id(USER, ALICE)), None);
}

fn file(n: u8, name: &str) -> blob::FileRef {
    blob::FileRef {
        sha256: format!("{:02x}", n).repeat(32),
        name: name.into(),
        size: 42,
        mime: "image/png".into(),
        device: "dev-a".into(),
    }
}

/// Card #643: a post carries files; they fold, refold from the store, and
/// are found by hash. With files the text may be empty, never without.
#[test]
fn posts_carry_files() {
    let e = Env::new();
    let t = talk(&e);
    let c = t.create_channel(USER, "dev", "").unwrap();
    let shot = file(1, "shot.png");
    let p = t
        .post(NewPost {
            conv_id: c.id.clone(),
            text: "  ".into(),
            by: USER.into(),
            files: vec![shot.clone(), shot.clone(), file(2, "log.txt")],
            ..NewPost::default()
        })
        .unwrap();
    assert_eq!(p.text, "", "files alone make a post");
    assert_eq!(p.files.len(), 2, "the same file goes once");
    assert_eq!(p.files[0], shot);
    let dm = t
        .open_dm(&Member::of(USER), &Member::of(BOB), USER)
        .unwrap();
    t.post(NewPost {
        conv_id: dm.clone(),
        text: "again".into(),
        by: USER.into(),
        files: vec![shot.clone()],
        ..NewPost::default()
    })
    .unwrap();
    assert_eq!(
        t.attachments(&shot.sha256)
            .into_iter()
            .map(|a| (a.conv, a.file))
            .collect::<Vec<_>>(),
        vec![(c.id.clone(), shot.clone()), (dm.clone(), shot.clone())],
        "earliest first"
    );
    assert!(t.attachments(&"ff".repeat(32)).is_empty());

    // The row names them, and a refold from the store keeps them.
    let rows = e.rows(kinds::TALK_POST);
    let v: serde_json::Value = serde_json::from_slice(&rows[0].payload).unwrap();
    assert_eq!(v["attach"][0]["sha256"], shot.sha256);
    let t2 = talk(&e);
    assert_eq!(t2.posts(&c.id), t.posts(&c.id));
    assert_eq!(t2.attachments(&shot.sha256).len(), 2);

    // An old row without "attach" still folds, with no files.
    let old = row(
        "dev-b",
        1,
        hlc_at(fixed_now()) + 10,
        &format!("agent:{ALICE}"),
        kinds::TALK_POST,
        &serde_json::json!({"postId": "p_old", "convId": c.id, "text": "old", "mentions": [], "agentsNow": true, "by": ALICE, "at": fixed_now()}),
    );
    assert!(t2.apply(&old));
    let got = t2.posts(&c.id);
    let o = got.iter().find(|p| p.post_id == "p_old").unwrap();
    assert!(o.files.is_empty());

    // Refused: no text and no files, too many files, a bad hash.
    let bare = |files: Vec<blob::FileRef>, text: &str| {
        t.post(NewPost {
            conv_id: c.id.clone(),
            text: text.into(),
            by: USER.into(),
            files,
            ..NewPost::default()
        })
    };
    want(bare(vec![], " "), ErrorKind::Invalid);
    let many: Vec<blob::FileRef> = (0..=coord::board::MAX_ATTACHMENTS as u8)
        .map(|i| file(10 + i, "f"))
        .collect();
    want(bare(many, "too many"), ErrorKind::Invalid);
    let mut bad = file(3, "bad");
    bad.sha256 = "nope".into();
    want(bare(vec![bad], "bad"), ErrorKind::Invalid);
    assert_eq!(t.posts(&c.id).len(), 1, "nothing refused was written");
}

fn call_post(t: &Talk, conv: &str, mark: talk::CallMark) -> coord::Result<talk::Post> {
    t.post(NewPost {
        conv_id: conv.into(),
        text: "a huddle".into(),
        by: USER.into(),
        call: Some(mark),
        ..NewPost::default()
    })
}

fn post_payload(e: &Env, post_id: &str) -> serde_json::Value {
    e.rows(kinds::TALK_POST)
        .iter()
        .map(|ev| serde_json::from_slice::<serde_json::Value>(&ev.payload).unwrap())
        .find(|v| v["postId"] == post_id)
        .expect("the post's row")
}

/// #654: a huddle's mark goes in the row and folds into the post, the same
/// after a reload; a post without one carries none.
#[test]
fn a_call_mark_is_written_and_folded() {
    let e = Env::new();
    let t = talk(&e);
    let c = t.create_channel(USER, "dev", "").unwrap();
    let end = talk::CallMark {
        call_id: " call_1 ".into(),
        op: talk::CALL_END.into(),
        duration_ms: 61_000,
        people: vec![USER.into(), ALICE.into(), USER.into(), " ".into()],
        transcript: false,
    };
    let p = call_post(&t, &c.id, end).unwrap();
    let want_mark = talk::CallMark {
        call_id: "call_1".into(),
        op: talk::CALL_END.into(),
        duration_ms: 61_000,
        people: vec![USER.into(), ALICE.into()],
        transcript: false,
    };
    assert_eq!(p.call.as_ref(), Some(&want_mark));
    assert_eq!(
        post_payload(&e, &p.post_id)["call"],
        serde_json::json!({"callId": "call_1", "op": "end", "durationMs": 61000, "people": [USER, ALICE]})
    );
    let plain = post(&t, &c.id, USER, "hi");
    assert_eq!(plain.call, None);
    let v = post_payload(&e, &plain.post_id);
    assert!(v.get("call").is_none(), "{v}");

    // A start carries no length and no people.
    let s = call_post(
        &t,
        &c.id,
        talk::CallMark {
            call_id: "call_2".into(),
            op: talk::CALL_START.into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        post_payload(&e, &s.post_id)["call"],
        serde_json::json!({"callId": "call_2", "op": "start"})
    );

    let t2 = talk(&e);
    assert_eq!(t2.posts(&c.id), t.posts(&c.id));
    assert_eq!(t2.posts(&c.id)[0].call.as_ref(), Some(&want_mark));
    assert_eq!(t2.posts(&c.id)[1].call, None);

    // A mark with no id or an unknown op is refused.
    let no_id = talk::CallMark {
        op: talk::CALL_START.into(),
        ..Default::default()
    };
    want(call_post(&t, &c.id, no_id), ErrorKind::Invalid);
    let ring = talk::CallMark {
        call_id: "call_3".into(),
        op: "ring".into(),
        ..Default::default()
    };
    want(call_post(&t, &c.id, ring), ErrorKind::Invalid);
}

/// #654: a reader that knows nothing of a field still reads the post: a
/// row with a `call` object, and one with an object nobody knows, decode
/// as plain posts.
#[test]
fn a_post_row_with_objects_it_does_not_know_still_decodes() {
    let raw = serde_json::json!({
        "postId": "post_1",
        "convId": "ch_1",
        "text": "started a huddle",
        "mentions": [],
        "agentsNow": false,
        "by": USER,
        "call": {"callId": "call_1", "op": "start", "ring": {"devices": 3}},
        "later": {"what": ["ever"], "n": 1}
    });
    let r: talk::PostRow = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(r.post_id, "post_1");
    assert_eq!(r.conv_id, "ch_1");
    assert_eq!(r.text, "started a huddle");
    assert_eq!(r.by, USER);
    let c = r.call.unwrap();
    assert_eq!((c.call_id.as_str(), c.op.as_str()), ("call_1", "start"));

    // A mark of a shape this version can't read drops the mark, never the
    // post.
    for bad in [
        serde_json::json!({"callId": "call_1", "op": "end", "durationMs": "long"}),
        serde_json::json!({"callId": "call_1", "people": [{"key": "x"}]}),
        serde_json::json!("not an object"),
    ] {
        let mut row = raw.clone();
        row["call"] = bad;
        let r: talk::PostRow = serde_json::from_value(row).unwrap();
        assert_eq!(r.text, "started a huddle");
        assert!(r.call.is_none());
    }

    // As a reader from before #654 has it: no call field at all.
    #[derive(Default, serde::Deserialize)]
    #[serde(rename_all = "camelCase", default)]
    struct Before {
        post_id: String,
        text: String,
        mentions: Vec<String>,
        thread_from: Option<String>,
    }
    let old: Before = serde_json::from_value(raw).unwrap();
    assert_eq!(old.post_id, "post_1");
    assert_eq!(old.text, "started a huddle");
    assert!(old.mentions.is_empty() && old.thread_from.is_none());

    // And a row from another device folds with it.
    let e = Env::new();
    let t = talk(&e);
    let ch = t.create_channel(USER, "dev", "").unwrap();
    let row = serde_json::json!({
        "postId": "post_9",
        "convId": ch.id,
        "text": "ended a huddle",
        "by": ALICE,
        "call": {"callId": "call_1", "op": "end", "durationMs": 5, "people": [ALICE]},
        "later": {"n": 1}
    });
    e.insert("dev-b", 1, 1, ALICE, kinds::TALK_POST, &row);
    t.load().unwrap();
    let p = &t.posts(&ch.id)[0];
    assert_eq!(p.text, "ended a huddle");
    assert_eq!(p.call.as_ref().unwrap().people, vec![ALICE.to_string()]);
}

// ---- a huddle's transcript (card #656) -------------------------------------

const T0: i64 = 1_762_470_000_000;

fn line(t: &Talk, conv: &str, seg: i64, start: i64, text: &str) -> coord::Result<talk::Post> {
    t.write_segment(
        conv,
        "call_1",
        seg,
        T0 + start,
        T0 + start + 900,
        text,
        USER,
    )
}

fn texts(ps: &[talk::Post]) -> Vec<&str> {
    ps.iter().map(|p| p.text.as_str()).collect()
}

/// `call.segment` rows fold into `call:<id>`, by speaker at the phrase's
/// time, whichever order they arrive in, each (call, device, seg) once; a
/// fresh fold reads the same.
#[test]
fn segments_fold_into_call_posts_in_order() {
    let e = Env::new();
    let t = talk(&e);
    let c = t.create_channel(USER, "dev", "").unwrap();
    let p = line(&t, &c.id, 1, 3000, " second ").unwrap();
    assert_eq!(p.conv_id, "call:call_1");
    assert_eq!((p.by.as_str(), p.text.as_str()), (USER, "second"));
    let ms = T0 + 3000;
    assert_eq!(
        p.at,
        gocompat::time::GoTime::unix(ms / 1000, (ms % 1000) * 1_000_000)
    );
    line(&t, &c.id, 0, 0, "first").unwrap();
    // Alice's device writes its own lines, one twice.
    let theirs = |seq: i64, seg: i64, start: i64, text: &str| {
        row(
            "dev-b",
            seq,
            hlc_at(fixed_now()) + 100 + seq as u64,
            "user",
            kinds::CALL_SEGMENT,
            &serde_json::json!({
                "callId": "call_1", "convId": c.id, "seg": seg, "startMs": T0 + start,
                "endMs": T0 + start + 500, "text": text, "by": ALICE,
            }),
        )
    };
    assert!(t.apply(&theirs(1, 0, 1000, "between")));
    assert!(t.apply(&theirs(2, 1, 3000, "same time")));
    assert!(
        !t.apply(&theirs(3, 1, 9000, "again")),
        "seg 1 of dev-b again"
    );
    // dev-b's seg 0 isn't dev-a's seg 0.
    assert_eq!(
        texts(&t.call_posts("call_1")),
        ["first", "between", "same time", "second"],
        "by time, then by speaker"
    );
    assert!(
        t.call_posts("call_1")
            .iter()
            .all(|p| p.conv_id == "call:call_1")
    );
    assert!(talk::is_call("call:call_1") && !talk::is_call(&c.id));
    assert_eq!(t.call_conv("call_1").as_deref(), Some(c.id.as_str()));
    assert_eq!(t.call_conv("nope"), None);
    assert!(
        t.posts(&c.id).is_empty(),
        "lines aren't the channel's posts"
    );

    // A line of the same call naming another conversation is dropped.
    let other = t.create_channel(USER, "other", "").unwrap();
    assert!(!t.apply(&row(
        "dev-b",
        4,
        hlc_at(fixed_now()) + 200,
        "user",
        kinds::CALL_SEGMENT,
        &serde_json::json!({"callId": "call_1", "convId": other.id, "seg": 7, "startMs": T0, "endMs": T0, "text": "x", "by": ALICE}),
    )));

    // Marks join the listing: the end says it has a transcript.
    let end = talk::CallMark {
        call_id: "call_1".into(),
        op: talk::CALL_END.into(),
        duration_ms: 9000,
        transcript: true,
        ..Default::default()
    };
    let ended = call_post(&t, &c.id, end.clone()).unwrap();
    assert_eq!(post_payload(&e, &ended.post_id)["call"]["transcript"], true);
    assert_eq!(t.posts(&c.id).len(), 1, "the end post stays in the channel");
    let calls = t.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].call_id, "call_1");
    assert_eq!(calls[0].conv_id, c.id);
    assert_eq!(calls[0].end.as_ref(), Some(&end));
    assert_eq!(calls[0].lines, 4);
    assert_eq!(calls[0].last, ended.at);

    // The person's own lines reload from the store; Alice's rows were
    // only handed over.
    let t2 = talk(&e);
    assert_eq!(texts(&t2.call_posts("call_1")), ["first", "second"]);
    assert_eq!(t2.calls()[0].end.as_ref(), Some(&end));

    // Refused: no words, no call, no such channel, a DM with an agent.
    want(line(&t, &c.id, 2, 0, "  "), ErrorKind::Invalid);
    want(
        t.write_segment(&c.id, " ", 0, T0, T0, "x", USER),
        ErrorKind::Invalid,
    );
    want(line(&t, "ch_nope", 2, 0, "x"), ErrorKind::NotFound);
    let dm = t
        .open_dm(&Member::of(USER), &Member::of(ALICE), USER)
        .unwrap();
    want(line(&t, &dm, 2, 0, "x"), ErrorKind::Invalid);
    want(line(&t, "call:call_1", 2, 0, "x"), ErrorKind::Invalid);
}

/// The fallback: one post carrying every line folds into `call:<id>` by its
/// writer, isn't listed in the channel, and an older reader still sees a
/// plain post.
#[test]
fn a_fallback_transcript_folds_and_is_hidden_from_the_channel() {
    let e = Env::new();
    let t = talk(&e);
    let c = t.create_channel(USER, "dev", "").unwrap();
    let before = post(&t, &c.id, USER, "hi");
    let mark = talk::TranscriptMark {
        call_id: " call_1 ".into(),
        lines: vec![
            talk::TranscriptLineMark {
                start_ms: T0 + 4000,
                end_ms: T0 + 5000,
                text: "later".into(),
            },
            talk::TranscriptLineMark {
                start_ms: T0,
                end_ms: T0 + 900,
                text: " sooner ".into(),
            },
            talk::TranscriptLineMark {
                start_ms: T0 + 6000,
                end_ms: T0,
                text: "  ".into(),
            },
        ],
    };
    let p = t
        .post(NewPost {
            conv_id: c.id.clone(),
            text: "0:00 sooner\n0:04 later".into(),
            by: USER.into(),
            transcript: Some(mark),
            ..NewPost::default()
        })
        .unwrap();
    let kept = p.transcript.clone().unwrap();
    assert_eq!(kept.call_id, "call_1");
    assert_eq!(kept.lines.len(), 2, "the empty line is dropped");
    assert_eq!(
        post_payload(&e, &p.post_id)["transcript"]["lines"][1],
        serde_json::json!({"startMs": T0, "endMs": T0 + 900, "text": "sooner"})
    );
    assert!(
        post_payload(&e, &before.post_id)
            .get("transcript")
            .is_none()
    );

    let ids: Vec<String> = t.posts(&c.id).into_iter().map(|p| p.post_id).collect();
    assert_eq!(
        ids,
        std::slice::from_ref(&before.post_id),
        "not in the channel"
    );
    assert!(t.has_post(&p.post_id));
    let lines = t.call_posts("call_1");
    assert_eq!(texts(&lines), ["sooner", "later"]);
    assert!(
        lines
            .iter()
            .all(|l| l.by == USER && l.conv_id == "call:call_1")
    );
    assert_eq!(t.call_conv("call_1").as_deref(), Some(c.id.as_str()));
    assert_eq!(t.calls()[0].lines, 2);

    let t2 = talk(&e);
    assert_eq!(t2.call_posts("call_1"), lines);
    assert_eq!(t2.posts(&c.id).len(), 1);

    // An older reader, and a mark this version can't read: a plain post.
    let mut raw = post_payload(&e, &p.post_id);
    raw["transcript"] = serde_json::json!({"callId": "call_2", "lines": "lots"});
    let r: talk::PostRow = serde_json::from_value(raw).unwrap();
    assert!(r.transcript.is_none());
    assert_eq!(r.text, "0:00 sooner\n0:04 later");

    // A transcript with no call or no lines is refused.
    for bad in [
        talk::TranscriptMark {
            call_id: "call_1".into(),
            lines: vec![],
        },
        talk::TranscriptMark {
            call_id: " ".into(),
            lines: kept.lines.clone(),
        },
    ] {
        want(
            t.post(NewPost {
                conv_id: c.id.clone(),
                text: "x".into(),
                by: USER.into(),
                transcript: Some(bad),
                ..NewPost::default()
            }),
            ErrorKind::Invalid,
        );
    }
}

/// A space's huddle lines go in its log and leave the fold with it, and
/// fold back when it returns.
#[test]
fn dropping_a_space_takes_its_huddles_out() {
    let (e, sp) = (Env::new(), Env::on("dev-a"));
    let space = std::sync::Arc::new(FakeSpace {
        st: sp.st.clone(),
        ready: true.into(),
    });
    let t = talk(&e);
    t.set_spaces(space.clone());
    let mine = t.create_channel(USER, "notes", "").unwrap();
    t.write_segment(&mine.id, "call_p", 0, T0, T0 + 1, "personal", USER)
        .unwrap();
    let c = t.create_channel_in(USER, "netcode", "", "s1").unwrap();
    call_post(
        &t,
        &c.id,
        talk::CallMark {
            call_id: "call_1".into(),
            op: talk::CALL_START.into(),
            ..Default::default()
        },
    )
    .unwrap();
    line(&t, &c.id, 0, 0, "in the space").unwrap();
    let fb = t
        .post(NewPost {
            conv_id: c.id.clone(),
            text: "0:01 kept back".into(),
            by: USER.into(),
            transcript: Some(talk::TranscriptMark {
                call_id: "call_1".into(),
                lines: vec![talk::TranscriptLineMark {
                    start_ms: T0 + 1000,
                    end_ms: T0 + 2000,
                    text: "kept back".into(),
                }],
            }),
            ..NewPost::default()
        })
        .unwrap();
    assert!(kinds_in(&sp.st).contains(&kinds::CALL_SEGMENT.to_string()));
    assert!(
        !kinds_in(&e.st).contains(&kinds::TALK_POST.to_string()),
        "Personal's line is a segment, not a post"
    );
    assert_eq!(
        texts(&t.call_posts("call_1")),
        ["in the space", "kept back"]
    );
    assert_eq!(t.calls().len(), 2);

    assert!(t.drop_space("s1"));
    assert!(t.call_posts("call_1").is_empty());
    assert_eq!(t.call_conv("call_1"), None);
    assert!(!t.has_post(&fb.post_id));
    let left: Vec<String> = t.calls().into_iter().map(|c| c.call_id).collect();
    assert_eq!(left, ["call_p"], "Personal's huddle stays");
    assert_eq!(texts(&t.call_posts("call_p")), ["personal"]);

    for ev in talk::SpaceLogs::rows(&*space) {
        t.apply(&ev);
    }
    assert_eq!(
        texts(&t.call_posts("call_1")),
        ["in the space", "kept back"]
    );
}

/// A person DM's huddle lines go to the DM's space, by the person only.
#[test]
fn a_person_dm_huddle_line_goes_to_its_space() {
    let e = Env::new();
    let (t, sp) = pair_talk(&e, Ok("s1"));
    let dm = t
        .open_dm(&Member::of(USER), &Member::of(THEM), USER)
        .unwrap();
    let p = line(&t, &dm, 0, 0, "just us").unwrap();
    assert_eq!(p.conv_id, "call:call_1");
    assert_eq!(t.call_conv("call_1"), Some(dm.clone()));
    assert_eq!(
        kinds_in(&sp.st),
        [kinds::TALK_MEMBER, kinds::TALK_MEMBER, kinds::CALL_SEGMENT],
        "both joins first, as a first post"
    );
    assert!(kinds_in(&e.st).is_empty());
    want(
        t.write_segment(&dm, "call_1", 1, T0, T0, "x", ALICE),
        ErrorKind::NotAllowed,
    );
}

/// What a folded mark or line can't be: a mark with no call or no words
/// leaves a plain post, an oversized line is skipped, and only a person
/// writes a line.
#[test]
fn a_bad_transcript_mark_or_line_folds_as_little_as_it_says() {
    let e = Env::new();
    let t = talk(&e);
    let c = t.create_channel(USER, "dev", "").unwrap();
    let big = "x".repeat(coord::board::MAX_PERSON_TEXT + 1);
    let fallback = |seq: i64, mark: serde_json::Value| {
        row(
            "dev-b",
            seq,
            hlc_at(fixed_now()) + seq as u64,
            "user",
            kinds::TALK_POST,
            &serde_json::json!({
                "postId": format!("fb{seq}"), "convId": c.id, "text": "0:00 hi",
                "by": ALICE, "at": fixed_now(), "transcript": mark,
            }),
        )
    };
    let l = |text: &str| serde_json::json!({"startMs": T0, "endMs": T0, "text": text});
    assert!(t.apply(&fallback(1, serde_json::json!({"lines": [l("hi")]}))));
    assert!(t.apply(&fallback(
        2,
        serde_json::json!({"callId": "call_9", "lines": [l("  "), l(&big)]})
    )));
    assert!(t.apply(&fallback(
        3,
        serde_json::json!({"callId": "call_9", "lines": [l(" kept "), l(&big)]})
    )));
    let plain = t.posts(&c.id);
    let ids: Vec<&str> = plain.iter().map(|p| p.post_id.as_str()).collect();
    assert_eq!(ids, ["fb1", "fb2"], "no call or no words: a plain post");
    assert!(plain.iter().all(|p| p.transcript.is_none()));
    assert_eq!(texts(&t.call_posts("call_9")), ["kept"]);
    assert_eq!(t.calls()[0].lines, 1, "the oversized line is gone");

    let seg = |seq: i64, text: &str| {
        row(
            "dev-b",
            seq,
            hlc_at(fixed_now()) + seq as u64,
            "user",
            kinds::CALL_SEGMENT,
            &serde_json::json!({
                "callId": "call_9", "convId": c.id, "seg": seq, "startMs": T0,
                "endMs": T0, "text": text, "by": ALICE,
            }),
        )
    };
    assert!(!t.apply(&seg(4, &big)));
    assert!(t.apply(&seg(5, " fine ")));
    assert_eq!(texts(&t.call_posts("call_9")), ["kept", "fine"]);

    want(
        t.write_segment(&c.id, "call_9", 0, T0, T0, "x", ALICE),
        ErrorKind::NotAllowed,
    );
}

/// A fallback transcript rings in a DM only when it came after where the
/// reader has read to.
#[test]
fn a_fallback_rings_only_past_the_cursor() {
    let e = Env::new();
    let t = talk(&e);
    let reader = "user:phone";
    let dm = t
        .open_dm(&Member::of(USER), &Member::of(ALICE), USER)
        .unwrap();
    let before = post(&t, &dm, ALICE, "before");
    e.clk.add(secs(1));
    let fb = t
        .post(NewPost {
            conv_id: dm.clone(),
            text: "0:00 hi".into(),
            by: ALICE.into(),
            transcript: Some(talk::TranscriptMark {
                call_id: "call_1".into(),
                lines: vec![talk::TranscriptLineMark {
                    start_ms: T0,
                    end_ms: T0,
                    text: "hi".into(),
                }],
            }),
            ..NewPost::default()
        })
        .unwrap();
    e.clk.add(secs(1));
    let after = post(&t, &dm, ALICE, "after");
    assert!(t.notifies(&dm, &fb.post_id, reader).is_some(), "no cursor");
    t.mark_read(&dm, &before.post_id, USER).unwrap();
    assert!(
        t.notifies(&dm, &fb.post_id, reader).is_some(),
        "read up to an older post"
    );
    t.mark_read(&dm, &after.post_id, USER).unwrap();
    assert!(
        t.notifies(&dm, &fb.post_id, reader).is_none(),
        "read past it"
    );
}
