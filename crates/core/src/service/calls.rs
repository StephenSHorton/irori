//! Calls on the Core (#654, docs/research/08-voice-calls/irori-fit.md).
//!
//! A huddle lives in a conversation: a space channel, a person DM (its
//! hidden space), or a Personal channel (the person's own devices, #663).
//! Everything about it rides that space, or for a Personal channel the
//! private mesh ([`Ride`]):
//! - ringing, joining, leaving and a heartbeat are board `call` frames to
//!   the space's devices that are online (`BoardMesh::send_call`), or
//!   `call` frames on the private mesh to the person's own devices
//!   (`mesh::Node::send_call`);
//! - the audio is one `irori/call/1` connection per pair of devices in the
//!   call, admitted by the space's verifier and a proof keyed on its topic
//!   key, or for a Personal channel by the person's chain (`ChainAdmit`'s
//!   check) and a proof keyed on the mesh key (`IrohNet::call_join` /
//!   `call_dial`), Opus frames in its QUIC datagrams (`call::Engine`);
//! - the start and the end are `talk.post` rows with a call mark, so the
//!   conversation keeps a record and an older Irori reads them as posts.
//!
//! Who dials: whoever joins dials every device already in the call. A
//! device rung later learns the roster from the frames.
//!
//! Transcripts (card #656, docs/design/call-transcripts.md): each device
//! writes down only its own person's voice. A transcriber hears the
//! engine's tap; each finished phrase goes live to the peers as a control
//! frame on the call stream, shows as a caption, and is stored as a
//! `call.segment` row, or, when the space couldn't take one as the call
//! began, kept for one fallback post at leave.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use coord::talk::{CallMark, NewPost, Post, TranscriptLineMark, TranscriptMark};
use iroh::endpoint::Connection;
use irori_identity::Chain;
use irori_net::call::{CallPeerState, ControlMsg, TranscriptLine};
use mesh::boardmesh::{BoardMesh, CallFrame, CallHooks, LivePeer};
use mesh::owncalls::OwnCalls;

use super::calls_over::{self, Heard, OverInput};
use super::calls_share::{ShareHub, ShareNote, ShareNow, ShareSink, ShareSource};
use super::{Core, Services};
use crate::events::CoreEvent;

/// A ring nobody answers stops after this long.
const RING_FOR: Duration = Duration::from_secs(60);
/// Each device in a call says so this often, so devices that come online
/// later see the huddle and a crashed device drops out.
const HEARTBEAT: Duration = Duration::from_secs(15);
/// A live call nobody has said anything about for this long is gone.
const LIVE_FOR: Duration = calls_over::QUIET_FOR;
/// While ringing, the ring goes out again this often.
const RING_AGAIN: Duration = Duration::from_secs(5);
/// The most devices a call holds (the card says 2 to 6; a little room).
const MAX_DEVICES: usize = 8;
/// How often the window hears about a call in progress (health, speaking).
const TICK: Duration = Duration::from_millis(400);
/// The captions a call keeps for the window.
const CAPTIONS: usize = 40;
/// A line's text is cut to this many bytes, so its frame fits
/// [`irori_net::call::CONTROL_MAX`].
const LINE_MAX: usize = 12 * 1024;
/// Control messages queued for one peer before new ones are dropped.
const CTL_QUEUE: usize = 64;
/// How long a leave waits for the last lines to be written.
const DRAIN_FOR: Duration = Duration::from_secs(10);
/// A fallback post's text is at most this long; more lines go in another.
const FALLBACK_MAX: usize = 64 * 1024;

/// The call state on [`Services`].
#[derive(Default)]
pub(crate) struct Calls {
    inner: Mutex<Inner>,
    /// The phone's microphone service (Android); `None` on desktops.
    platform: Option<Arc<dyn call::Platform>>,
    audio: Option<call::device::OpenerFn>,
    /// Starts a transcriber (card #656); `None`: this device can't.
    transcriber: Option<call::TranscriberFn>,
}

impl Calls {
    pub(crate) fn new(
        platform: Option<Arc<dyn call::Platform>>,
        audio: Option<call::device::OpenerFn>,
        transcriber: Option<call::TranscriberFn>,
    ) -> Self {
        Calls {
            inner: Mutex::default(),
            platform,
            audio,
            transcriber,
        }
    }
}

#[derive(Default)]
struct Inner {
    mine: Option<Active>,
    ringing: BTreeMap<String, Ring>,
    live: BTreeMap<String, Live>,
    /// When this device last heard each huddle live, kept for
    /// [`Services::call_over`]: written beside `live`, never pruned by its
    /// expiry (card #1043).
    heard: Heard,
    dismissed: BTreeSet<String>,
    /// A start or join is opening the devices and the call: no second
    /// one may begin until it is in (or failed).
    entering: bool,
    /// The phone's Hang up or a failure while entering, for when it's in.
    pending_hangup: bool,
    pending_error: Option<String>,
    /// Counts calls entered: a ticker serves only its own.
    generation: u64,
}

/// Clears [`Inner::entering`] however `call_enter` ends.
struct Entering<'a>(&'a Calls);

impl Drop for Entering<'_> {
    fn drop(&mut self) {
        let mut g = self.0.inner.lock().expect("calls");
        g.entering = false;
        g.pending_hangup = false;
        g.pending_error = None;
    }
}

struct Ring {
    r: Ringing,
    devices: Vec<String>,
    until: SystemTime,
}

struct Live {
    conv_id: String,
    space_id: String,
    /// When it started (the earliest ring heard), Unix ms; 0 if only joins
    /// were heard.
    started_ms: i64,
    /// Device id to person id.
    devices: BTreeMap<String, String>,
    heard: SystemTime,
}

struct Active {
    call_id: String,
    conv_id: String,
    space_id: String,
    joined_at_ms: i64,
    /// When the call started, Unix ms (for the end post's length).
    started_at_ms: i64,
    /// This device started the call: it keeps ringing for a while.
    started_here: bool,
    muted: bool,
    /// Nothing plays here and the mic is off; `muted` is kept for after.
    deafened: bool,
    /// Which call this is, of those this device entered.
    generation: u64,
    engine: Arc<call::Engine>,
    handle: irori_net::call::CallHandle,
    /// The call's screen share, both ends (#658).
    share: Arc<ShareHub>,
    mesh: Ride,
    peers: BTreeMap<String, Slot>,
    /// Person ids of everyone seen in the call, this person included.
    people: BTreeSet<String>,
    error: Option<String>,
    /// This call's transcript.
    script: Arc<Script>,
    /// The transcriber running now.
    scribe: Option<Scribe>,
    /// A transcriber failed to start: not tried again until the switch
    /// goes off and on.
    scribe_failed: bool,
}

struct Slot {
    person_id: String,
    conn: Option<Connection>,
    gone: bool,
    /// The connection's control writer (card #656).
    ctl: Option<tokio::sync::mpsc::Sender<ControlMsg>>,
    /// Whether it writes down what its person says; None until it says.
    transcribing: Option<bool>,
    /// Its lines already heard, by seq.
    seen: BTreeSet<u64>,
}

impl Slot {
    fn new(person_id: String) -> Slot {
        Slot {
            person_id,
            conn: None,
            gone: false,
            ctl: None,
            transcribing: None,
            seen: BTreeSet::new(),
        }
    }
}

/// A call's transcript (card #656), shared by the call, the thread that
/// writes this device's lines and the leave.
struct Script {
    /// The calls gate as read on entering, for the whole call: rows in the
    /// log, else the lines are kept for one fallback post at leave.
    rows: bool,
    /// Held while a transcriber starts or stops, so a leave waits for it.
    toggle: Mutex<()>,
    /// The call was left: a line the writer gets now (one the drain
    /// didn't wait for) is dropped, never written after the end post.
    closed: AtomicBool,
    /// Lines dropped since `closed`.
    dropped: AtomicU64,
    st: Mutex<ScriptState>,
}

#[derive(Default)]
struct ScriptState {
    /// This device's next line's seg and seq: after its lines already in
    /// the call, so a rejoin never reuses a seg.
    next: u64,
    /// The newest lines, oldest first.
    captions: VecDeque<Caption>,
    /// Some line was heard in this call, this device's or a peer's.
    heard: bool,
    /// This device's lines for the fallback post.
    fallback: Vec<TranscriptLineMark>,
}

impl Script {
    fn new(rows: bool, next: u64) -> Script {
        Script {
            rows,
            toggle: Mutex::new(()),
            closed: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
            st: Mutex::new(ScriptState {
                next,
                ..ScriptState::default()
            }),
        }
    }

    /// Whether the call was left; counts a line dropped if so.
    fn drop_late(&self) -> bool {
        let closed = self.closed.load(Ordering::SeqCst);
        if closed {
            self.dropped.fetch_add(1, Ordering::SeqCst);
        }
        closed
    }

    fn caption(&self, c: Caption) {
        let mut st = self.st.lock().expect("script");
        st.heard = true;
        if st.captions.len() >= CAPTIONS {
            st.captions.pop_front();
        }
        st.captions.push_back(c);
    }
}

/// A transcriber made but not yet on a tap (card #656): stopped if
/// dropped, so a call that fails to come up doesn't leak it.
struct Opened {
    t: Option<Box<dyn call::Transcriber>>,
    tx: std::sync::mpsc::Sender<ScribeMsg>,
    rx: Option<std::sync::mpsc::Receiver<ScribeMsg>>,
}

impl Drop for Opened {
    fn drop(&mut self) {
        if let Some(t) = self.t.take() {
            t.stop();
        }
    }
}

/// A running transcriber and the thread that writes its lines.
struct Scribe {
    t: Box<dyn call::Transcriber>,
    tx: std::sync::mpsc::Sender<ScribeMsg>,
    writer: std::thread::JoinHandle<()>,
}

enum ScribeMsg {
    Line(call::Segment),
    /// The transcriber stopped: nothing comes after.
    Done,
}

impl Scribe {
    /// Stops the transcriber (its last phrase is queued before this
    /// returns) and waits a while for the writer to finish every line.
    /// Blocks: never under the calls lock. Take the tap off first. A
    /// leave then closes the script, so a line still queued is dropped.
    fn finish(self) {
        self.t.stop();
        let _ = self.tx.send(ScribeMsg::Done);
        drop(self.tx);
        let until = std::time::Instant::now() + DRAIN_FOR;
        while !self.writer.is_finished() && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        if self.writer.is_finished() {
            let _ = self.writer.join();
        } else {
            crate::logf(format_args!("call: the transcript writer didn't finish"));
        }
    }
}

