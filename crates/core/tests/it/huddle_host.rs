//! Which computer hosts a person's assistant in a huddle (card #1049,
//! docs/design/call-agents.md §7, §8) through real Cores on loopback iroh.
//! Alice has a phone and two computers; the phone is in the call, the
//! computers are not. Exactly one computer holds the held session (card
//! #1046, a fake runner here), `call.assist` rows arbitrate, a paused or
//! gone host hands over, and the session closes when she leaves. Made-up
//! people only.

use crate::common;

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use call::device::fake::{self, Recording};
use common::mesh::{Dev, Edit, person_with};
use common::{wait_for, write_session};
use irori_core::spawn::{SpawnReq, SpawnSetting, SpawnStarted, Spawner};
use irori_core::{HuddleRunner, Look, LookError};

const WAIT: u64 = 90;

/// A held-session runner that only counts what it was asked, and can
/// answer a look as a signed-out Claude Code would.
#[derive(Default)]
struct Runner {
    log: Mutex<Vec<String>>,
    signed_out: AtomicBool,
}

impl Runner {
    fn count(&self, what: &str) -> usize {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|w| *w == what)
            .count()
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
        self.log.lock().unwrap().push("open".into());
        Ok(ok())
    }
    fn look(&self, _call: &str, _message: &str) -> Result<Look, LookError> {
        self.log.lock().unwrap().push("look".into());
        if self.signed_out.load(Ordering::SeqCst) {
            return Err(LookError::Failed(
                "Not logged in \u{b7} Please run /login".into(),
            ));
        }
        Ok(ok())
    }
    fn close(&self, _call: &str) {
        self.log.lock().unwrap().push("close".into());
    }
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

fn skewed(skew: Arc<AtomicI64>) -> Edit {
    Arc::new(move |o: &mut irori_core::Options| {
        let sk = skew.clone();
        o.now = Some(Arc::new(move || {
            let d = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap();
            gocompat::time::GoTime::unix(
                d.as_secs() as i64 + sk.load(Ordering::SeqCst),
                i64::from(d.subsec_nanos()),
            )
        }));
    })
}

fn fake_audio(rec: Arc<Mutex<Recording>>) -> Edit {
    Arc::new(move |o| {
        let rec = rec.clone();
        o.call_audio = Some(Arc::new(move || {
            let silence: fake::Gen = Box::new(|_| 0.0);
            fake::devices(silence, rec.clone())
        }));
    })
}

struct Rig {
    phone: Dev,
    /// Computers with a runner each.
    macs: Vec<(Dev, Arc<Runner>)>,
    ch: String,
    skews: Vec<Arc<AtomicI64>>,
}

/// Alice with a phone and `computers` computers (each with a spawner and a
/// fake held-session runner), a Personal channel, and the devices reaching
/// each other on the private mesh.
fn rig(computers: usize) -> Rig {
    let skews: Vec<Arc<AtomicI64>> = (0..=computers).map(|_| Arc::default()).collect();
    let sk = skews.clone();
    let mut devs = person_with("alice", computers + 1, move |i| {
        let rec = Arc::new(Mutex::new(Recording::default()));
        let (s, a) = (skewed(sk[i].clone()), fake_audio(rec));
        Some(Arc::new(move |o: &mut irori_core::Options| {
            s(o);
            a(o);
        }) as Edit)
    });
    let phone = devs.remove(0);
    phone.core.set_runs_no_agents();
    let mut macs = Vec::new();
    for d in devs {
        d.core
            .set_spawn_setting(SpawnSetting {
                on: true,
                ..SpawnSetting::default()
            })
            .unwrap();
        d.core.set_spawner(Some(spawner(d.claude.join("sessions"))));
        let r = Arc::new(Runner::default());
        d.core.set_huddle_runner(Some(r.clone()));
        macs.push((d, r));
    }
    let ch = phone
        .core
        .talk_channel_create(&phone.user(), "us", "")
        .unwrap()
        .id;
    let all: Vec<&Dev> = std::iter::once(&phone)
        .chain(macs.iter().map(|(d, _)| d))
        .collect();
    for d in &all {
        wait_for(WAIT, "the channel reaches every device", || {
            d.core
                .talk_conversations(&d.user())
                .is_ok_and(|cs| cs.iter().any(|c| c.conv_id == ch))
        });
    }
    wait_for(
        WAIT,
        "the devices reach each other on the private mesh",
        || {
            all.iter().all(|d| {
                let r = d.core.call_reachable("");
                all.iter()
                    .filter(|o| o.core.device_id() != d.core.device_id())
                    .all(|o| r.contains(&o.core.device_id().to_string()))
            })
        },
    );
    // Every computer knows the others run agents, so they rank each other.
    for (d, _) in &macs {
        for (o, _) in macs
            .iter()
            .filter(|(o, _)| o.core.device_id() != d.core.device_id())
        {
            let id = o.core.device_id().to_string();
            wait_for(WAIT, "a computer hears the other's spawn.host", || {
                d.core.spawn_hosts().contains_key(&id)
            });
        }
    }
    Rig {
        phone,
        macs,
        ch,
        skews,
    }
}

