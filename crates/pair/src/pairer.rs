//! The Pairer (I:internal/pair/pair.go): at most one pairing at a time,
//! either side. The flow, states, timers and person-facing texts port
//! exactly (identity.md §4.7); the room sits on the mesh seam
//! (`irori_meshseam`), so FakeNet and the iroh transport both carry it.
//!
//! Locking: `Inner::m` (status and the current session) is taken before a
//! session's own `f`, never the other way round, and neither is held
//! across an `.await`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, SystemTime};

use gocompat::time::GoTime;
use irori_identity::{Chain, Identity};
use irori_meshseam::{Conn, Envelope, Refused, Room, RoomConfig, Secret, Transport};
use serde::{Deserialize, Serialize};
use store::{Filter, Store};
use tokio_util::sync::CancellationToken;

use crate::code::{
    check_proof, fingerprint, info, key_match, normalize_code, proof, room_secret, sas, words_match,
};
use crate::saved::{ROWS_FILE, learn_all, save_rows};
use crate::wire::{
    FRAME_CANCEL, FRAME_DONE, FRAME_FAIL, FRAME_GRANT, FRAME_KEY, FRAME_PROOF, Frame, Hello,
    WirePerson, WireRow,
};

/// How long a pairing stays open: one use, 10 minutes.
pub const WINDOW: Duration = Duration::from_secs(10 * 60);
/// How often B repeats `pair.hello` until A connects.
pub const HELLO_EVERY: Duration = Duration::from_secs(2);
/// How many wrong typings close the window.
pub const MAX_MISSES: i64 = 2;

/// Envelope types in the pairing room.
pub const MSG_HELLO: &str = "pair.hello";
pub const MSG_BUSY: &str = "pair.busy";

pub const MODE_ADD: &str = "add";
pub const MODE_JOIN: &str = "join";

/// add: the code is shown; join: looking for the other device.
pub const STATE_WAITING: &str = "waiting";
/// add: type the words B shows.
pub const STATE_CONFIRM: &str = "confirm";
/// add: the words matched; type this device's key on B.
pub const STATE_GRANTED: &str = "granted";
/// join: read these words out, and type the key A shows.
pub const STATE_WORDS: &str = "words";
pub const STATE_DONE: &str = "done";
pub const STATE_FAILED: &str = "failed";
pub const STATE_CANCELED: &str = "canceled";
pub const STATE_EXPIRED: &str = "expired";
/// join: A was already pairing with someone (`pair.busy`).
pub const STATE_CODE_IN_USE: &str = "code_used";

pub const ERR_TEXT_SOMEONE_ELSE: &str = "Someone else tried to use this code. Make a new one.";
pub const ERR_TEXT_CODE_USED: &str =
    "That code was already used. Make a new one on the other device.";
pub const ERR_TEXT_NOT_THAT_KEY: &str =
    "That isn't the key the other device shows: someone else may have the code. Make a new one.";

const GONE: &str = "The other device went away before pairing finished.";

/// The Pairer's errors, with Go's texts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Busy,
    InMesh,
    NotOpen,
    WrongWords,
    WrongKey,
    NotCertified,
    BadCode,
    /// `pair: open the pairing room: …`
    Room(String),
    /// Anything else, as its text.
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Busy => f.write_str("pair: a pairing is already open; finish or cancel it first"),
            Error::InMesh => f.write_str("pair: this device is already one of a person's devices; remove it there or reset it first"),
            Error::NotOpen => f.write_str("pair: no pairing is waiting for words"),
            Error::WrongWords => f.write_str("pair: those aren't the words the other device shows"),
            Error::WrongKey => f.write_str("pair: that isn't the key the other device shows"),
            Error::NotCertified => f.write_str("pair: this device isn't certified, so it can't add another"),
            Error::BadCode => f.write_str(crate::code::ERR_BAD_CODE),
            Error::Room(e) => write!(f, "pair: open the pairing room: {e}"),
            Error::Other(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for Error {}

/// The device on the other end, as the add side shows it above the words box.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Joiner {
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub name: String,
    pub os: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub fingerprint: String,
}

/// The current (or last) pairing, for the Devices dialog (Go `Status`,
/// same JSON field order and omissions). It also reads back as the
/// loopback API's client takes it (`irori pair join`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub mode: String,
    pub state: String,
    /// add, while waiting.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub code: String,
    /// (add) The home server's address from outside the home, shown under
    /// the code for a device joining from away.
    #[serde(rename = "serverAddr", skip_serializing_if = "String::is_empty")]
    pub server_addr: String,
    /// (join) The home server address typed, if any.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub via: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<GoTime>,
    /// add: the role B gets.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub role: String,
    /// (add) This device's key, shown beside the code.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub fingerprint: String,
    /// (add) B, once it connected and proved the code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub joiner: Option<Joiner>,
    /// (join) What B shows once both proved the code.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<String>,
    /// (join) A's name.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub peer: String,
    /// (join) The key typed here is the one A's handshake proved.
    #[serde(rename = "keyOk", skip_serializing_if = "std::ops::Not::not")]
    pub key_ok: bool,
    #[serde(skip_serializing_if = "is_zero")]
    pub misses: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

impl Status {
    /// The pairing is still in progress.
    pub fn open(&self) -> bool {
        matches!(
            self.state.as_str(),
            STATE_WAITING | STATE_CONFIRM | STATE_GRANTED | STATE_WORDS
        )
    }
}

