//! "Agents answer now" (card #391, docs/design/talk-channels.md §3): a
//! thread's toggle. On by default when the text names nobody or an agent,
//! off when it names only people. Off, no agent is woken or handed it; the
//! agents in it read new posts at their next turn and may reply.

mod common;

use common::*;
use coord::roster::AgentCard;
use coord::*;

fn agents(e: &Env) -> Vec<AgentCard> {
    use coord::roster::Agents as _;
    e.ros.agents()
}

fn thread(e: &Env, text: &str, quiet: bool) -> (Task, Notice) {
    e.c.post_ask(
        USER,
        NewAsk {
            text: text.into(),
            thread: true,
            quiet,
            ..NewAsk::default()
        },
        &agents(e),
    )
    .unwrap()
}

fn ask_of(c: &Coord, id: &str) -> Ask {
    c.get_task(id).and_then(|t| t.ask).unwrap()
}

fn m(key: &str) -> Mention {
    Mention {
        key: key.into(),
        name: key.into(),
        ..Mention::default()
    }
}

/// A card alice holds and is busy on.
fn alices_card(e: &Env) -> Task {
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "lobby join".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.ros.set(ALICE, "busy");
    e.c.claim_task(ALICE, &t.id).unwrap()
}

#[test]
fn the_default_is_off_only_when_just_people_are_named() {
    assert!(agents_now_default(&[]), "nobody named");
    assert!(agents_now_default(&[m(ALICE)]), "an agent");
    assert!(agents_now_default(&[m("agent:c1")]), "a board agent");
    assert!(!agents_now_default(&[m("person:p2")]), "a person");
    assert!(!agents_now_default(&[m("person:p2"), m("user:u1")]), "two people");
    assert!(
        agents_now_default(&[m("person:p2"), m("agent:c1")]),
        "a person and an agent"
    );
}

#[test]
fn a_quiet_thread_naming_nobody_goes_to_no_agent() {
    let e = Env::new();
    let (tk, n) = thread(&e, "Note to self: the relay restarts at 5.", true);
    assert!(n.to.is_empty() && !n.wake, "{n:?}");
    let a = ask_of(&e.c, &tk.id);
    assert!(a.thread && !a.wake && a.taker.is_empty(), "{a:?}");
    e.clk.add(secs(120));
    assert!(e.c.sweep_asks(&agents(&e)).unwrap().is_empty(), "no hand-out");
    assert!(ask_of(&e.c, &tk.id).taker.is_empty());
}

#[test]
fn the_reading_quotes_five_lines_and_counts_the_rest() {
    let e = Env::new();
    let card = alices_card(&e);
    let (tk, n) = thread(&e, &format!("Is #{} moving?", card.short), true);
    assert!(n.to == [ALICE] && !n.wake && n.text.contains("optional"), "{n:?}");
    e.c.add_comment(ALICE, &tk.id, "Yes.", false).unwrap();
    let mut last = Vec::new();
    for i in 1..=7 {
        let (_, ns) =
            e.c.person_comment(USER, &tk.id, &format!("line {i}"), false, &[], None)
                .unwrap();
        last = ns;
    }
    assert!(last.len() == 1 && last[0].to == [ALICE] && !last[0].wake, "{last:?}");
    let t = &last[0].text;
    assert!(t.contains("7 new posts, the last 5 below"), "{t}");
    assert!(!t.contains("line 2") && t.contains("line 3") && t.contains("line 7"), "{t}");
    assert!(t.contains("A reply is optional") && t.contains(&format!("talk_read conv={}", tk.id)), "{t}");
    // What alice wrote, she has read: after her reply the count restarts.
    e.c.add_comment(ALICE, &tk.id, "Noted, all seven.", false).unwrap();
    let (_, ns) =
        e.c.person_comment(USER, &tk.id, "line 8", false, &[], None)
            .unwrap();
    assert!(ns[0].text.contains("1 new post.") && ns[0].text.contains("line 8"), "{ns:?}");
}

#[test]
fn the_person_turns_it_off_and_on() {
    let e = Env::new();
    let card = alices_card(&e);
    let (tk, n) = thread(&e, &format!("Is #{} moving?", card.short), false);
    assert!(n.wake);
    want_err(e.c.set_agents_now(ALICE, &tk.id, false), ErrorKind::NotAllowed);
    let t = e.c.set_agents_now(USER, &tk.id, false).unwrap();
    assert!(!t.ask.unwrap().wake);
    // Off: past the quiet spell, no hand-out; a waking post wakes nobody.
    e.clk.add(secs(60));
    assert!(e.c.sweep_asks(&agents(&e)).unwrap().is_empty());
    let (cm, ns) =
        e.c.person_comment(USER, &tk.id, "Anyone?", true, &[], None)
            .unwrap();
    assert!(!cm.wake && ns.iter().all(|n| !n.wake), "{ns:?}");
    // On again: nobody answered, so an idle agent gets it at the sweep.
    let t = e.c.set_agents_now(USER, &tk.id, true).unwrap();
    assert!(t.ask.unwrap().wake);
    let ns = e.c.sweep_asks(&agents(&e)).unwrap();
    assert!(
        ns.len() == 1 && ns[0].wake && ns[0].text.contains("handed it to you"),
        "{ns:?}"
    );
    // The thread's log says who turned it.
    let log = e.c.activity(&tk.id).unwrap_or_default();
    assert!(
        log.iter().any(|a| a.what.contains("turned Agents answer now off")),
        "{log:?}"
    );
}
