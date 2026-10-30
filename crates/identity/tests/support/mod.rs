//! Test helpers shared by the ported Go tests.
//!
//! [`MemStore`] keeps the name of the in-memory stand-in these tests were
//! first written against, but it is now the real `store::Store` on a temp
//! dir, opened with the folds through `irori_identity::wire`: verify,
//! revocation cuts, HLC order, gaps, forks and cut-hash claims, the capped
//! vector and reopening are the store's own (store.md §3.3-3.7).
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use ed25519_dalek::SigningKey;
use irori_identity::*;
use serde::Serialize;

pub fn rand_key() -> SigningKey {
    let mut s = [0u8; 32];
    getrandom::fill(&mut s).unwrap();
    SigningKey::from_bytes(&s)
}

pub fn pubh(k: &SigningKey) -> String {
    pub_hex(k)
}

pub fn sign_hex<T: Serialize>(k: &SigningKey, v: &T) -> String {
    sign_person(k, v).unwrap()
}

pub fn to_json<T: Serialize>(v: &T) -> Vec<u8> {
    gocompat::json::marshal(v).unwrap()
}

pub fn unix(secs: i64) -> gocompat::time::GoTime {
    gocompat::time::GoTime::unix(secs, 0)
}

pub fn now() -> gocompat::time::GoTime {
    gocompat::time::GoTime::from_system(std::time::SystemTime::now())
}

/// Every permutation of `0..n` (Go `perm`).
pub fn perm(n: usize, f: &mut dyn FnMut(&[usize])) {
    fn rec(k: usize, idx: &mut Vec<usize>, f: &mut dyn FnMut(&[usize])) {
        if k == idx.len() {
            f(idx);
            return;
        }
        for i in k..idx.len() {
            idx.swap(k, i);
            rec(k + 1, idx, f);
            idx.swap(k, i);
        }
    }
    let mut idx: Vec<usize> = (0..n).collect();
    rec(0, &mut idx, f);
}

/// `n` seeded shuffles of `0..len` (a fixed xorshift, so runs repeat).
pub fn shuffles(n: usize, len: usize) -> Vec<Vec<usize>> {
    let mut x: u64 = 0x9e3779b97f4a7c15;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    (0..n)
        .map(|_| {
            let mut idx: Vec<usize> = (0..len).collect();
            for i in (1..len).rev() {
                let j = (next() % (i as u64 + 1)) as usize;
                idx.swap(i, j);
            }
            idx
        })
        .collect()
}

/// Store errors the tests match on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreErr {
    NoCert,
    BadSig,
    BoardActor,
    PastCut,
    Unsigned,
    Fork,
    HlcOrder,
    Gap,
    Ahead,
    Restoring,
    /// Any other store error, by its text.
    Other(String),
}

