//! Ported from I:internal/identity/board_test.go and
//! membership_board_test.go: a board Verifier fed rows directly, and in a
//! board log (`support::MemStore` standing in for the store; the store's
//! refusal counters are the store crate's to test).

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use ed25519_dalek::SigningKey;
use irori_identity::*;
use serde::Serialize;
use support::*;

#[derive(Clone)]
struct BPerson {
    pid: String,
    root: String,
    keys: BTreeMap<i64, SigningKey>,
}

impl BPerson {
    fn pin(&self) -> MemberPin {
        MemberPin {
            person_id: self.pid.clone(),
            person_pub: pubh(&self.keys[&1]),
            root_device_id: self.root.clone(),
            person_epoch: 1,
        }
    }
}

struct BFold {
    ppl: BTreeMap<String, BPerson>,
    devs: BTreeMap<String, SigningKey>,
    seq: BTreeMap<String, i64>,
    hlc: u64,
    v: BoardVerifier,
}

impl BFold {
    fn new() -> BFold {
        BFold {
            ppl: BTreeMap::new(),
            devs: BTreeMap::new(),
            seq: BTreeMap::new(),
            hlc: 0,
            v: BoardVerifier::new(&[]),
        }
    }

    /// A person rooted at `root`, pinned.
    fn person(&mut self, name: &str, root: &str) -> BPerson {
        let p = self.stranger(name, root);
        self.v.pin(&p.pin()).unwrap();
        p
    }

    /// A person the board doesn't anchor.
    fn stranger(&mut self, name: &str, root: &str) -> BPerson {
        let k = rand_key();
        let p = BPerson {
            pid: person_id_of(&k.verifying_key().to_bytes()),
            root: root.into(),
            keys: [(1, k)].into_iter().collect(),
        };
        self.ppl.insert(name.into(), p.clone());
        p
    }

    fn dev(&mut self, id: &str) -> SigningKey {
        self.devs.entry(id.into()).or_insert_with(rand_key).clone()
    }

    fn row<T: Serialize>(&mut self, writer: &str, actor: &str, kind: &str, payload: &T) -> Row {
        *self.seq.entry(writer.into()).or_default() += 1;
        self.hlc += 1;
        let mut r = Row {
            device_id: writer.into(),
            seq: self.seq[writer],
            hlc: self.hlc,
            actor: actor.into(),
            kind: kind.into(),
            payload: to_json(payload),
            sig: Vec::new(),
        };
        r.sign(&self.dev(writer));
        r
    }

    fn unseq(&mut self, writer: &str) {
        *self.seq.get_mut(writer).unwrap() -= 1;
        self.hlc -= 1;
    }

    fn cert(&mut self, p: &BPerson, epoch: i64, writer: &str, dev: &str) -> Row {
        let c = MemberCert {
            person_id: p.pid.clone(),
            device_id: dev.into(),
            device_pub: pubh(&self.dev(dev)),
            role: ROLE_DESKTOP.into(),
            person_epoch: epoch,
            at: unix(1),
        };
        let a = MemberDeviceAdded {
            person_sig: sign_hex(&p.keys[&epoch], &c),
            cert: c,
        };
        self.row(writer, "irori", kinds::MEMBER_DEVICE_ADDED, &a)
    }

    fn revoke(&mut self, p: &BPerson, epoch: i64, writer: &str, dev: &str, cut: i64) -> Row {
        let r = MemberRevoke {
            person_id: p.pid.clone(),
            device_id: dev.into(),
            board_cut: cut,
            person_epoch: epoch,
            at: unix(2),
            ..Default::default()
        };
        let a = MemberDeviceRevoked {
            person_sig: sign_hex(&p.keys[&epoch], &r),
            revoke: r,
        };
        self.row(writer, "irori", kinds::MEMBER_DEVICE_REVOKED, &a)
    }

