//! Whole huddles through the real Core on loopback iroh (#654): a channel
//! huddle rings the space's devices, a rung device joins over irori/call/1,
//! voice from one fake mic comes out of the others' fake speakers, a third
//! device joins, and the last one out writes the end post with everyone in
//! it. Fake mic and speaker, real everything else. Made-up people only.
//!
//! In this harness a person's devices find each other on the LAN, but
//! another person's device learns only the inviter's address, so the
//! three-way call is one person's three devices (the Mac, the phone and a
//! third, as on the card) and the two-person call is two devices.
//!
//! A Personal channel's huddle (#663) has no space: it rings on the private
//! mesh and its connections are proved with the mesh key.
//!
//! Transcripts (#656) use the fake transcriber, which calls each loud run
//! "{label} {k}": the speaker's lines go live to the others as captions and
//! into the log as `call.segment` rows, or into one fallback post at leave
//! when the space isn't ready for them.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use call::device::fake::{self, Recording};
use coord::talk::{CALL_END, CALL_START, CallMark};
use irori_core::Core;
use store::{Filter, kinds};

use crate::common::mesh::{Dev, Edit, invited, person_with};
use crate::common::wait_for;

const WAIT: u64 = 60;

/// 200 ms of a 600 Hz tone every second.
fn bursts() -> fake::Gen {
    Box::new(|n| {
        let t = n as f64 / 48_000.0;
        if t % 1.0 < 0.2 {
            0.3 * (2.0 * std::f64::consts::PI * 600.0 * t).sin() as f32
        } else {
            0.0
        }
    })
}

/// Options that give every call on this device a fake mic (bursts when
/// `loud`, else silence) and a speaker recording into `rec`.
fn fake_audio(loud: bool, rec: Arc<Mutex<Recording>>) -> Edit {
    Arc::new(move |o| {
        let rec = rec.clone();
        o.call_audio = Some(Arc::new(move || {
            let sound: fake::Gen = if loud { bursts() } else { Box::new(|_| 0.0) };
            fake::devices(sound, rec.clone())
        }));
    })
}

/// 1 s of a 600 Hz tone then 2 s of quiet, over and over: a phrase every
/// 3 s to the fake transcriber.
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

/// [`fake_audio`] with [`talking`] for a loud mic, and the fake transcriber
/// labelled `label`.
fn scribe_audio(loud: bool, rec: Arc<Mutex<Recording>>, label: &'static str) -> Edit {
    Arc::new(move |o| {
        let rec = rec.clone();
        o.call_audio = Some(Arc::new(move || {
            let sound: fake::Gen = if loud { talking() } else { Box::new(|_| 0.0) };
            fake::devices(sound, rec.clone())
        }));
        o.call_transcriber = Some(call::transcribe::fake::energy(label));
    })
}

/// How many 10 ms blocks of `rec` since sample `from` are loud.
fn loud_blocks(rec: &Arc<Mutex<Recording>>, from: usize) -> usize {
    let r = rec.lock().unwrap();
    r.samples
        .get(from.min(r.samples.len())..)
        .unwrap_or_default()
        .chunks(480)
        .filter(|b| (b.iter().map(|v| v * v).sum::<f32>() / b.len() as f32).sqrt() > 0.05)
        .count()
}

fn len(rec: &Arc<Mutex<Recording>>) -> usize {
    rec.lock().unwrap().samples.len()
}

/// Live peers in `d`'s call.
fn live_peers(d: &Dev) -> usize {
    d.core
        .call_snapshot()
        .mine
        .map_or(0, |m| m.peers.iter().filter(|p| p.phase == "live").count())
}

struct Setup {
    alice: Vec<Dev>,
    bob: Vec<Dev>,
    recs: Vec<Arc<Mutex<Recording>>>,
    bob_rec: Arc<Mutex<Recording>>,
    space: String,
    ch: String,
}

/// Alice with `n` devices (the first one speaking) and Bob, in a space
/// with a channel; every device `who` picks reaches the others in it.
fn setup(n: usize, who: impl Fn(&Setup) -> Vec<&Dev>) -> Setup {
    setup_with(n, false, who)
}

/// [`setup`], with the fake transcriber on every device when `scribe`
/// (Alice's devices labelled "alice", Bob's "bob").
fn setup_with(n: usize, scribe: bool, who: impl Fn(&Setup) -> Vec<&Dev>) -> Setup {
    let recs: Vec<_> = (0..n)
        .map(|_| Arc::new(Mutex::new(Recording::default())))
        .collect();
    let bob_rec = Arc::new(Mutex::new(Recording::default()));
    let alice = {
        let recs = recs.clone();
        person_with("alice", n, move |i| {
            Some(if scribe {
                scribe_audio(i == 0, recs[i].clone(), "alice")
            } else {
                fake_audio(i == 0, recs[i].clone())
            })
        })
    };
    let bob = {
        let r = bob_rec.clone();
        person_with("bob", 1, move |_| {
            Some(if scribe {
                scribe_audio(false, r.clone(), "bob")
            } else {
                fake_audio(false, r.clone())
            })
        })
    };
    let space = invited(&alice[0], &bob[0]);
    for a in &alice[1..] {
        wait_for(WAIT, "alice's other devices hold the space", || {
            a.core.boards().contains(&space)
        });
    }
    wait_for(WAIT, "alice hears bob read spaces", || {
        alice[0].core.space_not_ready(&space).unwrap().is_empty()
    });
    alice[0].core.space_gate(&space).unwrap();
    let ch = alice[0]
        .core
        .talk_channel_create_in(&alice[0].user(), "general", "", &space)
        .unwrap()
        .id;
    let s = Setup {
        alice,
        bob,
        recs,
        bob_rec,
        space,
        ch,
    };
    let devs = who(&s);
    wait_for(
        WAIT,
        "the call's devices reach each other in the space",
        || {
            devs.iter().all(|d| {
                let r = d.core.call_reachable(&s.space);
                devs.iter()
                    .filter(|o| o.core.device_id() != d.core.device_id())
                    .all(|o| r.contains(&o.core.device_id().to_string()))
            })
        },
    );
    s
}

