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

type Reply = Box<dyn Fn(&str) -> String + Send>;

/// Counts what it was asked, and keeps the messages of the looks. Its
/// replies (card #1051) come from `reply`, which sees the message; while
/// `hold` is set a look waits; `fail` makes the look fail.
struct Runner {
    opens: Mutex<usize>,
    messages: Mutex<Vec<String>>,
    reply: Mutex<Reply>,
    hold: Mutex<bool>,
    fail: Mutex<bool>,
}

impl Default for Runner {
    fn default() -> Runner {
        Runner {
            opens: Mutex::new(0),
            messages: Mutex::new(Vec::new()),
            reply: Mutex::new(Box::new(|_| String::new())),
            hold: Mutex::new(false),
            fail: Mutex::new(false),
        }
    }
}

impl Runner {
    fn looks(&self) -> Vec<String> {
        self.messages.lock().unwrap().clone()
    }

    fn says(&self, f: impl Fn(&str) -> String + Send + 'static) {
        *self.reply.lock().unwrap() = Box::new(f);
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
        while *self.hold.lock().unwrap() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if *self.fail.lock().unwrap() {
            return Err(LookError::Failed("timed out".into()));
        }
        Ok(Look {
            text: (self.reply.lock().unwrap())(message),
            ..ok()
        })
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

// ---- the canvas (card #1051) -------------------------------------------------

/// The items of `call` on `d`, as its Talk folds them.
fn items(d: &Dev, call: &str) -> Vec<coord::talk::CanvasItem> {
    d.core.talk().call_items(call)
}

fn texts(d: &Dev, call: &str) -> Vec<String> {
    items(d, call).into_iter().map(|i| i.text).collect()
}

/// The reply id a look's message names: "reply=\"ID\"".
fn reply_id(m: &str) -> Option<String> {
    let at = m.find("reply=\"")? + "reply=\"".len();
    Some(m[at..].split('"').next()?.to_string())
}

/// A line calling the assistant writes a placeholder at once; the look's
/// blocks become items on both devices, the first answer taking the
/// placeholder's place. Bob called it, so the items group under Bob on
/// every device. Nothing but items is written, and none is settled.
#[test]
fn blocks_become_items_and_the_answer_replaces_the_placeholder() {
    let r = rig(None, Some("Juno who is working on signing"));
    *r.runner.hold.lock().unwrap() = true;
    r.runner.says(|_| {
        "Sure.\n<<item kind=\"answer\" reply=\"ITEM-ID\">>Addam has #412 on signing.<</item>>\n\
         <<item kind=\"task\">>Stephen: split the notes.<</item>>\n\
         <<item kind=\"note\" status=\"settled\">>Two cards cover the sync bug.<</item>>"
            .into()
    });
    let call = r.call();
    // The placeholder is there before the look answers, on both devices.
    wait_for(WAIT, "a placeholder on both devices", || {
        [&r.alice, &r.bob].iter().all(|d| {
            texts(d, &call)
                .iter()
                .any(|t| t.starts_with("Juno is looking: Juno who is working on signing"))
        })
    });
    let ph = items(&r.alice, &call)[0].item_id.clone();
    *r.runner.hold.lock().unwrap() = false;
    wait_for(WAIT, "the blocks on both devices", || {
        [&r.alice, &r.bob].iter().all(|d| {
            texts(d, &call).len() >= 3 && !texts(d, &call).iter().any(|t| t.contains("is looking"))
        })
    });
    for d in [&r.alice, &r.bob] {
        let v = items(d, &call);
        assert_eq!(v[0].item_id, ph, "the answer took the placeholder's place");
        assert_eq!(v[0].text, "Addam has #412 on signing.");
        assert!(v.iter().all(|i| i.assistant && i.status != "settled"));
        assert_eq!(v[1].kind, "task");
        assert_eq!(v[1].status, "open");
        assert_eq!(v[2].kind, "note");
        assert_eq!(
            v[2].status, "",
            "a note has none, and a block can't set one"
        );
        // Called by Bob: grouped under him, on Alice's device and his own.
        assert!(
            v.iter()
                .all(|i| i.asker.is_some() && i.why.as_deref() == Some("name"))
        );
    }
    let a = r.alice.core.call_canvas(&call);
    let b = r.bob.core.call_canvas(&call);
    assert!(
        a.iter()
            .all(|e| e.label.starts_with("Juno \u{b7} ") && e.mine && !e.asker_mine)
    );
    assert!(b.iter().all(|e| !e.mine && e.asker_mine));
    assert_eq!(a[0].asker_name, b[0].asker_name, "both name Bob alike");
    assert!(!a[0].asker_name.is_empty());
    r.close();
}

/// A timeout turns the placeholder into "couldn't answer"; a look with
/// nothing to say turns it into "found nothing".
#[test]
fn a_failed_look_and_an_empty_one_close_the_placeholder() {
    let r = rig(None, Some("Juno who is working on signing"));
    *r.runner.fail.lock().unwrap() = true;
    let call = r.call();
    wait_for(WAIT, "the placeholder says it couldn't answer", || {
        texts(&r.bob, &call) == vec!["Juno couldn't answer".to_string()]
    });
    r.close();

    let r = rig(None, Some("Juno who is working on signing"));
    r.runner.says(|_| "I think nothing.".into());
    let call = r.call();
    wait_for(WAIT, "the placeholder says nothing was found", || {
        texts(&r.bob, &call) == vec!["Juno found nothing".to_string()]
    });
    r.close();
}

/// A person's Ask is their own question item; their assistant's host gets
/// a look at it and its answer replies to it. Bob's page can ask the same
/// way for his own assistant only: Alice cannot ask on Bob's behalf.
#[test]
fn ask_gets_a_look_and_an_item() {
    let r = rig(None, None);
    r.runner.says(|m| match reply_id(m) {
        Some(id) => format!("<<item kind=\"answer\" reply=\"{id}\">>Addam has #412.<</item>>"),
        None => String::new(),
    });
    let call = r.call();
    wait_for(WAIT, "alice's assistant is connected", || {
        r.alice
            .core
            .call_assistants(&call)
            .iter()
            .any(|a| a.state == "connected")
    });
    let q = r
        .alice
        .core
        .huddle_ask(&call, "who has Windows signing?")
        .unwrap();
    assert_eq!(q.kind, "question");
    assert!(!q.assistant && q.why.as_deref() == Some("ask"));
    wait_for(WAIT, "the answer on both devices", || {
        [&r.alice, &r.bob]
            .iter()
            .all(|d| items(d, &call).iter().any(|i| i.text == "Addam has #412."))
    });
    for d in [&r.alice, &r.bob] {
        let a = items(d, &call)
            .into_iter()
            .find(|i| i.text == "Addam has #412.")
            .unwrap();
        assert_eq!(a.reply.as_deref(), Some(q.item_id.as_str()));
        assert!(a.assistant && a.status == "open");
        assert_eq!(a.why.as_deref(), Some("ask"));
    }
    // The look carried the question and its id.
    assert!(
        r.runner
            .looks()
            .iter()
            .any(|m| m.contains("who has Windows signing?") && m.contains(&q.item_id))
    );
    // A question the packet can't answer leaves an open note.
    r.runner.says(|_| String::new());
    r.alice
        .core
        .huddle_ask(&call, "what is the weather?")
        .unwrap();
    wait_for(WAIT, "a note saying nothing answered", || {
        texts(&r.bob, &call)
            .iter()
            .any(|t| t.starts_with("Juno found nothing to answer: what is the weather?"))
    });
    // Bob has no assistant of his own connected here (no host): no Ask.
    assert!(r.bob.core.huddle_ask(&call, "hello?").is_err());
    r.close();
}

/// A person adds, settles and moves items; the other member sees them. The
/// write path refuses `carded` while the huddle is live.
#[test]
fn a_person_writes_the_canvas_and_cannot_card_it_while_live() {
    let r = rig(None, None);
    let call = r.call();
    let add = r
        .bob
        .core
        .huddle_item_add(&call, "decision", "Keep the old updater until 0.16.", "")
        .unwrap();
    assert!(!add.assistant && add.status == "open");
    wait_for(WAIT, "alice has bob's item", || {
        texts(&r.alice, &call).contains(&"Keep the old updater until 0.16.".to_string())
    });
    r.alice
        .core
        .huddle_item_set(
            &call,
            coord::talk::NewItemSet {
                item_id: add.item_id.clone(),
                status: Some("settled".into()),
                x: Some(120.0),
                y: Some(-40.0),
                ..Default::default()
            },
        )
        .unwrap();
    wait_for(WAIT, "bob sees it settled and moved", || {
        items(&r.bob, &call).iter().any(|i| {
            i.item_id == add.item_id
                && i.status == "settled"
                && i.x == Some(120.0)
                && i.y == Some(-40.0)
        })
    });
    // Carding waits for the end of the call.
    for set in [
        coord::talk::NewItemSet {
            item_id: add.item_id.clone(),
            status: Some("carded".into()),
            ..Default::default()
        },
        coord::talk::NewItemSet {
            item_id: add.item_id.clone(),
            card: Some("mlg#7".into()),
            ..Default::default()
        },
    ] {
        let e = r.alice.core.huddle_item_set(&call, set).unwrap_err();
        assert!(e.contains("still live"), "{e}");
    }
    assert!(
        r.alice
            .core
            .huddle_item_add(&call, "task", "x", "carded")
            .is_err()
    );
    // An unknown item and a bad kind are refused.
    assert!(
        r.alice
            .core
            .huddle_item_add(&call, "chat", "x", "")
            .is_err()
    );
    r.close();
}

/// A look carries only what changed since the last one: the canvas items
/// others added, not the ones it wrote itself.
#[test]
fn a_look_carries_the_items_added_since_the_last() {
    let r = rig(None, None);
    r.runner.says(|m| {
        if m.contains("who has signing") {
            "<<item kind=\"answer\">>Addam has #412.<</item>>".into()
        } else {
            String::new()
        }
    });
    let call = r.call();
    wait_for(WAIT, "connected", || {
        r.alice
            .core
            .call_assistants(&call)
            .iter()
            .any(|a| a.state == "connected")
    });
    r.alice.core.huddle_ask(&call, "who has signing?").unwrap();
    wait_for(WAIT, "the first answer", || {
        texts(&r.alice, &call).contains(&"Addam has #412.".to_string())
    });
    let n = r.runner.looks().len();
    r.bob
        .core
        .huddle_item_add(&call, "note", "Bob's own note.", "")
        .unwrap();
    wait_for(WAIT, "alice has bob's note", || {
        texts(&r.alice, &call).contains(&"Bob's own note.".to_string())
    });
    r.alice.core.huddle_ask(&call, "anything else?").unwrap();
    wait_for(WAIT, "a second ask look", || r.runner.looks().len() > n);
    let m = r.runner.looks().last().cloned().unwrap();
    assert!(
        m.contains("Added to the canvas since your last look"),
        "{m}"
    );
    assert!(m.contains("Bob's own note."), "{m}");
    assert!(
        !m.contains("Addam has #412."),
        "its own items aren't repeated: {m}"
    );
    r.close();
}
