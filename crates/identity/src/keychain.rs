//! The macOS keystore's key-encryption key (I:internal/identity/keychain.go;
//! identity.md §5): 32 bytes in the login Keychain, reached only through
//! `/usr/bin/security`. Free of `cfg` so the rules are tested on every OS
//! with a fake `security`; `keystore::keychain` wires the real one.
//!
//! A key is made only on first run: while a file in the data dir is sealed
//! with the `keychain` keystore, a missing item is [`Error::KekMissing`],
//! never a new key that could not open them.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use gocompat::json::{self, Schema};
use zeroize::Zeroizing;

use crate::Error;

/// The Keychain service name.
pub const KEYCHAIN_SERVICE: &str = "Irori";
/// `security` exit code: item not found.
pub const ERR_SEC_ITEM_NOT_FOUND: i32 = 44;
/// `security` exit code: duplicate item.
pub const ERR_SEC_DUPLICATE_ITEM: i32 = 45;

/// Runs `/usr/bin/security` with these arguments: its stdout, or its exit
/// code (-1 when it didn't run) and an error text.
pub type SecurityRunner = Box<dyn Fn(&[&str]) -> Result<Vec<u8>, (i32, String)> + Send + Sync>;

/// Go `kekAccounts`: the account of the data dir's absolute, clean path
/// (this OS's `filepath` rules, lexical), then the dir as given when that
/// differs (older builds hashed it as given). On Unix a relative dir joins
/// Go's `os.Getwd`, which keeps `$PWD`'s spelling of a symlinked cwd.
pub fn kek_accounts(data_dir: &str) -> Vec<String> {
    use gocompat::filepath;
    // Go keeps the dir as given when Abs fails.
    let canon = filepath::abs(data_dir).unwrap_or_else(|_| data_dir.to_string());
    crate::keystore::accounts_of(&filepath::clean(&canon), data_dir)
}

/// One data dir's KEK (Go `keychainKEK`).
pub struct KeychainKek {
    data_dir: PathBuf,
    accounts: Vec<String>,
    run: SecurityRunner,
    /// Kept once read or made; a failure is retried next call.
    key: Mutex<Option<Zeroizing<Vec<u8>>>>,
}

enum Find {
    Found(Zeroizing<Vec<u8>>),
    NotFound,
}

impl KeychainKek {
    /// Go `newKeychainKEK(dataDir, run)`.
    pub fn new(data_dir: &Path, run: SecurityRunner) -> KeychainKek {
        let accounts = kek_accounts(&data_dir.to_string_lossy());
        KeychainKek::with_accounts(data_dir, accounts, run)
    }

    /// With the account names given (the dev-only `IRORI_DEV_KEK_DIR` names
    /// them after another dir than the one checked for sealed files).
    pub fn with_accounts(data_dir: &Path, accounts: Vec<String>, run: SecurityRunner) -> Self {
        KeychainKek {
            data_dir: data_dir.to_path_buf(),
            accounts,
            run,
            key: Mutex::new(None),
        }
    }

    /// The account a new item is made under.
    pub fn account(&self) -> &str {
        &self.accounts[0]
    }

    /// The KEK: found, or made on first run. Errors are
    /// [`Error::KeystoreUnavailable`], so unseal doesn't blame a copied
    /// data dir for them.
    pub fn get(&self) -> Result<Zeroizing<Vec<u8>>, Error> {
        let mut g = self.key.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(k) = g.as_ref() {
            return Ok(k.clone());
        }
        let key = self.load().map_err(|e| match e {
            Error::KekMissing { .. } => e,
            other => Error::KeystoreUnavailable(other.to_string()),
        })?;
        *g = Some(key.clone());
        Ok(key)
    }

