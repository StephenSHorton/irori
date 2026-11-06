//! Ported from I:internal/identity/keychain_test.go: the Keychain KEK rules
//! over a fake `/usr/bin/security` (an in-memory Keychain), on every OS.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use irori_identity::keychain::{
    ERR_SEC_DUPLICATE_ITEM, ERR_SEC_ITEM_NOT_FOUND, KeychainKek, SecurityRunner, kek_accounts,
};
use irori_identity::*;

#[derive(Default)]
struct Fake {
    items: BTreeMap<String, String>,
    race_add: String,
    fail: i32,
}

#[derive(Clone, Default)]
struct FakeSecurity(Arc<Mutex<Fake>>);

impl FakeSecurity {
    fn runner(&self) -> SecurityRunner {
        let me = self.clone();
        Box::new(move |args: &[&str]| {
            let mut f = me.0.lock().unwrap();
            if f.fail != 0 {
                return Err((f.fail, format!("exit status {}", f.fail)));
            }
            let flag = |name: &str| {
                args.windows(2)
                    .find(|w| w[0] == name)
                    .map(|w| w[1].to_string())
                    .unwrap_or_default()
            };
            let acct = flag("-a");
            match args[0] {
                "find-generic-password" => match f.items.get(&acct) {
                    Some(pw) => Ok(format!("{pw}\n").into_bytes()),
                    None => Err((ERR_SEC_ITEM_NOT_FOUND, "exit status 44".into())),
                },
                "add-generic-password" => {
                    if !f.race_add.is_empty() {
                        let theirs = std::mem::take(&mut f.race_add);
                        f.items.insert(acct.clone(), theirs);
                    }
                    if f.items.contains_key(&acct) {
                        return Err((ERR_SEC_DUPLICATE_ITEM, "exit status 45".into()));
                    }
                    f.items.insert(acct, flag("-w"));
                    Ok(Vec::new())
                }
                _ => Err((1, "unknown command".into())),
            }
        })
    }
    fn items(&self) -> usize {
        self.0.lock().unwrap().items.len()
    }
}

fn keychain_store(dir: &Path, sec: &FakeSecurity) -> Box<dyn Keystore + Send + Sync> {
    let kek = Arc::new(KeychainKek::new(dir, sec.runner()));
    Box::new(AesKeystore::new("keychain", Box::new(move || kek.get())))
}

#[test]
fn keychain_kek_made_once_then_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let sec = FakeSecurity::default();
    let dev = DeviceJson::load_or_create(dir.path()).unwrap();
    let a = Identity::load(
        dir.path(),
        dev.clone(),
        Some(keychain_store(dir.path(), &sec)),
    )
    .unwrap();
    assert_eq!(sec.items(), 1);
    let b = Identity::load(
        dir.path(),
        dev.clone(),
        Some(keychain_store(dir.path(), &sec)),
    )
    .unwrap();
    assert_eq!(a.current().id, b.current().id);
    assert_eq!(a.device_pub(), b.device_pub());
    assert_eq!(sec.items(), 1, "restart made new keys");
    // Another spelling of the same dir finds the same item.
    let sep = std::path::MAIN_SEPARATOR;
    let other = format!("{}{sep}.{sep}", dir.path().display());
    let other = Path::new(&other);
    Identity::load(other, dev, Some(keychain_store(other, &sec))).unwrap();
    assert_eq!(sec.items(), 1);
}

#[test]
fn keychain_kek_missing_is_not_remade() {
    let dir = tempfile::tempdir().unwrap();
    let sec = FakeSecurity::default();
    let dev = DeviceJson::load_or_create(dir.path()).unwrap();
    Identity::load(
        dir.path(),
        dev.clone(),
        Some(keychain_store(dir.path(), &sec)),
    )
    .unwrap();
    sec.0.lock().unwrap().items.clear(); // the item was deleted
    let err = Identity::load(dir.path(), dev, Some(keychain_store(dir.path(), &sec))).unwrap_err();
    assert!(matches!(err.root(), Error::KekMissing { .. }), "{err}");
    let text = err.to_string();
    assert!(
        !text.contains("copied") && text.contains(DEVICE_KEY_FILE) && text.contains(PERSON_FILE),
        "misleading error: {text}"
    );
    assert_eq!(
        sec.items(),
        0,
        "a new key was stored while old secrets need the old one"
    );
}

#[test]
fn keychain_kek_moved_dir() {
    let dir = tempfile::tempdir().unwrap();
    let sec = FakeSecurity::default();
    let dev = DeviceJson::load_or_create(dir.path()).unwrap();
    Identity::load(
        dir.path(),
        dev.clone(),
        Some(keychain_store(dir.path(), &sec)),
    )
    .unwrap();
    let other = tempfile::tempdir().unwrap();
    let moved = other.path().join("moved");
    std::fs::rename(dir.path(), &moved).unwrap();
    let err = Identity::load(&moved, dev, Some(keychain_store(&moved, &sec))).unwrap_err();
    assert!(matches!(err.root(), Error::KekMissing { .. }), "{err}");
    assert_eq!(sec.items(), 1, "a key was made for the moved dir");
}

#[test]
fn keychain_kek_finds_the_legacy_name() {
    let dir = tempfile::tempdir().unwrap();
    let wd = std::env::current_dir().unwrap();
    let Ok(rel) = gocompat::filepath::rel(&wd.to_string_lossy(), &dir.path().to_string_lossy())
    else {
        return; // no relative path to the temp dir (another volume)
    };
    let accts = kek_accounts(&rel);
    assert_eq!(accts.len(), 2, "a relative path has two names: {accts:?}");
    let sec = FakeSecurity::default();
    sec.0
        .lock()
        .unwrap()
        .items
        .insert(accts[1].clone(), "ab".repeat(32));
    let k = KeychainKek::new(Path::new(&rel), sec.runner());
    let key = k.get().unwrap();
    assert_eq!(key.len(), 32);
    assert_eq!(sec.items(), 1);
}

#[test]
fn keychain_kek_add_race() {
    let dir = tempfile::tempdir().unwrap();
    let sec = FakeSecurity::default();
    let theirs = "cd".repeat(32);
    sec.0.lock().unwrap().race_add = theirs.clone();
    let key = KeychainKek::new(dir.path(), sec.runner()).get().unwrap();
    assert_eq!(hex::encode(&key[..]), theirs, "the loser kept its own key");
}

#[test]
fn keychain_kek_failure_is_retried() {
    let dir = tempfile::tempdir().unwrap();
    let sec = FakeSecurity::default();
    sec.0.lock().unwrap().fail = 51;
    let k = KeychainKek::new(dir.path(), sec.runner());
    assert!(matches!(k.get(), Err(Error::KeystoreUnavailable(_))));
    assert_eq!(sec.items(), 0, "made a key while the Keychain failed");
    sec.0.lock().unwrap().fail = 0;
    k.get().unwrap();
    // The unseal error names the keystore's key, not a copied dir.
    let failing = FakeSecurity::default();
    failing.0.lock().unwrap().fail = 51;
    let ks = keychain_store(dir.path(), &failing);
    let w = Wrapped::parse(
        format!(
            r#"{{"v":1,"keystore":"keychain","blob":"{}"}}"#,
            "A".repeat(56)
        )
        .as_bytes(),
    )
    .unwrap();
    let err = w.unseal(ks.as_ref()).unwrap_err();
    assert!(!err.to_string().contains("copied"), "{err}");
}
