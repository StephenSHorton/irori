//! The private cert fold (I:internal/identity/chain.go; identity.md §4.3):
//! which devices belong to this device's person, and the revocation cuts.
//! It is the private log's store Verifier.
//!
//! The rules, all independent of arrival order:
//! - the root is pinned: a self-written cert is valid only for the
//!   person's `rootDeviceId`;
//! - every other cert counts only when its writer holds a valid cert; a
//!   device can't certify itself;
//! - a revoke is honoured when its writer was ever certified (cuts
//!   ignored); a device's cut is the lowest honoured cut, each raised to the
//!   rows its writer's own cert chain is made of;
//! - a cert written above its writer's cut doesn't count;
//! - only this person's rows count; another person's are stored and inert;
//! - a person signature counts only by the first key (the one personId
//!   derives from) or a key a rotation row from an ever-certified device
//!   announces.

use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;

use crate::kinds;
use crate::objects::{
    Cert, DeviceAdded, DeviceRevoked, KeyRotated, Profile, Revoke, decode, ed25519_verify,
    first_key_of, valid_pub, verify_person,
};
use crate::row::{CutClaim, RowPos, RowRef, Verifier, VerifyError, signing_bytes};

#[derive(Clone, Debug)]
struct CertRow {
    pos: RowPos,
    cert: Cert,
    /// The person key that signed it.
    pub_hex: String,
}

#[derive(Clone, Debug)]
struct RevokeRow {
    pos: RowPos,
    revoke: Revoke,
    pub_hex: String,
}

#[derive(Clone, Debug)]
struct RotationRow {
    pos: RowPos,
    /// The key it announces.
    pub_hex: String,
}

#[derive(Clone, Debug)]
struct ProfileRow {
    pos: RowPos,
    pub_hex: String,
    root_id: String,
}

/// One certified device (GUI JSON `{cert, revoked, cut?}`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct DeviceState {
    pub cert: Cert,
    pub revoked: bool,
    #[serde(skip_serializing_if = "is_zero")]
    pub cut: i64,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

#[derive(Default)]
struct State {
    person_id: String,
    root_id: String,
    adds: Vec<CertRow>,
    revokes: Vec<RevokeRow>,
    rotations: Vec<RotationRow>,
    profiles: Vec<ProfileRow>,
    profile: bool,
    bad: i64,
    seen: HashSet<(String, i64)>,
    valid: BTreeMap<String, Cert>,
    cuts: BTreeMap<String, i64>,
    claims: Vec<CutClaim>,
}

/// The cert fold for one person.
pub struct Chain {
    self_id: String,
    st: Mutex<State>,
}

/// The chain rule both folds share: rows (sorted) certify the device
/// `link` names; they count when the root wrote its own self-cert or their
/// writer holds a counted cert; a device can't certify itself; with cuts, a
/// row above its writer's cut doesn't count (Go `certifyRows`).
pub(crate) fn certify_rows<R: Clone>(
    root: &str,
    rows: &[R],
    link: impl Fn(&R) -> (&RowPos, &str),
    cuts: Option<&BTreeMap<String, i64>>,
) -> BTreeMap<String, R> {
    let within = |p: &RowPos| match cuts.and_then(|c| c.get(&p.writer)) {
        Some(cut) => p.seq <= *cut,
        None => true,
    };
    let mut valid: BTreeMap<String, R> = BTreeMap::new();
    for a in rows {
        let (pos, dev) = link(a);
        if dev == root && pos.writer == root && within(pos) {
            valid.insert(root.to_string(), a.clone());
            break;
        }
    }
    if valid.is_empty() {
        return valid;
    }
    let mut changed = true;
    while changed {
        changed = false;
        for a in rows {
            let (pos, dev) = link(a);
            if valid.contains_key(dev) || dev == pos.writer {
                continue;
            }
            if !valid.contains_key(&pos.writer) || !within(pos) {
                continue;
            }
            valid.insert(dev.to_string(), a.clone());
            changed = true;
        }
    }
    valid
}

