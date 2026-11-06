//! Agents mentioning agents (card #77; board.md "Agents mention too").

use crate::common;

use common::*;
use inbox::mode::WAKE;

const KEY_M: &str = "mac1/dddddddd-4444-4444-8444-444444444444";
const KEY_M2: &str = "mac1/eeeeeeee-5555-4555-8555-555555555555";

#[test]
fn an_agent_mention_wakes_and_watches() {
    let h = new_harness();
    let (a, b) = (call_a(), call_b());
    let id = h.new_card(&a, json!({"title": "port clash"}));
    h.okj(&a, "board_join", json!({"task_id": id}));
    // gamma is idle: a mention wakes it with the card, as a watcher.
    let out = h.okj(
        &b,
        "board_comment",
        json!({"task_id": id, "text": "@gamma knows the ports, I think"}),
    );
    has(
        &out,
        &["Told at their next turn: alpha", "Mentioned: gamma (woke)."],
    );
    let ns = h.notices(KEY_C);
    assert_eq!(ns.len(), 1, "{ns:?}");
    assert_eq!(ns[0].mode, WAKE);
    assert_eq!(ns[0].message.reason, coord::NOTICE_MENTIONED);
    has(
        &ns[0].message.text,
        &[
            &format!("beta ({KEY_B}) mentioned you on"),
            "knows the ports",
            &format!("alpha ({KEY_A}) holds it"),
        ],
    );
    let t = h.b.co.get_task(&id).unwrap();
    assert_eq!(t.watchers, [KEY_C]);
    assert_eq!(t.holders, [KEY_A]);
    // alpha's own notice isn't doubled; gamma hears once.
    assert_eq!(h.notices(KEY_A).len(), 1);
    let cm = h.b.co.comments(&id).unwrap().pop().unwrap();
    assert_eq!(cm.mentions.len(), 1);
    assert_eq!(cm.mentions[0].key, KEY_C);
}

#[test]
fn mention_by_key_across_devices_and_misses() {
    let h = new_harness();
    let a = call_a();
    let mut m = card(KEY_M, "maple", "", "");
    m.device_id = "mac1".into();
    let mut m2 = card(KEY_M2, "maple", "", "");
    m2.device_id = "mac1".into();
    h.b.roster.put(m);
    h.b.roster.put(m2);
    let mut f = card("dev1/ffffffff-6666-4666-8666-666666666666", "fern", "", "");
    f.status = "gone".into();
    h.b.roster.put(f);
    let id = h.new_card(&a, json!({"title": "nobody holds it"}));
    let out = h.okj(
        &a,
        "board_comment",
        json!({"task_id": id, "text": "@maple or @cedr?",
            "mention": ["dev1/ffffffff-6666-4666-8666-666666666666", "agent:zzz"]}),
    );
    has(
        &out,
        &[
            "Not delivered: fern is gone",
            &format!(
                "maple is ambiguous: mention one by key ({KEY_M}, C:\\src\\maple; {KEY_M2}, C:\\src\\maple)"
            ),
            "no agent has key agent:zzz",
            "\"@cedr\" names no live agent",
        ],
    );
    assert!(h.notices(KEY_M).is_empty() && h.notices(KEY_M2).is_empty());
    // By key it reaches that one, on another device, and it is told to join.
    let out = h.okj(
        &a,
        "board_comment",
        json!({"task_id": id, "text": "@maple, this one", "mention": [KEY_M2]}),
    );
    has(&out, &["Mentioned: maple (woke)."]);
    // The text's @maple is the one named by key: no ambiguous line.
    has_not(&out, &["ambiguous", "Not delivered"]);
    let ns = h.notices(KEY_M2);
    assert_eq!(ns.len(), 1);
    has(
        &ns[0].message.text,
        &["Nobody holds it. If the work is yours, board_join it"],
    );
    assert!(h.b.co.get_task(&id).unwrap().holders.is_empty());
}

#[test]
fn a_message_mention_gets_its_own_copy() {
    let h = new_harness();
    let (a, b) = (call_a(), call_b());
    let id = h.new_card(&a, json!({"title": "about a card"}));
    let out = h.okj(
        &a,
        "send",
        json!({"to": "beta", "text": "ask @gamma too", "task_id": id}),
    );
    has(&out, &["Mentioned: gamma (woke)."]);
    let d = h.b.ib.for_agent(KEY_C, true, 0);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0].message.reason, inbox::REASON_MENTIONED);
    has(
        &d[0].message.text,
        &[
            "alpha mentioned you in a message to beta:",
            "ask @gamma too",
        ],
    );
    // The card entry carries the mention and gamma watches the card.
    assert_eq!(h.b.co.get_task(&id).unwrap().watchers, [KEY_C]);
    // A reply can mention as well.
    let dl = h.b.ib.for_agent(KEY_B, true, 0);
    let did = dl
        .iter()
        .find(|d| d.message.text == "ask @gamma too")
        .unwrap()
        .id
        .clone();
    let out = h.okj(
        &b,
        "reply",
        json!({"delivery_id": did, "text": "cc @gamma"}),
    );
    has(&out, &["Mentioned: gamma (woke)."]);
}

#[test]
fn a_blocker_mention_wakes_without_storing() {
    let h = new_harness();
    let a = call_a();
    let id = h.new_card(&a, json!({"title": "needs a call"}));
    h.okj(&a, "board_join", json!({"task_id": id}));
    let out = h.okj(
        &a,
        "board_block",
        json!({"task_id": id, "question": "which port should we use?", "mention": ["beta"]}),
    );
    has(&out, &["Mentioned: beta (woke)."]);
    assert_eq!(h.notices(KEY_B).len(), 1);
    assert_eq!(h.b.co.get_task(&id).unwrap().watchers, [KEY_B]);
}

#[test]
fn a_mentioned_creator_hears_an_unblock_once() {
    let h = new_harness();
    let (a, b) = (call_a(), call_b());
    // beta made it and nobody holds it: the unblock notice goes to beta.
    let id = h.new_card(&b, json!({"title": "nobody on it"}));
    h.okj(
        &b,
        "board_block",
        json!({"task_id": id, "question": "which port?"}),
    );
    let before = h.notices(KEY_B).len();
    let out = h.okj(
        &a,
        "board_unblock",
        json!({"task_id": id, "note": "@beta your turn"}),
    );
    has(&out, &["Unblocked", "Mentioned: beta (woke)."]);
    let ns = h.notices(KEY_B);
    assert_eq!(ns.len(), before + 1, "{ns:?}");
    // The one notice is the unblock itself, woken.
    assert_eq!(ns[0].mode, WAKE);
    has(&ns[0].message.text, &["your turn"]);
}

#[test]
fn mentions_past_the_cap_are_reported() {
    let h = new_harness();
    let a = call_a();
    for i in 0..11 {
        h.b.roster.put(card(
            &format!("dev1/0000000{i:x}-7777-4777-8777-777777777777"),
            &format!("n{i}"),
            "",
            "",
        ));
    }
    let id = h.new_card(&a, json!({"title": "everyone"}));
    let text: String = (0..11).map(|i| format!("@n{i} ")).collect();
    let out = h.okj(&a, "board_comment", json!({"task_id": id, "text": text}));
    has(
        &out,
        &["Not delivered: n10: at most 10 mentions per write."],
    );
}