/// `t` cut to [`LINE_MAX`] bytes on a char boundary.
fn cap_line(t: &str) -> &str {
    if t.len() <= LINE_MAX {
        return t;
    }
    let mut i = LINE_MAX;
    while !t.is_char_boundary(i) {
        i -= 1;
    }
    &t[..i]
}

/// What a call's frames ride and who it admits: a space's board mesh, or
/// for a Personal channel (space id "") the private mesh among the
/// person's own devices (card #663).
#[derive(Clone)]
enum Ride {
    Space(Arc<BoardMesh>),
    Own {
        node: Arc<mesh::Node>,
        chain: Arc<Chain>,
        /// This device's id.
        me: String,
    },
}

impl Ride {
    /// The devices connected now that said hello, with the person each
    /// counts as.
    fn live(&self) -> Vec<LivePeer> {
        match self {
            Ride::Space(m) => m.live_peers(),
            Ride::Own { node, chain, .. } => {
                let person = chain.person_id();
                node.own_peers()
                    .into_iter()
                    .map(|p| LivePeer {
                        device_id: p.device_id,
                        device_pub: p.device_pub,
                        person_id: person.clone(),
                        dms: 0,
                        calls: p.calls,
                    })
                    .collect()
            }
        }
    }

    /// Every device the call may hold, counted or certified: (device id,
    /// person id, key hex).
    fn members(&self) -> Vec<(String, String, String)> {
        match self {
            Ride::Space(m) => m
                .member_devices()
                .into_iter()
                .map(|d| (d.device_id, d.person_id, d.device_pub))
                .collect(),
            Ride::Own { chain, .. } => chain
                .devices()
                .into_iter()
                .filter(|d| !d.revoked)
                .map(|d| (d.cert.device_id, d.cert.person_id, d.cert.device_pub))
                .collect(),
        }
    }

    /// The keys a call connection is proved with, the one to dial with
    /// first.
    fn keys(&self) -> Vec<(i64, [u8; 32])> {
        match self {
            Ride::Space(m) => m.topic_keys(),
            Ride::Own { node, .. } => node.call_keys(),
        }
    }

    /// Whether device `peer` with key `eid` may be in the call: a counted
    /// member device whose key matches, or one of the person's own devices
    /// live in the space; for a Personal channel, a device the person's
    /// chain certifies with that key and hasn't revoked (`ChainAdmit`).
    fn accepts(&self, peer: &str, eid: &iroh::EndpointId) -> bool {
        match self {
            Ride::Space(m) => {
                let k = hex::encode(eid.as_bytes());
                m.member_devices()
                    .iter()
                    .any(|d| d.device_id == peer && d.device_pub == k)
                    || m.live_peers()
                        .iter()
                        .any(|d| d.device_id == peer && d.device_pub == k)
            }
            Ride::Own { chain, me, .. } => {
                irori_net::admit::check_device(chain, me, peer, eid.as_bytes()).is_ok()
            }
        }
    }

    /// Sends `f` to `device` alone.
    async fn send(&self, device: &str, f: CallFrame) -> Result<(), String> {
        match self {
            Ride::Space(m) => m.send_call(device, f).await,
            Ride::Own { node, .. } => node.send_call(device, &f),
        }
    }

    /// The person `device` counts as here, if known.
    fn person_of(&self, device: &str) -> Option<String> {
        self.members()
            .into_iter()
            .find(|(d, _, _)| d == device)
            .map(|(_, p, _)| p)
            .or_else(|| {
                self.live()
                    .into_iter()
                    .find(|p| p.device_id == device)
                    .map(|p| p.person_id)
            })
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn new_call_id() -> Result<String, String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    Ok(hex::encode(b))
}

/// Runs `f` on the net runtime and waits for it, from a thread that isn't
/// one of that runtime's own.
fn wait_on<T: Send + 'static>(
    h: &tokio::runtime::Handle,
    f: impl std::future::Future<Output = T> + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    h.spawn(async move {
        let _ = tx.send(f.await);
    });
    rx.recv().map_err(|_| "the network stopped".to_string())
}

/// Runs future `f()` on a thread of its own with a current-thread runtime:
/// capture's futures (ScreenCaptureKit, WGC) aren't `Send`.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn on_own_thread<T, F, Fut>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = T>,
{
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map(|rt| rt.block_on(f()))
            .map_err(|e| e.to_string())
    })
    .join()
    .map_err(|_| "the screen capture crashed".to_string())?
}

/// A peer's datagrams, for the engine.
struct Datagrams(Connection);

impl call::Link for Datagrams {
    fn send(&self, d: &[u8]) -> bool {
        self.0.send_datagram(d.to_vec().into()).is_ok()
    }
}

/// The mesh's call frames into the Core.
pub(super) struct CallBridge(pub(super) Weak<Services>);

impl CallHooks for CallBridge {
    fn hello(&self, _via: &str, _peer: &LivePeer) {}

    fn frame(&self, via: &str, peer: &LivePeer, frame: CallFrame) {
        let Some(s) = self.0.upgrade() else { return };
        let (via, peer) = (via.to_string(), peer.clone());
        // The connection's task must not wait on our locks for long; the
        // work is small, but keep it off that task anyway.
        let h = s.pairing.handle.clone();
        h.spawn_blocking(move || s.on_call_frame(&via, &peer, frame));
    }
}

/// Call frames from the person's own devices on the private mesh (card
/// #663): a Personal channel's huddle.
impl OwnCalls for CallBridge {
    fn frame(&self, peer: &str, peer_pub: &str, frame: CallFrame) {
        let Some(s) = self.0.upgrade() else { return };
        let peer = LivePeer {
            device_id: peer.to_string(),
            device_pub: peer_pub.to_string(),
            person_id: s.person_id(),
            dms: 0,
            calls: mesh::frames::CALLS_PROTOCOL,
        };
        let h = s.pairing.handle.clone();
        h.spawn_blocking(move || s.on_call_frame("", &peer, frame));
    }
}

impl Services {
    fn calls_changed(&self) {
        self.hub.publish(CoreEvent::Call);
    }

    /// The space a huddle in `conv` belongs to.
    fn call_space(&self, conv: &str) -> Result<String, String> {
        let conv = conv.trim();
        if coord::talk::is_dm(conv) {
            let Some(other) = self.talk.pair_other(conv) else {
                return Err("a DM with an agent has no huddle".into());
            };
            let id = other
                .strip_prefix(store::kinds::PERSON_ACTOR_PREFIX)
                .unwrap_or_default();
            return self
                .dm_writer(id)
                .ok_or_else(|| "this DM hasn't started yet: write in it first".to_string());
        }
        if !coord::talk::is_channel(conv) {
            return Err("a huddle starts in a channel or a DM".into());
        }
        let ch = self
            .talk
            .channel(conv)
            .ok_or_else(|| format!("no channel {conv}"))?;
        // A Personal channel's huddle is the person's own devices (#663).
        Ok(ch.space_id)
    }

    /// What a call in space `space` rides: its board mesh, or for "" (a
    /// Personal channel) the private mesh.
    fn ride(&self, space: &str) -> Option<Ride> {
        if space.is_empty() {
            return Some(Ride::Own {
                node: self.mesh.node.clone(),
                chain: self.chain.clone(),
                me: self.device.id.clone(),
            });
        }
        self.mesh.board(space).map(Ride::Space)
    }

    fn frame(
        &self,
        call_id: &str,
        conv_id: &str,
        space_id: &str,
        op: &str,
        devices: Vec<String>,
    ) -> CallFrame {
        CallFrame {
            space_id: space_id.to_string(),
            call_id: call_id.to_string(),
            op: op.to_string(),
            conv_id: conv_id.to_string(),
            epoch: 0,
            devices,
            at_ms: now_ms(),
        }
    }

    /// Sends `f` to every device of the space (or the person's own, for a
    /// Personal channel) online now that speaks calls.
    fn tell_space(&self, m: &Ride, f: CallFrame) {
        let m = m.clone();
        self.pairing.handle.spawn(async move {
            for p in m.live() {
                if p.calls >= mesh::boardmesh::CALLS_PROTOCOL {
                    let _ = m.send(&p.device_id, f.clone()).await;
                }
            }
        });
    }

