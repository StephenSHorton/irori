//! The code, the pairing room it derives, and what the proofs and words
//! cover (I:internal/pair/code.go, identity.md §4.7). Everything here ports
//! exactly; only the fingerprint inputs to [`info`] change under iroh (D5).

use hmac::{Hmac, KeyInit, Mac};
use irori_meshseam::Secret;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::wordlist::{EVEN_WORDS, ODD_WORDS};

/// The pairing room's app name: the scrypt salt and topic prefix
/// (go-room's `SecretFromCode("irori-pair-v1", code)`).
pub const APP: &str = "irori-pair-v1";

/// The SAS's HKDF salt.
pub const SAS_SALT: &str = "irori/pair/1";

/// The number of words the person reads and types.
pub const SAS_WORDS: usize = 5;

const CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
const CODE_LEN: usize = 16;

/// Go `ErrBadCode`: a string that isn't a pairing code.
pub const ERR_BAD_CODE: &str =
    "pair: that isn't a pairing code (16 letters and numbers, like ABCD-EFGH-JKMN-PQRS)";

/// A fresh code, `XXXX-XXXX-XXXX-XXXX` (Go `NewCode`): 16 random bytes per
/// batch, a byte kept when below 248 (rejection sampling), symbol `v % 31`.
pub fn new_code() -> String {
    let limit = (256 / CODE_ALPHABET.len() * CODE_ALPHABET.len()) as u8;
    let mut out = Vec::with_capacity(CODE_LEN);
    let mut b = [0u8; CODE_LEN];
    while out.len() < CODE_LEN {
        getrandom::fill(&mut b).expect("getrandom");
        for v in b {
            if v < limit && out.len() < CODE_LEN {
                out.push(CODE_ALPHABET[v as usize % CODE_ALPHABET.len()]);
            }
        }
    }
    format_code(std::str::from_utf8(&out).expect("ascii"))
}

fn format_code(raw: &str) -> String {
    let mut s = String::with_capacity(CODE_LEN + 3);
    for (i, c) in raw.chars().enumerate() {
        if i > 0 && i % 4 == 0 {
            s.push('-');
        }
        s.push(c);
    }
    s
}

/// Go `NormalizeCode`: a code typed or pasted any way (any case, spaces,
/// with or without dashes) in its canonical form.
pub fn normalize_code(input: &str) -> Option<String> {
    let mut raw = String::with_capacity(CODE_LEN);
    for r in input
        .trim_matches(gocompat::text::is_space)
        .chars()
        .map(gocompat::text::to_upper)
    {
        match r {
            '-' | ' ' | '\t' => continue,
            r if r.is_ascii() && CODE_ALPHABET.contains(&(r as u8)) => raw.push(r),
            _ => return None,
        }
    }
    (raw.len() == CODE_LEN).then(|| format_code(&raw))
}

/// Go `RoomSecret`: scrypt(N=2^15, r=8, p=1, 48 bytes) over the canonical
/// code with salt [`APP`]; the key is the first 32 bytes and the topic
/// `APP/` + hex(the next 16). The room has no epoch (0).
pub fn room_secret(code: &str) -> Result<Secret, &'static str> {
    room_secret_for(APP, code)
}

/// [`room_secret`] under another app name, which is both the scrypt salt
/// and the topic prefix (go-room's `SecretFromCode(app, code)`; the invite
/// room is `irori-invite-v1`).
pub fn room_secret_for(app: &str, code: &str) -> Result<Secret, &'static str> {
    let canon = normalize_code(code).ok_or(ERR_BAD_CODE)?;
    let params = scrypt::Params::new(15, 8, 1).expect("scrypt params");
    let mut dk = zeroize::Zeroizing::new([0u8; 48]);
    scrypt::scrypt(canon.as_bytes(), app.as_bytes(), &params, &mut dk[..])
        .expect("48 bytes is a valid scrypt length");
    let mut key = [0u8; 32];
    key.copy_from_slice(&dk[..32]);
    Ok(Secret {
        key,
        topic: format!("{app}/{}", hex::encode(&dk[32..])),
        epoch: 0,
    })
}