    /// `by` rotating `p` to `epoch` with a new key, prevSig by `prev`'s.
    fn rotate(&mut self, p: &BPerson, prev: i64, epoch: i64, by: &str) -> (Row, SigningKey) {
        let k = rand_key();
        let mut r = MemberKeyRotated {
            person_id: p.pid.clone(),
            person_epoch: epoch,
            person_pub: pubh(&k),
            by: by.into(),
            at: unix(3),
            prev_sig: String::new(),
        };
        r.prev_sig = sign_hex(&p.keys[&prev], &r.body());
        (self.row(by, "irori", kinds::MEMBER_KEY_ROTATED, &r), k)
    }

    fn profile(&mut self, writer: &str, pid: &str, name: &str) -> Row {
        let p = MemberProfile {
            person_id: pid.into(),
            name: name.into(),
            ..Default::default()
        };
        self.row(writer, "irori", kinds::MEMBER_PROFILE, &p)
    }

    fn board_created(&mut self, p: &BPerson) -> Row {
        let mut r = BoardCreated {
            name: "mlg".into(),
            tag: "mlg".into(),
            creator: p.pin(),
            at: unix(1),
            ..Default::default()
        };
        r.person_sig = sign_hex(&p.keys[&1], &r);
        let root = p.root.clone();
        self.row(
            &root,
            &format!("person:{}", p.pid),
            kinds::BOARD_CREATED,
            &r,
        )
    }

    fn member_added(&mut self, by: &BPerson, writer: &str, who: &BPerson, role: &str) -> Row {
        let pin = who.pin();
        let mut r = BoardMemberAdded {
            person_id: pin.person_id,
            person_pub: pin.person_pub,
            root_device_id: pin.root_device_id,
            person_epoch: 1,
            name: "x".into(),
            role: role.into(),
            by: by.pid.clone(),
            by_epoch: 1,
            at: unix(2),
            person_sig: String::new(),
        };
        r.person_sig = sign_hex(&by.keys[&1], &r);
        self.row(
            writer,
            &format!("person:{}", by.pid),
            kinds::BOARD_MEMBER_ADDED,
            &r,
        )
    }

    fn member_removed(
        &mut self,
        by: &BPerson,
        writer: &str,
        who: &BPerson,
        cut: &[(&str, i64)],
    ) -> Row {
        let mut r = BoardMemberRemoved {
            person_id: who.pid.clone(),
            by: by.pid.clone(),
            cut: Some(cut.iter().map(|(k, v)| (k.to_string(), *v)).collect()),
            at: unix(3),
            person_epoch: 1,
            ..Default::default()
        };
        r.person_sig = sign_hex(&by.keys[&1], &r);
        self.row(
            writer,
            &format!("person:{}", by.pid),
            kinds::BOARD_MEMBER_REMOVED,
            &r,
        )
    }

    fn member_left(&mut self, who: &BPerson, writer: &str, cut: &[(&str, i64)]) -> Row {
        let mut r = BoardMemberLeft {
            person_id: who.pid.clone(),
            cut: Some(cut.iter().map(|(k, v)| (k.to_string(), *v)).collect()),
            at: unix(4),
            person_epoch: 1,
            person_sig: String::new(),
        };
        r.person_sig = sign_hex(&who.keys[&1], &r);
        self.row(
            writer,
            &format!("person:{}", who.pid),
            kinds::BOARD_MEMBER_LEFT,
            &r,
        )
    }

    fn feed(&self, rows: &[&Row]) {
        for r in rows {
            self.v.observe(r.as_ref());
        }
    }

    fn certified(&self, dev: &str) -> bool {
        self.v.cert(dev).is_some()
    }

    /// Every arrival order of `rows`, each into a fresh Verifier with the
    /// same pins.
    fn orders(&self, rows: &[Row], check: &mut dyn FnMut(&BoardVerifier)) {
        let pins: Vec<MemberPin> = self
            .ppl
            .values()
            .filter_map(|p| self.v.pinned(&p.pid))
            .collect();
        perm(rows.len(), &mut |idx| {
            let v = BoardVerifier::new(&pins);
            for &i in idx {
                v.observe(rows[i].as_ref());
            }
            check(&v);
        });
    }
}

