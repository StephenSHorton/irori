//! Voice calls on iroh (#654): the `irori/call/1` connection.
//!
//! A call is a full mesh of devices. Each pair has its own QUIC connection
//! on the endpoint, apart from the board connection, so audio has its own
//! congestion control. Opus frames ride its QUIC datagrams; the first bi
//! stream is the call's control stream.
//!
//! - The endpoint gate admits `irori/call/1` like `irori/board/1`: this
//!   person's devices, or a member device of a board held here
//!   ([`crate::board::MemberAdmit`]).
//! - The dialer opens the first bi stream, label `call`, and writes a
//!   [`CallOpenMsg`]; the acceptor answers with its own. The proof is
//!   `hex(HMAC-SHA256(key, "irori/call-open/1" || callId || EKM32 || role))`,
//!   EKM = `EXPORTER-irori-call`, role 0 from the dialer and 1 from the
//!   acceptor. The key is the space's board topic key of the epoch named
//!   (the mesh key for a personal call). A call id is exactly 32 lower-case
//!   hex characters, so the label needs no length prefix.
//! - The call's own `accept` (the space's verifier) runs on the device the
//!   message names and the key that dialed.
//! - A peer whose QUIC has no datagrams is closed: it can't do calls.
//! - Two live connections to one peer in one call: the boards' `replaced`
//!   rule picks one ([`crate::transport::keep_new`]). [`IrohNet::call_dial`]
//!   doesn't dial a peer it is already dialing or connected to in the call,
//!   so the rule only meets dials from both ends.
//! - Revocation reaches a handshake in flight: each call counts
//!   rechecks and drops, and a connection whose open overlapped one is
//!   checked again before it joins the call.
//! - [`IrohNet::recheck`], [`IrohNet::drop_device`], [`IrohNet::drop_board`]
//!   and [`IrohNet::close`] reach call connections too.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use iroh::EndpointId;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use irori_meshseam::{MAX_LABEL, TransportError, read_frame, reason, write_frame};
use serde::{Deserialize, Serialize};

use crate::transport::{
    Inner, IrohNet, OPEN_MAX, PeerAddr, ROLE_ACCEPT, ROLE_DIAL, close_conn, closed_error, ekm,
    keep_new,
};

/// The call connection's ALPN: one connection per (device, device, call).
pub const CALL_ALPN: &[u8] = b"irori/call/1";
/// The call-open proof's label; the call id follows it.
pub const CALL_OPEN_LABEL: &[u8] = b"irori/call-open/1";
/// The TLS exporter label of the call-open proof's EKM32.
pub const EKM_CALL: &[u8] = b"EXPORTER-irori-call";
/// The call-open message's version.
pub const CALL_OPEN_V: u32 = 1;
/// The label of a call connection's first bi stream, its control stream.
pub const CALL_LABEL: &str = "call";
/// The close reason for a peer whose QUIC has no datagrams.
pub const NO_DATAGRAMS: &str = "calls unsupported: no datagrams";

/// The first message on a call connection's `call` stream, each way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallOpenMsg {
    pub v: u32,
    /// The space (board) the call is in; "" for a personal call.
    #[serde(rename = "spaceId")]
    pub space_id: String,
    pub epoch: i64,
    #[serde(rename = "callId")]
    pub call_id: String,
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub proof: String,
}

/// Whether `id` is a call id: exactly 32 lower-case hex characters.
pub fn is_call_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn call_label(call_id: &str) -> Vec<u8> {
    let mut l = CALL_OPEN_LABEL.to_vec();
    l.extend_from_slice(call_id.as_bytes());
    l
}

/// The call-open proof of call `call_id` for `key` (the epoch's key).
pub fn call_open_proof(key: &[u8; 32], call_id: &str, ekm: &[u8; 32], role: u8) -> String {
    crate::transport::open_proof(&call_label(call_id), key, ekm, role)
}

/// Checks a call-open proof in constant time.
pub fn call_open_proof_ok(
    key: &[u8; 32],
    call_id: &str,
    ekm: &[u8; 32],
    role: u8,
    got: &str,
) -> bool {
    crate::transport::open_proof_ok(&call_label(call_id), key, ekm, role, got)
}

