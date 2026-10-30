//! `--from-db` lookup against a fixture log with the store's `events` table
//! (store.md §2.3). Payloads are the byte shapes Go writes (mesh.md §2.6, identity.md §2.2).

use irori_net::fromdb;

fn fixture(rows: &[(&str, i64, i64, &str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let c = rusqlite::Connection::open(dir.path().join("irori.db")).unwrap();
    c.execute_batch(
        "CREATE TABLE events (device_id TEXT NOT NULL, seq INTEGER NOT NULL, hlc INTEGER NOT NULL,
         actor TEXT NOT NULL, kind TEXT NOT NULL, payload BLOB, sig BLOB, PRIMARY KEY (device_id, seq)) WITHOUT ROWID;",
    )
    .unwrap();
    for (dev, seq, hlc, kind, payload) in rows {
        c.execute(
            "INSERT INTO events (device_id, seq, hlc, actor, kind, payload) VALUES (?1, ?2, ?3, 'irori', ?4, ?5)",
            rusqlite::params![dev, seq, hlc, kind, payload.as_bytes()],
        )
        .unwrap();
    }
    dir
}

const PC: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90";
const MAC: &str = "b2c3d4e5f60718293a4b5c6d7e8f9001";
const PERSON: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f0";
const PC_PUB: &str = "2152f8d19b791d24453242e15f2eab6cb7cffa7b6a5ed30097960e069881db12";
const MAC_PUB: &str = "22fc297792f0b6ffc0bfcfdb7edb0c0aa14e025a365ec0e342e86e3829cb74b6";

fn cert(dev: &str, pubk: &str, person: &str) -> String {
    format!(
        r#"{{"cert":{{"personId":"{person}","deviceId":"{dev}","devicePub":"{pubk}","label":"X","role":"desktop","at":"2026-01-02T03:04:05.0000006Z"}},"personPub":"db99","personSig":"0000"}}"#
    )
}

#[test]
fn finds_the_pc_from_its_newest_node_addr_and_cert() {
    let pc_cert = cert(PC, PC_PUB, PERSON);
    let mac_cert = cert(MAC, MAC_PUB, PERSON);
    let other_person = cert(
        MAC,
        "11".repeat(32).as_str(),
        "c3d4e5f60718293a4b5c6d7e8f900112",
    );
    let old = r#"{"v":1,"port":25817,"lan":["172.16.0.5"],"public":null,"via":"","reach":"unmapped","at":1}"#;
    let new = r#"{"v":1,"port":25817,"lan":["192.168.1.20"],"public":{"ip":"203.0.113.7","tcp":25817,"udp":25817},"via":"upnp","reach":"ok","at":2}"#;
    let mac_off = r#"{"v":1,"public":null,"via":"","reach":"off","at":3}"#;
    let db = fixture(&[
        (PC, 1, 100, "person.device.added", &pc_cert),
        (MAC, 1, 50, "person.device.added", &other_person),
        (PC, 2, 200, "person.device.added", &mac_cert),
        (PC, 3, 300, "node.addr", old),
        (PC, 4, 400, "node.addr", new),
        (MAC, 2, 500, "node.addr", mac_off),
    ]);
    let hs = fromdb::home_server(&db.path().join("irori.db"), Some(MAC)).unwrap();
    assert_eq!(hs.device_id, PC);
    assert_eq!(hs.device_pub, PC_PUB);
    assert_eq!(hs.port, Some(25817));
    assert_eq!(
        hs.hosts,
        vec![
            "192.168.1.20".parse::<std::net::IpAddr>().unwrap(),
            "203.0.113.7".parse().unwrap()
        ]
    );
    assert_eq!(hs.public_tcp, Some(25817));

    let mut devs = fromdb::certified_devices(&db.path().join("irori.db"), PERSON).unwrap();
    devs.sort();
    let mut want = vec![PC_PUB.to_string(), MAC_PUB.to_string()];
    want.sort();
    assert_eq!(devs, want);
}

#[test]
fn no_server_row_is_an_error() {
    let db = fixture(&[(PC, 1, 100, "person.device.added", &cert(PC, PC_PUB, PERSON))]);
    assert!(fromdb::home_server(&db.path().join("irori.db"), None).is_err());
}
