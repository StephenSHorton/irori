//! Go-produced vectors (testdata/golden/identity, exported by
//! tools/goldens/export.sh from Go 1.26.5 with `go test -overlay`). Every
//! key is a fixture key derived from a label; nothing real is read.

use std::collections::BTreeMap;

use gocompat::json;
use gocompat::time::GoTime;
use irori_identity::*;
use serde_json::Value;
use sha2::Digest;

fn golden(name: &str) -> Value {
    let p = manifest_dir()
        .join("../../testdata/golden/identity")
        .join(name);
    serde_json::from_slice(&std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display())))
        .unwrap()
}

/// The exporter's `goldKey(label)` seed.
fn seed(label: &str) -> [u8; 32] {
    sha2::Sha256::digest(format!("irori-golden/{label}").as_bytes()).into()
}

fn key_pub(label: &str) -> String {
    hex::encode(
        ed25519_dalek::SigningKey::from_bytes(&seed(label))
            .verifying_key()
            .to_bytes(),
    )
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("not a string: {v}"))
}

fn time(v: &Value) -> GoTime {
    GoTime::parse_rfc3339(s(v)).unwrap()
}

fn marshal<T: serde::Serialize>(v: &T) -> String {
    String::from_utf8(json::marshal(v).unwrap()).unwrap()
}

fn rows(v: &Value) -> Vec<Row> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|r| Row {
            device_id: s(&r["device"]).into(),
            seq: r["seq"].as_i64().unwrap(),
            hlc: r["hlc"].as_u64().unwrap(),
            actor: s(&r["actor"]).into(),
            kind: s(&r["kind"]).into(),
            payload: hex::decode(s(&r["payloadHex"])).unwrap(),
            sig: hex::decode(s(&r["sig"])).unwrap(),
        })
        .collect()
}

fn err_text(r: Result<(), VerifyError>) -> String {
    r.err().map(|e| e.to_string()).unwrap_or_default()
}