fn rung(d: &Dev, call_id: &str, conv: &str) -> bool {
    d.core
        .call_snapshot()
        .ringing
        .iter()
        .any(|r| r.call_id == call_id && r.conv_id == conv)
}

fn mark(d: &Dev, conv: &str, call_id: &str, op: &str) -> Option<CallMark> {
    d.core
        .talk_posts(&d.user(), conv, None, 50)
        .ok()?
        .into_iter()
        .filter_map(|p| p.call)
        .find(|c| c.call_id == call_id && c.op == op)
}

fn close(s: Setup) {
    for d in s.alice.into_iter().chain(s.bob) {
        d.core.close().unwrap();
    }
}

/// The card's case: the Mac and the phone (one person's two devices) in a
/// huddle, and a third device joining.
#[test]
fn own_devices_huddle_and_a_third_device_joins() {
    let s = setup(3, |s| s.alice.iter().collect());
    let (a1, a2, a3) = (&s.alice[0], &s.alice[1], &s.alice[2]);

    let call_id = a1.core.call_start(&s.ch).expect("start");
    assert_eq!(call_id.len(), 32);
    wait_for(WAIT, "a2 rings", || rung(a2, &call_id, &s.ch));
    a2.core.call_join(&call_id).expect("a2 joins");
    wait_for(WAIT, "a1 and a2 are connected", || {
        live_peers(a1) == 1 && live_peers(a2) == 1
    });
    let from = len(&s.recs[1]);
    wait_for(WAIT, "a1's voice comes out of a2's speaker", || {
        loud_blocks(&s.recs[1], from) >= 10
    });

    // The third device.
    wait_for(WAIT, "a3 sees the huddle", || {
        let sn = a3.core.call_snapshot();
        sn.ringing.iter().any(|r| r.call_id == call_id)
            || sn.live.iter().any(|l| l.call_id == call_id)
    });
    a3.core.call_join(&call_id).expect("a3 joins");
    wait_for(WAIT, "a full mesh of three", || {
        live_peers(a1) == 2 && live_peers(a2) == 2 && live_peers(a3) == 2
    });
    let from = len(&s.recs[2]);
    wait_for(WAIT, "a1's voice comes out of a3's speaker", || {
        loud_blocks(&s.recs[2], from) >= 10
    });
    let snap = a3.core.call_snapshot().mine.expect("a3 is in it");
    assert!(
        snap.peers
            .iter()
            .all(|p| p.path == "direct" && p.phase == "live"),
        "{snap:?}"
    );

    // Deafen: a1's voice stops coming out of a3's speaker, a3's own mic
    // goes off (a1 sees it muted), and Undeafen brings both back.
    a3.core.call_deafen(true);
    let m = a3.core.call_snapshot().mine.expect("a3 is in it");
    assert!(m.deafened && !m.muted, "{m:?}");
    std::thread::sleep(Duration::from_millis(600));
    let from = len(&s.recs[2]);
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(loud_blocks(&s.recs[2], from), 0, "a deafened speaker plays");
    wait_for(WAIT, "a1 sees a3's mic off", || {
        a1.core.call_snapshot().mine.is_some_and(|m| {
            m.peers
                .iter()
                .any(|p| p.device_id == a3.core.device_id() && p.muted)
        })
    });
    a3.core.call_deafen(false);
    let from = len(&s.recs[2]);
    wait_for(WAIT, "a1's voice plays at a3 again", || {
        loud_blocks(&s.recs[2], from) >= 10
    });
    assert!(!a3.core.call_snapshot().mine.expect("in it").deafened);

    // Mute: a1's voice stops, and the others see it.
    a1.core.call_mute(true);
    std::thread::sleep(Duration::from_millis(600));
    let from = len(&s.recs[2]);
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(loud_blocks(&s.recs[2], from), 0, "a muted mic is heard");
    wait_for(WAIT, "a3 sees a1 muted", || {
        a3.core.call_snapshot().mine.is_some_and(|m| {
            m.peers
                .iter()
                .any(|p| p.device_id == a1.core.device_id() && p.muted)
        })
    });

    // Out one by one: the last one writes the end post.
    a3.core.call_leave().unwrap();
    wait_for(WAIT, "a1 sees a3 gone", || live_peers(a1) == 1);
    a2.core.call_leave().unwrap();
    wait_for(WAIT, "a1 is alone", || live_peers(a1) == 0);
    // a1 still rings for its first minute; a2, which left, isn't rung
    // again for it.
    std::thread::sleep(Duration::from_secs(6));
    assert!(
        !rung(a2, &call_id, &s.ch),
        "a2 is rung again for the call it left"
    );
    assert!(
        mark(a1, &s.ch, &call_id, CALL_END).is_none(),
        "ended while a1 is still in it"
    );
    a1.core.call_leave().unwrap();
    assert!(a1.core.call_snapshot().mine.is_none());
    let end = mark(a1, &s.ch, &call_id, CALL_END).expect("the end post");
    assert_eq!(end.people, vec![a1.user()]);
    assert!(end.duration_ms > 0);
    assert!(mark(a1, &s.ch, &call_id, CALL_START).is_some());
    close(s);
}

