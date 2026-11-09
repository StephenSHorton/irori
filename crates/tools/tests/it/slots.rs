//! Card #1168: the slot_take and slot_release tools. The holder is the
//! calling agent: a second taker over the limit is told who holds the
//! slots, a release frees one, and a gone agent's slots are freed.

use crate::common;

use common::*;

#[test]
fn a_second_taker_over_the_limit_is_told_who_holds_it() {
    let h = new_harness();
    let (a, b, c) = (call_a(), call_b(), call_c());
    let args = r#"{"name":"check/mac","limit":2}"#;
    has(&h.ok(&a, "slot_take", args), &["Took check/mac/slot-1"]);
    has(&h.ok(&b, "slot_take", args), &["Took check/mac/slot-2"]);
    // Taking again renews the same slot.
    has(&h.ok(&a, "slot_take", args), &["Took check/mac/slot-1"]);

    let out = h.fails(
        &c,
        "slot_take",
        args,
        &["every check/mac slot is held by", "alpha", "beta"],
    );
    has(&out, &["wait_seconds"]);

    has(
        &h.ok(&a, "slot_release", r#"{"name":"check/mac"}"#),
        &["Released"],
    );
    has(&h.ok(&c, "slot_take", args), &["Took check/mac/slot-1"]);
    has(
        &h.ok(&a, "slot_release", r#"{"name":"check/mac"}"#),
        &["held no check/mac slot"],
    );
}

#[test]
fn slot_arguments_are_checked() {
    let h = new_harness();
    let a = call_a();
    h.fails(&a, "slot_take", r#"{"limit":1}"#, &["give name"]);
    h.fails(&a, "slot_take", r#"{"name":"x/y"}"#, &["give limit"]);
    h.fails(
        &a,
        "slot_take",
        r#"{"name":"x/y","limit":0}"#,
        &["limit must be"],
    );
    h.fails(
        &a,
        "slot_take",
        r#"{"name":"x/y","limit":99}"#,
        &["limit must be"],
    );
    h.fails(
        &a,
        "slot_take",
        r#"{"name":"x/y","limit":1,"wait_seconds":999}"#,
        &["wait_seconds must be"],
    );
    h.fails(
        &a,
        "slot_take",
        r#"{"name":"port:80","limit":1}"#,
        &["plain name"],
    );
    h.fails(&a, "slot_release", "{}", &["give name"]);
}

#[test]
fn a_waiting_take_gives_up_after_wait_seconds() {
    let h = new_harness();
    let (a, b) = (call_a(), call_b());
    h.ok(&a, "slot_take", r#"{"name":"gpu/one","limit":1}"#);
    let out = h.fails(
        &b,
        "slot_take",
        r#"{"name":"gpu/one","limit":1,"wait_seconds":1}"#,
        &["held by", "alpha"],
    );
    has(&out, &["Still held after 1s"]);
}

#[test]
fn a_slot_is_renewed_while_the_agent_works_and_ends_when_it_stops() {
    let h = new_harness();
    let (a, b) = (call_a(), call_b());
    let args = r#"{"name":"build/x","limit":1}"#;
    h.ok(&a, "slot_take", args);
    // 20 minutes on, 10 left of 30: any call by a renews it (under half).
    h.clk.advance(std::time::Duration::from_secs(20 * 60));
    h.ok(&a, "leases", "{}");
    // 20 more minutes: it would have ended at 30, but was renewed.
    h.clk.advance(std::time::Duration::from_secs(20 * 60));
    h.fails(&b, "slot_take", args, &["held by", "alpha"]);
    // An agent that makes no calls for the whole ttl loses it.
    h.clk.advance(std::time::Duration::from_secs(31 * 60));
    has(&h.ok(&b, "slot_take", args), &["Took build/x/slot-1"]);
}
