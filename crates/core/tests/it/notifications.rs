//! The person's notification history (card #377): a notification the
//! desktop shows is kept once per thing it is about, kept with the popups
//! off, read through the Core, and still there after a restart.

use crate::common;

use std::sync::{Arc, Mutex};

use common::*;
use coord::NewTask;
use irori_core::Core;

/// Records the cards a Notifier posts.
#[derive(Default)]
struct Cards(Mutex<Vec<toast::Note>>);

impl notify::Toaster for Cards {
    fn configure(&self, _: toast::Config) {}
    fn set_options(&self, _: toast::Options) {}
    fn post(&self, n: toast::Note) {
        self.0.lock().unwrap().push(n);
    }
    fn close(&self, _: &str) {}
}

fn notifier() -> (Arc<Cards>, notify::Notifier) {
    let cards = Arc::new(Cards::default());
    let n = notify::Notifier::new(notify::Config {
        toaster: Some(cards.clone()),
        ..notify::Config::default()
    });
    (cards, n)
}

#[test]
fn a_blocker_is_kept_once_read_and_survives_a_restart() {
    let (cards, n) = notifier();
    let r = Rig::with(|o| {
        o.headless = false;
        o.notifier = Some(n.clone());
    });
    let co = r.core.coord();
    let task = co
        .create_task(
            &r.key_a,
            NewTask {
                title: "Pick a colour".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &task.id).unwrap();
    co.block(
        &r.key_a,
        &task.id,
        "Blue or green?",
        Some(coord::BlockedBy {
            kind: coord::BLOCKED_ON_PERSON.into(),
            reference: String::new(),
        }),
    )
    .unwrap();
    wait_for(10, "the blocker's notification", || {
        r.core.notifications().len() == 1
    });
    let it = &r.core.notifications()[0];
    let b = co.get_task(&task.id).unwrap().open_blockers()[0].id.clone();
    assert_eq!(it.posted.kind, coord::notifications::KIND_BLOCKER, "{it:?}");
    assert_eq!(it.posted.task_id, task.id);
    assert_eq!(it.posted.blocker_id, b);
    assert!(it.posted.title.contains("blocked on you"), "{it:?}");
    assert!(!it.read);
    assert_eq!(r.core.unread_notifications(), 1);
    assert!(
        cards
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.id.starts_with("needs:"))
    );

    // Shown again (as another desktop would): still one item.
    n.post(notify::Note {
        id: notify::needs_id(&task.id),
        title: "again".into(),
        about: Some(coord::notifications::PostedRow {
            kind: coord::notifications::KIND_BLOCKER.into(),
            task_id: task.id.clone(),
            blocker_id: b.clone(),
            ..Default::default()
        }),
        ..notify::Note::default()
    });
    assert_eq!(r.core.notifications().len(), 1);
    assert!(
        r.core.notifications()[0]
            .posted
            .title
            .contains("blocked on you")
    );

    // With the popups off a notification is still kept.
    let mut o = n.options();
    o.enabled = false;
    let _ = n.set_options(o);
    assert!(!n.post(notify::Note {
        id: "notify:x".into(),
        title: "alpha".into(),
        body: "the build is green".into(),
        about: Some(coord::notifications::PostedRow {
            kind: coord::notifications::KIND_AGENT.into(),
            from: r.key_a.clone(),
            ..Default::default()
        }),
        ..notify::Note::default()
    }));
    let all = r.core.notifications();
    assert_eq!(all.len(), 2, "{all:?}");
    assert_eq!(
        all[0].posted.kind,
        coord::notifications::KIND_AGENT,
        "newest first"
    );
    assert_eq!(r.core.unread_notifications(), 2);

    // Reading one, then all.
    r.core.read_notifications(&[all[1].key.clone()]).unwrap();
    assert_eq!(r.core.unread_notifications(), 1);
    assert!(r.core.notifications()[1].read);
    r.core.read_notifications(&[]).unwrap();
    assert_eq!(r.core.unread_notifications(), 0);

    // A restart: the history and what was read come back from the log.
    r.core.close().unwrap();
    let mut opts = options(&r.data, &r.claude);
    opts.headless = false;
    opts.notifier = Some(notifier().1);
    let again = Core::start(opts).unwrap();
    let back = again.notifications();
    assert_eq!(back.len(), 2, "{back:?}");
    assert!(back.iter().all(|i| i.read), "{back:?}");
    assert_eq!(back[1].posted.blocker_id, b);
    again.close().unwrap();
}