/// Two people: Bob is rung in the channel, joins and hears Alice; the end
/// post names both and reaches Bob.
#[test]
fn two_people_huddle_in_a_space_channel() {
    let s = setup(1, |s| vec![&s.alice[0], &s.bob[0]]);
    let (a1, b1) = (&s.alice[0], &s.bob[0]);
    let call_id = a1.core.call_start(&s.ch).expect("start");
    wait_for(WAIT, "bob rings", || rung(b1, &call_id, &s.ch));
    b1.core.call_join(&call_id).expect("bob joins");
    wait_for(WAIT, "connected", || {
        live_peers(a1) == 1 && live_peers(b1) == 1
    });
    let from = len(&s.bob_rec);
    wait_for(WAIT, "alice's voice comes out of bob's speaker", || {
        loud_blocks(&s.bob_rec, from) >= 10
    });
    let peer = b1.core.call_snapshot().mine.unwrap().peers.remove(0);
    assert!(!peer.person_id.is_empty());
    b1.core.call_leave().unwrap();
    wait_for(WAIT, "alice sees bob gone", || live_peers(a1) == 0);
    a1.core.call_leave().unwrap();
    wait_for(WAIT, "bob has the end post", || {
        mark(b1, &s.ch, &call_id, CALL_END).is_some()
    });
    let end = mark(a1, &s.ch, &call_id, CALL_END).unwrap();
    assert_eq!(end.people.len(), 2, "{end:?}");
    close(s);
}

/// #663: one person's three devices (the Mac, the phone and the PC) in a
/// Personal channel, no space anywhere: the Mac rings both, each joins, and
/// the Mac's voice comes out of both speakers over a full mesh of three.
#[test]
fn own_devices_huddle_in_a_personal_channel() {
    let recs: Vec<_> = (0..3)
        .map(|_| Arc::new(Mutex::new(Recording::default())))
        .collect();
    let alice = {
        let recs = recs.clone();
        person_with("alice", 3, move |i| {
            Some(fake_audio(i == 0, recs[i].clone()))
        })
    };
    let (mac, phone, pc) = (&alice[0], &alice[1], &alice[2]);
    let ch = mac
        .core
        .talk_channel_create(&mac.user(), "us", "")
        .unwrap()
        .id;
    for d in [phone, pc] {
        wait_for(WAIT, "the channel reaches the other devices", || {
            d.core
                .talk_conversations(&d.user())
                .is_ok_and(|cs| cs.iter().any(|c| c.conv_id == ch))
        });
    }
    wait_for(
        WAIT,
        "the devices reach each other on the private mesh",
        || {
            alice.iter().all(|d| {
                let r = d.core.call_reachable("");
                alice
                    .iter()
                    .filter(|o| o.core.device_id() != d.core.device_id())
                    .all(|o| r.contains(&o.core.device_id().to_string()))
            })
        },
    );

    let call_id = mac.core.call_start(&ch).expect("start");
    wait_for(WAIT, "the phone and the PC ring", || {
        rung(phone, &call_id, &ch) && rung(pc, &call_id, &ch)
    });
    let ring = phone.core.call_snapshot().ringing.remove(0);
    assert_eq!(ring.space_id, "");
    phone.core.call_join(&call_id).expect("the phone joins");
    wait_for(WAIT, "the mac and the phone are connected", || {
        live_peers(mac) == 1 && live_peers(phone) == 1
    });
    // Answered on the phone: the PC still rings, since every device here
    // is the person's own.
    assert!(rung(pc, &call_id, &ch), "the PC stopped ringing");
    pc.core.call_join(&call_id).expect("the PC joins");
    wait_for(WAIT, "a full mesh of three", || {
        live_peers(mac) == 2 && live_peers(phone) == 2 && live_peers(pc) == 2
    });
    for (i, who) in [(1, "phone"), (2, "PC")] {
        let from = len(&recs[i]);
        wait_for(
            WAIT,
            &format!("the mac's voice comes out of the {who}"),
            || loud_blocks(&recs[i], from) >= 10,
        );
    }
    let mine = pc.core.call_snapshot().mine.expect("the PC is in it");
    assert_eq!(mine.space_id, "");
    assert!(mine.peers.iter().all(|p| p.phase == "live"), "{mine:?}");

    pc.core.call_leave().unwrap();
    phone.core.call_leave().unwrap();
    wait_for(WAIT, "the mac is alone", || live_peers(mac) == 0);
    mac.core.call_leave().unwrap();
    let end = mark(mac, &ch, &call_id, CALL_END).expect("the end post");
    assert_eq!(end.people, vec![mac.user()]);
    wait_for(WAIT, "the phone has the end post", || {
        mark(phone, &ch, &call_id, CALL_END).is_some()
    });
    for d in alice {
        d.core.close().unwrap();
    }
}

