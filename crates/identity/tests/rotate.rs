//! Ported from I:internal/identity/rotate_test.go and the private-row half
//! of board_test.go's TestPrivateRowsGainEpochAndPrevSig.

mod support;

use gocompat::json::{self, Schema};
use irori_identity::*;
use support::*;

fn new_id(dir: &std::path::Path) -> Identity {
    let dev = DeviceJson::load_or_create(dir).unwrap();
    Identity::load(dir, dev, Some(Box::new(FileKeystore))).unwrap()
}

fn reload(id: &Identity) -> Identity {
    Identity::load(
        id.data_dir(),
        id.self_device(),
        Some(Box::new(FileKeystore)),
    )
    .unwrap()
}

#[test]
fn rotate_keeps_the_previous_epoch_and_survives_reload() {
    let dir = tempfile::tempdir().unwrap();
    let id = new_id(dir.path());
    let first = id.current();
    let seed1 = id.person_seed(1).unwrap();
    assert_eq!(id.prev_epoch(), 0);
    let rot = id.rotate(now()).unwrap();
    let cur = id.current();
    assert_eq!((rot.epoch, cur.epoch), (2, 2));
    assert_eq!(rot.person_pub, cur.pub_hex);
    assert_ne!(cur.pub_hex, first.pub_hex);
    assert_eq!(rot.person_id, first.id);
    assert_eq!(*id.person_seed(1).unwrap(), *seed1);
    assert_eq!(id.prev_epoch(), 1);
    // New certs are signed with the new key.
    let add = id
        .cert_for("dev-b", &first.pub_hex, "b", ROLE_DESKTOP, now())
        .unwrap();
    assert_eq!(add.person_pub, cur.pub_hex);
    verify_person(&add.person_pub, &add.person_sig, &add.cert).unwrap();
    let re = reload(&id);
    assert_eq!(re.current(), cur);
    assert_eq!(re.prev_epoch(), 1);
    assert_eq!(*re.person_seed(2).unwrap(), *id.person_seed(2).unwrap());
}

#[test]
fn adopt_secret_checks_the_rotation_row() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (a, b) = (new_id(da.path()), new_id(db.path()));
    let rot = a.rotate(now()).unwrap();
    let seed = a.person_seed(2).unwrap();
    assert!(
        b.adopt_secret(2, &seed[..], &b.current().pub_hex).is_err(),
        "adopted a secret that does not match its rotation row"
    );
    assert!(matches!(
        b.adopt_secret(1, &seed[..], &rot.person_pub),
        Err(Error::StaleEpoch)
    ));
    b.adopt_secret(2, &seed[..], &rot.person_pub).unwrap();
    assert_eq!(
        (b.current().epoch, b.current().pub_hex, b.prev_epoch()),
        (2, rot.person_pub, 1)
    );
}

#[test]
fn every_older_epoch_is_kept_until_forget() {
    let dir = tempfile::tempdir().unwrap();
    let id = new_id(dir.path());
    let seed1 = id.person_seed(1).unwrap();
    for _ in 0..2 {
        id.rotate(now()).unwrap();
    }
    let re = reload(&id);
    assert_eq!(*re.person_seed(1).unwrap(), *seed1);
    assert_eq!((re.current().epoch, re.prev_epoch()), (3, 2));
    re.forget(3).unwrap();
    let re2 = reload(&id);
    assert!(re2.person_seed(1).is_none());
    assert!(re2.older_epochs().is_empty());
}

#[test]
fn replace_secret_swaps_the_current_epochs_key() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (a, b) = (new_id(da.path()), new_id(db.path()));
    let ra = a.rotate(now()).unwrap();
    b.rotate(now()).unwrap();
    let seed = a.person_seed(2).unwrap();
    assert!(
        b.replace_secret(2, &seed[..], &b.current().pub_hex)
            .is_err()
    );
    assert!(matches!(
        b.replace_secret(3, &seed[..], &ra.person_pub),
        Err(Error::StaleEpoch)
    ));
    b.replace_secret(2, &seed[..], &ra.person_pub).unwrap();
    let re = reload(&b);
    assert_eq!(re.current().pub_hex, ra.person_pub);
    assert_eq!(*re.person_seed(2).unwrap(), *seed);
    assert_eq!(re.prev_epoch(), 1);
}

#[test]
fn adopt_person_rewrites_person_and_device() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (a, b) = (new_id(da.path()), new_id(db.path()));
    a.rotate(now()).unwrap();
    let p = a.current();
    let seed = a.person_seed(p.epoch).unwrap();
    let wrong = Person {
        pub_hex: b.current().pub_hex,
        ..p.clone()
    };
    assert!(matches!(
        b.adopt_person(&wrong, &seed[..]),
        Err(Error::NotThisKey)
    ));
    assert!(
        b.adopt_person(
            &Person {
                epoch: 0,
                ..p.clone()
            },
            &seed[..]
        )
        .is_err()
    );
    b.adopt_person(&p, &seed[..]).unwrap();
    let re = reload(&b);
    assert_eq!(re.current(), p);
    assert!(re.older_epochs().is_empty());
    assert_eq!(
        DeviceJson::load_or_create(db.path()).unwrap().person_id,
        p.id
    );
}

