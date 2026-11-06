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