/// The `call.segment` rows in space `space`'s log on `c`.
fn segment_rows(c: &Core, space: &str) -> Vec<store::Event> {
    c.board_store(space)
        .unwrap()
        .list(&Filter {
            kinds: vec![kinds::CALL_SEGMENT.into()],
            ..Default::default()
        })
        .unwrap()
}

/// The captions in `d`'s call from device `from`.
fn captions_from(d: &Dev, from: &str) -> Vec<irori_core::Caption> {
    d.core.call_snapshot().mine.map_or(Vec::new(), |m| {
        m.captions
            .into_iter()
            .filter(|c| c.device_id == from)
            .collect()
    })
}

/// Whether `d`'s call sees peer `peer` as transcribing `on`.
fn sees_transcribing(d: &Dev, peer: &str, on: Option<bool>) -> bool {
    d.core.call_snapshot().mine.is_some_and(|m| {
        m.peers
            .iter()
            .any(|p| p.device_id == peer && p.transcribing == on)
    })
}

/// Rings `b` in `a`'s new huddle in `ch` and waits for them to connect.
fn huddle(a: &Dev, b: &Dev, ch: &str) -> String {
    let call_id = a.core.call_start(ch).expect("start");
    wait_for(WAIT, "the other device rings", || rung(b, &call_id, ch));
    b.core.call_join(&call_id).expect("join");
    wait_for(WAIT, "connected", || {
        live_peers(a) == 1 && live_peers(b) == 1
    });
    call_id
}

/// #656: Alice speaks, Bob doesn't. Alice's lines are rows in the space's
/// log signed by her device, show live as captions on Bob's device, fold
/// there as the call's lines by Alice, and the end post says there's a
/// transcript.
#[test]
fn a_huddle_writes_the_speakers_lines() {
    let s = setup_with(1, true, |s| vec![&s.alice[0], &s.bob[0]]);
    let (a1, b1) = (&s.alice[0], &s.bob[0]);
    let (a_dev, b_dev) = (
        a1.core.device_id().to_string(),
        b1.core.device_id().to_string(),
    );
    let call_id = huddle(a1, b1, &s.ch);

    wait_for(WAIT, "bob sees alice's lines live", || {
        captions_from(b1, &a_dev).len() >= 2
    });
    let c = &captions_from(b1, &a_dev)[0];
    assert!(!c.mine && c.text.starts_with("alice "), "{c:?}");
    assert_eq!(c.person_id, a1.core.person().id);
    assert!(
        c.end_ms >= c.start_ms && c.start_ms > 1_700_000_000_000,
        "{c:?}"
    );
    wait_for(WAIT, "each sees the other transcribing", || {
        sees_transcribing(b1, &a_dev, Some(true)) && sees_transcribing(a1, &b_dev, Some(true))
    });
    let mine = a1.core.call_snapshot().mine.unwrap();
    assert!(mine.transcribing && mine.can_transcribe);
    assert!(
        mine.captions.iter().any(|c| c.mine && c.device_id == a_dev),
        "{mine:?}"
    );
    // Bob is quiet: no line of his.
    assert!(captions_from(a1, &b_dev).is_empty());

    let rows = segment_rows(&a1.core, &s.space);
    assert!(!rows.is_empty());
    for r in &rows {
        assert_eq!(r.actor, a1.person_actor());
        assert_eq!(r.device_id, a_dev);
    }
    wait_for(WAIT, "bob folds alice's lines", || {
        !b1.core.talk().call_posts(&call_id).is_empty()
    });
    let lines: Vec<_> = b1
        .core
        .talk()
        .call_posts(&call_id)
        .into_iter()
        .filter(|p| p.call.is_none())
        .collect();
    assert!(!lines.is_empty());
    for p in &lines {
        assert!(p.text.starts_with("alice "), "{p:?}");
        assert!(p.by.contains(&a1.core.person().id), "{p:?}");
        assert_eq!(p.conv_id, format!("call:{call_id}"));
    }

    b1.core.call_leave().unwrap();
    wait_for(WAIT, "alice sees bob gone", || live_peers(a1) == 0);
    a1.core.call_leave().unwrap();
    let end = mark(a1, &s.ch, &call_id, CALL_END).expect("the end post");
    assert!(end.transcript, "{end:?}");
    wait_for(WAIT, "bob has the end post", || {
        mark(b1, &s.ch, &call_id, CALL_END).is_some_and(|m| m.transcript)
    });
    close(s);
}