/// The home server directory as pairing uses it (Go `*mesh.Servers`): A
/// hands B the servers it knows, and B keeps them until their rows arrive.
pub trait ServerHints: Send + Sync {
    fn hints(&self) -> std::collections::BTreeMap<String, Vec<String>>;
    fn hint(&self, id: &str, addrs: &[String]);
}

pub type Callback = Arc<dyn Fn() + Send + Sync>;
pub type StatusFn = Arc<dyn Fn(Status) + Send + Sync>;
pub type LogFn = Arc<dyn Fn(&str) + Send + Sync>;
pub type NowFn = Arc<dyn Fn() -> SystemTime + Send + Sync>;
pub type AddrFn = Arc<dyn Fn() -> String + Send + Sync>;

/// Sets up a [`Pairer`] (Go `Options`).
#[derive(Clone)]
pub struct Options {
    pub net: Arc<dyn Transport>,
    pub ident: Arc<Identity>,
    pub chain: Arc<Chain>,
    pub store: Arc<Store>,
    /// Where B keeps the person rows it was handed until they are stored.
    pub data_dir: Option<PathBuf>,
    /// Re-reads the cert fold in the net layer (mesh `Node::kick`).
    pub kick: Option<Callback>,
    /// Zero: [`WINDOW`] / [`HELLO_EVERY`].
    pub window: Duration,
    pub hello_every: Duration,
    pub now: Option<NowFn>,
    pub logf: Option<LogFn>,
    /// Told whenever Status changes (the GUI refreshes).
    pub on_change: Option<StatusFn>,
    /// Also told, for the Core: the home server starts while a pairing
    /// window is open.
    pub notify: Option<Callback>,
    pub servers: Option<Arc<dyn ServerHints>>,
    /// The address a device away from home dials to join (`serverAddr`).
    pub server_addr: Option<AddrFn>,
}

impl Options {
    pub fn new(
        net: Arc<dyn Transport>,
        ident: Arc<Identity>,
        chain: Arc<Chain>,
        store: Arc<Store>,
    ) -> Options {
        Options {
            net,
            ident,
            chain,
            store,
            data_dir: None,
            kick: None,
            window: Duration::ZERO,
            hello_every: Duration::ZERO,
            now: None,
            logf: None,
            on_change: None,
            notify: None,
            servers: None,
            server_addr: None,
        }
    }
}

struct Shared {
    o: Options,
    cur: Option<Arc<Session>>,
    st: Status,
}

struct Inner {
    m: Mutex<Shared>,
    rt: Option<tokio::runtime::Handle>,
}

/// Runs at most one pairing at a time, either side. Cheap to clone.
#[derive(Clone)]
pub struct Pairer {
    inner: Arc<Inner>,
}

/// One open pairing window.
struct Session {
    p: Weak<Inner>,
    add: bool,
    secret: Secret,
    role: String,
    via: String,
    room: OnceLock<Arc<dyn Room>>,
    token: CancellationToken,
    /// add: cancelled once `pair.done` arrives.
    granted: CancellationToken,
    f: Mutex<Fields>,
}

#[derive(Default)]
struct Fields {
    /// add: the first hello's device id.
    joiner: String,
    hello: Hello,
    /// The one peer this side lets up, and its key.
    pin_id: String,
    pin_pub: Option<[u8; 32]>,
    conn: Option<Arc<dyn Conn>>,
    words: Vec<String>,
    misses: i64,
    /// add: the grant went out; join: it was adopted.
    sent: bool,
    closed: bool,
    /// join: the person typed A's key; add: B said so (`pair.key`).
    key_ok: bool,
    /// add: the person typed B's words.
    words_ok: bool,
    /// add: the grant is being written.
    giving: bool,
    /// join: A's grant, waiting for the key.
    pending: Option<Frame>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Inner {
    fn logf(&self, msg: &str) {
        let f = lock(&self.m).o.logf.clone();
        if let Some(f) = f {
            f(msg);
        }
    }

    fn spawn<F: std::future::Future<Output = ()> + Send + 'static>(&self, fut: F) {
        match tokio::runtime::Handle::try_current()
            .ok()
            .or_else(|| self.rt.clone())
        {
            Some(h) => {
                h.spawn(fut);
            }
            None => {
                std::thread::spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("runtime")
                        .block_on(fut)
                });
            }
        }
    }

    /// The status as callers see it; `g` is held.
    fn copy_status(g: &Shared) -> Status {
        let mut st = g.st.clone();
        if st.mode == MODE_ADD
            && st.open()
            && let Some(f) = &g.o.server_addr
        {
            // The home server may come up (or map its port) after the
            // window opened: the address under the code follows it.
            st.server_addr = f();
        }
        st
    }

    /// Publishes Status; `g` is held.
    fn changed(&self, g: &Shared) {
        if let Some(f) = g.o.on_change.clone() {
            let st = Self::copy_status(g);
            self.spawn(async move { f(st) });
        }
        if let Some(f) = g.o.notify.clone() {
            self.spawn(async move { f() });
        }
    }
}

impl Pairer {
    pub fn new(mut o: Options) -> Pairer {
        if o.window.is_zero() {
            o.window = WINDOW;
        }
        if o.hello_every.is_zero() {
            o.hello_every = HELLO_EVERY;
        }
        Pairer {
            inner: Arc::new(Inner {
                m: Mutex::new(Shared {
                    o,
                    cur: None,
                    st: Status::default(),
                }),
                rt: tokio::runtime::Handle::try_current().ok(),
            }),
        }
    }

