//! Boards stage 2 (card #43, v0.5 §6.2, B1): the `board.server` fold keeps
//! the newest row per server device written by that device while it is a
//! counted, uncut member device; rows are written only on change.

use std::collections::BTreeMap;
use std::sync::Arc;

use ed25519_dalek::SigningKey;
use irori_identity::*;
use mesh::PublicAddr;
use mesh::boardmesh::{BoardServer, KIND_BOARD_SERVER, board_servers, write_board_server};
use serde::Serialize;
use store::{Event, Store};

fn rand_key() -> SigningKey {
    let mut s = [0u8; 32];
    getrandom::fill(&mut s).unwrap();
    SigningKey::from_bytes(&s)
}

fn unix(secs: i64) -> gocompat::time::GoTime {
    gocompat::time::GoTime::unix(secs, 0)
}

struct P {
    pid: String,
    root: String,
    key: SigningKey,
}

impl P {
    fn new(root: &str) -> P {
        let key = rand_key();
        P {
            pid: person_id_of(&key.verifying_key().to_bytes()),
            root: root.into(),
            key,
        }
    }
    fn pin(&self) -> MemberPin {
        MemberPin {
            person_id: self.pid.clone(),
            person_pub: pub_hex(&self.key),
            root_device_id: self.root.clone(),
            person_epoch: 1,
        }
    }
}

/// Signed board identity rows fed straight into a verifier.
#[derive(Default)]
struct Rows {
    devs: BTreeMap<String, SigningKey>,
    seq: BTreeMap<String, i64>,
    hlc: u64,
}

impl Rows {
    fn row<T: Serialize>(&mut self, writer: &str, actor: &str, kind: &str, payload: &T) -> Row {
        *self.seq.entry(writer.into()).or_default() += 1;
        self.hlc += 1;
        let mut r = Row {
            device_id: writer.into(),
            seq: self.seq[writer],
            hlc: self.hlc,
            actor: actor.into(),
            kind: kind.into(),
            payload: gocompat::json::marshal(payload).unwrap(),
            sig: Vec::new(),
        };
        let k = self
            .devs
            .entry(writer.into())
            .or_insert_with(rand_key)
            .clone();
        r.sign(&k);
        r
    }

    fn cert(&mut self, p: &P, writer: &str, dev: &str) -> Row {
        let k = self.devs.entry(dev.into()).or_insert_with(rand_key).clone();
        let c = MemberCert {
            person_id: p.pid.clone(),
            device_id: dev.into(),
            device_pub: pub_hex(&k),
            role: ROLE_DESKTOP.into(),
            person_epoch: 1,
            at: unix(1),
        };
        let a = MemberDeviceAdded {
            person_sig: sign_person(&p.key, &c).unwrap(),
            cert: c,
        };
        self.row(writer, "irori", kinds::MEMBER_DEVICE_ADDED, &a)
    }

    fn revoke(&mut self, p: &P, writer: &str, dev: &str) -> Row {
        let r = MemberRevoke {
            person_id: p.pid.clone(),
            device_id: dev.into(),
            board_cut: 0,
            person_epoch: 1,
            at: unix(2),
            ..Default::default()
        };
        let a = MemberDeviceRevoked {
            person_sig: sign_person(&p.key, &r).unwrap(),
            revoke: r,
        };
        self.row(writer, "irori", kinds::MEMBER_DEVICE_REVOKED, &a)
    }

    fn created(&mut self, p: &P) -> Row {
        let mut r = BoardCreated {
            name: "mlg".into(),
            tag: "mlg".into(),
            creator: p.pin(),
            at: unix(1),
            ..Default::default()
        };
        r.person_sig = sign_person(&p.key, &r).unwrap();
        let root = p.root.clone();
        self.row(
            &root,
            &format!("person:{}", p.pid),
            kinds::BOARD_CREATED,
            &r,
        )
    }

    fn added(&mut self, by: &P, writer: &str, who: &P) -> Row {
        let pin = who.pin();
        let mut r = BoardMemberAdded {
            person_id: pin.person_id,
            person_pub: pin.person_pub,
            root_device_id: pin.root_device_id,
            person_epoch: 1,
            name: "x".into(),
            role: "member".into(),
            by: by.pid.clone(),
            by_epoch: 1,
            at: unix(2),
            person_sig: String::new(),
        };
        r.person_sig = sign_person(&by.key, &r).unwrap();
        self.row(
            writer,
            &format!("person:{}", by.pid),
            kinds::BOARD_MEMBER_ADDED,
            &r,
        )
    }
}

