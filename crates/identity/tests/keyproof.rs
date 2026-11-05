//! Key proofs for person-to-person DMs (card #538, talk-dms.md §2).

mod support;

use irori_identity::*;
use support::*;

/// Rotate `n`'s person key and write the row.
fn rotate(n: &Node) -> KeyRotated {
    let rot = n.id.rotate(now()).unwrap();
    n.st.append("irori", kinds::PERSON_KEY_ROTATED, &rot);
    rot
}

/// The proof of `n`'s current key, as a DM hand-off builds it.
fn own_proof(n: &Node) -> Option<KeyProof> {
    let pin = n.id.board_pin();
    n.chain.key_proof(pin.person_epoch, &pin.person_pub)
}

/// A person at epoch 3, with the proof of that key.
fn chain_of_three() -> (tempfile::TempDir, Node, KeyProof) {
    let dir = tempfile::tempdir().unwrap();
    let n = Node::new(dir.path(), None);
    rotate(&n);
    rotate(&n);
    let proof = own_proof(&n).unwrap();
    (dir, n, proof)
}

#[test]
fn a_new_person_proves_its_first_key() {
    let dir = tempfile::tempdir().unwrap();
    let n = Node::new(dir.path(), None);
    let p = n.id.current();
    let proof = own_proof(&n).unwrap();
    assert_eq!(proof.first_pub, p.pub_hex);
    assert!(proof.rotations.is_empty());
    assert_eq!(verify_key_proof(&p.id, &proof), Some((1, p.pub_hex)));
}

#[test]
fn a_good_chain_proves_the_pin() {
    let (_dir, n, proof) = chain_of_three();
    let pin = n.id.board_pin();
    assert_eq!(pin.person_epoch, 3);
    assert_eq!(proof.rotations.len(), 2);
    assert_eq!(
        verify_key_proof(&pin.person_id, &proof),
        Some((pin.person_epoch, pin.person_pub))
    );
}

#[test]
fn a_proof_survives_the_wire() {
    let (_dir, n, proof) = chain_of_three();
    let raw = to_json(&proof);
    let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(v["firstPub"], proof.first_pub.as_str());
    assert_eq!(v["rotations"].as_array().unwrap().len(), 2);
    let back: KeyProof = decode(&raw).unwrap();
    assert_eq!(back, proof);
    let cur = n.id.current();
    assert_eq!(verify_key_proof(&cur.id, &back), Some((3, cur.pub_hex)));
}

#[test]
fn a_restart_still_proves() {
    let (_dir, mut n, proof) = chain_of_three();
    n.restart();
    assert_eq!(own_proof(&n), Some(proof));
}

#[test]
fn a_device_that_dropped_the_first_key_still_proves() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = Node::new(da.path(), None);
    let b = Node::new(db.path(), Some(&a));
    a.certify(&b);
    rotate(&a);
    rotate(&a);
    copy_rows(&a, &b).unwrap();
    let cur = a.id.current();
    let seed = a.id.person_seed(cur.epoch).unwrap();
    b.id.adopt_secret(cur.epoch, &seed[..], &cur.pub_hex)
        .unwrap();
    b.id.forget(cur.epoch).unwrap();
    assert!(b.id.person_seed(1).is_none());
    assert_eq!(own_proof(&b), own_proof(&a));
    let proof = own_proof(&b).unwrap();
    assert_eq!(verify_key_proof(&cur.id, &proof), Some((3, cur.pub_hex)));
}

#[test]
fn a_rotation_by_another_device_is_on_the_chain() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = Node::new(da.path(), None);
    let b = Node::new(db.path(), Some(&a));
    a.certify(&b);
    copy_rows(&a, &b).unwrap();
    let rot = rotate(&b);
    copy_rows(&b, &a).unwrap();
    let seed = b.id.person_seed(2).unwrap();
    a.id.adopt_secret(2, &seed[..], &rot.person_pub).unwrap();
    rotate(&a);
    let proof = own_proof(&a).unwrap();
    assert_eq!(proof.rotations[0].by, b.dev());
    assert_eq!(proof.rotations[1].by, a.dev());
    let cur = a.id.current();
    assert_eq!(verify_key_proof(&cur.id, &proof), Some((3, cur.pub_hex)));
}