/// Card #482: once what an item asks is settled, by the person or the
/// agent, it reads by itself and leaves the badge.
#[test]
fn a_resolved_blocker_or_done_check_leaves_the_badge_by_itself() {
    let (_cards, n) = notifier();
    let r = Rig::with(|o| {
        o.headless = false;
        o.notifier = Some(n.clone());
    });
    let co = r.core.coord();
    let person = r.core.user_sender().key;
    let task = co
        .create_task(
            &r.key_a,
            NewTask {
                title: "Pick a colour".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &task.id).unwrap();
    let on = || {
        Some(coord::BlockedBy {
            kind: coord::BLOCKED_ON_PERSON.into(),
            reference: String::new(),
        })
    };
    let unread_of = |kind: &str| {
        r.core
            .notifications()
            .iter()
            .filter(|i| i.posted.kind == kind && !i.read)
            .count()
    };

    // The person answers in their own words: it's with the agent now.
    co.block(&r.key_a, &task.id, "Blue or green?", on())
        .unwrap();
    wait_for(10, "the blocker's notification", || {
        r.core.notifications().len() == 1
    });
    assert_eq!(r.core.unread_notifications(), 1);
    let b1 = co.get_task(&task.id).unwrap().open_blockers()[0].id.clone();
    co.answer(
        &person,
        &task.id,
        &b1,
        coord::BlockerAnswer {
            text: "Blue".into(),
            ..Default::default()
        },
    )
    .unwrap();
    wait_for(10, "the answered blocker read", || {
        r.core.unread_notifications() == 0
    });
    assert!(r.core.notifications()[0].read);

    // The agent asks back: news, unread, though the answer was read.
    co.comment_on(
        &r.key_a,
        &task.id,
        coord::CommentInput {
            text: "Which blue?".into(),
            blocker_id: b1.clone(),
            missing: vec!["the shade".into()],
            ..Default::default()
        },
    )
    .unwrap();
    wait_for(10, "the ask-back's notification", || {
        r.core.unread_notifications() == 1
    });
    co.answer(
        &person,
        &task.id,
        &b1,
        coord::BlockerAnswer {
            text: "Navy".into(),
            ..Default::default()
        },
    )
    .unwrap();
    wait_for(10, "the answered ask-back read", || {
        r.core.unread_notifications() == 0
    });

    // The agent resolves the next one itself.
    co.unblock(&r.key_a, &task.id, "went with blue").unwrap();
    co.block(&r.key_a, &task.id, "Matte or gloss?", on())
        .unwrap();
    wait_for(10, "the second blocker's notification", || {
        unread_of(coord::notifications::KIND_BLOCKER) == 1
    });
    co.unblock(&r.key_a, &task.id, "gloss, as the brief says")
        .unwrap();
    wait_for(10, "the unblocked blocker read", || {
        r.core.unread_notifications() == 0
    });

    // A done check, marked Done.
    co.set_task_state(&r.key_a, &task.id, "completed", "done: it's blue")
        .unwrap();
    wait_for(10, "the done check's notification", || {
        unread_of(coord::notifications::KIND_DONE_CHECK) == 1
    });
    notify::watch::mark_done(co, &person, &task.id).unwrap();
    wait_for(10, "the marked done check read", || {
        r.core.unread_notifications() == 0
    });

    // Irori read them and said why; the person read nothing.
    let reads = r
        .core
        .store()
        .list(&store::Filter {
            kinds: vec![coord::notifications::READ.into()],
            limit: store::MAX_LIST_LIMIT,
            ..store::Filter::default()
        })
        .unwrap();
    assert!(!reads.is_empty());
    assert!(reads.iter().all(|e| e.actor == "irori"), "{reads:?}");
    r.core.close().unwrap();
}

/// Card #482: an agent's message to the person reads by itself once they
/// handle it anywhere (here, marked read in Talk).
#[test]
fn a_message_handled_elsewhere_leaves_the_badge_by_itself() {
    let (_cards, n) = notifier();
    let r = Rig::with(|o| {
        o.headless = false;
        o.notifier = Some(n.clone());
    });
    let ib = r.core.inbox();
    let you = r.core.user_sender();
    let (m, _) = ib
        .send(
            &you,
            &inbox::Address::new(inbox::addr::AGENT, &r.key_a),
            "status?",
            "",
        )
        .unwrap();
    let (_, d) = ib
        .send(
            &inbox::Sender::new(&r.key_a, "alpha", "claude"),
            &inbox::Address::new(inbox::addr::REPLY, &m.id),
            "all green",
            "",
        )
        .unwrap();
    wait_for(10, "the message's notification", || {
        r.core
            .notifications()
            .iter()
            .any(|i| i.posted.kind == coord::notifications::KIND_MESSAGE && !i.read)
    });
    ib.skip(&you.key, &d.id, "").unwrap();
    wait_for(10, "the read message read", || {
        r.core.unread_notifications() == 0
    });
    r.core.close().unwrap();
}

/// Card #483: the name a returned card's notification gives its agent
/// comes from the same source as `agent_name`, which keeps the last name
/// an agent went by once it has left the roster (a returned card's agent
/// always has).
#[test]
fn a_gone_agent_keeps_its_name() {
    let r = Rig::new();
    wait_for(10, "alpha by name", || {
        r.core.agent_name(&r.key_a) == "alpha"
    });
    std::fs::remove_file(r.claude.join("sessions").join("900001.json")).unwrap();
    wait_for(30, "alpha leaves the roster", || {
        r.core.mesh_agents().iter().all(|a| a.key != r.key_a)
    });
    assert_eq!(r.core.agent_name(&r.key_a), "alpha");
}

/// Every Notifications change seen in a test, to check none named nothing.
struct Changes {
    feed: irori_core::Feed,
    seen: Vec<irori_core::NotifChange>,
}

impl Changes {
    /// Keeps a change; a lost feed fails the test.
    fn take(&mut self, ev: irori_core::CoreEvent) -> Option<&irori_core::NotifChange> {
        match ev {
            irori_core::CoreEvent::Notifications(c) => {
                self.seen.push(c);
                self.seen.last()
            }
            irori_core::CoreEvent::Resync => panic!("the feed lost changes: {:?}", self.seen),
            _ => None,
        }
    }

    /// Reads what the feed holds.
    fn drain(&mut self) {
        while let Some(ev) = self.feed.try_recv() {
            self.take(ev);
        }
    }

    /// Waits for a change naming exactly `keys`.
    fn wait(&mut self, what: &str, keys: &[String]) {
        let want = irori_core::NotifChange::keys(keys.to_vec());
        let mut found = false;
        wait_for(20, what, || {
            while let Some(ev) = self.feed.try_recv() {
                found |= self.take(ev).is_some_and(|c| *c == want);
            }
            found
        });
    }
}

/// list-signals.md §3.2 (card #1144): every Notifications change names
/// the items it touched, by the key the history keeps them under: a kept
/// blocker, an agent's note (which has no key of its own, so its row's
/// `row:<device>:<seq>`), the person reading one and then the rest, Irori
/// reading a settled blocker, and a row that comes from the log (another
/// device's) kept or read. None names nothing, which would read as all.
#[test]
fn each_notification_change_names_its_keys() {
    let (_cards, n) = notifier();
    let r = Rig::with(|o| {
        o.headless = false;
        o.notifier = Some(n.clone());
    });
    let mut ch = Changes {
        feed: r.core.feed(&[irori_core::Topic::Notifications]),
        seen: Vec::new(),
    };
    let co = r.core.coord();
    let person = r.core.user_sender().key;
    let task = co
        .create_task(
            &r.key_a,
            NewTask {
                title: "Pick a colour".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &task.id).unwrap();

    // A blocker on the person: kept, by its own key.
    co.block(
        &r.key_a,
        &task.id,
        "Blue or green?",
        Some(coord::BlockedBy {
            kind: coord::BLOCKED_ON_PERSON.into(),
            reference: String::new(),
        }),
    )
    .unwrap();
    wait_for(10, "the blocker's notification", || {
        r.core.notifications().len() == 1
    });
    let blocker = r.core.notifications()[0].key.clone();
    assert!(blocker.starts_with("blocker:"), "{blocker}");
    ch.wait("the blocker's change", &[blocker.clone()]);

    // An agent's note has no key: its row's.
    let note = |id: &str, body: &str| {
        n.post(notify::Note {
            id: id.into(),
            title: "alpha".into(),
            body: body.into(),
            about: Some(coord::notifications::PostedRow {
                kind: coord::notifications::KIND_AGENT.into(),
                from: r.key_a.clone(),
                ..Default::default()
            }),
            ..notify::Note::default()
        });
        r.core.notifications()[0].key.clone()
    };
    let first = note("notify:1", "the build is green");
    assert!(first.starts_with("row:"), "{first}");
    ch.wait("the agent note's change", &[first.clone()]);

    // The person reads it.
    r.core.read_notifications(&[first.clone()]).unwrap();
    ch.wait("the read's change", &[first.clone()]);

    // The person answers the blocker: Irori reads it as settled.
    let b = co.get_task(&task.id).unwrap().open_blockers()[0].id.clone();
    co.answer(
        &person,
        &task.id,
        &b,
        coord::BlockerAnswer {
            text: "Blue".into(),
            ..Default::default()
        },
    )
    .unwrap();
    ch.wait("the settle's change", &[blocker.clone()]);

    // Two more notes, then "read all": the change names both.
    let two = note("notify:2", "the tests pass");
    ch.wait("the second note's change", &[two.clone()]);
    let three = note("notify:3", "the PR is open");
    ch.wait("the third note's change", &[three.clone()]);
    let mut rest = vec![two, three];
    rest.sort();
    assert_eq!(r.core.notifications().iter().filter(|i| !i.read).count(), 2);
    r.core.read_notifications(&[]).unwrap();
    ch.wait("the read-all's change", &rest);

    // A row from the log, as another device's would come: the store
    // follower folds it and names its key.
    r.core
        .store()
        .append(
            "irori",
            coord::notifications::POSTED,
            &coord::notifications::PostedRow {
                key: "x:1".into(),
                kind: coord::notifications::KIND_AGENT.into(),
                title: "from the other device".into(),
                at: gocompat::time::GoTime::from_system(std::time::SystemTime::now()),
                ..Default::default()
            },
        )
        .unwrap();
    ch.wait("the synced kept row's change", &["x:1".to_string()]);
    r.core
        .store()
        .append(
            "user",
            coord::notifications::READ,
            &coord::notifications::ReadRow {
                keys: vec!["x:1".into()],
                at: gocompat::time::GoTime::from_system(std::time::SystemTime::now()),
                ..Default::default()
            },
        )
        .unwrap();
    ch.wait("the synced read row's change", &["x:1".to_string()]);
    assert_eq!(r.core.unread_notifications(), 0);

    std::thread::sleep(std::time::Duration::from_millis(300));
    ch.drain();
    for c in &ch.seen {
        assert!(
            !c.all && !c.is_all(),
            "a change named nothing: {:?}",
            ch.seen
        );
    }
    r.core.close().unwrap();
}
