//! Ported from I:internal/identity/identity_test.go and
//! I:internal/device/device_test.go. As in Go, the tests run against the
//! real store: `support::MemStore` is a `store::Store` on a temp dir with
//! the identity folds wired in (see tests/support/mod.rs).

mod support;

use irori_identity::*;
use support::*;

fn mixed_keystore() -> Box<dyn Keystore + Send + Sync> {
    Box::new(AesKeystore::new(
        "other",
        Box::new(|| Ok(zeroize::Zeroizing::new(vec![0u8; 32]))),
    ))
}

#[test]
fn keystores_round_trip() {
    let mut kek = [0u8; 32];
    getrandom::fill(&mut kek).unwrap();
    let kek2 = kek;
    let aes = AesKeystore::new(
        "test-aes",
        Box::new(move || Ok(zeroize::Zeroizing::new(kek2.to_vec()))),
    );
    let dir = tempfile::tempdir().unwrap();
    let mut stores: Vec<Box<dyn Keystore>> = vec![Box::new(FileKeystore), Box::new(aes)];
    // The real Keychain only on request; DPAPI always (it is per-user).
    if platform_keystore() != "keychain" || std::env::var_os("IRORI_TEST_KEYCHAIN").is_some() {
        stores.push(default_keystore(dir.path()));
    }
    let secret = b"0123456789abcdef0123456789abcdef";
    for ks in &stores {
        let w = Wrapped::seal(ks.as_ref(), secret).unwrap();
        let raw = w.to_file_bytes();
        let blob = Wrapped::parse(&raw).unwrap();
        if ks.name() != "file" {
            let text = String::from_utf8(raw.clone()).unwrap();
            let b64 = base64_of(secret);
            assert!(
                !text.contains(&b64),
                "{}: sealed blob holds the secret in the clear",
                ks.name()
            );
        }
        assert_eq!(blob.unseal(ks.as_ref()).unwrap().as_slice(), secret);
        let mut other = String::from_utf8(raw).unwrap();
        other = other.replace(
            &format!("\"keystore\": \"{}\"", ks.name()),
            "\"keystore\": \"elsewhere\"",
        );
        let err = Wrapped::parse(other.as_bytes())
            .unwrap()
            .unseal(ks.as_ref())
            .unwrap_err();
        assert!(
            matches!(err, Error::WrongKeystore { .. }),
            "{}: {err}",
            ks.name()
        );
    }
    // AES-GCM with another KEK can't open the blob.
    let a = AesKeystore::new(
        "k",
        Box::new(move || Ok(zeroize::Zeroizing::new(kek.to_vec()))),
    );
    let b = AesKeystore::new("k", Box::new(|| Ok(zeroize::Zeroizing::new(vec![9u8; 32]))));
    let blob = a.seal(secret).unwrap();
    assert!(b.unseal(&blob).is_err());
}

fn base64_of(b: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(b)
}

#[test]
fn load_creates_once_and_records_keys_in_device_json() {
    let dir = tempfile::tempdir().unwrap();
    let dev = DeviceJson::load_or_create(dir.path()).unwrap();
    assert_eq!(dev.os, goos());
    assert_eq!(dev.id.len(), 32);
    let a = Identity::load(dir.path(), dev.clone(), Some(Box::new(FileKeystore))).unwrap();
    let p = a.current();
    let pub_bytes: [u8; 32] = hex::decode(&p.pub_hex).unwrap().try_into().unwrap();
    assert_eq!(
        (p.root_device_id.as_str(), p.epoch, p.id.as_str()),
        (dev.id.as_str(), 1, person_id_of(&pub_bytes).as_str())
    );
    let reread = DeviceJson::load_or_create(dir.path()).unwrap();
    assert_eq!(reread.person_id, p.id);
    assert_eq!(reread.device_pub, hex::encode(a.device_pub()));
    let b = Identity::load(dir.path(), reread.clone(), Some(Box::new(FileKeystore))).unwrap();
    assert_eq!(b.current(), p, "second Load made new keys");
    assert_eq!(a.device_pub(), b.device_pub());
    for f in [DEVICE_KEY_FILE, PERSON_FILE] {
        let raw = std::fs::read_to_string(dir.path().join(f)).unwrap();
        assert!(
            raw.contains("\"keystore\": \"file\""),
            "{f} does not name its keystore"
        );
    }
    let err = Identity::load(dir.path(), reread.clone(), Some(mixed_keystore())).unwrap_err();
    assert!(matches!(err.root(), Error::WrongKeystore { .. }), "{err}");
    let mut mixed = reread;
    mixed.device_pub = "00".repeat(32);
    assert!(
        Identity::load(dir.path(), mixed, Some(Box::new(FileKeystore))).is_err(),
        "loaded a device.json whose key differs from device.key"
    );
}