    /// The current or last pairing.
    pub fn status(&self) -> Status {
        Inner::copy_status(&lock(&self.inner.m))
    }

    /// Replaces `Options::on_change` (the GUI subscribes after the Core is up).
    pub fn set_on_change(&self, f: Option<StatusFn>) {
        lock(&self.inner.m).o.on_change = f;
    }

    /// Replaces `Options::notify` (the Core's home server, built after).
    pub fn set_notify(&self, f: Option<Callback>) {
        lock(&self.inner.m).o.notify = f;
    }

    /// Replaces `Options::kick` and `Options::servers` (the mesh, built
    /// after pairing in the Core).
    pub fn set_mesh(&self, kick: Option<Callback>, servers: Option<Arc<dyn ServerHints>>) {
        let mut g = lock(&self.inner.m);
        g.o.kick = kick;
        g.o.servers = servers;
    }

    /// Shortens or restores the window (tests).
    pub fn set_window(&self, d: Duration) {
        lock(&self.inner.m).o.window = if d.is_zero() { WINDOW } else { d };
    }

    fn opts(&self) -> Options {
        lock(&self.inner.m).o.clone()
    }

    /// Go `InMesh`: this device already belongs to a mesh of more than
    /// itself (it joined another device's person, or its person has another
    /// certified device).
    pub fn in_mesh(&self) -> bool {
        let o = self.opts();
        in_mesh(&o)
    }

    /// Go `Add`: opens a pairing window on this device (A): a fresh code and
    /// the pairing room. `role` is what the joiner's cert says (`node`, else
    /// `desktop`).
    pub async fn add(&self, role: &str) -> Result<Status, Error> {
        let role = if role == irori_identity::ROLE_NODE {
            irori_identity::ROLE_NODE
        } else {
            irori_identity::ROLE_DESKTOP
        };
        let o = self.opts();
        let me = o.ident.device_id();
        if o.chain.cert(&me).is_none() || o.chain.cut(&me).is_some() {
            return Err(Error::NotCertified);
        }
        let code = crate::code::new_code();
        let secret = derive(&code).await?;
        let s = self.reserve(true, secret, role, "")?;
        if let Err(e) = self.open_room(&s).await {
            self.unreserve(&s);
            return Err(Error::Room(e));
        }
        let window = {
            let mut g = lock(&self.inner.m);
            let now = (g.o.now.clone()).map_or_else(SystemTime::now, |f| f());
            g.st = Status {
                mode: MODE_ADD.into(),
                state: STATE_WAITING.into(),
                code,
                expires: Some(GoTime::from_system(now + g.o.window)),
                role: role.into(),
                fingerprint: fingerprint(&g.o.ident.device_pub()),
                ..Default::default()
            };
            if let Some(f) = &g.o.server_addr {
                g.st.server_addr = f();
            }
            self.inner.changed(&g);
            g.o.window
        };
        s.arm_timer(window, "Nobody used the code in time. Make a new one.");
        Ok(self.status())
    }

    /// Go `Join`: enters a code on this device (B). It returns at once;
    /// Status moves to `words` when the words are ready to read out.
    pub async fn join(&self, code: &str) -> Result<Status, Error> {
        self.join_via(code, "").await
    }

    /// Go `JoinVia`: Join from away from home; `server` is the home
    /// server's address (host:port) as A's Add screen shows it.
    pub async fn join_via(&self, code: &str, server: &str) -> Result<Status, Error> {
        let canon = normalize_code(code).ok_or(Error::BadCode)?;
        if self.in_mesh() {
            return Err(Error::InMesh);
        }
        let secret = derive(&canon).await?;
        let s = self.reserve(false, secret, "", server)?;
        if let Err(e) = self.open_room(&s).await {
            self.unreserve(&s);
            return Err(Error::Room(e));
        }
        let (window, every) = {
            let mut g = lock(&self.inner.m);
            let now = (g.o.now.clone()).map_or_else(SystemTime::now, |f| f());
            g.st = Status {
                mode: MODE_JOIN.into(),
                state: STATE_WAITING.into(),
                expires: Some(GoTime::from_system(now + g.o.window)),
                via: server.into(),
                ..Default::default()
            };
            self.inner.changed(&g);
            (g.o.window, g.o.hello_every)
        };
        s.arm_timer(window, "The other device didn't answer in time.");
        let s2 = s.clone();
        self.inner.spawn(async move { s2.hello_loop(every).await });
        Ok(self.status())
    }

    /// Go `Cancel`: closes the open pairing, if any.
    pub fn cancel(&self) {
        let s = lock(&self.inner.m).cur.clone();
        if let Some(s) = s {
            s.fail_spawn(STATE_CANCELED, "Canceled.", FRAME_CANCEL);
        }
    }

    /// Go `CancelJoin`: closes the open pairing only when this device is the
    /// joining side (B). The loopback API cancels with it, so a token holder
    /// can't close the person's Add window.
    pub fn cancel_join(&self) -> bool {
        let s = lock(&self.inner.m).cur.clone();
        match s {
            Some(s) if !s.add => {
                s.fail_spawn(STATE_CANCELED, "Canceled.", FRAME_CANCEL);
                true
            }
            _ => false,
        }
    }