fn no_cert(r: Result<(), VerifyError>) -> bool {
    matches!(r, Err(VerifyError::NoCert(_)))
}

#[test]
fn board_cert_chains_to_the_pin() {
    let mut f = BFold::new();
    let s = f.person("stephen", "pc");
    let root = f.cert(&s, 1, "pc", "pc");
    f.v.verify_row(root.as_ref()).unwrap();
    let mac = f.cert(&s, 1, "pc", "mac");
    assert!(
        no_cert(f.v.verify_row(mac.as_ref())),
        "pc's row before its cert folded"
    );
    let selfc = f.cert(&s, 1, "rogue", "rogue");
    let wrong_key = {
        let c = MemberCert {
            person_id: s.pid.clone(),
            device_id: "laptop".into(),
            device_pub: pubh(&f.dev("laptop")),
            role: ROLE_DESKTOP.into(),
            person_epoch: 1,
            ..Default::default()
        };
        let a = MemberDeviceAdded {
            person_sig: sign_hex(&rand_key(), &c),
            cert: c,
        };
        f.row("pc", "irori", kinds::MEMBER_DEVICE_ADDED, &a)
    };
    let mut wrong_epoch = f.cert(&s, 1, "pc", "node");
    {
        let mut p: MemberDeviceAdded = decode(&wrong_epoch.payload).unwrap();
        p.cert.person_epoch = 2;
        p.person_sig = sign_hex(&s.keys[&1], &p.cert);
        f.unseq("pc");
        wrong_epoch = f.row("pc", "irori", kinds::MEMBER_DEVICE_ADDED, &p);
    }
    f.feed(&[&root, &mac, &selfc, &wrong_key, &wrong_epoch]);
    assert!(
        f.certified("pc") && f.certified("mac"),
        "pc and mac chain to the pin"
    );
    assert!(!f.certified("rogue"), "a device certified itself");
    assert!(
        !f.certified("laptop"),
        "a cert signed by another key counted"
    );
    assert!(
        !f.certified("node"),
        "a cert naming an unknown epoch counted"
    );
    let pid = s.pid.clone();
    let r = f.profile("mac", &pid, "");
    f.v.verify_row(r.as_ref()).unwrap();
    let mut forged = f.profile("mac", &pid, "");
    forged.sign(&rand_key());
    assert!(matches!(
        f.v.verify_row(forged.as_ref()),
        Err(VerifyError::BadSig(_))
    ));
    // A self-cert for the root by a key that isn't the pinned one is no
    // bootstrap.
    let mut g = BFold::new();
    let b = g.person("blake", "bpc");
    let c = MemberCert {
        person_id: b.pid.clone(),
        device_id: "bpc".into(),
        device_pub: pubh(&g.dev("bpc")),
        person_epoch: 1,
        ..Default::default()
    };
    let a = MemberDeviceAdded {
        person_sig: sign_hex(&rand_key(), &c),
        cert: c,
    };
    let r = g.row("bpc", "irori", kinds::MEMBER_DEVICE_ADDED, &a);
    assert!(no_cert(g.v.verify_row(r.as_ref())));
}

#[test]
fn board_older_epoch_after_rotation_is_refused() {
    let mut f = BFold::new();
    let mut s = f.person("stephen", "pc");
    let root = f.cert(&s, 1, "pc", "pc");
    let mac = f.cert(&s, 1, "pc", "mac");
    let (rot, k2) = f.rotate(&s, 1, 2, "pc");
    s.keys.insert(2, k2);
    let stale = f.cert(&s, 1, "mac", "laptop");
    let fresh = f.cert(&s, 2, "mac", "node");
    let pid = s.pid.clone();
    f.orders(&[root, mac, rot, stale, fresh], &mut |v| {
        assert!(
            v.cert("mac").is_some(),
            "mac, certified before the rotation, stopped counting"
        );
        assert!(
            v.cert("laptop").is_none(),
            "a cert by the ended epoch's key after its end counted"
        );
        assert!(
            v.cert("node").is_some() && v.epoch(&pid) == 2,
            "the epoch 2 cert: epoch {}",
            v.epoch(&pid)
        );
    });
}

