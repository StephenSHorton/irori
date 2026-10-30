//! I:internal/mesh/servers_test.go and devices_test.go.

mod common;

use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::{Dev, eventually, trio};
use mesh::{
    BoxFut, FakeNet, NodeAddr, PublicAddr, Room, RoomConfig, STATE_ERROR, Secret, Servers,
    Transport, TransportError,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn row(a: &NodeAddr) -> serde_json::Value {
    serde_json::from_slice(&a.marshal()).unwrap()
}

/// The node.addr fold: each server's newest row by seq, LAN first, "off"
/// drops it, a revoked writer's rows are ignored, the LAN address its
/// datagrams came from is dialed first, and pairing hints last only until
/// a row arrives.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn servers_fold_node_addr() {
    let net = FakeNet::new();
    let (pc, mac, nd) = trio(&net).await;
    let dir = Arc::new(Servers::new(
        &mac.id(),
        Some(mac.chain.clone()),
        Some(&mac.dir),
    ));
    let cancel = CancellationToken::new();
    {
        let (dir, st, c) = (dir.clone(), mac.st.clone(), cancel.clone());
        tokio::spawn(async move { dir.follow(st, c).await });
    }
    tokio::time::sleep(Duration::from_millis(50)).await; // subscribed before the rows land

    // Before the node's row: a hint from pairing.
    dir.hint(&nd.id(), &["192.168.1.20:24817".to_string()]);
    assert_eq!(dir.addrs(&nd.id()), vec!["192.168.1.20:24817"]);
    nd.append(
        "node.addr",
        row(&NodeAddr {
            v: 1,
            port: 24817,
            lan: vec!["192.168.1.20".into()],
            public: Some(PublicAddr {
                ip: "203.0.113.7".into(),
                tcp: 24900,
                udp: 24901,
            }),
            via: "upnp".into(),
            reach: "ok".into(),
            at: 1,
            ..Default::default()
        }),
    );
    eventually("the node's row reaches the mac", || {
        dir.row(&nd.id()).is_some_and(|r| r.port == 24817)
    })
    .await;
    assert_eq!(
        dir.addrs(&nd.id()),
        vec!["192.168.1.20:24817", "203.0.113.7:24900"]
    );
    assert_eq!(dir.row(&nd.id()).unwrap().outside(), "203.0.113.7:24900");
    // The LAN address its datagrams come from wins.
    dir.heard(&nd.id(), "192.168.1.33".parse::<IpAddr>().unwrap());
    assert_eq!(dir.addrs(&nd.id())[0], "192.168.1.33:24817");
    // A device that runs no server is never kept from what it's heard.
    dir.heard(&pc.id(), "192.168.1.40".parse::<IpAddr>().unwrap());
    assert!(
        dir.list().iter().all(|s| s.id != pc.id()),
        "a device with no server is listed"
    );
    // The last address that worked is kept across restarts.
    dir.worked(&nd.id(), "192.168.1.33:24817");
    let again = Servers::new(&mac.id(), Some(mac.chain.clone()), Some(&mac.dir));
    assert!(
        again
            .addrs(&nd.id())
            .contains(&"192.168.1.33:24817".to_string()),
        "the last good address is lost"
    );
    // A newer row replaces it; "off" stops the dialing.
    nd.append(
        "node.addr",
        row(&NodeAddr {
            v: 1,
            reach: "off".into(),
            at: 2,
            ..Default::default()
        }),
    );
    eventually("off reaches the mac", || {
        dir.addrs(&nd.id()).is_empty() && dir.list().is_empty()
    })
    .await;

    // A revoked writer's row is ignored.
    pc.append(
        "node.addr",
        row(&NodeAddr {
            v: 1,
            port: 2000,
            lan: vec!["192.168.1.40".into()],
            reach: "unmapped".into(),
            at: 3,
            ..Default::default()
        }),
    );
    eventually("the pc's row reaches the mac", || {
        dir.row(&pc.id()).is_some()
    })
    .await;
    nd.node().revoke(&pc.id(), "test").await.unwrap();
    eventually("the revoke reaches the mac", || dir.row(&pc.id()).is_none()).await;
    assert!(
        dir.list().is_empty(),
        "a revoked server is listed: {:?}",
        dir.list()
    );
    cancel.cancel();
}

