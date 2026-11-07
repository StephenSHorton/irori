//! Screen share in a huddle (#658): one device shares a window or a
//! display, the others in the call watch it.
//!
//! The wire is `irori_net::share`: control messages on each call
//! connection's `call` stream, video on one uni stream per keyframe group.
//! A [`ShareHub`] per call holds both ends:
//!
//! - **Sharing** (desktops only: the capture and the hardware encoder are
//!   `irori-share`, ScreenCaptureKit + VideoToolbox on the Mac, WGC + Media
//!   Foundation on the PC). One thread owns the capture and the encoder,
//!   which block and aren't `Send`; each access unit goes to every watching
//!   peer's queue. A peer's sender task writes them on that peer's GOP
//!   stream. A queue that fills (the peer's path can't keep up) is a peer
//!   behind: its stream is reset, it waits for the next keyframe, and the
//!   bitrate comes down. Keyframes are rationed (at most one every
//!   [`KEY_GAP`] for asks), because a Mac keyframe is 0.6-1 MB (#657).
//! - **Rate**: starts at [`START_KBPS`] and ramps to [`CAP_KBPS`] while no
//!   viewer falls behind. #653: n0's relays carry 2.5 Mbit/s with voice
//!   beside it untouched; 8 Mbit/s from the Mac queued frames and made
//!   voice late, and the phone's first second over a relay was slow.
//! - **Watching**: the window attaches a sink; the hub tells the sharer to
//!   send (`watch`), turns each keyframe's SPS/PPS into a decoder config and
//!   each access unit into AVCC, and drops a GOP as soon as a newer one
//!   opens.
//! - **One share at a time**: the newer share wins (its `startedMs`, then
//!   the device id). A device sharing when a newer one starts stops its own
//!   and says who took over.

// The phone only views: the sharer's half is unused there.
#![cfg_attr(
    not(any(target_os = "macos", target_os = "windows")),
    allow(dead_code, unused_imports)
)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use iroh::endpoint::{Connection, RecvStream, SendStream};
use irori_net::share::{self as wire, Au, Avc, GopHeader, ShareInfo, ShareMsg};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

/// The rate a share starts at, kbit/s.
pub(crate) const START_KBPS: u32 = 1000;
/// The rate a share ramps up to while every viewer keeps up (#653).
pub(crate) const CAP_KBPS: u32 = 2500;
/// The least a share drops to.
pub(crate) const MIN_KBPS: u32 = 300;
/// Asked-for keyframes come at most this often.
pub(crate) const KEY_GAP: Duration = Duration::from_secs(2);
/// Frames a second: text stays sharp, the encoder and the path stay light.
pub(crate) const FPS: u32 = 10;
/// A keyframe at least this often, unasked.
pub(crate) const KEY_EVERY: Duration = Duration::from_secs(10);
/// Access units queued for one viewer at most (half a second at 10 fps):
/// more means its path can't keep up.
const QUEUE: usize = 5;
/// The bitrate goes up by a quarter this often while nobody falls behind.
const RAMP_EVERY: Duration = Duration::from_secs(2);
/// After a viewer falls behind, no ramp for this long.
const CALM_FOR: Duration = Duration::from_secs(6);
/// An access unit reaching its viewer's stream this much later than its
/// GOP's pace says: the viewer is behind (QUIC buffers far more than the
/// queue, so a full queue alone comes too late).
const LAG_MAX: Duration = Duration::from_millis(500);

/// Where a watched share's frames go: the window's channel. False when it
/// is gone.
pub type ShareSink = Arc<dyn Fn(Vec<u8>) -> bool + Send + Sync>;

/// The share on in a call, as a snapshot shows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShareNow {
    pub info: ShareInfo,
    /// This device is the one sharing.
    pub mine: bool,
    /// The sharing device ("" when mine).
    pub device_id: String,
}

/// Something to tell the person about sharing.
#[derive(Debug, Clone, PartialEq)]
pub enum ShareNote {
    /// Device `.0` started sharing, so this device's share stopped.
    Replaced(String),
    /// This device's share stopped or couldn't start: why, in plain words.
    Failed(String),
}

/// What can be shared, as the window lists it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShareSource {
    /// Pass to `share_start`: `w:<id>` a window, `d:<id>` a display.
    pub id: String,
    /// `window` or `display`.
    pub kind: String,
    pub title: String,
    pub app: String,
    pub width: u32,
    pub height: u32,
}

