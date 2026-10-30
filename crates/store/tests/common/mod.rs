//! Helpers shared by the store tests: Go's test helpers (openTest,
//! mustAppend, all, keySigner, devVerifier, v03DB) and the golden loader.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use sha2::{Digest, Sha256};
use store::{
    Clock, CutClaim, Error, ErrorKind, Event, MAX_MISSING_LIMIT, OpenOptions, Signer, Store,
    Timestamp, Vector, Verifier, signing_bytes,
};

pub fn clock() -> Arc<Clock> {
    Arc::new(Clock::new())
}

/// Go `openTest`: an unsigned private log at dir/irori.db.
pub fn open_test(dir: &Path, device: &str) -> Store {
    Store::open(dir.join("irori.db"), device, clock()).expect("open")
}

/// Go `mustAppend` as actor irori.
pub fn must_append<P: serde::Serialize + ?Sized>(s: &Store, kind: &str, payload: &P) -> Event {
    s.append("irori", kind, payload).expect("append")
}

/// Go `all`: every replicated row, by (device, seq).
pub fn all(s: &Store) -> Vec<Event> {
    s.missing(&Vector::new(), MAX_MISSING_LIMIT)
        .expect("missing")
}

pub fn ev(dev: &str, seq: i64, hlc: u64, kind: &str) -> Event {
    Event {
        device_id: dev.into(),
        seq,
        hlc: Timestamp(hlc),
        kind: kind.into(),
        ..Event::default()
    }
}

/// An ed25519 signer (Go `keySigner`).
#[derive(Clone)]
pub struct KeySigner(pub SigningKey);

impl KeySigner {
    pub fn random() -> KeySigner {
        let mut seed = [0u8; 32];
        getrandom_fill(&mut seed);
        KeySigner(SigningKey::from_bytes(&seed))
    }

    /// The golden exporter's key: seed sha256("irori-rust golden " + name).
    pub fn golden(name: &str) -> KeySigner {
        let seed: [u8; 32] = Sha256::digest(format!("irori-rust golden {name}")).into();
        KeySigner(SigningKey::from_bytes(&seed))
    }

    pub fn public(&self) -> VerifyingKey {
        self.0.verifying_key()
    }

    pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
        self.0.sign(msg).to_bytes().to_vec()
    }

    pub fn verifies(&self, ev: &Event) -> bool {
        verify_with(&self.public(), ev)
    }
}

pub fn verify_with(pub_key: &VerifyingKey, ev: &Event) -> bool {
    let Ok(sig) = <[u8; 64]>::try_from(ev.sig.as_slice()) else {
        return false;
    };
    pub_key
        .verify(&signing_bytes(ev), &Signature::from_bytes(&sig))
        .is_ok()
}

impl Signer for KeySigner {
    fn sign_row(&self, msg: &[u8]) -> store::Result<Vec<u8>> {
        Ok(self.sign(msg))
    }
}

/// Test randomness without another dependency: hash the time and a counter.
fn getrandom_fill(buf: &mut [u8]) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let h = Sha256::digest(format!("{t}/{n}/{:p}", buf.as_ptr()));
    buf.copy_from_slice(&h[..buf.len()]);
}

/// Go's devVerifier (board_test.go) and the golden exporter's goldVerifier:
/// certifies devices by key, answers Cut and CutClaims from what the test
/// sets, and records what it observes.
#[derive(Default)]
pub struct FakeVerifier {
    pub st: Mutex<FakeState>,
}

#[derive(Default)]
pub struct FakeState {
    pub pubs: BTreeMap<String, VerifyingKey>,
    pub cuts: BTreeMap<String, i64>,
    pub claims: Vec<CutClaim>,
    /// `"<dev>/<seq> <kind>"` per observed row.
    pub observed: Vec<String>,
    pub resets: usize,
}

impl FakeVerifier {
    pub fn new(pubs: &[(&str, VerifyingKey)]) -> Arc<FakeVerifier> {
        let v = FakeVerifier::default();
        {
            let mut s = v.st.lock().unwrap();
            for (d, k) in pubs {
                s.pubs.insert((*d).into(), *k);
            }
        }
        Arc::new(v)
    }

    pub fn observed_kinds(&self) -> Vec<String> {
        let s = self.st.lock().unwrap();
        s.observed
            .iter()
            .map(|o| o.split_once(' ').map_or(o.clone(), |(_, k)| k.to_string()))
            .collect()
    }
}

impl Verifier for FakeVerifier {
    fn verify_row(&self, ev: &Event) -> store::Result<()> {
        let pubk = self.st.lock().unwrap().pubs.get(&ev.device_id).copied();
        let Some(pubk) = pubk else {
            return Err(Error::wrap(ErrorKind::NoCert, &ev.device_id));
        };
        if !verify_with(&pubk, ev) {
            return Err(Error::wrap(
                ErrorKind::BadSig,
                format!("{}/{}", ev.device_id, ev.seq),
            ));
        }
        Ok(())
    }

    fn cut(&self, device_id: &str) -> Option<i64> {
        self.st.lock().unwrap().cuts.get(device_id).copied()
    }

    fn cut_claims(&self) -> Vec<CutClaim> {
        self.st.lock().unwrap().claims.clone()
    }

    fn observe(&self, ev: &Event) {
        self.st
            .lock()
            .unwrap()
            .observed
            .push(format!("{}/{} {}", ev.device_id, ev.seq, ev.kind));
    }

