//! The live line feed of a huddle's assistant (card #1050,
//! docs/design/call-agents.md §6 L1 and L2, §8 "The feed") through real
//! Cores on loopback iroh. Alice's computer hosts her assistant (a fake
//! held-session runner here that keeps the messages it was given) and is in
//! the call with Bob. The fake transcriber calls each phrase
//! "{label} {k}", so a device's label is what it "says". Made-up people
//! only.

use crate::common;

use std::sync::{Arc, Mutex};

use call::device::fake::{self, Recording};
use common::mesh::{Dev, Edit, invited, person_with};
use common::{wait_for, write_session};
use irori_core::spawn::{SpawnReq, SpawnSetting, SpawnStarted, Spawner};
use irori_core::{HuddleRunner, Look, LookError};

const WAIT: u64 = 90;

/// Counts what it was asked, and keeps the messages of the looks.
#[derive(Default)]
struct Runner {
    opens: Mutex<usize>,
    messages: Mutex<Vec<String>>,
}

impl Runner {
    fn looks(&self) -> Vec<String> {
        self.messages.lock().unwrap().clone()
    }
}

fn ok() -> Look {
    Look {
        text: String::new(),
        context: 6000,
        cost_usd: 0.0002,
        ms: 900,
    }
}

impl HuddleRunner for Runner {
    fn open(&self, _call: &str, _system: &str, _first: &str) -> Result<Look, LookError> {
        *self.opens.lock().unwrap() += 1;
        Ok(ok())
    }
    fn look(&self, _call: &str, message: &str) -> Result<Look, LookError> {
        self.messages.lock().unwrap().push(message.to_string());
        Ok(ok())
    }
    fn close(&self, _call: &str) {}
}

fn spawner(sessions: std::path::PathBuf) -> Spawner {
    Spawner {
        ready: None,
        spawn: Arc::new(move |r: &SpawnReq| {
            write_session(
                &sessions,
                "900003.json",
                "bbbbbbbb-5555-4555-8555-00000000000b",
                &r.name,
                std::path::Path::new(&r.folder),
            );
            Ok(SpawnStarted {
                name: r.name.clone(),
                prefix: "bbbbbbbb".into(),
            })
        }),
        end: Arc::new(|_| Ok(String::new())),
        trusted: Arc::new(|_| true),
        list: None,
    }
}

/// A phrase every 3 s: 1 s of tone, 2 s of quiet.
fn talking() -> fake::Gen {
    Box::new(|n| {
        let t = n as f64 / 48_000.0;
        if t % 3.0 < 1.0 {
            0.3 * (2.0 * std::f64::consts::PI * 600.0 * t).sin() as f32
        } else {
            0.0
        }
    })
}

/// A mic that speaks (or not) and a transcriber that calls each phrase
/// "{label} {k}".
fn voice(loud: bool, label: String) -> Edit {
    Arc::new(move |o| {
        let rec = Arc::new(Mutex::new(Recording::default()));
        o.call_audio = Some(Arc::new(move || {
            let sound: fake::Gen = if loud { talking() } else { Box::new(|_| 0.0) };
            fake::devices(sound, rec.clone())
        }));
        o.call_transcriber = Some(call::transcribe::fake::energy(&label));
    })
}

struct Rig {
    alice: Dev,
    bob: Dev,
    runner: Arc<Runner>,
    ch: String,
}