    fn on_call_frame(self: &Arc<Self>, via: &str, peer: &LivePeer, mut f: CallFrame) {
        if f.space_id != via || !irori_net::call::is_call_id(&f.call_id) {
            return;
        }
        // A frame names its conversation: it must be one of this space's,
        // or a member could make a huddle show (and its posts land) in a
        // conversation of another space or a DM.
        if matches!(f.op.as_str(), "ring" | "join")
            && self.call_space(&f.conv_id).ok().as_deref() != Some(via)
        {
            return;
        }
        let Some(m) = self.ride(via) else {
            return;
        };
        // Only devices of this space (the person's own, in a Personal
        // channel), and no more than a call holds.
        let known: BTreeMap<String, String> = m
            .members()
            .into_iter()
            .map(|(d, p, _)| (d, p))
            .chain(m.live().into_iter().map(|p| (p.device_id, p.person_id)))
            .collect();
        f.devices.retain(|d| known.contains_key(d));
        f.devices.dedup();
        f.devices.truncate(MAX_DEVICES);
        // A start time far from now is a wrong clock or a lie: ignore it.
        let now_ms = now_ms();
        if (f.at_ms - now_ms).abs() > 2 * RING_FOR.as_millis() as i64 {
            f.at_ms = 0;
        }
        let me_person = self.person_id();
        let now = SystemTime::now();
        let mut changed = true;
        let mut expire = None;
        // Devices of my call I've just heard of: dial them (whoever joined
        // may not have known the whole roster).
        let mut dial = Vec::new();
        let mut keep_heard = false;
        {
            let mut g = self.calls.inner.lock().expect("calls");
            let mine = g.mine.as_ref().is_some_and(|a| a.call_id == f.call_id);
            match f.op.as_str() {
                "ring" | "join" => {
                    if !g.heard.ticking {
                        g.heard.start(now);
                        keep_heard = true;
                    }
                    g.heard.hear(&f.call_id, now);
                    let live = g.live.entry(f.call_id.clone()).or_insert_with(|| Live {
                        conv_id: f.conv_id.clone(),
                        space_id: f.space_id.clone(),
                        started_ms: 0,
                        devices: BTreeMap::new(),
                        heard: now,
                    });
                    live.heard = now;
                    if f.op == "ring"
                        && f.at_ms > 0
                        && (live.started_ms == 0 || f.at_ms < live.started_ms)
                    {
                        live.started_ms = f.at_ms;
                    }
                    live.devices
                        .insert(peer.device_id.clone(), peer.person_id.clone());
                    expire = Some(LIVE_FOR);
                    if f.op == "join" && peer.person_id == me_person && !via.is_empty() {
                        // Answered on another of this person's devices. Not
                        // in a Personal channel: there every device is the
                        // person's, and each is rung to join (#663).
                        g.ringing.remove(&f.call_id);
                    } else if f.op == "ring"
                        && !mine
                        && !g.dismissed.contains(&f.call_id)
                        && !g.ringing.contains_key(&f.call_id)
                    {
                        g.ringing.insert(
                            f.call_id.clone(),
                            Ring {
                                r: Ringing {
                                    call_id: f.call_id.clone(),
                                    conv_id: f.conv_id.clone(),
                                    space_id: f.space_id.clone(),
                                    by_device: peer.device_id.clone(),
                                    by_person: peer.person_id.clone(),
                                    at_ms: if f.at_ms > 0 { f.at_ms } else { now_ms },
                                },
                                devices: f.devices.clone(),
                                until: now + RING_FOR,
                            },
                        );
                        expire = Some(RING_FOR);
                    } else if let Some(r) = g.ringing.get_mut(&f.call_id) {
                        for d in &f.devices {
                            if !r.devices.contains(d) && r.devices.len() < MAX_DEVICES {
                                r.devices.push(d.clone());
                            }
                        }
                    }
                    if mine && let Some(a) = g.mine.as_mut() {
                        a.people.insert(peer.person_id.clone());
                        let me = &self.device.id;
                        let named = std::iter::once(peer.device_id.clone())
                            .chain(f.devices.iter().cloned());
                        for d in named.filter(|d| d != me) {
                            if a.peers.len() >= MAX_DEVICES && !a.peers.contains_key(&d) {
                                continue;
                            }
                            let slot = a.peers.entry(d.clone()).or_insert_with(|| {
                                Slot::new(known.get(&d).cloned().unwrap_or_default())
                            });
                            // A device that left comes back only on its own
                            // word, never on a roster someone else sent.
                            if d == peer.device_id {
                                slot.gone = false;
                            }
                            if slot.conn.is_none() && !slot.gone {
                                dial.push(d);
                            }
                        }
                    }
                }
                "leave" => {
                    if let Some(l) = g.live.get_mut(&f.call_id) {
                        l.devices.remove(&peer.device_id);
                        if l.devices.is_empty() {
                            g.live.remove(&f.call_id);
                        }
                    }
                    if mine
                        && let Some(a) = g.mine.as_mut()
                        && let Some(s) = a.peers.get_mut(&peer.device_id)
                    {
                        s.gone = true;
                    }
                }
                "end" => {
                    // Only from a device that was in it (or rang it).
                    let in_it = g
                        .live
                        .get(&f.call_id)
                        .is_some_and(|l| l.devices.contains_key(&peer.device_id))
                        || g.ringing
                            .get(&f.call_id)
                            .is_some_and(|r| r.r.by_device == peer.device_id);
                    if in_it {
                        g.live.remove(&f.call_id);
                        g.ringing.remove(&f.call_id);
                    } else {
                        changed = false;
                    }
                }
                _ => changed = false,
            }
        }
        for d in dial {
            self.call_dial(&f.call_id, d);
        }
        if keep_heard {
            self.heard_ticker();
        }
        if matches!(f.op.as_str(), "ring" | "join" | "leave" | "end") {
            // The person's assistant follows the call (card #1049).
            self.huddle_host_kick();
        }
        if let Some(d) = expire {
            self.publish_after(d + Duration::from_secs(1));
        }
        if changed {
            self.calls_changed();
        }
    }

    /// Keeps the heard record: looks at this device's own network every few
    /// seconds, and ends when nothing is left in the record (the next ring
    /// or join starts it again).
    fn heard_ticker(self: &Arc<Self>) {
        let w = Arc::downgrade(self);
        self.pairing.handle.spawn(async move {
            loop {
                tokio::time::sleep(calls_over::TICK_EVERY).await;
                let Some(s) = w.upgrade() else { return };
                let up = s.mesh.net.link_up();
                let mut g = s.calls.inner.lock().expect("calls");
                if !g.heard.tick(SystemTime::now(), up) {
                    g.heard.ticking = false;
                    return;
                }
            }
        });
    }

    /// Whether huddle `call_id` is over, as this device can tell (card
    /// #1043, call-agents.md §3): an end mark is in the rows; or this device
    /// heard it live, then nothing for [`calls_over::QUIET_FOR`] while its
    /// own network stayed up, with no line in the last two minutes (in a
    /// two-device space the survivor of a crash has nobody else to see); or
    /// it never heard it live and its newest row is over ten minutes old.
    /// Otherwise live: not knowing counts as live, so a device that starts
    /// or reconnects mid-call never calls it over. The one answer every
    /// rule that says "after the call" uses.
    pub(crate) fn call_over(&self, call_id: &str) -> bool {
        let ms = |t: gocompat::time::GoTime| {
            let n = t.unix_milli_value();
            (n > 0).then(|| UNIX_EPOCH + Duration::from_millis(n as u64))
        };
        let info = self.talk.calls().into_iter().find(|c| c.call_id == call_id);
        let g = self.calls.inner.lock().expect("calls");
        calls_over::over(&OverInput {
            now: SystemTime::now(),
            ended: info.as_ref().is_some_and(|c| c.end.is_some()),
            in_it: g.mine.as_ref().is_some_and(|a| a.call_id == call_id),
            heard: g.heard.of(call_id),
            online_since: g.heard.online_since(),
            last_line: info.as_ref().and_then(|c| c.last_line).and_then(ms),
            newest_row: info.as_ref().and_then(|c| ms(c.last)),
        })
    }

    /// The huddles whose call has `person` in it, as this device knows:
    /// (callId, convId, spaceId). Live ones heard within [`LIVE_FOR`], and
    /// the one this device is in (card #1049).
    pub(super) fn calls_with_person(&self, person: &str) -> Vec<(String, String, String)> {
        let now = SystemTime::now();
        let me = self.person_id();
        let g = self.calls.inner.lock().expect("calls");
        let mut out: Vec<(String, String, String)> = g
            .live
            .iter()
            .filter(|(_, l)| now.duration_since(l.heard).unwrap_or_default() < LIVE_FOR)
            .filter(|(_, l)| l.devices.values().any(|p| p == person))
            .map(|(id, l)| (id.clone(), l.conv_id.clone(), l.space_id.clone()))
            .collect();
        if person == me
            && let Some(a) = g.mine.as_ref()
            && !out.iter().any(|(id, _, _)| *id == a.call_id)
        {
            out.push((a.call_id.clone(), a.conv_id.clone(), a.space_id.clone()));
        }
        out
    }

    /// The devices of space `space` reachable now ("" the person's own), or
    /// None when that space isn't open here (card #1049).
    pub(super) fn space_devices_live(&self, space: &str) -> Option<BTreeSet<String>> {
        Some(
            self.ride(space)?
                .live()
                .into_iter()
                .map(|p| p.device_id)
                .collect(),
        )
    }

    /// Huddle `call_id` as the tools show it to an agent (card #1055):
    /// whether it is over, who is in it now and whose assistant is
    /// connected. `None` for a huddle this device doesn't know.
    pub(crate) fn huddle_now(&self, call_id: &str) -> Option<tools::HuddleNow> {
        self.talk.call_conv(call_id)?;
        let over = self.call_over(call_id);
        if over {
            return Some(tools::HuddleNow {
                over,
                ..tools::HuddleNow::default()
            });
        }
        let me = self.person_id();
        let me_key = self.user_sender().key;
        let people = self
            .call_people_now(call_id)
            .0
            .into_iter()
            .map(|p| {
                if p == me {
                    me_key.clone()
                } else {
                    format!("{}{p}", store::kinds::PERSON_ACTOR_PREFIX)
                }
            })
            .collect();
        let assistants = self
            .call_assistants(call_id)
            .into_iter()
            .map(|a| (a.person, a.state))
            .collect();
        Some(tools::HuddleNow {
            over,
            people,
            assistants,
        })
    }

    /// The person ids in huddle `call_id` now, as this device knows (card
    /// #1049), and its space.
    pub(super) fn call_people_now(&self, call_id: &str) -> (Vec<String>, String) {
        let now = SystemTime::now();
        let g = self.calls.inner.lock().expect("calls");
        let mut out: Vec<String> = Vec::new();
        let mut space = String::new();
        if let Some(l) = g
            .live
            .get(call_id)
            .filter(|l| now.duration_since(l.heard).unwrap_or_default() < LIVE_FOR)
        {
            space = l.space_id.clone();
            for p in l.devices.values() {
                if !out.contains(p) {
                    out.push(p.clone());
                }
            }
        }
        if let Some(a) = g.mine.as_ref().filter(|a| a.call_id == call_id) {
            space = a.space_id.clone();
            let me = self.person_id();
            if !out.contains(&me) {
                out.push(me);
            }
        }
        (out, space)
    }

    /// Writes huddle `call_id`'s title as this device's person
    /// (`assistant`: proposed by their assistant, which a person's title
    /// beats). Returns the title the huddle now shows. A refused write is
    /// logged too, so a space that isn't ready leaves a trace.
    pub(crate) fn call_title_set(
        &self,
        call_id: &str,
        title: &str,
        assistant: bool,
    ) -> Result<String, String> {
        let conv = self
            .talk
            .call_conv(call_id)
            .ok_or_else(|| format!("no huddle {call_id} here"))?;
        let r = self
            .talk
            .write_title(&conv, call_id, title, assistant, &self.user_sender().key)
            .map_err(|e| e.to_string());
        match &r {
            Ok(_) => self.hub.publish(CoreEvent::Talk),
            Err(e) => crate::logf(format_args!("call {call_id}: title: {e}")),
        }
        r
    }

