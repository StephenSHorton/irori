//! The invite room the code derives, what the proofs and words cover, and
//! the person fingerprint (v0.5-shared-boards §4.1 steps 1, 4, 5;
//! architecture §3.3).
//!
//! It is pairing's construction (identity.md §4.7, `irori_pair::code`)
//! under the invite's own names, so no proof or word list from one flow
//! is valid in the other:
//!
//! - room: scrypt over the code with app [`APP`] (`irori-invite-v1`);
//! - info: pairing's (both device keys, both channel bindings, both device
//!   ids, inviter first) followed by both person keys and the board id;
//! - proof: HMAC under the room key over [`PROOF_LABEL`], the side and the
//!   info;
//! - person proof: each person key signs the canonical JSON of
//!   `{label, side, info}` ([`PersonProof`]), so the invitee knows the key
//!   that matches the link's fingerprint is on the other end, and the
//!   inviter learns which person key to pin;
//! - words: HKDF under salt [`SAS_SALT`] (`irori/invite/1`), five PGP words.

use hmac::{Hmac, KeyInit, Mac};
use irori_meshseam::Secret;
use serde::Serialize;
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// The invite room's app name (design §4.1 step 1).
pub const APP: &str = "irori-invite-v1";
/// The words' HKDF salt.
pub const SAS_SALT: &str = "irori/invite/1";
/// The code proof's label.
pub const PROOF_LABEL: &[u8] = b"irori/invite-proof/1";
/// The person proof's label.
pub const PERSON_LABEL: &str = "irori/invite-person/1";

/// The inviter's side.
pub const SIDE_A: u8 = b'A';
/// The invitee's side.
pub const SIDE_B: u8 = b'B';

/// The invite room for a code (scrypt, as pairing's, under [`APP`]).
pub fn room_secret(code: &str) -> Result<Secret, &'static str> {
    irori_pair::room_secret_for(APP, code)
}

/// One side of the invite, as its hello names it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Side {
    pub device_id: String,
    /// The device key, 32 bytes.
    pub device_pub: Vec<u8>,
    /// The person key, 32 bytes.
    pub person_pub: Vec<u8>,
}

/// What the proofs, the person proofs and the words cover: pairing's info
/// with the session binding (the connection's `fingerprints()`, the TLS
/// exporter, colon-hex) on both sides, then both person keys and the
/// board id, each `u32 BE` length-prefixed, inviter (A) first.
pub fn info(a: &Side, b: &Side, binding: &str, board_id: &str) -> Vec<u8> {
    let mut out = irori_pair::info(
        &a.device_pub,
        &b.device_pub,
        binding,
        binding,
        &a.device_id,
        &b.device_id,
    );
    for f in [
        a.person_pub.as_slice(),
        b.person_pub.as_slice(),
        board_id.as_bytes(),
    ] {
        out.extend_from_slice(&(f.len() as u32).to_be_bytes());
        out.extend_from_slice(f);
    }
    out
}

/// The five words both screens show.
pub fn words(room_key: &[u8; 32], info: &[u8]) -> Vec<String> {
    irori_pair::sas_with(SAS_SALT, room_key, info)
}

/// The code proof of `side`.
pub fn proof(room_key: &[u8; 32], side: u8, info: &[u8]) -> String {
    let mut m = <Hmac<Sha256> as KeyInit>::new_from_slice(room_key).expect("any key length");
    m.update(PROOF_LABEL);
    m.update(&[side]);
    m.update(info);
    hex::encode(m.finalize().into_bytes())
}

/// Checks `side`'s code proof in constant time.
pub fn proof_ok(room_key: &[u8; 32], side: u8, info: &[u8], got: &str) -> bool {
    let want = proof(room_key, side, info);
    want.len() == got.len() && bool::from(want.as_bytes().ct_eq(got.as_bytes()))
}

/// What a person key signs to prove it is on this invite.
#[derive(Serialize)]
pub struct PersonProof {
    pub label: &'static str,
    pub side: String,
    /// The info, hex.
    pub info: String,
}

impl PersonProof {
    pub fn new(side: u8, info: &[u8]) -> PersonProof {
        PersonProof {
            label: PERSON_LABEL,
            side: char::from(side).to_string(),
            info: hex::encode(info),
        }
    }
}

/// Checks `side`'s person proof against the person key it named.
pub fn person_proof_ok(person_pub: &[u8], side: u8, info: &[u8], sig_hex: &str) -> bool {
    irori_identity::verify_person(
        &hex::encode(person_pub),
        sig_hex,
        &PersonProof::new(side, info),
    )
    .is_ok()
}

/// A person's fingerprint as the screens and the link show it: pairing's
/// device fingerprint over the person public key.
pub fn person_fingerprint(person_pub: &[u8]) -> String {
    irori_pair::fingerprint(person_pub)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side(n: u8) -> Side {
        Side {
            device_id: format!("dev-{n}"),
            device_pub: vec![n; 32],
            person_pub: vec![n + 100; 32],
        }
    }

    #[test]
    fn the_invite_room_is_not_the_pairing_room() {
        let code = "ABCD-EFGH-JKMN-PQRS";
        let inv = room_secret(code).unwrap();
        let pair = irori_pair::room_secret(code).unwrap();
        assert_ne!(inv.key, pair.key);
        assert!(inv.topic.starts_with("irori-invite-v1/"), "{}", inv.topic);
        assert_eq!(inv.epoch, 0);
        assert_eq!(room_secret("abcd efgh jkmn pqrs").unwrap().key, inv.key);
        assert!(room_secret("nope").is_err());
    }

    #[test]
    fn info_binds_both_people_and_the_board() {
        let (a, b) = (side(1), side(2));
        let bind = "09:09";
        let base = info(&a, &b, bind, "board-1");
        let other_person = Side {
            person_pub: vec![42; 32],
            ..b.clone()
        };
        assert_ne!(base, info(&a, &other_person, bind, "board-1"));
        assert_ne!(base, info(&a, &b, bind, "board-2"));
        assert_ne!(base, info(&a, &b, "08:08", "board-1"));
        assert_ne!(base, info(&b, &a, bind, "board-1"));
    }

    #[test]
    fn words_and_proofs_differ_from_pairings() {
        let key = [5u8; 32];
        let inf = info(&side(1), &side(2), "09:09", "b");
        let w = words(&key, &inf);
        assert_eq!(w.len(), 5);
        assert_ne!(w, irori_pair::sas(&key, &inf));
        let p = proof(&key, SIDE_A, &inf);
        assert!(proof_ok(&key, SIDE_A, &inf, &p));
        assert!(!proof_ok(&key, SIDE_B, &inf, &p), "reflected");
        assert!(!proof_ok(&[6u8; 32], SIDE_A, &inf, &p), "another code");
        assert_ne!(
            hex::decode(&p).unwrap(),
            irori_pair::proof(&key, SIDE_A, &inf),
            "a pairing proof is not an invite proof"
        );
    }
}
