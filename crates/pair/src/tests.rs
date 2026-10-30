//! pair_test.go's unit tests (TestCodeCodec, TestWordlists,
//! TestSASBindsKeysFingerprintsAndCode) and the ROOM, INFO and FP goldens
//! (testdata/golden/pair/vectors.json, identity.md §7.1).

use std::collections::HashSet;

use crate::code::*;
use crate::wordlist::{EVEN_WORDS, ODD_WORDS};

#[test]
fn code_codec() {
    let c = new_code();
    assert_eq!(c.len(), 19, "{c}");
    assert_eq!(c.matches('-').count(), 3, "{c}");
    for input in [
        c.clone(),
        c.to_lowercase(),
        c.replace('-', ""),
        format!(" {} ", c.replace('-', " ")),
    ] {
        assert_eq!(
            normalize_code(&input).as_deref(),
            Some(c.as_str()),
            "{input:?}"
        );
    }
    for bad in [
        String::new(),
        "ABCD".into(),
        format!("{c}A"),
        c.replacen(&c[..1], "0", 1),
    ] {
        assert!(normalize_code(&bad).is_none(), "{bad:?} accepted");
    }
    assert_ne!(new_code(), c, "two codes in a row are the same");
    let a = room_secret(&c.to_lowercase()).unwrap();
    let b = room_secret(&c).unwrap();
    assert_eq!(a, b, "room secret differs by spelling");
    assert!(a.topic.starts_with(&format!("{APP}/")));
    assert_eq!(a.epoch, 0);
    assert_eq!(room_secret("nope").unwrap_err(), ERR_BAD_CODE);
}

#[test]
fn wordlists() {
    let mut seen = HashSet::new();
    for list in [&EVEN_WORDS, &ODD_WORDS] {
        for (i, w) in list.iter().enumerate() {
            assert!(
                !w.is_empty() && *w == w.to_lowercase() && seen.insert(*w),
                "word {i} {w:?} is empty, not lower case, or repeated"
            );
        }
    }
    assert_eq!(EVEN_WORDS[0], "aardvark");
    assert_eq!(EVEN_WORDS[255], "zulu");
    assert_eq!(ODD_WORDS[0], "adroitness");
    assert_eq!(ODD_WORDS[255], "yucatan");
}

fn key() -> [u8; 32] {
    let mut s = [0u8; 32];
    getrandom::fill(&mut s).unwrap();
    s
}

#[test]
fn sas_binds_keys_fingerprints_and_code() {
    let (ap, bp, xp) = (key(), key(), key());
    let s1 = room_secret(&new_code()).unwrap();
    let s2 = room_secret(&new_code()).unwrap();
    let inf = info(&ap, &bp, "fpA", "fpB", "a", "b");
    let w = sas(&s1.key, &inf);
    assert_eq!(w.len(), SAS_WORDS);
    assert_eq!(w, sas(&s1.key, &info(&ap, &bp, "fpA", "fpB", "a", "b")));
    for (name, other) in [
        (
            "another device key",
            sas(&s1.key, &info(&ap, &xp, "fpA", "fpB", "a", "b")),
        ),
        (
            "another fingerprint",
            sas(&s1.key, &info(&ap, &bp, "fpA", "fpX", "a", "b")),
        ),
        ("another code", sas(&s2.key, &inf)),
        (
            "sides swapped",
            sas(&s1.key, &info(&bp, &ap, "fpB", "fpA", "b", "a")),
        ),
    ] {
        assert_ne!(other, w, "{name} gives the same words");
    }
    assert!(words_match(&w, &w.join("-").to_uppercase()));
    assert!(!words_match(&w, &w[..4].join(" ")));
    let p = proof(&s1.key, b'A', &inf);
    assert!(check_proof(&s1.key, b'A', &inf, &p));
    assert!(!check_proof(&s1.key, b'B', &inf, &p));
    assert!(!check_proof(&s2.key, b'A', &inf, &p));
}

#[test]
fn key_match_and_words() {
    let fp = fingerprint(&[1u8; 32]);
    assert!(key_match(&fp, &fp.to_uppercase().replace(' ', "-")));
    assert!(!key_match(&fp, "0000 0000 0000 0000"));
    assert!(!key_match("", ""));
    assert_eq!(
        parse_words("Orca, detergent-offload.narrative\tEGGHEAD\r\n"),
        ["orca", "detergent", "offload", "narrative", "egghead"]
    );
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Gold {
    room: GoldRoom,
    info: GoldInfo,
    fp: GoldFp,
}

#[derive(serde::Deserialize)]
struct GoldRoom {
    code: String,
    key: String,
    topic: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoldInfo {
    a_pub_byte: u8,
    b_pub_byte: u8,
    fp_a: String,
    fp_b: String,
    a_id: String,
    b_id: String,
    sas_bytes: String,
    words: Vec<String>,
    proof_a: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoldFp {
    pub_byte: u8,
    fingerprint: String,
}

#[test]
fn goldens_match_go() {
    let path = manifest_dir().join("../../testdata/golden/pair/vectors.json");
    let g: Gold = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let s = room_secret(&g.room.code).unwrap();
    assert_eq!(hex::encode(s.key), g.room.key, "ROOM key");
    assert_eq!(s.topic, g.room.topic, "ROOM topic");
    let i = &g.info;
    let inf = info(
        &[i.a_pub_byte; 32],
        &[i.b_pub_byte; 32],
        &i.fp_a,
        &i.fp_b,
        &i.a_id,
        &i.b_id,
    );
    assert_eq!(
        hex::encode(sas_bytes(&s.key, &inf)),
        i.sas_bytes,
        "INFO SAS bytes"
    );
    assert_eq!(sas(&s.key, &inf), i.words, "INFO words");
    assert_eq!(
        hex::encode(proof(&s.key, b'A', &inf)),
        i.proof_a,
        "INFO Proof('A')"
    );
    assert_eq!(fingerprint(&[g.fp.pub_byte; 32]), g.fp.fingerprint, "FP");
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