    /// Tells the window again after `d`: rings and live huddles expire by
    /// time, and nothing else would make it look.
    fn publish_after(self: &Arc<Self>, d: Duration) {
        let w = Arc::downgrade(self);
        self.pairing.handle.spawn(async move {
            tokio::time::sleep(d).await;
            if let Some(s) = w.upgrade() {
                s.calls_changed();
            }
        });
    }

    pub(crate) fn call_snapshot(&self) -> CallSnapshot {
        let now = SystemTime::now();
        let mut g = self.calls.inner.lock().expect("calls");
        g.ringing.retain(|_, r| r.until > now);
        g.live
            .retain(|_, l| now.duration_since(l.heard).unwrap_or_default() < LIVE_FOR);
        let mine = g.mine.as_ref().map(|a| {
            let st = a.engine.stats();
            let peers = a
                .peers
                .iter()
                .map(|(id, s)| {
                    let es = st.peers.iter().find(|p| &p.id == id);
                    let (path, rtt_ms) = match &s.conn {
                        Some(c) if !s.gone => {
                            let r = irori_net::path::report(c);
                            let kind = match r.kind {
                                irori_net::path::PathKind::Direct => "direct",
                                irori_net::path::PathKind::Relay => "relay",
                                irori_net::path::PathKind::None => "",
                            };
                            (
                                kind.to_string(),
                                r.rtt.map_or(0.0, |d| d.as_secs_f64() * 1000.0),
                            )
                        }
                        _ => (String::new(), 0.0),
                    };
                    let loss_pct = es.map_or(0.0, |p| {
                        let j = &p.jitter;
                        let total = j.received + j.lost;
                        if total == 0 {
                            0.0
                        } else {
                            j.lost as f64 * 100.0 / total as f64
                        }
                    });
                    CallPeer {
                        device_id: id.clone(),
                        person_id: s.person_id.clone(),
                        phase: if s.gone {
                            "gone"
                        } else if s.conn.is_some() {
                            "live"
                        } else {
                            "connecting"
                        }
                        .into(),
                        path,
                        rtt_ms,
                        loss_pct,
                        speaking: es.is_some_and(|p| p.speaking),
                        muted: es.is_some_and(|p| p.jitter.muted),
                        transcribing: s.transcribing,
                    }
                })
                .collect();
            let captions = a
                .script
                .st
                .lock()
                .expect("script")
                .captions
                .iter()
                .cloned()
                .collect();
            let (share_now, share_note) = a.share.view();
            let share = share_now.map(|n| {
                let slot = a.peers.get(&n.device_id);
                let path = slot
                    .and_then(|s| s.conn.as_ref().filter(|_| !s.gone))
                    .map(|c| match irori_net::path::report(c).kind {
                        irori_net::path::PathKind::Direct => "direct",
                        irori_net::path::PathKind::Relay => "relay",
                        irori_net::path::PathKind::None => "",
                    })
                    .unwrap_or("")
                    .to_string();
                CallShare {
                    person_id: if n.mine {
                        self.person_id()
                    } else {
                        slot.map(|s| s.person_id.clone()).unwrap_or_default()
                    },
                    now: n,
                    path,
                }
            });
            let share_note = share_note.map(|n| match n {
                ShareNote::Replaced(d) => CallShareNote::Replaced {
                    person_id: a
                        .peers
                        .get(&d)
                        .map(|s| s.person_id.clone())
                        .unwrap_or_default(),
                    device_id: d,
                },
                ShareNote::Failed(why) => CallShareNote::Failed(why),
            });
            MyCall {
                share,
                share_note,
                can_share: CAN_SHARE,
                call_id: a.call_id.clone(),
                conv_id: a.conv_id.clone(),
                space_id: a.space_id.clone(),
                joined_at_ms: a.joined_at_ms,
                muted: a.muted,
                deafened: a.deafened,
                speaking: st.speaking,
                peers,
                transcribing: a.scribe.is_some(),
                can_transcribe: self.calls.transcriber.is_some(),
                captions,
                error: a.error.clone().or_else(|| {
                    st.device_failed.then(|| {
                        "The microphone or speaker stopped: leave the huddle and join again"
                            .to_string()
                    })
                }),
            }
        });
        let mine_id = g.mine.as_ref().map(|a| a.call_id.clone());
        CallSnapshot {
            mine,
            ringing: g
                .ringing
                .values()
                .filter(|r| Some(&r.r.call_id) != mine_id.as_ref())
                .map(|r| r.r.clone())
                .collect(),
            live: g
                .live
                .iter()
                .map(|(id, l)| LiveCall {
                    call_id: id.clone(),
                    conv_id: l.conv_id.clone(),
                    space_id: l.space_id.clone(),
                    devices: l
                        .devices
                        .iter()
                        .map(|(d, p)| (d.clone(), p.clone()))
                        .collect(),
                })
                .collect(),
        }
    }

    pub(crate) fn call_start(self: &Arc<Self>, conv: &str) -> Result<String, String> {
        let conv = conv.trim().to_string();
        let space = self.call_space(&conv)?;
        let call_id = new_call_id()?;
        self.call_enter(&call_id, &conv, &space, Vec::new(), true, now_ms())?;
        let mark = CallMark {
            call_id: call_id.clone(),
            op: "start".into(),
            ..CallMark::default()
        };
        if let Err(e) = self.post_call_mark(&conv, "started a huddle", mark) {
            crate::logf(format_args!("call {call_id}: start post: {e}"));
        }
        let m = self.ride(&space).ok_or("this space isn't open here")?;
        let me = self.device.id.clone();
        self.tell_space(&m, self.frame(&call_id, &conv, &space, "ring", vec![me]));
        Ok(call_id)
    }

    pub(crate) fn call_join(self: &Arc<Self>, call_id: &str) -> Result<(), String> {
        let (conv, space, devices, started) = {
            let mut g = self.calls.inner.lock().expect("calls");
            if g.mine.as_ref().is_some_and(|a| a.call_id == call_id) {
                return Ok(());
            }
            let ring = g.ringing.remove(call_id);
            match (ring, g.live.get(call_id)) {
                (Some(r), live) => {
                    let mut d = r.devices.clone();
                    d.extend(
                        live.map(|l| l.devices.keys().cloned().collect::<Vec<_>>())
                            .unwrap_or_default(),
                    );
                    let started = live.map_or(0, |l| l.started_ms);
                    let started = if started > 0 {
                        started.min(r.r.at_ms)
                    } else {
                        r.r.at_ms
                    };
                    (r.r.conv_id, r.r.space_id, d, started)
                }
                (None, Some(l)) => (
                    l.conv_id.clone(),
                    l.space_id.clone(),
                    l.devices.keys().cloned().collect(),
                    l.started_ms,
                ),
                (None, None) => return Err("that huddle has ended".into()),
            }
        };
        let started = if started > 0 { started } else { now_ms() };
        // The conversation must still be one of the call's space.
        if self.call_space(&conv)? != space {
            return Err("that huddle isn't in this conversation's space".into());
        }
        self.call_enter(call_id, &conv, &space, devices, false, started)
    }

