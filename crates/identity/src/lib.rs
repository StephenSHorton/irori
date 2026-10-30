//! `irori-identity` (identity.md; I:internal/{identity,device,owner}).
//!
//! | Module | Go | What |
//! |---|---|---|
//! | [`keystore`], [`keychain`] | `keystore*.go`, `keychain.go` | wrapped secrets; DPAPI / Keychain / file keystores; the Keychain KEK, made on first run only |
//! | [`person`] | `identity.go`, `rotate.go`, `device` | [`Identity`]: `device.key`, `person.json`, epochs and rotation, every person-signed row builder, `EnsureSelfCert` |
//! | [`objects`] | `rows.go`, payload types | person-signed objects in Go field order, `Canonical`, `verifyPerson` |
//! | [`chain`] | `chain.go` | the private cert fold |
//! | [`membership`] | `membership.go` | the board membership fold (pure) |
//! | [`board`] | `board.go` | the board Verifier |
//! | [`boardsecret`] | `boardsecret.go` | board secrets and HKDF topics |
//! | [`owner`] | `owner` | the owner table |
//! | [`row`] | store's `Event`, `Verifier` | the row view the folds read |
//! | [`wire`] | `core` wiring | `store::Verifier` for [`Chain`] / [`BoardVerifier`], `store::Signer` for [`Identity`], [`Appender`] for `store::Store` |
//!
//! [`load`] is M1's read-only loader: it unseals `device.key` and checks
//! it against `device.json`, creating and rewriting nothing. The seed stays
//! inside [`DeviceIdentity`], whose `Debug` is redacted. Pairing is the
//! `pair` crate's.

pub mod board;
pub mod boardsecret;
pub mod chain;
pub mod keychain;
pub mod keystore;
pub mod kinds;
pub mod membership;
pub mod objects;
pub mod owner;
pub mod person;
pub mod row;
pub mod wire;

pub use board::{BoardDevice, BoardVerifier};
pub use boardsecret::{BOARD_SECRET_SIZE, BoardSecrets, BoardTopic, derive_board_topic};
pub use chain::{Chain, DeviceState, LearnError, USER_KEY_PREFIX, owner_only, user_key};
pub use membership::{
    BoardPerson, Conflict, Membership, MembershipKeys, MembershipRow, RowAt, fold_membership,
    valid_board_name, valid_board_tag,
};
pub use objects::*;
pub use person::{DEVICE_KEY_FILE, Identity, PERSON_FILE, Person};
pub use row::{Appender, CutClaim, Row, RowRef, Verifier, VerifyError, row_hash, signing_bytes};

use std::fmt;
use std::path::{Path, PathBuf};

use gocompat::json::{self, Schema};
use zeroize::Zeroizing;

pub use keystore::{
    AesKeystore, FileKeystore, KEY_CONTEXT, Keystore, Wrapped, aes_seal_for_tests, aes_unseal,
    default_keystore, dev_kek_dir, dev_kek_dir_of, keychain_account, keychain_accounts,
    platform_keystore, wrap_for_tests,
};
#[cfg(windows)]
pub use keystore::{DpapiKeystore, dpapi};

/// Identity errors. Texts follow Go's where Go has one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(String),
    #[error("{0} is unreadable")]
    Unreadable(String),
    /// Go `ErrWrongKeystore`.
    #[error(
        "identity: key sealed by a different keystore: sealed with {sealed:?}, this device uses {uses:?} (was the data dir copied from another machine?)"
    )]
    WrongKeystore { sealed: String, uses: String },
    #[error(
        "identity: unseal with {keystore} (was the data dir copied from another machine or user?): {detail}"
    )]
    Unseal { keystore: String, detail: String },
    /// Go `errKeystoreUnavailable`: the keystore could not produce its key,
    /// as opposed to a blob that does not open.
    #[error("identity: the keystore's key is unavailable: {0}")]
    KeystoreUnavailable(String),
    /// Go `ErrKEKMissing`, wrapped in `errKeystoreUnavailable` as Go does.
    #[error(
        "identity: the keystore's key is unavailable: identity: Irori's key is missing from the Keychain: {detail}"
    )]
    KekMissing { account: String, detail: String },
    #[error("{0}")]
    Keystore(String),
    #[error("{0}")]
    Invalid(String),
    /// Go `ErrStaleEpoch`.
    #[error("identity: person secret is not for a newer epoch")]
    StaleEpoch,
    /// Go `ErrNotThisKey`.
    #[error("identity: person secret does not match its key")]
    NotThisKey,
    /// A Go `fmt.Errorf("<context>: %w", err)`: the text gains the context,
    /// [`Error::root`] keeps the cause.
    #[error("{0}: {1}")]
    Context(String, Box<Error>),
}

