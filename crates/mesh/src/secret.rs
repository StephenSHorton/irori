//! The mesh topic of an epoch and the `person.secret` sealing (mesh.md §2.4;
//! I:internal/mesh/mesh.go:24-56, secret.go). Both are frozen formats
//! (D12): the goldens in testdata/golden/mesh hold Go's bytes.

use crypto_secretbox::aead::{Aead, KeyInit};
use crypto_secretbox::{Nonce, XSalsa20Poly1305};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

/// The mesh topic, its key and `MeshInfo` live in `irori-meshseam`, the
/// seam the iroh transport also builds on, so there is one copy of the
/// frozen format. Its `Secret` carries the epoch the iroh epoch-open
/// message names.
pub use irori_meshseam::{MESH_APP, Secret, mesh_info, mesh_secret};

/// The HKDF salt of a `person.secret` key.
const SECRET_APP: &str = "irori/person-secret/1";

/// NaCl secretbox's tag length.
const OVERHEAD: usize = 16;

fn secret_key(
    old_seed: &[u8],
    person_id: &str,
    epoch: i64,
    recipient: &str,
) -> Zeroizing<[u8; 32]> {
    let mut info = mesh_info(person_id, epoch);
    info.extend_from_slice(recipient.as_bytes());
    let mut k = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(Some(SECRET_APP.as_bytes()), old_seed)
        .expand(&info, &mut k[..])
        .expect("32 bytes always fits");
    k
}

/// Go `sealSecret`: `nonce(24) || secretbox(newSeed, nonce, k)`, with k
/// derived from the recipient's current seed, the new epoch and the
/// recipient's device id.
pub fn seal_secret(
    old_seed: &[u8],
    new_seed: &[u8],
    person_id: &str,
    epoch: i64,
    recipient: &str,
) -> Result<Vec<u8>, String> {
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut nonce).map_err(|e| format!("random: {e}"))?;
    seal_secret_with_nonce(old_seed, new_seed, person_id, epoch, recipient, nonce)
}

/// [`seal_secret`] with a given nonce (goldens).
pub fn seal_secret_with_nonce(
    old_seed: &[u8],
    new_seed: &[u8],
    person_id: &str,
    epoch: i64,
    recipient: &str,
    nonce: [u8; 24],
) -> Result<Vec<u8>, String> {
    let k = secret_key(old_seed, person_id, epoch, recipient);
    let cipher = XSalsa20Poly1305::new((&*k).into());
    let boxed = cipher
        .encrypt(&Nonce::from(nonce), new_seed)
        .map_err(|_| "mesh: seal person secret".to_string())?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&boxed);
    Ok(out)
}

/// Go `errBadSecret`.
pub const ERR_BAD_SECRET: &str = "mesh: person.secret does not open";

/// Go `openSecret`: opens a [`seal_secret`] result on the recipient.
pub fn open_secret(
    old_seed: &[u8],
    sealed: &[u8],
    person_id: &str,
    epoch: i64,
    self_id: &str,
) -> Result<Vec<u8>, &'static str> {
    if sealed.len() < 24 + OVERHEAD {
        return Err(ERR_BAD_SECRET);
    }
    let k = secret_key(old_seed, person_id, epoch, self_id);
    let cipher = XSalsa20Poly1305::new((&*k).into());
    let mut nonce = [0u8; 24];
    nonce.copy_from_slice(&sealed[..24]);
    cipher
        .decrypt(&Nonce::from(nonce), &sealed[24..])
        .map_err(|_| ERR_BAD_SECRET)
}