#[test]
fn signed_shapes_match_go_bytes_and_signatures() {
    let g = golden("signed.json");
    let p = &g["person"];
    let d = &g["device"];
    let person = Person {
        id: s(&p["personId"]).into(),
        name: s(&p["name"]).into(),
        epoch: p["epoch"].as_i64().unwrap(),
        root_device_id: s(&p["rootDeviceId"]).into(),
        pub_hex: s(&p["personPub"]).into(),
    };
    let device = DeviceJson {
        id: s(&d["id"]).into(),
        name: s(&d["name"]).into(),
        os: s(&d["os"]).into(),
        person_id: s(&d["personId"]).into(),
        device_pub: s(&d["devicePub"]).into(),
    };
    let dir = tempfile::tempdir().unwrap();
    let id = Identity::from_seeds_for_tests(
        dir.path(),
        device,
        seed("dev/pc"),
        person.clone(),
        seed("person/s/1"),
    );
    assert_eq!(hex::encode(id.device_pub()), s(&d["devicePub"]));
    assert_eq!(
        person_id_of(&hex::decode(&person.pub_hex).unwrap().try_into().unwrap()),
        person.id
    );
    let at = time(&g["at"]);
    let at_local = time(&g["atLocal"]);
    assert_eq!(at_local.offset, 3600);
    let board = "0123456789abcdef0123456789abcdef";
    let other = MemberPin {
        person_id: "b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1".into(),
        person_pub: key_pub("person/b/1"),
        root_device_id: "bpc".into(),
        person_epoch: 3,
    };
    let mut rot = KeyRotated {
        person_id: person.id.clone(),
        epoch: 2,
        person_pub: key_pub("person/s/2"),
        at,
        by: "pc-0001".into(),
        prev_sig: String::new(),
    };
    let pk = ed25519_dalek::SigningKey::from_bytes(&seed("person/s/1"));
    rot.prev_sig = sign_person(&pk, &rot.body()).unwrap();
    let cuts: BTreeMap<String, i64> = [("bpc", 7), ("blap", 0), ("a<b", -1)]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

    // (payload, canonical body, signature) for each shape, built the Rust way.
    let mut built: BTreeMap<&str, (String, Vec<u8>, String)> = BTreeMap::new();
    let add = id
        .cert_for("mac-0002", &key_pub("dev/mac"), "Mac <b>", ROLE_DESKTOP, at)
        .unwrap();
    built.insert(
        "cert",
        (
            marshal(&add),
            canonical(&add.cert).unwrap(),
            add.person_sig.clone(),
        ),
    );
    let add2 = id
        .cert_for("node-3", &key_pub("dev/node"), "", ROLE_NODE, at_local)
        .unwrap();
    built.insert(
        "cert-local-time",
        (
            marshal(&add2),
            canonical(&add2.cert).unwrap(),
            add2.person_sig.clone(),
        ),
    );
    let rev = id
        .revoke_of("mac-0002", 42, &"ab".repeat(32), "lost <it>", at)
        .unwrap();
    built.insert(
        "revoke",
        (
            marshal(&rev),
            canonical(&rev.revoke).unwrap(),
            rev.person_sig.clone(),
        ),
    );
    let rev2 = id.revoke_of("mac-0002", 0, "", "", at).unwrap();
    built.insert(
        "revoke-no-reason",
        (
            marshal(&rev2),
            canonical(&rev2.revoke).unwrap(),
            rev2.person_sig.clone(),
        ),
    );
    let prof = id.profile_row().unwrap();
    built.insert(
        "profile",
        (
            marshal(&prof),
            canonical(&prof.body()).unwrap(),
            prof.person_sig.clone(),
        ),
    );
    built.insert(
        "rotation",
        (
            marshal(&rot),
            canonical(&rot.body()).unwrap(),
            rot.prev_sig.clone(),
        ),
    );
    let (mrot, ok) = member_rotation(&rot);
    assert!(ok);
    built.insert(
        "member-rotation",
        (
            marshal(&mrot),
            canonical(&mrot.body()).unwrap(),
            mrot.prev_sig.clone(),
        ),
    );
    let old = KeyRotated {
        by: String::new(),
        prev_sig: String::new(),
        ..rot.clone()
    };
    assert!(
        !member_rotation(&old).1,
        "a pre-v0.5 rotation went on a board"
    );
    built.insert(
        "rotation-v04",
        (
            marshal(&old),
            canonical(&old.body()).unwrap(),
            String::new(),
        ),
    );
    let pin = id.board_pin();
    let bc = id
        .board_created_row(board, "MLG & co", "mlg", &pin, at)
        .unwrap();
    let body = BoardCreated {
        person_sig: String::new(),
        ..bc.clone()
    };
    built.insert(
        "board.created",
        (
            marshal(&bc),
            canonical(&body).unwrap(),
            bc.person_sig.clone(),
        ),
    );
    let br = id.board_renamed_row(1, "ours <x>", at).unwrap();
    let body = BoardRenamed {
        person_sig: String::new(),
        ..br.clone()
    };
    built.insert(
        "board.renamed",
        (
            marshal(&br),
            canonical(&body).unwrap(),
            br.person_sig.clone(),
        ),
    );
    let ba = id
        .board_member_added_row(1, &other, "Blake", "owner", at)
        .unwrap();
    let body = BoardMemberAdded {
        person_sig: String::new(),
        ..ba.clone()
    };
    built.insert(
        "board.member.added",
        (
            marshal(&ba),
            canonical(&body).unwrap(),
            ba.person_sig.clone(),
        ),
    );
    let bro = id
        .board_member_role_row(1, &other.person_id, "member", at)
        .unwrap();
    let body = BoardMemberRole {
        person_sig: String::new(),
        ..bro.clone()
    };
    built.insert(
        "board.member.role",
        (
            marshal(&bro),
            canonical(&body).unwrap(),
            bro.person_sig.clone(),
        ),
    );
    for (name, cut, reason) in [
        ("board.member.removed", Some(cuts.clone()), "spam"),
        ("board.member.removed-nil", None, ""),
        ("board.member.removed-empty", Some(BTreeMap::new()), ""),
    ] {
        let bm = id
            .board_member_removed_row(1, &other.person_id, reason, cut, at)
            .unwrap();
        let body = BoardMemberRemoved {
            person_sig: String::new(),
            ..bm.clone()
        };
        built.insert(
            name,
            (
                marshal(&bm),
                canonical(&body).unwrap(),
                bm.person_sig.clone(),
            ),
        );
    }
    for (name, cut) in [
        (
            "board.member.left",
            Some([("pc-0001".to_string(), 9i64)].into_iter().collect()),
        ),
        ("board.member.left-nil", None),
    ] {
        let bl = id.board_member_left_row(1, cut, at).unwrap();
        let body = BoardMemberLeft {
            person_sig: String::new(),
            ..bl.clone()
        };
        built.insert(
            name,
            (
                marshal(&bl),
                canonical(&body).unwrap(),
                bl.person_sig.clone(),
            ),
        );
    }
    let mc = id
        .member_cert_for(1, "mac-0002", &key_pub("dev/mac"), ROLE_DESKTOP, at)
        .unwrap();
    built.insert(
        "member.device.added",
        (
            marshal(&mc),
            canonical(&mc.cert).unwrap(),
            mc.person_sig.clone(),
        ),
    );
    let mr = id
        .member_revoke_of(1, "mac-0002", 5, &"cd".repeat(32), at)
        .unwrap();
    built.insert(
        "member.device.revoked",
        (
            marshal(&mr),
            canonical(&mr.revoke).unwrap(),
            mr.person_sig.clone(),
        ),
    );
    let mp = id.member_profile_row(1).unwrap();
    built.insert(
        "member.profile",
        (
            marshal(&mp),
            canonical(&mp.body()).unwrap(),
            mp.person_sig.clone(),
        ),
    );
    assert!(
        id.member_cert_for(2, "x", &key_pub("dev/x"), ROLE_DESKTOP, at)
            .is_err(),
        "signed for an epoch the key isn't on"
    );

    let entries = g["entries"].as_array().unwrap();
    assert_eq!(entries.len(), built.len());
    for e in entries {
        let name = s(&e["name"]);
        let (payload, canon, sig) = built
            .get(name)
            .unwrap_or_else(|| panic!("no Rust case {name}"));
        assert_eq!(payload, s(&e["payload"]), "{name}: payload bytes");
        assert_eq!(
            String::from_utf8_lossy(canon),
            s(&e["canonical"]),
            "{name}: canonical bytes"
        );
        assert_eq!(sig, s(&e["sig"]), "{name}: signature");
        if !sig.is_empty() {
            // Go's signature verifies over Rust's canonical bytes.
            let pk = hex::decode(s(&e["signer"])).unwrap();
            assert!(
                ed25519_verify(&pk, canon, &hex::decode(s(&e["sig"])).unwrap()),
                "{name}"
            );
        }
    }

    // identity.md §7.1 SEED and BOARD, and more topic vectors.
    let k5 = ed25519_dalek::SigningKey::from_bytes(&[5u8; 32]);
    assert_eq!(pub_hex(&k5), s(&g["seed5"]["pub"]));
    assert_eq!(
        person_id_of(&k5.verifying_key().to_bytes()),
        s(&g["seed5"]["personId"])
    );
    let t = derive_board_topic(board, 1, &[7u8; 32]).unwrap();
    assert_eq!(hex::encode(t.key), s(&g["boardTopic"]["key"]));
    assert_eq!(t.topic, s(&g["boardTopic"]["topic"]));
    for v in g["topics"].as_array().unwrap() {
        let t = derive_board_topic(
            s(&v["board"]),
            v["epoch"].as_i64().unwrap(),
            &hex::decode(s(&v["secret"])).unwrap(),
        )
        .unwrap();
        assert_eq!(hex::encode(t.key), s(&v["key"]));
        assert_eq!(t.topic, s(&v["topic"]));
    }
    for v in g["badTopics"].as_array().unwrap() {
        let secret = hex::decode(s(&v["secret"])).unwrap();
        assert!(
            derive_board_topic(s(&v["board"]), v["epoch"].as_i64().unwrap(), &secret).is_err(),
            "{v}"
        );
    }
}