/// The highest seq of `target`'s rows on `writer`'s cert chain in `ever`
/// (the cert rows from writer back to the root), walking at most
/// `len(ever)` steps (Go `floorOf`).
pub(crate) fn floor_of<R>(
    root: &str,
    ever: &BTreeMap<String, R>,
    pos_of: impl Fn(&R) -> &RowPos,
    writer: &str,
    target: &str,
) -> Option<i64> {
    let mut f = 0i64;
    let mut found = false;
    let mut cur = writer.to_string();
    for _ in 0..ever.len() {
        let Some(row) = ever.get(&cur) else {
            break;
        };
        let pos = pos_of(row);
        if pos.writer == target && (!found || pos.seq > f) {
            f = pos.seq;
            found = true;
        }
        if cur == root {
            break;
        }
        cur = pos.writer.clone();
    }
    found.then_some(f)
}

impl State {
    fn certify(
        &self,
        adds: &[CertRow],
        cuts: Option<&BTreeMap<String, i64>>,
    ) -> BTreeMap<String, CertRow> {
        certify_rows(
            &self.root_id,
            adds,
            |a| (&a.pos, a.cert.device_id.as_str()),
            cuts,
        )
    }

    fn refold(&mut self) {
        let mut adds = self.adds.clone();
        adds.sort_by(|a, b| a.pos.cmp(&b.pos));
        let mut revs = self.revokes.clone();
        revs.sort_by(|a, b| a.pos.cmp(&b.pos));

        let person_id = self.person_id.clone();
        let mut keys: HashSet<String> = HashSet::new();
        let trusted =
            |keys: &HashSet<String>, p: &str| keys.contains(p) || first_key_of(&person_id, p);
        let mut signed: Vec<CertRow>;
        let mut ever;
        loop {
            signed = adds
                .iter()
                .filter(|a| trusted(&keys, &a.pub_hex))
                .cloned()
                .collect();
            ever = self.certify(&signed, None);
            let mut grew = false;
            for r in &self.rotations {
                if ever.contains_key(&r.pos.writer) && !keys.contains(&r.pub_hex) {
                    keys.insert(r.pub_hex.clone());
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        let mut cuts: BTreeMap<String, i64> = BTreeMap::new();
        let mut claims = Vec::new();
        for r in &revs {
            if !ever.contains_key(&r.pos.writer) || !trusted(&keys, &r.pub_hex) {
                continue;
            }
            let dev = &r.revoke.device_id;
            let mut cut = r.revoke.cut;
            if let Some(f) = floor_of(&self.root_id, &ever, |a| &a.pos, &r.pos.writer, dev)
                && f > cut
            {
                cut = f;
            }
            match cuts.get(dev) {
                Some(cur) if *cur <= cut => {}
                _ => {
                    cuts.insert(dev.clone(), cut);
                }
            }
            if !r.revoke.cut_hash.is_empty() && r.revoke.cut > 0 {
                claims.push(CutClaim {
                    device_id: dev.clone(),
                    seq: r.revoke.cut,
                    hash: r.revoke.cut_hash.clone(),
                });
            }
        }
        let valid = self
            .certify(&signed, Some(&cuts))
            .into_iter()
            .map(|(d, a)| (d, a.cert))
            .collect();
        let profile = self.profiles.iter().any(|p| {
            ever.contains_key(&p.pos.writer)
                && trusted(&keys, &p.pub_hex)
                && p.root_id == self.root_id
        });
        self.valid = valid;
        self.cuts = cuts;
        self.claims = claims;
        self.profile = profile;
    }

    fn reset(&mut self) {
        let (p, r) = (
            std::mem::take(&mut self.person_id),
            std::mem::take(&mut self.root_id),
        );
        *self = State {
            person_id: p,
            root_id: r,
            ..State::default()
        };
    }
}

impl Chain {
    /// Go `NewChain(personID, rootDeviceID, selfID)`.
    pub fn new(person_id: &str, root_device_id: &str, self_id: &str) -> Chain {
        Chain {
            self_id: self_id.to_string(),
            st: Mutex::new(State {
                person_id: person_id.to_string(),
                root_id: root_device_id.to_string(),
                ..State::default()
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Point the fold at another person and root (pairing) and forget every
    /// row; the caller has the store feed its rows again.
    pub fn repin(&self, person_id: &str, root_device_id: &str) {
        let mut st = self.lock();
        st.person_id = person_id.to_string();
        st.root_id = root_device_id.to_string();
        st.reset();
    }

    pub fn person_id(&self) -> String {
        self.lock().person_id.clone()
    }

    /// Fold one person.* row (store Verifier `Observe`).
    pub fn observe(&self, ev: RowRef<'_>) {
        let mut st = self.lock();
        if !st.seen.insert((ev.device_id.to_string(), ev.seq)) {
            return;
        }
        let pos = RowPos::of(ev);
        match ev.kind {
            kinds::PERSON_DEVICE_ADDED => {
                let Some(p) = decode::<DeviceAdded>(ev.payload) else {
                    st.bad += 1;
                    return;
                };
                if p.cert.person_id != st.person_id {
                    return; // another person's: stored, inert
                }
                if verify_person(&p.person_pub, &p.person_sig, &p.cert).is_err()
                    || !valid_pub(&p.cert.device_pub)
                    || p.cert.device_id.is_empty()
                {
                    st.bad += 1;
                    return;
                }
                st.adds.push(CertRow {
                    pos,
                    cert: p.cert,
                    pub_hex: p.person_pub,
                });
            }
            kinds::PERSON_DEVICE_REVOKED => {
                let Some(p) = decode::<DeviceRevoked>(ev.payload) else {
                    st.bad += 1;
                    return;
                };
                if p.revoke.person_id != st.person_id {
                    return;
                }
                if verify_person(&p.person_pub, &p.person_sig, &p.revoke).is_err()
                    || p.revoke.device_id.is_empty()
                    || p.revoke.cut < 0
                {
                    st.bad += 1;
                    return;
                }
                st.revokes.push(RevokeRow {
                    pos,
                    revoke: p.revoke,
                    pub_hex: p.person_pub,
                });
            }
            kinds::PERSON_KEY_ROTATED => {
                let Some(p) = decode::<KeyRotated>(ev.payload) else {
                    st.bad += 1;
                    return;
                };
                if p.person_id != st.person_id {
                    return;
                }
                if !valid_pub(&p.person_pub) {
                    st.bad += 1;
                    return;
                }
                st.rotations.push(RotationRow {
                    pos,
                    pub_hex: p.person_pub,
                });
            }
            kinds::PERSON_PROFILE => {
                let Some(p) = decode::<Profile>(ev.payload) else {
                    st.bad += 1;
                    return;
                };
                if p.person_id != st.person_id {
                    return;
                }
                if verify_person(&p.person_pub, &p.person_sig, &p.body()).is_err() {
                    st.bad += 1;
                    return;
                }
                st.profiles.push(ProfileRow {
                    pos,
                    pub_hex: p.person_pub,
                    root_id: p.root_device_id,
                });
            }
            _ => return,
        }
        st.refold();
    }

    /// Check a row's device signature against its device's cert. Before the
    /// root's cert is held, a root self-cert signed by the person's first
    /// key verifies from the key it carries.
    pub fn verify_row(&self, ev: RowRef<'_>) -> Result<(), VerifyError> {
        let (cert, person_id, root_id) = {
            let st = self.lock();
            (
                st.valid.get(ev.device_id).cloned(),
                st.person_id.clone(),
                st.root_id.clone(),
            )
        };
        let pub_hex = match cert {
            Some(c) => c.device_pub,
            None => {
                let no_cert = || VerifyError::NoCert(ev.device_id.to_string());
                if ev.kind != kinds::PERSON_DEVICE_ADDED || ev.device_id != root_id {
                    return Err(no_cert());
                }
                let Some(p) = decode::<DeviceAdded>(ev.payload) else {
                    return Err(no_cert());
                };
                if p.cert.device_id != ev.device_id
                    || p.cert.person_id != person_id
                    || !first_key_of(&person_id, &p.person_pub)
                    || verify_person(&p.person_pub, &p.person_sig, &p.cert).is_err()
                {
                    return Err(no_cert());
                }
                p.cert.device_pub
            }
        };
        let pk = match hex::decode(&pub_hex) {
            Ok(b) if b.len() == 32 => b,
            _ => {
                return Err(VerifyError::BadSig(format!(
                    "{} has a bad key",
                    ev.device_id
                )));
            }
        };
        if !ed25519_verify(&pk, &signing_bytes(ev), ev.sig) {
            return Err(VerifyError::BadSig(format!("{}/{}", ev.device_id, ev.seq)));
        }
        Ok(())
    }

    /// Fold a person.* row that is not stored yet (pairing, a sync hello):
    /// it must verify first.
    pub fn learn(&self, ev: RowRef<'_>) -> Result<(), LearnError> {
        if !kinds::is_person_kind(ev.kind) {
            return Err(LearnError::NotPerson(ev.kind.to_string()));
        }
        self.verify_row(ev).map_err(LearnError::Verify)?;
        self.observe(ev);
        Ok(())
    }

    /// A device's effective revocation cut.
    pub fn cut(&self, device_id: &str) -> Option<i64> {
        self.lock().cuts.get(device_id).copied()
    }

    /// The row each honoured revoke names by cutHash.
    pub fn cut_claims(&self) -> Vec<CutClaim> {
        self.lock().claims.clone()
    }

    /// A device's valid cert.
    pub fn cert(&self, device_id: &str) -> Option<Cert> {
        self.lock().valid.get(device_id).cloned()
    }

    /// Go `mesh.CheckDevice` (I:internal/mesh/node.go:306-322), the one rule
    /// for who gets in, on a connection, on the home server's relay and in a
    /// pairing: `id` is a device other than `self_id` with a valid, unrevoked
    /// cert for this person whose cert names `pubkey`. The mesh and net
    /// crates wrap it in their refusal errors.
    pub fn check_device(&self, self_id: &str, id: &str, pubkey: &[u8; 32]) -> bool {
        if id == self_id || id.is_empty() {
            return false;
        }
        match self.cert(id) {
            Some(cert) if hex::encode(pubkey) == cert.device_pub => {}
            _ => return false,
        }
        self.cut(id).is_none()
    }

    /// Whether this person's person.profile is held.
    pub fn has_profile(&self) -> bool {
        self.lock().profile
    }

    /// Person rows for this person that failed their checks.
    pub fn bad(&self) -> i64 {
        self.lock().bad
    }

    /// Every device ever certified to the person, by device id.
    pub fn devices(&self) -> Vec<DeviceState> {
        let st = self.lock();
        st.valid
            .iter()
            .map(|(id, cert)| {
                let cut = st.cuts.get(id);
                DeviceState {
                    cert: cert.clone(),
                    revoked: cut.is_some(),
                    cut: cut.copied().unwrap_or(0),
                }
            })
            .collect()
    }

    /// Forget every observed row (store Verifier `Reset`).
    pub fn reset(&self) {
        self.lock().reset();
    }

    /// Whether `key` is the owner's: exactly `user:<dev>` for this device or
    /// a device ever certified to the person, revoked ones included.
    pub fn is_owner(&self, key: &str) -> bool {
        let Some(dev) = key.strip_prefix(USER_KEY_PREFIX) else {
            return false;
        };
        if dev.is_empty() {
            return false;
        }
        if dev == self.self_id {
            return true;
        }
        self.lock().valid.contains_key(dev)
    }
}

impl Verifier for Chain {
    fn observe(&self, ev: RowRef<'_>) {
        Chain::observe(self, ev)
    }
    fn verify_row(&self, ev: RowRef<'_>) -> Result<(), VerifyError> {
        Chain::verify_row(self, ev)
    }
    fn cut(&self, device_id: &str) -> Option<i64> {
        Chain::cut(self, device_id)
    }
    fn cut_claims(&self) -> Vec<CutClaim> {
        Chain::cut_claims(self)
    }
    fn reset(&self) {
        Chain::reset(self)
    }
}

/// `Chain.Learn` errors.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LearnError {
    #[error("identity: Learn takes person rows, not {0}")]
    NotPerson(String),
    #[error(transparent)]
    Verify(VerifyError),
}

/// Starts the owner's sender key, `user:<deviceId>`.
pub const USER_KEY_PREFIX: &str = "user:";

/// The owner's sender key on a device.
pub fn user_key(device_id: &str) -> String {
    format!("{USER_KEY_PREFIX}{device_id}")
}

/// The IsOwner rule for a device with no chain: only `user:<deviceId>`.
pub fn owner_only(device_id: &str) -> impl Fn(&str) -> bool + use<> {
    let want = user_key(device_id);
    move |key: &str| key == want
}