#[test]
fn private_rows_gain_epoch_and_prev_sig() {
    let old = Cert {
        person_id: "p".into(),
        device_id: "d".into(),
        device_pub: "k".into(),
        label: "pc".into(),
        role: ROLE_DESKTOP.into(),
        at: unix(1),
    };
    assert_eq!(
        String::from_utf8(canonical(&old).unwrap()).unwrap(),
        r#"{"at":"1970-01-01T00:00:01Z","deviceId":"d","devicePub":"k","label":"pc","personId":"p","role":"desktop"}"#,
        "an epoch-less cert's signed bytes changed"
    );
    let dir = tempfile::tempdir().unwrap();
    let id = new_id(dir.path());
    let prev = id.current();
    let add = id
        .cert_for("d2", &pubh(&rand_key()), "mac", ROLE_DESKTOP, unix(5))
        .unwrap();
    assert_eq!(add.person_epoch, prev.epoch);
    // A v0.4 device decodes into structs with no personEpoch and re-marshals
    // the cert: it still verifies.
    let v04 = Schema::object(vec![
        (
            "cert",
            Schema::object(vec![
                ("personId", Schema::String),
                ("deviceId", Schema::String),
                ("devicePub", Schema::String),
                ("label", Schema::String),
                ("role", Schema::String),
                ("at", Schema::Time),
            ]),
        ),
        ("personPub", Schema::String),
        ("personSig", Schema::String),
    ]);
    let o = json::unmarshal(&to_json(&add), &v04).unwrap();
    let c = Cert::from_out(o.get("cert").unwrap());
    verify_person(o.get_str("personPub"), o.get_str("personSig"), &c).unwrap();
    let rev = id.revoke_of("d2", 3, "h", "lost", unix(5)).unwrap();
    assert_eq!(rev.person_epoch, prev.epoch);
    let back: DeviceRevoked = decode(&to_json(&rev)).unwrap();
    verify_person(&back.person_pub, &back.person_sig, &back.revoke).unwrap();
    let rot = id.rotate(unix(6)).unwrap();
    assert_eq!(
        (rot.by.as_str(), rot.epoch, rot.person_pub.as_str()),
        (
            id.device_id().as_str(),
            prev.epoch + 1,
            id.current().pub_hex.as_str()
        )
    );
    verify_person(&prev.pub_hex, &rot.prev_sig, &rot.body()).unwrap();
    let (m, ok) = member_rotation(&rot);
    assert!(ok);
    verify_person(&prev.pub_hex, &m.prev_sig, &m.body()).unwrap();
    assert!(
        !member_rotation(&KeyRotated {
            person_id: "p".into(),
            epoch: 2,
            person_pub: rot.person_pub.clone(),
            ..KeyRotated::default()
        })
        .1
    );
    assert!(
        id.member_cert_for(prev.epoch, "d3", &pubh(&rand_key()), ROLE_DESKTOP, now())
            .is_err(),
        "signed a board cert for an epoch the key has left"
    );
}

#[test]
fn load_or_create_secret_is_sealed_and_stable() {
    let dir = tempfile::tempdir().unwrap();
    let id = new_id(dir.path());
    let a = id.load_or_create_secret("turn.key").unwrap();
    let b = id.load_or_create_secret("turn.key").unwrap();
    assert_eq!(*a, *b);
    let raw = std::fs::read_to_string(dir.path().join("turn.key")).unwrap();
    assert!(raw.contains("\"keystore\": \"file\""));
}

/// identity.md §7.1: unseal a scratch copy of a real data dir's
/// `device.key` and `person.json` with this OS's keystore and check both
/// keys against their public halves. Prints nothing. The copy must hold
/// `device.json` with `devicePub` and `personId` set, so Load rewrites
/// nothing.
#[test]
#[ignore = "needs IRORI_LIVE_COPY, a scratch copy of device.json, device.key and person.json"]
fn live_copy_loads_with_the_platform_keystore() {
    let dir =
        std::path::PathBuf::from(std::env::var_os("IRORI_LIVE_COPY").expect("IRORI_LIVE_COPY"));
    let before = std::fs::read(dir.join("device.json")).unwrap();
    let dev = DeviceJson::parse(&before).expect("device.json");
    let id = Identity::load(&dir, dev.clone(), None).expect("load the copy");
    let p = id.current();
    assert_eq!(hex::encode(id.device_pub()), dev.device_pub);
    assert_eq!(p.id, dev.person_id);
    let pub_bytes: [u8; 32] = hex::decode(&p.pub_hex).unwrap().try_into().unwrap();
    assert!(
        p.epoch > 1 || person_id_of(&pub_bytes) == p.id,
        "epoch 1's key derives the person id"
    );
    assert_eq!(
        std::fs::read(dir.join("device.json")).unwrap(),
        before,
        "Load rewrote device.json"
    );
}
