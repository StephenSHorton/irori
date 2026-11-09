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
        stop: None,
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
    /// Bob's computer's runner, installed only by [`rig_two`].
    runner_b: Arc<Runner>,
    ch: String,
}

/// Alice (a computer that hosts, named Juno) and Bob in a space's channel;
/// `alice` and `bob` are what each says (None: silent).
fn rig(alice: Option<&str>, bob: Option<&str>) -> Rig {
    rig_with(alice, bob, false)
}

/// [`rig`] with Bob's computer hosting his assistant too.
fn rig_two(alice: Option<&str>, bob: Option<&str>) -> Rig {
    rig_with(alice, bob, true)
}

fn rig_with(alice: Option<&str>, bob: Option<&str>, bob_hosts: bool) -> Rig {
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
    let runner_b = Arc::new(Runner::default());
    if bob_hosts {
        bob.core
            .set_spawn_setting(SpawnSetting {
                on: true,
                ..SpawnSetting::default()
            })
            .unwrap();
        bob.core
            .set_spawner(Some(spawner(bob.claude.join("sessions"))));
        bob.core.set_huddle_runner(Some(runner_b.clone()));
    }
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
        runner_b,
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

// ---- digests, routed questions and the chair (card #1053) --------------------

/// A made-up shared card, held by Alice's agent: what a routed question
/// matches.
fn alice_holds(r: &Rig, title: &str) {
    let space = r.alice.core.talk().channel(&r.ch).unwrap().space_id;
    let t = r
        .alice
        .core
        .board_coord(&space)
        .unwrap()
        .create_task(
            &r.alice.person_actor(),
            coord::NewTask {
                title: title.into(),
                ..Default::default()
            },
        )
        .unwrap();
    r.alice.core.kick_board(&space);
    let out = r.alice.tool(
        "board_join",
        serde_json::json!({ "task_id": t.short_ref() }),
    );
    assert!(!out.to_lowercase().contains("error"), "{out}");
    r.alice.core.kick_board(&space);
}

fn digest_looks(r: &Rig) -> Vec<String> {
    r.runner
        .looks()
        .into_iter()
        .filter(|m| m.contains("write notes only for what"))
        .collect()
}

/// Alice's assistant digests what Alice says: her lines, with Bob's for
/// context, become items of hers on both devices; nothing is settled.
#[test]
fn an_assistant_digests_its_own_persons_lines() {
    let r = rig(Some("we should move the release"), Some("fine with me"));
    r.runner.says(|m| {
        if m.contains("write notes only for what") {
            "<<item kind=\"decision\">>Move the release.<</item>>".into()
        } else {
            String::new()
        }
    });
    let call = r.call();
    wait_for(WAIT, "a digest look", || !digest_looks(&r).is_empty());
    let m = digest_looks(&r).remove(0);
    assert!(m.contains("we should move the release"), "{m}");
    assert!(m.contains("fine with me"), "Bob's lines are context: {m}");
    wait_for(WAIT, "the item on both devices", || {
        [&r.alice, &r.bob]
            .iter()
            .all(|d| texts(d, &call).contains(&"Move the release.".to_string()))
    });
    for d in [&r.alice, &r.bob] {
        let i = items(d, &call)
            .into_iter()
            .find(|i| i.text == "Move the release.")
            .unwrap();
        assert!(i.assistant && i.status == "open" && i.asker.is_none());
        assert_eq!(i.why.as_deref(), Some("digest"));
    }
    r.close();
}

/// A person who has said nothing has no digest, however much the others
/// talk.
#[test]
fn a_silent_persons_assistant_digests_nothing() {
    let r = rig(None, Some("we should move the release"));
    let call = r.call();
    wait_for(WAIT, "bob's lines reach alice's computer", || {
        r.lines_of(&r.alice, &call, "we should") >= 10 && r.alice.core.huddle_fed(&call) >= 10
    });
    assert!(r.runner.looks().is_empty(), "{:?}", r.runner.looks());
    r.close();
}

/// "Let my assistant add notes from what I say" off stops the digest.
#[test]
fn the_notes_switch_stops_the_digest() {
    let r = rig(Some("we should move the release"), Some("fine with me"));
    r.alice.core.set_huddle_notes_on(false).unwrap();
    assert!(!r.alice.core.huddle_notes_on());
    let call = r.call();
    wait_for(WAIT, "alice's lines reach her computer", || {
        r.lines_of(&r.alice, &call, "we should") >= 16 && r.alice.core.huddle_fed(&call) >= 16
    });
    assert!(digest_looks(&r).is_empty(), "{:?}", r.runner.looks());
    r.close();
}

/// A question goes to the assistant whose person holds a matching card,
/// once; its look may write only an answer, which wakes nothing; a question
/// that matches no card wakes no one.
#[test]
fn a_question_is_routed_to_the_holder_of_a_matching_card() {
    let r = rig(None, None);
    alice_holds(&r, "Windows signing for the installer");
    r.runner.says(|m| {
        if m.contains("put this question on the canvas") {
            let id = reply_id(m).unwrap_or_default();
            format!(
                "<<item kind=\"answer\" reply=\"{id}\">>Alice has it: the agent is on signing.<</item>>\n\
                 <<item kind=\"task\">>Alice: nothing.<</item>>\n\
                 <<item kind=\"question\">>And who else?<</item>>\n\
                 <<title>>Not mine to set<</title>>"
            )
        } else {
            String::new()
        }
    });
    let call = r.call();
    let q = r
        .bob
        .core
        .huddle_item_add(&call, "question", "who is working on Windows signing?", "")
        .unwrap();
    wait_for(WAIT, "the routed answer on both devices", || {
        [&r.alice, &r.bob].iter().all(|d| {
            texts(d, &call)
                .iter()
                .any(|t| t.starts_with("Alice has it"))
        })
    });
    for d in [&r.alice, &r.bob] {
        let v = items(d, &call);
        let a = v
            .iter()
            .find(|i| i.text.starts_with("Alice has it"))
            .unwrap();
        assert_eq!(a.reply.as_deref(), Some(q.item_id.as_str()));
        assert!(a.assistant && a.status == "open");
        assert_eq!(a.why.as_deref(), Some("routed"));
        assert_eq!(
            v.len(),
            2,
            "the question and its answer, nothing else: {v:?}"
        );
    }
    assert_eq!(
        r.alice.core.talk().call_title(&call),
        None,
        "a routed look sets no title"
    );
    // The look carried the card, and one question gave one look.
    let routed = |r: &Rig| {
        r.runner
            .looks()
            .into_iter()
            .filter(|m| m.contains("put this question on the canvas"))
            .collect::<Vec<_>>()
    };
    assert_eq!(routed(&r).len(), 1);
    assert!(routed(&r)[0].contains("Windows signing for the installer"));
    // A question that matches no card wakes no one.
    r.bob
        .core
        .huddle_item_add(&call, "question", "what is for lunch today?", "")
        .unwrap();
    wait_for(WAIT, "alice has the lunch question", || {
        texts(&r.alice, &call).contains(&"what is for lunch today?".to_string())
    });
    std::thread::sleep(std::time::Duration::from_secs(3));
    assert_eq!(routed(&r).len(), 1);
    r.close();
}

fn assist_rows(d: &Dev, call: &str) -> Vec<coord::talk::AssistInfo> {
    d.core.talk().call_assists(call)
}

/// The starter's assistant chairs: its row says so on both devices. It
/// proposes a title once the huddle has run a while (a person's title beats
/// it), and at the end writes one summary with action items and `refs`,
/// changing no status; the chair's session closes when it is done.
#[test]
fn the_chair_titles_and_summarises_the_huddle() {
    let r = rig(None, None);
    r.alice
        .core
        .set_huddle_title_after(std::time::Duration::from_millis(500));
    let qid = Arc::new(Mutex::new(String::new()));
    let seen = qid.clone();
    r.runner.says(move |m| {
        if m.contains("The huddle is over") {
            let id = seen.lock().unwrap().clone();
            format!(
                "<<item kind=\"summary\" refs=\"{id},not-an-item\">>Moved the release; Alice ships it.<</item>>\n\
                 <<item kind=\"task\">>Alice: ship the release.<</item>>\n\
                 <<title>>Release planning<</title>>"
            )
        } else if m.contains("no title yet") {
            "<<title>>First guess<</title>>".into()
        } else {
            String::new()
        }
    });
    let call = r.call();
    wait_for(WAIT, "alice's row says she chairs, on both devices", || {
        [&r.alice, &r.bob].iter().all(|d| {
            assist_rows(d, &call)
                .iter()
                .any(|a| a.on && a.chair && !a.declines_chair)
        })
    });
    let q = r
        .bob
        .core
        .huddle_item_add(&call, "question", "when do we ship?", "")
        .unwrap();
    *qid.lock().unwrap() = q.item_id.clone();
    wait_for(WAIT, "the chair's title", || {
        [&r.alice, &r.bob]
            .iter()
            .all(|d| d.core.talk().call_title(&call).as_deref() == Some("First guess"))
    });
    // A person's title beats the assistant's, now and at the end.
    r.bob.core.call_title_set(&call, "Bob's title").unwrap();
    r.bob.core.call_leave().unwrap();
    r.alice.core.call_leave().unwrap();
    wait_for(WAIT, "one summary on both devices", || {
        [&r.alice, &r.bob].iter().all(|d| {
            items(d, &call)
                .iter()
                .any(|i| i.kind == "summary" && i.text.starts_with("Moved the release"))
        })
    });
    for d in [&r.alice, &r.bob] {
        let v = items(d, &call);
        let s: Vec<_> = v.iter().filter(|i| i.kind == "summary").collect();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].refs, vec![q.item_id.clone()], "only a real item");
        assert_eq!(s[0].why.as_deref(), Some("end"));
        assert!(s[0].assistant && s[0].status.is_empty());
        let t = v.iter().find(|i| i.kind == "task").unwrap();
        assert_eq!((t.status.as_str(), t.why.as_deref()), ("open", Some("end")));
        let qi = v.iter().find(|i| i.item_id == q.item_id).unwrap();
        assert_eq!(qi.status, "open", "the end changes no status");
        assert_eq!(
            d.core.talk().call_title(&call).as_deref(),
            Some("Bob's title")
        );
    }
    // It ran once, and the chair's session closes with it.
    std::thread::sleep(std::time::Duration::from_secs(4));
    let ends = r
        .runner
        .looks()
        .into_iter()
        .filter(|m| m.contains("The huddle is over"))
        .count();
    assert_eq!(ends, 1);
    wait_for(WAIT, "the chair's session closes", || {
        !r.alice.core.huddle_is_open(&call)
    });
    r.close();
}

