//! A message's mentions (card #77; board.md "Direct messages").

use crate::common;

use common::*;
use inbox::{Address, Mention, REASON_MENTIONED, SendOptions, addr, mode};

fn m(key: &str, name: &str, gone: bool) -> Mention {
    Mention {
        key: key.into(),
        name: name.into(),
        gone,
    }
}

#[test]
fn mentioned_agents_get_woken_copies() {
    let h = Harness::new();
    h.abc();
    let alpha = claude(KEY_A, "alpha");
    let (orig, _) =
        h.ib.send_with(
            &alpha,
            &Address::new(addr::AGENT, KEY_B),
            "ask @gamma about the port",
            SendOptions {
                ref_: "task:T1".into(),
                mentions: vec![m(KEY_C, "gamma", false), m(KEY_B, "beta", false)],
                ..SendOptions::default()
            },
        )
        .unwrap();
    assert_eq!(orig.mentions.len(), 2);
    let got = h.ib.send_mention_copies(&alpha, &orig, KEY_B, "beta");
    // The recipient already has the message: only gamma gets a copy.
    assert_eq!(got.len(), 1);
    let (mm, d) = &got[0];
    assert_eq!(mm.key, KEY_C);
    let d = d.as_ref().unwrap();
    assert_eq!(d.to, KEY_C);
    assert_eq!(d.mode, mode::WAKE);
    let copy = &d.message;
    assert_eq!(copy.reason, REASON_MENTIONED);
    assert_eq!(copy.parent_id, orig.id);
    assert_eq!(copy.ref_, "task:T1");
    assert_eq!(
        copy.text,
        "alpha mentioned you in a message to beta:\nask @gamma about the port"
    );
}

#[test]
fn a_gone_mention_gets_no_copy() {
    let h = Harness::new();
    h.abc();
    let alpha = claude(KEY_A, "alpha");
    let (orig, _) =
        h.ib.send_with(
            &alpha,
            &Address::new(addr::AGENT, KEY_B),
            "hi",
            SendOptions {
                mentions: vec![m(KEY_C, "gamma", true)],
                ..SendOptions::default()
            },
        )
        .unwrap();
    let got = h.ib.send_mention_copies(&alpha, &orig, KEY_B, "beta");
    assert_eq!(got.len(), 1);
    assert!(got[0].1.is_err());
    assert!(h.ib.for_agent(KEY_C, false, 0).is_empty());
}

#[test]
fn mentions_survive_a_replay() {
    let mut h = Harness::new();
    h.abc();
    let alpha = claude(KEY_A, "alpha");
    let (orig, _) =
        h.ib.send_with(
            &alpha,
            &Address::new(addr::AGENT, KEY_B),
            "hi @gamma",
            SendOptions {
                mentions: vec![m(KEY_C, "gamma", false)],
                ..SendOptions::default()
            },
        )
        .unwrap();
    h.reopen();
    let d = h.ib.for_agent(KEY_B, false, 0);
    let back = d.iter().find(|d| d.message.id == orig.id).unwrap();
    assert_eq!(back.message.mentions, [m(KEY_C, "gamma", false)]);
}