/// Go's `CutClaim` JSON (no tags): `[{"DeviceID","Seq","Hash"}]`.
fn claims_json(c: &[CutClaim]) -> String {
    #[derive(serde::Serialize)]
    #[allow(non_snake_case)]
    struct C<'a> {
        DeviceID: &'a str,
        Seq: i64,
        Hash: &'a str,
    }
    let v: Vec<C> = c
        .iter()
        .map(|c| C {
            DeviceID: &c.device_id,
            Seq: c.seq,
            Hash: &c.hash,
        })
        .collect();
    if v.is_empty() {
        "null".into()
    } else {
        marshal(&v)
    }
}

#[test]
fn chain_folds_like_go() {
    let g = golden("chain.json");
    let rs = rows(&g["rows"]);
    let pid = s(&g["personId"]);
    let check = |ch: &Chain, order: &str| {
        assert_eq!(marshal(&ch.devices()), s(&g["devices"]), "{order}: devices");
        for c in g["cuts"].as_array().unwrap() {
            let got = ch.cut(s(&c["device"]));
            let want = c["ok"]
                .as_bool()
                .unwrap()
                .then(|| c["cut"].as_i64().unwrap());
            assert_eq!(got, want, "{order}: cut of {}", c["device"]);
        }
        assert_eq!(
            claims_json(&ch.cut_claims()),
            s(&g["claims"]),
            "{order}: claims"
        );
        assert_eq!(
            ch.has_profile(),
            g["hasProfile"].as_bool().unwrap(),
            "{order}"
        );
        assert_eq!(ch.bad(), g["bad"].as_i64().unwrap(), "{order}: bad");
        for (k, v) in g["owners"].as_object().unwrap() {
            assert_eq!(
                ch.is_owner(k),
                v.as_bool().unwrap(),
                "{order}: IsOwner({k})"
            );
        }
    };
    let ch = Chain::new(pid, s(&g["root"]), s(&g["self"]));
    for r in &rs {
        ch.observe(r.as_ref());
    }
    check(&ch, "log order");
    for (r, want) in rs.iter().zip(g["verify"].as_array().unwrap()) {
        assert_eq!(
            err_text(ch.verify_row(r.as_ref())),
            s(want),
            "VerifyRow {}/{}",
            r.device_id,
            r.seq
        );
    }
    let forged = &rows(&Value::Array(vec![g["forged"].clone()]))[0];
    assert_eq!(
        err_text(ch.verify_row(forged.as_ref())),
        s(&g["forgedVerify"])
    );
    assert!(matches!(
        ch.verify_row(forged.as_ref()),
        Err(VerifyError::BadSig(_))
    ));

    // The fold doesn't depend on arrival order.
    let rev = Chain::new(pid, s(&g["root"]), s(&g["self"]));
    for r in rs.iter().rev() {
        rev.observe(r.as_ref());
    }
    check(&rev, "reversed");
    // Observing a row twice folds it once.
    for r in &rs {
        rev.observe(r.as_ref());
    }
    check(&rev, "twice");

    let fresh = Chain::new(pid, s(&g["root"]), s(&g["self"]));
    for (r, want) in rs.iter().zip(g["bootstrap"].as_array().unwrap()) {
        let got = fresh
            .learn(r.as_ref())
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert_eq!(got, s(want), "Learn {}/{}", r.device_id, r.seq);
    }
}

