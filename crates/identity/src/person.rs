//! This device's keys and person (I:internal/identity/identity.go,
//! rotate.go, board.go:107-137, membership.go:129-178; identity.md §2.1,
//! §4.1-4.2).
//!
//! [`Identity::load`] reads, or on first start makes, `device.key` and
//! `person.json`, sealing new secrets with the keystore, and records
//! `personId` and `devicePub` in `device.json`. Every person-signed row
//! this device writes is built here. Keys never leave: callers get
//! signatures, and the person seed only through [`Identity::person_seed`]
//! for the mesh's topic and frames.
//!
//! Rotation writes `person.json` first and swaps the key in memory only
//! after the write succeeds, so a crash mid-rotation loses no key (trap
//! 12); [`Identity::sign_epoch`] refuses to sign for an epoch the key has
//! left.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use ed25519_dalek::{Signer as _, SigningKey};
use gocompat::json::{self, Out, Schema};
use gocompat::time::GoTime;
use serde::Serialize;
use zeroize::Zeroizing;

use crate::chain::Chain;
use crate::keystore::{Keystore, Wrapped};
use crate::kinds;
use crate::objects::*;
use crate::row::Appender;
use crate::{DeviceJson, Error};

/// File names in the data dir.
pub const PERSON_FILE: &str = "person.json";
pub const DEVICE_KEY_FILE: &str = "device.key";

/// The public view of `person.json` (GUI JSON).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Person {
    #[serde(rename = "personId")]
    pub id: String,
    pub name: String,
    pub epoch: i64,
    #[serde(rename = "rootDeviceId")]
    pub root_device_id: String,
    /// The current epoch's key, hex.
    #[serde(rename = "personPub")]
    pub pub_hex: String,
}

/// One epoch's person key.
struct EpochKey {
    epoch: i64,
    key: SigningKey,
}

/// `person.json` as written: field order `personId, name, epoch,
/// rootDeviceId, personPub, secretKey, older?`; `prev` is read, never
/// written.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PersonFileOut<'a> {
    person_id: &'a str,
    name: &'a str,
    epoch: i64,
    root_device_id: &'a str,
    person_pub: &'a str,
    secret_key: Wrapped,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    older: Vec<PrevKeyOut>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PrevKeyOut {
    epoch: i64,
    person_pub: String,
    secret_key: Wrapped,
}

struct Inner {
    device: DeviceJson,
    person: Person,
    person_key: SigningKey,
    /// The epochs before the current one, oldest first.
    older: Vec<EpochKey>,
}

/// This device's keys and person (Go `Identity`), a store Signer.
pub struct Identity {
    data_dir: PathBuf,
    device_key: SigningKey,
    ks: Box<dyn Keystore + Send + Sync>,
    inner: Mutex<Inner>,
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let g = self.lock();
        f.debug_struct("Identity")
            .field("data_dir", &self.data_dir)
            .field("device", &g.device)
            .field("person", &g.person)
            .field("keys", &"(redacted)")
            .finish()
    }
}

fn random_key() -> Result<SigningKey, Error> {
    let mut seed = Zeroizing::new([0u8; 32]);
    getrandom::fill(&mut seed[..]).map_err(|e| Error::Io(format!("random: {e}")))?;
    Ok(SigningKey::from_bytes(&seed))
}

fn seed_key(seed: &[u8]) -> Option<SigningKey> {
    let s: [u8; 32] = seed.try_into().ok()?;
    Some(SigningKey::from_bytes(&s))
}

/// Go `loadOrCreateSeed`: unseal a 32-byte secret, or make and seal one.
fn load_or_create_seed(path: &Path, ks: &dyn Keystore) -> Result<Zeroizing<[u8; 32]>, Error> {
    match std::fs::read(path) {
        Ok(raw) => {
            let w =
                Wrapped::parse(&raw).map_err(|_| Error::Unreadable(path.display().to_string()))?;
            let plain = w.unseal(ks)?;
            if plain.len() != 32 {
                return Err(Error::Invalid(format!(
                    "{} holds {} bytes, want an ed25519 seed",
                    path.display(),
                    plain.len()
                )));
            }
            let mut out = Zeroizing::new([0u8; 32]);
            out.copy_from_slice(&plain);
            Ok(out)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut seed = Zeroizing::new([0u8; 32]);
            getrandom::fill(&mut seed[..]).map_err(|e| Error::Io(format!("random: {e}")))?;
            Wrapped::seal(ks, &seed[..])?.write(path)?;
            Ok(seed)
        }
        Err(e) => Err(Error::Io(format!("read {}: {e}", path.display()))),
    }
}