#[test]
fn canonical_sorts_keys() {
    let c = Cert {
        person_id: "p".into(),
        device_id: "d".into(),
        device_pub: "k".into(),
        label: "a<b".into(),
        role: ROLE_DESKTOP.into(),
        at: gocompat::time::GoTime::parse_rfc3339("2026-10-27T00:00:00Z").unwrap(),
    };
    assert_eq!(
        String::from_utf8(canonical(&c).unwrap()).unwrap(),
        r#"{"at":"2026-10-27T00:00:00Z","deviceId":"d","devicePub":"k","label":"a<b","personId":"p","role":"desktop"}"#
    );
}

#[test]
fn root_self_cert_is_pinned() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = Node::new(dir.path(), None);
    assert!(
        a.chain.cert(&a.dev()).is_some() && a.chain.has_profile(),
        "the root did not certify itself"
    );
    assert!(a.chain.is_owner(&user_key(&a.dev())));
    assert!(!a.chain.is_owner("user:made-up"));
    assert!(!a.chain.is_owner(&format!("{}/s", a.dev())));
    let n = a.st.list("person.").len();
    assert_eq!(n, 2);
    a.restart();
    assert!(
        a.chain.cert(&a.dev()).is_some(),
        "the chain did not fold the held self-cert at open"
    );
    a.id.ensure_self_cert(&a.st, &a.chain, ROLE_DESKTOP)
        .unwrap();
    assert_eq!(
        a.st.list("person.").len(),
        n,
        "restart wrote another self-cert"
    );
}

#[test]
fn cert_chain_and_sync() {
    let (da, db, dc) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let a = Node::new(da.path(), None);
    let b = Node::new(db.path(), Some(&a));
    assert!(
        b.chain.cert(&b.dev()).is_none(),
        "a non-root device certified itself"
    );
    let b_work = b.st.append(
        &format!("agent:{}/s", b.dev()),
        "task.created",
        &[("taskId", "t")]
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>(),
    );

    assert_eq!(copy_rows(&b, &a), Err(StoreErr::NoCert), "uncertified rows");
    a.certify(&b);
    assert!(a.chain.is_owner(&user_key(&b.dev())));
    assert_eq!(copy_rows(&b, &a), Ok(()));
    assert_eq!(copy_rows(&a, &b), Ok(()));
    assert!(
        b.chain.cert(&b.dev()).is_some(),
        "b did not learn its own cert from a"
    );
    let got: Vec<Row> =
        a.st.list("task.created")
            .into_iter()
            .filter(|r| r.device_id == b.dev())
            .collect();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].sig, b_work.sig);

    // A forged row: right device, wrong key.
    let mut forged = Row {
        device_id: b.dev(),
        seq: b_work.seq + 1,
        hlc: b_work.hlc + (1 << 16),
        actor: "user".into(),
        kind: "task.created".into(),
        payload: b"{}".to_vec(),
        sig: Vec::new(),
    };
    forged.sig = a.id.sign_row(&signing_bytes(forged.as_ref()));
    assert_eq!(a.st.insert(&forged), Err(StoreErr::BadSig));
    // An agent row relabelled as the person fails the signature too.
    let mut relabel = got[0].clone();
    relabel.seq = b_work.seq + 1;
    relabel.actor = "user".into();
    assert_eq!(a.st.insert(&relabel), Err(StoreErr::BadSig));
    let mut unsigned = forged.clone();
    unsigned.sig.clear();
    assert_eq!(a.st.insert(&unsigned), Err(StoreErr::Unsigned));
    // A device the root never certified is refused.
    let c = Node::new(dc.path(), Some(&a));
    let c_row = c.st.append("irori", "task.created", &serde_json::json!({}));
    assert_eq!(a.st.insert(&c_row), Err(StoreErr::NoCert));
    assert!(!a.chain.is_owner(&user_key(&c.dev())));
}

