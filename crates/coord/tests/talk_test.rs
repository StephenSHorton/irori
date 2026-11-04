//! Talk's fold (card #389): channels, members, posts, DMs and read
//! cursors over `talk.*` rows, written through the store and folded again
//! from it.

mod common;

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
                kinds: vec!["talk.".into()],
                ..Default::default()
            })
            .unwrap()
    }
}

fn kinds_in(st: &store::Store) -> Vec<String> {
    st.list(&store::Filter {
        kinds: vec!["talk.".into()],
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
    // Personal channels and DMs are never gated.
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