impl From<store::Error> for StoreErr {
    fn from(e: store::Error) -> Self {
        use store::ErrorKind as K;
        match e.kind() {
            K::NoCert => StoreErr::NoCert,
            K::BadSig => StoreErr::BadSig,
            K::BoardActor => StoreErr::BoardActor,
            K::PastCut => StoreErr::PastCut,
            K::Unsigned => StoreErr::Unsigned,
            K::Fork => StoreErr::Fork,
            K::HlcOrder => StoreErr::HlcOrder,
            K::Gap => StoreErr::Gap,
            K::Ahead => StoreErr::Ahead,
            K::Restoring => StoreErr::Restoring,
            _ => StoreErr::Other(e.to_string()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Fork {
    pub device_id: String,
    pub seq: i64,
    pub reason: &'static str,
    pub other_hash: String,
}

type SignFn = Box<dyn Fn(&[u8]) -> Vec<u8> + Send + Sync>;

struct FnSigner(SignFn);

impl store::Signer for FnSigner {
    fn sign_row(&self, msg: &[u8]) -> store::Result<Vec<u8>> {
        Ok((self.0)(msg))
    }
}

/// A real `store::Store` in a temp dir, with the small API the ported Go
/// tests call (rows as identity [`Row`]s, errors as [`StoreErr`]).
pub struct MemStore {
    pub self_id: String,
    board: bool,
    signer: Option<Arc<dyn store::Signer>>,
    path: std::path::PathBuf,
    st: Mutex<store::Store>,
    _dir: tempfile::TempDir,
}

impl MemStore {
    pub fn new(
        self_id: &str,
        verifier: Arc<dyn store::Verifier>,
        signer: Option<SignFn>,
        board: bool,
    ) -> MemStore {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(if board { "board.db" } else { "irori.db" });
        let signer = signer.map(|f| Arc::new(FnSigner(f)) as Arc<dyn store::Signer>);
        let st = Self::open(&path, self_id, verifier, signer.clone(), board);
        MemStore {
            self_id: self_id.into(),
            board,
            signer,
            path,
            st: Mutex::new(st),
            _dir: dir,
        }
    }

    fn open(
        path: &Path,
        self_id: &str,
        verifier: Arc<dyn store::Verifier>,
        signer: Option<Arc<dyn store::Signer>>,
        board: bool,
    ) -> store::Store {
        store::Store::open_with(
            path,
            self_id,
            Arc::new(store::Clock::new()),
            store::OpenOptions {
                signer,
                verifier: Some(verifier),
                board,
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn st(&self) -> std::sync::MutexGuard<'_, store::Store> {
        self.st.lock().unwrap()
    }

    /// Store `Insert` of one row from another device.
    pub fn insert(&self, ev: &Row) -> Result<(), StoreErr> {
        self.st()
            .insert(&store::Event::from(ev))
            .map_err(StoreErr::from)
    }

    /// Store `InsertBatch`: how many rows were applied, and the first
    /// refusal.
    pub fn insert_batch(&self, rows: &[Row]) -> (usize, Result<(), StoreErr>) {
        let evs: Vec<store::Event> = rows.iter().map(store::Event::from).collect();
        let (n, r) = self.st().insert_batch(&evs);
        (n, r.map_err(StoreErr::from))
    }

    /// Store `HoldAppends`.
    pub fn hold_appends(&self, seq: i64) {
        self.st().hold_appends(seq);
    }

    /// Store `Append`, its error kept.
    pub fn try_append(&self, actor: &str, kind: &str, payload: Vec<u8>) -> Result<Row, StoreErr> {
        self.st()
            .append(actor, kind, &gocompat::json::RawJson(payload))
            .map(Row::from)
            .map_err(StoreErr::from)
    }

    /// Append a row as this device (store `Append`).
    pub fn append_raw(&self, actor: &str, kind: &str, payload: Vec<u8>) -> Row {
        self.st()
            .append(actor, kind, &gocompat::json::RawJson(payload))
            .unwrap()
            .into()
    }

    pub fn append<T: Serialize>(&self, actor: &str, kind: &str, payload: &T) -> Row {
        self.append_raw(actor, kind, to_json(payload))
    }

    /// The store vector, each other device capped at its cut.
    pub fn vector(&self) -> BTreeMap<String, i64> {
        self.st().vector().unwrap()
    }

    /// Rows this store has that a holder of `have` lacks.
    pub fn missing(&self, have: &BTreeMap<String, i64>) -> Vec<Row> {
        let evs = self.st().missing(have, store::MAX_MISSING_LIMIT).unwrap();
        evs.into_iter().map(Row::from).collect()
    }

    /// Rows whose kind is `kind_prefix` (exact, or a prefix ending in
    /// `.`); every row for "".
    pub fn list(&self, kind_prefix: &str) -> Vec<Row> {
        let f = store::Filter {
            kinds: if kind_prefix.is_empty() {
                Vec::new()
            } else {
                vec![kind_prefix.to_string()]
            },
            limit: store::MAX_LIST_LIMIT,
            ..Default::default()
        };
        let evs = self.st().list(&f).unwrap();
        evs.into_iter().map(Row::from).collect()
    }

    pub fn refused(&self) -> Vec<Row> {
        let evs = self.st().refused(store::MAX_LIST_LIMIT).unwrap();
        evs.into_iter().map(Row::from).collect()
    }

    pub fn forks(&self) -> Vec<Fork> {
        self.st()
            .forks()
            .unwrap()
            .into_iter()
            .map(|f| Fork {
                device_id: f.other.device_id,
                seq: f.other.seq,
                reason: match f.reason.as_str() {
                    store::FORK_TWO_ROWS => "two rows",
                    store::FORK_CUT_HASH => "cut hash",
                    r => panic!("unknown fork reason {r:?}"),
                },
                other_hash: f.other_hash,
            })
            .collect()
    }

    /// Reopen the log with a fresh Verifier: the store folds the held
    /// identity rows, then the refused revokes, then applies the cuts
    /// (store.md §3.7).
    pub fn reopen(&self, verifier: Arc<dyn store::Verifier>) {
        let mut st = self.st();
        st.close().unwrap();
        *st = Self::open(
            &self.path,
            &self.self_id,
            verifier,
            self.signer.clone(),
            self.board,
        );
    }

    /// Close the log and open an empty one in its place, with a fresh
    /// Verifier: the data dir restored without its database. (A new file
    /// beside the old one; Windows won't delete a file still mapped.)
    pub fn reopen_empty(&mut self, verifier: Arc<dyn store::Verifier>) {
        let path = self.path.with_file_name("restored.db");
        let mut st = self.st.lock().unwrap();
        st.close().unwrap();
        *st = Self::open(
            &path,
            &self.self_id,
            verifier,
            self.signer.clone(),
            self.board,
        );
        drop(st);
        self.path = path;
    }

    /// Store `ReloadVerifier`.
    pub fn reload(&self) {
        self.st().reload_verifier();
    }
}

impl Appender for MemStore {
    fn append(&self, actor: &str, kind: &str, payload: &[u8]) -> Result<Row, String> {
        Ok(self.append_raw(actor, kind, payload.to_vec()))
    }
}

/// One device of a person: identity, chain and log (Go test `node`).
pub struct Node {
    pub id: Arc<Identity>,
    pub chain: Arc<Chain>,
    pub st: MemStore,
}

impl Node {
    /// Go `newNode(t, dir, p)`: a device; with `join`, a copy of that
    /// node's person.json first, so the root stays the joined person's.
    pub fn new(dir: &Path, join: Option<&Node>) -> Node {
        let dev = DeviceJson::load_or_create(dir).unwrap();
        if let Some(p) = join {
            std::fs::copy(p.id.data_dir().join(PERSON_FILE), dir.join(PERSON_FILE)).unwrap();
        }
        let id = Arc::new(Identity::load(dir, dev, Some(Box::new(FileKeystore))).unwrap());
        let cur = id.current();
        let chain = Arc::new(Chain::new(&cur.id, &cur.root_device_id, &id.device_id()));
        let signer_id = id.clone();
        let st = MemStore::new(
            &id.device_id(),
            chain.clone(),
            Some(Box::new(move |m: &[u8]| signer_id.sign_row(m))),
            false,
        );
        id.ensure_self_cert(&st, &chain, ROLE_DESKTOP).unwrap();
        Node { id, chain, st }
    }

    pub fn dev(&self) -> String {
        self.id.device_id()
    }

    /// This node writes a cert for `o`.
    pub fn certify(&self, o: &Node) -> Row {
        let add = self
            .id
            .cert_for(
                &o.dev(),
                &hex::encode(o.id.device_pub()),
                "dev",
                ROLE_DESKTOP,
                now(),
            )
            .unwrap();
        self.st.append("irori", kinds::PERSON_DEVICE_ADDED, &add)
    }

    pub fn revoke(&self, dev: &str, cut: i64) -> Row {
        self.revoke_hash(dev, cut, "")
    }

    pub fn revoke_hash(&self, dev: &str, cut: i64, hash: &str) -> Row {
        let r = self.id.revoke_of(dev, cut, hash, "test", now()).unwrap();
        self.st.append("irori", kinds::PERSON_DEVICE_REVOKED, &r)
    }

    /// Reopen the log with a new Chain (a restart).
    pub fn restart(&mut self) {
        let cur = self.id.current();
        self.chain = Arc::new(Chain::new(&cur.id, &cur.root_device_id, &self.dev()));
        self.st.reopen(self.chain.clone());
    }
}

/// Go `copyRows`: every row src holds that dst lacks, person rows learned
/// first (retried while a cert they need is still to come); the first
/// refusal is returned.
pub fn copy_rows(src: &Node, dst: &Node) -> Result<(), StoreErr> {
    let evs = src.st.missing(&dst.st.vector());
    let mut person: Vec<&Row> = evs
        .iter()
        .filter(|r| kinds::is_person_kind(&r.kind))
        .collect();
    let mut progress = true;
    while progress {
        progress = false;
        let mut left = Vec::new();
        for ev in person {
            if dst.chain.learn(ev.as_ref()).is_err() {
                left.push(ev);
            } else {
                progress = true;
            }
        }
        person = left;
    }
    for ev in &evs {
        dst.st.insert(ev)?;
    }
    Ok(())
}