fn unseal_person_key(
    ks: &dyn Keystore,
    w: &Wrapped,
    pub_hex: &str,
    path: &Path,
) -> Result<SigningKey, Error> {
    let seed = w.unseal(ks)?;
    let key = seed_key(&seed)
        .ok_or_else(|| Error::Invalid(format!("{} holds a bad person key", path.display())))?;
    if crate::objects::pub_hex(&key) != pub_hex {
        return Err(Error::Invalid(format!(
            "{}: personPub does not match the secret",
            path.display()
        )));
    }
    Ok(key)
}

fn person_file_schema() -> Schema {
    let prev = || {
        Schema::object(vec![
            ("epoch", Schema::Int(64)),
            ("personPub", Schema::String),
            ("secretKey", Wrapped::schema()),
        ])
    };
    Schema::object(vec![
        ("personId", Schema::String),
        ("name", Schema::String),
        ("epoch", Schema::Int(64)),
        ("rootDeviceId", Schema::String),
        ("personPub", Schema::String),
        ("secretKey", Wrapped::schema()),
        ("older", Schema::Slice(Box::new(prev()))),
        ("prev", Schema::Ptr(Box::new(prev()))),
    ])
}

type LoadedPerson = (Person, SigningKey, Vec<EpochKey>);

/// Go `loadOrCreatePerson`.
fn load_or_create_person(
    path: &Path,
    device_id: &str,
    ks: &dyn Keystore,
) -> Result<LoadedPerson, Error> {
    let unreadable = || Error::Unreadable(path.display().to_string());
    match std::fs::read(path) {
        Ok(raw) => {
            let o = json::unmarshal(&raw, &person_file_schema()).map_err(|_| unreadable())?;
            let wrapped = |o: &Out| o.get("secretKey").and_then(Wrapped::from_out);
            let p = Person {
                id: o.get_str("personId").to_string(),
                name: o.get_str("name").to_string(),
                epoch: get_i64(&o, "epoch"),
                root_device_id: o.get_str("rootDeviceId").to_string(),
                pub_hex: o.get_str("personPub").to_string(),
            };
            if p.id.is_empty() {
                return Err(unreadable());
            }
            // Every nested []byte must decode, or Go's Unmarshal fails whole.
            let w = wrapped(&o).ok_or_else(unreadable)?;
            let decode_all = |items: &[&Out]| -> Result<Vec<(i64, String, Wrapped)>, Error> {
                items
                    .iter()
                    .map(|e| {
                        let w = wrapped(e).ok_or_else(unreadable)?;
                        Ok((get_i64(e, "epoch"), e.get_str("personPub").to_string(), w))
                    })
                    .collect()
            };
            let olds: Vec<&Out> = o
                .get("older")
                .map(|s| s.items().iter().collect())
                .unwrap_or_default();
            let prev: Vec<&Out> = match o.get("prev") {
                Some(Out::Ptr(Some(p))) => vec![p.as_ref()],
                _ => Vec::new(),
            };
            let olds_w = decode_all(&olds)?;
            let prev_w = decode_all(&prev)?;
            let key = unseal_person_key(ks, &w, &p.pub_hex, path)?;
            // `prev` (an earlier build's one previous epoch) counts only
            // when `older` is empty.
            let older_w = if olds_w.is_empty() { prev_w } else { olds_w };
            let mut older = Vec::new();
            for (epoch, pub_hex, w) in &older_w {
                older.push(EpochKey {
                    epoch: *epoch,
                    key: unseal_person_key(ks, w, pub_hex, path)?,
                });
            }
            Ok((p, key, older))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = random_key()?;
            let p = Person {
                id: crate::person_id_of(&key.verifying_key().to_bytes()),
                name: default_name(),
                epoch: 1,
                root_device_id: device_id.to_string(),
                pub_hex: crate::objects::pub_hex(&key),
            };
            write_person(path, ks, &p, &key, &[])?;
            Ok((p, key, Vec::new()))
        }
        Err(e) => Err(Error::Io(format!("read {}: {e}", path.display()))),
    }
}

