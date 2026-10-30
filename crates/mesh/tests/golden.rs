//! Replays of testdata/golden/mesh/mesh.json (tools/goldens/mesh_golden_test.go):
//! the frozen formats (D12) byte for byte.

use std::collections::BTreeMap;

use gocompat::time::GoTime;
use mesh::devices::{DeviceRow, PeerState, Status};
use mesh::files::FileOffer;
use mesh::frames::*;
use mesh::peers::PeerEpoch;
use mesh::{NodeAddr, Presence, PublicAddr, Relay, decode_frame, mesh_secret, open_secret};
use serde_json::Value;

fn golden() -> Value {
    let p = manifest_dir().join("../../testdata/golden/mesh/mesh.json");
    serde_json::from_slice(&std::fs::read(p).expect("golden file")).expect("golden JSON")
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap()
}

fn h(v: &Value, k: &str) -> Vec<u8> {
    hex::decode(s(v, k)).unwrap()
}

fn texts(g: &Value, section: &str) -> BTreeMap<String, String> {
    g[section]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (s(c, "name").to_string(), s(c, "text").to_string()))
        .collect()
}

fn text(b: Vec<u8>) -> String {
    String::from_utf8(b).unwrap()
}

#[test]
fn mesh_secret_matches_go() {
    for c in golden()["meshSecret"].as_array().unwrap() {
        let got = mesh_secret(
            &h(c, "seed"),
            s(c, "personId"),
            c["epoch"].as_i64().unwrap(),
        );
        assert_eq!(hex::encode(got.key), s(c, "key"), "{c}");
        assert_eq!(got.topic, s(c, "topic"), "{c}");
    }
}

#[test]
fn person_secret_sealing_matches_go() {
    for c in golden()["sealed"].as_array().unwrap() {
        let (old, nw, pid, rcpt) = (
            h(c, "old"),
            h(c, "new"),
            s(c, "personId"),
            s(c, "recipient"),
        );
        let epoch = c["epoch"].as_i64().unwrap();
        let nonce: [u8; 24] = h(c, "nonce").try_into().unwrap();
        let sealed =
            mesh::secret::seal_secret_with_nonce(&old, &nw, pid, epoch, rcpt, nonce).unwrap();
        assert_eq!(hex::encode(&sealed), s(c, "sealed"), "{c}");
        assert!(c["opens"].as_bool().unwrap());
        assert_eq!(
            open_secret(&old, &h(c, "sealed"), pid, epoch, rcpt).unwrap(),
            nw
        );
    }
}

fn golden_row() -> Row {
    Row {
        d: "dev-a".into(),
        s: 7,
        h: (1 << 62) + 5,
        a: "agent:dev-a/s<1>&".into(),
        k: "agent.seen".into(),
        p: br#"{"b": 2 , "t":"<&>"}"#.to_vec(),
        sig: vec![1, 2, 3, 250],
    }
}

