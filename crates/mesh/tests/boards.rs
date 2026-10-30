//! Boards stage 1 (mesh.md §2.3 "Board"; I:internal/mesh/boards.go): a
//! board log moves between the person's devices on board/<id> streams,
//! never on the sync channel, and only boards both hold are served.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use common::{Dev, converged, eventually, never};
use mesh::{BoardLogs, FakeNet};
use serde_json::json;
use store::{Event, Store};

#[derive(Default)]
struct Boards(Mutex<BTreeMap<String, Arc<Store>>>);

impl Boards {
    fn add(&self, id: &str, st: Arc<Store>) {
        self.0.lock().unwrap().insert(id.into(), st);
    }
}

impl BoardLogs for Boards {
    fn board_ids(&self) -> Vec<String> {
        self.0.lock().unwrap().keys().cloned().collect()
    }
    fn board_store(&self, id: &str) -> Option<Arc<Store>> {
        self.0.lock().unwrap().get(id).cloned()
    }
}

/// A board log for `d`: its rows signed by `d`'s device key.
fn board_log(d: &Dev, id: &str) -> Arc<Store> {
    let path = d.dir.join("boards").join(format!("{id}.db"));
    Arc::new(
        Store::open_with(
            path,
            &d.id(),
            Arc::new(store::Clock::new()),
            store::OpenOptions {
                signer: Some(d.ident.clone()),
                ..Default::default()
            },
        )
        .unwrap(),
    )
}

fn has(st: &Store, ev: &Event) -> bool {
    st.list(&store::Filter {
        device_id: ev.device_id.clone(),
        after_seq: ev.seq - 1,
        limit: 1,
        ..Default::default()
    })
    .is_ok_and(|evs| evs.len() == 1 && store::row_hash(&evs[0]) == store::row_hash(ev))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn board_logs_move_on_board_streams_both_ways() {
    let net = FakeNet::new();
    let (mut a, mut b) = (Dev::new(&net, "pc"), Dev::new(&net, "mac"));
    b.join(&a).await;
    let (ba, bb) = (Arc::new(Boards::default()), Arc::new(Boards::default()));
    let (la, lb) = (board_log(&a, "b1"), board_log(&b, "b1"));
    ba.add("b1", la.clone());
    bb.add("b1", lb.clone());
    // A board only the pc holds is never served to the mac.
    let only = board_log(&a, "b2");
    ba.add("b2", only.clone());
    let secret = only
        .append("irori", "card.created", &json!({"t": 1}))
        .unwrap();
    a.boards = Some(ba);
    b.boards = Some(bb.clone());
    // More rows than one pull carries (2000): the rest come on the next.
    let mut rows = Vec::new();
    for i in 0..2300 {
        rows.push(
            la.append("irori", "card.created", &json!({"i": i}))
                .unwrap(),
        );
    }
    a.start();
    b.start();
    eventually("the private logs converge", || converged(&[&a, &b])).await;
    eventually("the mac pulls the pc's board rows", || {
        has(&lb, rows.last().unwrap())
    })
    .await;
    // A row on the mac's board log reaches the pc, on a kick.
    let back = lb
        .append("irori", "card.created", &json!({"from": "mac"}))
        .unwrap();
    b.node().kick_boards();
    eventually("the pc pulls the mac's board row", || has(&la, &back)).await;
    // No board row ever lands in a private log.
    let private =
        a.st.list(&store::Filter {
            kinds: vec!["card.".into()],
            ..Default::default()
        })
        .unwrap();
    assert!(private.is_empty(), "a board row reached the private log");
    // The mac opens b2 later: now it is pulled.
    never("the mac holds b2 rows it never opened", || {
        bb.board_store("b2").is_some_and(|s| has(&s, &secret))
    })
    .await;
    let lb2 = board_log(&b, "b2");
    bb.add("b2", lb2.clone());
    b.node().kick_boards();
    eventually("b2 is pulled once the mac holds it", || has(&lb2, &secret)).await;
}