    /// Go `Confirm`: the person typing B's words into A. A miss counts, and
    /// the second one closes the window. A match certifies B and hands it
    /// the person once B also said the person typed this device's key there
    /// (`pair.key`); until then A waits in `granted`, and nothing is
    /// written, so a pairing that fails on B's side leaves no cert behind.
    pub async fn confirm(&self, typed: &str) -> Result<(), Error> {
        let (s, miss) = {
            let mut g = lock(&self.inner.m);
            let Some(s) = g.cur.clone() else {
                return Err(Error::NotOpen);
            };
            if !s.add || g.st.state != STATE_CONFIRM {
                return Err(Error::NotOpen);
            }
            let mut f = lock(&s.f);
            if words_match(&f.words, typed) {
                f.words_ok = true;
                drop(f);
                g.st.state = STATE_GRANTED.into();
                self.inner.changed(&g);
                (s, None)
            } else {
                f.misses += 1;
                g.st.misses = f.misses;
                let miss = f.misses;
                drop(f);
                self.inner.changed(&g);
                (s, Some(miss))
            }
        };
        if let Some(miss) = miss {
            if miss >= MAX_MISSES {
                s.fail(STATE_FAILED, ERR_TEXT_SOMEONE_ELSE, FRAME_FAIL)
                    .await;
            }
            return Err(Error::WrongWords);
        }
        s.maybe_grant().await
    }

    /// Go `ConfirmKey`: the person typing, on B, the key A shows beside its
    /// code. B takes a grant only from the device whose handshake proved
    /// that key. A grant that arrived before the key was typed waits for it;
    /// a miss counts, and the second one closes the window.
    pub async fn confirm_key(&self, typed: &str) -> Result<(), Error> {
        let (s, conn, pending, miss) = {
            let mut g = lock(&self.inner.m);
            let Some(s) = g.cur.clone() else {
                return Err(Error::NotOpen);
            };
            let mut f = lock(&s.f);
            let conn = match &f.conn {
                Some(c) if !s.add && g.st.state == STATE_WORDS && !f.key_ok => c.clone(),
                _ => return Err(Error::NotOpen),
            };
            if key_match(&fingerprint(&conn.pub_key()), typed) {
                f.key_ok = true;
                g.st.key_ok = true;
                let pending = f.pending.take();
                drop(f);
                self.inner.changed(&g);
                (s, conn, pending, None)
            } else {
                f.misses += 1;
                g.st.misses = f.misses;
                let miss = f.misses;
                drop(f);
                self.inner.changed(&g);
                (s, conn, None, Some(miss))
            }
        };
        if let Some(miss) = miss {
            if miss >= MAX_MISSES {
                s.fail(STATE_FAILED, ERR_TEXT_NOT_THAT_KEY, FRAME_FAIL)
                    .await;
            }
            return Err(Error::WrongKey);
        }
        // A grants only after this (and its own words): nothing is
        // certified for a pairing B never confirmed.
        if let Err(e) = s.send(&conn, &Frame::of(FRAME_KEY)).await {
            s.fail(STATE_FAILED, GONE, "").await;
            return Err(Error::Other(e));
        }
        if let Some(g) = pending {
            s.take(conn, g).await;
        }
        Ok(())
    }

    fn reserve(
        &self,
        add: bool,
        secret: Secret,
        role: &str,
        via: &str,
    ) -> Result<Arc<Session>, Error> {
        let mut g = lock(&self.inner.m);
        if g.cur.is_some() {
            return Err(Error::Busy);
        }
        let s = Arc::new(Session {
            p: Arc::downgrade(&self.inner),
            add,
            secret,
            role: role.into(),
            via: via.into(),
            room: OnceLock::new(),
            token: CancellationToken::new(),
            granted: CancellationToken::new(),
            f: Mutex::new(Fields::default()),
        });
        g.cur = Some(s.clone());
        Ok(s)
    }

    fn unreserve(&self, s: &Arc<Session>) {
        let mut g = lock(&self.inner.m);
        if g.cur.as_ref().is_some_and(|c| Arc::ptr_eq(c, s)) {
            g.cur = None;
        }
        s.token.cancel();
    }

    /// Go `session.join`: opens the pairing room.
    async fn open_room(&self, s: &Arc<Session>) -> Result<(), String> {
        let o = self.opts();
        let me = o.ident.device_id();
        let mut cfg = RoomConfig::new(me);
        let w = Arc::downgrade(s);
        cfg.accept = Arc::new({
            let w = w.clone();
            move |peer, pub_key| match w.upgrade() {
                Some(s) => s.accept(peer, pub_key),
                None => Err(Refused),
            }
        });
        cfg.allow = Arc::new({
            let w = w.clone();
            move |peer| w.upgrade().is_some_and(|s| s.allow(peer))
        });
        let add = s.add;
        // In the pairing room A always offers.
        cfg.offerer = Some(Arc::new(move |_| add));
        cfg.on_msg = Arc::new({
            let w = w.clone();
            move |env| {
                if let Some(s) = w.upgrade() {
                    s.on_msg(env);
                }
            }
        });
        cfg.on_peer = Arc::new({
            let w = w.clone();
            move |c| {
                if let Some(s) = w.upgrade() {
                    s.on_peer(c);
                }
            }
        });
        // A opens the room to guests on every home server it reaches, so B
        // can join from away; B dials the address typed, if any.
        cfg.guest_ttl = s.add.then_some(o.window);
        cfg.hub_addr = (!s.via.is_empty()).then(|| s.via.clone());
        let room = o.net.join(s.secret.clone(), cfg).await.map_err(|e| e.0)?;
        if s.token.is_cancelled() {
            room.close();
            return Err("canceled".into());
        }
        let _ = s.room.set(room);
        Ok(())
    }
}