/// What the encoder thread is told while it runs.
#[derive(Default)]
pub struct EncCtl {
    stop: AtomicBool,
    key_ask: AtomicBool,
    /// The bitrate wanted, kbit/s.
    kbps: AtomicU32,
    /// Bumped each time a viewer falls behind (or is still behind).
    behind: AtomicU64,
    /// The share's GOP numbers: one counter for every viewer's sender, so a
    /// viewer that watches again, or reconnects, still sees them rise.
    gop: AtomicU64,
}

impl EncCtl {
    fn new() -> Arc<EncCtl> {
        let c = EncCtl::default();
        c.kbps.store(START_KBPS, Ordering::SeqCst);
        Arc::new(c)
    }

    fn ask_key(&self) {
        self.key_ask.store(true, Ordering::SeqCst);
    }

    fn fell_behind(&self) {
        self.behind.fetch_add(1, Ordering::SeqCst);
        let k = self.kbps.load(Ordering::SeqCst);
        self.kbps
            .store((k * 7 / 10).max(MIN_KBPS), Ordering::SeqCst);
        self.ask_key();
    }
}

/// The bitrate's ramp, run by the encoder thread: up a quarter every
/// [`RAMP_EVERY`] to [`CAP_KBPS`] unless a viewer fell behind in the last
/// [`CALM_FOR`].
pub(crate) struct Ramp {
    seen_behind: u64,
    calm_since: std::time::Instant,
    last_up: std::time::Instant,
}

impl Ramp {
    pub(crate) fn new(now: std::time::Instant) -> Ramp {
        Ramp {
            seen_behind: 0,
            calm_since: now,
            last_up: now,
        }
    }

    /// The bitrate to run at now.
    pub(crate) fn tick(&mut self, ctl: &EncCtl, now: std::time::Instant) -> u32 {
        let b = ctl.behind.load(Ordering::SeqCst);
        if b != self.seen_behind {
            self.seen_behind = b;
            self.calm_since = now;
        }
        let k = ctl.kbps.load(Ordering::SeqCst);
        if k < CAP_KBPS
            && now.duration_since(self.calm_since) >= CALM_FOR
            && now.duration_since(self.last_up) >= RAMP_EVERY
        {
            self.last_up = now;
            let up = (k + k / 4).min(CAP_KBPS);
            ctl.kbps.store(up, Ordering::SeqCst);
            return up;
        }
        k
    }
}

struct PeerLink {
    conn: Connection,
    ctl: mpsc::UnboundedSender<ShareMsg>,
}

struct Viewer {
    tx: mpsc::Sender<Au>,
    behind: Arc<AtomicBool>,
}

struct Mine {
    info: ShareInfo,
    ctl: Arc<EncCtl>,
    viewers: HashMap<String, Viewer>,
    /// The call has been told (the capture runs).
    announced: bool,
    /// The share this one replaces, kept until it is announced: a share that
    /// fails to start gives the old one back.
    prev: Option<Theirs>,
}

struct Theirs {
    peer: String,
    info: ShareInfo,
    /// The GOP being read; an older one stops.
    gop: u64,
    /// The decoder config last sent to the window.
    avc: Option<Avc>,
}

#[derive(Default)]
struct Inner {
    closed: bool,
    peers: HashMap<String, PeerLink>,
    mine: Option<Mine>,
    theirs: Option<Theirs>,
    sink: Option<ShareSink>,
    note: Option<ShareNote>,
}

/// One call's share, both ends.
pub struct ShareHub {
    me: String,
    rt: Handle,
    changed: Arc<dyn Fn() + Send + Sync>,
    inner: Mutex<Inner>,
}

/// Whether share `a` (by device `da`) is newer than `b` (by `db`).
fn newer(a: &ShareInfo, da: &str, b: &ShareInfo, db: &str) -> bool {
    (a.started_ms, da) > (b.started_ms, db)
}

