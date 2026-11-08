//! The held huddle session's policy in the Core (card #1046,
//! docs/design/call-agents.md §7): a priming turn with the packet, a look
//! per message on the same session, a restart from the seed when the
//! session is gone or past the context limit, and no retry of a failed
//! look. The process side is faked here; the window's tests run it.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use irori_core::huddle_session::RESTART_AT;
use irori_core::{HuddleRunner, HuddleSeed, Look, LookError};

use crate::common::Rig;

#[derive(Default)]
struct Fake {
    /// "open", "look" and "close", with what each was given.
    calls: Mutex<Vec<(String, String)>>,
    systems: Mutex<Vec<String>>,
    /// What the next looks answer; Ok with a small context when empty.
    looks: Mutex<VecDeque<Result<Look, LookError>>>,
}

fn look(context: u64) -> Look {
    Look {
        text: "<<item kind=\"note\">>noted<</item>>".into(),
        context,
        cost_usd: 0.0002,
        ms: 900,
    }
}

impl Fake {
    fn kinds(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(k, _)| k.clone())
            .collect()
    }
    fn opens(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| k == "open")
            .map(|(_, v)| v.clone())
            .collect()
    }
    fn answer(&self, r: Result<Look, LookError>) {
        self.looks.lock().unwrap().push_back(r);
    }
}

impl HuddleRunner for Fake {
    fn open(&self, call: &str, system: &str, first: &str) -> Result<Look, LookError> {
        assert_eq!(call, CALL);
        self.calls
            .lock()
            .unwrap()
            .push(("open".into(), first.into()));
        self.systems.lock().unwrap().push(system.into());
        Ok(look(6000))
    }
    fn look(&self, call: &str, message: &str) -> Result<Look, LookError> {
        assert_eq!(call, CALL);
        self.calls
            .lock()
            .unwrap()
            .push(("look".into(), message.into()));
        self.looks
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(look(7000)))
    }
    fn close(&self, call: &str) {
        self.calls
            .lock()
            .unwrap()
            .push(("close".into(), call.into()));
    }
}

const CALL: &str = "0123456789abcdef0123456789abcdef";