impl Rig {
    fn close(self) {
        self.phone.core.close().unwrap();
        for (d, _) in self.macs {
            d.core.close().unwrap();
        }
    }

    /// The computers that hold a session of `call` now.
    fn hosts(&self, call: &str) -> Vec<usize> {
        self.macs
            .iter()
            .enumerate()
            .filter(|(_, (d, _))| d.core.huddle_is_open(call))
            .map(|(i, _)| i)
            .collect()
    }

    /// The device ids whose newest row says `on`, as computer `i` folds it.
    fn on_rows(&self, i: usize, call: &str) -> Vec<String> {
        let mut v: Vec<String> = self.macs[i]
            .0
            .core
            .talk()
            .call_assists(call)
            .into_iter()
            .filter(|a| a.on)
            .map(|a| a.device)
            .collect();
        v.sort();
        v
    }
}

impl Rig {
    /// Waits until exactly one computer holds `call`'s session and both
    /// computers' rows name it alone (the two may have picked themselves
    /// first); returns its index.
    fn settled(&self, call: &str) -> usize {
        wait_for(WAIT, "one computer alone holds the session", || {
            let h = self.hosts(call);
            h.len() == 1 && {
                let me = vec![id(&self.macs[h[0]].0)];
                self.on_rows(0, call) == me && self.on_rows(1, call) == me
            }
        });
        self.hosts(call)[0]
    }
}

fn id(d: &Dev) -> String {
    d.core.device_id().to_string()
}

/// The phone starts the huddle (and is the one in it); the computers are
/// rung but never join.
fn phone_calls(r: &Rig) -> String {
    let call = r.phone.core.call_start(&r.ch).expect("start");
    for (d, _) in &r.macs {
        wait_for(WAIT, "a computer hears of the call", || {
            d.core
                .call_snapshot()
                .ringing
                .iter()
                .any(|g| g.call_id == call)
        });
    }
    call
}

#[test]
fn two_computers_end_with_one_host_and_it_follows_the_person_in_and_out() {
    let r = rig(2);
    let call = phone_calls(&r);

    // The two may pick themselves at first (each learns of the other's
    // `spawn.host` at its own time); the rows settle it on one.
    let first = r.settled(&call);
    let second = 1 - first;
    assert!(r.macs[first].0.core.huddle_hosting(&call));
    assert!(!r.macs[second].0.core.huddle_hosting(&call));

    // The roster: the phone (in the call) sees Alice's assistant connected.
    wait_for(
        WAIT,
        "the phone's roster shows the assistant connected",
        || {
            r.phone
                .core
                .call_assistants(&call)
                .iter()
                .any(|a| a.person == r.phone.user() && a.state == "connected")
        },
    );

    // The host's agents are paused: it stops at once on its own pick, and
    // the other computer takes over.
    r.macs[first].0.core.set_agents_paused(true).unwrap();
    assert!(
        !r.macs[first].0.core.huddle_hosting(&call),
        "a look finds out it is no longer the host"
    );
    wait_for(WAIT, "the other computer holds the session", || {
        r.hosts(&call) == vec![second]
    });
    wait_for(WAIT, "both see the other computer's `on` alone", || {
        r.on_rows(0, &call) == vec![id(&r.macs[second].0)]
            && r.on_rows(1, &call) == vec![id(&r.macs[second].0)]
    });

    // The first is back: the second keeps the role (one host, no second
    // priming turn on the first).
    r.macs[first].0.core.set_agents_paused(false).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(6));
    assert_eq!(r.hosts(&call), vec![second]);
    assert_eq!(r.settled(&call), second);
    assert!(!r.macs[first].0.core.huddle_hosting(&call));
    assert!(r.macs[second].0.core.huddle_hosting(&call));

    // Alice leaves on the phone: the host closes and says `off`.
    r.phone.core.call_leave().unwrap();
    wait_for(WAIT, "the host closes its session", || {
        r.hosts(&call).is_empty()
    });
    wait_for(WAIT, "no `on` row is left", || {
        r.on_rows(0, &call).is_empty() && r.on_rows(1, &call).is_empty()
    });
    r.close();
}