/// "Let my assistant chair huddles I start" off: nobody chairs the
/// starter's huddle here, and the row says why; the title and the summary
/// are not written.
#[test]
fn the_chair_switch_passes_the_chair_on() {
    let r = rig(None, None);
    r.alice.core.set_huddle_chair_on(false).unwrap();
    r.alice
        .core
        .set_huddle_title_after(std::time::Duration::from_millis(500));
    r.runner.says(|m| {
        if m.contains("The huddle is over") || m.contains("no title yet") {
            "<<title>>Mine<</title>>\n<<item kind=\"summary\">>Done.<</item>>".into()
        } else {
            String::new()
        }
    });
    let call = r.call();
    wait_for(WAIT, "alice's row declines the chair", || {
        assist_rows(&r.bob, &call)
            .iter()
            .any(|a| a.on && a.declines_chair && !a.chair)
    });
    std::thread::sleep(std::time::Duration::from_secs(3));
    r.bob.core.call_leave().unwrap();
    r.alice.core.call_leave().unwrap();
    std::thread::sleep(std::time::Duration::from_secs(8));
    assert!(
        !items(&r.alice, &call).iter().any(|i| i.kind == "summary"),
        "{:?}",
        r.runner.looks()
    );
    assert_eq!(r.alice.core.talk().call_title(&call), None);
    r.close();
}

