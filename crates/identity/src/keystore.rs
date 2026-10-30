//! Keystores and wrapped secrets (identity.md §2.1, §5;
//! I:internal/identity/keystore*.go, keychain.go).
//!
//! Every secret file in the data dir (`device.key`, `turn.key`, the
//! `secretKey` fields of `person.json`, board `.secrets`) is a wrapped
//! secret: `{"v": 1, "keystore": "<name>", "blob": "<std base64>"}`, written
//! with Go's `MarshalIndent` (two spaces, no trailing newline). The blob is
//! sealed by this OS's keystore:
//!
//! | OS | name | sealing |
//! |---|---|---|
//! | Windows | `dpapi` | `CryptProtectData`, current user, entropy `irori/key/1` |
//! | macOS | `keychain` | AES-256-GCM, `nonce(12) ‖ ct+tag`, AAD `irori/key/1`, KEK in the login Keychain |
//! | other | `file` | the plaintext |
//!
//! The Keychain KEK is found or, on first run only, made by
//! [`crate::keychain::KeychainKek`].

use std::fmt;
use std::path::Path;

use base64::Engine as _;
use base64::engine::{GeneralPurpose, GeneralPurposeConfig};
use gocompat::json::{self, Schema};
use zeroize::Zeroizing;

use crate::Error;

/// DPAPI entropy and the keychain AES-GCM AAD (identity.md §5, trap 5).
pub const KEY_CONTEXT: &[u8] = b"irori/key/1";

/// Go's `base64.StdEncoding` for decoding: padding required, non-zero
/// trailing bits accepted (Go's non-strict default).
const GO_STD: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
);

/// Go's `[]byte` JSON decoding: StdEncoding, with `\r` and `\n` ignored.
pub(crate) fn go_base64_decode(s: &str) -> Option<Vec<u8>> {
    let cleaned: String = s.chars().filter(|&c| c != '\r' && c != '\n').collect();
    GO_STD.decode(cleaned.as_bytes()).ok()
}

/// Seals and unseals secrets at rest (Go `Keystore`).
pub trait Keystore {
    /// Written into each wrapped file: `dpapi`, `keychain` or `file`.
    fn name(&self) -> &'static str;
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, Error>;
    fn unseal(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error>;
}

/// The keystore name Go uses on this OS.
pub fn platform_keystore() -> &'static str {
    if cfg!(windows) {
        "dpapi"
    } else if cfg!(target_os = "macos") {
        "keychain"
    } else {
        "file"
    }
}

/// Go's `DefaultKeystore(dataDir)` for this OS.
pub fn default_keystore(data_dir: &Path) -> Box<dyn Keystore + Send + Sync> {
    #[cfg(windows)]
    {
        let _ = data_dir;
        Box::new(DpapiKeystore)
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(keychain::KeychainKeystore::new(data_dir))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = data_dir;
        Box::new(FileKeystore)
    }
}

/// Plaintext in a 0600 file: the fallback off Windows and macOS, and what
/// tests use.
pub struct FileKeystore;

impl Keystore for FileKeystore {
    fn name(&self) -> &'static str {
        "file"
    }
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, Error> {
        Ok(plain.to_vec())
    }
    fn unseal(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
        Ok(Zeroizing::new(blob.to_vec()))
    }
}

/// AES-256-GCM under a key-encryption key (Go `aesKeystore`). The macOS
/// keystore is this with the Keychain KEK.
pub struct AesKeystore {
    pub name: &'static str,
    kek: Box<dyn Fn() -> Result<Zeroizing<Vec<u8>>, Error> + Send + Sync>,
}

impl AesKeystore {
    pub fn new(
        name: &'static str,
        kek: Box<dyn Fn() -> Result<Zeroizing<Vec<u8>>, Error> + Send + Sync>,
    ) -> Self {
        AesKeystore { name, kek }
    }

    fn cipher(&self) -> Result<aes_gcm::Aes256Gcm, Error> {
        use aes_gcm::aead::KeyInit;
        let k = (self.kek)()?;
        if k.len() != 32 {
            return Err(Error::Keystore(format!(
                "key-encryption key is {} bytes, want 32",
                k.len()
            )));
        }
        aes_gcm::Aes256Gcm::new_from_slice(&k)
            .map_err(|_| Error::Keystore("bad key-encryption key".into()))
    }