#[test]
fn board_verifier_folds_like_go() {
    let g = golden("board.json");
    let rs = rows(&g["rows"]);
    let a = &g["anchor"];
    let anchor = MemberPin {
        person_id: s(&a["personId"]).into(),
        person_pub: s(&a["personPub"]).into(),
        root_device_id: s(&a["rootDeviceId"]).into(),
        person_epoch: a["personEpoch"].as_i64().unwrap(),
    };
    let board_id = s(&g["boardId"]);
    let check = |v: &BoardVerifier, order: &str| {
        let ms = v.membership();
        assert_eq!(marshal(&ms), s(&g["membership"]), "{order}: membership");
        assert_eq!(marshal(&ms.pins), s(&g["pins"]), "{order}: pins");
        assert_eq!(
            v.board_epoch(),
            g["boardEpoch"].as_i64().unwrap(),
            "{order}"
        );
        assert_eq!(claims_json(&v.cut_claims()), s(&g["claims"]), "{order}");
        for d in g["devices"].as_array().unwrap() {
            let dev = s(&d["device"]);
            let want_cut = d["cutOk"]
                .as_bool()
                .unwrap()
                .then(|| d["cut"].as_i64().unwrap());
            assert_eq!(v.cut(dev), want_cut, "{order}: cut {dev}");
            assert_eq!(
                v.person_of(dev).unwrap_or_default(),
                s(&d["personOf"]),
                "{order}: PersonOf {dev}"
            );
            assert_eq!(
                v.cert(dev).map(|c| marshal(&c)).unwrap_or_default(),
                s(&d["cert"]),
                "{order}: Cert {dev}"
            );
        }
        for p in g["people"].as_array().unwrap() {
            let pid = s(&p["person"]);
            assert_eq!(
                v.epoch(pid),
                p["epoch"].as_i64().unwrap(),
                "{order}: epoch {pid}"
            );
            assert_eq!(
                v.writable(pid),
                p["writable"].as_bool().unwrap(),
                "{order}: writable {pid}"
            );
            let prof = v.profile(pid);
            assert_eq!(
                prof.is_some(),
                p["hasProfile"].as_bool().unwrap(),
                "{order}"
            );
            assert_eq!(prof.unwrap_or_default(), s(&p["profile"]), "{order}");
            // Go writes no devices as a nil slice, `null`.
            let devs = v.devices(pid);
            let devs = if devs.is_empty() {
                "null".to_string()
            } else {
                marshal(&devs)
            };
            assert_eq!(devs, s(&p["devices"]), "{order}: devices {pid}");
            assert_eq!(
                marshal(&v.pinned(pid).unwrap_or_default()),
                s(&p["pinned"]),
                "{order}"
            );
        }
        for (k, want) in g["keys"].as_object().unwrap() {
            assert_eq!(v.key_person(k), s(want), "{order}: KeyPerson({k})");
        }
        let amb: Vec<String> = g["ambiguous"]
            .as_array()
            .map(|a| a.iter().map(|x| s(x).to_string()).collect())
            .unwrap_or_default();
        assert_eq!(v.ambiguous(), amb, "{order}");
        assert_eq!(v.bad(), g["bad"].as_i64().unwrap(), "{order}: bad");
    };
    let v = BoardVerifier::new_for(board_id, std::slice::from_ref(&anchor));
    for r in &rs {
        v.observe(r.as_ref());
    }
    check(&v, "log order");
    for (r, want) in rs.iter().zip(g["verify"].as_array().unwrap()) {
        assert_eq!(
            err_text(v.verify_row(r.as_ref())),
            s(want),
            "VerifyRow {}/{} {}",
            r.device_id,
            r.seq,
            r.kind
        );
    }
    for at in g["attempts"].as_array().unwrap() {
        let r = &rows(&Value::Array(vec![at["row"].clone()]))[0];
        assert_eq!(
            err_text(v.verify_row(r.as_ref())),
            s(&at["err"]),
            "attempt {} {}",
            r.actor,
            r.kind
        );
    }
    let rev = BoardVerifier::new_for(board_id, std::slice::from_ref(&anchor));
    for r in rs.iter().rev() {
        rev.observe(r.as_ref());
    }
    check(&rev, "reversed");
    // Reset keeps the anchor; refeeding gives the same fold.
    rev.reset();
    assert!(!rev.membership().created);
    for r in &rs {
        rev.observe(r.as_ref());
    }
    check(&rev, "after reset");
}