#[test]
fn board_rotation_needs_prev_sig_and_its_device() {
    let mut f = BFold::new();
    let s = f.person("stephen", "pc");
    let (a, b) = (f.cert(&s, 1, "pc", "pc"), f.cert(&s, 1, "pc", "mac"));
    f.feed(&[&a, &b]);
    let k = rand_key();
    let mut bad = MemberKeyRotated {
        person_id: s.pid.clone(),
        person_epoch: 2,
        person_pub: pubh(&k),
        by: "mac".into(),
        at: unix(3),
        prev_sig: String::new(),
    };
    bad.prev_sig = sign_hex(&rand_key(), &bad.body());
    let r = f.row("mac", "irori", kinds::MEMBER_KEY_ROTATED, &bad);
    f.feed(&[&r]);
    assert_eq!(
        f.v.epoch(&s.pid),
        1,
        "a rotation with a forged prevSig counted"
    );
    let mut other = MemberKeyRotated {
        by: "pc".into(),
        ..bad.clone()
    };
    other.prev_sig = sign_hex(&s.keys[&1], &other.body());
    let r = f.row("mac", "irori", kinds::MEMBER_KEY_ROTATED, &other);
    let bad_before = f.v.bad();
    f.feed(&[&r]);
    assert_eq!(
        f.v.epoch(&s.pid),
        1,
        "a rotation written by a device other than its By counted"
    );
    assert_eq!(f.v.bad(), bad_before + 1);
    let (stranger, _) = f.rotate(&s, 1, 2, "stranger");
    f.feed(&[&stranger]);
    assert_eq!(
        f.v.epoch(&s.pid),
        1,
        "a rotation by an uncertified device counted"
    );
    let (good, _) = f.rotate(&s, 1, 2, "mac");
    f.feed(&[&good]);
    assert_eq!(f.v.epoch(&s.pid), 2, "a valid rotation by mac didn't count");
}

#[test]
fn board_two_rotations_take_the_lower_by() {
    let mut f = BFold::new();
    let mut s = f.person("stephen", "a-pc");
    let root = f.cert(&s, 1, "a-pc", "a-pc");
    let mac = f.cert(&s, 1, "a-pc", "b-mac");
    let (from_mac, mac_key) = f.rotate(&s, 1, 2, "b-mac");
    let (from_pc, pc_key) = f.rotate(&s, 1, 2, "a-pc");
    s.keys.insert(2, pc_key);
    let by_pc = f.cert(&s, 2, "a-pc", "node");
    s.keys.insert(2, mac_key);
    let by_mac = f.cert(&s, 2, "b-mac", "laptop");
    let pid = s.pid.clone();
    f.orders(&[root, mac, from_mac, from_pc, by_pc, by_mac], &mut |v| {
        assert_eq!(v.epoch(&pid), 2);
        assert!(
            v.cert("node").is_some(),
            "a-pc's key, the lower By, didn't win epoch 2"
        );
        assert!(
            v.cert("laptop").is_none(),
            "b-mac's key signed a cert that counted"
        );
    });
}

#[test]
fn board_rotation_by_a_revoked_device_is_discarded() {
    let mut f = BFold::new();
    let mut s = f.person("stephen", "pc");
    let root = f.cert(&s, 1, "pc", "pc");
    let thief_cert = f.cert(&s, 1, "pc", "a-thief");
    let (thief_rot, thief_key) = f.rotate(&s, 1, 2, "a-thief");
    let rev = f.revoke(&s, 1, "pc", "a-thief", 0);
    let (pc_rot, pc_key) = f.rotate(&s, 1, 2, "pc");
    s.keys.insert(2, pc_key);
    let node = f.cert(&s, 2, "pc", "node");
    s.keys.insert(2, thief_key);
    let fake = f.cert(&s, 2, "a-thief", "fake");
    let pid = s.pid.clone();
    f.orders(
        &[root, thief_cert, thief_rot, rev, pc_rot, node, fake],
        &mut |v| {
            assert!(v.cut("a-thief").is_some(), "the thief isn't revoked");
            assert!(v.cert("node").is_some(), "pc's rotation didn't win epoch 2");
            assert!(
                v.cert("fake").is_none(),
                "the thief's key certified a device"
            );
            assert!(v.writable(&pid), "the person can't write after pc rotated");
        },
    );
}