#[test]
fn a_host_away_for_a_minute_hands_over_to_the_next_computer() {
    let r = rig(2);
    let call = phone_calls(&r);
    r.settled(&call);
    let Rig {
        phone,
        mut macs,
        skews,
        ..
    } = r;
    let host = r_host(&macs, &call);
    let other = 1 - host;
    // Computer i's clock is skews[i + 1] (the phone's is skews[0]).
    let rest_skew = skews[other + 1].clone();
    // The host goes away; the other sees it away, then a minute passes on
    // its clock.
    let (gone, _) = macs.remove(host);
    let gone_id = id(&gone);
    gone.core.close().unwrap();
    let (rest, runner) = &macs[0];
    let rest_id = id(rest);
    wait_for(
        WAIT,
        "the other computer still sees the host's `on`",
        || {
            rest.core
                .talk()
                .call_assists(&call)
                .iter()
                .any(|a| a.on && a.device == gone_id)
        },
    );
    // Let it notice the host away (it starts the clock), then fast-forward.
    std::thread::sleep(std::time::Duration::from_secs(8));
    assert!(
        !rest.core.huddle_is_open(&call),
        "a host away under a minute keeps the role"
    );
    rest_skew.fetch_add(65, Ordering::SeqCst);
    wait_for(WAIT, "the other computer takes over", || {
        rest.core.huddle_is_open(&call)
    });
    assert_eq!(runner.count("open"), 1);
    wait_for(WAIT, "its `on` row is out", || {
        rest.core
            .talk()
            .call_assists(&call)
            .iter()
            .any(|a| a.on && a.device == rest_id)
    });
    phone.core.close().unwrap();
    for (d, _) in macs {
        d.core.close().unwrap();
    }
}

fn r_host(macs: &[(Dev, Arc<Runner>)], call: &str) -> usize {
    macs.iter()
        .position(|(d, _)| d.core.huddle_is_open(call))
        .expect("a host")
}

#[test]
fn a_signed_out_claude_code_stops_hosting_and_reads_not_connected() {
    let r = rig(1);
    let (mac, runner) = (&r.macs[0].0, r.macs[0].1.clone());
    let call = phone_calls(&r);
    wait_for(WAIT, "the computer holds the session", || {
        mac.core.huddle_is_open(&call)
    });
    runner.signed_out.store(true, Ordering::SeqCst);
    let seed = irori_core::HuddleSeed {
        call_id: call.clone(),
        conv_id: r.ch.clone(),
        ..Default::default()
    };
    assert!(mac.core.huddle_look(&seed, "Addam: hello").is_err());
    wait_for(WAIT, "it stops hosting", || {
        !mac.core.huddle_is_open(&call) && r.on_rows(0, &call).is_empty()
    });
    assert!(!mac.core.huddle_hosting(&call));
    let ours = mac.user();
    let state = mac
        .core
        .call_assistants(&call)
        .into_iter()
        .find(|a| a.person == ours)
        .expect("alice is in the roster");
    assert_eq!(state.state, "not connected");
    assert!(state.why.contains("signed in"), "{state:?}");
    r.close();
}

#[test]
fn a_person_with_no_computer_on_reads_not_connected() {
    let devs = person_with("alice", 2, |_| None);
    let (phone, other) = {
        let mut d = devs;
        let o = d.pop().unwrap();
        (d.pop().unwrap(), o)
    };
    phone.core.set_runs_no_agents();
    other.core.set_runs_no_agents();
    let ch = phone
        .core
        .talk_channel_create(&phone.user(), "us", "")
        .unwrap()
        .id;
    wait_for(WAIT, "the channel reaches the other device", || {
        other
            .core
            .talk_conversations(&other.user())
            .is_ok_and(|cs| cs.iter().any(|c| c.conv_id == ch))
    });
    wait_for(WAIT, "the devices reach each other", || {
        phone.core.call_reachable("").contains(&id(&other))
    });
    let call = phone.core.call_start(&ch).expect("start");
    let ours = phone.user();
    let state = phone
        .core
        .call_assistants(&call)
        .into_iter()
        .find(|a| a.person == ours)
        .expect("alice is in the roster");
    assert_eq!(state.state, "not connected");
    assert_eq!(state.why, "needs a computer that's on");
    phone.core.close().unwrap();
    other.core.close().unwrap();
}
