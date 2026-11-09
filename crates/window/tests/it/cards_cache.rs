//! The grid's summary cache (card #1147, list-signals.md §6.1) on a scratch
//! Core with the real pumps keeping it: after every batch of random writes
//! the cached grid equals one built cold, and an inbox change alone re-reads
//! the card it names.

use crate::common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use irori_window::cards::{CardSummary, Holding, SCOPE_ALL};
use irori_window::cards_cache::rows_differ;
use irori_window::events;
use irori_window::new_card::NewCardInput;
use sys::cancel::Cancel;

/// What the emitter saw, in order.
#[derive(Default, Clone)]
struct Seen(Arc<Mutex<Vec<(String, serde_json::Value)>>>);

impl Seen {
    fn of(&self, name: &str) -> Vec<serde_json::Value> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
            .collect()
    }
}

fn pumps(r: &Rig) -> (Seen, Cancel) {
    let seen = Seen::default();
    let s = seen.clone();
    r.w.set_emitter(move |name, v| s.0.lock().unwrap().push((name.to_string(), v)));
    let stop = Cancel::new();
    events::start(r.w.clone(), stop.clone());
    (seen, stop)
}

/// A seeded xorshift64: the same seed makes the same writes.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn pick<'a, T>(&mut self, v: &'a [T]) -> Option<&'a T> {
        if v.is_empty() {
            None
        } else {
            Some(&v[self.below(v.len())])
        }
    }
}