/// A huddle that is over with no end mark, and no assistant to write it,
/// gets one from a member device after three minutes; the fold keeps one.
#[test]
fn a_crashed_huddle_gets_one_end_mark() {
    use crate::common::mesh::person;
    let alice = person("alice", 1);
    let bob = person("bob", 1);
    let (a, b) = (&alice[0], &bob[0]);
    let space = invited(a, b);
    wait_for(WAIT, "alice hears bob read spaces", || {
        a.core.space_not_ready(&space).unwrap().is_empty()
    });
    a.core.space_gate(&space).unwrap();
    let ch = a
        .core
        .talk_channel_create_in(&a.user(), "general", "", &space)
        .unwrap();
    wait_for(WAIT, "bob holds the channel", || {
        b.core.talk().channel(&ch.id).is_some()
    });
    let call = "0123456789abcdef0123456789abcdef";
    let old = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        - 20 * 60 * 1000;
    a.core
        .talk()
        .write_segment(
            &ch.id,
            call,
            0,
            old,
            old + 900,
            "we were talking",
            &a.user(),
        )
        .unwrap();
    wait_for(WAIT, "bob holds the huddle", || {
        b.core.talk().call_conv(call).is_some()
    });
    let ended = |d: &Dev| {
        d.core
            .talk()
            .calls()
            .into_iter()
            .find(|c| c.call_id == call)
            .and_then(|c| c.end)
    };
    assert!(a.core.call_over(call));
    let now = std::time::SystemTime::now();
    a.core.huddle_crash_check_at(now);
    assert!(ended(a).is_none(), "not before three minutes");
    a.core
        .huddle_crash_check_at(now + std::time::Duration::from_secs(4 * 60));
    let m = ended(a).expect("the end mark");
    assert!(m.transcript && m.people.contains(&a.user()), "{m:?}");
    wait_for(WAIT, "bob sees the end mark", || ended(b).is_some());
    // Asked again, the first mark stays.
    a.core
        .huddle_crash_check_at(now + std::time::Duration::from_secs(5 * 60));
    assert_eq!(ended(a).unwrap().duration_ms, m.duration_ms);
    for d in alice.iter().chain(bob.iter()) {
        d.core.close().unwrap();
    }
}