    /// Opens the devices, joins the call on the endpoint, tells the space
    /// and dials `devices`.
    fn call_enter(
        self: &Arc<Self>,
        call_id: &str,
        conv: &str,
        space: &str,
        devices: Vec<String>,
        started_here: bool,
        started_at_ms: i64,
    ) -> Result<(), String> {
        {
            let mut g = self.calls.inner.lock().expect("calls");
            if g.mine.is_some() || g.entering {
                return Err("you're already in a huddle: leave it first".into());
            }
            g.entering = true;
        }
        let _entering = Entering(&self.calls);
        let m = self.ride(space).ok_or("this space isn't open here")?;
        let keys = m.keys();
        if keys.is_empty() {
            return Err(if space.is_empty() {
                "your devices have no mesh key yet: try again in a moment".into()
            } else {
                "this space has no key yet: try again in a moment".into()
            });
        }

        // The transcriber before the mic opens or anyone can connect: a
        // model may take seconds to load, and nothing is locked here.
        let want_scribe = self.call_transcribe();
        let opened = if want_scribe {
            self.scribe_open(call_id)
        } else {
            None
        };

        // The phone first: Android only lets the mic run on with its
        // service up, and only while Irori is on screen.
        let platform = self.calls.platform.clone();
        if let Some(p) = &platform {
            let w = Arc::downgrade(self);
            let sink: call::PlatformSink = Arc::new(move |e| {
                if let Some(s) = w.upgrade() {
                    match e {
                        call::PlatformEvent::HangUp => {
                            let mut g = s.calls.inner.lock().expect("calls");
                            if g.mine.is_none() && g.entering {
                                // Still coming up: hang up once it's in.
                                g.pending_hangup = true;
                                return;
                            }
                            drop(g);
                            std::thread::spawn(move || {
                                let _ = s.call_leave();
                            });
                        }
                        call::PlatformEvent::Failed(why) => {
                            {
                                let mut g = s.calls.inner.lock().expect("calls");
                                match g.mine.as_mut() {
                                    Some(a) => a.error = Some(why),
                                    None => g.pending_error = Some(why),
                                }
                            }
                            s.calls_changed();
                        }
                    }
                }
            });
            p.begin(&self.call_title(conv), sink)
                .map_err(|e| e.to_string())?;
        }
        let open = match &self.calls.audio {
            Some(f) => f(),
            None => call::device::default_devices(),
        };
        let engine = match call::Engine::start(call::Config::default(), open) {
            Ok(e) => Arc::new(e),
            Err(e) => {
                if let Some(p) = &platform {
                    p.end();
                }
                return Err(format!("couldn't open the microphone or speaker: {e}"));
            }
        };

        let accept_mesh = m.clone();
        let accept: irori_net::call::CallAccept =
            Arc::new(move |peer, eid| accept_mesh.accepts(peer, eid));
        let w = Arc::downgrade(self);
        let cid = call_id.to_string();
        let on_peer: irori_net::call::OnCallPeer = Arc::new(move |cc| {
            if let Some(s) = w.upgrade() {
                s.call_peer(&cid, cc);
            }
        });
        let join = irori_net::call::CallJoin {
            call_id: call_id.to_string(),
            space_id: space.to_string(),
            keys,
            accept,
            on_peer,
        };
        let net = self.mesh.net.clone();
        let handle = match wait_on(
            &self.pairing.handle,
            async move { net.call_join(join).await },
        ) {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => {
                drop(engine);
                if let Some(p) = &platform {
                    p.end();
                }
                return Err(format!("couldn't join the call: {e}"));
            }
            Err(e) => {
                if let Some(p) = &platform {
                    p.end();
                }
                return Err(e);
            }
        };

        let me = self.device.id.clone();
        let mut peers = BTreeMap::new();
        let member_of: BTreeMap<String, String> =
            m.members().into_iter().map(|(d, p, _)| (d, p)).collect();
        for d in devices.iter().filter(|d| **d != me) {
            peers.insert(
                d.clone(),
                Slot::new(member_of.get(d).cloned().unwrap_or_default()),
            );
        }
        let mut people: BTreeSet<String> = peers.values().map(|s| s.person_id.clone()).collect();
        people.insert(self.person_id());
        people.retain(|p| !p.is_empty());
        // The calls gate, read once for the whole call (card #656): a
        // Personal channel has no board and writes to the private log.
        // Lines count on from this device's last one in the call, if it
        // was in it before.
        let next = self
            .talk
            .call_posts(call_id)
            .iter()
            .filter(|p| p.post_id.starts_with("seg_") && p.device == me)
            .map(|p| p.ord.2.max(0) as u64 + 1)
            .max()
            .unwrap_or(0);
        let script = Arc::new(Script::new(
            space.is_empty() || self.space_tolerance(space).is_ok_and(|t| t.ready()),
            next,
        ));
        // The engine is up: the transcriber goes on its tap.
        let scribe = opened.and_then(|o| self.scribe_attach(o, call_id, conv, &engine, &script));
        let scribe_failed = want_scribe && scribe.is_none() && self.calls.transcriber.is_some();
        let w = Arc::downgrade(self);
        let share = ShareHub::new(
            me.clone(),
            self.pairing.handle.clone(),
            Arc::new(move || {
                if let Some(s) = w.upgrade() {
                    s.calls_changed();
                }
            }),
        );
        let (generation, hang_up) = {
            let mut g = self.calls.inner.lock().expect("calls");
            g.generation += 1;
            let generation = g.generation;
            let error = g.pending_error.take();
            let hang_up = std::mem::take(&mut g.pending_hangup);
            g.mine = Some(Active {
                generation,
                call_id: call_id.to_string(),
                conv_id: conv.to_string(),
                space_id: space.to_string(),
                joined_at_ms: now_ms(),
                started_at_ms: started_at_ms.min(now_ms()),
                started_here,
                muted: false,
                deafened: false,
                engine,
                handle,
                share,
                mesh: m.clone(),
                peers,
                people,
                error,
                script,
                scribe,
                scribe_failed,
            });
            g.dismissed.remove(call_id);
            g.entering = false;
            (generation, hang_up)
        };
        drop(_entering);
        self.calls_changed();
        if hang_up {
            return self.call_leave();
        }

        let mut roster = vec![me.clone()];
        roster.extend(devices.iter().filter(|d| **d != me).cloned());
        self.tell_space(&m, self.frame(call_id, conv, space, "join", roster));
        for d in devices.into_iter().filter(|d| *d != me) {
            self.call_dial(call_id, d);
        }
        self.call_ticker(call_id.to_string(), generation);
        Ok(())
    }

    fn call_title(&self, conv: &str) -> String {
        match self.talk.channel(conv) {
            Some(c) => format!("In a huddle · #{}", c.name),
            None => "In a huddle".into(),
        }
    }

    fn call_dial(&self, call_id: &str, device: String) {
        let net = self.mesh.net.clone();
        let cid = call_id.to_string();
        self.pairing.handle.spawn(async move {
            // A few tries: the peer may still be joining when we dial.
            for wait in [0u64, 1500, 4000] {
                tokio::time::sleep(Duration::from_millis(wait)).await;
                match net.call_dial(&cid, &device).await {
                    Ok(()) => return,
                    Err(e) if e.contains("already dialing") => return,
                    Err(e) => crate::logf(format_args!("call {cid}: dial {device}: {e}")),
                }
            }
        });
    }

    /// A connection to `cc.peer` is up in call `call_id`.
    fn call_peer(self: &Arc<Self>, call_id: &str, cc: irori_net::call::CallConn) {
        let irori_net::call::CallConn {
            peer,
            conn,
            control,
        } = cc;
        let inlet = {
            let mut g = self.calls.inner.lock().expect("calls");
            let Some(a) = g.mine.as_mut().filter(|a| a.call_id == call_id) else {
                conn.close(0u32.into(), b"not in this call");
                return;
            };
            let person = a.mesh.person_of(&peer).unwrap_or_default();
            if !person.is_empty() {
                a.people.insert(person.clone());
            }
            let slot = a
                .peers
                .entry(peer.clone())
                .or_insert_with(|| Slot::new(person.clone()));
            if slot.person_id.is_empty() {
                slot.person_id = person;
            }
            slot.conn = Some(conn.clone());
            slot.gone = false;
            // No line is resent across connections, and a device that left
            // and came back may count its lines from 0 again.
            slot.seen.clear();
            // Its control writer; first it hears whether this device
            // transcribes.
            let (ctl, ctl_rx) = tokio::sync::mpsc::channel(CTL_QUEUE);
            let _ = ctl.try_send(ControlMsg::State(CallPeerState {
                transcribe: a.scribe.is_some(),
            }));
            slot.ctl = Some(ctl);
            a.engine.add_peer(&peer, Arc::new(Datagrams(conn.clone())));
            let mut roster = vec![self.device.id.clone()];
            roster.extend(
                a.peers
                    .iter()
                    .filter(|(_, p)| !p.gone)
                    .map(|(d, _)| d.clone()),
            );
            let f = self.frame(&a.call_id, &a.conv_id, &a.space_id, "join", roster);
            (a.engine.inlet(), a.mesh.clone(), f, ctl_rx, a.share.clone())
        };
        let (inlet, m, f, mut ctl_rx, share) = inlet;
        // The share's messages (#658) ride the same control stream.
        let mut share_rx = share.add_peer(&peer, conn.clone());
        crate::logf(format_args!(
            "call {call_id}: {} connected to {peer} (conn {})",
            self.device.id,
            conn.stable_id()
        ));
        // Everyone hears the roster grew, so the mesh fills in at once.
        self.tell_space(&m, f);
        self.calls_changed();
        // One writer and one reader on the call stream for both the
        // transcript (#656) and the share (#658).
        let (mut send, mut recv) = control;
        let (p, cid) = (peer.clone(), call_id.to_string());
        self.pairing.handle.spawn(async move {
            let (mut ctl_open, mut share_open) = (true, true);
            while ctl_open || share_open {
                let r = tokio::select! {
                    m = ctl_rx.recv(), if ctl_open => match m {
                        Some(m) => irori_net::call::write_control(&mut send, &m).await,
                        None => {
                            ctl_open = false;
                            Ok(())
                        }
                    },
                    m = share_rx.recv(), if share_open => match m {
                        Some(m) => irori_net::share::write_msg(&mut send, &m).await,
                        None => {
                            share_open = false;
                            Ok(())
                        }
                    },
                };
                if let Err(e) = r {
                    crate::logf(format_args!("call {cid}: control to {p}: {e}"));
                    break;
                }
            }
        });
        let (w, p, cid, c, hub) = (
            Arc::downgrade(self),
            peer.clone(),
            call_id.to_string(),
            conn.clone(),
            Arc::downgrade(&share),
        );
        self.pairing.handle.spawn(async move {
            while let Ok(Some(m)) = irori_net::call::read_call_msg(&mut recv).await {
                match m {
                    irori_net::call::CallMsg::Control(m) => {
                        let Some(s) = w.upgrade() else { break };
                        s.on_control(&cid, &p, m);
                    }
                    irori_net::call::CallMsg::Share(m) => {
                        let Some(h) = hub.upgrade() else { break };
                        h.on_msg(&p, m);
                    }
                }
            }
            // Held for the connection's life: dropping it would stop the
            // peer's writes.
            c.closed().await;
            drop(recv);
        });
        let w = Arc::downgrade(self);
        let cid = call_id.to_string();
        self.pairing.handle.spawn(async move {
            while let Ok(d) = conn.read_datagram().await {
                inlet.receive(&peer, d.to_vec());
            }
            share.remove_peer(&peer, &conn);
            let Some(s) = w.upgrade() else { return };
            crate::logf(format_args!(
                "call {cid}: {} lost {peer} (conn {}): {:?}",
                s.device.id,
                conn.stable_id(),
                conn.close_reason()
            ));
            {
                let mut g = s.calls.inner.lock().expect("calls");
                if let Some(a) = g.mine.as_mut().filter(|a| a.call_id == cid)
                    && let Some(slot) = a.peers.get_mut(&peer)
                    && slot
                        .conn
                        .as_ref()
                        .is_some_and(|c| c.stable_id() == conn.stable_id())
                {
                    a.engine.remove_peer(&peer);
                    slot.conn = None;
                    slot.ctl = None;
                    slot.gone = true;
                }
            }
            s.calls_changed();
        });
    }