#[test]
fn frames_match_go() {
    let want = texts(&golden(), "frames");
    let row = golden_row();
    let vec = |kv: &[(&str, i64)]| kv.iter().map(|(k, v)| (k.to_string(), *v)).collect();
    let frames = [
        (
            "hello",
            Frame {
                v: PROTOCOL,
                device_id: "dev-a".into(),
                person_id: "p1".into(),
                epoch: 2,
                core_version: "0.5.0".into(),
                schema: 3,
                wall_ms: 1761700000123,
                vector: vec(&[("dev-b", 12), ("dev-a", 40), ("dev-c", 0)]),
                top: "ab12".into(),
                top_row: Some(Box::new(row.clone())),
                person_pub: "ff00".into(),
                ..Frame::of_type(FRAME_HELLO)
            },
        ),
        (
            "helloEmpty",
            Frame {
                v: PROTOCOL,
                device_id: "dev-a".into(),
                person_id: "p1".into(),
                epoch: 1,
                schema: 3,
                ..Frame::of_type(FRAME_HELLO)
            },
        ),
        (
            "rows",
            Frame {
                events: vec![
                    row.clone(),
                    Row {
                        d: "dev-b".into(),
                        s: 1,
                        h: 9,
                        a: "irori".into(),
                        k: "x".into(),
                        ..Row::default()
                    },
                ],
                batch: 3,
                ..Frame::of_type(FRAME_ROWS)
            },
        ),
        (
            "rowsLoose",
            Frame {
                events: vec![row.clone()],
                loose: true,
                ..Frame::of_type(FRAME_ROWS)
            },
        ),
        (
            "vectorAck",
            Frame {
                vector: vec(&[("d", 1 << 50)]),
                ack: true,
                batch: 9,
                ..Frame::of_type(FRAME_VECTOR)
            },
        ),
        (
            "vector",
            Frame {
                vector: vec(&[("d", 3)]),
                ..Frame::of_type(FRAME_VECTOR)
            },
        ),
        (
            "ping",
            Frame {
                t: 1761700000999,
                ..Frame::of_type(FRAME_PING)
            },
        ),
        (
            "pong",
            Frame {
                t: 1761700000999,
                ..Frame::of_type(FRAME_PONG)
            },
        ),
        (
            "personSecret",
            Frame {
                epoch: 3,
                from: 1,
                sealed: b"0123456789abcdef0123456789abcdef0123456789".to_vec(),
                ..Frame::of_type(FRAME_PERSON_SECRET)
            },
        ),
        (
            "rowAt",
            Frame {
                device_id: "dev-a".into(),
                seq: 41,
                ..Frame::of_type(FRAME_ROW_AT)
            },
        ),
        ("bare", Frame::of_type("future.kind")),
    ];
    for (name, f) in frames {
        let go = &want[name];
        assert_eq!(text(f.encode()), *go, "frame {name}");
        // Go's bytes decode to the same frame, and re-encode to themselves.
        let back = decode_frame(go.as_bytes()).unwrap();
        assert_eq!(back, f, "decode {name}");
    }
    // The stream bodies and envelopes.
    assert_eq!(
        text(serde_json::json!({"vector": {"dev-a": 3}}).bytes()),
        want["boardRequest"]
    );
    use erased::Ser as _;
    assert_eq!(
        text(
            gocompat::json::marshal(&FileOffer {
                sha256: "ab".into(),
                size: 12,
                mime: "text/plain".into(),
                ..FileOffer::default()
            })
            .unwrap()
        ),
        want["fileOffer"]
    );
    assert_eq!(
        text(
            gocompat::json::marshal(&Presence {
                device_id: "dev-a".into(),
                epoch: 2,
                core_version: "0.5.0".into()
            })
            .unwrap()
        ),
        want["presence"]
    );
    // A board chunk's rows decode as Go wrote them.
    let f = decode_frame(format!("{{\"type\":\"rows\",{}", &want["boardChunk"][1..]).as_bytes())
        .unwrap();
    assert_eq!(f.events, vec![row]);
}

/// Serializes through Go's marshal (a tiny object-safe shim).
mod erased {
    pub trait Ser {
        fn bytes(&self) -> Vec<u8>;
    }
    impl<T: serde::Serialize> Ser for T {
        fn bytes(&self) -> Vec<u8> {
            gocompat::json::marshal(self).unwrap()
        }
    }
}

#[test]
fn node_addr_matches_go() {
    let rows = [
        (
            "pc",
            NodeAddr {
                v: 1,
                port: 25817,
                lan: vec!["192.168.1.20".into()],
                reach: "unmapped".into(),
                at: 1761700000000,
                ..NodeAddr::default()
            },
        ),
        (
            "mapped",
            NodeAddr {
                v: 1,
                port: 24817,
                lan: vec!["192.168.1.20".into(), "10.0.0.2".into()],
                v6: vec!["2001:db8::1".into()],
                public: Some(PublicAddr {
                    ip: "203.0.113.7".into(),
                    tcp: 24900,
                    udp: 24901,
                }),
                via: "upnp".into(),
                reach: "ok".into(),
                at: 5,
            },
        ),
        (
            "cgnat",
            NodeAddr {
                v: 1,
                port: 24817,
                public: Some(PublicAddr {
                    ip: "100.64.0.1".into(),
                    tcp: 24817,
                    udp: 24817,
                }),
                via: "natpmp".into(),
                reach: "cgnat".into(),
                at: 6,
                ..NodeAddr::default()
            },
        ),
        (
            "off",
            NodeAddr {
                v: 1,
                reach: "off".into(),
                at: 7,
                ..NodeAddr::default()
            },
        ),
    ];
    let g = golden();
    for (c, (name, a)) in g["nodeAddr"].as_array().unwrap().iter().zip(rows) {
        assert_eq!(s(c, "name"), name);
        assert_eq!(text(a.marshal()), s(c, "payload"), "{name}");
        assert_eq!(NodeAddr::parse(s(c, "payload").as_bytes()).unwrap(), a);
        let dial: Vec<String> = c["dial"]
            .as_array()
            .map(|d| d.iter().map(|x| x.as_str().unwrap().to_string()).collect())
            .unwrap_or_default();
        assert_eq!(a.dial(), dial, "{name} dial");
        assert_eq!(a.outside(), s(c, "outside"), "{name} outside");
        let mut moved = a.clone();
        moved.at += 1;
        assert_eq!(a.same(&moved), c["sameAt"].as_bool().unwrap());
    }
}

