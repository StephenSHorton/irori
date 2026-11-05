//! A catch-up folds in pages and lets go of the lock between them (card
//! #567). It must say and do what the same rows say one at a time through
//! [`Coord::handle`]: the fold reads parents and subtasks on the rows after
//! the one that moved them, so each case below has a row that does, then a
//! row that reads it, at page sizes that split them and keep them together.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::*;
use coord::*;
use gocompat::sub::TryRecv;
use serde_json::{Value, json};
use store::{Event, Filter, kinds};

const DEV: &str = "dev-b";

#[derive(Clone)]
struct Row {
    actor: &'static str,
    kind: &'static str,
    body: Value,
}

fn at() -> String {
    fixed_now().format_rfc3339_nano()
}

fn created(id: &str, parent: &str) -> Row {
    Row {
        actor: "agent:dev-b/writer",
        kind: kinds::TASK_CREATED,
        body: json!({"taskId": id, "title": id, "objective": id, "parent": parent,
            "by": "dev-b/writer", "at": at()}),
    }
}

fn claimed(id: &str, by: &str) -> Row {
    Row {
        actor: "agent:dev-b/writer",
        kind: kinds::TASK_CLAIMED,
        body: json!({"taskId": id, "by": by, "at": at()}),
    }
}

fn status(id: &str, state: &str) -> Row {
    Row {
        actor: "agent:dev-b/writer",
        kind: kinds::TASK_STATUS,
        body: json!({"taskId": id, "state": state, "by": "dev-b/writer", "at": at()}),
    }
}

fn parent(id: &str, to: &str) -> Row {
    Row {
        actor: "agent:dev-b/writer",
        kind: kinds::TASK_PARENT,
        body: json!({"taskId": id, "parent": to, "by": "dev-b/writer", "at": at()}),
    }
}

fn offered(id: &str, to: &str) -> Row {
    Row {
        actor: "agent:dev-b/writer",
        kind: kinds::TASK_OFFERED,
        body: json!({"taskId": id, "to": to, "reason": "handed", "since": at(),
            "by": "dev-b/writer", "at": at()}),
    }
}

fn deleted(id: &str) -> Row {
    Row {
        actor: "user",
        kind: kinds::TASK_DELETED,
        body: json!({"taskId": id, "by": USER, "at": at()}),
    }
}

/// A row on card `id` that moves nothing but syncs its hand-out.
fn touched(id: &str) -> Row {
    Row {
        actor: "agent:dev-b/writer",
        kind: kinds::TASK_WHERE,
        body: json!({"taskId": id, "who": "dev-b/writer", "branch": "b", "at": at()}),
    }
}

/// A row no fold follows.
fn unfollowed() -> Row {
    Row {
        actor: "agent:dev-b/writer",
        kind: kinds::AGENT_SEEN,
        body: json!({"key": "dev-b/writer"}),
    }
}

struct Log {
    e: Env,
    seq: i64,
    q: Arc<gocompat::sub::Queue<Change>>,
}

impl Log {
    fn new(setup: &[Row]) -> Log {
        let e = Env::new();
        let q = e.c.subscribe();
        let mut l = Log { e, seq: 0, q };
        for r in setup {
            l.live(r);
        }
        l.e.c.sweep_notices().unwrap();
        while matches!(l.q.try_recv(), TryRecv::Item(_)) {}
        l
    }

    fn insert(&mut self, r: &Row) -> Event {
        self.seq += 1;
        let hlc = hlc_at(fixed_now()) - 100_000 + self.seq as u64;
        self.e.insert(DEV, self.seq, hlc, r.actor, r.kind, &r.body)
    }

    fn live(&mut self, r: &Row) {
        let ev = self.insert(r);
        self.e.c.handle(&ev).unwrap();
    }