/// Whether a call takes device `peer` dialing or answering with key `id`.
pub type CallAccept = Arc<dyn Fn(&str, &EndpointId) -> bool + Send + Sync>;
/// Hands over each peer connection of a call once it is open.
pub type OnCallPeer = Arc<dyn Fn(CallConn) + Send + Sync>;

/// A call to join ([`IrohNet::call_join`]).
#[derive(Clone)]
pub struct CallJoin {
    /// 32 lower-case hex characters.
    pub call_id: String,
    /// The space the call is in; "" for a personal call.
    pub space_id: String,
    /// The epochs this call accepts, with their keys. `keys[0]` is the one
    /// this device proves with when it dials.
    pub keys: Vec<(i64, [u8; 32])>,
    /// The space's check of a peer device and its key.
    pub accept: CallAccept,
    /// Called once per peer connection, dialed or accepted.
    pub on_peer: OnCallPeer,
}

impl fmt::Debug for CallJoin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallJoin")
            .field("call_id", &self.call_id)
            .field("space_id", &self.space_id)
            .field("epochs", &self.keys.iter().map(|k| k.0).collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// One open peer connection of a call. Audio goes on `conn`'s datagrams
/// (`send_datagram`, `read_datagram`); `control` is the `call` stream, past
/// the open messages.
#[derive(Debug)]
pub struct CallConn {
    /// The peer's device id.
    pub peer: String,
    pub conn: Connection,
    pub control: (SendStream, RecvStream),
}

/// A joined call. [`CallHandle::leave`], or dropping it, closes every
/// connection of the call `leaving` and stops accepting new ones.
pub struct CallHandle {
    net: Weak<Inner>,
    state: Arc<CallState>,
}

impl fmt::Debug for CallHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallHandle")
            .field("call_id", &self.state.join.call_id)
            .finish_non_exhaustive()
    }
}

impl CallHandle {
    pub fn call_id(&self) -> &str {
        &self.state.join.call_id
    }

    /// The peers with a live connection, sorted.
    pub fn peers(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .state
            .conns
            .lock()
            .expect("call conns")
            .iter()
            .filter(|(_, e)| e.conn.close_reason().is_none())
            .map(|(p, _)| p.clone())
            .collect();
        v.sort();
        v
    }

    /// Leaves the call. Calling it again does nothing.
    pub fn leave(&self) {
        match self.net.upgrade() {
            Some(inner) => inner.close_call(&self.state, reason::LEAVING),
            None => self.state.close_all(reason::LEAVING),
        }
    }
}

impl Drop for CallHandle {
    fn drop(&mut self) {
        self.leave();
    }
}

/// A joined call as the endpoint holds it.
pub(crate) struct CallState {
    pub(crate) join: CallJoin,
    conns: Mutex<HashMap<String, Arc<CallEntry>>>,
    closed: AtomicBool,
    /// Peers [`IrohNet::call_dial`] is dialing in this call.
    dialing: Mutex<HashSet<String>>,
    /// Bumped, under `conns`, by every recheck and drop: a handshake that
    /// began at an older value is checked again before its connection is
    /// taken.
    revoke_gen: AtomicU64,
    /// The `revoke_gen` and reason of each peer's latest drop.
    dropped: Mutex<HashMap<String, (u64, String)>>,
}

/// Unmarks a peer as being dialed when the dial ends, however it ends.
struct Dialing<'a> {
    call: &'a CallState,
    peer: String,
}

impl Drop for Dialing<'_> {
    fn drop(&mut self) {
        self.call
            .dialing
            .lock()
            .expect("dialing")
            .remove(&self.peer);
    }
}

struct CallEntry {
    dialer: String,
    conn: Connection,
}

impl CallState {
    fn key(&self, epoch: i64) -> Option<[u8; 32]> {
        self.join
            .keys
            .iter()
            .find(|(e, _)| *e == epoch)
            .map(|(_, k)| *k)
    }

    fn entries(&self) -> Vec<(String, Arc<CallEntry>)> {
        self.conns
            .lock()
            .expect("call conns")
            .iter()
            .map(|(p, e)| (p.clone(), e.clone()))
            .collect()
    }

