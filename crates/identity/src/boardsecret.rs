//! A board's secrets on this device (I:internal/identity/boardsecret.go;
//! identity.md §4.6): 32 random bytes per board epoch, sealed by the
//! device's keystore in `boards/<boardId>.secrets`, and the topic and
//! sealing key each derives.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use gocompat::json::{self, Out, Schema};
use hkdf::Hkdf;
use serde::Serialize;
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::Error;
use crate::keystore::{Keystore, Wrapped, go_base64_decode};

/// A board secret's length.
pub const BOARD_SECRET_SIZE: usize = 32;

/// One epoch's board topic: its name and its sealing key.
#[derive(Clone, PartialEq, Eq)]
pub struct BoardTopic {
    pub epoch: i64,
    pub topic: String,
    pub key: [u8; 32],
}

impl std::fmt::Debug for BoardTopic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoardTopic")
            .field("epoch", &self.epoch)
            .field("topic", &self.topic)
            .field("key", &"(redacted)")
            .finish()
    }
}

/// Go `DeriveBoardTopic`: HKDF-SHA256, salt `irori/board/1`, info the
/// board id's 16 bytes then the epoch as u64 big-endian, 48 bytes: the
/// sealing key, then the topic suffix.
pub fn derive_board_topic(board_id: &str, epoch: i64, secret: &[u8]) -> Result<BoardTopic, Error> {
    let id = match hex::decode(board_id) {
        Ok(b) if b.len() == 16 => b,
        _ => {
            return Err(Error::Invalid(format!(
                "identity: {} is not a board id",
                gocompat::quote::quote(board_id)
            )));
        }
    };
    if epoch < 1 || secret.len() != BOARD_SECRET_SIZE {
        return Err(Error::Invalid(
            "identity: a board topic needs an epoch and a 32-byte secret".into(),
        ));
    }
    let mut info = id;
    info.extend_from_slice(&(epoch as u64).to_be_bytes());
    let mut out = Zeroizing::new([0u8; 48]);
    Hkdf::<Sha256>::new(Some(b"irori/board/1"), secret)
        .expand(&info, &mut out[..])
        .map_err(|_| Error::Invalid("identity: hkdf".into()))?;
    let mut key = [0u8; 32];
    key.copy_from_slice(&out[..32]);
    Ok(BoardTopic {
        epoch,
        topic: format!("irori/board/1/{}", hex::encode(&out[32..])),
        key,
    })
}

/// What the sealed file holds: epoch -> maker device id -> secret.
#[derive(Default, Serialize)]
struct SecretFile {
    epochs: BTreeMap<i64, BTreeMap<String, SecretBytes>>,
    settled: i64,
}

/// A Go `[]byte`: base64 in JSON. Zeroized on drop.
#[derive(Clone, PartialEq, Eq)]
struct SecretBytes(Zeroizing<Vec<u8>>);

impl Serialize for SecretBytes {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(&self.0)
    }
}

type MakerRule = Box<dyn Fn(&str) -> bool + Send + Sync>;

struct Inner {
    f: SecretFile,
    maker: Option<MakerRule>,
}

/// One board's secrets on this device.
pub struct BoardSecrets {
    path: PathBuf,
    board_id: String,
    ks: Box<dyn Keystore + Send + Sync>,
    inner: Mutex<Inner>,
}

fn parse_secret_file(plain: &[u8]) -> Option<SecretFile> {
    let schema = Schema::object(vec![
        (
            "epochs",
            Schema::Map(Box::new(Schema::Map(Box::new(Schema::String)))),
        ),
        ("settled", Schema::Int(64)),
    ]);
    let o = json::unmarshal(plain, &schema).ok()?;
    let mut f = SecretFile {
        settled: match o.get("settled") {
            Some(Out::Int(n)) => *n,
            _ => 0,
        },
        ..SecretFile::default()
    };
    if let Some(Out::Map(Some(m))) = o.get("epochs") {
        for (k, v) in m {
            // Go parses an int map key with strconv.ParseInt(k, 10, 64).
            let e: i64 = go_parse_int(k)?;
            let mut makers = BTreeMap::new();
            if let Out::Map(Some(mm)) = v {
                for (dev, b64) in mm {
                    let b = go_base64_decode(b64.str())?;
                    makers.insert(dev.clone(), SecretBytes(Zeroizing::new(b)));
                }
            }
            f.epochs.insert(e, makers);
        }
    }
    Some(f)
}

/// `strconv.ParseInt(s, 10, 64)`: an optional sign, then decimal digits.
fn go_parse_int(s: &str) -> Option<i64> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

