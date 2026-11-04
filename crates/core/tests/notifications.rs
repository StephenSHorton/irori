//! The person's notification history (card #377): a notification the
//! desktop shows is kept once per thing it is about, kept with the popups
//! off, read through the Core, and still there after a restart.

mod common;

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