/// Two assistants: the starter's chairs. When she leaves with Bob still in
/// the call the chair passes to Bob's, whose row says so and hers stands
/// down; when Bob leaves last, his assistant writes the one summary.
#[test]
fn the_chair_passes_on_when_its_person_leaves() {
    let r = rig_two(None, None);
    r.runner.says(|m| {
        if m.contains("The huddle is over") {
            "<<item kind=\"summary\">>Alice's chair wrote this.<</item>>".into()
        } else {
            String::new()
        }
    });
    r.runner_b.says(|m| {
        if m.contains("The huddle is over") {
            "<<item kind=\"summary\">>Bob's chair wrote this.<</item>>".into()
        } else {
            String::new()
        }
    });
    let call = r.call();
    wait_for(WAIT, "bob's computer holds a session too", || {
        r.bob.core.huddle_is_open(&call)
    });
    let chair_of = |d: &Dev, who: &str| {
        assist_rows(d, &call)
            .iter()
            .any(|a| a.on && a.chair && a.by == who)
    };
    wait_for(WAIT, "the starter chairs, on both devices", || {
        chair_of(&r.alice, &r.alice.user()) && chair_of(&r.bob, &r.alice.person_actor())
    });
    assert!(
        !assist_rows(&r.bob, &call)
            .iter()
            .any(|a| a.on && a.chair && a.by == r.bob.user())
    );
    r.alice.core.call_leave().unwrap();
    wait_for(WAIT, "the chair passes to bob", || {
        r.bob
            .core
            .talk()
            .call_assists(&call)
            .iter()
            .any(|a| a.on && a.chair && a.by == r.bob.user())
    });
    wait_for(WAIT, "alice's host closes", || {
        !r.alice.core.huddle_is_open(&call)
    });
    r.bob.core.call_leave().unwrap();
    wait_for(WAIT, "bob's chair writes the summary", || {
        items(&r.alice, &call)
            .iter()
            .any(|i| i.kind == "summary" && i.text.starts_with("Bob's chair"))
    });
    std::thread::sleep(std::time::Duration::from_secs(4));
    let sums = items(&r.alice, &call)
        .iter()
        .filter(|i| i.kind == "summary")
        .count();
    assert_eq!(sums, 1);
    assert!(
        !r.runner
            .looks()
            .iter()
            .any(|m| m.contains("The huddle is over")),
        "the one who left writes nothing"
    );
    r.close();
}