#[test]
fn two_rotations_to_one_epoch_prove_through_the_key_that_won() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = Node::new(da.path(), None);
    let b = Node::new(db.path(), Some(&a));
    a.certify(&b);
    copy_rows(&a, &b).unwrap();
    let won = rotate(&a);
    let lost = rotate(&b);
    copy_rows(&a, &b).unwrap();
    copy_rows(&b, &a).unwrap();
    let seed = a.id.person_seed(2).unwrap();
    b.id.replace_secret(2, &seed[..], &won.person_pub).unwrap();
    rotate(&b);
    copy_rows(&b, &a).unwrap();
    let cur = b.id.current();
    let proof = own_proof(&b).unwrap();
    assert_eq!(proof.rotations[0], member_rotation(&won).0);
    assert_eq!(
        verify_key_proof(&cur.id, &proof),
        Some((3, cur.pub_hex.clone()))
    );
    // The key that lost is still provable at its epoch, and asked for it
    // gets its own path.
    let side = a.chain.key_proof(2, &lost.person_pub).unwrap();
    assert_eq!(side.rotations, vec![member_rotation(&lost).0]);
    assert_eq!(a.chain.key_proof(3, &cur.pub_hex), Some(proof));
}

#[test]
fn a_key_off_the_chain_has_no_proof() {
    let (_dir, n, _) = chain_of_three();
    let cur = n.id.current();
    assert!(n.chain.key_proof(3, &pubh(&rand_key())).is_none());
    assert!(
        n.chain.key_proof(4, &cur.pub_hex).is_none(),
        "a future epoch"
    );
    assert!(
        n.chain.key_proof(2, &cur.pub_hex).is_none(),
        "the wrong epoch"
    );
    assert!(n.chain.key_proof(0, &cur.pub_hex).is_none());
}

#[test]
fn a_short_chain_proves_only_the_older_key() {
    let (_dir, n, mut proof) = chain_of_three();
    let cur = n.id.current();
    let second = proof.rotations[0].person_pub.clone();
    proof.rotations.pop();
    assert_eq!(verify_key_proof(&cur.id, &proof), Some((2, second)));
}

#[test]
fn a_bad_link_proves_nothing() {
    let (_dir, n, good) = chain_of_three();
    let id = n.id.current().id;
    let thief = rand_key();

    // The new key swapped under a signature that doesn't cover it.
    let mut p = good.clone();
    p.rotations[1].person_pub = pubh(&thief);
    assert_eq!(verify_key_proof(&id, &p), None, "swapped key");

    // Signed by a key that isn't the epoch before's.
    let mut p = good.clone();
    p.rotations[1].person_pub = pubh(&thief);
    p.rotations[1].prev_sig = sign_hex(&thief, &p.rotations[1].body());
    assert_eq!(verify_key_proof(&id, &p), None, "wrong signer");

    // An epoch skipped: the signature is good, the walk isn't.
    let mut p = good.clone();
    p.rotations.remove(0);
    assert_eq!(verify_key_proof(&id, &p), None, "skipped epoch");

    // Out of order.
    let mut p = good.clone();
    p.rotations.reverse();
    assert_eq!(verify_key_proof(&id, &p), None, "out of order");

    // A link repeated.
    let mut p = good.clone();
    p.rotations.push(p.rotations[1].clone());
    assert_eq!(verify_key_proof(&id, &p), None, "repeated link");

    // Anything the signature covers, changed.
    let mut p = good.clone();
    p.rotations[0].by = "another".into();
    assert_eq!(verify_key_proof(&id, &p), None, "changed by");
    let mut p = good.clone();
    p.rotations[0].at = unix(1);
    assert_eq!(verify_key_proof(&id, &p), None, "changed at");

    // A signature that isn't one.
    let mut p = good.clone();
    p.rotations[0].prev_sig = "zz".into();
    assert_eq!(verify_key_proof(&id, &p), None, "bad hex");
}