/// #656: with "Transcribe what I say in calls" off, Alice's device writes
/// nothing and Bob sees it isn't transcribing; turned on mid-call it
/// writes, and turned off again it stops.
#[test]
fn the_switch_stops_the_transcript() {
    let s = setup_with(1, true, |s| vec![&s.alice[0], &s.bob[0]]);
    let (a1, b1) = (&s.alice[0], &s.bob[0]);
    let a_dev = a1.core.device_id().to_string();
    a1.core.set_call_transcribe(false).unwrap();
    let call_id = huddle(a1, b1, &s.ch);

    wait_for(WAIT, "bob sees alice isn't transcribing", || {
        sees_transcribing(b1, &a_dev, Some(false))
    });
    let mine = a1.core.call_snapshot().mine.unwrap();
    assert!(!mine.transcribing && mine.can_transcribe, "{mine:?}");
    // Two phrases' worth of talking.
    let from = len(&s.bob_rec);
    wait_for(WAIT, "alice's voice comes out of bob's speaker", || {
        loud_blocks(&s.bob_rec, from) >= 10
    });
    std::thread::sleep(Duration::from_secs(6));
    assert!(captions_from(b1, &a_dev).is_empty());
    assert!(segment_rows(&a1.core, &s.space).is_empty());

    // On mid-call.
    a1.core.set_call_transcribe(true).unwrap();
    wait_for(WAIT, "bob sees alice transcribing", || {
        sees_transcribing(b1, &a_dev, Some(true))
    });
    wait_for(WAIT, "alice's lines come", || {
        !captions_from(b1, &a_dev).is_empty() && !segment_rows(&a1.core, &s.space).is_empty()
    });

    // Off again: once Bob hears so, no more lines.
    a1.core.set_call_transcribe(false).unwrap();
    wait_for(WAIT, "bob sees alice stop", || {
        sees_transcribing(b1, &a_dev, Some(false))
    });
    assert!(!a1.core.call_snapshot().mine.unwrap().transcribing);
    let (caps, rows) = (
        captions_from(b1, &a_dev).len(),
        segment_rows(&a1.core, &s.space).len(),
    );
    std::thread::sleep(Duration::from_secs(7));
    assert_eq!(captions_from(b1, &a_dev).len(), caps);
    assert_eq!(segment_rows(&a1.core, &s.space).len(), rows);

    b1.core.call_leave().unwrap();
    wait_for(WAIT, "alice sees bob gone", || live_peers(a1) == 0);
    a1.core.call_leave().unwrap();
    assert!(mark(a1, &s.ch, &call_id, CALL_END).unwrap().transcript);
    close(s);
}

/// #656: a Personal channel's lines go to the private log and fold on the
/// person's other device.
#[test]
fn a_personal_huddle_writes_to_the_private_log() {
    let recs: Vec<_> = (0..2)
        .map(|_| Arc::new(Mutex::new(Recording::default())))
        .collect();
    let alice = {
        let recs = recs.clone();
        person_with("alice", 2, move |i| {
            Some(scribe_audio(i == 0, recs[i].clone(), "alice"))
        })
    };
    let (mac, phone) = (&alice[0], &alice[1]);
    let mac_dev = mac.core.device_id().to_string();
    let ch = mac
        .core
        .talk_channel_create(&mac.user(), "us", "")
        .unwrap()
        .id;
    wait_for(WAIT, "the channel reaches the phone", || {
        phone
            .core
            .talk_conversations(&phone.user())
            .is_ok_and(|cs| cs.iter().any(|c| c.conv_id == ch))
    });
    wait_for(WAIT, "the devices reach each other", || {
        mac.core
            .call_reachable("")
            .contains(&phone.core.device_id().to_string())
            && phone.core.call_reachable("").contains(&mac_dev)
    });
    let call_id = huddle(mac, phone, &ch);

    wait_for(WAIT, "the phone sees the mac's lines", || {
        !captions_from(phone, &mac_dev).is_empty()
    });
    wait_for(WAIT, "the phone folds them", || {
        phone
            .core
            .talk()
            .call_posts(&call_id)
            .iter()
            .any(|p| p.call.is_none() && p.text.starts_with("alice "))
    });
    let rows = mac
        .core
        .store()
        .list(&Filter {
            kinds: vec![kinds::CALL_SEGMENT.into()],
            ..Default::default()
        })
        .unwrap();
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|r| r.device_id == mac_dev));

    phone.core.call_leave().unwrap();
    wait_for(WAIT, "the mac is alone", || live_peers(mac) == 0);
    mac.core.call_leave().unwrap();
    assert!(mark(mac, &ch, &call_id, CALL_END).unwrap().transcript);
    for d in alice {
        d.core.close().unwrap();
    }
}

