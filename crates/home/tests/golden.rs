//! testdata/golden/home/home.json, exported from the Go build by
//! tools/goldens/home_golden_test.go (Go at be67a86): node.addr, Settings and
//! Status bytes, Same/Dial/Outside, HostPort and splitPublic, CGNAT, and a
//! Go server's Status and row with a fake router, replayed here against the
//! Rust server with the same fake (the random port masked as N).

mod common;

use std::collections::BTreeMap;

use common::{Node, eventually};
use home::hostport::{host_port, split_public};
use home::nodeaddr::{NodeAddr, REACH_CGNAT, REACH_OK, REACH_UNMAPPED};
use home::{Settings, Status};
use serde::Deserialize;

#[derive(Deserialize)]
struct Golden {
    #[serde(rename = "nodeAddr")]
    node_addr: Vec<GAddr>,
    same: Vec<GSame>,
    settings: Vec<String>,
    status: Vec<String>,
    #[serde(rename = "hostPort")]
    host_port: Vec<GHostPort>,
    cgnat: BTreeMap<String, bool>,
    scenarios: Vec<GScenario>,
}

#[derive(Deserialize)]
struct GAddr {
    json: String,
    dial: Vec<String>,
    outside: String,
}

#[derive(Deserialize)]
struct GSame {
    a: String,
    b: String,
    same: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GHostPort {
    #[serde(rename = "in")]
    input: String,
    port: i64,
    out: String,
    err: String,
    pub_host: String,
    pub_port: i64,
    pub_err: String,
}

#[derive(Deserialize)]
struct GScenario {
    name: String,
    status: String,
    row: String,
}

fn golden() -> Golden {
    let p = manifest_dir().join("../../testdata/golden/home/home.json");
    serde_json::from_slice(&std::fs::read(p).expect("home golden")).expect("parse home golden")
}

fn addr(s: &str) -> NodeAddr {
    NodeAddr::parse(s.as_bytes()).unwrap()
}

#[test]
fn node_addr_bytes_dial_outside() {
    let g = golden();
    assert!(g.node_addr.len() >= 8);
    for c in &g.node_addr {
        let a = addr(&c.json);
        assert_eq!(String::from_utf8(a.marshal()).unwrap(), c.json);
        assert_eq!(a.dial(), c.dial, "dial of {}", c.json);
        assert_eq!(a.outside(), c.outside, "outside of {}", c.json);
    }
    for c in &g.same {
        assert_eq!(addr(&c.a).same(&addr(&c.b)), c.same, "{} vs {}", c.a, c.b);
    }
}

#[test]
fn settings_and_status_bytes() {
    let g = golden();
    for raw in &g.settings {
        let s: Settings = serde_json::from_str(raw).unwrap();
        assert_eq!(String::from_utf8(s.to_file_bytes()).unwrap(), *raw);
    }
    for raw in &g.status {
        let s: Status = serde_json::from_str(raw).unwrap();
        assert_eq!(
            String::from_utf8(gocompat::json::marshal(&s).unwrap()).unwrap(),
            *raw
        );
    }
}

#[test]
fn host_port_and_split_public() {
    let g = golden();
    for c in &g.host_port {
        match host_port(&c.input, c.port) {
            Ok(out) => assert_eq!(
                (out.as_str(), ""),
                (c.out.as_str(), c.err.as_str()),
                "{:?}",
                c.input
            ),
            Err(e) => assert_eq!(
                ("", e.as_str()),
                (c.out.as_str(), c.err.as_str()),
                "{:?}",
                c.input
            ),
        }
        match split_public(&c.input, c.port) {
            Ok((h, p)) => assert_eq!(
                (h.as_str(), p, ""),
                (c.pub_host.as_str(), c.pub_port, c.pub_err.as_str()),
                "{:?}",
                c.input
            ),
            Err(e) => assert_eq!(e, c.pub_err, "{:?}", c.input),
        }
    }
}

#[test]
fn cgnat_table() {
    for (a, want) in golden().cgnat {
        assert_eq!(home::portmap::cgnat(a.parse().unwrap()), want, "{a}");
    }
}

/// The Go server's Status and row for each router state, against the Rust
/// server with the same fake router.
#[tokio::test(flavor = "multi_thread")]
async fn server_scenarios_match_go() {
    let g = golden();
    assert_eq!(g.scenarios.len(), 5);
    for sc in &g.scenarios {
        let mut n = Node::new(|_| {});
        let (outside, public, reach) = match sc.name.as_str() {
            "mapped" => (true, "", REACH_OK),
            "cgnat" => {
                n.gw.st.lock().unwrap().ext = Some("100.64.3.4".parse().unwrap());
                (true, "", REACH_CGNAT)
            }
            "failed" => {
                n.gw.set_fail("no IGD");
                (true, "", REACH_UNMAPPED)
            }
            "home" => (false, "", REACH_UNMAPPED),
            "manual" => (true, "pc.example:443", REACH_OK),
            other => panic!("unknown scenario {other}"),
        };
        n.run();
        n.srv.set(true, outside, public).unwrap();
        eventually(&sc.name, || {
            let r = n.rows();
            r.len() == 1 && r[0].reach == reach
        })
        .await;
        let port = n.srv.settings().port.to_string();
        let mask = |s: String| s.replace(&port, "N");
        let st =
            mask(String::from_utf8(gocompat::json::marshal(&n.srv.status()).unwrap()).unwrap());
        let mut row = n.rows().pop().unwrap();
        row.at = 0;
        let row = mask(String::from_utf8(row.marshal()).unwrap());
        assert_eq!(st, sc.status, "status, {}", sc.name);
        assert_eq!(row, sc.row, "row, {}", sc.name);
        n.shutdown().await;
    }
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