    /// While in call `call_id`: the window hears about health and speaking
    /// every tick, and the space hears this device is still in it every
    /// heartbeat.
    fn call_ticker(self: &Arc<Self>, call_id: String, generation: u64) {
        self.huddle_host_kick();
        let w = Arc::downgrade(self);
        self.pairing.handle.spawn(async move {
            let mut beat = tokio::time::Instant::now();
            loop {
                tokio::time::sleep(TICK).await;
                let Some(s) = w.upgrade() else { return };
                let want = s.call_transcribe() && s.calls.transcriber.is_some();
                let here = {
                    let mut g = s.calls.inner.lock().expect("calls");
                    g.mine
                        .as_mut()
                        .filter(|a| a.generation == generation)
                        .map(|a| {
                            if !want {
                                a.scribe_failed = false;
                            }
                            let toggle = want != a.scribe.is_some() && !(want && a.scribe_failed);
                            let mut roster = vec![s.device.id.clone()];
                            roster.extend(
                                a.peers
                                    .iter()
                                    .filter(|(_, p)| !p.gone)
                                    .map(|(d, _)| d.clone()),
                            );
                            // The one who started it rings on for a while, for
                            // devices that weren't reachable at first.
                            let ringing = a.started_here
                                && now_ms() - a.joined_at_ms < RING_FOR.as_millis() as i64;
                            (
                                a.mesh.clone(),
                                a.conv_id.clone(),
                                a.space_id.clone(),
                                roster,
                                ringing,
                                toggle,
                            )
                        })
                };
                let Some((m, conv, space, roster, ringing, toggle)) = here else {
                    return;
                };
                if toggle {
                    // The switch changed mid-call (card #656): starting or
                    // stopping a transcriber blocks.
                    let (s2, cid) = (s.clone(), call_id.clone());
                    let _ = tokio::task::spawn_blocking(move || {
                        s2.scribe_toggle(&cid, generation, want)
                    })
                    .await;
                }
                s.calls_changed();
                let every = if ringing { RING_AGAIN } else { HEARTBEAT };
                if beat.elapsed() >= every {
                    beat = tokio::time::Instant::now();
                    let op = if ringing { "ring" } else { "join" };
                    s.tell_space(&m, s.frame(&call_id, &conv, &space, op, roster));
                }
            }
        });
    }

    pub(crate) fn call_leave(&self) -> Result<(), String> {
        let me = self.device.id.clone();
        let (mut a, others) = {
            let mut g = self.calls.inner.lock().expect("calls");
            let Some(a) = g.mine.take() else {
                return Ok(());
            };
            // Don't ring here again for the call just left.
            g.dismissed.insert(a.call_id.clone());
            // Others are still in it if connected now, or if the space's
            // frames say so (a connection may be between redials).
            let others = a.peers.values().any(|p| p.conn.is_some() && !p.gone)
                || g.live
                    .get(&a.call_id)
                    .is_some_and(|l| l.devices.keys().any(|d| *d != me));
            (a, others)
        };
        let op = if others { "leave" } else { "end" };
        self.tell_space(
            &a.mesh,
            self.frame(&a.call_id, &a.conv_id, &a.space_id, op, vec![me]),
        );
        // The connections close before the transcriber stops, so the
        // phrase in progress is stored but never reaches the peers live.
        // Stopping it first and sending its line before leaving would fix
        // that, at the cost of a longer leave.
        a.share.close();
        a.handle.leave();
        // The transcript (card #656): every line this device heard is
        // written before the end post, and a ticker's start or stop in
        // progress finishes first. A line the drain didn't wait for is
        // dropped, never written after the end.
        a.engine.set_tap(None);
        {
            let _t = a.script.toggle.lock().expect("toggle");
            if let Some(sc) = a.scribe.take() {
                sc.finish();
            }
            a.script.closed.store(true, Ordering::SeqCst);
        }
        self.post_fallback(&a);
        if !others {
            // The last one out: the call is over.
            // People by key in this device's form, as mentions are.
            let me = self.person_id();
            let people = a
                .people
                .iter()
                .map(|p| {
                    if *p == me {
                        self.user_sender().key
                    } else {
                        format!("{}{p}", store::kinds::PERSON_ACTOR_PREFIX)
                    }
                })
                .collect();
            let transcript = a.script.st.lock().expect("script").heard
                || self
                    .talk
                    .call_posts(&a.call_id)
                    .iter()
                    .any(|p| p.call.is_none());
            let mark = CallMark {
                call_id: a.call_id.clone(),
                op: "end".into(),
                duration_ms: (now_ms() - a.started_at_ms).max(0),
                people,
                transcript,
            };
            if let Err(e) = self.post_call_mark(&a.conv_id, "ended the huddle", mark) {
                crate::logf(format_args!("call {}: end post: {e}", a.call_id));
            }
            self.calls
                .inner
                .lock()
                .expect("calls")
                .live
                .remove(&a.call_id);
            // The channel's agents hear once that it has a transcript (a
            // person DM's never reach an agent).
            if transcript
                && coord::talk::is_channel(&a.conv_id)
                && let Some(me) = self.this.get().and_then(Weak::upgrade)
            {
                tools::talk::tell_call_end(
                    &super::ToolsView(me),
                    &self.user_sender(),
                    &a.conv_id,
                    &a.call_id,
                );
            }
        }
        drop(a.engine);
        // Not if another call has begun since: its service is the phone's
        // one now.
        let newer = {
            let g = self.calls.inner.lock().expect("calls");
            g.mine.is_some() || g.entering
        };
        if let Some(p) = &self.calls.platform
            && !newer
        {
            p.end();
        }
        self.calls_changed();
        Ok(())
    }

    /// Makes a transcriber for call `call_id` (card #656), not yet on a
    /// tap. Blocks while a model loads: never under the calls lock. None
    /// when this device can't or it failed (logged).
    fn scribe_open(&self, call_id: &str) -> Option<Opened> {
        let f = self.calls.transcriber.clone()?;
        let (tx, rx) = std::sync::mpsc::channel();
        // The transcriber's thread only queues: the writer does the work.
        let sink_tx = tx.clone();
        let sink: call::SegmentSink = Arc::new(move |seg| {
            let _ = sink_tx.send(ScribeMsg::Line(seg));
        });
        match f(sink) {
            Ok(t) => Some(Opened {
                t: Some(t),
                tx,
                rx: Some(rx),
            }),
            Err(e) => {
                crate::logf(format_args!("call {call_id}: no transcriber: {e}"));
                None
            }
        }
    }

    /// Puts transcriber `o` on `engine`'s tap for call `call_id` in `conv`,
    /// with a thread that writes each line it finishes. None when the
    /// thread couldn't start (logged).
    fn scribe_attach(
        self: &Arc<Self>,
        mut o: Opened,
        call_id: &str,
        conv: &str,
        engine: &call::Engine,
        script: &Arc<Script>,
    ) -> Option<Scribe> {
        let (Some(mut t), Some(rx)) = (o.t.take(), o.rx.take()) else {
            return None;
        };
        let tx = o.tx.clone();
        drop(o);
        let tap = t.tap();
        // A line's times count from the first tapped sample.
        let anchor_ms = now_ms();
        engine.set_tap(Some(tap));
        let w = Arc::downgrade(self);
        let (cid, conv, sc) = (call_id.to_string(), conv.to_string(), script.clone());
        let writer = std::thread::Builder::new()
            .name("irori-call-transcript".into())
            .spawn(move || {
                while let Ok(ScribeMsg::Line(seg)) = rx.recv() {
                    let Some(s) = w.upgrade() else { return };
                    s.call_line(&cid, &conv, &sc, anchor_ms, seg);
                }
                let n = sc.dropped.load(Ordering::SeqCst);
                if n > 0 {
                    crate::logf(format_args!(
                        "call {cid}: {n} lines came after the call was left: not written"
                    ));
                }
            });
        match writer {
            Ok(writer) => Some(Scribe { t, tx, writer }),
            Err(e) => {
                crate::logf(format_args!("call {call_id}: transcript writer: {e}"));
                engine.set_tap(None);
                t.stop();
                None
            }
        }
    }

    /// One line this device's transcriber finished: to each peer live, to
    /// the captions, and into the log (or the fallback buffer).
    fn call_line(
        &self,
        call_id: &str,
        conv: &str,
        script: &Script,
        anchor_ms: i64,
        seg: call::Segment,
    ) {
        let text = cap_line(seg.text.trim()).to_string();
        if text.is_empty() || script.drop_late() {
            return;
        }
        let start_ms = anchor_ms + seg.start_ms as i64;
        let end_ms = (anchor_ms + seg.end_ms as i64).max(start_ms);
        let seq = {
            let mut st = script.st.lock().expect("script");
            st.next += 1;
            st.next - 1
        };
        let line = TranscriptLine {
            device_id: self.device.id.clone(),
            seq,
            start_ms,
            end_ms,
            text: text.clone(),
            // The notice says agents hear the huddle (card #1047).
            agents: true,
        };
        let ctls = self.call_ctls(call_id);
        for c in ctls {
            let _ = c.try_send(ControlMsg::Transcript(line.clone()));
        }
        script.caption(Caption {
            device_id: self.device.id.clone(),
            person_id: self.person_id(),
            mine: true,
            seq,
            start_ms,
            end_ms,
            text: text.clone(),
        });
        self.calls_changed();
        // Left while this line went out live.
        if script.drop_late() {
            return;
        }
        if script.rows {
            match self.talk.write_segment_marked(
                conv,
                call_id,
                seq as i64,
                start_ms,
                end_ms,
                &text,
                &self.user_sender().key,
                true,
            ) {
                Ok(_) => {
                    self.hub.publish(CoreEvent::Talk);
                    // The follows hear this row too; the feed drops the
                    // second sighting by the line's id.
                    self.huddle_line(&super::huddle_feed::Heard {
                        call_id: call_id.to_string(),
                        conv_id: conv.to_string(),
                        device: self.device.id.clone(),
                        seg: seq as i64,
                        start_ms,
                        end_ms,
                        text: text.clone(),
                        agents: true,
                        by: self.user_sender().key,
                    });
                    return;
                }
                // Kept for the fallback post, so nothing is lost (the gate
                // may have closed mid-call).
                Err(e) => crate::logf(format_args!("call {call_id}: line {seq}: {e}")),
            }
        }
        script
            .st
            .lock()
            .expect("script")
            .fallback
            .push(TranscriptLineMark {
                start_ms,
                end_ms,
                text,
            });
    }