    /// Seal with a chosen nonce (fixtures; `seal` draws a random one).
    pub fn seal_with_nonce(&self, nonce: [u8; 12], plain: &[u8]) -> Result<Vec<u8>, Error> {
        use aes_gcm::aead::{Aead, Payload};
        let c = self.cipher()?;
        let n = aes_gcm::Nonce::from(nonce);
        let mut out = nonce.to_vec();
        out.extend(
            c.encrypt(
                &n,
                Payload {
                    msg: plain,
                    aad: KEY_CONTEXT,
                },
            )
            .map_err(|_| Error::Keystore("seal failed".into()))?,
        );
        Ok(out)
    }
}

impl Keystore for AesKeystore {
    fn name(&self) -> &'static str {
        self.name
    }
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, Error> {
        let mut nonce = [0u8; 12];
        getrandom_fill(&mut nonce)?;
        self.seal_with_nonce(nonce, plain)
    }
    fn unseal(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
        use aes_gcm::aead::{Aead, Payload};
        let c = self.cipher()?;
        if blob.len() < 12 {
            return Err(Error::Keystore("sealed key is truncated".into()));
        }
        let (nonce, ct) = blob.split_at(12);
        let nonce =
            aes_gcm::Nonce::try_from(nonce).map_err(|_| Error::Keystore("bad nonce".into()))?;
        c.decrypt(
            &nonce,
            Payload {
                msg: ct,
                aad: KEY_CONTEXT,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| Error::Keystore("cipher: message authentication failed".into()))
    }
}

/// Random bytes from the OS for a GCM nonce (Go `crypto/rand`).
fn getrandom_fill(buf: &mut [u8]) -> Result<(), Error> {
    getrandom::fill(buf).map_err(|e| Error::Keystore(format!("random nonce: {e}")))
}

/// DPAPI, current-user scope (Go `DPAPIKeystore`).
#[cfg(windows)]
pub struct DpapiKeystore;

#[cfg(windows)]
impl Keystore for DpapiKeystore {
    fn name(&self) -> &'static str {
        "dpapi"
    }
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, Error> {
        dpapi::seal(plain)
    }
    fn unseal(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
        dpapi::unseal(blob)
    }
}

#[cfg(windows)]
pub mod dpapi {
    //! `CryptProtectData` / `CryptUnprotectData` exactly as Go calls them
    //! (I:internal/identity/keystore_windows.go:13-49).
    use super::*;
    use windows_sys::Win32::Foundation::{HLOCAL, LocalFree};
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    fn blob_of(b: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: b.len() as u32,
            pbData: if b.is_empty() {
                std::ptr::null_mut()
            } else {
                b.as_ptr() as *mut u8
            },
        }
    }

    fn take(out: CRYPT_INTEGER_BLOB) -> Zeroizing<Vec<u8>> {
        let v = if out.pbData.is_null() {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec()
        };
        unsafe {
            LocalFree(out.pbData as HLOCAL);
        }
        Zeroizing::new(v)
    }

    /// CryptUnprotectData, entropy `irori/key/1`, UI forbidden.
    pub fn unseal(blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
        let input = blob_of(blob);
        let entropy = blob_of(KEY_CONTEXT);
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(Error::Keystore(format!(
                "CryptUnprotectData: {}",
                std::io::Error::last_os_error()
            )));
        }
        Ok(take(out))
    }

    /// CryptProtectData, description `Irori key`, entropy `irori/key/1`.
    pub fn seal(plain: &[u8]) -> Result<Vec<u8>, Error> {
        let input = blob_of(plain);
        let entropy = blob_of(KEY_CONTEXT);
        let desc: Vec<u16> = "Irori key\0".encode_utf16().collect();
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let ok = unsafe {
            CryptProtectData(
                &input,
                desc.as_ptr(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(Error::Keystore(format!(
                "CryptProtectData: {}",
                std::io::Error::last_os_error()
            )));
        }
        Ok(take(out).to_vec())
    }

    /// Fixture sealing, kept under its M1 name.
    pub fn seal_for_tests(plain: &[u8]) -> Result<Vec<u8>, Error> {
        seal(plain)
    }
}

/// A wrapped secret as stored (Go `wrapped`). `Debug` never shows the blob,
/// which under the `file` keystore is the secret itself.
pub struct Wrapped {
    pub v: i64,
    pub keystore: String,
    blob: Zeroizing<Vec<u8>>,
}

impl fmt::Debug for Wrapped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Wrapped")
            .field("v", &self.v)
            .field("keystore", &self.keystore)
            .field("blob", &"(redacted)")
            .finish()
    }
}

#[derive(serde::Serialize)]
struct WrappedOut<'a> {
    v: i64,
    keystore: &'a str,
    #[serde(serialize_with = "ser_bytes")]
    blob: &'a [u8],
}

fn ser_bytes<S: serde::Serializer>(b: &&[u8], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_bytes(b)
}