#[test]
fn board_mutual_revoke_freezes_the_person() {
    let mut f = BFold::new();
    let mut s = f.person("stephen", "pc");
    let root = f.cert(&s, 1, "pc", "pc");
    let thief_cert = f.cert(&s, 1, "pc", "thief");
    let (thief_rot, thief_key) = f.rotate(&s, 1, 2, "thief");
    let mine = f.revoke(&s, 1, "pc", "thief", 0);
    let (pc_rot, _) = f.rotate(&s, 1, 2, "pc");
    s.keys.insert(2, thief_key);
    let theirs = f.revoke(&s, 2, "thief", "pc", 1);
    let late = f.cert(&s, 1, "pc", "late");
    let pid = s.pid.clone();
    f.orders(
        &[root, thief_cert, thief_rot, mine, pc_rot, theirs, late],
        &mut |v| {
            assert!(
                v.cut("thief").is_some() && v.cut("pc").is_some(),
                "mutual revoke"
            );
            assert!(
                v.epoch(&pid) == 1 && !v.writable(&pid),
                "epoch {} writable {}: both rotations should be discarded",
                v.epoch(&pid),
                v.writable(&pid)
            );
            assert!(
                v.cert("late").is_none(),
                "a new cert counted after both rotations were discarded"
            );
        },
    );
}

fn board_store(pins: &[MemberPin]) -> (Arc<BoardVerifier>, MemStore) {
    let v = Arc::new(BoardVerifier::new(pins));
    let st = MemStore::new("self", v.clone(), None, true);
    (v, st)
}

#[test]
fn board_revoke_cuts_in_the_store() {
    let mut f = BFold::new();
    let s = f.person("stephen", "pc");
    let b = f.person("blake", "bpc");
    let root = f.cert(&s, 1, "pc", "pc");
    let mac = f.cert(&s, 1, "pc", "mac");
    let m1 = f.profile("mac", &s.pid, "x");
    let m2 = f.profile("mac", &s.pid, "y");
    let broot = f.cert(&b, 1, "bpc", "bpc");
    let (v, st) = board_store(&[s.pin(), b.pin()]);
    for ev in [&root, &mac, &m1, &broot] {
        st.insert(ev)
            .unwrap_or_else(|e| panic!("{}/{}: {e:?}", ev.device_id, ev.seq));
    }
    let rev = f.revoke(&s, 1, "pc", "mac", 1);
    st.insert(&rev).unwrap();
    assert_eq!(v.cut("mac"), Some(1));
    assert_eq!(st.insert(&m2), Err(StoreErr::PastCut));
    let r = f.revoke(&b, 1, "bpc", "pc", 0);
    st.insert(&r).unwrap();
    assert!(v.cut("pc").is_none(), "blake's revoke cut stephen's device");
    // A device certified by both people counts for neither, unless it is
    // one's pinned root.
    let r = f.cert(&b, 1, "bpc", "pc");
    st.insert(&r).unwrap();
    assert_eq!(
        v.person_of("pc"),
        Some(s.pid.clone()),
        "stephen's pinned root"
    );
    let r = f.cert(&b, 1, "bpc", "shared");
    st.insert(&r).unwrap();
    let r = f.cert(&s, 1, "pc", "shared");
    st.insert(&r).unwrap();
    assert!(v.person_of("shared").is_none());
    assert_eq!(v.ambiguous().len(), 2, "{:?}", v.ambiguous());
}