    fn new(join: CallJoin) -> CallState {
        CallState {
            join,
            conns: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            dialing: Mutex::new(HashSet::new()),
            revoke_gen: AtomicU64::new(0),
            dropped: Mutex::new(HashMap::new()),
        }
    }

    /// The revocation count a handshake starts from.
    fn revokes(&self) -> u64 {
        self.revoke_gen.load(Ordering::SeqCst)
    }

    /// Whether a live connection to `peer` is in the call.
    fn live(&self, peer: &str) -> bool {
        self.conns
            .lock()
            .expect("call conns")
            .get(peer)
            .is_some_and(|e| e.conn.close_reason().is_none())
    }

    /// Re-runs `accept` on every connection; the ones it now refuses close
    /// `revoked`.
    pub(crate) fn recheck(&self) {
        {
            let _conns = self.conns.lock().expect("call conns");
            self.revoke_gen.fetch_add(1, Ordering::SeqCst);
        }
        for (peer, e) in self.entries() {
            if !(self.join.accept)(&peer, &e.conn.remote_id()) {
                close_conn(&e.conn, reason::REVOKED);
            }
        }
    }

    /// Closes the connection to `peer`, if any, with `why`.
    pub(crate) fn drop_peer(&self, peer: &str, why: &str) {
        let e = {
            let conns = self.conns.lock().expect("call conns");
            let g = self.revoke_gen.fetch_add(1, Ordering::SeqCst) + 1;
            self.dropped
                .lock()
                .expect("call dropped")
                .insert(peer.to_string(), (g, why.to_string()));
            conns.get(peer).cloned()
        };
        if let Some(e) = e {
            close_conn(&e.conn, why);
        }
    }

    fn close_all(&self, why: &str) {
        self.closed.store(true, Ordering::SeqCst);
        let all: Vec<_> = self
            .conns
            .lock()
            .expect("call conns")
            .drain()
            .map(|(_, e)| e)
            .collect();
        for e in all {
            close_conn(&e.conn, why);
        }
    }
}

impl IrohNet {
    /// Joins call `j.call_id`: incoming `irori/call/1` connections for it
    /// are taken from now on, and [`IrohNet::call_dial`] can dial its
    /// peers. Joining a call already joined here replaces it.
    pub async fn call_join(&self, j: CallJoin) -> Result<CallHandle, TransportError> {
        if !is_call_id(&j.call_id) {
            return Err(TransportError(
                "call: a call id is 32 lower-case hex characters".into(),
            ));
        }
        if j.keys.is_empty() {
            return Err(TransportError("call: a call needs a key".into()));
        }
        let inner = self.inner();
        inner.bind().await?;
        let state = Arc::new(CallState::new(j));
        let old = inner
            .calls
            .lock()
            .expect("calls")
            .insert(state.join.call_id.clone(), state.clone());
        if let Some(old) = old {
            old.close_all(reason::LEAVING);
        }
        Ok(CallHandle {
            net: Arc::downgrade(inner),
            state,
        })
    }

    /// Dials `peer_device` for joined call `call_id`, at its address in the
    /// space's board book, else the private book. On success its
    /// [`CallConn`] goes to the call's `on_peer`, as an incoming one does.
    ///
    /// A peer the call already has a live connection to is not dialed
    /// again (`Ok`); a peer this call is already dialing is an error, so
    /// two dials from one end never race each other's `replaced`.
    pub async fn call_dial(&self, call_id: &str, peer_device: &str) -> Result<(), String> {
        if !is_call_id(call_id) {
            return Err(format!("call: {call_id:?} is not a call id"));
        }
        let inner = self.inner();
        if peer_device.is_empty() || peer_device == inner.cfg.self_id {
            return Err(format!("call: can't dial {peer_device:?}"));
        }
        let call = inner
            .calls
            .lock()
            .expect("calls")
            .get(call_id)
            .cloned()
            .ok_or_else(|| format!("call: not in call {call_id}"))?;
        if call.live(peer_device) {
            return Ok(());
        }
        if !call
            .dialing
            .lock()
            .expect("dialing")
            .insert(peer_device.to_string())
        {
            return Err(format!("call: already dialing {peer_device}"));
        }
        let _dialing = Dialing {
            call: &call,
            peer: peer_device.to_string(),
        };
        // The dial that just ended may have opened it.
        if call.live(peer_device) {
            return Ok(());
        }
        let since = call.revokes();
        let space = &call.join.space_id;
        let addr = inner
            .addr_of((!space.is_empty()).then_some(space.as_str()), peer_device)
            .ok_or_else(|| format!("call: no address for {peer_device}"))?;
        let ep = inner.bind().await.map_err(|e| e.0)?;
        let conn = tokio::time::timeout(
            inner.cfg.connect_timeout,
            ep.connect(
                crate::public::with_public(addr.endpoint_addr(), inner.cfg.public_relays.on()),
                CALL_ALPN,
            ),
        )
        .await
        .map_err(|_| reason::TIMEOUT.to_string())?
        .map_err(|e| format!("connect: {e:#}"))?;
        inner
            .dial_call(conn, &call, peer_device, &addr, since)
            .await
    }
}