#[test]
fn node_addr_same_ignores_when() {
    let a = NodeAddr {
        v: 1,
        port: 1,
        lan: vec!["10.0.0.2".into()],
        reach: "unmapped".into(),
        at: 5,
        ..Default::default()
    };
    let mut b = a.clone();
    b.at = 9;
    assert!(a.same(&b), "rows that differ only in At aren't the same");
    b.lan = vec!["192.168.1.3".into()];
    assert!(!a.same(&b), "a new LAN address is the same");
}

#[test]
fn node_addr_writes_null_public_and_always_via_reach_at() {
    let off = NodeAddr {
        v: 1,
        reach: "off".into(),
        at: 7,
        ..Default::default()
    };
    assert_eq!(
        String::from_utf8(off.marshal()).unwrap(),
        r#"{"v":1,"public":null,"via":"","reach":"off","at":7}"#
    );
    let back = NodeAddr::parse(&off.marshal()).unwrap();
    assert_eq!(back, off);
}

/// A node.addr row written after Load but before Follow has subscribed is
/// still folded.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn servers_follow_keeps_a_row_written_before_it_subscribed() {
    let nd = Dev::new(&FakeNet::new(), "node");
    let dir = Arc::new(Servers::new(&nd.id(), Some(nd.chain.clone()), None));
    dir.load(&nd.st).unwrap();
    nd.append(
        "node.addr",
        json!({"v": 1, "port": 24817, "lan": ["127.0.0.1"], "public": null, "via": "", "reach": "unmapped", "at": 1}),
    );
    let cancel = CancellationToken::new();
    let task = {
        let (dir, st, c) = (dir.clone(), nd.st.clone(), cancel.clone());
        tokio::spawn(async move { dir.follow(st, c).await })
    };
    eventually("the server's own row is folded", || {
        dir.hints().contains_key(&nd.id())
    })
    .await;
    cancel.cancel();
    task.await.unwrap();
}

/// A transport whose rooms never open: no carrier can start.
#[derive(Default)]
struct DownNet {
    joins: AtomicUsize,
}

impl Transport for DownNet {
    fn join(&self, _: Secret, _: RoomConfig) -> BoxFut<'_, Result<Arc<dyn Room>, TransportError>> {
        self.joins.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(TransportError("no carrier could start".to_string())) })
    }
}

/// With the net layer failing to start the device keeps working on its
/// own, Devices says why, and the join is retried.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unreachable_signaling_leaves_a_working_device() {
    let (mut a, mut b) = (
        Dev::new(&FakeNet::new(), "pc"),
        Dev::new(&FakeNet::new(), "mac"),
    );
    b.join(&a).await; // a has a second device, so its net starts
    let down = Arc::new(DownNet::default());
    let mut o = a.options(down.clone());
    o.data_dir = None;
    a.start_node(mesh::Node::new(o));
    eventually("the join is retried", || {
        down.joins.load(Ordering::SeqCst) >= 3
    })
    .await;
    let st = a.node().status();
    assert!(
        st.state == STATE_ERROR && st.detail.contains("Can't open the network"),
        "status = {st:?}"
    );
    let rows = a.node().devices(false);
    assert!(
        rows[0].this && rows[0].warning.contains("Can't open the network"),
        "this device's row = {:?}",
        rows[0]
    );
    assert!(
        rows.len() == 2
            && rows[1].device_id == b.id()
            && rows[1].state == "offline"
            && rows[1].last_seen.is_none(),
        "the mac's row = {rows:?}"
    );
    // The log, and everything on it, still works here.
    let ev = a.append(
        store::kinds::AGENT_SEEN,
        json!({"key": format!("{}/s1", a.id())}),
    );
    assert!(a.has(&ev), "a row written with the net down is missing");
}