/// Go's `wrapped` as JSON (`{v, keystore, blob}`), for files that nest one.
impl serde::Serialize for Wrapped {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        WrappedOut {
            v: self.v,
            keystore: &self.keystore,
            blob: &self.blob,
        }
        .serialize(s)
    }
}

impl Wrapped {
    /// Parse a wrapped file's JSON the way Go's `json.Unmarshal` reads it.
    pub fn parse(raw: &[u8]) -> Result<Self, Error> {
        let out = json::unmarshal(raw, &Wrapped::schema())
            .map_err(|_| Error::Unreadable("wrapped key".into()))?;
        Wrapped::from_out(&out).ok_or_else(|| Error::Unreadable("wrapped key".into()))
    }

    /// The Go `wrapped` struct as a lenient-decode schema, for files that
    /// nest one (`person.json`'s `secretKey`). `blob` is read as a string
    /// and base64-decoded by [`Wrapped::from_out`].
    pub(crate) fn schema() -> Schema {
        Schema::object(vec![
            ("v", Schema::Int(64)),
            ("keystore", Schema::String),
            ("blob", Schema::String),
        ])
    }

    /// A wrapped value decoded with [`Wrapped::schema`]; `None` where Go's
    /// `[]byte` decoding fails.
    pub(crate) fn from_out(out: &json::Out) -> Option<Self> {
        let blob = go_base64_decode(out.get_str("blob"))?;
        let v = match out.get("v") {
            Some(json::Out::Int(v)) => *v,
            _ => 0,
        };
        Some(Wrapped {
            v,
            keystore: out.get_str("keystore").to_string(),
            blob: Zeroizing::new(blob),
        })
    }

    /// Go `seal(ks, plain)`.
    pub fn seal(ks: &dyn Keystore, plain: &[u8]) -> Result<Self, Error> {
        let blob = ks
            .seal(plain)
            .map_err(|e| Error::Keystore(format!("identity: seal with {}: {e}", ks.name())))?;
        Ok(Wrapped {
            v: 1,
            keystore: ks.name().to_string(),
            blob: Zeroizing::new(blob),
        })
    }

    /// Go `unseal(ks, w)`, with Go's error texts.
    pub fn unseal(&self, ks: &dyn Keystore) -> Result<Zeroizing<Vec<u8>>, Error> {
        if self.keystore != ks.name() {
            return Err(Error::WrongKeystore {
                sealed: self.keystore.clone(),
                uses: ks.name().to_string(),
            });
        }
        ks.unseal(&self.blob).map_err(|e| match e {
            Error::KeystoreUnavailable(_) | Error::KekMissing { .. } => e,
            other => Error::Unseal {
                keystore: ks.name().to_string(),
                detail: other.to_string(),
            },
        })
    }

    /// The file bytes Go's `writeSealed` writes: `MarshalIndent`, two spaces.
    pub fn to_file_bytes(&self) -> Vec<u8> {
        json::marshal_indent(
            &WrappedOut {
                v: self.v,
                keystore: &self.keystore,
                blob: &self.blob,
            },
            "",
            "  ",
        )
        .expect("a wrapped secret always marshals")
    }

    /// Go `writeSealed`: `<path>.tmp` (0600) then rename, parent made 0700.
    pub fn write(&self, path: &Path) -> Result<(), Error> {
        crate::write_atomic(path, &self.to_file_bytes())
    }

    /// Read and parse a wrapped file.
    pub fn read(path: &Path) -> Result<Self, Error> {
        let raw =
            std::fs::read(path).map_err(|e| Error::Io(format!("read {}: {e}", path.display())))?;
        Wrapped::parse(&raw).map_err(|_| Error::Unreadable(path.display().to_string()))
    }
}

/// Keychain account name: `"kek-" + hex(sha256(path)[0:8])`
/// (I:internal/identity/keychain.go:60-75).
pub fn keychain_account(path: &str) -> String {
    use sha2::Digest;
    let h = sha2::Sha256::digest(path.as_bytes());
    format!("kek-{}", hex::encode(&h[..8]))
}

/// Go's `kekAccounts` for a POSIX data dir: the account of
/// `Clean(Abs(dir))` first, then the dir as given when that differs (older
/// builds hashed it as given). `cwd` is what `filepath.Abs` joins a relative
/// dir onto. Lexical: no symlink is resolved (identity.md §7.3 trap 4).
pub fn keychain_accounts(data_dir: &str, cwd: &str) -> Vec<String> {
    use gocompat::filepath::unix;
    accounts_of(&unix::clean(&unix::abs_from(data_dir, cwd)), data_dir)
}