impl ShareHub {
    pub fn new(me: String, rt: Handle, changed: Arc<dyn Fn() + Send + Sync>) -> Arc<ShareHub> {
        Arc::new(ShareHub {
            me,
            rt,
            changed,
            inner: Mutex::default(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("share hub")
    }

    /// The share on now and the note for the person.
    pub fn view(&self) -> (Option<ShareNow>, Option<ShareNote>) {
        let g = self.lock();
        let now = match (&g.mine, &g.theirs) {
            (Some(m), _) => Some(ShareNow {
                info: m.info.clone(),
                mine: true,
                device_id: String::new(),
            }),
            (None, Some(t)) => Some(ShareNow {
                info: t.info.clone(),
                mine: false,
                device_id: t.peer.clone(),
            }),
            (None, None) => None,
        };
        (now, g.note.clone())
    }

    /// Ends everything: the call is being left.
    pub(crate) fn close(&self) {
        let mut g = self.lock();
        g.closed = true;
        if let Some(m) = g.mine.take() {
            m.ctl.stop.store(true, Ordering::SeqCst);
        }
        g.theirs = None;
        g.sink = None;
        g.peers.clear();
    }

    /// Tells every peer.
    fn tell_all(g: &Inner, m: &ShareMsg) {
        for p in g.peers.values() {
            let _ = p.ctl.send(m.clone());
        }
    }

    fn tell(g: &Inner, peer: &str, m: ShareMsg) {
        if let Some(p) = g.peers.get(peer) {
            let _ = p.ctl.send(m);
        }
    }

    // ---- peers ---------------------------------------------------------

    /// A connection to `peer` is up. The call service owns the call
    /// stream (the transcript's messages ride it too): it writes what comes
    /// out of the returned queue and hands what it reads to [`Self::on_msg`].
    pub fn add_peer(
        self: &Arc<Self>,
        peer: &str,
        conn: Connection,
    ) -> mpsc::UnboundedReceiver<ShareMsg> {
        let (tx, rx) = mpsc::unbounded_channel::<ShareMsg>();
        {
            let mut g = self.lock();
            if g.closed {
                return rx;
            }
            // A newer connection to the same peer replaces the older; its
            // tasks end with it.
            g.peers.insert(
                peer.to_string(),
                PeerLink {
                    conn: conn.clone(),
                    ctl: tx.clone(),
                },
            );
            if let Some(m) = g.mine.as_mut() {
                // A viewer on the old connection asks again on this one.
                m.viewers.remove(peer);
                let _ = tx.send(ShareMsg::Start(m.info.clone()));
            }
            if g.sink.is_some()
                && let Some(t) = g.theirs.as_ref().filter(|t| t.peer == peer)
            {
                let _ = tx.send(ShareMsg::Watch {
                    share_id: t.info.share_id.clone(),
                    on: true,
                });
            }
        }
        let w = Arc::downgrade(self);
        let p = peer.to_string();
        self.rt.spawn(async move {
            while let Ok(r) = conn.accept_uni().await {
                let Some(h) = w.upgrade() else { return };
                let p = p.clone();
                h.rt.clone().spawn(async move { h.on_gop(&p, r).await });
            }
        });
        rx
    }

    pub(crate) fn remove_peer(&self, peer: &str, conn: &Connection) {
        let mut g = self.lock();
        if g.peers
            .get(peer)
            .is_some_and(|l| l.conn.stable_id() == conn.stable_id())
        {
            g.peers.remove(peer);
            if let Some(m) = g.mine.as_mut() {
                m.viewers.remove(peer);
            }
            if g.theirs.as_ref().is_some_and(|t| t.peer == peer) {
                g.theirs = None;
                drop(g);
                (self.changed)();
            }
        }
    }

    pub(crate) fn on_msg(self: &Arc<Self>, peer: &str, m: ShareMsg) {
        let mut g = self.lock();
        if g.closed {
            return;
        }
        let mut changed = false;
        match m {
            ShareMsg::Start(info) => {
                if info.v < 1 || !irori_net::call::is_call_id(&info.share_id) {
                    return;
                }
                if let Some(m) = &g.mine {
                    if !newer(&info, peer, &m.info, &self.me) {
                        // Ours is newer: tell them, so they stop theirs.
                        if m.announced {
                            Self::tell(&g, peer, ShareMsg::Start(m.info.clone()));
                        }
                        return;
                    }
                    let m = g.mine.take().expect("mine");
                    m.ctl.stop.store(true, Ordering::SeqCst);
                    Self::tell_all(
                        &g,
                        &ShareMsg::Stop {
                            share_id: m.info.share_id,
                        },
                    );
                    g.note = Some(ShareNote::Replaced(peer.to_string()));
                }
                let same = g
                    .theirs
                    .as_ref()
                    .is_some_and(|t| t.peer == peer && t.info.share_id == info.share_id);
                if same {
                    let t = g.theirs.as_mut().expect("theirs");
                    changed = t.info != info;
                    t.info = info;
                } else {
                    if let Some(t) = &g.theirs
                        && !newer(&info, peer, &t.info, &t.peer)
                    {
                        return;
                    }
                    let watch = g.sink.is_some();
                    let id = info.share_id.clone();
                    g.theirs = Some(Theirs {
                        peer: peer.to_string(),
                        info,
                        gop: 0,
                        avc: None,
                    });
                    if watch {
                        Self::tell(
                            &g,
                            peer,
                            ShareMsg::Watch {
                                share_id: id,
                                on: true,
                            },
                        );
                    }
                    changed = true;
                }
            }
            ShareMsg::Stop { share_id } => {
                if let Some(m) = g.mine.as_mut()
                    && m.prev
                        .as_ref()
                        .is_some_and(|t| t.peer == peer && t.info.share_id == share_id)
                {
                    m.prev = None;
                }
                if g.theirs
                    .as_ref()
                    .is_some_and(|t| t.peer == peer && t.info.share_id == share_id)
                {
                    g.theirs = None;
                    changed = true;
                }
            }
            ShareMsg::Watch { share_id, on } => {
                let conn = g.peers.get(peer).map(|l| l.conn.clone());
                let rt = self.rt.clone();
                if let Some(m) = g.mine.as_mut().filter(|m| m.info.share_id == share_id) {
                    if !on {
                        m.viewers.remove(peer);
                    } else if let Some(conn) = conn
                        && !m.viewers.contains_key(peer)
                    {
                        let (tx, rx) = mpsc::channel(QUEUE);
                        let behind = Arc::new(AtomicBool::new(false));
                        rt.spawn(send_gops(conn, share_id, m.ctl.clone(), rx, behind.clone()));
                        m.viewers.insert(peer.to_string(), Viewer { tx, behind });
                        m.ctl.ask_key();
                    }
                }
            }
            ShareMsg::Keyframe { share_id } => {
                if let Some(m) = g.mine.as_ref().filter(|m| m.info.share_id == share_id) {
                    m.ctl.ask_key();
                }
            }
            ShareMsg::Unknown => {}
        }
        drop(g);
        if changed {
            (self.changed)();
        }
    }

    /// A uni stream from `peer`: a GOP of the share being watched, or
    /// nothing to keep.
    async fn on_gop(self: Arc<Self>, peer: &str, mut r: RecvStream) {
        let h = match tokio::time::timeout(Duration::from_secs(5), wire::read_gop(&mut r)).await {
            Ok(Ok(h)) => h,
            _ => {
                let _ = r.stop(0u32.into());
                return;
            }
        };
        let older = {
            let mut g = self.lock();
            let watching = g.sink.is_some();
            match g
                .theirs
                .as_mut()
                .filter(|t| t.peer == peer && t.info.share_id == h.share_id)
            {
                Some(t) if watching && h.gop > t.gop => {
                    t.gop = h.gop;
                    false
                }
                _ => true,
            }
        };
        if older {
            let _ = r.stop(0u32.into());
            return;
        }
        loop {
            let au = match wire::read_au(&mut r).await {
                Ok(Some(au)) => au,
                _ => return,
            };
            let (sink, out) =
                {
                    let mut g = self.lock();
                    let sink = g.sink.clone();
                    let Some(t) = g.theirs.as_mut().filter(|t| {
                        t.peer == peer && t.info.share_id == h.share_id && t.gop == h.gop
                    }) else {
                        drop(g);
                        let _ = r.stop(0u32.into());
                        return;
                    };
                    let Some(sink) = sink else {
                        drop(g);
                        let _ = r.stop(0u32.into());
                        return;
                    };
                    let mut out = Vec::with_capacity(2);
                    if au.key
                        && let Some(a) = Avc::from_annexb(&au.data)
                        && t.avc.as_ref() != Some(&a)
                    {
                        out.push(wire::view_config(&a));
                        t.avc = Some(a);
                    }
                    if t.avc.is_some() {
                        out.push(wire::view_chunk(au.key, au.ts_us, &wire::to_avcc(&au.data)));
                    }
                    // Delivered under the lock, so an older GOP's frame can't
                    // land after a newer GOP's config. The sink (a Tauri
                    // channel send) doesn't block or call back here.
                    let ok = out.into_iter().all(|b| sink(b));
                    (sink, ok)
                };
            if !out {
                self.detach_sink(&sink);
                let _ = r.stop(0u32.into());
                return;
            }
        }
    }

    // ---- the window's side ---------------------------------------------

    /// The window watches the share: frames go to `sink`. Replaces an
    /// earlier sink; the sharer is asked for a keyframe so the new decoder
    /// can start.
    pub fn attach(&self, sink: ShareSink) {
        let mut g = self.lock();
        let had = g.sink.replace(sink).is_some();
        if let Some(t) = g.theirs.as_mut() {
            t.avc = None;
            let (peer, id) = (t.peer.clone(), t.info.share_id.clone());
            let m = if had {
                ShareMsg::Keyframe { share_id: id }
            } else {
                ShareMsg::Watch {
                    share_id: id,
                    on: true,
                }
            };
            Self::tell(&g, &peer, m);
        }
    }

    /// The window stops watching.
    pub(crate) fn detach(&self) {
        let mut g = self.lock();
        if g.sink.take().is_some() {
            Self::unwatch(&g);
        }
    }

    fn detach_sink(&self, sink: &ShareSink) {
        let mut g = self.lock();
        if g.sink.as_ref().is_some_and(|s| Arc::ptr_eq(s, sink)) {
            g.sink = None;
            Self::unwatch(&g);
        }
    }

    fn unwatch(g: &Inner) {
        if let Some(t) = &g.theirs {
            Self::tell(
                g,
                &t.peer,
                ShareMsg::Watch {
                    share_id: t.info.share_id.clone(),
                    on: false,
                },
            );
        }
    }

    /// The window's decoder lost its place: ask the sharer for a keyframe.
    pub(crate) fn keyframe(&self) {
        let g = self.lock();
        if let Some(t) = &g.theirs {
            Self::tell(
                &g,
                &t.peer,
                ShareMsg::Keyframe {
                    share_id: t.info.share_id.clone(),
                },
            );
        }
    }

    // ---- sharing ---------------------------------------------------------

    /// Starts sharing `info` (its size and rate filled in as the encoder
    /// opens): any share of ours ends first, and a share of theirs gives way.
    /// Returns the encoder's controls.
    pub fn begin(&self, mut info: ShareInfo) -> Result<Arc<EncCtl>, String> {
        let mut g = self.lock();
        if g.closed {
            return Err("you're not in a huddle".into());
        }
        if let Some(m) = g.mine.take() {
            m.ctl.stop.store(true, Ordering::SeqCst);
            Self::tell_all(
                &g,
                &ShareMsg::Stop {
                    share_id: m.info.share_id,
                },
            );
        }
        if g.sink.is_some() {
            Self::unwatch(&g);
        }
        // Replacing theirs on purpose: newer than it whatever the clocks say.
        if let Some(t) = &g.theirs {
            info.started_ms = info.started_ms.max(t.info.started_ms + 1);
        }
        let prev = g.theirs.take();
        g.note = None;
        let ctl = EncCtl::new();
        g.mine = Some(Mine {
            info,
            ctl: ctl.clone(),
            viewers: HashMap::new(),
            announced: false,
            prev,
        });
        Ok(ctl)
    }

    /// The encoder is running (or its picture or rate changed): tell the
    /// call. False when this share is no longer the one on.
    pub(crate) fn announce(&self, ctl: &Arc<EncCtl>, width: u32, height: u32, kbps: u32) -> bool {
        let mut g = self.lock();
        let Some(m) = g.mine.as_mut().filter(|m| Arc::ptr_eq(&m.ctl, ctl)) else {
            return false;
        };
        m.info.width = width;
        m.info.height = height;
        m.info.kbps = kbps;
        m.announced = true;
        m.prev = None;
        let msg = ShareMsg::Start(m.info.clone());
        Self::tell_all(&g, &msg);
        drop(g);
        (self.changed)();
        true
    }

    /// One encoded access unit of our share, from the encoder thread.
    pub(crate) fn push(&self, ctl: &Arc<EncCtl>, au: Au) {
        let g = self.lock();
        let Some(m) = g.mine.as_ref().filter(|m| Arc::ptr_eq(&m.ctl, ctl)) else {
            return;
        };
        for v in m.viewers.values() {
            match v.tx.try_send(au.clone()) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    if !v.behind.swap(true, Ordering::SeqCst) {
                        ctl.fell_behind();
                    } else {
                        // Still behind: hold the ramp, no second cut.
                        ctl.behind.fetch_add(1, Ordering::SeqCst);
                    }
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {}
            }
        }
    }

    /// Stops our share, with `why` for the person when it failed.
    pub fn end(&self, ctl: Option<&Arc<EncCtl>>, why: Option<String>) {
        let mut g = self.lock();
        let ours = match (&g.mine, ctl) {
            (Some(m), Some(c)) => Arc::ptr_eq(&m.ctl, c),
            (Some(_), None) => true,
            (None, _) => false,
        };
        if !ours {
            return;
        }
        let m = g.mine.take().expect("mine");
        m.ctl.stop.store(true, Ordering::SeqCst);
        if m.announced {
            Self::tell_all(
                &g,
                &ShareMsg::Stop {
                    share_id: m.info.share_id,
                },
            );
        } else if let Some(mut t) = m.prev {
            // Never told anyone: the share it would have replaced is back.
            t.gop = 0;
            t.avc = None;
            if g.sink.is_some() {
                Self::tell(
                    &g,
                    &t.peer,
                    ShareMsg::Watch {
                        share_id: t.info.share_id.clone(),
                        on: true,
                    },
                );
            }
            g.theirs = Some(t);
        }
        g.note = why.map(ShareNote::Failed);
        drop(g);
        (self.changed)();
    }

    /// Clears the note once the person has seen it (a new share does too).
    pub(crate) fn clear_note(&self) {
        self.lock().note = None;
    }
}

/// One viewer's sender: each GOP on a stream of its own, started at a
/// keyframe; the GOP before it is reset, not drained, so stale frames never
/// go ahead of the new keyframe. An access unit that goes out more than
/// [`LAG_MAX`] behind its GOP's pace, or a full queue (`behind`), resets the
/// stream: nothing more goes until the next keyframe, which is asked for,
/// and the bitrate comes down.
async fn send_gops(
    conn: Connection,
    share_id: String,
    ctl: Arc<EncCtl>,
    mut rx: mpsc::Receiver<Au>,
    behind: Arc<AtomicBool>,
) {
    let mut cur: Option<SendStream> = None;
    // When the GOP's keyframe went out, and its timestamp.
    let mut pace: Option<(tokio::time::Instant, u64)> = None;
    while let Some(au) = rx.recv().await {
        if behind.swap(false, Ordering::SeqCst)
            && let Some(mut s) = cur.take()
        {
            let _ = s.reset(0u32.into());
        }
        if au.key {
            if let Some(mut s) = cur.take() {
                let _ = s.reset(0u32.into());
            }
            let h = GopHeader {
                share_id: share_id.clone(),
                gop: ctl.gop.fetch_add(1, Ordering::SeqCst) + 1,
            };
            match wire::open_gop(&conn, &h).await {
                Ok(s) => {
                    cur = Some(s);
                    pace = Some((tokio::time::Instant::now(), au.ts_us));
                }
                Err(_) if conn.close_reason().is_some() => return,
                Err(_) => continue,
            }
        }
        let Some(s) = cur.as_mut() else { continue };
        let due = pace.map(|(t, ts0)| t + Duration::from_micros(au.ts_us.saturating_sub(ts0)));
        let late = |now: tokio::time::Instant| due.is_some_and(|d| now > d + LAG_MAX);
        // A write blocked on QUIC's buffer past the limit is behind too.
        let wait = due.map_or(LAG_MAX * 2, |d| {
            (d + LAG_MAX).saturating_duration_since(tokio::time::Instant::now()) + LAG_MAX
        });
        let ok = if late(tokio::time::Instant::now()) {
            false
        } else {
            match tokio::time::timeout(wait, wire::write_au(s, &au)).await {
                Ok(Ok(())) => !late(tokio::time::Instant::now()),
                Ok(Err(_)) => {
                    cur = None;
                    continue;
                }
                Err(_) => false,
            }
        };
        if !ok && let Some(mut s) = cur.take() {
            let _ = s.reset(0u32.into());
            ctl.fell_behind();
        }
    }
    if let Some(mut s) = cur {
        let _ = s.finish();
    }
}

// ---- the encoder thread (desktops) ----------------------------------------

/// What to share: a window or display by its source id, or the test
/// picture (`synthetic:WxH`, no capture, no grant).
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub enum Picture {
    Source(share::Source),
    Synthetic(u32, u32),
}

/// Parses a source id from [`ShareSource::id`] (or `synthetic:WxH`).
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn picture_of(id: &str) -> Result<Picture, String> {
    if let Some(w) = id.strip_prefix("w:")
        && !w.is_empty()
    {
        return Ok(Picture::Source(share::Source::Window(w.to_string())));
    }
    if let Some(d) = id.strip_prefix("d:") {
        return Ok(Picture::Source(share::Source::Display(
            (!d.is_empty()).then(|| d.to_string()),
        )));
    }
    if let Some(s) = id.strip_prefix("synthetic:")
        && let Some((w, h)) = s.split_once('x')
        && let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>())
        && (16..=4096).contains(&w)
        && (16..=4096).contains(&h)
    {
        return Ok(Picture::Synthetic(w & !1, h & !1));
    }
    Err("that window or screen can't be shared".into())
}

/// Lists the displays, then the windows.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) async fn sources() -> Result<Vec<ShareSource>, String> {
    let plain = |e: share::Error| -> String {
        #[cfg(target_os = "macos")]
        if !share::grant::granted() {
            return "Irori needs the Screen Recording permission to share: System Settings → Privacy & Security → Screen & System Audio Recording".into();
        }
        format!("couldn't list what can be shared: {e}")
    };
    let mut out: Vec<ShareSource> = share::displays()
        .await
        .map_err(plain)?
        .into_iter()
        .map(|d| ShareSource {
            id: format!("d:{}", d.id),
            kind: "display".into(),
            title: d.name,
            app: String::new(),
            width: d.width,
            height: d.height,
        })
        .collect();
    out.extend(
        share::windows()
            .await
            .map_err(plain)?
            .into_iter()
            .filter(|w| !w.title.trim().is_empty() && w.width >= 64 && w.height >= 64)
            .map(|w| ShareSource {
                id: format!("w:{}", w.id),
                kind: "window".into(),
                title: w.title,
                app: w.app,
                width: w.width,
                height: w.height,
            }),
    );
    Ok(out)
}