/// The scrypt derivation off the async threads (it takes a while in debug).
async fn derive(code: &str) -> Result<Secret, Error> {
    let code = code.to_string();
    tokio::task::spawn_blocking(move || room_secret(&code))
        .await
        .map_err(|e| Error::Other(e.to_string()))?
        .map_err(|_| Error::BadCode)
}

fn in_mesh(o: &Options) -> bool {
    let me = o.ident.device_id();
    if o.ident.current().root_device_id != me {
        return true;
    }
    o.chain.devices().iter().any(|d| d.cert.device_id != me)
}

impl Session {
    fn pairer(&self) -> Option<Arc<Inner>> {
        self.p.upgrade()
    }

    fn opts(&self) -> Option<Options> {
        self.pairer().map(|p| lock(&p.m).o.clone())
    }

    fn arm_timer(self: &Arc<Self>, window: Duration, why: &'static str) {
        let s = self.clone();
        let Some(p) = self.pairer() else { return };
        p.spawn(async move {
            tokio::select! {
                _ = tokio::time::sleep(window) => s.fail(STATE_EXPIRED, why, "").await,
                _ = s.token.cancelled() => {}
            }
        });
    }

    /// Lets up only the one device this side is pairing with: on A the
    /// first hello's sender, on B the first device that offers.
    fn allow(&self, peer: &str) -> bool {
        let Some(o) = self.opts() else { return false };
        let f = lock(&self.f);
        if f.closed || peer == o.ident.device_id() {
            return false;
        }
        if self.add {
            return peer == f.joiner;
        }
        f.pin_id.is_empty() || f.pin_id == peer
    }

    /// Takes any key once, as nothing is certified yet, and refuses a
    /// different one for the same device afterwards. A refuses a device
    /// that is itself, revoked, or already certified.
    fn accept(&self, peer: &str, pub_key: &[u8; 32]) -> Result<(), Refused> {
        let Some(o) = self.opts() else {
            return Err(Refused);
        };
        if peer == o.ident.device_id() {
            return Err(Refused);
        }
        if self.add && o.chain.cert(peer).is_some() {
            // Already one of the person's devices, or a copied data dir or
            // restored backup under another key.
            return Err(Refused);
        }
        let mut f = lock(&self.f);
        if f.closed || (self.add && peer != f.joiner) {
            return Err(Refused);
        }
        if f.pin_id.is_empty() {
            f.pin_id = peer.into();
            f.pin_pub = Some(*pub_key);
            return Ok(());
        }
        if f.pin_id != peer || f.pin_pub.as_ref() != Some(pub_key) {
            return Err(Refused);
        }
        Ok(())
    }

    fn on_msg(self: &Arc<Self>, env: Envelope) {
        let Some(o) = self.opts() else { return };
        match env.typ.as_str() {
            MSG_HELLO => {
                if !self.add {
                    return;
                }
                let Ok(h) = serde_json::from_value::<Hello>(env.body) else {
                    return;
                };
                let me = o.ident.device_id();
                if h.device_id.is_empty() || h.device_id != env.from || h.device_id == me {
                    return;
                }
                if o.chain.cut(&h.device_id).is_some() {
                    // A removed device: re-pairing it is a reset plus a join.
                    return;
                }
                let mut f = lock(&self.f);
                if f.closed {
                    return;
                }
                if f.joiner.is_empty() {
                    f.joiner = h.device_id.clone();
                    f.hello = h.clone();
                } else if f.joiner != h.device_id {
                    // A second device has the code: it gets pair.busy, and
                    // the window closes.
                    drop(f);
                    if let Some(r) = self.room.get() {
                        let _ = r.send(&h.device_id, MSG_BUSY, serde_json::json!({}));
                    }
                    self.fail_spawn(STATE_FAILED, ERR_TEXT_SOMEONE_ELSE, FRAME_FAIL);
                    return;
                } else if f.conn.is_some() {
                    return; // already connected; a repeated hello
                }
                drop(f);
                if let Some(r) = self.room.get() {
                    r.connect(&h.device_id);
                }
            }
            MSG_BUSY if !self.add => {
                self.fail_spawn(STATE_CODE_IN_USE, ERR_TEXT_CODE_USED, "");
            }
            _ => {}
        }
    }

    /// Repeats B's hello until a connection is up or the window ends.
    async fn hello_loop(self: Arc<Self>, every: Duration) {
        let Some(o) = self.opts() else { return };
        let dev = o.ident.self_device();
        let h = Hello {
            device_id: dev.id.clone(),
            name: dev.name.clone(),
            os: dev.os.clone(),
        };
        let body = serde_json::to_value(&h).expect("hello json");
        let mut t = tokio::time::interval(every);
        loop {
            t.tick().await;
            {
                let f = lock(&self.f);
                if f.conn.is_some() || f.closed {
                    return;
                }
            }
            if let Some(r) = self.room.get() {
                let _ = r.send("", MSG_HELLO, body.clone());
            }
            if self.token.is_cancelled() {
                return;
            }
        }
    }