#[test]
fn self_cert_other_than_the_root_is_refused() {
    let (da, dx) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = Node::new(da.path(), None);
    let x = Node::new(dx.path(), Some(&a));
    let add =
        x.id.cert_for(
            &x.dev(),
            &hex::encode(x.id.device_pub()),
            "thief",
            ROLE_DESKTOP,
            now(),
        )
        .unwrap();
    let ev = x.st.append("irori", kinds::PERSON_DEVICE_ADDED, &add);
    assert!(
        x.chain.cert(&x.dev()).is_none(),
        "a non-root self-cert certified its writer"
    );
    assert_eq!(a.st.insert(&ev), Err(StoreErr::NoCert));
    assert!(!a.chain.is_owner(&user_key(&x.dev())));
}

#[test]
fn revoke_cuts_and_moves_rows() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = Node::new(da.path(), None);
    let b = Node::new(db.path(), Some(&a));
    a.certify(&b);
    copy_rows(&a, &b).unwrap();
    let b_rows: Vec<Row> = (0..4)
        .map(|i| {
            b.st.append("irori", "task.created", &serde_json::json!({"i": i}))
        })
        .collect();
    copy_rows(&b, &a).unwrap();
    let cut = b_rows[1].seq;
    a.revoke(&b.dev(), cut);
    assert_eq!(a.chain.cut(&b.dev()), Some(cut));
    // Rows above the cut moved out (refused_events in the store).
    assert_eq!(a.st.vector()[&b.dev()], cut);
    let refused = a.st.refused();
    assert_eq!(refused.len(), 2);
    assert_eq!(refused[0].seq, cut + 1);
    // New rows past the cut are refused; revoked devices stay owners.
    b.st.append("irori", "task.created", &serde_json::Value::Null);
    assert_eq!(copy_rows(&b, &a), Err(StoreErr::PastCut));
    assert!(
        a.chain.is_owner(&user_key(&b.dev())),
        "revoked device's owner key dropped"
    );
}

#[test]
fn mutual_revoke_in_either_order() {
    for pc_first in [true, false] {
        let dirs: Vec<_> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
        let mut root = Node::new(dirs[0].path(), None);
        let pc = Node::new(dirs[1].path(), Some(&root));
        let lap = Node::new(dirs[2].path(), Some(&root));
        root.certify(&pc);
        root.certify(&lap);
        for n in [&pc, &lap] {
            copy_rows(&root, n).unwrap();
        }
        let rp = pc.revoke(&lap.dev(), 0);
        let rl = lap.revoke(&pc.dev(), 0);
        let evs = if pc_first { [rp, rl] } else { [rl, rp] };
        for ev in &evs {
            root.st.insert(ev).unwrap();
        }
        assert!(
            root.chain.cut(&pc.dev()).is_some() && root.chain.cut(&lap.dev()).is_some(),
            "pc_first={pc_first}"
        );
        // The revoke kept past its writer's cut survives a restart.
        root.restart();
        assert!(
            root.chain.cut(&pc.dev()).is_some() && root.chain.cut(&lap.dev()).is_some(),
            "after restart, pc_first={pc_first}"
        );
    }
}

