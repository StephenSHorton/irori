//! Wrapped-secret and device.json file shapes, against Go's writers
//! (I:internal/identity/keystore.go `writeSealed`, I:internal/device/device.go
//! `Save`). Fixture keys only; nothing real is read.

use irori_identity::{DeviceJson, Error, FileKeystore, Keystore, Wrapped};

#[test]
fn wrapped_file_is_go_marshal_indent() {
    let w = Wrapped::seal(&FileKeystore, &[5u8; 32]).unwrap();
    let text = String::from_utf8(w.to_file_bytes()).unwrap();
    // json.MarshalIndent(wrapped{1, "file", seed}, "", "  "): v, keystore,
    // blob in that order, std base64 with padding, no trailing newline.
    assert_eq!(
        text,
        "{\n  \"v\": 1,\n  \"keystore\": \"file\",\n  \"blob\": \"BQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQU=\"\n}"
    );
    let back = Wrapped::parse(text.as_bytes()).unwrap();
    assert_eq!(back.v, 1);
    assert_eq!(back.unseal(&FileKeystore).unwrap().as_slice(), &[5u8; 32]);
}

#[test]
fn wrapped_parse_reads_like_go_unmarshal() {
    // Case-insensitive keys and last-duplicate-wins, as json.Unmarshal.
    let raw = br#"{"V":1,"KEYSTORE":"file","blob":"AAAA","Blob":"BQUF"}"#;
    let w = Wrapped::parse(raw).unwrap();
    assert_eq!(w.keystore, "file");
    assert_eq!(w.unseal(&FileKeystore).unwrap().as_slice(), &[5u8, 5, 5]);
    // A type mismatch fails the whole value.
    assert!(Wrapped::parse(br#"{"v":"1","keystore":"file","blob":"AAAA"}"#).is_err());
    assert!(Wrapped::parse(br#"{"v":1,"keystore":"file","blob":"not base64!"}"#).is_err());
}

#[test]
fn unseal_refuses_another_keystore_with_gos_text() {
    let w = Wrapped::parse(br#"{"v":1,"keystore":"dpapi","blob":"AAAA"}"#).unwrap();
    let err = w.unseal(&FileKeystore).unwrap_err();
    assert!(matches!(err, Error::WrongKeystore { .. }));
    assert_eq!(
        err.to_string(),
        "identity: key sealed by a different keystore: sealed with \"dpapi\", this device uses \"file\" (was the data dir copied from another machine?)"
    );
}

#[test]
fn wrapped_write_is_atomic_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("sub").join("turn.key");
    let w = Wrapped::seal(&FileKeystore, b"0123456789abcdef0123456789abcdef").unwrap();
    w.write(&p).unwrap();
    assert!(!dir.path().join("sub").join("turn.key.tmp").exists());
    let back = Wrapped::read(&p).unwrap();
    assert_eq!(
        back.unseal(&FileKeystore).unwrap().as_slice(),
        b"0123456789abcdef0123456789abcdef"
    );
    assert_eq!(std::fs::read(&p).unwrap(), w.to_file_bytes());
}

#[test]
fn aes_keystore_round_trips_and_fails_on_a_wrong_kek() {
    use irori_identity::AesKeystore;
    use zeroize::Zeroizing;
    let ks = AesKeystore::new("keychain", Box::new(|| Ok(Zeroizing::new(vec![3u8; 32]))));
    let blob = ks.seal(b"secret").unwrap();
    assert_eq!(blob.len(), 12 + 6 + 16);
    assert_eq!(ks.unseal(&blob).unwrap().as_slice(), b"secret");
    let other = AesKeystore::new("keychain", Box::new(|| Ok(Zeroizing::new(vec![4u8; 32]))));
    assert!(other.unseal(&blob).is_err());
    let short = AesKeystore::new("keychain", Box::new(|| Ok(Zeroizing::new(vec![4u8; 16]))));
    assert!(
        short
            .seal(b"x")
            .unwrap_err()
            .to_string()
            .contains("want 32")
    );
}

#[test]
fn device_json_matches_go_save() {
    let d = DeviceJson {
        id: "0123456789abcdef0123456789abcdef".into(),
        name: "DESKTOP-<X>".into(),
        os: "darwin".into(),
        person_id: String::new(),
        device_pub: "ab".into(),
    };
    // MarshalIndent with HTML escaping on; personId omitempty.
    assert_eq!(
        String::from_utf8(d.to_file_bytes()).unwrap(),
        "{\n  \"id\": \"0123456789abcdef0123456789abcdef\",\n  \"name\": \"DESKTOP-\\u003cX\\u003e\",\n  \"os\": \"darwin\",\n  \"devicePub\": \"ab\"\n}"
    );
    assert_eq!(DeviceJson::parse(&d.to_file_bytes()).unwrap(), d);
    assert!(
        DeviceJson::parse(br#"{"name":"x"}"#).is_none(),
        "empty id is unreadable"
    );
    assert!(DeviceJson::parse(br#"{"id":5}"#).is_none());
    assert_eq!(
        DeviceJson::parse(br#"{"ID":"x","Os":"windows"}"#)
            .unwrap()
            .os,
        "windows"
    );
}

/// identity.md §7.1 CANON/VERIFY: a fixture root self-cert signed by Go
/// (`ed25519.Sign` over `identity.Canonical`; person key from seed 32x0x42,
/// device key from seed 32x0x43), re-canonicalised with gocompat, verifies
/// under the person key with dalek's non-strict `verify`, as Go's
/// `ed25519.Verify` does.
#[test]
fn root_cert_signature_verifies() {
    use ed25519_dalek::{Signature, Verifier as _, VerifyingKey};
    let body = br#"{"personId":"3097e2dee2cb4a34b53840cdb705aed7","deviceId":"a1b2c3d4e5f60718293a4b5c6d7e8f90","devicePub":"22fc297792f0b6ffc0bfcfdb7edb0c0aa14e025a365ec0e342e86e3829cb74b6","label":"DESKTOP-FIXTURE","role":"desktop","at":"2026-10-28T14:25:23.9472209Z"}"#;
    let msg = gocompat::json::canonical_raw(body).unwrap();
    let pk: [u8; 32] =
        hex::decode("2152f8d19b791d24453242e15f2eab6cb7cffa7b6a5ed30097960e069881db12")
            .unwrap()
            .try_into()
            .unwrap();
    let sig: [u8; 64] = hex::decode("f0383b98004df4c4794e59cefc6bcb1d5ab3dbd382591aa536aa9df1d5a84a372d53799fd1363ae8ee62e871278ade9cad0391baebd8c48ab48fea07f8d68708")
        .unwrap()
        .try_into()
        .unwrap();
    let vk = VerifyingKey::from_bytes(&pk).unwrap();
    vk.verify(&msg, &Signature::from_bytes(&sig))
        .expect("the Go-signed cert verifies");
    // A Rust re-encode that kept serde's escaping would not.
    let mut bad = msg.clone();
    bad.push(b' ');
    assert!(vk.verify(&bad, &Signature::from_bytes(&sig)).is_err());
    // personId is PersonIDOf(personPub).
    assert_eq!(
        irori_identity::person_id_of(&pk),
        "3097e2dee2cb4a34b53840cdb705aed7"
    );
}