    fn on_peer(self: &Arc<Self>, c: Arc<dyn Conn>) {
        let replaced = {
            let mut f = lock(&self.f);
            if f.closed || c.id() != f.pin_id {
                drop(f);
                c.close("not pairing");
                return;
            }
            if f.conn.is_some() {
                true
            } else {
                f.conn = Some(c.clone());
                false
            }
        };
        if replaced {
            // The connection was replaced: the words were (or would be)
            // computed on the old one, so pairing starts over.
            c.close("replaced");
            self.fail_spawn(
                STATE_FAILED,
                "The connection changed while pairing. Make a new code.",
                FRAME_FAIL,
            );
            return;
        }
        let s = self.clone();
        if let Some(p) = self.pairer() {
            p.spawn(async move { s.run(c).await });
        }
    }

    async fn send(&self, c: &Arc<dyn Conn>, f: &Frame) -> Result<(), String> {
        let b = f.encode();
        tokio::select! {
            r = c.send(&b) => r.map_err(|e| e.to_string()),
            _ = self.token.cancelled() => Err("canceled".into()),
        }
    }

    async fn recv(&self, c: &Arc<dyn Conn>) -> Result<Frame, String> {
        let b = tokio::select! {
            r = c.recv() => r.map_err(|e| e.to_string())?,
            _ = self.token.cancelled() => return Err("canceled".into()),
        };
        Frame::decode(&b).map_err(str::to_string)
    }