#[test]
fn min_cut_wins() {
    let dirs: Vec<_> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
    let root = Node::new(dirs[0].path(), None);
    let b = Node::new(dirs[1].path(), Some(&root));
    let c = Node::new(dirs[2].path(), Some(&root));
    root.certify(&b);
    root.certify(&c);
    copy_rows(&root, &c).unwrap();
    let r1 = root.revoke(&b.dev(), 5);
    let r2 = c.revoke(&b.dev(), 3);
    c.st.insert(&r1).unwrap();
    root.st.insert(&r2).unwrap();
    for n in [&root, &c] {
        assert_eq!(n.chain.cut(&b.dev()), Some(3));
    }
}

#[test]
fn revoke_below_its_writers_cert_is_raised() {
    let dirs: Vec<_> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
    let a = Node::new(dirs[0].path(), None);
    for _ in 0..3 {
        a.st.append("irori", "task.created", &serde_json::Value::Null);
    }
    let b = Node::new(dirs[1].path(), Some(&a));
    let c = Node::new(dirs[2].path(), Some(&a));
    let cert_b = a.certify(&b);
    a.certify(&c);
    for n in [&b, &c] {
        copy_rows(&a, n).unwrap();
    }
    b.revoke(&a.dev(), 0);
    for dev in [a.dev(), b.dev()] {
        assert!(b.chain.cert(&dev).is_some(), "{dev} lost its cert on b");
    }
    assert_eq!(
        b.chain.cut(&a.dev()),
        Some(cert_b.seq),
        "raised to a's cert of b"
    );
    assert_eq!(b.chain.devices().len(), 2, "a and b");
    assert!(
        b.chain.cert(&c.dev()).is_none(),
        "a cert written past its writer's cut still counts"
    );
    copy_rows(&b, &c).unwrap();
    assert!(c.chain.cut(&a.dev()).is_some());
    assert!(c.chain.cert(&b.dev()).is_some());
    assert_eq!(c.chain.cut(&a.dev()), Some(cert_b.seq));
    assert_eq!((a.chain.bad(), b.chain.bad(), c.chain.bad()), (0, 0, 0));
}

#[test]
fn cut_hash_mismatch_flags_a_fork() {
    let dirs: Vec<_> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
    let a = Node::new(dirs[0].path(), None);
    let b = Node::new(dirs[1].path(), Some(&a));
    a.certify(&b);
    copy_rows(&a, &b).unwrap();
    let b_rows: Vec<Row> = (0..4)
        .map(|i| {
            b.st.append("irori", "task.created", &serde_json::json!({"i": i}))
        })
        .collect();
    let cut_row = b_rows[1].clone();

    // The right hash.
    copy_rows(&b, &a).unwrap();
    a.revoke_hash(&b.dev(), cut_row.seq, &row_hash(cut_row.as_ref()));
    assert!(a.st.forks().is_empty(), "forks for a matching cutHash");
    assert_eq!(
        a.chain.cut_claims(),
        vec![CutClaim {
            device_id: b.dev(),
            seq: cut_row.seq,
            hash: row_hash(cut_row.as_ref())
        }]
    );

    // A wrong hash, revoke first: the row at the cut is checked as it lands.
    let c = Node::new(dirs[2].path(), Some(&a));
    a.certify(&c);
    copy_rows(&a, &c).unwrap();
    let bogus = c.revoke_hash(&b.dev(), cut_row.seq, &"ab".repeat(32));
    assert_eq!(copy_rows(&b, &c), Err(StoreErr::PastCut));
    let f = c.st.forks();
    assert_eq!(f.len(), 1);
    assert_eq!(
        (f[0].reason, f[0].device_id.as_str(), f[0].seq),
        ("cut hash", b.dev().as_str(), cut_row.seq)
    );
    // Rows held first: the revoke is checked against them when it lands.
    a.st.insert(&bogus).unwrap();
    let f = a.st.forks();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].reason, "cut hash");
}