/// The account of the canonical path, then of the dir as given when that
/// differs: the tail [`keychain_accounts`] and
/// [`crate::keychain::kek_accounts`] share.
pub(crate) fn accounts_of(canon: &str, data_dir: &str) -> Vec<String> {
    let mut out = vec![keychain_account(canon)];
    let legacy = keychain_account(data_dir);
    if legacy != out[0] {
        out.push(legacy);
    }
    out
}

/// The dev-only `IRORI_DEV_KEK_DIR` (architecture §5.2, D14): the data dir
/// a copied Mac data dir's Keychain item is named after. Read only in
/// debug builds; a release build (`debug_assertions` off) ignores it, so
/// no environment variable there changes which item unseals the keys.
pub fn dev_kek_dir() -> Option<String> {
    dev_kek_dir_of(std::env::var_os("IRORI_DEV_KEK_DIR"))
}

/// [`dev_kek_dir`] with the variable's value given.
pub fn dev_kek_dir_of(v: Option<std::ffi::OsString>) -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    v.map(|d| d.to_string_lossy().into_owned())
        .filter(|d| !d.is_empty())
}

#[cfg(target_os = "macos")]
pub mod keychain {
    //! The macOS keystore: AES-GCM under a KEK held in the login Keychain
    //! and reached through `/usr/bin/security`, as Go does (identity.md §5).
    //! Finding and first-run creation are [`crate::keychain::KeychainKek`]'s.
    use super::*;
    use crate::keychain::{KeychainKek, SecurityRunner};
    use std::sync::Arc;

    /// Go's `runSecurity`: stdout, or the exit code (-1 when it didn't run).
    fn run_security() -> SecurityRunner {
        Box::new(|args: &[&str]| {
            let out = std::process::Command::new("/usr/bin/security")
                .args(args)
                .stderr(std::process::Stdio::null())
                .output()
                .map_err(|e| (-1, format!("run /usr/bin/security: {e}")))?;
            match out.status.code() {
                Some(0) => Ok(out.stdout),
                Some(c) => Err((c, format!("exit status {c}"))),
                None => Err((-1, "security was killed".into())),
            }
        })
    }

    /// The `keychain` keystore for a data dir. In debug builds the dev-only
    /// `IRORI_DEV_KEK_DIR` names the item from the original data dir, so a
    /// copied Mac data dir still finds its key ([`dev_kek_dir`]).
    pub struct KeychainKeystore(AesKeystore);

    impl KeychainKeystore {
        pub fn new(data_dir: &Path) -> Self {
            let named = dev_kek_dir().unwrap_or_else(|| data_dir.to_string_lossy().into_owned());
            let accounts = crate::keychain::kek_accounts(&named);
            let kek = Arc::new(KeychainKek::with_accounts(
                data_dir,
                accounts,
                run_security(),
            ));
            KeychainKeystore(AesKeystore::new("keychain", Box::new(move || kek.get())))
        }
    }

    impl Keystore for KeychainKeystore {
        fn name(&self) -> &'static str {
            "keychain"
        }
        fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, Error> {
            self.0.seal(plain)
        }
        fn unseal(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
            self.0.unseal(blob)
        }
    }
}

/// Go's `aesKeystore.Unseal` with an explicit KEK.
pub fn aes_unseal(kek: &[u8], blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
    let k = Zeroizing::new(kek.to_vec());
    AesKeystore::new("keychain", Box::new(move || Ok(k.clone()))).unseal(blob)
}

/// Go's `aesKeystore.Seal` with a chosen nonce, for fixtures.
pub fn aes_seal_for_tests(kek: &[u8], nonce: [u8; 12], plain: &[u8]) -> Vec<u8> {
    let k = Zeroizing::new(kek.to_vec());
    AesKeystore::new("keychain", Box::new(move || Ok(k.clone())))
        .seal_with_nonce(nonce, plain)
        .expect("seal")
}

/// Seal `plain` with this OS's keystore the Go way and return the wrapped
/// file text. For fixtures; on macOS it seals under `kek` instead of the
/// Keychain.
pub fn wrap_for_tests(plain: &[u8], kek: Option<&[u8]>) -> Result<String, Error> {
    let ks = platform_keystore();
    let blob = match ks {
        #[cfg(windows)]
        "dpapi" => dpapi::seal(plain)?,
        "keychain" => aes_seal_for_tests(
            kek.ok_or_else(|| Error::Keystore("kek needed".into()))?,
            [7u8; 12],
            plain,
        ),
        _ => plain.to_vec(),
    };
    let w = Wrapped {
        v: 1,
        keystore: ks.to_string(),
        blob: Zeroizing::new(blob),
    };
    Ok(String::from_utf8(w.to_file_bytes()).expect("JSON is UTF-8"))
}