/// A Rig with a Personal channel holding one open card, and the fake
/// runner installed.
fn rig() -> (Rig, Arc<Fake>, HuddleSeed) {
    let r = Rig::new();
    let me = r.core.user_sender().key;
    let ch = r.core.talk_channel_create(&me, "us", "").unwrap().id;
    r.core
        .coord()
        .create_task(
            &me,
            coord::NewTask {
                title: "Ship the loader".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let fake = Arc::new(Fake::default());
    r.core.set_huddle_runner(Some(fake.clone()));
    let seed = HuddleSeed {
        call_id: CALL.into(),
        conv_id: ch,
        name: "Juno \u{b7} Addam's assistant".into(),
        ..Default::default()
    };
    (r, fake, seed)
}

#[test]
fn a_priming_turn_with_the_packet_then_looks_on_one_session() {
    let (r, fake, seed) = rig();
    r.core.huddle_open(&seed).unwrap();
    assert!(r.core.huddle_is_open(CALL));
    // Opening again leaves the session as it is.
    r.core.huddle_open(&seed).unwrap();
    for i in 0..3 {
        let l = r.core.huddle_look(&seed, &format!("lines {i}")).unwrap();
        assert!(l.text.contains("noted"));
    }
    assert_eq!(fake.kinds(), ["open", "look", "look", "look"]);
    let first = &fake.opens()[0];
    assert!(
        first.contains("<irori-data kind=\"space-cards\">"),
        "{first}"
    );
    assert!(first.contains("Ship the loader"), "{first}");
    assert!(first.contains("Title: (none yet)"), "{first}");
    assert!(
        first.ends_with(irori_core::huddle_session::PRIMING_ASK),
        "{first}"
    );
    assert!(
        !first.contains("<irori-lines"),
        "a priming turn has no lines: {first}"
    );
    let system = &fake.systems.lock().unwrap()[0];
    assert!(system.starts_with("In this huddle you are called Juno \u{b7} Addam's assistant."));
    r.core.close().unwrap();
}

#[test]
fn a_look_with_no_session_opens_one_first() {
    let (r, fake, seed) = rig();
    r.core.huddle_look(&seed, "lines").unwrap();
    assert_eq!(fake.kinds(), ["open", "look"]);
    r.core.close().unwrap();
}

#[test]
fn a_gone_session_starts_again_from_the_seed() {
    let (r, fake, mut seed) = rig();
    r.core.huddle_open(&seed).unwrap();
    // It was killed between looks: the next look finds it gone.
    fake.answer(Err(LookError::Gone("it stopped".into())));
    seed.title = "Release planning".into();
    seed.recent = vec!["[12:01:02] Addam: do we have anyone on signing?".into()];
    let l = r.core.huddle_look(&seed, "lines 1").unwrap();
    assert!(l.text.contains("noted"));
    assert_eq!(fake.kinds(), ["open", "look", "open", "look"]);
    let again = &fake.opens()[1];
    assert!(
        again.contains("Ship the loader"),
        "the seed has the packet: {again}"
    );
    assert!(again.contains("Title: Release planning"), "{again}");
    assert!(
        again.contains("<irori-lines kind=\"transcript\">\n[12:01:02] Addam: do we have anyone on signing?\n</irori-lines>"),
        "{again}"
    );
    // The retried look carries the same lines.
    let calls = fake.calls.lock().unwrap().clone();
    assert_eq!(calls[3], ("look".to_string(), "lines 1".to_string()));
    r.core.close().unwrap();
}

#[test]
fn a_failed_look_is_not_retried() {
    let (r, fake, seed) = rig();
    r.core.huddle_open(&seed).unwrap();
    fake.answer(Err(LookError::Failed(
        "The assistant took too long. Try again.".into(),
    )));
    let e = r.core.huddle_look(&seed, "lines").unwrap_err();
    assert!(e.contains("too long"), "{e}");
    assert_eq!(fake.kinds(), ["open", "look"]);
    // A session the runner killed reads as gone next time, and that look
    // starts again.
    fake.answer(Err(LookError::Gone("it stopped".into())));
    r.core.huddle_look(&seed, "lines 2").unwrap();
    assert_eq!(fake.kinds(), ["open", "look", "look", "open", "look"]);
    r.core.close().unwrap();
}

#[test]
fn past_the_context_limit_the_next_look_starts_from_the_seed() {
    let (r, fake, mut seed) = rig();
    r.core.huddle_open(&seed).unwrap();
    fake.answer(Ok(look(RESTART_AT + 1)));
    r.core.huddle_look(&seed, "lines 1").unwrap();
    assert_eq!(fake.kinds(), ["open", "look", "close"]);
    assert!(!r.core.huddle_is_open(CALL));
    seed.recent = vec!["[12:30:00] Mira: let's keep going".into()];
    r.core.huddle_look(&seed, "lines 2").unwrap();
    assert_eq!(fake.kinds(), ["open", "look", "close", "open", "look"]);
    let again = &fake.opens()[1];
    assert!(
        again.contains("Ship the loader") && again.contains("let's keep going"),
        "{again}"
    );
    r.core.close().unwrap();
}

#[test]
fn a_look_that_opens_and_finds_it_gone_is_not_retried_again() {
    let (r, fake, seed) = rig();
    fake.answer(Err(LookError::Gone("it stopped".into())));
    let e = r.core.huddle_look(&seed, "lines").unwrap_err();
    assert!(e.contains("it stopped"), "{e}");
    assert_eq!(fake.kinds(), ["open", "look"]);
    assert!(!r.core.huddle_is_open(CALL));
    r.core.close().unwrap();
}

#[test]
fn close_ends_the_session_and_no_runner_means_no_look() {
    let (r, fake, seed) = rig();
    r.core.huddle_open(&seed).unwrap();
    r.core.huddle_close(CALL);
    assert!(!r.core.huddle_is_open(CALL));
    assert_eq!(fake.kinds(), ["open", "close"]);
    r.core.set_huddle_runner(None);
    let e = r.core.huddle_look(&seed, "lines").unwrap_err();
    assert!(e.contains("no assistant on this device"), "{e}");
    r.core.close().unwrap();
}