impl Error {
    /// The innermost error under any [`Error::Context`] (Go `errors.Is`).
    pub fn root(&self) -> &Error {
        match self {
            Error::Context(_, e) => e.root(),
            e => e,
        }
    }

    pub(crate) fn context(self, ctx: &str) -> Error {
        Error::Context(ctx.to_string(), Box::new(self))
    }
}

/// `device.json` (I:internal/device/device.go:21-32).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct DeviceJson {
    pub id: String,
    pub name: String,
    /// Go `runtime.GOOS`: `windows`, `darwin`, `linux`.
    pub os: String,
    #[serde(rename = "personId", skip_serializing_if = "String::is_empty")]
    pub person_id: String,
    #[serde(rename = "devicePub", skip_serializing_if = "String::is_empty")]
    pub device_pub: String,
}

impl DeviceJson {
    /// Parse as Go's `json.Unmarshal` does (case-insensitive keys, last
    /// duplicate wins). Unreadable JSON or an empty `id` is an error, as in Go.
    pub fn parse(raw: &[u8]) -> Option<DeviceJson> {
        let schema = Schema::object(vec![
            ("id", Schema::String),
            ("name", Schema::String),
            ("os", Schema::String),
            ("personId", Schema::String),
            ("devicePub", Schema::String),
        ]);
        let o = json::unmarshal(raw, &schema).ok()?;
        let d = DeviceJson {
            id: o.get_str("id").into(),
            name: o.get_str("name").into(),
            os: o.get_str("os").into(),
            person_id: o.get_str("personId").into(),
            device_pub: o.get_str("devicePub").into(),
        };
        if d.id.is_empty() { None } else { Some(d) }
    }

    /// The bytes Go's `device.Save` writes: `MarshalIndent`, two spaces, no
    /// trailing newline.
    pub fn to_file_bytes(&self) -> Vec<u8> {
        json::marshal_indent(self, "", "  ").expect("device.json always marshals")
    }

    /// Go `device.Save`: `<dir>/device.json` (0600) replaced atomically,
    /// the dir made 0700 first.
    pub fn save(&self, dir: &Path) -> Result<(), Error> {
        write_atomic(&dir.join("device.json"), &self.to_file_bytes())
    }

    /// Go `device.LoadOrCreate`: read `<dir>/device.json`, or on first run
    /// make one with a random id, the host name and Go's GOOS.
    pub fn load_or_create(dir: &Path) -> Result<DeviceJson, Error> {
        let path = dir.join("device.json");
        match std::fs::read(&path) {
            Ok(raw) => DeviceJson::parse(&raw).ok_or_else(|| {
                Error::Invalid(format!(
                    "device: {} is unreadable; move it aside to make a new identity",
                    path.display()
                ))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut id = [0u8; 16];
                getrandom::fill(&mut id).map_err(|e| Error::Io(format!("random: {e}")))?;
                let d = DeviceJson {
                    id: hex::encode(id),
                    name: gethostname::gethostname().to_string_lossy().into_owned(),
                    os: goos().into(),
                    person_id: String::new(),
                    device_pub: String::new(),
                };
                d.save(dir)?;
                Ok(d)
            }
            Err(e) => Err(Error::Io(format!("read {}: {e}", path.display()))),
        }
    }
}

/// Go `runtime.GOOS` for this build.
pub fn goos() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    }
}

