//! Ported from I:internal/identity/chain_keys_test.go: a person and a Chain
//! fed rows directly, so rows the store would refuse fold too.

mod support;

use std::collections::BTreeMap;

use ed25519_dalek::SigningKey;
use irori_identity::*;
use serde::Serialize;
use support::*;

struct Fold {
    person: SigningKey,
    pid: String,
    devs: BTreeMap<String, SigningKey>,
    seq: BTreeMap<String, i64>,
    hlc: u64,
    ch: Chain,
}

impl Fold {
    fn new() -> Fold {
        let person = rand_key();
        let pid = person_id_of(&person.verifying_key().to_bytes());
        let ch = Chain::new(&pid, "root", "self");
        Fold {
            person,
            pid,
            devs: BTreeMap::new(),
            seq: BTreeMap::new(),
            hlc: 0,
            ch,
        }
    }

    fn dev(&mut self, id: &str) -> SigningKey {
        self.devs.entry(id.into()).or_insert_with(rand_key).clone()
    }

    fn row<T: Serialize>(&mut self, writer: &str, kind: &str, payload: &T) -> Row {
        *self.seq.entry(writer.into()).or_default() += 1;
        self.hlc += 1;
        let mut r = Row {
            device_id: writer.into(),
            seq: self.seq[writer],
            hlc: self.hlc,
            actor: "irori".into(),
            kind: kind.into(),
            payload: to_json(payload),
            sig: Vec::new(),
        };
        r.sign(&self.dev(writer));
        r
    }

    fn cert(&mut self, writer: &str, dev: &str, pk: &SigningKey) -> Row {
        let c = Cert {
            person_id: self.pid.clone(),
            device_id: dev.into(),
            device_pub: pubh(&self.dev(dev)),
            label: dev.into(),
            role: ROLE_DESKTOP.into(),
            at: unix(1),
        };
        let p = DeviceAdded {
            person_pub: pubh(pk),
            person_sig: sign_hex(pk, &c),
            cert: c,
            person_epoch: 0,
        };
        self.row(writer, kinds::PERSON_DEVICE_ADDED, &p)
    }

    fn revoke(&mut self, writer: &str, dev: &str, pk: &SigningKey) -> Row {
        let r = Revoke {
            person_id: self.pid.clone(),
            device_id: dev.into(),
            cut: 0,
            at: unix(2),
            ..Revoke::default()
        };
        let p = DeviceRevoked {
            person_pub: pubh(pk),
            person_sig: sign_hex(pk, &r),
            revoke: r,
            person_epoch: 0,
        };
        self.row(writer, kinds::PERSON_DEVICE_REVOKED, &p)
    }

    fn rotation(&mut self, writer: &str, epoch: i64, pk: &SigningKey) -> Row {
        let r = KeyRotated {
            person_id: self.pid.clone(),
            epoch,
            person_pub: pubh(pk),
            at: unix(3),
            ..KeyRotated::default()
        };
        self.row(writer, kinds::PERSON_KEY_ROTATED, &r)
    }

    fn profile(&mut self, writer: &str, root: &str, pk: &SigningKey) -> Row {
        let b = ProfileBody {
            person_id: self.pid.clone(),
            name: "Stephen".into(),
            root_device_id: root.into(),
            person_pub: pubh(pk),
        };
        let p = Profile {
            person_id: b.person_id.clone(),
            name: b.name.clone(),
            root_device_id: b.root_device_id.clone(),
            person_pub: b.person_pub.clone(),
            person_sig: sign_hex(pk, &b),
        };
        self.row(writer, kinds::PERSON_PROFILE, &p)
    }

    fn observe(&self, r: &Row) {
        self.ch.observe(r.as_ref());
    }

    fn certified(&self, dev: &str) -> bool {
        self.ch.cert(dev).is_some()
    }
}

#[test]
fn cert_by_another_key_certifies_nobody() {
    let mut f = Fold::new();
    let pk = f.person.clone();
    let r = f.cert("root", "root", &pk);
    f.observe(&r);
    assert!(
        f.certified("root"),
        "the root's self-cert by the first key didn't count"
    );
    let r = f.cert("root", "b", &rand_key());
    f.observe(&r);
    assert!(
        !f.certified("b") && !f.ch.is_owner(&user_key("b")),
        "a cert signed by a stranger's key certified b"
    );
    let mut g = Fold::new();
    let r = g.cert("root", "root", &rand_key());
    g.observe(&r);
    assert!(
        !g.certified("root"),
        "a root self-cert by a stranger's key counted"
    );
}

