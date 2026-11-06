//! Proof that a key belongs to a personId (card #538, talk-dms.md §2).
//!
//! A board pin doesn't bind `personId` to a key: it is only what some
//! owner wrote. But a personId is a hash of the person's first key, and
//! each rotation since v0.5 is signed by the key before it. So the first
//! key and the rotations from it prove a key to anyone, with no log and no
//! trust in who sent them.
//!
//! A proof shows a key was this person's at its epoch, not that it is
//! still the newest: a chain cut short proves an older key. The caller
//! compares the result with what it was told, and the cert it holds must
//! verify under that key.

use std::collections::BTreeMap;

use gocompat::json::{Out, Schema};
use serde::Serialize;

use crate::objects::{
    GoDecode, KeyRotated, MemberKeyRotated, first_key_of, member_rotation, valid_pub, verify_person,
};

/// The epoch a personId's first key has.
const FIRST_EPOCH: i64 = 1;

/// A person's first key and each rotation from it, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyProof {
    pub first_pub: String,
    pub rotations: Vec<MemberKeyRotated>,
}

impl GoDecode for KeyProof {
    fn schema() -> Schema {
        Schema::object(vec![
            ("firstPub", Schema::String),
            (
                "rotations",
                Schema::Slice(Box::new(MemberKeyRotated::schema())),
            ),
        ])
    }

    fn from_out(o: &Out) -> Self {
        KeyProof {
            first_pub: o.get_str("firstPub").to_string(),
            rotations: o
                .get("rotations")
                .map(|s| s.items().iter().map(MemberKeyRotated::from_out).collect())
                .unwrap_or_default(),
        }
    }
}

/// Whether `r` takes `person_id` from `prev_pub` to `epoch`.
fn links(person_id: &str, prev_pub: &str, epoch: i64, r: &MemberKeyRotated) -> bool {
    r.person_id == person_id
        && r.person_epoch == epoch
        && !r.by.is_empty()
        && valid_pub(&r.person_pub)
        && verify_person(prev_pub, &r.prev_sig, &r.body()).is_ok()
}

/// The epoch and key `proof` proves for `person_id`. `None` when `firstPub`
/// isn't the key the id derives from, or any rotation doesn't follow the
/// one before it by exactly one epoch under that epoch's key. A rotation
/// from before v0.5 has no prevSig, so it never follows.
pub fn verify_key_proof(person_id: &str, proof: &KeyProof) -> Option<(i64, String)> {
    if !first_key_of(person_id, &proof.first_pub) {
        return None;
    }
    let (mut epoch, mut key) = (FIRST_EPOCH, proof.first_pub.as_str());
    for r in &proof.rotations {
        if !links(person_id, key, epoch + 1, r) {
            return None;
        }
        epoch = r.person_epoch;
        key = &r.person_pub;
    }
    Some((epoch, key.to_string()))
}

/// The proof of `pub_hex` at `epoch` from a person's private rotations, in
/// any order. Two devices can rotate to one epoch and one key loses, and
/// both rows stay in the log, so this walks every key each epoch can reach
/// and keeps the path that ends at `pub_hex`. `None` when none does, which
/// a rotation from before v0.5 on the way makes certain.
pub(crate) fn prove_key(
    person_id: &str,
    first_pub: &str,
    rotations: &[KeyRotated],
    epoch: i64,
    pub_hex: &str,
) -> Option<KeyProof> {
    if epoch < FIRST_EPOCH || !first_key_of(person_id, first_pub) {
        return None;
    }
    let mut reached: BTreeMap<String, Vec<MemberKeyRotated>> =
        BTreeMap::from([(first_pub.to_string(), Vec::new())]);
    for next in FIRST_EPOCH + 1..=epoch {
        let mut after: BTreeMap<String, Vec<MemberKeyRotated>> = BTreeMap::new();
        for r in rotations.iter().filter(|r| r.epoch == next) {
            let (m, ok) = member_rotation(r);
            if !ok || after.contains_key(&m.person_pub) {
                continue;
            }
            let from = reached
                .iter()
                .find(|(key, _)| links(person_id, key, next, &m));
            if let Some((_, path)) = from {
                let mut path = path.clone();
                path.push(m.clone());
                after.insert(m.person_pub, path);
            }
        }
        if after.is_empty() {
            return None;
        }
        reached = after;
    }
    reached.remove(pub_hex).map(|rotations| KeyProof {
        first_pub: first_pub.to_string(),
        rotations,
    })
}