#[test]
fn board_actors_are_the_writers_person() {
    let mut f = BFold::new();
    let s = f.person("stephen", "pc");
    let b = f.person("blake", "bpc");
    let (v, st) = board_store(&[s.pin(), b.pin()]);
    for ev in [f.cert(&s, 1, "pc", "pc"), f.cert(&b, 1, "bpc", "bpc")] {
        st.insert(&ev).unwrap();
    }
    let card = |id: &str, person: &str| serde_json::json!({"cardId": id, "personId": person, "name": "lead", "kind": "claude", "status": "active"});
    let r = f.row("pc", "agent:sc", kinds::AGENT_CARD, &card("sc", &s.pid));
    st.insert(&r).unwrap();
    assert_eq!(v.key_person("agent:sc"), s.pid);
    let comment = serde_json::json!({"taskId": "t1", "commentId": "c1", "text": "hi"});
    let attempts: Vec<(&str, String, &str, serde_json::Value)> = vec![
        (
            "blake writing as person:stephen",
            format!("person:{}", s.pid),
            "task.comment",
            comment.clone(),
        ),
        (
            "blake minting a card with stephen's personId",
            "agent:fake".into(),
            kinds::AGENT_CARD,
            card("fake", &s.pid),
        ),
        (
            "blake acting as stephen's agent",
            "agent:sc".into(),
            "task.comment",
            comment.clone(),
        ),
        (
            "blake writing stephen's member rows",
            "irori".into(),
            kinds::MEMBER_PROFILE,
            serde_json::json!({"personId": s.pid}),
        ),
        (
            "blake announcing pc as a server",
            "irori".into(),
            kinds::BOARD_SERVER,
            serde_json::json!({"deviceId": "pc", "port": 1}),
        ),
        (
            "blake's card claiming for stephen's agent",
            "agent:bc".into(),
            "task.claimed",
            serde_json::json!({"taskId": "t1", "by": "agent:sc"}),
        ),
        (
            "blake's irori row credited to stephen",
            "irori".into(),
            "task.left",
            serde_json::json!({"taskId": "t1", "who": "agent:sc", "by": format!("person:{}", s.pid)}),
        ),
    ];
    for (name, actor, kind, payload) in &attempts {
        let r = f.row("bpc", actor, kind, payload);
        assert_eq!(st.insert(&r), Err(StoreErr::BoardActor), "{name}");
        f.unseq("bpc");
        f.hlc += 1;
    }
    // A card not held yet is a row that waits, not a forgery.
    let r = f.row("bpc", "agent:later", "task.comment", &comment);
    assert_eq!(st.insert(&r), Err(StoreErr::NoCert));
    f.unseq("bpc");
    f.hlc += 1;
    for ev in [
        f.row("bpc", "agent:bc", kinds::AGENT_CARD, &card("bc", &b.pid)),
        f.row(
            "bpc",
            "irori",
            kinds::BOARD_SERVER,
            &serde_json::json!({"deviceId": "bpc", "port": 1}),
        ),
    ] {
        st.insert(&ev)
            .unwrap_or_else(|e| panic!("{}: {e:?}", ev.kind));
    }
    assert_eq!(v.key_person("agent:bc"), b.pid);
    assert_eq!(v.key_person(&format!("person:{}", b.pid)), b.pid);
    assert_eq!(v.key_person("person:nobody"), "");
    // The cards fold again from the log alone.
    st.reload();
    assert_eq!(
        v.key_person("agent:sc"),
        s.pid,
        "the card's person is lost after a reload"
    );
}