/// #656: when a device in the space may be too old for `call.segment` rows
/// as the call begins (Alice's other device, offline, last said an old
/// `tolerant`), the lines still show live but are kept, and written as one
/// fallback post at leave that folds into the call on Bob's device and
/// isn't a channel post.
#[test]
fn a_space_not_ready_gets_one_fallback_post() {
    let mut s = setup_with(2, true, |s| vec![&s.alice[0], &s.bob[0]]);
    let old = s.alice.pop().unwrap();
    let old_dev = old.core.device_id().to_string();
    old.core.close().unwrap();
    let (a1, b1) = (&s.alice[0], &s.bob[0]);
    let a_dev = a1.core.device_id().to_string();
    a1.core
        .board_store(&s.space)
        .unwrap()
        .append_local(
            "irori",
            mesh::boardmesh::KIND_BOARD_HELLO,
            &mesh::boardmesh::SavedHello {
                device_id: old_dev,
                core_version: "0.12.4".into(),
                spaces: 1,
                spawns: 1,
                tolerant: 1,
                seq: i64::MAX,
                ..Default::default()
            },
        )
        .unwrap();
    wait_for(WAIT, "alice's space isn't ready for call rows", || {
        !a1.core.space_tolerance(&s.space).unwrap().ready()
    });
    let call_id = huddle(a1, b1, &s.ch);

    wait_for(WAIT, "bob sees alice's lines live", || {
        captions_from(b1, &a_dev).len() >= 2
    });
    assert!(segment_rows(&a1.core, &s.space).is_empty());
    assert!(
        b1.core
            .talk()
            .call_posts(&call_id)
            .iter()
            .all(|p| p.call.is_some())
    );

    b1.core.call_leave().unwrap();
    wait_for(WAIT, "alice sees bob gone", || live_peers(a1) == 0);
    a1.core.call_leave().unwrap();
    assert!(segment_rows(&a1.core, &s.space).is_empty());
    let end = mark(a1, &s.ch, &call_id, CALL_END).expect("the end post");
    assert!(end.transcript);
    wait_for(WAIT, "bob folds the fallback", || {
        b1.core
            .talk()
            .call_posts(&call_id)
            .iter()
            .filter(|p| p.call.is_none())
            .count()
            >= 2
    });
    let lines: Vec<_> = b1
        .core
        .talk()
        .call_posts(&call_id)
        .into_iter()
        .filter(|p| p.call.is_none())
        .collect();
    assert!(
        lines.iter().all(|p| p.text.starts_with("alice ")),
        "{lines:?}"
    );
    // One post, not a channel post.
    let ids: std::collections::BTreeSet<_> = lines
        .iter()
        .map(|p| p.post_id.split('#').next().unwrap_or_default())
        .collect();
    assert_eq!(ids.len(), 1, "{lines:?}");
    let posts = b1.core.talk_posts(&b1.user(), &s.ch, None, 50).unwrap();
    assert!(posts.iter().all(|p| p.transcript.is_none()), "{posts:?}");
    let all = a1.core.talk().posts(&s.ch);
    assert!(all.iter().all(|p| p.transcript.is_none()));
    close(s);
}

/// The segs of device `dev`'s lines in huddle `call_id`, as `d` folds them.
fn segs(d: &Dev, call_id: &str, dev: &str) -> Vec<i64> {
    d.core
        .talk()
        .call_posts(call_id)
        .into_iter()
        .filter(|p| p.post_id.starts_with("seg_") && p.device == dev)
        .map(|p| p.ord.2)
        .collect()
}

/// #656: Alice leaves the huddle and joins it again. Her new lines count
/// on from her old ones, so they're written and fold, the call's lines
/// grow, and Bob sees them live.
#[test]
fn a_rejoin_keeps_the_speakers_lines() {
    let s = setup_with(1, true, |s| vec![&s.alice[0], &s.bob[0]]);
    let (a1, b1) = (&s.alice[0], &s.bob[0]);
    let a_dev = a1.core.device_id().to_string();
    let call_id = huddle(a1, b1, &s.ch);
    wait_for(WAIT, "bob sees alice's lines live", || {
        captions_from(b1, &a_dev).len() >= 2
    });

    a1.core.call_leave().unwrap();
    wait_for(WAIT, "bob sees alice gone", || live_peers(b1) == 0);
    let before = segs(a1, &call_id, &a_dev);
    assert!(!before.is_empty());
    assert_eq!(segment_rows(&a1.core, &s.space).len(), before.len());
    let last = *before.iter().max().unwrap();
    wait_for(WAIT, "bob folds alice's first lines", || {
        segs(b1, &call_id, &a_dev).len() == before.len()
    });
    let caps = captions_from(b1, &a_dev).len();

    wait_for(WAIT, "alice sees the huddle go on", || {
        a1.core
            .call_snapshot()
            .live
            .iter()
            .any(|l| l.call_id == call_id)
    });
    a1.core.call_join(&call_id).expect("alice joins again");
    wait_for(WAIT, "connected again", || {
        live_peers(a1) == 1 && live_peers(b1) == 1
    });
    wait_for(WAIT, "bob sees alice's new lines live", || {
        captions_from(b1, &a_dev).len() >= caps + 2
    });
    let new: Vec<_> = captions_from(b1, &a_dev)
        .into_iter()
        .filter(|c| c.seq > last as u64)
        .collect();
    assert!(new.len() >= 2, "{new:?}");
    wait_for(WAIT, "bob folds alice's new lines", || {
        segs(b1, &call_id, &a_dev).len() >= before.len() + 2
    });
    let after = segs(b1, &call_id, &a_dev);
    let mut uniq = after.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), after.len(), "{after:?}");
    assert!(
        after.iter().filter(|g| **g > last).count() >= 2,
        "{after:?}"
    );

    b1.core.call_leave().unwrap();
    wait_for(WAIT, "alice sees bob gone", || live_peers(a1) == 0);
    a1.core.call_leave().unwrap();
    assert_eq!(
        segment_rows(&a1.core, &s.space).len(),
        segs(a1, &call_id, &a_dev).len()
    );
    assert!(mark(a1, &s.ch, &call_id, CALL_END).unwrap().transcript);
    close(s);
}