impl Inner {
    pub(crate) fn calls_snapshot(&self) -> Vec<Arc<CallState>> {
        self.calls
            .lock()
            .expect("calls")
            .values()
            .cloned()
            .collect()
    }

    /// Ends `call` here: unregisters it and closes its connections `why`.
    pub(crate) fn close_call(&self, call: &Arc<CallState>, why: &str) {
        {
            let mut calls = self.calls.lock().expect("calls");
            if calls
                .get(&call.join.call_id)
                .is_some_and(|c| Arc::ptr_eq(c, call))
            {
                calls.remove(&call.join.call_id);
            }
        }
        call.close_all(why);
    }

    /// The call-open exchange, dialer side: write our [`CallOpenMsg`] on
    /// `call`, read the acceptor's, check its proof (role 1) and that the
    /// device it names is `peer` and taken by the call's `accept`.
    async fn dial_call(
        self: &Arc<Self>,
        conn: Connection,
        call: &Arc<CallState>,
        peer: &str,
        addr: &PeerAddr,
        since: u64,
    ) -> Result<(), String> {
        let (epoch, key) = call.join.keys[0];
        let call_id = call.join.call_id.as_str();
        let k = ekm(&conn, EKM_CALL).map_err(|e| e.to_string())?;
        let hs = async {
            let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
            write_frame(&mut send, CALL_LABEL.as_bytes(), MAX_LABEL)
                .await
                .map_err(|e| e.to_string())?;
            let open = CallOpenMsg {
                v: CALL_OPEN_V,
                space_id: call.join.space_id.clone(),
                epoch,
                call_id: call_id.to_string(),
                device_id: self.cfg.self_id.clone(),
                proof: call_open_proof(&key, call_id, &k, ROLE_DIAL),
            };
            let b = serde_json::to_vec(&open).expect("open msg");
            write_frame(&mut send, &b, OPEN_MAX)
                .await
                .map_err(|e| e.to_string())?;
            let reply = read_frame(&mut recv, OPEN_MAX, false)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "no call-open reply".to_string())?;
            let reply: CallOpenMsg = serde_json::from_slice(&reply).map_err(|e| e.to_string())?;
            Ok::<_, String>((send, recv, reply))
        };
        let (send, recv, reply) = match tokio::time::timeout(self.cfg.open_timeout, hs).await {
            Err(_) => {
                close_conn(&conn, reason::TIMEOUT);
                return Err(reason::TIMEOUT.into());
            }
            Ok(Err(e)) => {
                let err = closed_error(&conn, e);
                close_conn(&conn, reason::PROTOCOL);
                return Err(err.to_string());
            }
            Ok(Ok(x)) => x,
        };
        if reply.v != CALL_OPEN_V {
            close_conn(&conn, reason::VERSION);
            return Err(reason::VERSION.into());
        }
        if reply.call_id != call_id
            || reply.space_id != call.join.space_id
            || reply.epoch != epoch
            || !call_open_proof_ok(&key, call_id, &k, ROLE_ACCEPT, &reply.proof)
            || reply.device_id != peer
            || !(call.join.accept)(peer, &addr.id)
        {
            close_conn(&conn, reason::REFUSED);
            return Err(reason::REFUSED.into());
        }
        self.add_call_conn(
            call,
            peer.to_string(),
            self.cfg.self_id.clone(),
            conn,
            (send, recv),
            since,
        )
    }

    /// The call-open exchange, acceptor side. The endpoint gate already let
    /// the key in; this is the per-call check: a well-formed call id joined
    /// here, in the space named, an epoch the call takes, the proof of its
    /// key, and the call's `accept`. Any failure closes `refused` (`version`
    /// for an unknown open version) before a reply.
    pub(crate) async fn handle_call(self: &Arc<Self>, conn: Connection) {
        let hs = async {
            let (send, mut recv) = conn.accept_bi().await.map_err(|e| e.to_string())?;
            let label = read_frame(&mut recv, MAX_LABEL, false)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "no label".to_string())?;
            if label != CALL_LABEL.as_bytes() {
                return Err("first stream is not call".to_string());
            }
            let open = read_frame(&mut recv, OPEN_MAX, false)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "no call-open message".to_string())?;
            let open: CallOpenMsg = serde_json::from_slice(&open).map_err(|e| e.to_string())?;
            Ok::<_, String>((send, recv, open))
        };
        let (mut send, recv, open) = match tokio::time::timeout(self.cfg.open_timeout, hs).await {
            Ok(Ok(x)) => x,
            Ok(Err(_)) => return close_conn(&conn, reason::PROTOCOL),
            Err(_) => return close_conn(&conn, reason::TIMEOUT),
        };
        if open.v != CALL_OPEN_V {
            return close_conn(&conn, reason::VERSION);
        }
        if !is_call_id(&open.call_id) {
            return close_conn(&conn, reason::REFUSED);
        }
        let call = self
            .calls
            .lock()
            .expect("calls")
            .get(&open.call_id)
            .cloned();
        let Some(call) = call else {
            return close_conn(&conn, reason::REFUSED);
        };
        // A drop or recheck from here on is seen again in `add_call_conn`.
        let since = call.revokes();
        if open.space_id != call.join.space_id {
            return close_conn(&conn, reason::REFUSED);
        }
        let Some(key) = call.key(open.epoch) else {
            return close_conn(&conn, reason::REFUSED);
        };
        let Ok(k) = ekm(&conn, EKM_CALL) else {
            return close_conn(&conn, reason::PROTOCOL);
        };
        if !call_open_proof_ok(&key, &open.call_id, &k, ROLE_DIAL, &open.proof) {
            return close_conn(&conn, reason::REFUSED);
        }
        let peer = open.device_id;
        if peer.is_empty()
            || peer == self.cfg.self_id
            || !(call.join.accept)(&peer, &conn.remote_id())
        {
            return close_conn(&conn, reason::REFUSED);
        }
        if conn.max_datagram_size().is_none() {
            return close_conn(&conn, NO_DATAGRAMS);
        }
        let reply = CallOpenMsg {
            v: CALL_OPEN_V,
            space_id: open.space_id,
            epoch: open.epoch,
            call_id: open.call_id,
            device_id: self.cfg.self_id.clone(),
            proof: call_open_proof(&key, &call.join.call_id, &k, ROLE_ACCEPT),
        };
        let b = serde_json::to_vec(&reply).expect("open msg");
        if write_frame(&mut send, &b, OPEN_MAX).await.is_err() {
            return close_conn(&conn, reason::PROTOCOL);
        }
        let _ = self.add_call_conn(&call, peer.clone(), peer, conn, (send, recv), since);
    }

    /// Takes an opened call connection: the datagram check, a recheck if a
    /// revocation overlapped its handshake (`since`, from before its `accept`
    /// ran), the `replaced` rule against a live one to the same peer, then
    /// `on_peer`.
    fn add_call_conn(
        self: &Arc<Self>,
        call: &Arc<CallState>,
        peer: String,
        dialer: String,
        conn: Connection,
        control: (SendStream, RecvStream),
        since: u64,
    ) -> Result<(), String> {
        if conn.max_datagram_size().is_none() {
            close_conn(&conn, NO_DATAGRAMS);
            return Err(format!("{peer} can't do calls: its QUIC has no datagrams"));
        }
        let e = Arc::new(CallEntry {
            dialer,
            conn: conn.clone(),
        });
        let remote = conn.remote_id();
        // `accept` runs outside the lock; if another revocation lands while
        // it runs, run it again.
        loop {
            let now = call.revokes();
            if now != since && !(call.join.accept)(&peer, &remote) {
                close_conn(&conn, reason::REVOKED);
                return Err(reason::REVOKED.into());
            }
            let mut conns = call.conns.lock().expect("call conns");
            if call.revokes() != now {
                drop(conns);
                continue;
            }
            if call.closed.load(Ordering::SeqCst) || self.closed() {
                close_conn(&conn, reason::LEAVING);
                return Err(reason::LEAVING.into());
            }
            let dropped = call
                .dropped
                .lock()
                .expect("call dropped")
                .get(&peer)
                .filter(|(g, _)| *g > since)
                .map(|(_, why)| why.clone());
            if let Some(why) = dropped {
                close_conn(&conn, &why);
                return Err(why);
            }
            if let Some(old) = conns.get(&peer)
                && old.conn.close_reason().is_none()
            {
                if keep_new(&self.cfg.self_id, &peer, &e.dialer, &old.dialer) {
                    close_conn(&old.conn, reason::REPLACED);
                } else {
                    close_conn(&conn, reason::REPLACED);
                    return Err(reason::REPLACED.into());
                }
            }
            conns.insert(peer.clone(), e.clone());
            break;
        }
        tokio::spawn(watch_call_conn(Arc::downgrade(call), peer.clone(), e));
        (call.join.on_peer)(CallConn {
            peer,
            conn,
            control,
        });
        Ok(())
    }
}