/// Go `writePerson`: seal the key and the older epochs' and write
/// `person.json` atomically (MarshalIndent, two spaces).
fn write_person(
    path: &Path,
    ks: &dyn Keystore,
    p: &Person,
    key: &SigningKey,
    older: &[EpochKey],
) -> Result<(), Error> {
    let mut olds = Vec::new();
    for o in older {
        olds.push(PrevKeyOut {
            epoch: o.epoch,
            person_pub: crate::objects::pub_hex(&o.key),
            secret_key: Wrapped::seal(ks, o.key.as_bytes())?,
        });
    }
    let f = PersonFileOut {
        person_id: &p.id,
        name: &p.name,
        epoch: p.epoch,
        root_device_id: &p.root_device_id,
        person_pub: &p.pub_hex,
        secret_key: Wrapped::seal(ks, key.as_bytes())?,
        older: olds,
    };
    let out = json::marshal_indent(&f, "", "  ").map_err(|e| Error::Invalid(e.to_string()))?;
    crate::write_atomic(path, &out)
}

/// `writePerson` with keys given as seeds, for golden tests and fixtures.
#[doc(hidden)]
pub fn write_person_file_for_tests(
    path: &Path,
    ks: &dyn Keystore,
    p: &Person,
    seed: [u8; 32],
    older: &[(i64, [u8; 32])],
) -> Result<(), Error> {
    let older: Vec<EpochKey> = older
        .iter()
        .map(|(epoch, s)| EpochKey {
            epoch: *epoch,
            key: SigningKey::from_bytes(s),
        })
        .collect();
    write_person(path, ks, p, &SigningKey::from_bytes(&seed), &older)
}

/// Go `defaultName`: the OS user name without a Windows domain, `"you"`
/// when there is none. On Windows it is the logon account's name
/// (`GetUserNameW`, the name `user.Current` reports after the domain);
/// elsewhere `$USER`, which Go's `user.Current` falls back to and which
/// matches the passwd name on a normal login.
pub fn default_name() -> String {
    let name = os_user_name();
    let name = match name.rfind(['\\', '/']) {
        Some(i) => name[i + 1..].to_string(),
        None => name,
    };
    if name.is_empty() { "you".into() } else { name }
}

#[cfg(windows)]
fn os_user_name() -> String {
    use windows_sys::Win32::System::WindowsProgramming::GetUserNameW;
    let mut buf = vec![0u16; 257];
    let mut n = buf.len() as u32;
    // SAFETY: buf holds n u16s; on success n counts the name and its NUL.
    let ok = unsafe { GetUserNameW(buf.as_mut_ptr(), &mut n) };
    if ok == 0 || n == 0 {
        return std::env::var("USERNAME").unwrap_or_default();
    }
    String::from_utf16_lossy(&buf[..(n as usize - 1).min(buf.len())])
}

#[cfg(not(windows))]
fn os_user_name() -> String {
    std::env::var("USER").unwrap_or_default()
}

fn short(s: &str) -> &str {
    &s[..s.len().min(12)]
}