#[test]
fn board_membership_pins_and_cuts() {
    let mut f = BFold::new();
    let s = f.person("stephen", "pc");
    let b = f.stranger("blake", "bpc");
    let sroot = f.cert(&s, 1, "pc", "pc");
    let created = f.board_created(&s);
    let broot = f.cert(&b, 1, "bpc", "bpc");
    assert!(
        no_cert(f.v.verify_row(broot.as_ref())),
        "blake's self-cert before anyone added him"
    );
    let add = f.member_added(&s, "pc", &b, "member");
    f.feed(&[&sroot, &created, &add]);
    f.v.verify_row(broot.as_ref()).unwrap();
    assert!(f.v.pinned(&b.pid).is_some(), "the add didn't pin blake");
    let blap = f.cert(&b, 1, "bpc", "blap");
    let p1 = f.profile("blap", &b.pid, "");
    let rm = f.member_removed(&s, "pc", &b, &[("bpc", 2)]);
    let carol = f.stranger("carol", "cpc");
    let forged = f.member_added(&s, "bpc", &carol, "owner");
    let rows = [sroot, created, add, broot, blap, p1, rm, forged];
    let anchor = s.pin();
    for idx in shuffles(300, rows.len()) {
        let v = BoardVerifier::new(std::slice::from_ref(&anchor));
        for i in idx {
            v.observe(rows[i].as_ref());
        }
        let m = v.membership();
        assert!(
            m.created && m.owner(&s.pid),
            "stephen isn't the board's owner"
        );
        assert_eq!(m.person(&b.pid).map(|p| p.state.as_str()), Some("removed"));
        assert!(
            m.person(&carol.pid).is_none(),
            "a forged add put carol on the board"
        );
        assert_eq!(v.cut("bpc"), Some(2));
        assert_eq!(
            v.cut("blap"),
            Some(0),
            "blap, which the removal didn't name"
        );
        assert!(v.cut("pc").is_none(), "the remover is cut");
        assert_eq!(v.board_epoch(), 2);
    }
    // The self-cert of a person nobody added never verifies.
    let mut g = BFold::new();
    let gs = g.person("stephen", "pc");
    let (a, c) = (g.cert(&gs, 1, "pc", "pc"), g.board_created(&gs));
    g.feed(&[&a, &c]);
    let other = g.stranger("blake", "bpc");
    let r = g.cert(&other, 1, "bpc", "bpc");
    assert!(no_cert(g.v.verify_row(r.as_ref())));
    // board.* rows bind their acting person to the writer.
    let rn = BoardRenamed {
        name: "x".into(),
        by: other.pid.clone(),
        ..Default::default()
    };
    let r = g.row(
        "pc",
        &format!("person:{}", gs.pid),
        kinds::BOARD_RENAMED,
        &rn,
    );
    assert!(matches!(
        g.v.verify_row(r.as_ref()),
        Err(VerifyError::BoardActor(_))
    ));
}

#[test]
fn board_removal_in_the_store() {
    let mut f = BFold::new();
    let s = f.person("stephen", "pc");
    let b = f.stranger("blake", "bpc");
    let c = f.stranger("carol", "cpc");
    let base = [
        f.cert(&s, 1, "pc", "pc"),
        f.board_created(&s),
        f.member_added(&s, "pc", &b, "owner"),
        f.member_added(&s, "pc", &c, "member"),
        f.cert(&b, 1, "bpc", "bpc"),
        f.cert(&c, 1, "cpc", "cpc"),
    ];
    let s_removes_b = f.member_removed(&s, "pc", &b, &[("bpc", 1)]);
    let b_removes_s = f.member_removed(&b, "bpc", &s, &[("pc", 4)]);
    let b_later = f.profile("bpc", &b.pid, "");
    let c_leaves = f.member_left(&c, "cpc", &[("cpc", 1)]);
    let c_later = f.profile("cpc", &c.pid, "");
    let run = |order: &[&Row]| {
        let (v, st) = board_store(&[s.pin()]);
        for ev in base.iter().chain(order.iter().copied()) {
            st.insert(ev)
                .unwrap_or_else(|e| panic!("{} {}/{}: {e:?}", ev.kind, ev.device_id, ev.seq));
        }
        for ev in [&b_later, &c_later] {
            assert_eq!(
                st.insert(ev),
                Err(StoreErr::PastCut),
                "{}/{} after its person left",
                ev.device_id,
                ev.seq
            );
        }
        let ms = v.membership();
        let got = (roles(&ms), ms.cuts.clone(), v.board_epoch());
        let v2 = Arc::new(BoardVerifier::new(&[s.pin()]));
        st.reopen(v2.clone());
        let ms2 = v2.membership();
        assert_eq!(
            (roles(&ms2), ms2.cuts.clone(), v2.board_epoch()),
            got,
            "after reopen"
        );
        got
    };
    let first = run(&[&s_removes_b, &b_removes_s, &c_leaves]);
    let second = run(&[&b_removes_s, &c_leaves, &s_removes_b]);
    let mut want_roles = [
        format!("{}=owner/removed", s.pid),
        format!("{}=owner/removed", b.pid),
        format!("{}=member/left", c.pid),
    ];
    want_roles.sort();
    let want = (
        want_roles.join(" "),
        [
            ("bpc".to_string(), 1i64),
            ("pc".into(), 4),
            ("cpc".into(), 2),
        ]
        .into_iter()
        .collect::<BTreeMap<_, _>>(),
        4,
    );
    assert_eq!(first, want);
    assert_eq!(second, want);
}