/// Forgets a call connection once it closes.
async fn watch_call_conn(call: Weak<CallState>, peer: String, e: Arc<CallEntry>) {
    e.conn.closed().await;
    if let Some(call) = call.upgrade() {
        let mut conns = call.conns.lock().expect("call conns");
        if conns.get(&peer).is_some_and(|x| Arc::ptr_eq(x, &e)) {
            conns.remove(&peer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_ids_are_32_lower_hex() {
        assert!(is_call_id("0123456789abcdef0123456789abcdef"));
        assert!(!is_call_id("0123456789ABCDEF0123456789abcdef"));
        assert!(!is_call_id("0123456789abcdef0123456789abcde"));
        assert!(!is_call_id("0123456789abcdef0123456789abcdef0"));
        assert!(!is_call_id("0123456789abcdef0123456789abcdeg"));
        assert!(!is_call_id(""));
    }

    #[test]
    fn open_proof_is_bound_to_role_key_and_call() {
        let (key, ekm) = ([7u8; 32], [9u8; 32]);
        let id = "0123456789abcdef0123456789abcdef";
        let p = call_open_proof(&key, id, &ekm, ROLE_DIAL);
        assert!(call_open_proof_ok(&key, id, &ekm, ROLE_DIAL, &p));
        assert!(!call_open_proof_ok(&key, id, &ekm, ROLE_ACCEPT, &p));
        assert!(!call_open_proof_ok(&[8u8; 32], id, &ekm, ROLE_DIAL, &p));
        let other = "1123456789abcdef0123456789abcdef";
        assert!(!call_open_proof_ok(&key, other, &ekm, ROLE_DIAL, &p));
        // Not the board-open proof under the same key.
        assert_ne!(p, crate::board::board_open_proof(&key, &ekm, ROLE_DIAL));
    }

    #[test]
    fn open_msg_uses_the_wire_names() {
        let m = CallOpenMsg {
            v: 1,
            space_id: "s".into(),
            epoch: 2,
            call_id: "c".into(),
            device_id: "d".into(),
            proof: "p".into(),
        };
        let j = serde_json::to_value(&m).unwrap();
        assert_eq!(
            j,
            serde_json::json!({"v":1,"spaceId":"s","epoch":2,"callId":"c","deviceId":"d","proof":"p"})
        );
    }
}