/// Alice (a computer that hosts, named Juno) and Bob in a space's channel;
/// `alice` and `bob` are what each says (None: silent).
fn rig(alice: Option<&str>, bob: Option<&str>) -> Rig {
    let (a, b) = (alice.map(String::from), bob.map(String::from));
    let alice = person_with("alice", 1, move |_| {
        Some(voice(a.is_some(), a.clone().unwrap_or_default()))
    })
    .remove(0);
    let bob = person_with("bob", 1, move |_| {
        Some(voice(b.is_some(), b.clone().unwrap_or_default()))
    })
    .remove(0);
    alice
        .core
        .set_spawn_setting(SpawnSetting {
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    alice
        .core
        .set_spawner(Some(spawner(alice.claude.join("sessions"))));
    let runner = Arc::new(Runner::default());
    alice.core.set_huddle_runner(Some(runner.clone()));
    alice.core.set_assistant_name("Juno").unwrap();
    let space = invited(&alice, &bob);
    wait_for(WAIT, "alice hears bob read spaces", || {
        alice.core.space_not_ready(&space).unwrap().is_empty()
    });
    alice.core.space_gate(&space).unwrap();
    let ch = alice
        .core
        .talk_channel_create_in(&alice.user(), "general", "", &space)
        .unwrap()
        .id;
    wait_for(WAIT, "bob has the channel", || {
        bob.core
            .talk_conversations(&bob.user())
            .is_ok_and(|cs| cs.iter().any(|c| c.conv_id == ch))
    });
    wait_for(WAIT, "the two devices reach each other", || {
        let r = alice.core.call_reachable(&space);
        r.contains(&bob.core.device_id().to_string())
    });
    Rig {
        alice,
        bob,
        runner,
        ch,
    }
}

impl Rig {
    /// Alice starts the huddle, Bob joins, and her computer hosts.
    fn call(&self) -> String {
        let call = self.alice.core.call_start(&self.ch).expect("start");
        wait_for(WAIT, "bob is rung", || {
            self.bob
                .core
                .call_snapshot()
                .ringing
                .iter()
                .any(|r| r.call_id == call)
        });
        self.bob.core.call_join(&call).expect("join");
        wait_for(WAIT, "alice's computer holds a session", || {
            self.alice.core.huddle_is_open(&call)
        });
        call
    }

    /// Lines of `who` that `d` has folded.
    fn lines_of(&self, d: &Dev, call: &str, who: &str) -> usize {
        d.core
            .talk()
            .call_posts(call)
            .iter()
            .filter(|p| p.text.starts_with(who))
            .count()
    }

    fn close(self) {
        self.alice.core.close().unwrap();
        self.bob.core.close().unwrap();
    }
}

/// A line that calls Alice's assistant gets a look, with the words in it;
/// her own write is seen again by the space follow and feeds once.
#[test]
fn a_line_calling_the_assistant_gets_a_look() {
    let r = rig(Some("Juno who is working on signing"), None);
    let call = r.call();
    wait_for(WAIT, "a look with the line in it", || {
        r.runner
            .looks()
            .iter()
            .any(|m| m.contains("Juno who is working on signing"))
    });
    let m = &r.runner.looks()[0];
    assert!(
        m.contains(": Juno who is working on signing 0") && m.contains("called you"),
        "{m}"
    );
    wait_for(WAIT, "two lines folded", || {
        r.lines_of(&r.alice, &call, "Juno") >= 2
    });
    // The follow has had the rows too; each line is buffered once.
    std::thread::sleep(std::time::Duration::from_secs(1));
    let rows = r.lines_of(&r.alice, &call, "Juno");
    assert!(
        r.alice.core.huddle_fed(&call) <= rows,
        "fed {} for {rows} rows",
        r.alice.core.huddle_fed(&call)
    );
    r.close();
}

/// A mention in the middle of a line calls nobody.
#[test]
fn a_mid_line_mention_gets_no_look() {
    let r = rig(Some("we told Juno about the signing"), None);
    let call = r.call();
    wait_for(WAIT, "alice's lines are folded and fed", || {
        r.lines_of(&r.alice, &call, "we told") >= 2 && r.alice.core.huddle_fed(&call) >= 2
    });
    assert!(r.runner.looks().is_empty(), "{:?}", r.runner.looks());
    r.close();
}

/// Another person's question, heard through the space follow, wakes
/// nothing: it is context only.
#[test]
fn another_persons_question_wakes_no_look() {
    let r = rig(None, Some("do we have anyone working on signing"));
    let call = r.call();
    wait_for(WAIT, "bob's lines reach alice's computer", || {
        r.lines_of(&r.alice, &call, "do we") >= 2 && r.alice.core.huddle_fed(&call) >= 2
    });
    assert!(r.runner.looks().is_empty(), "{:?}", r.runner.looks());
    r.close();
}