#[test]
fn only_a_signed_copy_is_a_fork() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = Node::new(da.path(), None);
    let b = Node::new(db.path(), Some(&a));
    a.certify(&b);
    copy_rows(&a, &b).unwrap();
    let held: Vec<Row> =
        b.st.list("")
            .into_iter()
            .filter(|r| r.device_id == a.dev())
            .collect();
    assert!(!held.is_empty());
    let mut other = held[0].clone();
    other.payload = br#"{"changed":true}"#.to_vec();
    for n in [&b, &a] {
        let mut junk = other.clone();
        junk.sig = b"garbage".to_vec();
        assert_eq!(n.st.insert(&junk), Err(StoreErr::BadSig));
        junk.sig.clear();
        assert_eq!(n.st.insert(&junk), Err(StoreErr::Unsigned));
        assert!(n.st.forks().is_empty());
    }
    other.sig = a.id.sign_row(&signing_bytes(other.as_ref()));
    assert_eq!(b.st.insert(&other), Err(StoreErr::Fork));
    let f = b.st.forks();
    assert_eq!(f.len(), 1);
    assert_eq!(
        (f[0].reason, f[0].other_hash.clone()),
        ("two rows", row_hash(other.as_ref()))
    );
}

#[test]
fn ensure_self_cert_writes_nothing_off_the_root() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = Node::new(da.path(), None);
    let b = Node::new(db.path(), Some(&a));
    assert!(b.st.list("person.").is_empty());
}

/// Go `TestLoadOrCreateIsStable` (I:internal/device/device_test.go).
#[test]
fn load_or_create_is_stable() {
    let dir = tempfile::tempdir().unwrap();
    let a = DeviceJson::load_or_create(dir.path()).unwrap();
    let b = DeviceJson::load_or_create(dir.path()).unwrap();
    assert!(
        !a.id.is_empty() && a == b,
        "identity changed: {a:?} vs {b:?}"
    );
}

/// Go `TestOwnRowsComeBackAndFutureRowsWait`: a data dir restored without
/// its database holds appends until its own rows come back, and a row
/// stamped well ahead waits without moving the clock.
#[test]
fn own_rows_come_back_and_future_rows_wait() {
    let da = tempfile::tempdir().unwrap();
    let mut a = Node::new(da.path(), None);
    for _ in 0..3 {
        a.st.append_raw("irori", "task.created", b"null".to_vec());
    }
    let held = a.st.missing(&Default::default());
    // Same keys, an empty log.
    let cur = a.id.current();
    a.chain = std::sync::Arc::new(Chain::new(&cur.id, &cur.root_device_id, &a.dev()));
    a.st.reopen_empty(a.chain.clone());
    a.st.hold_appends(held.len() as i64);
    assert_eq!(
        a.st.try_append("irori", "task.created", b"null".to_vec())
            .err(),
        Some(StoreErr::Restoring),
        "append before own rows came back"
    );
    let (n, r) = a.st.insert_batch(&held);
    assert_eq!((n, r), (held.len(), Ok(())), "own rows back");
    a.st.try_append("irori", "task.created", b"null".to_vec())
        .expect("append after catching up");

    // A row stamped 10 minutes ahead waits (not stored, clock untouched).
    let db = tempfile::tempdir().unwrap();
    let b = Node::new(db.path(), Some(&a));
    a.certify(&b);
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 10 * 60 * 1000;
    let mut ahead = Row {
        device_id: b.dev(),
        seq: 1,
        hlc: ms << 16,
        actor: "irori".into(),
        kind: "task.created".into(),
        payload: b"{}".to_vec(),
        sig: Vec::new(),
    };
    ahead.sig = b.id.sign_row(&signing_bytes(ahead.as_ref()));
    assert_eq!(
        a.st.insert(&ahead),
        Err(StoreErr::Ahead),
        "row 10 min ahead"
    );
    let next = a.st.append_raw("irori", "task.created", b"null".to_vec());
    assert!(next.hlc < ahead.hlc, "a held future row moved the clock");
}