    /// The control writers of call `call_id`'s connected peers.
    fn call_ctls(&self, call_id: &str) -> Vec<tokio::sync::mpsc::Sender<ControlMsg>> {
        let g = self.calls.inner.lock().expect("calls");
        g.mine
            .as_ref()
            .filter(|a| a.call_id == call_id)
            .map(|a| {
                a.peers
                    .values()
                    .filter(|p| !p.gone)
                    .filter_map(|p| p.ctl.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A control message from `peer` in call `call_id`. The speaker is the
    /// connection's device, never the frame's.
    fn on_control(&self, call_id: &str, peer: &str, m: ControlMsg) {
        let mut g = self.calls.inner.lock().expect("calls");
        let Some(a) = g.mine.as_mut().filter(|a| a.call_id == call_id) else {
            return;
        };
        let Some(slot) = a.peers.get_mut(peer) else {
            return;
        };
        match m {
            ControlMsg::State(st) => {
                slot.transcribing = Some(st.transcribe);
                drop(g);
            }
            ControlMsg::Transcript(l) => {
                let text = cap_line(l.text.trim()).to_string();
                if text.is_empty() || !slot.seen.insert(l.seq) {
                    return;
                }
                let c = Caption {
                    device_id: peer.to_string(),
                    person_id: slot.person_id.clone(),
                    mine: false,
                    seq: l.seq,
                    start_ms: l.start_ms,
                    end_ms: l.end_ms.max(l.start_ms),
                    text,
                };
                let script = a.script.clone();
                drop(g);
                script.caption(c);
            }
        }
        self.calls_changed();
    }

    /// The switch changed mid-call (card #656): starts a transcriber for
    /// the call this device entered as `generation` when `on`, else stops
    /// it; the peers hear which. Blocks.
    fn scribe_toggle(self: &Arc<Self>, call_id: &str, generation: u64, on: bool) {
        let here = {
            let g = self.calls.inner.lock().expect("calls");
            g.mine
                .as_ref()
                .filter(|a| a.generation == generation)
                .map(|a| (a.script.clone(), a.engine.clone(), a.conv_id.clone()))
        };
        let Some((script, engine, conv)) = here else {
            return;
        };
        let _t = script.toggle.lock().expect("toggle");
        let running = if on {
            let started = self
                .scribe_open(call_id)
                .and_then(|o| self.scribe_attach(o, call_id, &conv, &engine, &script));
            let failed = started.is_none();
            let mut g = self.calls.inner.lock().expect("calls");
            match g.mine.as_mut().filter(|a| a.generation == generation) {
                Some(a) if a.scribe.is_none() => {
                    a.scribe = started;
                    a.scribe_failed = failed;
                    !failed
                }
                // Left (or started) meanwhile: this one isn't wanted.
                _ => {
                    drop(g);
                    if let Some(sc) = started {
                        engine.set_tap(None);
                        sc.finish();
                    }
                    return;
                }
            }
        } else {
            let sc = {
                let mut g = self.calls.inner.lock().expect("calls");
                g.mine
                    .as_mut()
                    .filter(|a| a.generation == generation)
                    .and_then(|a| a.scribe.take())
            };
            if let Some(sc) = sc {
                engine.set_tap(None);
                sc.finish();
            }
            false
        };
        for c in self.call_ctls(call_id) {
            let _ = c.try_send(ControlMsg::State(CallPeerState {
                transcribe: running,
            }));
        }
    }

    /// The call's share hub, while in a call.
    fn share_hub(&self) -> Option<Arc<ShareHub>> {
        self.calls
            .inner
            .lock()
            .expect("calls")
            .mine
            .as_ref()
            .map(|a| a.share.clone())
    }

    pub(crate) fn share_sources(&self) -> Result<Vec<ShareSource>, String> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            on_own_thread(super::calls_share::sources)?
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err("this device can't share its screen".into())
        }
    }

    /// Starts sharing source `id` in the call this device is in. Blocks
    /// until the capture and the encoder are running (or failed).
    pub(crate) fn share_start(&self, id: &str) -> Result<(), String> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            use super::calls_share::{FPS, Picture, START_KBPS, picture_of, spawn_encoder};
            let hub = self.share_hub().ok_or("join a huddle first")?;
            let picture = picture_of(id)?;
            let (kind, title, app) = match &picture {
                Picture::Synthetic(..) => ("window".into(), "Test picture".into(), "Irori".into()),
                Picture::Source(_) => {
                    let found = on_own_thread(super::calls_share::sources)??
                        .into_iter()
                        .find(|s| s.id == id);
                    match found {
                        Some(s) => (s.kind, s.title, s.app),
                        None => return Err("that window or screen is gone".into()),
                    }
                }
            };
            let info = irori_net::share::ShareInfo {
                v: irori_net::share::SHARE_V,
                share_id: new_call_id()?,
                started_ms: now_ms(),
                kind,
                title,
                app,
                width: 0,
                height: 0,
                fps: FPS,
                kbps: START_KBPS,
            };
            let ctl = hub.begin(info)?;
            self.calls_changed();
            if let Err(e) = spawn_encoder(Arc::downgrade(&hub), ctl.clone(), picture) {
                hub.end(Some(&ctl), Some(e.clone()));
                return Err(e);
            }
            Ok(())
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = id;
            Err("this device can't share its screen".into())
        }
    }

    pub(crate) fn share_stop(&self) {
        if let Some(h) = self.share_hub() {
            h.end(None, None);
        }
        self.calls_changed();
    }

    /// Writes this device's lines that weren't stored as rows as fallback
    /// posts in the call's conversation (card #656): plain text an older
    /// Irori shows, with a transcript mark a newer one folds into the call.
    fn post_fallback(&self, a: &Active) {
        let lines = std::mem::take(&mut a.script.st.lock().expect("script").fallback);
        if lines.is_empty() {
            return;
        }
        let at = |l: &TranscriptLineMark| {
            let s = (l.start_ms - a.started_at_ms).max(0) / 1000;
            format!("{}:{:02} {}", s / 60, s % 60, l.text)
        };
        let mut chunks: Vec<(String, Vec<TranscriptLineMark>)> = Vec::new();
        for l in lines {
            let row = at(&l);
            match chunks.last_mut() {
                Some((t, ls)) if t.len() + 1 + row.len() <= FALLBACK_MAX => {
                    t.push('\n');
                    t.push_str(&row);
                    ls.push(l);
                }
                _ => chunks.push((row, vec![l])),
            }
        }
        for (text, lines) in chunks {
            let p = NewPost {
                conv_id: a.conv_id.clone(),
                text,
                by: self.user_sender().key,
                transcript: Some(TranscriptMark {
                    call_id: a.call_id.clone(),
                    lines,
                }),
                ..NewPost::default()
            };
            match self.talk.post(p) {
                Ok(_) => self.hub.publish(CoreEvent::Talk),
                Err(e) => crate::logf(format_args!("call {}: transcript post: {e}", a.call_id)),
            }
        }
    }

    pub(crate) fn share_attach(&self, sink: ShareSink) -> Result<(), String> {
        let h = self.share_hub().ok_or("you're not in a huddle")?;
        h.attach(sink);
        Ok(())
    }

    pub(crate) fn share_detach(&self) {
        if let Some(h) = self.share_hub() {
            h.detach();
        }
    }

    pub(crate) fn share_keyframe(&self) {
        if let Some(h) = self.share_hub() {
            h.keyframe();
        }
    }

    pub(crate) fn share_note_seen(&self) {
        if let Some(h) = self.share_hub() {
            h.clear_note();
        }
        self.calls_changed();
    }

    pub(crate) fn call_mute(&self, on: bool) {
        if let Some(a) = self.calls.inner.lock().expect("calls").mine.as_mut() {
            a.muted = on;
            a.engine.set_muted(on);
        }
        self.calls_changed();
    }

    pub(crate) fn call_deafen(&self, on: bool) {
        if let Some(a) = self.calls.inner.lock().expect("calls").mine.as_mut() {
            a.deafened = on;
            a.engine.set_deafened(on);
        }
        self.calls_changed();
    }

    /// The mic a huddle would open, by name ("" when it can't say).
    pub(crate) fn call_mic_name(&self) -> String {
        if self.calls.audio.is_some() {
            // Tests' fake devices.
            return "Test microphone".into();
        }
        if self.calls.platform.is_some() {
            // The phone: cpal's AAudio only knows "Default Device".
            return String::new();
        }
        call::device::default_mic_name()
    }

    pub(crate) fn call_dismiss(&self, call_id: &str) {
        {
            let mut g = self.calls.inner.lock().expect("calls");
            g.ringing.remove(call_id);
            g.dismissed.insert(call_id.to_string());
        }
        self.calls_changed();
    }

    /// Writes huddle mark `mark` in channel or person DM `conv` as the
    /// person, with `text` as what an older reader shows. It goes straight
    /// to Talk, not through `tools::talk::post`: nobody is mentioned and no
    /// agent in the channel is told at its next turn. A thread and a DM
    /// with an agent have no huddle. A person DM must have its space
    /// already: a mark never founds one.
    pub(crate) fn post_call_mark(
        &self,
        conv: &str,
        text: &str,
        mark: CallMark,
    ) -> Result<Post, String> {
        let conv = conv.trim();
        if coord::talk::is_dm(conv) {
            let Some(other) = self.talk.pair_other(conv) else {
                return Err("a DM with an agent has no huddle".into());
            };
            let id = other
                .strip_prefix(store::kinds::PERSON_ACTOR_PREFIX)
                .unwrap_or_default();
            if id.is_empty() || self.dm_writer(id).is_none() {
                return Err("this DM hasn't started yet: write in it first".into());
            }
        } else if !coord::talk::is_channel(conv) {
            return Err(format!(
                "{conv} isn't a channel or a DM: a thread has no huddle"
            ));
        }
        let p = self
            .talk
            .post(NewPost {
                conv_id: conv.to_string(),
                text: text.to_string(),
                by: self.user_sender().key,
                call: Some(mark),
                ..NewPost::default()
            })
            .map_err(|e| e.to_string())?;
        self.hub.publish(crate::events::CoreEvent::Talk);
        Ok(p)
    }
}

/// What the window shows about calls (card #654).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CallSnapshot {
    /// The call this device is in.
    pub mine: Option<MyCall>,
    /// Calls this device is rung for and hasn't joined or dismissed.
    pub ringing: Vec<Ringing>,
    /// Calls on now in conversations this device can see, from the frames
    /// it heard (the list rows' "huddle · 3").
    pub live: Vec<LiveCall>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MyCall {
    pub call_id: String,
    pub conv_id: String,
    /// The space its rows and connections belong to (a channel's space, or
    /// a person DM's hidden space).
    pub space_id: String,
    /// When this device joined, Unix ms.
    pub joined_at_ms: i64,
    pub muted: bool,
    /// Deafened: nothing plays here and the mic is off.
    pub deafened: bool,
    /// This device's mic is above the speech gate.
    pub speaking: bool,
    /// The other devices in the call.
    pub peers: Vec<CallPeer>,
    /// Why the call isn't working, in plain words (the mic, the network).
    pub error: Option<String>,
    /// This device writes down what its person says now (card #656).
    pub transcribing: bool,
    /// This device can transcribe (it has a transcriber).
    pub can_transcribe: bool,
    /// The newest lines said in the call, oldest first.
    pub captions: Vec<Caption>,
    /// The screen share on in the call, this device's or another's (#658).
    pub share: Option<CallShare>,
    /// Something to tell the person about sharing.
    pub share_note: Option<CallShareNote>,
    /// This device can share its screen (the desktops).
    pub can_share: bool,
}