/// What reached `d`'s agent referring to `conv`.
fn told(d: &Dev, conv: &str) -> Vec<inbox::Delivery> {
    d.core
        .inbox()
        .list(0)
        .into_iter()
        .filter(|x| x.to == d.agent && x.message.ref_ == format!("talk:{conv}"))
        .collect()
}

/// What reached `d`'s agent naming huddle `call_id`.
fn told_of(d: &Dev, call_id: &str) -> usize {
    d.core
        .inbox()
        .list(0)
        .into_iter()
        .filter(|x| x.to == d.agent && x.message.text.contains(call_id))
        .count()
}

/// #656, through the real leave: a channel huddle with lines wakes the
/// channel's agents once on each person's side (Alice's from her own
/// device, which ends it; Bob's from the end post); one with no lines and
/// one in a person DM wake nobody.
#[test]
fn a_huddles_end_wakes_the_channels_agents_once() {
    let s = setup_with(1, true, |s| vec![&s.alice[0], &s.bob[0]]);
    let (a1, b1) = (&s.alice[0], &s.bob[0]);
    let a_dev = a1.core.device_id().to_string();
    wait_for(WAIT, "bob holds the channel", || {
        b1.core.talk().channel(&s.ch).is_some()
    });
    for d in [a1, b1] {
        let out = d.tool("talk_join", serde_json::json!({"conv": s.ch}));
        assert!(!out.starts_with("Error"), "talk_join: {out}");
    }

    // No lines: Alice's switch is off and Bob is quiet.
    a1.core.set_call_transcribe(false).unwrap();
    let quiet = huddle(a1, b1, &s.ch);
    std::thread::sleep(Duration::from_secs(4));
    b1.core.call_leave().unwrap();
    wait_for(WAIT, "alice sees bob gone", || live_peers(a1) == 0);
    a1.core.call_leave().unwrap();
    assert!(!mark(a1, &s.ch, &quiet, CALL_END).unwrap().transcript);
    wait_for(WAIT, "bob has the quiet huddle's end", || {
        mark(b1, &s.ch, &quiet, CALL_END).is_some()
    });

    // With lines.
    a1.core.set_call_transcribe(true).unwrap();
    let call_id = huddle(a1, b1, &s.ch);
    wait_for(WAIT, "bob sees alice's lines live", || {
        !captions_from(b1, &a_dev).is_empty()
    });
    b1.core.call_leave().unwrap();
    wait_for(WAIT, "alice sees bob gone", || live_peers(a1) == 0);
    a1.core.call_leave().unwrap();
    assert!(mark(a1, &s.ch, &call_id, CALL_END).unwrap().transcript);
    wait_for(WAIT, "both agents are woken", || {
        !told(a1, &s.ch).is_empty() && !told(b1, &s.ch).is_empty()
    });

    // A person DM's huddle, Alice alone in it.
    let dm = a1.core.talk_dm(&a1.user(), &b1.person_actor()).unwrap();
    a1.core
        .talk_post(
            &a1.core.user_sender(),
            &dm,
            "hi bob",
            &[],
            false,
            None,
            vec![],
        )
        .unwrap();
    let dm_call = std::cell::RefCell::new(String::new());
    wait_for(WAIT, "a huddle starts in the DM", || {
        match a1.core.call_start(&dm) {
            Ok(id) => {
                *dm_call.borrow_mut() = id;
                true
            }
            Err(_) => false,
        }
    });
    let dm_call = dm_call.into_inner();
    wait_for(WAIT, "alice's line in the DM", || {
        a1.core
            .call_snapshot()
            .mine
            .is_some_and(|m| m.captions.iter().any(|c| c.mine))
    });
    a1.core.call_leave().unwrap();
    assert!(mark(a1, &dm, &dm_call, CALL_END).unwrap().transcript);

    // Once each, however often the log is pulled.
    a1.core.kick_board(&s.space);
    b1.core.kick_board(&s.space);
    std::thread::sleep(Duration::from_secs(3));
    for d in [a1, b1] {
        let t = told(d, &s.ch);
        assert_eq!(t.len(), 1, "{t:?}");
        assert_eq!(t[0].mode, inbox::mode::WAKE);
        assert!(
            t[0].message
                .text
                .contains(&format!("talk_read conv=call:{call_id}")),
            "{}",
            t[0].message.text
        );
        assert_eq!(told_of(d, &quiet), 0);
        assert_eq!(told_of(d, &dm_call), 0);
        assert!(told(d, &dm).is_empty());
    }
    close(s);
}