fn env_num(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// One log's cards and who writes on it.
struct LogPool {
    co: Arc<coord::Coord>,
    me: String,
    agents: Vec<String>,
    cards: Vec<String>,
    /// Its threads: a card made from one shows it.
    threads: Vec<String>,
    /// A shared board's id; "" for the private log.
    board: String,
}

/// Another device of the person's, writing signed rows this device takes
/// as they come: a wait from there may close a loop this device would
/// refuse to write (card #570).
struct Far {
    key: ed25519_dalek::SigningKey,
    dev: String,
    seq: i64,
    /// Its last row's clock (ms): each row's must follow it.
    last_ms: i64,
}

impl Far {
    fn new(r: &Rig) -> Far {
        let core = &r.w.core;
        let dev = "dev-far".to_string();
        let key = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let add = core
            .identity()
            .cert_for(
                &dev,
                &hex::encode(key.verifying_key().to_bytes()),
                "laptop",
                irori_identity::ROLE_DESKTOP,
                now(),
            )
            .unwrap();
        core.store()
            .append("irori", store::kinds::PERSON_DEVICE_ADDED, &add)
            .unwrap();
        Far {
            key,
            dev,
            seq: 0,
            last_ms: 0,
        }
    }

    /// Card `id` waits on card `on`, from the other device.
    fn wait(&mut self, r: &Rig, id: &str, on: &str) -> Result<(), String> {
        let core = &r.w.core;
        let (seq, ms) = (self.seq + 1, (unix_ms() + 1).max(self.last_ms + 1));
        let bb = coord::BlockedBy {
            kind: coord::BLOCKED_ON_TASK.into(),
            reference: on.into(),
        };
        let by = format!("user:{}", self.dev);
        let row = coord::rows::TaskStateRow {
            task_id: id.into(),
            state: coord::BLOCKED.into(),
            reason: "after it".into(),
            blocked_by: Some(bb.clone()),
            blocker: Some(coord::rows::BlockerRow {
                id: format!("far{seq}"),
                question: "after it".into(),
                on: Some(bb),
                ..Default::default()
            }),
            by: by.clone(),
            at: now(),
            ..Default::default()
        };
        let mut ev = store::Event {
            device_id: self.dev.clone(),
            seq,
            hlc: store::Timestamp::from_unix_ms(ms),
            actor: by,
            kind: store::kinds::TASK_STATUS.into(),
            payload: gocompat::json::marshal(&row).unwrap(),
            sig: Vec::new(),
        };
        use ed25519_dalek::Signer as _;
        ev.sig = self
            .key
            .sign(&store::signing_bytes(&ev))
            .to_bytes()
            .to_vec();
        core.store().insert(&ev).map_err(|e| e.to_string())?;
        (self.seq, self.last_ms) = (seq, ms);
        core.coord()
            .handle(&ev)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

fn now() -> gocompat::time::GoTime {
    gocompat::time::GoTime::from_system(std::time::SystemTime::now())
}

fn ids(v: &[CardSummary]) -> Vec<&str> {
    v.iter().map(|s| s.id.as_str()).collect()
}

/// Waits until the cache, a fresh cache and the cold grid read agree, up
/// to `secs`. On a timeout it says which rows differ and how.
fn converge(r: &Rig, what: &str, secs: u64) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let (kept, fresh) = r.w.cards_cache_check().expect("the pump keeps the cache");
        let cold = r.w.cards_in(SCOPE_ALL);
        let (n_fresh, d_fresh) = rows_differ(&kept, &fresh);
        let (n_cold, d_cold) = rows_differ(&kept, &cold);
        if n_fresh == 0 && n_cold == 0 {
            return;
        }
        if Instant::now() >= deadline {
            let row = |v: &[CardSummary], id: &str| {
                v.iter()
                    .find(|s| s.id == id)
                    .map(|s| serde_json::to_string_pretty(s).unwrap())
                    .unwrap_or_else(|| "(none)".into())
            };
            let mut msg = format!(
                "{what}: the cache never matched\n vs a fresh cache: {n_fresh} {d_fresh:?}\n vs cold: {n_cold} {d_cold:?}\n"
            );
            msg.push_str(&format!(
                " order kept {:?}\n order cold {:?}\n",
                ids(&kept),
                ids(&cold)
            ));
            for id in d_fresh.iter().chain(&d_cold).take(3) {
                msg.push_str(&format!(
                    "--- {id} kept:\n{}\n--- fresh:\n{}\n--- cold:\n{}\n",
                    row(&kept, id),
                    row(&fresh, id),
                    row(&cold, id)
                ));
            }
            panic!("{msg}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// One random write on a log; returns what it did, for the failure note.
fn write(r: &Rig, rng: &mut Rng, l: &mut LogPool, far: &mut Far) -> String {
    let w = &r.w;
    let pick = |rng: &mut Rng, l: &LogPool| rng.pick(&l.cards).cloned();
    let who = |rng: &mut Rng, l: &LogPool| {
        if rng.below(3) == 0 {
            l.me.clone()
        } else {
            rng.pick(&l.agents).cloned().unwrap_or_else(|| l.me.clone())
        }
    };
    let op = rng.below(18);
    let log = if l.board.is_empty() {
        "private"
    } else {
        "board"
    };
    let res: Result<(), String> = match op {
        // A card, sometimes a subtask of another.
        0 | 1 => {
            let parent = if rng.below(3) == 0 {
                pick(rng, l).unwrap_or_default()
            } else {
                String::new()
            };
            let n = l.cards.len();
            l.co.create_task(
                &l.me,
                coord::NewTask {
                    title: format!("{log} card {n}"),
                    parent: parent.clone(),
                    ..Default::default()
                },
            )
            .map(|t| l.cards.push(t.id))
            .map_err(|e| format!("{e} (parent {parent})"))
        }
        // A question for the person.
        2 => match pick(rng, l) {
            Some(id) => {
                let by = who(rng, l);
                l.co.raise(
                    &by,
                    &id,
                    coord::NewBlocker {
                        question: format!("q {}?", rng.below(100)),
                        ..Default::default()
                    },
                )
                .map(|_| ())
                .map_err(|e| e.to_string())
            }
            None => Ok(()),
        },
        // A wait on another card of the log.
        3 => match (pick(rng, l), pick(rng, l)) {
            (Some(a), Some(b)) if a != b => {
                let by = who(rng, l);
                l.co.raise(
                    &by,
                    &a,
                    coord::NewBlocker {
                        question: "after it".into(),
                        blocked_by: Some(coord::BlockedBy {
                            kind: coord::BLOCKED_ON_TASK.into(),
                            reference: b,
                        }),
                        ..Default::default()
                    },
                )
                .map(|_| ())
                .map_err(|e| e.to_string())
            }
            _ => Ok(()),
        },
        // An order: a card after another, or taken off it.
        4 => match (pick(rng, l), pick(rng, l)) {
            (Some(a), Some(b)) if a != b => if rng.below(3) == 0 {
                l.co.remove_after(&l.me, &a, &[b])
            } else {
                l.co.add_after(&l.me, &a, &[b])
            }
            .map(|_| ())
            .map_err(|e| e.to_string()),
            _ => Ok(()),
        },
        // Talk on a card.
        5 => match pick(rng, l) {
            Some(id) => {
                let by = who(rng, l);
                l.co.add_comment(&by, &id, "a word", false)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            None => Ok(()),
        },
        // The person pins one, or moves a pin.
        6 => match pick(rng, l) {
            Some(id) => match rng.below(3) {
                0 => w.pin(&id, false),
                1 => w.pin(&id, true),
                _ => match pick(rng, l) {
                    Some(before) => w.move_pin(&id, &before),
                    None => Ok(()),
                },
            },
            None => Ok(()),
        },
        // Dropped on the canvas.
        7 => match pick(rng, l) {
            Some(id) => w.place(&id, rng.below(800) as i64, rng.below(600) as i64),
            None => Ok(()),
        },
        // An agent takes one up, or leaves it.
        8 => match pick(rng, l) {
            Some(id) => {
                let a = rng.pick(&l.agents).cloned().unwrap_or_default();
                if rng.below(2) == 0 {
                    l.co.claim_task(&a, &id)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                } else {
                    l.co.leave(&a, &id, &a)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                }
            }
            None => Ok(()),
        },
        // Done or canceled: its edges end and loops open.
        9 => match pick(rng, l) {
            Some(id) => {
                let st = if rng.below(2) == 0 {
                    coord::COMPLETED
                } else {
                    coord::CANCELED
                };
                l.co.set_task_state(&l.me, &id, st, "done here")
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            None => Ok(()),
        },
        // Not yet, on the person's word or until another card is done; or
        // brought back.
        10 => match pick(rng, l) {
            Some(id) => match rng.below(3) {
                0 => w.start_now(&id),
                1 => match pick(rng, l) {
                    Some(on) if on != id => {
                        let short =
                            l.co.get_task(&on)
                                .map(|t| t.short_ref())
                                .unwrap_or_default();
                        w.not_yet(&id, &short, "after that one")
                    }
                    _ => Ok(()),
                },
                _ => w.not_yet(&id, "", "later"),
            },
            None => Ok(()),
        },
        // A question answered and closed.
        11 => match pick(rng, l) {
            Some(id) => {
                l.co.unblock(&l.me, &id, "go on")
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            None => Ok(()),
        },
        // Split into two.
        12 => match pick(rng, l) {
            Some(id) => {
                l.co.split(
                    &l.me,
                    &id,
                    vec![
                        coord::NewSubtask {
                            title: "part one".into(),
                            ..Default::default()
                        },
                        coord::NewSubtask {
                            title: "part two".into(),
                            ..Default::default()
                        },
                    ],
                    Vec::new(),
                )
                .map(|(_, kids)| l.cards.extend(kids.into_iter().map(|k| k.id)))
                .map_err(|e| e.to_string())
            }
            None => Ok(()),
        },
        // Weighed.
        13 => match pick(rng, l) {
            Some(id) => {
                let level = ["light", "normal", "heavy"][rng.below(3)];
                w.set_weight(&id, level, "as it looks").map(|_| ())
            }
            None => Ok(()),
        },
        // A thread, and a card made from it.
        15 => {
            let th = w
                .start_thread("shall we?", &[], &[], &l.board)
                .map(|t| t.task.id);
            match th {
                Ok(th) => {
                    l.threads.push(th.clone());
                    let n = l.cards.len();
                    l.co.create_task(
                        &l.me,
                        coord::NewTask {
                            title: format!("{log} card {n} from a thread"),
                            from: th,
                            ..Default::default()
                        },
                    )
                    .map(|t| l.cards.push(t.id))
                    .map_err(|e| e.to_string())
                }
                Err(e) => Err(e),
            }
        }
        // A thread talked on, or closed: the cards made from it show it.
        16 => match rng.pick(&l.threads).cloned() {
            Some(th) => {
                if rng.below(2) == 0 {
                    l.co.add_comment(&l.me, &th, "and another thing", false)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                } else {
                    l.co.set_task_state(&l.me, &th, coord::CANCELED, "settled")
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                }
            }
            None => Ok(()),
        },
        // Two cards waiting on each other, from the person's other device.
        17 if l.board.is_empty() => {
            let open: Vec<String> = l
                .cards
                .iter()
                .filter(|id| {
                    l.co.get_task(id)
                        .is_some_and(|t| !coord::is_terminal(&t.state))
                })
                .cloned()
                .collect();
            match (rng.pick(&open).cloned(), rng.pick(&open).cloned()) {
                (Some(a), Some(b)) if a != b => {
                    far.wait(r, &a, &b).and_then(|_| far.wait(r, &b, &a))
                }
                _ => Ok(()),
            }
        }
        // Edited.
        _ => match pick(rng, l) {
            Some(id) => {
                l.co.edit_task(
                    &l.me,
                    &id,
                    coord::TaskEdit {
                        title: Some(format!("renamed {}", rng.below(1000))),
                        ..Default::default()
                    },
                )
                .map(|_| ())
                .map_err(|e| e.to_string())
            }
            None => Ok(()),
        },
    };
    match res {
        Ok(()) => format!("{log} op {op}"),
        Err(e) => format!("{log} op {op} (refused: {e})"),
    }
}

/// Card #1147's done check: after every batch of random writes on the
/// private log and a shared board (cards, subtasks, questions, waits that
/// close loops, orders, talk, pins on both logs, the canvas, claims,
/// completions, holds, splits, weights, edits), the cached grid equals a
/// fresh cache's and the cold `cards_in("")`, row for row and in order,
/// and the tray's count from the cache equals the cold grid's.
/// `IRORI_TEST_SEED` and `IRORI_TEST_STEPS` replay or lengthen a run.
#[test]
fn the_cache_equals_a_cold_read_after_every_batch() {
    let r = Rig::new();
    let w = &r.w;
    let (_seen, stop) = pumps(&r);
    let seed = env_num("IRORI_TEST_SEED", 0x1147_5eed);
    let steps = env_num("IRORI_TEST_STEPS", 40);
    let mut rng = Rng(seed.max(1));
    let b = w.boards_create("Cache", "cache").unwrap();
    let bco = w.core.board_coord(&b.board_id).unwrap();
    let bkey = |k: &str| w.core.board_key(&b.board_id, k).unwrap();
    let me = w.core.user_sender().key;
    let mut logs = [
        LogPool {
            co: w.core.coord().clone(),
            me: me.clone(),
            agents: vec![r.key_a.clone(), r.key_b.clone()],
            cards: Vec::new(),
            threads: Vec::new(),
            board: String::new(),
        },
        LogPool {
            co: bco,
            me: bkey(&me),
            agents: vec![bkey(&r.key_a), bkey(&r.key_b)],
            cards: Vec::new(),
            threads: Vec::new(),
            board: b.board_id.clone(),
        },
    ];
    // A few cards on each before the random writes, and a first read so
    // the cache is built and kept from here.
    for l in logs.iter_mut() {
        for i in 0..4 {
            let t =
                l.co.create_task(
                    &l.me,
                    coord::NewTask {
                        title: format!("seed {i}"),
                        ..Default::default()
                    },
                )
                .unwrap();
            l.cards.push(t.id);
        }
    }
    let mut far = Far::new(&r);
    converge(&r, &format!("seed {seed:#x}: start"), 20);
    let mut loops = 0;
    for step in 0..steps {
        let n = 1 + rng.below(4);
        let mut done: Vec<String> = Vec::new();
        for _ in 0..n {
            let i = rng.below(2);
            done.push(write(&r, &mut rng, &mut logs[i], &mut far));
        }
        converge(
            &r,
            &format!("seed {seed:#x} step {step}: {}", done.join("; ")),
            20,
        );
        let cold = w.cards_in(SCOPE_ALL);
        let want = cold
            .iter()
            .filter(|s| s.open_blockers > 0 || s.gone_wait.is_some())
            .count();
        assert_eq!(
            w.decide_count(),
            want,
            "seed {seed:#x} step {step}: tray count"
        );
        loops += cold.iter().filter(|s| !s.wait_loop.is_empty()).count();
    }
    // The run closed loops of waits, so their rows were checked too.
    if steps >= 40 {
        assert!(loops > 0, "seed {seed:#x}: no loop of waits was made");
    }
    stop.cancel();
}

/// Card #1147's done check: an assignment notice going from queued to
/// delivered changes its card's row (the holder no longer waits) with no
/// card change, and the board pump names that card so the grid re-reads
/// it.
#[test]
fn a_delivered_assignment_notice_re_reads_its_card() {
    let r = Rig::new();
    let w = &r.w;
    let (seen, stop) = pumps(&r);
    let res = w
        .create_card(NewCardInput {
            title: "for beta".into(),
            assign_to: r.key_b.clone(),
            ..Default::default()
        })
        .unwrap();
    let id = res.task.id.clone();
    let holder = |id: &str| {
        w.cards()
            .into_iter()
            .find(|s| s.id == id)
            .map(|s| s.holder.state)
    };
    wait("the card waits on its notice", || {
        holder(&id) == Some(Holding::Waiting)
    });
    let d = w
        .core
        .inbox()
        .for_agent(&r.key_b, false, 0)
        .into_iter()
        .find(|d| {
            d.message.ref_ == format!("task:{id}") && d.message.reason == coord::NOTICE_ASSIGNED
        })
        .expect("the assignment notice");
    let at = w.core.coord().get_task(&id).unwrap().updated_at;
    let before = seen.of(events::EVENT_BOARD_CHANGED).len();
    w.core.inbox().mark_read(&r.key_b, &d.id).unwrap();
    wait("board:changed names the card", || {
        seen.of(events::EVENT_BOARD_CHANGED)[before..]
            .iter()
            .any(|v| {
                v["all"] == false
                    && v["cards"]
                        .as_array()
                        .is_some_and(|cs| cs.iter().any(|c| c == id.as_str()))
            })
    });
    let p = w.cards_patch(std::slice::from_ref(&id));
    let row = p.rows.iter().find(|s| s.id == id).expect("its row");
    assert_ne!(row.holder.state, Holding::Waiting, "{:?}", row.holder);
    assert!(p.order.contains(&id));
    assert!(
        w.core.coord().get_task(&id).unwrap().updated_at.equal(&at),
        "no card row was written: the inbox change alone named it"
    );
    stop.cancel();
}

/// The drift read builds every row afresh, keeps that cache, and logs
/// `drift cards-cache 0` when the kept one was right.
#[test]
fn the_drift_read_is_cold_and_replaces_the_cache() {
    let r = Rig::new();
    let w = &r.w;
    let (_seen, stop) = pumps(&r);
    let co = w.core.coord();
    let a = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "first".into(),
                ..Default::default()
            },
        )
        .unwrap();
    wait("the cache has it", || {
        w.cards().iter().any(|s| s.id == a.id)
    });
    let cold = w.cards_cold();
    assert_eq!(cold, w.cards_in(SCOPE_ALL));
    // Patches name the changed card and give the whole order.
    let b = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "second".into(),
                ..Default::default()
            },
        )
        .unwrap();
    wait("the patch has it", || {
        let p = w.cards_patch(std::slice::from_ref(&b.id));
        p.rows.iter().any(|s| s.id == b.id) && p.order == ids(&w.cards_in(SCOPE_ALL))
    });
    // A closed card is gone from the order and its row isn't sent.
    co.set_task_state(&r.key_a, &a.id, coord::CANCELED, "no")
        .unwrap();
    wait("the closed card left the order", || {
        let p = w.cards_patch(std::slice::from_ref(&a.id));
        p.rows.is_empty() && !p.order.contains(&a.id)
    });
    stop.cancel();
    // With no pump keeping it, every read is cold.
    assert!(w.cards_cache_check().is_none());
    assert_eq!(w.cards(), w.cards_in(SCOPE_ALL));
}