    fn reset(&self) {
        let mut s = self.st.lock().unwrap();
        s.observed.clear();
        s.resets += 1;
    }
}

pub fn opts(signer: Option<KeySigner>, verifier: Option<Arc<FakeVerifier>>) -> OpenOptions {
    OpenOptions {
        signer: signer.map(|s| Arc::new(s) as Arc<dyn Signer>),
        verifier: verifier.map(|v| v as Arc<dyn Verifier>),
        ..OpenOptions::default()
    }
}

/// testdata/golden/<rel>.
pub fn golden_path(rel: &str) -> PathBuf {
    manifest_dir().join("../../testdata/golden").join(rel)
}

pub fn golden(rel: &str) -> serde_json::Value {
    let raw = std::fs::read(golden_path(rel)).expect("golden file");
    serde_json::from_slice(&raw).expect("golden JSON")
}

/// A golden row (`gEvent`): hex payload and sig.
pub fn g_event(v: &serde_json::Value) -> Event {
    Event {
        device_id: v["deviceId"].as_str().unwrap().into(),
        seq: v["seq"].as_i64().unwrap(),
        hlc: Timestamp(v["hlc"].as_u64().unwrap()),
        actor: v["actor"].as_str().unwrap().into(),
        kind: v["kind"].as_str().unwrap().into(),
        payload: hex::decode(v["payload"].as_str().unwrap()).unwrap(),
        sig: hex::decode(v["sig"].as_str().unwrap()).unwrap(),
    }
}

/// An Event as the golden writes it.
pub fn to_g(ev: &Event) -> serde_json::Value {
    serde_json::json!({
        "deviceId": ev.device_id,
        "seq": ev.seq,
        "hlc": ev.hlc.0,
        "actor": ev.actor,
        "kind": ev.kind,
        "payload": hex::encode(&ev.payload),
        "sig": hex::encode(&ev.sig),
        "rowHash": store::row_hash(ev),
    })
}

pub fn to_gs(evs: &[Event]) -> serde_json::Value {
    serde_json::Value::Array(evs.iter().map(to_g).collect())
}

/// The v1 schema and the v0.3 rows of Go's `v03DB` (v04_test.go:215-248),
/// written with a WAL left behind. Returns the rows.
pub fn v03_db(path: &Path, device: &str) -> Vec<Event> {
    let c = rusqlite::Connection::open(path).unwrap();
    let _: String = c
        .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
        .unwrap();
    c.execute_batch("CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL DEFAULT (unixepoch()))").unwrap();
    c.execute_batch(V1_SQL).unwrap();
    c.execute_batch("INSERT INTO schema_migrations (version) VALUES (1)")
        .unwrap();
    let rows: Vec<(&str, &str, String)> = vec![
        (
            store::kinds::DEVICE_STARTED,
            "irori",
            r#"{"name":"pc","os":"windows","version":"0.3.2"}"#.into(),
        ),
        (
            store::kinds::AGENT_SEEN,
            "irori",
            format!(r#"{{"key":"{device}/s1","name":"alpha"}}"#),
        ),
        (
            store::kinds::POLICY_SET,
            "user",
            r#"{"mode":"allowlist","allow":["alpha"],"block":[],"paused":false}"#.into(),
        ),
        (
            store::kinds::MESSAGE_SENT,
            "",
            r#"{"id":"msg_1","text":"hi"}"#.into(),
        ),
        (
            store::kinds::MEMBER_JOINED,
            "irori",
            format!(r#"{{"key":"{device}/grok-1","name":"grokky"}}"#),
        ),
        (
            store::kinds::POLICY_SET,
            "user",
            r#"{"mode":"everyone","allow":[],"block":["beta"],"paused":true}"#.into(),
        ),
        (
            store::kinds::TASK_CREATED,
            "",
            r#"{"taskId":"t1","title":"x"}"#.into(),
        ),
    ];
    let agent = format!("agent:{device}/s1");
    let mut out = Vec::new();
    for (i, (kind, actor, payload)) in rows.into_iter().enumerate() {
        let seq = i as i64 + 1;
        let ev = Event {
            device_id: device.into(),
            seq,
            hlc: Timestamp(((seq * 1000) as u64) << 16),
            actor: if actor.is_empty() {
                agent.clone()
            } else {
                actor.into()
            },
            kind: kind.into(),
            payload: payload.into_bytes(),
            sig: Vec::new(),
        };
        c.execute(
            "INSERT INTO events (device_id, seq, hlc, actor, kind, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![ev.device_id, ev.seq, ev.hlc.as_i64(), ev.actor, ev.kind, ev.payload],
        )
        .unwrap();
        out.push(ev);
    }
    out
}

/// Go's migrations[0].sql, as the golden exporter recorded it.
pub const V1_SQL: &str = concat!(
    "CREATE TABLE events (\n",
    "\t\tdevice_id TEXT    NOT NULL,\n",
    "\t\tseq       INTEGER NOT NULL,\n",
    "\t\thlc       INTEGER NOT NULL,\n",
    "\t\tactor     TEXT    NOT NULL,\n",
    "\t\tkind      TEXT    NOT NULL,\n",
    "\t\tpayload   BLOB,\n",
    "\t\tPRIMARY KEY (device_id, seq)\n",
    "\t) WITHOUT ROWID;\n",
    "\tCREATE INDEX events_hlc ON events (hlc);\n",
    "\tCREATE INDEX events_kind ON events (kind, hlc);",
);

pub fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
