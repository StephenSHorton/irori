//! The read-only identity loader and the keystores, against fixtures sealed
//! the Go way here (moved from irori-net's M1 tests).
//! Never touches a real data dir or key.

use irori_identity as identity;

fn seed() -> [u8; 32] {
    let mut s = [0u8; 32];
    getrandom::fill(&mut s).unwrap();
    s
}

fn write_fixture(dir: &std::path::Path, seed: &[u8; 32], device_pub: Option<&str>, wrapped: &str) {
    let mut dj = String::from(
        "{\n  \"id\": \"0123456789abcdef0123456789abcdef\",\n  \"name\": \"FIXTURE\",\n  \"os\": \"windows\"",
    );
    if let Some(p) = device_pub {
        dj.push_str(&format!(
            ",\n  \"personId\": \"0f1e2d3c4b5a69788796a5b4c3d2e1f0\",\n  \"devicePub\": \"{p}\""
        ));
    }
    dj.push_str("\n}");
    std::fs::write(dir.join("device.json"), dj).unwrap();
    std::fs::write(dir.join("device.key"), wrapped).unwrap();
    let _ = seed;
}

fn pub_hex(seed: &[u8; 32]) -> String {
    hex::encode(
        ed25519_dalek::SigningKey::from_bytes(seed)
            .verifying_key()
            .to_bytes(),
    )
}

#[test]
fn loads_a_key_sealed_the_go_way() {
    let dir = tempfile::tempdir().unwrap();
    let s = seed();
    let wrapped = identity::wrap_for_tests(&s, Some(&[3u8; 32])).unwrap();
    let want = pub_hex(&s);
    write_fixture(dir.path(), &s, Some(&want), &wrapped);
    if cfg!(target_os = "macos") {
        // The keychain KEK cannot be planted in a test; the AES shape is tested below.
        return;
    }
    let before: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    let id = identity::load(dir.path()).unwrap();
    assert_eq!(id.device_pub_hex(), want);
    assert_eq!(id.device.device_pub, want);
    assert_eq!(id.device.name, "FIXTURE");
    // Debug never shows the seed.
    let dbg = format!("{id:?}");
    assert!(dbg.contains("redacted"));
    assert!(!dbg.contains(&hex::encode(s)));
    // Read-only: no file created or rewritten.
    let after: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(before, after);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("device.key")).unwrap(),
        wrapped
    );
}

#[test]
fn loads_without_device_pub_and_refuses_a_mixed_dir() {
    if cfg!(target_os = "macos") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let s = seed();
    let wrapped = identity::wrap_for_tests(&s, None).unwrap();
    write_fixture(dir.path(), &s, None, &wrapped);
    assert_eq!(
        identity::load(dir.path()).unwrap().device_pub_hex(),
        pub_hex(&s)
    );

    let other = pub_hex(&seed());
    write_fixture(dir.path(), &s, Some(&other), &wrapped);
    let err = identity::load(dir.path()).unwrap_err().to_string();
    assert!(err.contains("mixed from two installs"), "{err}");
}

#[test]
fn refuses_another_keystore() {
    let dir = tempfile::tempdir().unwrap();
    let s = seed();
    let other = if identity::platform_keystore() == "file" {
        "dpapi"
    } else {
        "file"
    };
    let wrapped =
        format!("{{\n  \"v\": 1,\n  \"keystore\": \"{other}\",\n  \"blob\": \"AAAA\"\n}}");
    write_fixture(dir.path(), &s, None, &wrapped);
    let err = identity::load(dir.path()).unwrap_err().to_string();
    assert!(err.contains("different keystore"), "{err}");
}

#[cfg(windows)]
#[test]
fn dpapi_blob_needs_the_irori_entropy() {
    // A blob sealed with the right entropy opens; a corrupted one fails.
    let s = seed();
    let mut blob = identity::dpapi::seal_for_tests(&s).unwrap();
    assert_eq!(identity::dpapi::unseal(&blob).unwrap().as_slice(), &s);
    let n = blob.len();
    blob[n - 1] ^= 0xff;
    assert!(identity::dpapi::unseal(&blob).is_err());
}

