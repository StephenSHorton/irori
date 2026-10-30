//! The Go inbox test harness (I:internal/inbox/helpers_test.go): a fake
//! roster and clock, keys dev1/aaaa…, the owner `user:dev1`.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use gocompat::time::GoTime;
use inbox::{
    Address, AgentCard, BYPASS_MODE, Change, ChangeKind, Delivery, ErrorKind, Inbox, KeyPredicate,
    Message, OnGoneFn, OnPassedFn, Options, PrOwnersFn, Result, Roster, Sender, StaleFn, Wake,
    addr, path,
};
use store::Store;

pub const KEY_A: &str = "dev1/aaaaaaaa-1111-4111-8111-111111111111";
pub const KEY_B: &str = "dev1/bbbbbbbb-2222-4222-8222-222222222222";
pub const KEY_C: &str = "dev1/cccccccc-3333-4333-8333-333333333333";
pub const KEY_G: &str = "dev1/grok-gggggggg";
pub const USER: &str = "user:dev1";

#[derive(Default)]
pub struct FakeRoster {
    cards: Mutex<BTreeMap<String, AgentCard>>,
}

impl Roster for FakeRoster {
    fn agents(&self) -> Vec<AgentCard> {
        self.cards.lock().unwrap().values().cloned().collect()
    }
}

impl FakeRoster {
    pub fn put(&self, cs: &[AgentCard]) {
        let mut m = self.cards.lock().unwrap();
        for c in cs {
            m.insert(c.key.clone(), c.clone());
        }
    }

    pub fn remove(&self, keys: &[&str]) {
        let mut m = self.cards.lock().unwrap();
        for k in keys {
            m.remove(*k);
        }
    }
}

#[derive(Clone)]
pub struct FakeClock(pub Arc<Mutex<GoTime>>);

impl FakeClock {
    pub fn now(&self) -> GoTime {
        *self.0.lock().unwrap()
    }

    pub fn advance(&self, d: Duration) {
        let mut t = self.0.lock().unwrap();
        let n = i128::from(t.secs) * 1_000_000_000 + i128::from(t.nanos) + d.as_nanos() as i128;
        *t = GoTime {
            secs: n.div_euclid(1_000_000_000) as i64,
            nanos: n.rem_euclid(1_000_000_000) as u32,
            offset: 0,
        };
    }
}

/// `t + d` (Go `t.Add(d)`).
pub fn add(t: GoTime, d: Duration) -> GoTime {
    let c = FakeClock(Arc::new(Mutex::new(t)));
    c.advance(d);
    c.now()
}

pub fn mins(n: u64) -> Duration {
    Duration::from_secs(60 * n)
}

pub fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// Options hooks for the next open.
#[derive(Default, Clone)]
pub struct Hooks {
    pub owners: Option<PrOwnersFn>,
    pub is_owner: Option<KeyPredicate>,
    pub on_gone: Option<OnGoneFn>,
    pub on_passed: Option<OnPassedFn>,
    pub revoked: Option<KeyPredicate>,
    pub stale: Option<StaleFn>,
}

pub struct Harness {
    pub dir: tempfile::TempDir,
    pub st: Arc<Store>,
    pub ros: Arc<FakeRoster>,
    pub clk: FakeClock,
    pub ib: Arc<Inbox>,
    pub hooks: Hooks,
    pub device: String,
    /// The current inbox, for hooks that call back into it.
    pub slot: Slot,
}

/// The harness's current inbox, for hooks.
pub type Slot = Arc<Mutex<Option<Weak<Inbox>>>>;

pub fn slot_inbox(s: &Slot) -> Arc<Inbox> {
    s.lock()
        .unwrap()
        .as_ref()
        .and_then(Weak::upgrade)
        .expect("inbox in slot")
}

/// 2026-10-26T09:00:00Z.
pub fn t0() -> GoTime {
    GoTime::parse_rfc3339("2026-10-26T09:00:00Z").unwrap()
}