/// Runs the capture and the encoder on a thread of their own until the
/// share stops. Returns once the picture is open (or failed to open).
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn spawn_encoder(
    hub: Weak<ShareHub>,
    ctl: Arc<EncCtl>,
    picture: Picture,
) -> Result<(), String> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    std::thread::Builder::new()
        .name("irori-share-encoder".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                    return;
                }
            };
            let why = rt.block_on(encode_loop(&hub, &ctl, picture, ready_tx));
            if let Some(h) = hub.upgrade() {
                h.end(Some(&ctl), why);
            }
        })
        .map_err(|e| e.to_string())?;
    match ready_rx.recv_timeout(Duration::from_secs(20)) {
        Ok(r) => r,
        Err(_) => Err("the screen capture didn't start".into()),
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
enum Frames {
    Capture(share::Capture),
    Synthetic {
        w: u32,
        h: u32,
        n: u64,
        next: tokio::time::Instant,
    },
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl Frames {
    fn size(&self) -> (u32, u32) {
        match self {
            Frames::Capture(c) => c.size(),
            Frames::Synthetic { w, h, .. } => (*w, *h),
        }
    }

    async fn next(&mut self, t0: std::time::Instant) -> share::Result<share::Frame> {
        match self {
            Frames::Capture(c) => c.next().await,
            Frames::Synthetic { w, h, n, next } => {
                tokio::time::sleep_until(*next).await;
                *next += Duration::from_millis(1000 / u64::from(FPS));
                *n += 1;
                share::synthetic_frame(*w, *h, *n, t0.elapsed())
            }
        }
    }
}

/// The capture → encode loop. `None` when the share was stopped; else why
/// it ended, in plain words.
#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn encode_loop(
    hub: &Weak<ShareHub>,
    ctl: &Arc<EncCtl>,
    picture: Picture,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
) -> Option<String> {
    let base = share::Settings {
        fps: FPS,
        bitrate_kbps: Some(START_KBPS),
        keyframe_every: KEY_EVERY,
        ..share::Settings::default()
    };
    let mut frames = match &picture {
        Picture::Synthetic(w, h) => Frames::Synthetic {
            w: *w,
            h: *h,
            n: 0,
            next: tokio::time::Instant::now(),
        },
        Picture::Source(src) => match share::Capture::open(src, &base).await {
            Ok(c) => Frames::Capture(c),
            Err(e) => {
                let why = plain_capture_error(&e);
                let _ = ready.send(Err(why.clone()));
                return Some(why);
            }
        },
    };
    let new_encoder = |size: (u32, u32), kbps: u32| {
        let mut s = base.sized(size);
        s.bitrate_kbps = Some(kbps);
        share::Encoder::new(&s)
    };
    let mut kbps = ctl.kbps.load(Ordering::SeqCst);
    let mut enc = match new_encoder(frames.size(), kbps) {
        Ok(e) => e,
        Err(e) => {
            let why = format!("couldn't start the video encoder: {e}");
            let _ = ready.send(Err(why.clone()));
            return Some(why);
        }
    };
    let _ = ready.send(Ok(()));
    let (w, h) = frames.size();
    if !hub.upgrade().is_some_and(|hb| hb.announce(ctl, w, h, kbps)) {
        return None;
    }
    let t0 = std::time::Instant::now();
    let mut ramp = Ramp::new(t0);
    let mut last_key: Option<std::time::Instant> = None;
    loop {
        if ctl.stop.load(Ordering::SeqCst) {
            return None;
        }
        // Capture::next is cancel-safe: wake now and then to see a stop.
        let f = tokio::select! {
            f = frames.next(t0) => f,
            _ = tokio::time::sleep(Duration::from_millis(250)) => continue,
        };
        let frame = match f {
            Ok(f) => f,
            Err(share::Error::Reopen) => {
                if let Frames::Capture(c) = &mut frames
                    && let Err(e) = c.reopen().await
                {
                    return Some(plain_capture_error(&e));
                }
                enc = match new_encoder(frames.size(), kbps) {
                    Ok(e) => e,
                    Err(e) => return Some(format!("couldn't restart the video encoder: {e}")),
                };
                last_key = None;
                let (w, h) = frames.size();
                if !hub.upgrade().is_some_and(|hb| hb.announce(ctl, w, h, kbps)) {
                    return None;
                }
                continue;
            }
            Err(e) => return Some(plain_capture_error(&e)),
        };
        let now = std::time::Instant::now();
        let want = ramp.tick(ctl, now);
        if want != kbps && enc.set_bitrate_kbps(want).is_ok() {
            kbps = want;
            let (w, h) = frames.size();
            if let Some(hb) = hub.upgrade() {
                hb.announce(ctl, w, h, kbps);
            }
        }
        if ctl.key_ask.load(Ordering::SeqCst)
            && last_key.is_none_or(|k| now.duration_since(k) >= KEY_GAP)
        {
            ctl.key_ask.store(false, Ordering::SeqCst);
            let _ = enc.request_keyframe();
        }
        let chunks = match enc.encode(&frame) {
            Ok(c) => c,
            Err(e) => return Some(format!("the video encoder stopped: {e}")),
        };
        let hb = hub.upgrade()?;
        for c in chunks {
            if c.keyframe {
                last_key = Some(now);
            }
            hb.push(
                ctl,
                Au {
                    ts_us: c.timestamp_us,
                    key: c.keyframe,
                    data: c.data,
                },
            );
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn plain_capture_error(e: &share::Error) -> String {
    #[cfg(target_os = "macos")]
    if !share::grant::granted() {
        return "Irori needs the Screen Recording permission to share: System Settings → Privacy & Security → Screen & System Audio Recording".into();
    }
    match e {
        share::Error::Video(moq) if format!("{moq:?}").contains("SourceUnavailable") => {
            "the shared window closed or capture was stopped".into()
        }
        _ => format!("the screen capture stopped: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(ms: i64) -> ShareInfo {
        ShareInfo {
            v: 1,
            share_id: "0123456789abcdef0123456789abcdef".into(),
            started_ms: ms,
            kind: "window".into(),
            title: "T".into(),
            app: "A".into(),
            width: 2,
            height: 2,
            fps: 10,
            kbps: 1000,
        }
    }

    #[test]
    fn the_newer_share_wins_then_the_higher_device() {
        assert!(newer(&info(2), "a", &info(1), "b"));
        assert!(!newer(&info(1), "z", &info(2), "a"));
        assert!(newer(&info(1), "b", &info(1), "a"));
        assert!(!newer(&info(1), "a", &info(1), "b"));
    }

    #[test]
    fn rate_ramps_to_the_cap_and_backs_off_when_behind() {
        let ctl = EncCtl::new();
        let t0 = std::time::Instant::now();
        let mut r = Ramp::new(t0);
        assert_eq!(r.tick(&ctl, t0), START_KBPS);
        let mut k = START_KBPS;
        let mut t = t0;
        for _ in 0..20 {
            t += RAMP_EVERY;
            k = r.tick(&ctl, t);
        }
        assert_eq!(k, CAP_KBPS);
        ctl.fell_behind();
        assert_eq!(ctl.kbps.load(Ordering::SeqCst), CAP_KBPS * 7 / 10);
        assert!(ctl.key_ask.load(Ordering::SeqCst));
        // No ramp right after falling behind.
        t += RAMP_EVERY;
        assert_eq!(r.tick(&ctl, t), CAP_KBPS * 7 / 10);
        for _ in 0..10 {
            ctl.fell_behind();
        }
        assert_eq!(ctl.kbps.load(Ordering::SeqCst), MIN_KBPS);
    }
}