/// One line said in a call, as a caption (card #656).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Caption {
    /// The device that wrote it down.
    pub device_id: String,
    /// Its person's id, as [`CallPeer::person_id`].
    pub person_id: String,
    /// Said on this device.
    pub mine: bool,
    /// Counts from 0 per device per call.
    pub seq: u64,
    /// Unix ms.
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

/// Whether this build can share a screen: capture and the hardware encoder
/// are the Mac's and the PC's.
pub const CAN_SHARE: bool = cfg!(any(target_os = "macos", target_os = "windows"));

/// The share on in a call.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CallShare {
    pub now: ShareNow,
    /// Who shares (this person when it is this device).
    pub person_id: String,
    /// How this device reaches the sharer: "direct", "relay", or "" (it is
    /// this device, or not connected).
    pub path: String,
}

/// Something to tell the person about sharing.
#[derive(Debug, Clone, PartialEq)]
pub enum CallShareNote {
    /// Someone else started sharing, so this device's share stopped.
    Replaced {
        device_id: String,
        person_id: String,
    },
    /// This device's share stopped or couldn't start, and why.
    Failed(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CallPeer {
    pub device_id: String,
    pub person_id: String,
    /// "connecting", "live" or "gone".
    pub phase: String,
    /// "direct", "relay" or "" (not connected).
    pub path: String,
    pub rtt_ms: f64,
    /// Datagrams lost on the way here, percent.
    pub loss_pct: f64,
    pub speaking: bool,
    pub muted: bool,
    /// Whether it writes down what its person says; None until it has said
    /// (an older Irori never does).
    pub transcribing: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ringing {
    pub call_id: String,
    pub conv_id: String,
    pub space_id: String,
    pub by_device: String,
    pub by_person: String,
    pub at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveCall {
    pub call_id: String,
    pub conv_id: String,
    pub space_id: String,
    /// (device id, person id) of who is in it.
    pub devices: Vec<(String, String)>,
}

impl Core {
    /// The calls on now, as the window shows them.
    pub fn call_snapshot(&self) -> CallSnapshot {
        self.s.call_snapshot()
    }

    /// Starts a huddle in channel or person DM `conv`: opens the mic and
    /// speaker, writes the start post and rings the conversation's devices
    /// that are online. Blocks while the devices open. Returns the call id.
    pub fn call_start(&self, conv: &str) -> Result<String, String> {
        self.s.call_start(conv)
    }

    /// Whether a huddle in `conv` reaches agents live (card #1047): false for
    /// a channel of a space where some device isn't known to take `call.*`
    /// rows yet, whose lines reach the channel only at the end.
    pub fn call_agents_ready(&self, conv: &str) -> bool {
        let space = self
            .talk()
            .channel(conv)
            .map(|c| c.space_id)
            .unwrap_or_default();
        space.is_empty() || self.s.space_tolerance(&space).is_ok_and(|t| t.ready())
    }

    /// Joins a call this device was rung for or sees live.
    pub fn call_join(&self, call_id: &str) -> Result<(), String> {
        self.s.call_join(call_id)
    }

    /// Leaves the call this device is in; the last one out writes the end
    /// post.
    pub fn call_leave(&self) -> Result<(), String> {
        self.s.call_leave()
    }

    pub fn call_mute(&self, on: bool) {
        self.s.call_mute(on)
    }

    /// Deafen: nothing from the others plays here and the mic is off.
    /// Undeafen goes back to the mute it had.
    pub fn call_deafen(&self, on: bool) {
        self.s.call_deafen(on)
    }

    /// The mic a huddle would open, by name ("" when it can't say). It
    /// opens nothing.
    pub fn call_mic_name(&self) -> String {
        self.s.call_mic_name()
    }

    /// Whether huddle `call_id` is over, as this device can tell (card
    /// #1043, call-agents.md section 3). True after an end mark, or after
    /// a crash left the call quiet; never on a device that started or
    /// reconnected mid-call and hasn't the rows to say so.
    pub fn call_over(&self, call_id: &str) -> bool {
        self.s.call_over(call_id)
    }

    /// How many lines the assistant feed holds for huddle `call_id` (for
    /// tests: a line seen twice counts once).
    #[doc(hidden)]
    pub fn huddle_fed(&self, call_id: &str) -> usize {
        self.s.huddle_fed(call_id)
    }

    /// Each person's assistant in huddle `call_id`, as the roster shows it
    /// (card #1049): connected, reconnecting or not connected.
    pub fn call_assistants(&self, call_id: &str) -> Vec<super::huddle_host::AssistantState> {
        self.s.call_assistants(call_id)
    }

    /// Whether this device hosts its person's assistant in huddle `call_id`,
    /// after running the pick again (card #1049): what a look asks before
    /// it runs.
    pub fn huddle_hosting(&self, call_id: &str) -> bool {
        self.s.huddle_hosting(call_id)
    }

    /// One pass of the hosting sweep now, apart from its timer (tests,
    /// card #1049). True while anything remains to watch.
    #[doc(hidden)]
    pub fn huddle_host_sweep(&self) -> bool {
        self.s.huddle_host_sweep()
    }

    /// Sets huddle `call_id`'s title as the person (card #1043); every
    /// member device gets it, and it beats an assistant's. Refused when a
    /// device of the space can't store `call.` rows yet (the gate).
    pub fn call_title_set(&self, call_id: &str, title: &str) -> Result<String, String> {
        self.s.call_title_set(call_id, title, false)
    }

    /// Huddle `call_id`'s canvas, each item with the names it is shown by
    /// (card #1051).
    pub fn call_canvas(&self, call_id: &str) -> Vec<super::huddle_canvas::CanvasEntry> {
        self.s.call_canvas(call_id)
    }

    /// A person adds an item (`question`, `answer`, `decision`, `task`,
    /// `note` or `summary`) to huddle `call_id`'s canvas (card #1051).
    /// `status` is "" for open. `carded` is refused while the huddle is
    /// live.
    pub fn huddle_item_add(
        &self,
        call_id: &str,
        kind: &str,
        text: &str,
        status: &str,
    ) -> Result<coord::talk::CanvasItem, String> {
        self.s.huddle_item_add(call_id, kind, text, status)
    }

    /// A person changes a canvas item: its text (their own), status,
    /// position or card. `carded` and a card are refused while the huddle
    /// is live.
    pub fn huddle_item_set(
        &self,
        call_id: &str,
        set: coord::talk::NewItemSet,
    ) -> Result<coord::talk::CanvasItem, String> {
        self.s.huddle_item_set(call_id, set)
    }

    /// The Ask box: puts `text` to this person's assistant as a question
    /// item and gives it a look (card #1051).
    pub fn huddle_ask(&self, call_id: &str, text: &str) -> Result<coord::talk::CanvasItem, String> {
        self.s.huddle_ask(call_id, text)
    }

    /// Stops ringing for `call_id` here.
    pub fn call_dismiss(&self, call_id: &str) {
        self.s.call_dismiss(call_id)
    }

    /// What this device can share: its displays, then its windows. On the
    /// Mac this needs the Screen Recording permission.
    pub fn share_sources(&self) -> Result<Vec<ShareSource>, String> {
        self.s.share_sources()
    }

    /// Shares source `id` (from [`Core::share_sources`]) in the huddle this
    /// device is in, replacing any share on in it. Blocks until capture
    /// runs.
    pub fn share_start(&self, id: &str) -> Result<(), String> {
        self.s.share_start(id)
    }

    /// Stops this device's share.
    pub fn share_stop(&self) {
        self.s.share_stop()
    }

    /// Watches the huddle's share: decoder configs and access units go to
    /// `sink` (irori_net::share's window messages). Replaces an earlier
    /// sink.
    pub fn share_attach(&self, sink: ShareSink) -> Result<(), String> {
        self.s.share_attach(sink)
    }

    /// Stops watching the share.
    pub fn share_detach(&self) {
        self.s.share_detach()
    }

    /// Asks the sharer for a keyframe (the window's decoder lost its place).
    pub fn share_keyframe(&self) {
        self.s.share_keyframe()
    }

    /// The person saw the share note: clear it.
    pub fn share_note_seen(&self) {
        self.s.share_note_seen()
    }

    /// On quit: leave any call and close the devices.
    pub fn call_shutdown(&self) {
        let _ = self.s.call_leave();
    }

    /// Devices of space `id` online now that a ring reaches: counted by
    /// the space and speaking calls (tests wait for them before ringing).
    /// "" is the person's own devices on the private mesh.
    #[doc(hidden)]
    pub fn call_reachable(&self, id: &str) -> Vec<String> {
        let Some(m) = self.s.ride(id) else {
            return Vec::new();
        };
        let counted: BTreeSet<String> = m.members().into_iter().map(|(d, _, _)| d).collect();
        m.live()
            .into_iter()
            .filter(|p| {
                p.calls >= mesh::boardmesh::CALLS_PROTOCOL && counted.contains(&p.device_id)
            })
            .map(|p| p.device_id)
            .collect()
    }

    /// (live device ids, counted device ids) of space `id` (tests).
    #[doc(hidden)]
    pub fn call_space_view(&self, id: &str) -> (Vec<String>, Vec<String>) {
        let Some(m) = self.s.mesh.board(id) else {
            return Default::default();
        };
        (
            m.live_peers().into_iter().map(|p| p.device_id).collect(),
            m.member_devices()
                .into_iter()
                .map(|d| d.device_id)
                .collect(),
        )
    }

    /// [`Services::post_call_mark`] (tests).
    #[doc(hidden)]
    pub fn post_call_mark(&self, conv: &str, text: &str, mark: CallMark) -> Result<Post, String> {
        self.s.post_call_mark(conv, text, mark)
    }
}
