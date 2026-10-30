//! The coord_test.go harness (coord.md §6): a fixed clock at
//! 2026-10-26T12:00Z, a store on dev-a whose HLC reads it, a roster of
//! alice, bob and carol (idle), and the person `user:dev-a`.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use coord::roster::{AgentCard, Agents};
use coord::{Coord, ErrorKind, Options};
use gocompat::time::GoTime;
use store::{Event, Filter, Store};

pub const ALICE: &str = "dev-a/alice-session";
pub const BOB: &str = "dev-a/bob-session";
pub const CAROL: &str = "dev-a/carol-session";
pub const USER: &str = "user:dev-a";
pub const REPO_A: &str = "remote:github.com/a/b";
pub const REPO_B: &str = "root:4b825dc642cb6eb9a060e54bf8d69288fbee4904";

pub fn fixed_now() -> GoTime {
    GoTime::parse_rfc3339("2026-10-26T12:00:00Z").unwrap()
}

pub fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

pub fn mins(n: u64) -> Duration {
    Duration::from_secs(60 * n)
}

/// `t.Add(d)`.
pub fn plus(t: GoTime, d: Duration) -> GoTime {
    let n = i128::from(t.secs) * 1_000_000_000 + i128::from(t.nanos) + d.as_nanos() as i128;
    GoTime::unix(
        n.div_euclid(1_000_000_000) as i64,
        n.rem_euclid(1_000_000_000) as i64,
    )
}

/// A settable clock.
#[derive(Clone)]
pub struct Clk(pub Arc<Mutex<GoTime>>);

impl Clk {
    pub fn now(&self) -> GoTime {
        *self.0.lock().unwrap()
    }
    pub fn add(&self, d: Duration) {
        let mut t = self.0.lock().unwrap();
        *t = plus(*t, d);
    }
    pub fn func(&self) -> Arc<dyn Fn() -> GoTime + Send + Sync> {
        let c = self.clone();
        Arc::new(move || c.now())
    }
}

/// Key -> status; `set(key, "")` drops the card.
#[derive(Default)]
pub struct TestRoster(pub Mutex<BTreeMap<String, String>>);

impl TestRoster {
    pub fn new(keys: &[&str]) -> Arc<TestRoster> {
        let r = TestRoster::default();
        for k in keys {
            r.0.lock().unwrap().insert(k.to_string(), "idle".into());
        }
        Arc::new(r)
    }
    pub fn set(&self, key: &str, status: &str) {
        let mut m = self.0.lock().unwrap();
        if status.is_empty() {
            m.remove(key);
        } else {
            m.insert(key.into(), status.into());
        }
    }
}

impl Agents for TestRoster {
    fn agents(&self) -> Vec<AgentCard> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|(k, s)| AgentCard {
                key: k.clone(),
                status: s.clone(),
                ..AgentCard::default()
            })
            .collect()
    }
}

pub type KeyFn = Arc<dyn Fn(&str) -> bool + Send + Sync>;

pub struct Env {
    pub _dir: tempfile::TempDir,
    pub st: Arc<Store>,
    pub clk: Clk,
    pub ros: Arc<TestRoster>,
    pub c: Coord,
    pub owner: Option<KeyFn>,
}

impl Env {
    pub fn new() -> Env {
        Env::on("dev-a")
    }

    /// The harness with the store appending as `dev`.
    pub fn on(dev: &str) -> Env {
        Env::with(dev, None)
    }

    pub fn with(dev: &str, owner: Option<KeyFn>) -> Env {
        let clk = Clk(Arc::new(Mutex::new(fixed_now())));
        let dir = tempfile::tempdir().unwrap();
        let c2 = clk.clone();
        let hc = hlc::Clock::with_now(Box::new(move || c2.now().to_system()));
        let st = Arc::new(Store::open(dir.path().join("irori.db"), dev, Arc::new(hc)).unwrap());
        let ros = TestRoster::new(&[ALICE, BOB, CAROL]);
        let mut e = Env {
            _dir: dir,
            st,
            clk,
            ros,
            c: Coord::from_rows(dev, &[], None, Options::default()),
            owner,
        };
        e.c = e.reopen();
        e
    }

    pub fn opts(&self) -> Options {
        Options {
            now: Some(self.clk.func()),
            is_owner: self.owner.clone(),
            ..Options::default()
        }
    }

    /// New over the same store (a replay).
    pub fn reopen(&self) -> Coord {
        self.reopen_with(self.opts())
    }

    pub fn reopen_with(&self, o: Options) -> Coord {
        let r: Arc<dyn Agents> = self.ros.clone();
        Coord::new(self.st.clone(), Some(r), o).unwrap()
    }

    pub fn rows(&self, prefix: &str) -> Vec<Event> {
        self.st
            .list(&Filter {
                kinds: vec![prefix.into()],
                limit: store::MAX_LIST_LIMIT,
                ..Filter::default()
            })
            .unwrap()
    }

    /// Inserts a row as another device (not folded).
    pub fn insert<P: serde::Serialize>(
        &self,
        dev: &str,
        seq: i64,
        at: u64,
        actor: &str,
        kind: &str,
        payload: &P,
    ) -> Event {
        let ev = Event {
            device_id: dev.into(),
            seq,
            hlc: hlc::Timestamp(at),
            actor: actor.into(),
            kind: kind.into(),
            payload: gocompat::json::marshal(payload).unwrap(),
            sig: Vec::new(),
        };
        self.st.insert(&ev).unwrap();
        ev
    }

    /// A row another device wrote, inserted and folded live as Follow does.
    pub fn remote<P: serde::Serialize>(
        &self,
        dev: &str,
        seq: i64,
        at: u64,
        actor: &str,
        kind: &str,
        payload: &P,
    ) -> Event {
        let ev = self.insert(dev, seq, at, actor, kind, payload);
        self.c.handle(&ev).unwrap();
        ev
    }
}

impl Env {
    /// Appends a row as this device with no checks and folds it, as Go's
    /// tests do with `c.impl.write` (for rows the calls would refuse).
    pub fn write_raw<P: serde::Serialize>(&self, actor: &str, kind: &str, payload: &P) -> Event {
        let ev = self.st.append(actor, kind, payload).unwrap();
        self.c.fold_live(std::slice::from_ref(&ev));
        ev
    }
}

/// An HLC at wall time t.
pub fn hlc_at(t: GoTime) -> u64 {
    (t.unix_milli_value() as u64) << 16
}

pub fn want_err<T: std::fmt::Debug>(r: coord::Result<T>, kind: ErrorKind) {
    match r {
        Ok(v) => panic!("got {v:?}, want {kind:?}"),
        Err(e) => assert!(e.is(kind), "err = {e} ({:?}), want {kind:?}", e.kind()),
    }
}

/// Decodes a row payload as Go would.
pub fn payload<T: coord::gojson::GoDecode>(ev: &Event) -> T {
    coord::gojson::decode(&ev.payload).expect("payload decodes")
}

pub fn ids(ts: &[coord::Task]) -> Vec<String> {
    ts.iter().map(|t| t.id.clone()).collect()
}