fn roles(m: &Membership) -> String {
    let mut out: Vec<String> = m
        .members
        .iter()
        .map(|p| format!("{}={}/{}", p.person_id, p.role, p.state))
        .collect();
    out.sort();
    out.join(" ")
}

#[test]
fn board_revoked_device_has_no_say_past_its_cut() {
    let mut f = BFold::new();
    let s = f.person("stephen", "pc");
    let (v, st) = board_store(&[s.pin()]);
    let pid = s.pid.clone();
    for ev in [
        f.cert(&s, 1, "pc", "pc"),
        f.board_created(&s),
        f.cert(&s, 1, "pc", "laptop"),
        f.profile("laptop", &pid, ""),
    ] {
        st.insert(&ev)
            .unwrap_or_else(|e| panic!("{} {}/{}: {e:?}", ev.kind, ev.device_id, ev.seq));
    }
    let r = f.revoke(&s, 1, "pc", "laptop", 1);
    st.insert(&r).unwrap();
    // The stolen laptop signs a leave for its person, cutting the pc to 0.
    let left = f.member_left(&s, "laptop", &[("pc", 0), ("laptop", 2)]);
    assert_eq!(st.insert(&left), Err(StoreErr::PastCut));
    let m = v.membership();
    assert!(m.created && m.active(&s.pid));
    // A revoke past its writer's cut is honoured, but never stamped before
    // its writer's last row here.
    let mut back = f.revoke(&s, 1, "laptop", "pc", 0);
    back.hlc = 1;
    back.sign(&f.dev("laptop"));
    assert_eq!(st.insert(&back), Err(StoreErr::HlcOrder));

    // A leave signed before its revoke, cutting the pc to 0, still keeps
    // board.created (pc seq 2).
    let mut f2 = BFold::new();
    let s2 = f2.person("stephen", "pc");
    let (v2, st2) = board_store(&[s2.pin()]);
    for ev in [
        f2.cert(&s2, 1, "pc", "pc"),
        f2.board_created(&s2),
        f2.cert(&s2, 1, "pc", "laptop"),
        f2.member_left(&s2, "laptop", &[("pc", 0)]),
    ] {
        st2.insert(&ev)
            .unwrap_or_else(|e| panic!("{} {}/{}: {e:?}", ev.kind, ev.device_id, ev.seq));
    }
    assert_eq!(v2.cut("pc"), Some(2), "board.created is pc/2");
    assert!(v2.membership().created, "a leave cut board.created away");
}

#[test]
fn board_pin_refuses_a_second_key() {
    let mut f = BFold::new();
    let s = f.person("stephen", "pc");
    assert!(f.v.pin(&s.pin()).is_ok(), "the same pin again");
    let other = MemberPin {
        person_pub: pubh(&rand_key()),
        ..s.pin()
    };
    assert!(f.v.pin(&other).is_err());
    assert!(
        f.v.pin(&MemberPin {
            person_epoch: 0,
            ..s.pin()
        })
        .is_err()
    );
}
