//! Card #395: on a device with notifications, a Talk post that mentions
//! the person or is a DM to them shows a card that opens the conversation;
//! a plain channel post and the person's own post show none.

use crate::common;

use std::sync::{Arc, Mutex};

use common::{Rig, wait_for};
use inbox::Sender;

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

impl Cards {
    fn talk(&self) -> Vec<(String, String, String)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|n| n.id.starts_with("talk:"))
            .map(|n| (n.id.clone(), n.title.clone(), n.body.clone()))
            .collect()
    }
}

#[test]
fn only_mentions_and_dms_notify() {
    let cards = Arc::new(Cards::default());
    let notes = notify::Notifier::new(notify::Config {
        toaster: Some(cards.clone()),
        ..notify::Config::default()
    });
    let opened = Arc::new(Mutex::new(Vec::<notify::Target>::new()));
    {
        let opened = opened.clone();
        notes.set_opener(move |t| opened.lock().unwrap().push(t));
    }
    let r = Rig::with(|o| o.notifier = Some(notes));
    let me = r.core.user_sender();
    let alpha = Sender::new(&r.key_a, "alpha", "claude");
    let ch = r.core.talk_channel_create(&me.key, "suzuri", "").unwrap();
    r.core
        .talk_join(&ch.id, &coord::talk::Member::of(&r.key_a), &me.key)
        .unwrap();

    r.core
        .talk_post(&me, &ch.id, "a note to the room", &[], true, None)
        .unwrap();
    r.core
        .talk_post(&alpha, &ch.id, "the build is green", &[], true, None)
        .unwrap();
    let tagged = r
        .core
        .talk_post(
            &alpha,
            &ch.id,
            "anything blocking?",
            &["person".into()],
            true,
            None,
        )
        .unwrap();
    let id = notify::talk_id(&ch.id, &tagged.post.post_id);
    wait_for(10, "the mention's card", || {
        cards.talk().iter().any(|c| c.0 == id)
    });

    let dm = r
        .core
        .talk_open_dm(
            &coord::talk::Member::of(&r.key_a),
            &coord::talk::Member::of(&me.key),
            &r.key_a,
        )
        .unwrap();
    let said = r
        .core
        .talk_post(&alpha, &dm, "took #371", &[], true, None)
        .unwrap();
    let dm_id = notify::talk_id(&dm, &said.post.post_id);
    wait_for(10, "the DM's card", || {
        cards.talk().iter().any(|c| c.0 == dm_id)
    });

    let shown = cards.talk();
    assert_eq!(shown.len(), 2, "only the mention and the DM: {shown:?}");
    assert_eq!(shown[0].1, "alpha in #suzuri");
    assert_eq!(shown[0].2, "anything blocking?");
    assert_eq!(shown[1].1, "alpha");

    // A tap opens the conversation.
    let n = cards
        .0
        .lock()
        .unwrap()
        .iter()
        .find(|n| n.id == dm_id)
        .cloned()
        .unwrap();
    (n.on_click.expect("a body click"))();
    wait_for(5, "opened", || !opened.lock().unwrap().is_empty());
    let t = opened.lock().unwrap()[0].clone();
    assert_eq!(
        (t.tab.as_str(), t.ref_.as_str()),
        (notify::TAB_TALK, dm.as_str())
    );
}