/// Go `Info`: what the proof and the SAS cover. Both device keys, both
/// channel fingerprints as the transport saw them, and both device ids,
/// always in A-then-B order, each `u32 BE` length-prefixed. On iroh both
/// fingerprints are the session's `EXPORTER-irori-sas` bytes (D5).
pub fn info(a_pub: &[u8], b_pub: &[u8], fp_a: &str, fp_b: &str, a_id: &str, b_id: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for f in [
        a_pub,
        b_pub,
        fp_a.as_bytes(),
        fp_b.as_bytes(),
        a_id.as_bytes(),
        b_id.as_bytes(),
    ] {
        out.extend_from_slice(&(f.len() as u32).to_be_bytes());
        out.extend_from_slice(f);
    }
    out
}

/// The SAS bytes: HKDF-SHA256(ikm = room key, salt = [`SAS_SALT`], info).
pub fn sas_bytes(room_key: &[u8; 32], info: &[u8]) -> [u8; SAS_WORDS] {
    sas_bytes_with(SAS_SALT, room_key, info)
}

/// [`sas_bytes`] under another HKDF salt (the invite's `irori/invite/1`).
pub fn sas_bytes_with(salt: &str, room_key: &[u8; 32], info: &[u8]) -> [u8; SAS_WORDS] {
    let mut b = [0u8; SAS_WORDS];
    hkdf::Hkdf::<Sha256>::new(Some(salt.as_bytes()), room_key)
        .expand(info, &mut b)
        .expect("5 bytes always fits");
    b
}

/// Go `SAS`: the five words, a PGP word per byte (even list at even
/// positions, odd list at odd ones).
pub fn sas(room_key: &[u8; 32], info: &[u8]) -> Vec<String> {
    sas_with(SAS_SALT, room_key, info)
}

/// [`sas`] under another HKDF salt.
pub fn sas_with(salt: &str, room_key: &[u8; 32], info: &[u8]) -> Vec<String> {
    sas_bytes_with(salt, room_key, info)
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if i % 2 == 0 {
                EVEN_WORDS[*v as usize]
            } else {
                ODD_WORDS[*v as usize]
            }
            .to_string()
        })
        .collect()
}

/// Go `Proof`: HMAC-SHA256 under the room key over the side (`b'A'` or
/// `b'B'`) and info, so one side's proof can't be reflected as the other's.
pub fn proof(room_key: &[u8; 32], side: u8, info: &[u8]) -> Vec<u8> {
    let mut m = <Hmac<Sha256> as KeyInit>::new_from_slice(room_key).expect("any key length");
    m.update(&[side]);
    m.update(info);
    m.finalize().into_bytes().to_vec()
}

/// Go `CheckProof`, in constant time.
pub fn check_proof(room_key: &[u8; 32], side: u8, info: &[u8], got: &[u8]) -> bool {
    let want = proof(room_key, side, info);
    want.len() == got.len() && bool::from(want.ct_eq(got))
}

/// Go `ParseWords`: lower-cased, split on space `,` `-` tab, CR, LF and `.`.
pub fn parse_words(typed: &str) -> Vec<String> {
    gocompat::text::to_lower_str(typed)
        .split([' ', ',', '-', '\t', '\n', '\r', '.'])
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Go `WordsMatch`: typed names exactly `want`, in order.
pub fn words_match(want: &[String], typed: &str) -> bool {
    let got = parse_words(typed);
    if got.len() != want.len() {
        return false;
    }
    let mut ok = 1u8;
    for (g, w) in got.iter().zip(want) {
        ok &= u8::from(g.len() == w.len() && bool::from(g.as_bytes().ct_eq(w.as_bytes())));
    }
    ok == 1
}

/// Go `Fingerprint`: the first 8 bytes of sha256(pub), hex, in groups of four.
pub fn fingerprint(pub_key: &[u8]) -> String {
    let h = hex::encode(&Sha256::digest(pub_key)[..8]);
    format!("{} {} {} {}", &h[0..4], &h[4..8], &h[8..12], &h[12..16])
}

/// Go `KeyMatch`: typed is the fingerprint `want`, in any case and with or
/// without its spaces or dashes.
pub fn key_match(want: &str, typed: &str) -> bool {
    fn norm(s: &str) -> String {
        gocompat::text::to_lower_str(s)
            .chars()
            .filter(|r| r.is_ascii_digit() || ('a'..='f').contains(r))
            .collect()
    }
    let (w, t) = (norm(want), norm(typed));
    !w.is_empty() && w.len() == t.len() && bool::from(w.as_bytes().ct_eq(t.as_bytes()))
}