#[test]
fn keychain_aes_shape_round_trips() {
    let kek = [9u8; 32];
    let s = seed();
    let blob = identity::aes_seal_for_tests(&kek, [1u8; 12], &s);
    assert_eq!(blob.len(), 12 + 32 + 16);
    assert_eq!(identity::aes_unseal(&kek, &blob).unwrap().as_slice(), &s);
    assert!(identity::aes_unseal(&[8u8; 32], &blob).is_err());
    assert!(identity::aes_unseal(&kek, &blob[..8]).is_err());
}

#[test]
fn keychain_account_name_matches_go() {
    // identity.md §5: the fixture Mac dir hashes to kek-8a2f5e0399ac9edc.
    assert_eq!(
        identity::keychain_account("/Users/pat/.irori"),
        "kek-8a2f5e0399ac9edc"
    );
    // Go `kekAccounts` names, printed by a Go test run over the Go package
    // (path.Clean, which is filepath.Clean on macOS): clean name first, then
    // the as-given legacy name when it differs.
    let cases: &[(&str, &[&str])] = &[
        ("/Users/pat/.irori", &["kek-8a2f5e0399ac9edc"]),
        (
            "/Users/pat/.irori/",
            &["kek-8a2f5e0399ac9edc", "kek-d811dddc482d6aef"],
        ),
        (
            "/Users/pat/./x/../.irori//",
            &["kek-8a2f5e0399ac9edc", "kek-4479ff3fe05231fd"],
        ),
        (
            "/tmp/m1/",
            &["kek-5d5d4b506e6d232b", "kek-7d8c028c5a358244"],
        ),
        (
            "/../tmp//m1/.",
            &["kek-5d5d4b506e6d232b", "kek-1a947355a99a8fec"],
        ),
        ("/", &["kek-8a5edab282632443"]),
    ];
    for (dir, want) in cases {
        assert_eq!(
            identity::keychain_accounts(dir, "/elsewhere"),
            want.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            "{dir}"
        );
    }
    // A relative dir joins the cwd first, as filepath.Abs does.
    assert_eq!(
        identity::keychain_accounts(".irori/", "/Users/pat")[0],
        "kek-8a2f5e0399ac9edc"
    );
}

#[test]
fn go_clean_matches_go_path_clean() {
    // Cases from Go's path/path_test.go.
    for (i, o) in [
        ("", "."),
        ("abc", "abc"),
        ("abc/def", "abc/def"),
        ("a/b/c", "a/b/c"),
        (".", "."),
        ("..", ".."),
        ("../..", "../.."),
        ("../../abc", "../../abc"),
        ("/abc", "/abc"),
        ("/", "/"),
        ("abc/", "abc"),
        ("abc/def/", "abc/def"),
        ("a/b/c/", "a/b/c"),
        ("./", "."),
        ("../", ".."),
        ("../../", "../.."),
        ("/abc/", "/abc"),
        ("abc//def//ghi", "abc/def/ghi"),
        ("//abc", "/abc"),
        ("///abc", "/abc"),
        ("//abc//", "/abc"),
        ("abc//", "abc"),
        ("abc/./def", "abc/def"),
        ("/./abc/def", "/abc/def"),
        ("abc/.", "abc"),
        ("abc/def/ghi/../jkl", "abc/def/jkl"),
        ("abc/def/../ghi/../jkl", "abc/jkl"),
        ("abc/def/..", "abc"),
        ("abc/def/../..", "."),
        ("/abc/def/../..", "/"),
        ("abc/def/../../..", ".."),
        ("/abc/def/../../..", "/"),
        ("abc/def/../../../ghi/jkl/../../../mno", "../../mno"),
        ("abc/./../def", "def"),
        ("abc//./../def", "def"),
        ("abc/../../././../def", "../../def"),
    ] {
        assert_eq!(gocompat::filepath::unix::clean(i), o, "{i:?}");
    }
}

