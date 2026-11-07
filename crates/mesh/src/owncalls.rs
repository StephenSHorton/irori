//! Huddle ringing between the person's own devices (card #663): a huddle in
//! a Personal channel has no space, so its `ring`, `join`, `leave` and `end`
//! ride the private mesh `sync` channel as `call` frames, the way a space's
//! ride its board connections (`boardmesh::CallFrame`).
//!
//! - **Send.** [`Node::send_call`] to one certified device whose hello said
//!   `calls >= 1` ([`crate::frames::CALLS_PROTOCOL`]), on its session of the
//!   newest epoch. An older build never says it, so it is never rung.
//! - **Take.** A `call` frame from a certified device goes to the Core's
//!   [`OwnCalls`] as a [`CallFrame`] with an empty `space_id` (personal).
//!   The Core checks the rest: the call id, and that the conversation is a
//!   Personal channel.

use std::sync::Arc;

use crate::boardmesh::CallFrame;
use crate::frames::{CALLS_PROTOCOL, FRAME_CALL, Frame};
use crate::node::Node;
use crate::secret::mesh_secret;
use crate::session::Session;

/// Who hears call frames from the person's own devices (the Core's).
pub trait OwnCalls: Send + Sync {
    /// Certified device `peer`, whose handshake proved `peer_pub` (hex),
    /// sent `frame` (`space_id` is always "").
    fn frame(&self, peer: &str, peer_pub: &str, frame: CallFrame);
}

/// One of the person's devices connected now on the private mesh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnPeer {
    pub device_id: String,
    /// The key its handshake proved, hex.
    pub device_pub: String,
    /// The `calls` its hello said.
    pub calls: i64,
}

impl Node {
    /// Sets who hears call frames from the person's own devices.
    pub fn set_own_calls(&self, c: Option<Arc<dyn OwnCalls>>) {
        *self.own_calls.lock().unwrap_or_else(|e| e.into_inner()) = c;
    }

    /// The person's certified devices connected now that said hello, each
    /// once (its newest epoch's session), by device id.
    pub fn own_peers(&self) -> Vec<OwnPeer> {
        let mut out: std::collections::BTreeMap<String, (i64, OwnPeer)> = Default::default();
        for s in self.session_list() {
            let peer = s.conn.id();
            if !s.hello_done() || s.is_ended() || !self.certified(&peer) {
                continue;
            }
            let p = OwnPeer {
                device_id: peer.clone(),
                device_pub: hex::encode(s.conn.pub_key()),
                calls: s.peer_calls(),
            };
            match out.get(&peer) {
                Some((e, _)) if *e >= s.epoch => {}
                _ => {
                    out.insert(peer, (s.epoch, p));
                }
            }
        }
        out.into_values().map(|(_, p)| p).collect()
    }

    fn call_session(&self, device: &str) -> Option<Arc<Session>> {
        self.session_list()
            .into_iter()
            .filter(|s| s.conn.id() == device && s.hello_done() && !s.is_ended())
            .max_by_key(|s| s.epoch)
    }

    /// Sends a call `frame` to `device` alone, on its session of the newest
    /// epoch (`frame.space_id` isn't sent: it is always personal here).
    /// Refused when it isn't connected, isn't a certified device of the
    /// person, or never said it speaks [`CALLS_PROTOCOL`].
    pub fn send_call(&self, device: &str, frame: &CallFrame) -> Result<(), String> {
        if !self.certified(device) {
            return Err(format!("{device} isn't one of your devices"));
        }
        let s = self
            .call_session(device)
            .ok_or_else(|| format!("{device} isn't connected"))?;
        if s.peer_calls() < CALLS_PROTOCOL {
            return Err(format!("{device}'s Irori is too old for calls"));
        }
        s.control(Frame {
            call_id: frame.call_id.clone(),
            op: frame.op.clone(),
            conv_id: frame.conv_id.clone(),
            epoch: frame.epoch,
            devices: frame.devices.clone(),
            at_ms: frame.at_ms,
            ..Frame::of_type(FRAME_CALL)
        });
        Ok(())
    }

    /// The mesh keys a personal call is proved with, newest epoch first:
    /// the current epoch's, then the one before it while its seed is still
    /// held (a device a rotation hasn't reached yet).
    pub fn call_keys(&self) -> Vec<(i64, [u8; 32])> {
        let cur = self.o.ident.current();
        let mut out = Vec::new();
        for e in [cur.epoch, cur.epoch - 1] {
            if e < 1 {
                continue;
            }
            if let Some(seed) = self.o.ident.person_seed(e) {
                out.push((e, mesh_secret(&seed[..], &cur.id, e).key));
            }
        }
        out
    }

    /// A `call` frame on the session with certified device `peer`.
    pub(crate) fn take_call(&self, s: &Session, f: &Frame) {
        let c = self
            .own_calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(c) = c else { return };
        let peer = s.conn.id();
        c.frame(
            &peer,
            &hex::encode(s.conn.pub_key()),
            CallFrame {
                space_id: String::new(),
                call_id: f.call_id.clone(),
                op: f.op.clone(),
                conv_id: f.conv_id.clone(),
                epoch: f.epoch,
                devices: f.devices.clone(),
                at_ms: f.at_ms,
            },
        );
    }
}
