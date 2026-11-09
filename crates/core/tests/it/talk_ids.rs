//! Card #1144 (docs/design/list-signals.md §3.2): every Talk change names
//! what it touched, so the window reads those conversations and calls
//! again, never everything. One core, a Personal channel and its huddle;
//! a site that names its ids never says `all`.

use std::time::Duration;

use coord::talk::{CALL_START, CallMark};
use irori_core::{CoreEvent, Feed, TalkChange, Topic};

use crate::common::*;

/// The huddle the channel's lines make (a 32-hex call id, as a real one).
const CALL: &str = "00112233445566778899aabbccddeeff";
/// The huddle the start mark names.
const MARKED: &str = "ffeeddccbbaa99887766554433221100";

/// Every Talk change seen in the test, to check none was `all`.
struct Seen {
    feed: Feed,
    all: Vec<TalkChange>,
}

impl Seen {
    fn take(&mut self, ev: CoreEvent) -> Option<TalkChange> {
        match ev {
            CoreEvent::Talk(t) => {
                self.all.push(t.clone());
                Some(t)
            }
            CoreEvent::Resync => panic!("the Talk feed lost changes: {:?}", self.all),
            _ => None,
        }
    }

    /// Reads until the feed is quiet for a moment: what an earlier step
    /// still had coming can't pass for the next one's.
    fn quiet(&mut self) {
        while let Ok(Some(ev)) = self.feed.recv_timeout(Duration::from_millis(400)) {
            self.take(ev);
        }
    }

    /// The changes already in the feed: a site that publishes itself has
    /// published by the time its call returns.
    fn now(&mut self) -> Vec<TalkChange> {
        let mut out = Vec::new();
        while let Some(ev) = self.feed.try_recv() {
            out.extend(self.take(ev));
        }
        out
    }

    /// Waits for a change `ok` takes (a row the store follower folds).
    fn until(&mut self, what: &str, ok: impl Fn(&TalkChange) -> bool) -> TalkChange {
        let mut found = None;
        wait_for(20, what, || {
            while let Some(ev) = self.feed.try_recv() {
                if let Some(t) = self.take(ev)
                    && found.is_none()
                    && ok(&t)
                {
                    found = Some(t);
                }
            }
            found.is_some()
        });
        found.unwrap()
    }

    /// No change in the test said `all`.
    fn none_all(&mut self) {
        self.quiet();
        assert!(!self.all.is_empty());
        for t in &self.all {
            assert!(
                !t.all && !t.is_all(),
                "a change named nothing: {:?}",
                self.all
            );
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// list-signals.md §3.2 (card #1144): a channel, its post, a huddle's
/// line, start mark, title and canvas item, a thread's read cursor and a
/// draft DM each publish a Talk change naming their conversation, and
/// their call where they have one; none of them says `all`. A store
/// resync (the one `all` the follower sends) isn't driven here: it needs
/// the store's subscription to overflow.
#[test]
fn every_talk_change_names_its_conversation_and_call() {
    let r = Rig::new();
    let me = r.core.user_sender();
    let mut s = Seen {
        feed: r.core.feed(&[Topic::Talk]),
        all: Vec::new(),
    };

    // A channel: its created and member rows, through the follower.
    let ch = r
        .core
        .talk_channel_create(&me.key, "general", "the room")
        .unwrap();
    s.until("the channel's change", |t| t.convs.contains(&ch.id));
    s.quiet();

    // A post in it, through the follower.
    r.core
        .talk_post(&me, &ch.id, "hello room", &[], false, None, vec![])
        .unwrap();
    let t = s.until("the post's change", |t| t.convs.contains(&ch.id));
    assert!(t.calls.is_empty(), "{t:?}");
    s.quiet();

    // A huddle's line: the call.segment row names the conv and the call.
    let at = now_ms();
    r.core
        .talk()
        .write_segment(&ch.id, CALL, 0, at, at + 900, "to the room", &me.key)
        .unwrap();
    let t = s.until("the line's change", |t| t.calls.iter().any(|c| c == CALL));
    assert_eq!(t.convs, vec![ch.id.clone()], "{t:?}");
    s.quiet();

    // A huddle's start mark: published as it is written.
    let p = r
        .core
        .post_call_mark(
            &ch.id,
            "started a huddle",
            CallMark {
                call_id: MARKED.into(),
                op: CALL_START.into(),
                ..CallMark::default()
            },
        )
        .unwrap();
    assert_eq!(p.conv_id, ch.id);
    let got = s.now();
    assert!(
        got.contains(&TalkChange::of([ch.id.as_str()], [MARKED])),
        "post_call_mark names the channel and the call: {got:?}"
    );
    s.quiet();

    // Its title.
    r.core.call_title_set(CALL, "Notes").unwrap();
    let got = s.now();
    assert!(
        got.contains(&TalkChange::of([ch.id.as_str()], [CALL])),
        "call_title_set names the call and its channel: {got:?}"
    );
    s.quiet();

    // A canvas item.
    r.core
        .huddle_item_add(CALL, "note", "bring the logs", "")
        .unwrap();
    let got = s.now();
    assert!(
        got.contains(&TalkChange::of([ch.id.as_str()], [CALL])),
        "huddle_item_add names the call and its channel: {got:?}"
    );
    s.quiet();

    // A thread's read cursor keeps the window's `thread:` id, through the
    // follower.
    r.core.talk_mark_read("thread:T", "p_1", &me.key).unwrap();
    let t = s.until("the read cursor's change", |t| {
        t.convs.iter().any(|c| c == "thread:T")
    });
    assert_eq!(t, TalkChange::convs(["thread:T"]));
    s.quiet();

    // A draft DM writes no row: talk_dm says so itself.
    let dm = r.core.talk_dm(&me.key, &r.key_a).unwrap();
    let got = s.now();
    assert!(
        got.contains(&TalkChange::convs([dm.as_str()])),
        "talk_dm names the DM it opened: {got:?}"
    );

    s.none_all();
    r.core.close().unwrap();
}