    fn outcome(&self) -> Outcome {
        let c = &self.e.c;
        let tasks = c.list_tasks(&TaskFilter::default());
        let mut offers = Vec::new();
        for t in &tasks {
            for to in [ALICE, BOB] {
                offers.push(c.stale_offer(&format!("task:{}", t.id), to));
            }
        }
        let mut notices: Vec<String> = c
            .sweep_notices()
            .unwrap()
            .iter()
            .map(|n| format!("{:?} {} {} {}", n.to, n.task_id, n.wake, n.text))
            .collect();
        notices.sort();
        let mut changes = BTreeSet::new();
        while let TryRecv::Item(x) = self.q.try_recv() {
            changes.insert((x.kind, x.task_id));
        }
        let written: Vec<(String, Vec<u8>)> = self
            .e
            .st
            .list(&Filter {
                device_id: "dev-a".into(),
                limit: store::MAX_LIST_LIMIT,
                ..Filter::default()
            })
            .unwrap()
            .into_iter()
            .map(|ev| (ev.kind, ev.payload))
            .collect();
        Outcome {
            tasks: gocompat::json::marshal(&tasks).unwrap(),
            offers,
            notices,
            changes,
            written,
        }
    }
}

#[derive(Debug, PartialEq)]
struct Outcome {
    tasks: Vec<u8>,
    offers: Vec<String>,
    notices: Vec<String>,
    changes: BTreeSet<(String, String)>,
    written: Vec<(String, Vec<u8>)>,
}

/// Every page size folds `rows` as `handle` does; what `handle` did.
fn same_as_live(setup: &[Row], rows: &[Row]) -> Outcome {
    let mut live = Log::new(setup);
    for r in rows {
        live.live(r);
    }
    let want = live.outcome();
    for page in [1, 2, 3, 7, store::MAX_LIST_LIMIT] {
        let mut l = Log::new(setup);
        for r in rows {
            l.insert(r);
        }
        l.e.c.catch_up_by(page).unwrap();
        assert_eq!(l.outcome(), want, "page of {page}");
    }
    want
}

fn woke(o: &Outcome, who: &str) -> bool {
    o.notices
        .iter()
        .any(|n| n.contains(who) && n.contains(" true "))
}

fn lead() -> Vec<Row> {
    vec![created("P", ""), claimed("P", ALICE)]
}

#[test]
fn a_subtask_made_and_failed_in_one_page_wakes_its_parents_holder() {
    let o = same_as_live(&lead(), &[created("K", "P"), status("K", FAILED)]);
    assert!(woke(&o, ALICE), "{:?}", o.notices);
}

#[test]
fn a_card_moved_under_a_parent_then_failed_wakes_its_holder() {
    let mut setup = lead();
    setup.push(created("K", ""));
    let o = same_as_live(&setup, &[parent("K", "P"), status("K", FAILED)]);
    assert!(woke(&o, ALICE), "{:?}", o.notices);
}

#[test]
fn a_card_moved_out_tells_its_old_parent_nothing_more() {
    let mut setup = lead();
    setup.push(created("K", "P"));
    let o = same_as_live(&setup, &[parent("K", ""), status("K", FAILED)]);
    assert!(!woke(&o, ALICE), "{:?}", o.notices);
}

#[test]
fn the_only_subtask_finishing_in_its_own_page_wakes_the_parent() {
    let o = same_as_live(&lead(), &[created("K", "P"), status("K", COMPLETED)]);
    assert!(woke(&o, ALICE), "{:?}", o.notices);
}

#[test]
fn a_new_sibling_keeps_the_old_subtask_from_being_the_last() {
    let mut setup = lead();
    setup.push(created("K1", "P"));
    let o = same_as_live(&setup, &[created("K2", "P"), status("K1", COMPLETED)]);
    assert!(!woke(&o, ALICE), "{:?}", o.notices);
}

#[test]
fn a_card_that_becomes_a_parent_stops_being_handed_out() {
    let setup = vec![created("P", ""), offered("P", ALICE)];
    let o = same_as_live(&setup, &[created("K", "P"), touched("P")]);
    assert!(o.offers.iter().any(|s| !s.is_empty()), "{:?}", o.offers);
}