    fn load(&self) -> Result<Zeroizing<Vec<u8>>, Error> {
        if let Find::Found(k) = self.find()? {
            return Ok(k);
        }
        let sealed = keychain_sealed(&self.data_dir);
        if !sealed.is_empty() {
            return Err(Error::KekMissing {
                account: self.accounts[0].clone(),
                detail: format!(
                    "{} in {} {} sealed with it. The Keychain item (service {}, account {}) was deleted, or the data dir was moved or renamed (the item is named after its path). Restore the item from a Keychain backup, or move the data dir back",
                    sealed.join(" and "),
                    self.data_dir.display(),
                    if sealed.len() == 1 { "is" } else { "are" },
                    gocompat::quote::quote(KEYCHAIN_SERVICE),
                    gocompat::quote::quote(&self.accounts[0]),
                ),
            });
        }
        let mut key = Zeroizing::new(vec![0u8; 32]);
        getrandom::fill(&mut key[..]).map_err(|e| Error::Keystore(format!("random: {e}")))?;
        let hex_key = Zeroizing::new(hex::encode(&key[..]));
        let args = [
            "add-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            &self.accounts[0],
            "-l",
            "Irori device key",
            "-w",
            &hex_key,
        ];
        match (self.run)(&args) {
            Ok(_) => Ok(key),
            // Another process made it between the find and the add.
            Err((ERR_SEC_DUPLICATE_ITEM, _)) => match self.find()? {
                Find::Found(k) => Ok(k),
                Find::NotFound => Err(Error::Keystore("not found".into())),
            },
            Err((_, e)) => Err(Error::Keystore(format!(
                "identity: store Irori's key in the Keychain: {e}"
            ))),
        }
    }

    fn find(&self) -> Result<Find, Error> {
        for acct in &self.accounts {
            let args = [
                "find-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                acct,
                "-w",
            ];
            let out = match (self.run)(&args) {
                Ok(out) => Zeroizing::new(out),
                Err((ERR_SEC_ITEM_NOT_FOUND, _)) => continue,
                Err((_, e)) => {
                    return Err(Error::Keystore(format!(
                        "identity: read Irori's key from the Keychain: {e}"
                    )));
                }
            };
            let text = Zeroizing::new(String::from_utf8_lossy(&out).trim().to_string());
            match hex::decode(text.as_bytes()) {
                Ok(k) if k.len() == 32 => return Ok(Find::Found(Zeroizing::new(k))),
                _ => {
                    return Err(Error::Keystore(format!(
                        "identity: the Keychain item for Irori's key (account {}) is damaged",
                        gocompat::quote::quote(acct)
                    )));
                }
            }
        }
        Ok(Find::NotFound)
    }
}

/// The files in `data_dir` holding a secret sealed with the `keychain`
/// keystore: person.json and the `*.key` files (Go `keychainSealed`).
///
/// Reads only the `keystore` name. Go unmarshals the whole file, so a file
/// damaged elsewhere (a field of the wrong type, a bad base64 blob) is not
/// "sealed" there and Go would make a new key. Here it still counts, so
/// no key is made that could orphan a file only partly damaged; the
/// deliberate, more cautious side.
pub fn keychain_sealed(data_dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let ks_of = |raw: &[u8], nested: bool| -> String {
        let w = Schema::object(vec![("keystore", Schema::String)]);
        let schema = if nested {
            Schema::object(vec![("secretKey", w)])
        } else {
            w
        };
        match json::unmarshal(raw, &schema) {
            Ok(o) if nested => o
                .get("secretKey")
                .map(|s| s.get_str("keystore").to_string())
                .unwrap_or_default(),
            Ok(o) => o.get_str("keystore").to_string(),
            Err(_) => String::new(),
        }
    };
    if let Ok(raw) = std::fs::read(data_dir.join(crate::PERSON_FILE))
        && ks_of(&raw, true) == "keychain"
    {
        out.push(crate::PERSON_FILE.to_string());
    }
    let mut keys: Vec<PathBuf> = std::fs::read_dir(data_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "key"))
                .collect()
        })
        .unwrap_or_default();
    keys.sort(); // filepath.Glob sorts
    for p in keys {
        let Ok(raw) = std::fs::read(&p) else {
            continue;
        };
        if ks_of(&raw, false) == "keychain"
            && let Some(name) = p.file_name()
        {
            out.push(name.to_string_lossy().into_owned());
        }
    }
    out
}