/// Stephen (devices `pc`, `old`) made the board and added Blake (`bpc`);
/// `old` was revoked.
fn verifier() -> BoardVerifier {
    let (s, b) = (P::new("pc"), P::new("bpc"));
    let v = BoardVerifier::new(&[s.pin()]);
    let mut f = Rows::default();
    let rows = [
        f.cert(&s, "pc", "pc"),
        f.created(&s),
        f.cert(&s, "pc", "old"),
        f.added(&s, "pc", &b),
        f.cert(&b, "bpc", "bpc"),
        f.revoke(&s, "pc", "old"),
    ];
    for r in &rows {
        v.observe(r.as_ref());
    }
    assert!(v.cert("pc").is_some() && v.cert("bpc").is_some());
    assert!(v.cut("old").is_some(), "old is revoked");
    v
}

fn log(dir: &std::path::Path, me: &str) -> Store {
    Store::open_with(
        dir.join("board.db"),
        me,
        Arc::new(store::Clock::new()),
        store::OpenOptions::default(),
    )
    .unwrap()
}

fn server_row(writer: &str, seq: i64, hlc: u64, s: &serde_json::Value) -> Event {
    Event {
        device_id: writer.into(),
        seq,
        hlc: store::Timestamp(hlc << 16),
        actor: "irori".into(),
        kind: KIND_BOARD_SERVER.into(),
        payload: serde_json::to_vec(s).unwrap(),
        sig: Vec::new(),
    }
}

#[test]
fn the_fold_keeps_each_counted_servers_newest_own_row() {
    let dir = tempfile::tempdir().unwrap();
    let st = log(dir.path(), "me");
    let v = verifier();
    let public = serde_json::json!({"ip": "203.0.113.7", "tcp": 41641, "udp": 41641});
    for ev in [
        server_row(
            "pc",
            1,
            1,
            &serde_json::json!({"deviceId": "pc", "port": 1, "public": null}),
        ),
        server_row(
            "bpc",
            1,
            2,
            &serde_json::json!({"deviceId": "bpc", "port": 7, "public": public}),
        ),
        server_row(
            "pc",
            2,
            3,
            &serde_json::json!({"deviceId": "pc", "port": 2, "public": public}),
        ),
        // Blake's device announcing Stephen's: not its own, ignored.
        server_row(
            "bpc",
            2,
            4,
            &serde_json::json!({"deviceId": "pc", "port": 99, "public": null}),
        ),
        // A revoked device and one with no cert on this board.
        server_row(
            "old",
            1,
            5,
            &serde_json::json!({"deviceId": "old", "port": 3, "public": public}),
        ),
        server_row(
            "eve",
            1,
            6,
            &serde_json::json!({"deviceId": "eve", "port": 4, "public": public}),
        ),
    ] {
        st.insert(&ev).unwrap();
    }
    let got = board_servers(&st, &v).unwrap();
    let pa = PublicAddr {
        ip: "203.0.113.7".into(),
        tcp: 41641,
        udp: 41641,
    };
    assert_eq!(
        got,
        vec![
            BoardServer {
                device_id: "bpc".into(),
                port: 7,
                public: Some(pa.clone()),
            },
            BoardServer {
                device_id: "pc".into(),
                port: 2,
                public: Some(pa),
            },
        ]
    );
}

#[test]
fn a_server_row_is_written_only_on_change() {
    let dir = tempfile::tempdir().unwrap();
    let st = log(dir.path(), "pc");
    let mut row = BoardServer {
        device_id: "pc".into(),
        port: 41641,
        public: None,
    };
    assert!(write_board_server(&st, &row).unwrap());
    assert!(!write_board_server(&st, &row).unwrap(), "same row twice");
    row.public = Some(PublicAddr {
        ip: "203.0.113.7".into(),
        tcp: 41641,
        udp: 41641,
    });
    assert!(write_board_server(&st, &row).unwrap());
    assert!(!write_board_server(&st, &row).unwrap());
    let rows = st
        .list(&store::Filter {
            kinds: vec![KIND_BOARD_SERVER.into()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(rows.len(), 2);
    // `public: null` is written, not left out (v0.5 §6.2).
    assert!(String::from_utf8_lossy(&rows[0].payload).contains("\"public\":null"));
}