#[test]
fn rotated_key_counts_in_either_order() {
    let next = rand_key();
    for rotation_first in [true, false] {
        let mut f = Fold::new();
        let pk = f.person.clone();
        let mut rows = vec![f.cert("root", "root", &pk)];
        let rot = f.rotation("root", 2, &next);
        let cert_b = f.cert("root", "b", &next);
        if rotation_first {
            rows.extend([rot, cert_b]);
        } else {
            rows.extend([cert_b, rot]);
        }
        for r in &rows {
            f.observe(r);
        }
        assert!(f.certified("b"), "a cert by the rotated key didn't count");
        let third = rand_key();
        let r = f.cert("b", "c", &third);
        f.observe(&r);
        assert!(
            !f.certified("c"),
            "c certified before any rotation named its key"
        );
        let r = f.rotation("b", 3, &third);
        f.observe(&r);
        assert!(f.certified("c"), "b's rotation didn't make its key count");
    }
    let mut f = Fold::new();
    let pk = f.person.clone();
    let r = f.cert("root", "root", &pk);
    f.observe(&r);
    let stranger = rand_key();
    let r = f.rotation("x", 2, &stranger);
    f.observe(&r);
    let r = f.cert("root", "b", &stranger);
    f.observe(&r);
    assert!(
        !f.certified("b"),
        "a rotation row from an uncertified device named a key"
    );
}

#[test]
fn revoke_by_another_key_is_not_honoured() {
    let mut f = Fold::new();
    let pk = f.person.clone();
    for r in [
        f.cert("root", "root", &pk),
        f.cert("root", "b", &pk),
        f.revoke("b", "root", &rand_key()),
    ] {
        f.observe(&r);
    }
    assert!(
        f.ch.cut("root").is_none(),
        "a revoke signed by a stranger's key cut the root"
    );
    let r = f.revoke("b", "root", &pk);
    f.observe(&r);
    assert!(
        f.ch.cut("root").is_some(),
        "a revoke by the person's key wasn't honoured"
    );
}

#[test]
fn profile_is_checked() {
    let mut f = Fold::new();
    let pk = f.person.clone();
    let r = f.cert("root", "root", &pk);
    f.observe(&r);
    let mut forged = f.profile("root", "root", &pk);
    let mut p: Profile = decode(&forged.payload).unwrap();
    p.name = "someone else".into();
    forged.payload = to_json(&p);
    let bad = f.ch.bad();
    f.observe(&forged);
    assert!(!f.ch.has_profile());
    assert_eq!(f.ch.bad(), bad + 1);
    let cases = [
        ("a stranger's key", f.profile("root", "root", &rand_key())),
        ("another root", f.profile("root", "other", &pk)),
        ("an uncertified dev", f.profile("x", "root", &pk)),
    ];
    for (name, r) in &cases {
        f.observe(r);
        assert!(!f.ch.has_profile(), "a profile by {name} counted");
    }
    let r = f.profile("root", "root", &pk);
    f.observe(&r);
    assert!(f.ch.has_profile(), "the person's profile didn't count");
}

#[test]
fn verify_row_bootstrap_needs_the_person_key() {
    let mut f = Fold::new();
    let pk = f.person.clone();
    let r = f.cert("root", "root", &rand_key());
    assert!(matches!(
        f.ch.verify_row(r.as_ref()),
        Err(VerifyError::NoCert(_))
    ));
    let ev = f.cert("root", "root", &pk);
    let mut p: DeviceAdded = decode(&ev.payload).unwrap();
    p.person_sig = sign_hex(
        &pk,
        &Cert {
            person_id: f.pid.clone(),
            device_id: "root".into(),
            ..Cert::default()
        },
    );
    *f.seq.get_mut("root").unwrap() -= 1;
    f.hlc -= 1;
    let r = f.row("root", kinds::PERSON_DEVICE_ADDED, &p);
    assert!(
        matches!(f.ch.verify_row(r.as_ref()), Err(VerifyError::NoCert(_))),
        "a person signature over other bytes"
    );
    let good = f.cert("root", "root", &pk);
    f.ch.learn(good.as_ref()).unwrap();
    assert!(f.certified("root"));
    // Learn refuses other kinds.
    assert!(matches!(
        f.ch.learn(
            Row {
                kind: "task.created".into(),
                ..good.clone()
            }
            .as_ref()
        ),
        Err(LearnError::NotPerson(_))
    ));
}

#[test]
fn owner_only_and_user_key() {
    let is = owner_only("pc");
    assert!(is("user:pc") && !is("user:mac") && !is("pc"));
    assert_eq!(user_key("mac"), "user:mac");
}