impl Identity {
    /// Go `Load(dataDir, dev, ks)`: read or make `device.key` and
    /// `person.json`, then record `personId` and `devicePub` in
    /// `device.json` when they differ. `ks` `None` is this OS's keystore.
    pub fn load(
        data_dir: &Path,
        dev: DeviceJson,
        ks: Option<Box<dyn Keystore + Send + Sync>>,
    ) -> Result<Identity, Error> {
        let ks = ks.unwrap_or_else(|| crate::keystore::default_keystore(data_dir));
        let seed = load_or_create_seed(&data_dir.join(DEVICE_KEY_FILE), ks.as_ref())
            .map_err(|e| e.context("device key"))?;
        let device_key = SigningKey::from_bytes(&seed);
        let pub_hex = crate::objects::pub_hex(&device_key);
        if !dev.device_pub.is_empty() && dev.device_pub != pub_hex {
            return Err(Error::Invalid(format!(
                "identity: device.json names key {}… but device.key holds {}…; the data dir is mixed from two installs",
                short(&dev.device_pub),
                short(&pub_hex)
            )));
        }
        let (person, person_key, older) =
            load_or_create_person(&data_dir.join(PERSON_FILE), &dev.id, ks.as_ref())
                .map_err(|e| e.context("person"))?;
        let mut dev = dev;
        if dev.device_pub != pub_hex || dev.person_id != person.id {
            dev.device_pub = pub_hex;
            dev.person_id = person.id.clone();
            dev.save(data_dir)?;
        }
        Ok(Identity {
            data_dir: data_dir.to_path_buf(),
            device_key,
            ks,
            inner: Mutex::new(Inner {
                device: dev,
                person,
                person_key,
                older,
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The keystore this identity seals with (board secrets use it too).
    pub fn keystore(&self) -> &(dyn Keystore + Send + Sync) {
        self.ks.as_ref()
    }

    /// Go `LoadOrCreateSecret(name)`: a 32-byte random secret in the data
    /// dir, sealed like `device.key` (the home server's `turn.key`).
    pub fn load_or_create_secret(&self, name: &str) -> Result<Zeroizing<[u8; 32]>, Error> {
        load_or_create_seed(&self.data_dir.join(name), self.ks.as_ref())
    }

    /// Sign a row's signing bytes with the device key (store Signer).
    pub fn sign_row(&self, msg: &[u8]) -> Vec<u8> {
        self.device_key.sign(msg).to_bytes().to_vec()
    }

    /// Sign arbitrary bytes with the device key.
    pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
        self.sign_row(msg)
    }

    /// The device's public key.
    pub fn device_pub(&self) -> [u8; 32] {
        self.device_key.verifying_key().to_bytes()
    }

    /// The device seed, for the iroh key (D2). Never print or log it.
    pub fn expose_device_seed(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.device_key.to_bytes())
    }

    /// The person with its current epoch and key.
    pub fn current(&self) -> Person {
        self.lock().person.clone()
    }

    /// This device's `device.json` view, its personId current after pairing.
    pub fn self_device(&self) -> DeviceJson {
        self.lock().device.clone()
    }

    /// This device's id (fixed after load).
    pub fn device_id(&self) -> String {
        self.lock().device.id.clone()
    }

    /// The person seed of `epoch`, current or kept older. It never goes
    /// into a row; never print or log it.
    pub fn person_seed(&self, epoch: i64) -> Option<Zeroizing<[u8; 32]>> {
        let g = self.lock();
        if epoch == g.person.epoch {
            return Some(Zeroizing::new(g.person_key.to_bytes()));
        }
        g.older
            .iter()
            .find(|o| o.epoch == epoch)
            .map(|o| Zeroizing::new(o.key.to_bytes()))
    }

    /// The newest epoch kept from before the current one (0 for none).
    pub fn prev_epoch(&self) -> i64 {
        self.lock().older.last().map(|o| o.epoch).unwrap_or(0)
    }

    /// The epochs kept from before the current one, oldest first.
    pub fn older_epochs(&self) -> Vec<i64> {
        self.lock().older.iter().map(|o| o.epoch).collect()
    }

    /// Sign Canonical(v) with whatever key is current (profile only).
    fn sign_person<T: Serialize>(&self, v: &T) -> Result<(String, String), Error> {
        let msg = canonical(v).map_err(|e| Error::Invalid(e.to_string()))?;
        let g = self.lock();
        Ok((
            g.person.pub_hex.clone(),
            hex::encode(g.person_key.sign(&msg).to_bytes()),
        ))
    }

    /// Sign Canonical(v) with the current key, only while it is still
    /// `epoch`'s.
    pub fn sign_epoch<T: Serialize>(&self, epoch: i64, v: &T) -> Result<String, Error> {
        let msg = canonical(v).map_err(|e| Error::Invalid(e.to_string()))?;
        let g = self.lock();
        if g.person.epoch != epoch {
            return Err(Error::Invalid(format!(
                "identity: the person key moved to epoch {} while signing for {}",
                g.person.epoch, epoch
            )));
        }
        Ok(hex::encode(g.person_key.sign(&msg).to_bytes()))
    }

    /// A `person.device.added` payload certifying a device.
    pub fn cert_for(
        &self,
        device_id: &str,
        device_pub: &str,
        label: &str,
        role: &str,
        at: GoTime,
    ) -> Result<DeviceAdded, Error> {
        let cur = self.current();
        let c = Cert {
            person_id: cur.id,
            device_id: device_id.into(),
            device_pub: device_pub.into(),
            label: label.into(),
            role: role.into(),
            at: at.utc(),
        };
        let sig = self.sign_epoch(cur.epoch, &c)?;
        Ok(DeviceAdded {
            cert: c,
            person_pub: cur.pub_hex,
            person_sig: sig,
            person_epoch: cur.epoch,
        })
    }

    /// A `person.device.revoked` payload.
    pub fn revoke_of(
        &self,
        device_id: &str,
        cut: i64,
        cut_hash: &str,
        reason: &str,
        at: GoTime,
    ) -> Result<DeviceRevoked, Error> {
        let cur = self.current();
        let r = Revoke {
            person_id: cur.id,
            device_id: device_id.into(),
            cut,
            cut_hash: cut_hash.into(),
            at: at.utc(),
            reason: reason.into(),
        };
        let sig = self.sign_epoch(cur.epoch, &r)?;
        Ok(DeviceRevoked {
            revoke: r,
            person_pub: cur.pub_hex,
            person_sig: sig,
            person_epoch: cur.epoch,
        })
    }

    /// The `person.profile` payload.
    pub fn profile_row(&self) -> Result<Profile, Error> {
        let cur = self.current();
        let b = ProfileBody {
            person_id: cur.id,
            name: cur.name,
            root_device_id: cur.root_device_id,
            person_pub: cur.pub_hex,
        };
        let (_, sig) = self.sign_person(&b)?;
        Ok(Profile {
            person_id: b.person_id,
            name: b.name,
            root_device_id: b.root_device_id,
            person_pub: b.person_pub,
            person_sig: sig,
        })
    }

    /// Go `EnsureSelfCert`: on the person's root only, write the
    /// self-written `person.device.added` when the chain holds no cert for
    /// this device (the cert first: it carries its own verifying key), then
    /// `person.profile` when none counts. `app` appends as this device and
    /// folds the row into `ch` (the store's Verifier).
    pub fn ensure_self_cert(
        &self,
        app: &dyn Appender,
        ch: &Chain,
        role: &str,
    ) -> Result<(), Error> {
        let dev = self.self_device();
        if dev.id != self.current().root_device_id {
            return Ok(());
        }
        let append = |kind: &str, raw: Vec<u8>| {
            app.append("irori", kind, &raw)
                .map(|_| ())
                .map_err(Error::Io)
        };
        if ch.cert(&dev.id).is_none() {
            let label = if dev.name.is_empty() {
                dev.id.clone()
            } else {
                dev.name.clone()
            };
            let add = self.cert_for(
                &dev.id,
                &hex::encode(self.device_pub()),
                &label,
                role,
                GoTime::from_system(std::time::SystemTime::now()),
            )?;
            append(
                kinds::PERSON_DEVICE_ADDED,
                json::marshal(&add).map_err(|e| Error::Invalid(e.to_string()))?,
            )?;
            if ch.cert(&dev.id).is_none() {
                return Err(Error::Invalid(
                    "identity: the self-cert row did not certify this device".into(),
                ));
            }
        }
        if !ch.has_profile() {
            let prof = self.profile_row()?;
            append(
                kinds::PERSON_PROFILE,
                json::marshal(&prof).map_err(|e| Error::Invalid(e.to_string()))?,
            )?;
        }
        Ok(())
    }

    /// Go `Rotate`: a new person key for the next epoch; the old one is
    /// kept as an older epoch. `prevSig` is the old key over the rotation,
    /// signed before the swap. The caller appends the row.
    pub fn rotate(&self, at: GoTime) -> Result<KeyRotated, Error> {
        let key = random_key()?;
        let mut g = self.lock();
        let next = g.person.epoch + 1;
        let mut r = KeyRotated {
            person_id: g.person.id.clone(),
            epoch: next,
            person_pub: crate::objects::pub_hex(&key),
            at: at.utc(),
            by: g.device.id.clone(),
            prev_sig: String::new(),
        };
        let msg = canonical(&r.body()).map_err(|e| Error::Invalid(e.to_string()))?;
        r.prev_sig = hex::encode(g.person_key.sign(&msg).to_bytes());
        self.adopt_locked(&mut g, next, key)?;
        Ok(r)
    }

    /// Go `AdoptSecret`: the person secret of a newer epoch, which must
    /// derive the rotation row's key.
    pub fn adopt_secret(&self, epoch: i64, seed: &[u8], pub_hex: &str) -> Result<(), Error> {
        let key = key_for(seed, pub_hex)?;
        let mut g = self.lock();
        if epoch <= g.person.epoch {
            return Err(Error::StaleEpoch);
        }
        self.adopt_locked(&mut g, epoch, key)
    }

    /// Go `ReplaceSecret`: take another device's key for the current
    /// epoch (two rotations to one epoch); the losing key is dropped.
    pub fn replace_secret(&self, epoch: i64, seed: &[u8], pub_hex: &str) -> Result<(), Error> {
        let key = key_for(seed, pub_hex)?;
        let mut g = self.lock();
        if epoch != g.person.epoch {
            return Err(Error::StaleEpoch);
        }
        if pub_hex == g.person.pub_hex {
            return Ok(());
        }
        let mut p = g.person.clone();
        p.pub_hex = pub_hex.to_string();
        write_person(
            &self.data_dir.join(PERSON_FILE),
            self.ks.as_ref(),
            &p,
            &key,
            &g.older,
        )?;
        g.person.pub_hex = p.pub_hex;
        g.person_key = key;
        Ok(())
    }

    /// Go `Forget`: drop kept keys of epochs below `below`; writes only when
    /// something changed.
    pub fn forget(&self, below: i64) -> Result<(), Error> {
        let mut g = self.lock();
        if g.older.iter().all(|o| o.epoch >= below) {
            return Ok(());
        }
        let keep: Vec<EpochKey> = g
            .older
            .iter()
            .filter(|o| o.epoch >= below)
            .map(|o| EpochKey {
                epoch: o.epoch,
                key: o.key.clone(),
            })
            .collect();
        write_person(
            &self.data_dir.join(PERSON_FILE),
            self.ks.as_ref(),
            &g.person,
            &g.person_key,
            &keep,
        )?;
        g.older = keep;
        Ok(())
    }

    /// Go `AdoptPerson`: replace this device's person with the one pairing
    /// handed over; `person.json` gets no older epochs and `device.json`'s
    /// personId follows. The caller re-pins the Chain.
    pub fn adopt_person(&self, p: &Person, seed: &[u8]) -> Result<(), Error> {
        let key = seed_key(seed).ok_or_else(|| {
            Error::Invalid(format!("identity: person secret is {} bytes", seed.len()))
        })?;
        if p.id.is_empty() || p.root_device_id.is_empty() || p.epoch < 1 {
            return Err(Error::Invalid("identity: incomplete person".into()));
        }
        if crate::objects::pub_hex(&key) != p.pub_hex {
            return Err(Error::NotThisKey);
        }
        let mut g = self.lock();
        write_person(
            &self.data_dir.join(PERSON_FILE),
            self.ks.as_ref(),
            p,
            &key,
            &[],
        )?;
        let mut dev = g.device.clone();
        dev.person_id = p.id.clone();
        dev.save(&self.data_dir)?;
        g.device.person_id = dev.person_id;
        g.person = p.clone();
        g.person_key = key;
        g.older = Vec::new();
        Ok(())
    }

    /// Go `adoptLocked`: `key` becomes `epoch`'s; the current one joins the
    /// older epochs; memory changes only after person.json is written.
    fn adopt_locked(&self, g: &mut Inner, epoch: i64, key: SigningKey) -> Result<(), Error> {
        let mut older: Vec<EpochKey> = g
            .older
            .iter()
            .map(|o| EpochKey {
                epoch: o.epoch,
                key: o.key.clone(),
            })
            .collect();
        older.push(EpochKey {
            epoch: g.person.epoch,
            key: g.person_key.clone(),
        });
        let mut p = g.person.clone();
        p.epoch = epoch;
        p.pub_hex = crate::objects::pub_hex(&key);
        write_person(
            &self.data_dir.join(PERSON_FILE),
            self.ks.as_ref(),
            &p,
            &key,
            &older,
        )?;
        g.person.epoch = p.epoch;
        g.person.pub_hex = p.pub_hex;
        g.person_key = key;
        g.older = older;
        Ok(())
    }

    /// This person pinned on a board this device creates, with this device
    /// as its board root.
    pub fn board_pin(&self) -> MemberPin {
        let g = self.lock();
        MemberPin {
            person_id: g.person.id.clone(),
            person_pub: g.person.pub_hex.clone(),
            root_device_id: g.device.id.clone(),
            person_epoch: g.person.epoch,
        }
    }

    /// A `member.device.added` payload for `epoch` (the board's newest for
    /// this person).
    pub fn member_cert_for(
        &self,
        epoch: i64,
        device_id: &str,
        device_pub: &str,
        role: &str,
        at: GoTime,
    ) -> Result<MemberDeviceAdded, Error> {
        let c = MemberCert {
            person_id: self.current().id,
            device_id: device_id.into(),
            device_pub: device_pub.into(),
            role: role.into(),
            person_epoch: epoch,
            at: at.utc(),
        };
        let sig = self.sign_epoch(epoch, &c)?;
        Ok(MemberDeviceAdded {
            cert: c,
            person_sig: sig,
        })
    }

    /// A `member.device.revoked` payload.
    pub fn member_revoke_of(
        &self,
        epoch: i64,
        device_id: &str,
        board_cut: i64,
        cut_hash: &str,
        at: GoTime,
    ) -> Result<MemberDeviceRevoked, Error> {
        let r = MemberRevoke {
            person_id: self.current().id,
            device_id: device_id.into(),
            board_cut,
            board_cut_hash: cut_hash.into(),
            person_epoch: epoch,
            at: at.utc(),
        };
        let sig = self.sign_epoch(epoch, &r)?;
        Ok(MemberDeviceRevoked {
            revoke: r,
            person_sig: sig,
        })
    }

    /// A `member.profile` payload.
    pub fn member_profile_row(&self, epoch: i64) -> Result<MemberProfile, Error> {
        let cur = self.current();
        let b = MemberProfileBody {
            person_id: cur.id,
            name: cur.name,
            person_epoch: epoch,
        };
        let sig = self.sign_epoch(epoch, &b)?;
        Ok(MemberProfile {
            person_id: b.person_id,
            name: b.name,
            person_epoch: b.person_epoch,
            person_sig: sig,
        })
    }

    /// A `board.created` payload, signed for `pin.person_epoch`.
    pub fn board_created_row(
        &self,
        board_id: &str,
        name: &str,
        tag: &str,
        pin: &MemberPin,
        at: GoTime,
    ) -> Result<BoardCreated, Error> {
        let mut p = BoardCreated {
            board_id: board_id.into(),
            name: name.into(),
            tag: tag.into(),
            creator: pin.clone(),
            at: at.utc(),
            person_sig: String::new(),
        };
        p.person_sig = self.sign_epoch(pin.person_epoch, &p)?;
        Ok(p)
    }

    /// A `board.renamed` payload.
    pub fn board_renamed_row(
        &self,
        epoch: i64,
        name: &str,
        at: GoTime,
    ) -> Result<BoardRenamed, Error> {
        let mut p = BoardRenamed {
            name: name.into(),
            by: self.current().id,
            at: at.utc(),
            person_epoch: epoch,
            person_sig: String::new(),
        };
        p.person_sig = self.sign_epoch(epoch, &p)?;
        Ok(p)
    }

    /// A `board.member.added` payload pinning `pin`.
    pub fn board_member_added_row(
        &self,
        epoch: i64,
        pin: &MemberPin,
        name: &str,
        role: &str,
        at: GoTime,
    ) -> Result<BoardMemberAdded, Error> {
        let mut p = BoardMemberAdded {
            person_id: pin.person_id.clone(),
            person_pub: pin.person_pub.clone(),
            root_device_id: pin.root_device_id.clone(),
            person_epoch: pin.person_epoch,
            name: name.into(),
            role: role.into(),
            by: self.current().id,
            by_epoch: epoch,
            at: at.utc(),
            person_sig: String::new(),
        };
        p.person_sig = self.sign_epoch(epoch, &p)?;
        Ok(p)
    }

    /// A `board.member.role` payload.
    pub fn board_member_role_row(
        &self,
        epoch: i64,
        person_id: &str,
        role: &str,
        at: GoTime,
    ) -> Result<BoardMemberRole, Error> {
        let mut p = BoardMemberRole {
            person_id: person_id.into(),
            role: role.into(),
            by: self.current().id,
            at: at.utc(),
            person_epoch: epoch,
            person_sig: String::new(),
        };
        p.person_sig = self.sign_epoch(epoch, &p)?;
        Ok(p)
    }

    /// A `board.member.removed` payload (`cut` `None` encodes `null`).
    pub fn board_member_removed_row(
        &self,
        epoch: i64,
        person_id: &str,
        reason: &str,
        cut: Option<BTreeMap<String, i64>>,
        at: GoTime,
    ) -> Result<BoardMemberRemoved, Error> {
        let mut p = BoardMemberRemoved {
            person_id: person_id.into(),
            by: self.current().id,
            reason: reason.into(),
            cut,
            at: at.utc(),
            person_epoch: epoch,
            person_sig: String::new(),
        };
        p.person_sig = self.sign_epoch(epoch, &p)?;
        Ok(p)
    }

    /// A `board.member.left` payload for this person.
    pub fn board_member_left_row(
        &self,
        epoch: i64,
        cut: Option<BTreeMap<String, i64>>,
        at: GoTime,
    ) -> Result<BoardMemberLeft, Error> {
        let mut p = BoardMemberLeft {
            person_id: self.current().id,
            cut,
            at: at.utc(),
            person_epoch: epoch,
            person_sig: String::new(),
        };
        p.person_sig = self.sign_epoch(epoch, &p)?;
        Ok(p)
    }

    /// Test and fixture constructor: an identity with these seeds whose
    /// files live in `data_dir` under the file keystore. Writes nothing.
    pub fn from_seeds_for_tests(
        data_dir: &Path,
        device: DeviceJson,
        device_seed: [u8; 32],
        person: Person,
        person_seed: [u8; 32],
    ) -> Identity {
        Identity {
            data_dir: data_dir.to_path_buf(),
            device_key: SigningKey::from_bytes(&device_seed),
            ks: Box::new(crate::keystore::FileKeystore),
            inner: Mutex::new(Inner {
                device,
                person,
                person_key: SigningKey::from_bytes(&person_seed),
                older: Vec::new(),
            }),
        }
    }
}

/// Go `keyFor`: a seed that must derive `pub_hex`.
fn key_for(seed: &[u8], pub_hex: &str) -> Result<SigningKey, Error> {
    let key = seed_key(seed).ok_or_else(|| {
        Error::Invalid(format!("identity: person secret is {} bytes", seed.len()))
    })?;
    if crate::objects::pub_hex(&key) != pub_hex {
        return Err(Error::Invalid(
            "identity: person secret does not match its rotation row".into(),
        ));
    }
    Ok(key)
}