pub fn open_store(path: PathBuf, device: &str, clk: &FakeClock) -> Arc<Store> {
    let c = clk.clone();
    let clock = hlc::Clock::with_now(Box::new(move || c.now().to_system()));
    Arc::new(Store::open(path, device, Arc::new(clock)).expect("open store"))
}

impl Harness {
    pub fn new() -> Harness {
        Harness::with(Hooks::default())
    }

    pub fn with(hooks: Hooks) -> Harness {
        Harness::on_device("dev1", hooks)
    }

    pub fn on_device(device: &str, hooks: Hooks) -> Harness {
        Harness::on_device_with(device, |_| hooks)
    }

    /// Hooks built with the slot the inbox will be in.
    pub fn with_slot(f: impl FnOnce(Slot) -> Hooks) -> Harness {
        Harness::on_device_with("dev1", f)
    }

    pub fn on_device_with(device: &str, f: impl FnOnce(Slot) -> Hooks) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let clk = FakeClock(Arc::new(Mutex::new(t0())));
        let ros = Arc::new(FakeRoster::default());
        let slot: Slot = Arc::new(Mutex::new(None));
        let hooks = f(slot.clone());
        let st = open_store(dir.path().join("irori.db"), device, &clk);
        let ib = Arc::new(Harness::inbox(&st, &ros, &clk, &hooks, device));
        *slot.lock().unwrap() = Some(Arc::downgrade(&ib));
        Harness {
            dir,
            st,
            ros,
            clk,
            ib,
            hooks,
            device: device.into(),
            slot,
        }
    }

    pub fn inbox(
        st: &Arc<Store>,
        ros: &Arc<FakeRoster>,
        clk: &FakeClock,
        hooks: &Hooks,
        device: &str,
    ) -> Inbox {
        let c = clk.clone();
        Inbox::new(
            st.clone(),
            ros.clone(),
            Options {
                device_id: device.into(),
                now: Some(Arc::new(move || c.now())),
                pr_owners: hooks.owners.clone(),
                is_owner: hooks.is_owner.clone(),
                on_recipient_gone: hooks.on_gone.clone(),
                on_passed: hooks.on_passed.clone(),
                revoked: hooks.revoked.clone(),
                stale: hooks.stale.clone(),
                ..Options::default()
            },
        )
        .expect("inbox")
    }

    /// Closes the store and replays it into a fresh inbox.
    pub fn reopen(&mut self) {
        self.st.close().unwrap();
        self.st = open_store(self.dir.path().join("irori.db"), &self.device, &self.clk);
        self.ib = Arc::new(Harness::inbox(
            &self.st,
            &self.ros,
            &self.clk,
            &self.hooks,
            &self.device,
        ));
        *self.slot.lock().unwrap() = Some(Arc::downgrade(&self.ib));
    }

    /// A fresh inbox over the same store (Go `New` again, store open).
    pub fn fresh(&self) -> Inbox {
        Harness::inbox(&self.st, &self.ros, &self.clk, &self.hooks, &self.device)
    }

    /// Three bypass-mode Claude sessions a, b, c.
    pub fn abc(&self) {
        self.ros.put(&[
            card(KEY_A, "alpha", "repo1", BYPASS_MODE),
            card(KEY_B, "beta", "repo1", BYPASS_MODE),
            card(KEY_C, "gamma", "repo2", BYPASS_MODE),
        ]);
    }

    #[track_caller]
    pub fn send(&self, from: &Sender, a: &Address, text: &str) -> (Message, Delivery) {
        match self.ib.send(from, a, text, "") {
            Ok(x) => x,
            Err(e) => panic!("send {text:?}: {e}"),
        }
    }

    #[track_caller]
    pub fn state(&self, id: &str) -> Delivery {
        self.ib
            .get(id)
            .unwrap_or_else(|| panic!("no delivery {id}"))
    }

    #[track_caller]
    pub fn want_state(&self, id: &str, want: &str) -> Delivery {
        let d = self.state(id);
        assert_eq!(
            d.state, want,
            "delivery {id} is {} (hold {:?}), want {want}",
            d.state, d.hold
        );
        d
    }

    #[track_caller]
    pub fn wake(&self, agent: &str) -> Wake {
        self.ib
            .next_wake(agent, path::MOD)
            .unwrap()
            .unwrap_or_else(|| panic!("no wake for {agent}"))
    }

    #[track_caller]
    pub fn no_wake(&self, agent: &str) {
        if let Some(w) = self.ib.next_wake(agent, path::MOD).unwrap() {
            panic!("unexpected wake for {agent}: {:?}", w.delivery_ids);
        }
    }

    #[track_caller]
    pub fn deliver_engage(&self, agent: &str, turn: &str) -> Wake {
        let w = self.wake(agent);
        self.ib.mark_delivered(agent, &w.token).unwrap();
        self.ib.mark_engaged(agent, &w.token, turn).unwrap();
        w
    }

    #[track_caller]
    pub fn sweep(&self) {
        self.ib.sweep().unwrap();
    }
}

