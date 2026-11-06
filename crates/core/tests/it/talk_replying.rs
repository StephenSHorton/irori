//! Talk's replying row (card #645): mentioning two agents in a channel
//! shows both until each answers, and a DM shows its agent.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gocompat::time::GoTime;
use irori_core::{Core, TalkReplying};

use crate::common::{Rig, wait_for};

fn names(rs: &[TalkReplying], conv: &str) -> Vec<String> {
    rs.iter()
        .find(|r| r.conv_id == conv)
        .map(|r| r.agents.iter().map(|a| a.name.clone()).collect())
        .unwrap_or_default()
}

fn engage(c: &Core, key: &str, turn: &str) -> inbox::Wake {
    let ib = c.inbox();
    let w = ib.next_wake(key, inbox::path::MOD).unwrap().expect("woken");
    ib.mark_delivered(key, &w.token).unwrap();
    ib.mark_engaged(key, &w.token, turn).unwrap();
    w
}

#[test]
fn two_mentions_show_together_until_each_answers() {
    let r = Rig::new();
    let me = r.core.user_sender();
    let ch = r.core.talk_channel_create(&me.key, "general", "").unwrap();
    assert!(r.core.talk_replying(&me.key).is_empty());
    r.core
        .talk_post(
            &me,
            &ch.id,
            "@alpha @beta can you look?",
            &[],
            true,
            None,
            vec![],
        )
        .unwrap();
    // From the wake, before either turn starts.
    assert_eq!(
        names(&r.core.talk_replying(&me.key), &ch.id),
        vec!["alpha", "beta"]
    );
    engage(&r.core, &r.key_a, "ta");
    let wb = engage(&r.core, &r.key_b, "tb");
    assert_eq!(
        names(&r.core.talk_replying(&me.key), &ch.id),
        vec!["alpha", "beta"],
        "a mention's notice closes as its turn starts: still replying"
    );
    // alpha answers in the channel: it drops off, beta stays.
    let alpha = inbox::Sender::new(&r.key_a, "alpha", "claude");
    r.core
        .talk_post(&alpha, &ch.id, "on it", &[], true, None, vec![])
        .unwrap();
    assert_eq!(names(&r.core.talk_replying(&me.key), &ch.id), vec!["beta"]);
    // beta's turn ends without a post: it drops off too.
    r.core
        .inbox()
        .turn_ended(&r.key_b, &wb.token, "tb")
        .unwrap();
    assert!(r.core.talk_replying(&me.key).is_empty());
}

#[test]
fn a_dm_shows_its_agent() {
    let r = Rig::new();
    let me = r.core.user_sender();
    let dm = r
        .core
        .talk_open_dm(
            &coord::talk::Member::of(&me.key),
            &coord::talk::Member::of(&r.key_b),
            &me.key,
        )
        .unwrap();
    r.core
        .talk_post(&me, &dm, "can you look at #3?", &[], false, None, vec![])
        .unwrap();
    let rs = r.core.talk_replying(&me.key);
    assert_eq!(names(&rs, &dm), vec!["beta"]);
    assert_eq!(rs[0].agents[0].key, r.key_b);
    engage(&r.core, &r.key_b, "t1");
    assert_eq!(names(&r.core.talk_replying(&me.key), &dm), vec!["beta"]);
    // A DM is answered with a post, not the inbox: the post clears it.
    let beta = inbox::Sender::new(&r.key_b, "beta", "claude");
    r.core
        .talk_post(&beta, &dm, "looking", &[], false, None, vec![])
        .unwrap();
    assert!(r.core.talk_replying(&me.key).is_empty());
    // The next DM shows it again.
    r.core
        .talk_post(&me, &dm, "and #4?", &[], false, None, vec![])
        .unwrap();
    assert_eq!(names(&r.core.talk_replying(&me.key), &dm), vec!["beta"]);
}

#[test]
fn it_drops_after_thirty_minutes() {
    let off = Arc::new(Mutex::new(Duration::ZERO));
    let o2 = off.clone();
    let r = Rig::with(move |o| {
        o.now = Some(Arc::new(move || {
            let n = presence::now_local();
            let d = *o2.lock().unwrap();
            GoTime::from_system(n.to_system() + d)
        }));
    });
    let me = r.core.user_sender();
    let ch = r.core.talk_channel_create(&me.key, "general", "").unwrap();
    r.core
        .talk_post(&me, &ch.id, "@alpha hello", &[], true, None, vec![])
        .unwrap();
    assert_eq!(names(&r.core.talk_replying(&me.key), &ch.id), vec!["alpha"]);
    *off.lock().unwrap() = irori_core::REPLYING_CAP + Duration::from_secs(1);
    assert!(r.core.talk_replying(&me.key).is_empty());
}

#[test]
fn a_gone_agent_drops() {
    let r = Rig::new();
    let me = r.core.user_sender();
    let ch = r.core.talk_channel_create(&me.key, "general", "").unwrap();
    r.core
        .talk_post(&me, &ch.id, "@beta hello", &[], true, None, vec![])
        .unwrap();
    assert_eq!(names(&r.core.talk_replying(&me.key), &ch.id), vec!["beta"]);
    std::fs::remove_file(r.claude.join("sessions").join("900002.json")).unwrap();
    wait_for(60, "beta gone drops off the row", || {
        r.core.talk_replying(&me.key).is_empty()
    });
}

/// Sets beta's session status as Claude Code writes it.
fn beta_status(r: &Rig, status: &str) {
    let f = r.claude.join("sessions").join("900002.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
    v["status"] = status.into();
    v["updatedAt"] = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64)
        .into();
    std::fs::write(&f, serde_json::to_vec(&v).unwrap()).unwrap();
    wait_for(60, &format!("beta {status}"), || {
        r.core
            .local_agents()
            .iter()
            .any(|c| c.key == r.key_b && c.status == status)
    });
}

#[test]
fn a_turn_never_reported_ending_ends_when_the_agent_goes_idle() {
    let r = Rig::new();
    let me = r.core.user_sender();
    let dm = r
        .core
        .talk_open_dm(
            &coord::talk::Member::of(&me.key),
            &coord::talk::Member::of(&r.key_b),
            &me.key,
        )
        .unwrap();
    r.core
        .talk_post(&me, &dm, "can you look?", &[], false, None, vec![])
        .unwrap();
    engage(&r.core, &r.key_b, "t1");
    beta_status(&r, "busy");
    assert_eq!(names(&r.core.talk_replying(&me.key), &dm), vec!["beta"]);
    // Its session died mid-turn: no turn-ended, but it is idle again.
    std::thread::sleep(Duration::from_secs(11));
    beta_status(&r, "idle");
    assert!(r.core.talk_replying(&me.key).is_empty());
}

#[test]
fn a_later_post_naming_the_agent_isnt_the_waking_one() {
    let r = Rig::new();
    let me = r.core.user_sender();
    let ch = r.core.talk_channel_create(&me.key, "general", "").unwrap();
    r.core
        .talk_post(&me, &ch.id, "@alpha look", &[], true, None, vec![])
        .unwrap();
    let alpha = inbox::Sender::new(&r.key_a, "alpha", "claude");
    r.core
        .talk_post(&alpha, &ch.id, "done", &[], true, None, vec![])
        .unwrap();
    // Thanking it with agents off wakes nobody, and its answer still counts.
    r.core
        .talk_post(&me, &ch.id, "thanks @alpha", &[], false, None, vec![])
        .unwrap();
    assert!(r.core.talk_replying(&me.key).is_empty());
}