#[test]
fn data_dir_files_match_go() {
    let want = texts(&golden(), "files");
    let peers: BTreeMap<String, PeerEpoch> = [
        (
            "dev-b".to_string(),
            PeerEpoch {
                epoch: 2,
                person_pub: "ff00".into(),
            },
        ),
        (
            "dev-a".to_string(),
            PeerEpoch {
                epoch: 1,
                person_pub: String::new(),
            },
        ),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        text(mesh::peers::peers_file_bytes(&peers)),
        want["mesh-peers.json"]
    );
    // The file reads back.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(mesh::PEERS_FILE), &want["mesh-peers.json"]).unwrap();
    assert_eq!(mesh::peers::load_peers(Some(dir.path())), peers);

    // hub-addrs.json, written by Worked and Hint, read back by a new
    // directory.
    let d2 = tempfile::tempdir().unwrap();
    let sv = mesh::Servers::new("self", None, Some(d2.path()));
    sv.worked("dev-z", "192.168.1.2:25817");
    sv.hint("dev-y", &["10.0.0.2:1".to_string(), "[::1]:2".to_string()]);
    let got = std::fs::read_to_string(d2.path().join(mesh::servers::SERVERS_FILE)).unwrap();
    assert_eq!(got, want["hub-addrs.json"]);
    std::fs::write(
        d2.path().join(mesh::servers::SERVERS_FILE),
        &want["hub-addrs.json"],
    )
    .unwrap();
    let again = mesh::Servers::new("self", None, Some(d2.path()));
    assert_eq!(again.addrs("dev-y"), vec!["10.0.0.2:1", "[::1]:2"]);

    let d3 = tempfile::tempdir().unwrap();
    let r = Relay {
        credential_url: "https://relay.example/cred?k=1".into(),
        turn_urls: vec!["turns:relay.example:5349".into()],
    };
    mesh::save_relay(d3.path(), &r).unwrap();
    let got = std::fs::read_to_string(d3.path().join(mesh::relay::RELAY_FILE)).unwrap();
    assert_eq!(got, want["relay.json"]);
}

#[test]
fn status_json_matches_go() {
    let want = texts(&golden(), "status");
    let at = GoTime::parse_rfc3339("2026-10-29T12:30:45.1234567Z").unwrap();
    let at1 = GoTime::parse_rfc3339("2026-10-29T12:30:44.1234567Z").unwrap();
    let ps = PeerState {
        device_id: "dev-b".into(),
        label: "Mac".into(),
        role: "desktop".into(),
        state: "connected".into(),
        epoch: 2,
        path: "direct".into(),
        rtt: 1_500_000,
        last_ping: Some(at),
        last_sync: Some(at1),
        skew: -250_000_000,
        refused: 1,
        ignored: 2,
        held: 3,
        stopped: "its clock went backwards".into(),
        ahead: true,
        old: true,
        ..PeerState::default()
    };
    let m = |v: &dyn erased::Ser| text(v.bytes());
    assert_eq!(m(&ps), want["peerFull"]);
    assert_eq!(
        m(&PeerState {
            device_id: "dev-c".into(),
            role: "node".into(),
            state: "offline".into(),
            revoked: true,
            pending: 1,
            remaining: 2,
            ..PeerState::default()
        }),
        want["peerRevoked"]
    );
    assert_eq!(
        m(&Status {
            state: "on".into(),
            epoch: 2,
            topics: 1,
            peers: Some(vec![ps.clone()]),
            ..Status::default()
        }),
        want["status"]
    );
    assert_eq!(
        m(&Status {
            state: "off".into(),
            epoch: 1,
            ..Status::default()
        }),
        want["statusEmpty"]
    );
    assert_eq!(
        m(&DeviceRow {
            device_id: "dev-a".into(),
            name: "PC".into(),
            role: "desktop".into(),
            os: "windows".into(),
            this: true,
            state: "this".into(),
            ..DeviceRow::default()
        }),
        want["rowThis"]
    );
    assert_eq!(
        m(&DeviceRow {
            device_id: "dev-b".into(),
            name: "Mac".into(),
            role: "desktop".into(),
            state: "connected".into(),
            warning: "It its clock went backwards. Remove it.".into(),
            action: "remove".into(),
            last_seen: Some(at),
            detail: Some(ps),
            ..DeviceRow::default()
        }),
        want["rowDetail"]
    );
}

#[test]
fn scrub_urls_matches_go() {
    for c in golden()["scrub"].as_array().unwrap() {
        assert_eq!(mesh::scrub_urls(s(c, "name")), s(c, "text"));
    }
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