    /// The SAS exchange and what follows it on one side.
    async fn run(self: Arc<Self>, c: Arc<dyn Conn>) {
        let Some(o) = self.opts() else { return };
        let me = o.ident.device_id();
        let self_pub = o.ident.device_pub();
        let (local, remote) = c.fingerprints();
        let peer_pub = c.pub_key();
        let (inf, my_side, their_side) = if self.add {
            (
                info(&self_pub, &peer_pub, &local, &remote, &me, c.id()),
                b'A',
                b'B',
            )
        } else {
            (
                info(&peer_pub, &self_pub, &remote, &local, c.id(), &me),
                b'B',
                b'A',
            )
        };
        {
            let s = self.clone();
            let c2 = c.clone();
            if let Some(p) = self.pairer() {
                p.spawn(async move {
                    tokio::select! {
                        _ = c2.done() => {
                            if lock(&s.f).sent {
                                return; // grant's wait (A) or take (B) finishes it
                            }
                            s.fail(STATE_FAILED, GONE, "").await;
                        }
                        _ = s.token.cancelled() => {}
                    }
                });
            }
        }
        let dev = o.ident.self_device();
        let mut hello = Frame::of(FRAME_PROOF);
        hello.proof = Some(proof(&self.secret.key, my_side, &inf));
        hello.name = dev.name.clone();
        hello.os = dev.os.clone();
        if self.send(&c, &hello).await.is_err() {
            self.fail(STATE_FAILED, GONE, "").await;
            return;
        }
        let f = match self.recv(&c).await {
            Ok(f) => f,
            Err(_) => {
                self.fail(STATE_FAILED, GONE, "").await;
                return;
            }
        };
        let ok = f.typ == FRAME_PROOF
            && f.proof
                .as_deref()
                .is_some_and(|p| check_proof(&self.secret.key, their_side, &inf, p));
        if !ok {
            if self.add {
                self.fail(STATE_FAILED, ERR_TEXT_SOMEONE_ELSE, FRAME_FAIL)
                    .await;
            } else {
                self.fail(
                    STATE_FAILED,
                    "The other device couldn't prove it has this code.",
                    FRAME_FAIL,
                )
                .await;
            }
            return;
        }
        let words = sas(&self.secret.key, &inf);
        {
            let Some(p) = self.pairer() else { return };
            let mut g = lock(&p.m);
            let mut sf = lock(&self.f);
            if sf.closed {
                return;
            }
            sf.words = words.clone();
            if self.add {
                g.st.state = STATE_CONFIRM.into();
                g.st.code.clear();
                let mut j = Joiner {
                    device_id: c.id().to_string(),
                    name: sf.hello.name.clone(),
                    os: sf.hello.os.clone(),
                    fingerprint: fingerprint(&peer_pub),
                };
                if !f.name.is_empty() {
                    j.name = f.name.clone();
                }
                if !f.os.is_empty() {
                    j.os = f.os.clone();
                }
                g.st.joiner = Some(j);
            } else {
                g.st.state = STATE_WORDS.into();
                g.st.words = words;
                g.st.peer = f.name.clone();
            }
            drop(sf);
            p.changed(&g);
        }

        // After the words, A waits for Confirm (and for B's done once it
        // granted); B waits for the grant. Either side may fail or cancel.
        loop {
            let Ok(f) = self.recv(&c).await else {
                return; // the watcher above reports a lost connection
            };
            match f.typ.as_str() {
                FRAME_FAIL | FRAME_CANCEL => {
                    let msg = if f.typ == FRAME_FAIL {
                        "The other device closed the pairing."
                    } else {
                        "The other device canceled."
                    };
                    self.fail(STATE_FAILED, msg, "").await;
                    return;
                }
                FRAME_KEY => {
                    if self.add {
                        lock(&self.f).key_ok = true;
                        let s = self.clone();
                        if let Some(p) = self.pairer() {
                            p.spawn(async move {
                                let _ = s.maybe_grant().await;
                            });
                        }
                    }
                }
                FRAME_DONE => {
                    if self.add {
                        self.granted.cancel();
                    }
                }
                FRAME_GRANT => {
                    if self.add {
                        continue;
                    }
                    let ok = {
                        let mut sf = lock(&self.f);
                        if !sf.key_ok {
                            // Held until the person types A's key.
                            sf.pending = Some(f.clone());
                        }
                        sf.key_ok
                    };
                    if ok {
                        self.take(c.clone(), f).await;
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    /// Grants once both the words and B's key check are in.
    async fn maybe_grant(self: &Arc<Self>) -> Result<(), Error> {
        let (conn, joiner, hello, pub_key) = {
            let mut f = lock(&self.f);
            if f.closed || !self.add || !f.words_ok || !f.key_ok || f.giving {
                return Ok(());
            }
            f.giving = true;
            (f.conn.clone(), f.joiner.clone(), f.hello.clone(), f.pin_pub)
        };
        if let Err(e) = self.grant(conn, &joiner, pub_key, &hello).await {
            self.fail(STATE_FAILED, &format!("Pairing failed: {e}"), FRAME_FAIL)
                .await;
            return Err(Error::Other(e));
        }
        Ok(())
    }

    /// A after the words matched: certify B, then send it the person
    /// secret, its epoch, the root, and every person row, on the very
    /// connection the words were computed on.
    async fn grant(
        self: &Arc<Self>,
        c: Option<Arc<dyn Conn>>,
        joiner: &str,
        pub_key: Option<[u8; 32]>,
        h: &Hello,
    ) -> Result<(), String> {
        let (Some(c), Some(pub_key)) = (c, pub_key) else {
            return Err(Error::NotOpen.to_string());
        };
        if joiner.is_empty() {
            return Err(Error::NotOpen.to_string());
        }
        if c.err().is_some() {
            return Err("the connection went down".into());
        }
        let o = self.opts().ok_or("closed")?;
        let label = if h.name.is_empty() { joiner } else { &h.name };
        let now = o.now.as_ref().map_or_else(SystemTime::now, |f| f());
        let add = o
            .ident
            .cert_for(
                joiner,
                &hex::encode(pub_key),
                label,
                &self.role,
                GoTime::from_system(now),
            )
            .map_err(|e| e.to_string())?;
        o.store
            .append("irori", irori_identity::kinds::PERSON_DEVICE_ADDED, &add)
            .map_err(|e| e.to_string())?;
        let rows = person_rows(&o.store)?;
        let cur = o.ident.current();
        let seed = o
            .ident
            .person_seed(cur.epoch)
            .ok_or("this device doesn't hold its person's secret")?;
        lock(&self.f).sent = true;
        let mut g = Frame::of(FRAME_GRANT);
        g.person = Some(WirePerson::from(&cur));
        g.seed = Some(seed.to_vec());
        g.rows = rows;
        g.servers = o.servers.as_ref().map(|s| s.hints()).unwrap_or_default();
        if let Err(e) = self.send(&c, &g).await {
            lock(&self.f).sent = false;
            return Err(e);
        }
        drop(g);
        if let Some(k) = &o.kick {
            k();
        }
        if let Some(p) = self.pairer() {
            let g = &mut lock(&p.m);
            if !lock(&self.f).closed {
                g.st.state = STATE_GRANTED.into();
                p.changed(g);
            }
        }
        // B answers done once the person typed this device's key there and
        // it adopted the person. The cert is written either way.
        let s = self.clone();
        if let Some(p) = self.pairer() {
            p.spawn(async move {
                tokio::select! {
                    _ = s.granted.cancelled() => { s.finish(); return; }
                    _ = s.token.cancelled() => return,
                    _ = c.done() => {}
                }
                // done may still be on its way when the connection closes
                tokio::select! {
                    _ = s.granted.cancelled() => s.finish(),
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {
                        s.fail(STATE_FAILED, "The other device closed before it took the pairing. Pair it again.", "").await;
                    }
                    _ = s.token.cancelled() => {}
                }
            });
        }
        Ok(())
    }

    /// B adopting A's grant once the key was typed, then telling A.
    async fn take(self: &Arc<Self>, c: Arc<dyn Conn>, f: Frame) {
        if let Err(e) = self.adopt(&c, &f) {
            if let Some(p) = self.pairer() {
                p.logf(&format!("pair: refusing the grant: {e}"));
            }
            self.fail(
                STATE_FAILED,
                &format!("The other device's answer didn't check out: {e}"),
                FRAME_FAIL,
            )
            .await;
            return;
        }
        // The person is adopted: the connection closing now is A's.
        lock(&self.f).sent = true;
        // A closes the connection once it reads done; closing first here
        // could drop done in flight, and A would think B went away.
        if self.send(&c, &Frame::of(FRAME_DONE)).await.is_ok() {
            tokio::select! {
                _ = c.done() => {}
                _ = tokio::time::sleep(Duration::from_secs(3)) => {}
            }
        }
        self.finish();
    }

    /// B taking A's grant: the rows must certify this device under its own
    /// key and A under the key its handshake proved, and the secret must
    /// match the person key, before anything is replaced.
    fn adopt(&self, c: &Arc<dyn Conn>, f: &Frame) -> Result<(), String> {
        let o = self.opts().ok_or("closed")?;
        let (Some(wp), Some(seed)) = (&f.person, &f.seed) else {
            return Err("no person in it".into());
        };
        if seed.is_empty() {
            return Err("no person in it".into());
        }
        let seed = zeroize::Zeroizing::new(seed.clone());
        let person = irori_identity::Person::from(wp);
        let mut evs = Vec::with_capacity(f.rows.len());
        for r in &f.rows {
            if !irori_identity::kinds::is_person_kind(&r.k) {
                return Err(format!("a {} row in it", r.k));
            }
            evs.push(r.event());
        }
        let me = o.ident.device_id();
        let trial = Chain::new(&person.id, &person.root_device_id, &me);
        learn_all(&trial, &evs);
        let mine = trial.cert(&me);
        if mine.is_none_or(|m| m.device_pub != hex::encode(o.ident.device_pub())) {
            return Err("it doesn't certify this device".into());
        }
        let theirs = trial.cert(c.id());
        if theirs.is_none_or(|t| t.device_pub != hex::encode(c.pub_key())) {
            return Err("it doesn't certify the device that sent it".into());
        }
        if in_mesh(&o) {
            return Err(Error::InMesh.to_string());
        }
        // Saved first: if this device stops before it syncs, the rows come
        // back at the next start and the net layer still finds the mesh.
        save_rows(o.data_dir.as_deref(), &f.rows).map_err(|e| e.to_string())?;
        if let Err(e) = o.ident.adopt_person(&person, &seed) {
            if let Some(d) = &o.data_dir {
                let _ = std::fs::remove_file(d.join(ROWS_FILE));
            }
            return Err(e.to_string());
        }
        o.chain.repin(&person.id, &person.root_device_id);
        o.store.reload_verifier();
        learn_all(&o.chain, &evs);
        if let Some(sv) = &o.servers {
            for (id, addrs) in &f.servers {
                sv.hint(id, addrs);
            }
        }
        if let Some(k) = &o.kick {
            k();
        }
        Ok(())
    }

    /// Closes the window as paired.
    fn finish(&self) {
        let Some(p) = self.pairer() else { return };
        {
            let mut g = lock(&p.m);
            let mut f = lock(&self.f);
            if f.closed {
                return;
            }
            f.closed = true;
            drop(f);
            g.st.state = STATE_DONE.into();
            g.st.error.clear();
            g.st.code.clear();
            g.st.words.clear();
            if g.cur
                .as_ref()
                .is_some_and(|c| std::ptr::eq(Arc::as_ptr(c), self))
            {
                g.cur = None;
            }
            p.changed(&g);
        }
        self.close();
    }

    fn fail_spawn(self: &Arc<Self>, state: &'static str, why: &'static str, tell: &'static str) {
        // Mark closed now, so callers see the state at once; the tell frame
        // goes out in the background.
        let conn = self.mark_failed(state, why);
        if let Some(conn) = conn {
            let s = self.clone();
            if let Some(p) = self.pairer() {
                p.spawn(async move { s.tell_and_close(conn, tell).await });
            }
        }
    }

    /// Closes the window with `state` and why; `tell`, if set, is the frame
    /// the other side gets first.
    async fn fail(self: &Arc<Self>, state: &str, why: &str, tell: &str) {
        if let Some(conn) = self.mark_failed(state, why) {
            self.tell_and_close(conn, tell).await;
        }
    }

    /// `None` when already closed; else the connection, if any.
    fn mark_failed(&self, state: &str, why: &str) -> Option<Option<Arc<dyn Conn>>> {
        let p = self.pairer()?;
        let mut g = lock(&p.m);
        let mut f = lock(&self.f);
        if f.closed {
            return None;
        }
        f.closed = true;
        let conn = f.conn.clone();
        drop(f);
        g.st.state = state.into();
        g.st.error = why.into();
        g.st.code.clear();
        g.st.words.clear();
        if g.cur
            .as_ref()
            .is_some_and(|c| std::ptr::eq(Arc::as_ptr(c), self))
        {
            g.cur = None;
        }
        p.changed(&g);
        Some(conn)
    }

    async fn tell_and_close(&self, conn: Option<Arc<dyn Conn>>, tell: &str) {
        if !tell.is_empty()
            && let Some(c) = &conn
        {
            let b = Frame::of(tell).encode();
            let _ = tokio::time::timeout(Duration::from_secs(2), c.send(&b)).await;
        }
        self.close();
    }

    /// Leaves the pairing room and drops its connection.
    fn close(&self) {
        self.token.cancel();
        let conn = lock(&self.f).conn.clone();
        if let Some(c) = conn {
            c.close("pairing over");
        }
        if let Some(r) = self.room.get() {
            r.close();
        }
    }
}

/// Every `person.*` row held, oldest first.
pub(crate) fn person_rows(st: &Store) -> Result<Vec<WireRow>, String> {
    let mut out = Vec::new();
    let mut f = Filter {
        kinds: vec!["person.".into()],
        limit: store::MAX_LIST_LIMIT,
        ..Default::default()
    };
    loop {
        let evs = st.list(&f).map_err(|e| e.to_string())?;
        out.extend(evs.iter().map(WireRow::of));
        if (evs.len() as i64) < f.limit {
            return Ok(out);
        }
        f.after = evs.last().expect("non-empty").cursor();
    }
}