/// #658: a screen share in a two-person huddle. Alice shares the test
/// picture (the real hardware encoder, no capture), Bob watches: his sink
/// gets a decoder config, then AVCC access units starting at a keyframe.
/// Bob sharing replaces Alice's, and she is told who took over; Bob stops
/// and the share is gone for both.
///
/// Ignored until #671: on the PC (NVIDIA) the test binary crashes, alone
/// or beside the other huddle tests (about 1 run in 4), in NVIDIA's H.264
/// encoder (nvEncMFTH264x.dll) on a Media Foundation work-queue thread.
#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
#[ignore = "#671: crashes in the NVIDIA H.264 encoder on Windows"]
fn a_share_in_a_huddle_reaches_the_watcher_and_newest_wins() {
    use irori_net::share::{VIEW_CHUNK, VIEW_CONFIG};

    let s = setup(1, |s| vec![&s.alice[0], &s.bob[0]]);
    let (a1, b1) = (&s.alice[0], &s.bob[0]);
    let call_id = a1.core.call_start(&s.ch).expect("start");
    wait_for(WAIT, "bob rings", || rung(b1, &call_id, &s.ch));
    b1.core.call_join(&call_id).expect("bob joins");
    wait_for(WAIT, "connected", || {
        live_peers(a1) == 1 && live_peers(b1) == 1
    });

    let got: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
    let sink: irori_core::ShareSink = {
        let got = got.clone();
        Arc::new(move |b| {
            got.lock().unwrap().push(b);
            true
        })
    };
    b1.core.share_attach(sink).expect("bob watches");
    a1.core
        .share_start("synthetic:640x360")
        .expect("alice shares");
    let mine = a1.core.call_snapshot().mine.unwrap();
    let share = mine.share.expect("alice sees her share");
    assert!(share.now.mine);
    assert!(mine.can_share);
    wait_for(WAIT, "bob sees alice's share", || {
        b1.core
            .call_snapshot()
            .mine
            .and_then(|m| m.share)
            .is_some_and(|x| !x.now.mine && x.now.info.width == 640 && x.now.info.height == 360)
    });
    wait_for(WAIT, "bob gets a config and 20 access units", || {
        got.lock()
            .unwrap()
            .iter()
            .filter(|b| b[0] == VIEW_CHUNK)
            .count()
            >= 20
    });
    {
        let g = got.lock().unwrap();
        assert_eq!(g[0][0], VIEW_CONFIG, "a config comes first");
        let codec_len = u16::from_be_bytes([g[0][1], g[0][2]]) as usize;
        let codec = std::str::from_utf8(&g[0][3..3 + codec_len]).unwrap();
        assert!(codec.starts_with("avc1."), "{codec}");
        let first = g.iter().find(|b| b[0] == VIEW_CHUNK).unwrap();
        assert_eq!(first[1] & 1, 1, "the first access unit is a keyframe");
        // AVCC: a 4-byte length, then the NAL unit; no start codes.
        for b in g.iter().filter(|b| b[0] == VIEW_CHUNK) {
            let au = &b[10..];
            let mut i = 0;
            while i < au.len() {
                let n = u32::from_be_bytes(au[i..i + 4].try_into().unwrap()) as usize;
                assert!(n > 0 && i + 4 + n <= au.len(), "bad AVCC length");
                i += 4 + n;
            }
        }
    }

    // Bob looks away and back (the stage unmounts and mounts again): the
    // sharer starts a new sender for him, and he gets a fresh config and
    // keyframe, not a black picture.
    b1.core.share_detach();
    let again: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
    let sink2: irori_core::ShareSink = {
        let again = again.clone();
        Arc::new(move |b| {
            again.lock().unwrap().push(b);
            true
        })
    };
    b1.core.share_attach(sink2).expect("bob watches again");
    // Within a few seconds: a keyframe ask waits at most 2 s (and a sender
    // that started its GOP numbers over would be ignored until the next
    // unasked keyframe, 10 s on).
    let t = std::time::Instant::now();
    loop {
        let ok = {
            let g = again.lock().unwrap();
            g.first().is_some_and(|b| b[0] == VIEW_CONFIG)
                && g.iter().filter(|b| b[0] == VIEW_CHUNK).count() >= 5
        };
        if ok {
            break;
        }
        assert!(
            t.elapsed() < Duration::from_secs(6),
            "bob got no picture within 6 s of watching again"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // Bob shares: Alice's stops and she's told it was Bob.
    b1.core
        .share_start("synthetic:320x240")
        .expect("bob shares");
    wait_for(WAIT, "alice's share gives way to bob's", || {
        let m = a1.core.call_snapshot().mine.unwrap();
        m.share
            .as_ref()
            .is_some_and(|x| !x.now.mine && x.now.info.width == 320)
            && matches!(
                m.share_note,
                Some(irori_core::CallShareNote::Replaced { .. })
            )
    });
    b1.core.share_stop();
    wait_for(WAIT, "the share is gone for both", || {
        a1.core.call_snapshot().mine.unwrap().share.is_none()
            && b1.core.call_snapshot().mine.unwrap().share.is_none()
    });
    b1.core.call_leave().unwrap();
    a1.core.call_leave().unwrap();
    close(s);
}