#[test]
fn a_card_whose_last_subtask_went_is_handed_out_again() {
    let setup = vec![created("P", ""), created("K", "P"), offered("P", ALICE)];
    let o = same_as_live(&setup, &[deleted("K"), touched("P")]);
    assert!(o.offers.iter().any(|s| s.is_empty()), "{:?}", o.offers);
}

/// A rollup moves on a plain state row, and its relink is the page's.
#[test]
fn a_page_of_plain_rows_still_moves_the_rollup_and_says_so() {
    let setup = vec![
        created("P", ""),
        claimed("P", ALICE),
        created("K1", "P"),
        created("K2", "P"),
    ];
    let o = same_as_live(
        &setup,
        &[
            status("K1", WORKING),
            status("K2", WORKING),
            status("K1", COMPLETED),
        ],
    );
    assert!(
        o.changes
            .contains(&(TASK_CHANGED.to_string(), "P".to_string()))
    );
}

#[test]
fn a_rollup_is_current_at_every_page_end() {
    let setup = vec![created("P", ""), created("K1", "P"), created("K2", "P")];
    let mut l = Log::new(&setup);
    l.insert(&status("K1", COMPLETED));
    l.e.c.catch_up_by(1).unwrap();
    let p = l.e.c.get_task("P").unwrap();
    assert_eq!(p.rollup.unwrap().done, 1);
}

#[test]
fn a_catch_up_moves_past_the_unfollowed_rows_at_a_devices_tail() {
    let mut l = Log::new(&[created("P", "")]);
    l.insert(&status("P", WORKING));
    l.insert(&unfollowed());
    l.insert(&unfollowed());
    assert_eq!(l.e.c.folded_to(DEV), 1);
    l.e.c.catch_up().unwrap();
    assert_eq!(l.e.c.folded_to(DEV), l.seq, "the tail is not read again");
    assert_eq!(l.e.c.get_task("P").unwrap().state, WORKING);
}

/// Unfollowed rows cut from the tail read as a cut: the fold rebuilds, and
/// the next pass ends where it should, without doing it again.
#[test]
fn a_cut_of_unfollowed_rows_refolds_once() {
    let mut l = Log::new(&[created("P", "")]);
    l.insert(&unfollowed());
    l.e.c.catch_up().unwrap();
    assert_eq!(l.e.c.folded_to(DEV), 2);
    let mut vec = l.e.st.vector().unwrap();
    vec.insert(DEV.into(), 1);
    l.e.c.refold_if_cut(&vec).unwrap();
    assert_eq!(l.e.c.folded_to(DEV), 1);
    l.e.c.catch_up().unwrap();
    assert_eq!(l.e.c.folded_to(DEV), 2);
    assert!(l.e.c.get_task("P").is_some());
}

/// A resync row takes the catch-up lock before coord's, like every other
/// way in: two passes, a reader and live rows run through each other and
/// end with everything folded once.
#[test]
fn passes_readers_and_resyncs_run_through_each_other() {
    let mut l = Log::new(&lead());
    let c = Arc::new(l.e.reopen());
    let n = 300;
    for i in 0..n {
        l.insert(&created(&format!("K{i}"), "P"));
    }
    assert!(c.get_task("P").unwrap().subtasks.is_empty(), "it is behind");
    let (a, b, r) = (c.clone(), c.clone(), c.clone());
    let done = Arc::new(AtomicBool::new(false));
    let stop = done.clone();
    let reader = std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            std::hint::black_box(r.unresolved());
        }
    });
    let pass = std::thread::spawn(move || a.catch_up_by(2).unwrap());
    let resync = std::thread::spawn(move || {
        b.handle(&Event::default()).unwrap();
    });
    pass.join().unwrap();
    resync.join().unwrap();
    done.store(true, Ordering::Relaxed);
    reader.join().unwrap();
    assert_eq!(c.get_task("P").unwrap().subtasks.len(), n);
}