pub fn card(key: &str, name: &str, repo: &str, mode: &str) -> AgentCard {
    let sess = key.rsplit('/').next().unwrap_or(key);
    AgentCard {
        key: key.into(),
        device_id: key.split('/').next().unwrap_or("").into(),
        session_id: sess.into(),
        agent: "claude".into(),
        name: name.into(),
        status: "idle".into(),
        repo_key: repo.into(),
        permission_mode: mode.into(),
    }
}

pub fn grok_card(key: &str, name: &str, repo: &str) -> AgentCard {
    let mut c = card(key, name, repo, "");
    c.agent = "grok".into();
    c
}

pub fn claude(key: &str, name: &str) -> Sender {
    Sender::new(key, name, "claude")
}
pub fn grok(key: &str, name: &str) -> Sender {
    Sender::new(key, name, "grok")
}
pub fn person() -> Sender {
    Sender::new(USER, "Stephen", "user")
}
pub fn to(v: &str) -> Address {
    Address::new(addr::AGENT, v)
}
pub fn to_repo(v: &str) -> Address {
    Address::new(addr::REPO, v)
}

#[track_caller]
pub fn want_err<T: std::fmt::Debug>(r: Result<T>, want: ErrorKind) {
    match r {
        Err(e) if e.is(want) => {}
        other => panic!("got {other:?}, want {want:?}"),
    }
}

pub fn count(cs: &[Change], kind: ChangeKind, to: &str) -> usize {
    cs.iter().filter(|c| c.kind == kind && c.to == to).count()
}

impl Harness {
    #[track_caller]
    pub fn send_with(
        &self,
        from: &Sender,
        a: &Address,
        text: &str,
        o: inbox::SendOptions,
    ) -> Delivery {
        match self.ib.send_with(from, a, text, o) {
            Ok((_, d)) => d,
            Err(e) => panic!("send {text:?}: {e}"),
        }
    }

    #[track_caller]
    pub fn append_for(&self, agent: &str) -> Wake {
        self.ib
            .next_append(agent)
            .unwrap()
            .unwrap_or_else(|| panic!("no append for {agent}"))
    }

    #[track_caller]
    pub fn no_append(&self, agent: &str) {
        if let Some(w) = self.ib.next_append(agent).unwrap() {
            panic!("unexpected append for {agent}: {:?}", w.delivery_ids);
        }
    }

    /// Past the reattach grace, then a sweep: unconfirmed wakes reclaimed.
    #[track_caller]
    pub fn end_a(&self) {
        self.ros.remove(&[KEY_A]);
        self.sweep(); // first sighting starts the clock
        self.clk.advance(inbox::GONE_GRACE + secs(1));
        self.sweep();
    }

    #[track_caller]
    pub fn reclaim_all(&self) {
        self.clk.advance(inbox::REATTACH_GRACE + secs(1));
        self.sweep();
    }
}

pub fn opts(mode: &str) -> inbox::SendOptions {
    inbox::SendOptions {
        mode: mode.into(),
        ..inbox::SendOptions::default()
    }
}