impl BoardSecrets {
    /// Go `OpenBoardSecrets`: none yet when the file doesn't exist.
    pub fn open(
        path: &Path,
        board_id: &str,
        ks: Box<dyn Keystore + Send + Sync>,
    ) -> Result<BoardSecrets, Error> {
        let mut f = SecretFile::default();
        match std::fs::read(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::Io(format!("read {}: {e}", path.display()))),
            Ok(raw) => {
                let w = Wrapped::parse(&raw)
                    .map_err(|_| Error::Unreadable(format!("identity: {}", path.display())))?;
                let plain = w.unseal(ks.as_ref())?;
                f = parse_secret_file(&plain)
                    .ok_or_else(|| Error::Unreadable(format!("identity: {}", path.display())))?;
            }
        }
        Ok(BoardSecrets {
            path: path.to_path_buf(),
            board_id: board_id.to_string(),
            ks,
            inner: Mutex::new(Inner { f, maker: None }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Which makers' secrets count (`None`: all).
    pub fn set_makers(&self, counts: Option<MakerRule>) {
        self.lock().maker = counts;
    }

    /// The newest epoch held (0 for none).
    pub fn epoch(&self) -> i64 {
        newest(&self.lock().f)
    }

    /// Make this device's secret for `epoch`; a no-op when `maker` already
    /// made one.
    pub fn mint(&self, epoch: i64, maker: &str) -> Result<(), Error> {
        let mut b = Zeroizing::new([0u8; BOARD_SECRET_SIZE]);
        getrandom::fill(&mut b[..]).map_err(|e| Error::Io(format!("random: {e}")))?;
        let mut g = self.lock();
        if g.f
            .epochs
            .get(&epoch)
            .is_some_and(|m| m.contains_key(maker))
        {
            return Ok(());
        }
        self.add_locked(&mut g, epoch, maker, &b[..])
    }

    /// Keep a secret another member device made for `epoch`.
    pub fn add(&self, epoch: i64, maker: &str, secret: &[u8]) -> Result<(), Error> {
        if secret.len() != BOARD_SECRET_SIZE {
            return Err(Error::Invalid(
                "identity: a board secret is 32 bytes".into(),
            ));
        }
        let mut g = self.lock();
        if g.f
            .epochs
            .get(&epoch)
            .and_then(|m| m.get(maker))
            .is_some_and(|old| old.0.as_slice() == secret)
        {
            return Ok(());
        }
        self.add_locked(&mut g, epoch, maker, secret)
    }

    fn add_locked(
        &self,
        g: &mut Inner,
        epoch: i64,
        maker: &str,
        secret: &[u8],
    ) -> Result<(), Error> {
        if epoch < 1 || maker.is_empty() {
            return Err(Error::Invalid(
                "identity: a board secret needs an epoch and its maker".into(),
            ));
        }
        g.f.epochs.entry(epoch).or_default().insert(
            maker.to_string(),
            SecretBytes(Zeroizing::new(secret.to_vec())),
        );
        // Only the newest epoch and the one before it stay.
        let cur = newest(&g.f);
        let prev = prev_of(&g.f, cur);
        g.f.epochs.retain(|e, _| !(*e < cur && *e != prev));
        self.save(&g.f)
    }

    /// `epoch`'s secret and its maker: of the makers that count, the lowest
    /// device id's.
    pub fn secret(&self, epoch: i64) -> Option<(Zeroizing<Vec<u8>>, String)> {
        let g = self.lock();
        secret_of(&g, epoch)
    }

    /// The topics kept open: the newest epoch's, and the one before until
    /// [`settle`](Self::settle) passes it. Epochs with no counting maker are
    /// skipped.
    pub fn topics(&self) -> Result<Vec<BoardTopic>, Error> {
        let g = self.lock();
        let cur = newest(&g.f);
        if cur == 0 {
            return Ok(Vec::new());
        }
        let mut epochs = vec![cur];
        let prev = prev_of(&g.f, cur);
        if prev > 0 && g.f.settled < cur {
            epochs.push(prev);
        }
        let mut out = Vec::new();
        for e in epochs {
            let Some((secret, _)) = secret_of(&g, e) else {
                continue;
            };
            out.push(derive_board_topic(&self.board_id, e, &secret)?);
        }
        Ok(out)
    }

    /// Every member device has said hello with `epoch`: the topics before
    /// it close.
    pub fn settle(&self, epoch: i64) -> Result<(), Error> {
        let mut g = self.lock();
        if epoch <= g.f.settled {
            return Ok(());
        }
        g.f.settled = epoch;
        g.f.epochs.retain(|e, _| *e >= epoch);
        self.save(&g.f)
    }

    fn save(&self, f: &SecretFile) -> Result<(), Error> {
        let plain =
            Zeroizing::new(json::marshal(f).map_err(|e| Error::Invalid(format!("identity: {e}")))?);
        Wrapped::seal(self.ks.as_ref(), &plain)?.write(&self.path)
    }

    /// The plaintext the sealed file holds, for golden tests. Never log it.
    #[doc(hidden)]
    pub fn plaintext_for_tests(&self) -> Vec<u8> {
        json::marshal(&self.lock().f).expect("secrets marshal")
    }
}

fn newest(f: &SecretFile) -> i64 {
    f.epochs.keys().copied().fold(0, i64::max)
}

fn prev_of(f: &SecretFile, cur: i64) -> i64 {
    f.epochs
        .keys()
        .copied()
        .filter(|e| *e < cur)
        .fold(0, i64::max)
}

fn secret_of(g: &Inner, epoch: i64) -> Option<(Zeroizing<Vec<u8>>, String)> {
    let makers = g.f.epochs.get(&epoch)?;
    let (dev, b) = makers
        .iter()
        .find(|(m, _)| g.maker.as_ref().is_none_or(|f| f(m)))?;
    Some((Zeroizing::new(b.0.to_vec()), dev.clone()))
}