/// identity.md §7.1 SEED: Go's ed25519 key and PersonIDOf for seed 32x0x05.
#[test]
fn seed_vector_matches_go() {
    use sha2::Digest;
    let id = identity::DeviceIdentity::from_seed_for_tests([5u8; 32]);
    let pub_hex = id.device_pub_hex();
    assert_eq!(
        pub_hex,
        "6e7a1cdd29b0b78fd13af4c5598feff4ef2a97166e3ca6f2e4fbfccd80505bf1"
    );
    let sum = sha2::Sha256::digest(id.public_key());
    assert_eq!(
        identity::person_id_of(&id.public_key()),
        "7599776c3085e3f9da0d13071eb0b4ab"
    );
    assert_eq!(hex::encode(&sum[..16]), "7599776c3085e3f9da0d13071eb0b4ab");
}

/// A wrapped file Go's `aesKeystore` sealed (seed 32x0x05 under KEK 32x0x03,
/// random nonce, `json.MarshalIndent`), opened by the Rust unseal. Not a
/// secret: both keys are test constants.
#[test]
fn go_sealed_keychain_blob_unseals() {
    const GO_WRAPPED: &str = "{
  \"v\": 1,
  \"keystore\": \"keychain\",
  \"blob\": \"QGC5iucHu8wQoD4wbK6j5vBYc6Ui7RhqtFD7e+YRq3M/r74jFHLbps7N41UKe4QyUCX10JVWW3ERRgyq\"
}";
    use base64::Engine as _;
    let v: serde_json::Value = serde_json::from_str(GO_WRAPPED).unwrap();
    let blob = base64::engine::general_purpose::STANDARD
        .decode(v["blob"].as_str().unwrap())
        .unwrap();
    let plain = identity::aes_unseal(&[3u8; 32], &blob).unwrap();
    assert_eq!(plain.as_slice(), &[5u8; 32]);
    // The Rust fixture writer produces the same file shape.
    let ours = identity::wrap_for_tests(&[5u8; 32], Some(&[3u8; 32])).unwrap();
    let ours: serde_json::Value = serde_json::from_str(&ours).unwrap();
    assert_eq!(ours["v"], 1);
    assert!(ours["keystore"].is_string());
}

#[test]
fn wrapped_debug_redacts_the_blob() {
    // Under the file keystore the blob is the seed in base64.
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode([5u8; 32]);
    let raw = format!("{{\"v\": 1, \"keystore\": \"file\", \"blob\": \"{b64}\"}}");
    let w = identity::Wrapped::parse(raw.as_bytes()).unwrap();
    let dbg = format!("{w:?}");
    assert!(dbg.contains("redacted") && dbg.contains("file"), "{dbg}");
    assert!(!dbg.contains(&b64), "{dbg}");
}

/// Unseal a data dir that Go sealed on this machine (a DPAPI or Keychain blob
/// is bound to the user, so it can't be committed). Point
/// `IRORI_GO_SEALED_DIR` at a scratch copy holding device.json and device.key,
/// never the live dir. Checks the key against devicePub; prints nothing.
#[test]
#[ignore = "needs IRORI_GO_SEALED_DIR, a Go-sealed scratch data dir"]
fn go_sealed_data_dir_unseals() {
    let dir = std::env::var_os("IRORI_GO_SEALED_DIR").expect("IRORI_GO_SEALED_DIR");
    let id = identity::load(std::path::Path::new(&dir)).expect("unseal the Go-sealed key");
    assert!(
        !id.device.device_pub.is_empty(),
        "device.json has no devicePub"
    );
    assert_eq!(id.device_pub_hex(), id.device.device_pub);
}

/// `IRORI_DEV_KEK_DIR` is honoured only in debug builds (D14): a release
/// build never lets an environment variable pick the Keychain item.
#[test]
fn dev_kek_dir_is_debug_only() {
    let got = identity::dev_kek_dir_of(Some("/Users/x/.irori".into()));
    if cfg!(debug_assertions) {
        assert_eq!(got.as_deref(), Some("/Users/x/.irori"));
    } else {
        assert_eq!(got, None);
    }
    assert_eq!(identity::dev_kek_dir_of(Some("".into())), None);
    assert_eq!(identity::dev_kek_dir_of(None), None);
}