#[test]
fn files_match_go_writers() {
    let g = golden("files.json");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(PERSON_FILE);
    let p = Person {
        id: person_id_of(
            &ed25519_dalek::SigningKey::from_bytes(&seed("person/s/1"))
                .verifying_key()
                .to_bytes(),
        ),
        name: "Stephen <&>".into(),
        epoch: 3,
        root_device_id: "pc".into(),
        pub_hex: key_pub("person/s/3"),
    };
    irori_identity::person::write_person_file_for_tests(
        &path,
        &FileKeystore,
        &p,
        seed("person/s/3"),
        &[(1, seed("person/s/1")), (2, seed("person/s/2"))],
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        s(&g["personWithOlder"])
    );
    irori_identity::person::write_person_file_for_tests(
        &path,
        &FileKeystore,
        &p,
        seed("person/s/3"),
        &[],
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        s(&g["personNoOlder"])
    );

    // Go's own files load: older epochs, then the legacy prev-only shape,
    // and Load records personId and devicePub in device.json Go's way.
    std::fs::write(dir.path().join(DEVICE_KEY_FILE), s(&g["deviceKey"])).unwrap();
    let dev = DeviceJson {
        id: "pc".into(),
        name: "PC".into(),
        os: "windows".into(),
        ..DeviceJson::default()
    };
    std::fs::write(&path, s(&g["personWithOlder"])).unwrap();
    let id = Identity::load(dir.path(), dev.clone(), Some(Box::new(FileKeystore))).unwrap();
    assert_eq!(id.older_epochs(), vec![1, 2]);
    assert_eq!(id.current(), p);
    assert_eq!(*id.person_seed(1).unwrap(), seed("person/s/1"));
    std::fs::write(&path, s(&g["personLegacy"])).unwrap();
    let id = Identity::load(dir.path(), dev, Some(Box::new(FileKeystore))).unwrap();
    assert_eq!(id.older_epochs(), vec![2]);
    assert_eq!(id.prev_epoch(), 2);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("device.json")).unwrap(),
        s(&g["deviceJson"])
    );

    // Board secrets: the plaintext after each step, then the sealed file.
    let board = "0123456789abcdef0123456789abcdef";
    let spath = dir.path().join(format!("{board}.secrets"));
    let sec = BoardSecrets::open(&spath, board, Box::new(FileKeystore)).unwrap();
    let steps = g["secretSteps"].as_array().unwrap();
    type Op = Box<dyn Fn(&BoardSecrets) -> Result<(), Error>>;
    let ops: Vec<Op> = vec![
        Box::new(|s| s.add(1, "pc", &[7; 32])),
        Box::new(|s| s.add(2, "pc", &[8; 32])),
        Box::new(|s| s.add(2, "aaa-mac", &[9; 32])),
        Box::new(|s| s.add(10, "pc", &[10; 32])),
        Box::new(|s| s.add(3, "pc", &[3; 32])),
        Box::new(|s| s.settle(10)),
    ];
    for (op, want) in ops.iter().zip(steps) {
        op(&sec).unwrap();
        let blob = Wrapped::read(&spath)
            .unwrap()
            .unseal(&FileKeystore)
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&blob), s(want));
        assert_eq!(
            String::from_utf8(sec.plaintext_for_tests()).unwrap(),
            s(want)
        );
    }
    assert_eq!(
        std::fs::read_to_string(&spath).unwrap(),
        s(&g["secretsFile"])
    );
    std::fs::write(&spath, s(&g["secretsFile"])).unwrap();
    let back = BoardSecrets::open(&spath, board, Box::new(FileKeystore)).unwrap();
    let (secret, maker) = back.secret(10).unwrap();
    assert_eq!((secret.as_slice(), maker.as_str()), (&[10u8; 32][..], "pc"));
    assert_eq!(back.epoch(), 10);
}

#[test]
fn tables_match_go() {
    let g = golden("tables.json");
    for (n, ok) in g["names"].as_object().unwrap() {
        assert_eq!(
            valid_board_name(n),
            ok.as_bool().unwrap(),
            "ValidBoardName({n:?})"
        );
    }
    for (t, ok) in g["tags"].as_object().unwrap() {
        assert_eq!(
            valid_board_tag(t),
            ok.as_bool().unwrap(),
            "ValidBoardTag({t:?})"
        );
    }
    for k in g["keys"].as_array().unwrap() {
        let key = s(&k["key"]);
        assert_eq!(owner::device_of(key), s(&k["device"]), "DeviceOf({key:?})");
        assert_eq!(owner::may_write(key, "pc"), k["mayPc"].as_bool().unwrap());
        assert_eq!(
            owner::may_write(key, "other"),
            k["mayOther"].as_bool().unwrap()
        );
    }
    for (kind, e) in g["entities"].as_object().unwrap() {
        assert_eq!(
            owner::entity_of(kind).unwrap_or(""),
            s(e),
            "EntityOf({kind})"
        );
    }
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