#[test]
fn a_link_for_another_person_proves_nothing() {
    // The first key signs a rotation naming someone else: the signature is
    // good, but it isn't this person's rotation.
    let first = rand_key();
    let id = person_id_of(&first.verifying_key().to_bytes());
    let mut r = MemberKeyRotated {
        person_id: "someone-else".into(),
        person_epoch: 2,
        person_pub: pubh(&rand_key()),
        by: "dev".into(),
        at: unix(1),
        prev_sig: String::new(),
    };
    r.prev_sig = sign_hex(&first, &r.body());
    let mut p = KeyProof {
        first_pub: pubh(&first),
        rotations: vec![r],
    };
    assert_eq!(verify_key_proof(&id, &p), None);
    p.rotations[0].person_id = id.clone();
    p.rotations[0].prev_sig = sign_hex(&first, &p.rotations[0].body());
    let want = p.rotations[0].person_pub.clone();
    assert_eq!(verify_key_proof(&id, &p), Some((2, want)));
}

#[test]
fn a_link_with_no_device_proves_nothing() {
    let first = rand_key();
    let id = person_id_of(&first.verifying_key().to_bytes());
    let mut r = MemberKeyRotated {
        person_id: id.clone(),
        person_epoch: 2,
        person_pub: pubh(&rand_key()),
        by: String::new(),
        at: unix(1),
        prev_sig: String::new(),
    };
    r.prev_sig = sign_hex(&first, &r.body());
    let p = KeyProof {
        first_pub: pubh(&first),
        rotations: vec![r],
    };
    assert_eq!(verify_key_proof(&id, &p), None);
}

#[test]
fn the_wrong_first_key_proves_nothing() {
    let (_dir, n, good) = chain_of_three();
    let id = n.id.current().id;

    // A real chain of another person, under this person's id.
    let (_other_dir, other, theirs) = chain_of_three();
    assert_ne!(other.id.current().id, id);
    assert_eq!(verify_key_proof(&id, &theirs), None, "another person's");

    // A later key of this person is not the one the id derives from.
    let mut p = good.clone();
    p.first_pub = p.rotations[0].person_pub.clone();
    p.rotations.remove(0);
    assert_eq!(verify_key_proof(&id, &p), None, "a later key");

    // A stranger's key, with no rotations to get in the way.
    let p = KeyProof {
        first_pub: pubh(&rand_key()),
        rotations: Vec::new(),
    };
    assert_eq!(verify_key_proof(&id, &p), None, "a stranger's key");

    let p = KeyProof::default();
    assert_eq!(verify_key_proof(&id, &p), None, "no key");
}

/// `rot` as a build from before v0.5 wrote it.
fn pre_v05(rot: &KeyRotated) -> KeyRotated {
    KeyRotated {
        by: String::new(),
        prev_sig: String::new(),
        ..rot.clone()
    }
}

#[test]
fn a_rotation_from_before_v05_has_no_proof() {
    let dir = tempfile::tempdir().unwrap();
    let n = Node::new(dir.path(), None);
    let rot = n.id.rotate(now()).unwrap();
    n.st.append("irori", kinds::PERSON_KEY_ROTATED, &pre_v05(&rot));
    assert!(own_proof(&n).is_none());

    // A good rotation after it doesn't mend the chain.
    rotate(&n);
    assert_eq!(n.id.current().epoch, 3);
    assert!(own_proof(&n).is_none());
}

#[test]
fn a_pre_v05_link_fails_under_the_real_first_key() {
    let dir = tempfile::tempdir().unwrap();
    let n = Node::new(dir.path(), None);
    let first = n.id.current();
    let rot = n.id.rotate(now()).unwrap();
    let p = KeyProof {
        first_pub: first.pub_hex,
        rotations: vec![member_rotation(&pre_v05(&rot)).0],
    };
    assert_eq!(verify_key_proof(&first.id, &p), None);
}