/// `<path>.tmp` with mode 0600, then rename over `path`; the parent is made
/// 0700 first (Go `writeSealed`, `device.Save`).
pub(crate) fn write_atomic(path: &Path, data: &[u8]) -> Result<(), Error> {
    let io = |e: std::io::Error| Error::Io(format!("write {}: {e}", path.display()));
    if let Some(dir) = path.parent() {
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            b.mode(0o700);
        }
        b.create(dir).map_err(io)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        use std::io::Write as _;
        let mut f = o.open(&tmp).map_err(io)?;
        f.write_all(data).map_err(io)?;
    }
    std::fs::rename(&tmp, path).map_err(io)
}

/// A device's identity: `device.json` plus the unsealed ed25519 seed.
pub struct DeviceIdentity {
    pub dir: PathBuf,
    pub device: DeviceJson,
    seed: Zeroizing<[u8; 32]>,
}

impl fmt::Debug for DeviceIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceIdentity")
            .field("dir", &self.dir)
            .field("device", &self.device)
            .field("seed", &"(redacted)")
            .finish()
    }
}

impl DeviceIdentity {
    /// The device signing key (row signatures, and the iroh key by D2).
    pub fn signing_key(&self) -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&self.seed)
    }

    /// The device public key.
    pub fn public_key(&self) -> [u8; 32] {
        self.signing_key().verifying_key().to_bytes()
    }

    /// The device public key as lowercase hex, as `device.json` and cert
    /// rows hold it.
    pub fn device_pub_hex(&self) -> String {
        hex::encode(self.public_key())
    }

    /// The raw seed, for building a transport key from it (the iroh
    /// `SecretKey`, D2). Never print or log it.
    pub fn expose_seed(&self) -> &[u8; 32] {
        &self.seed
    }

    /// Test and fixture constructor. Never used on real data.
    pub fn from_seed_for_tests(seed: [u8; 32]) -> Self {
        let mut id = DeviceIdentity {
            dir: PathBuf::new(),
            device: DeviceJson {
                id: hex::encode(&seed[..16]),
                name: "test".into(),
                os: goos().into(),
                person_id: String::new(),
                device_pub: String::new(),
            },
            seed: Zeroizing::new(seed),
        };
        id.device.device_pub = id.device_pub_hex();
        id
    }
}

/// Go `PersonIDOf(pub)`: hex of the first 16 bytes of sha256(pub).
pub fn person_id_of(public_key: &[u8; 32]) -> String {
    use sha2::Digest;
    let sum = sha2::Sha256::digest(public_key);
    hex::encode(&sum[..16])
}

fn short(s: &str) -> &str {
    &s[..s.len().min(12)]
}

/// Read a wrapped file and unseal it with this OS's keystore.
pub fn unseal_file(path: &Path, data_dir: &Path) -> Result<Zeroizing<Vec<u8>>, Error> {
    let w = Wrapped::read(path)?;
    let ks = default_keystore(data_dir);
    w.unseal(ks.as_ref())
}

/// Load `device.json` and unseal `device.key` from `dir`. Read-only: nothing
/// is created or rewritten.
pub fn load(dir: &Path) -> Result<DeviceIdentity, Error> {
    let dj = dir.join("device.json");
    let raw = std::fs::read(&dj).map_err(|e| Error::Io(format!("read {}: {e}", dj.display())))?;
    let device = DeviceJson::parse(&raw)
        .ok_or_else(|| Error::Unreadable(format!("device: {}", dj.display())))?;
    let kp = dir.join("device.key");
    let plain = unseal_file(&kp, dir)?;
    if plain.len() != 32 {
        return Err(Error::Invalid(format!(
            "{} holds {} bytes, want an ed25519 seed",
            kp.display(),
            plain.len()
        )));
    }
    let mut seed = Zeroizing::new([0u8; 32]);
    seed.copy_from_slice(&plain);
    let id = DeviceIdentity {
        dir: dir.to_path_buf(),
        device,
        seed,
    };
    let pub_hex = id.device_pub_hex();
    if !id.device.device_pub.is_empty() && id.device.device_pub != pub_hex {
        return Err(Error::Invalid(format!(
            "device.json names key {}… but device.key holds {}…; the data dir is mixed from two installs",
            short(&id.device.device_pub),
            short(&pub_hex)
        )));
    }
    Ok(id)
}
