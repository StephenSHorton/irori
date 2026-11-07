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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use coord::talk::{CallMark, NewPost, Post};
use iroh::endpoint::Connection;
use irori_identity::Chain;
use mesh::boardmesh::{BoardMesh, CallFrame, CallHooks, LivePeer};
use mesh::owncalls::OwnCalls;

use super::{Core, Services};
use crate::events::CoreEvent;

/// A ring nobody answers stops after this long.
const RING_FOR: Duration = Duration::from_secs(60);
/// Each device in a call says so this often, so devices that come online
/// later see the huddle and a crashed device drops out.
const HEARTBEAT: Duration = Duration::from_secs(15);
/// A live call nobody has said anything about for this long is gone.
const LIVE_FOR: Duration = Duration::from_secs(50);
/// While ringing, the ring goes out again this often.
const RING_AGAIN: Duration = Duration::from_secs(5);
/// The most devices a call holds (the card says 2 to 6; a little room).
const MAX_DEVICES: usize = 8;
/// How often the window hears about a call in progress (health, speaking).
const TICK: Duration = Duration::from_millis(400);

/// The call state on [`Services`].
#[derive(Default)]
pub(crate) struct Calls {
    inner: Mutex<Inner>,
    /// The phone's microphone service (Android); `None` on desktops.
    platform: Option<Arc<dyn call::Platform>>,
    audio: Option<call::device::OpenerFn>,
}

impl Calls {
    pub(crate) fn new(
        platform: Option<Arc<dyn call::Platform>>,
        audio: Option<call::device::OpenerFn>,
    ) -> Self {
        Calls {
            inner: Mutex::default(),
            platform,
            audio,
        }
    }
}

#[derive(Default)]
struct Inner {
    mine: Option<Active>,
    ringing: BTreeMap<String, Ring>,
    live: BTreeMap<String, Live>,
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
    mesh: Ride,
    peers: BTreeMap<String, Slot>,
    /// Person ids of everyone seen in the call, this person included.
    people: BTreeSet<String>,
    error: Option<String>,
}

struct Slot {
    person_id: String,
    conn: Option<Connection>,
    gone: bool,
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
        {
            let mut g = self.calls.inner.lock().expect("calls");
            let mine = g.mine.as_ref().is_some_and(|a| a.call_id == f.call_id);
            match f.op.as_str() {
                "ring" | "join" => {
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
                            let slot = a.peers.entry(d.clone()).or_insert(Slot {
                                person_id: known.get(&d).cloned().unwrap_or_default(),
                                conn: None,
                                gone: false,
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
        if let Some(d) = expire {
            self.publish_after(d + Duration::from_secs(1));
        }
        if changed {
            self.calls_changed();
        }
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
                    }
                })
                .collect();
            MyCall {
                call_id: a.call_id.clone(),
                conv_id: a.conv_id.clone(),
                space_id: a.space_id.clone(),
                joined_at_ms: a.joined_at_ms,
                muted: a.muted,
                deafened: a.deafened,
                speaking: st.speaking,
                peers,
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
                Slot {
                    person_id: member_of.get(d).cloned().unwrap_or_default(),
                    conn: None,
                    gone: false,
                },
            );
        }
        let mut people: BTreeSet<String> = peers.values().map(|s| s.person_id.clone()).collect();
        people.insert(self.person_id());
        people.retain(|p| !p.is_empty());
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
                mesh: m.clone(),
                peers,
                people,
                error,
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
            let slot = a.peers.entry(peer.clone()).or_insert(Slot {
                person_id: person.clone(),
                conn: None,
                gone: false,
            });
            if slot.person_id.is_empty() {
                slot.person_id = person;
            }
            slot.conn = Some(conn.clone());
            slot.gone = false;
            a.engine.add_peer(&peer, Arc::new(Datagrams(conn.clone())));
            let mut roster = vec![self.device.id.clone()];
            roster.extend(
                a.peers
                    .iter()
                    .filter(|(_, p)| !p.gone)
                    .map(|(d, _)| d.clone()),
            );
            let f = self.frame(&a.call_id, &a.conv_id, &a.space_id, "join", roster);
            (a.engine.inlet(), a.mesh.clone(), f)
        };
        let (inlet, m, f) = inlet;
        crate::logf(format_args!(
            "call {call_id}: {} connected to {peer} (conn {})",
            self.device.id,
            conn.stable_id()
        ));
        // Everyone hears the roster grew, so the mesh fills in at once.
        self.tell_space(&m, f);
        self.calls_changed();
        let w = Arc::downgrade(self);
        let cid = call_id.to_string();
        self.pairing.handle.spawn(async move {
            // Held for the connection's life: the control stream carries
            // nothing yet, but closing it would end the stream for the
            // peer.
            let _control = control;
            while let Ok(d) = conn.read_datagram().await {
                inlet.receive(&peer, d.to_vec());
            }
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
        let w = Arc::downgrade(self);
        self.pairing.handle.spawn(async move {
            let mut beat = tokio::time::Instant::now();
            loop {
                tokio::time::sleep(TICK).await;
                let Some(s) = w.upgrade() else { return };
                let here = {
                    let g = s.calls.inner.lock().expect("calls");
                    g.mine
                        .as_ref()
                        .filter(|a| a.generation == generation)
                        .map(|a| {
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
                            )
                        })
                };
                let Some((m, conv, space, roster, ringing)) = here else {
                    return;
                };
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
        let (a, others) = {
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
        if others {
            self.tell_space(
                &a.mesh,
                self.frame(&a.call_id, &a.conv_id, &a.space_id, "leave", vec![me]),
            );
        } else {
            // The last one out: the call is over.
            self.tell_space(
                &a.mesh,
                self.frame(&a.call_id, &a.conv_id, &a.space_id, "end", vec![me]),
            );
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
            let mark = CallMark {
                call_id: a.call_id.clone(),
                op: "end".into(),
                duration_ms: (now_ms() - a.started_at_ms).max(0),
                people,
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
        }
        a.handle.leave();
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

    /// Stops ringing for `call_id` here.
    pub fn call_dismiss(&self, call_id: &str) {
        self.s.call_dismiss(call_id)
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
