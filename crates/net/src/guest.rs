//! Relay guest windows (M7, mesh.md §5.4, architecture critic finding 3):
//! while a member has a pairing window open on a home server, the server's
//! relay admits an endpoint it doesn't know if the relay client presents the
//! window's guest token, and the server's LAN lookup carries the window's
//! room (the home/away bridge).
//!
//! A window is the pairing room's secret, owned by the member who opened it
//! (its EndpointId, so a joiner that reached the server learns whom to dial
//! through it). The token and the tag are both derived from the room key:
//! `token = hex(HMAC(key, "irori/pair-guest/1"))[..32]`,
//! `tag = hex(SHA-256("irori/pair-tag/1" || topic))[..32]`. Anyone with the
//! code can compute them, as anyone with the code could join Go's guest
//! topic; the words and the typed key still decide the pairing.
//!
//! Limits follow go-room's hub (R:hub.go:25,49-71): at most 8 guests on a
//! relay, 2 windows per member, a window lives at most its TTL (10 minutes,
//! the pairing window) and ends at once when its owner closes it.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use hmac::{Hmac, KeyInit, Mac};
use iroh::EndpointId;
use irori_meshseam::Secret;
use sha2::{Digest, Sha256};

/// Guests on one relay at a time.
pub const MAX_GUESTS: usize = 8;
/// Open windows per member.
pub const MAX_WINDOWS_PER_MEMBER: usize = 2;
/// The longest a window stays open.
pub const MAX_TTL: Duration = Duration::from_secs(10 * 60);

const TOKEN_LABEL: &[u8] = b"irori/pair-guest/1";
const TAG_LABEL: &[u8] = b"irori/pair-tag/1";

/// The relay auth token a joiner presents for the room `key`.
pub fn guest_token(key: &[u8; 32]) -> String {
    let mut m = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("any key length");
    m.update(TOKEN_LABEL);
    hex::encode(&m.finalize().into_bytes()[..16])
}

/// The tag a joiner names the room by when it asks a server who holds it.
pub fn room_tag(topic: &str) -> String {
    let mut h = Sha256::new();
    h.update(TAG_LABEL);
    h.update(topic.as_bytes());
    hex::encode(&h.finalize()[..16])
}

/// One open window.
#[derive(Clone)]
pub struct Window {
    pub id: u64,
    pub owner: EndpointId,
    pub secret: Secret,
    pub token: String,
    pub tag: String,
    pub until: Instant,
}

#[derive(Default)]
struct State {
    next: u64,
    windows: Vec<Window>,
}

/// The windows open on this device's relay. Cheap to clone.
#[derive(Clone, Default)]
pub struct GuestWindows(Arc<Mutex<State>>);

impl std::fmt::Debug for GuestWindows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestWindows")
            .field("open", &self.open().len())
            .finish()
    }
}

impl GuestWindows {
    fn lock(&self) -> MutexGuard<'_, State> {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        g.windows.retain(|w| w.until > now);
        g
    }

    /// Opens a window owned by `owner` for `ttl` (capped at [`MAX_TTL`]).
    /// `None` when the owner already has [`MAX_WINDOWS_PER_MEMBER`] open.
    pub fn open_window(&self, owner: EndpointId, secret: Secret, ttl: Duration) -> Option<u64> {
        let mut g = self.lock();
        if g.windows.iter().filter(|w| w.owner == owner).count() >= MAX_WINDOWS_PER_MEMBER {
            return None;
        }
        g.next += 1;
        let id = g.next;
        let w = Window {
            id,
            owner,
            token: guest_token(&secret.key),
            tag: room_tag(&secret.topic),
            secret,
            until: Instant::now() + ttl.min(MAX_TTL),
        };
        g.windows.push(w);
        Some(id)
    }

    pub fn close_window(&self, id: u64) {
        self.lock().windows.retain(|w| w.id != id);
    }

    /// Every open window.
    pub fn open(&self) -> Vec<Window> {
        self.lock().windows.clone()
    }

    pub fn any_open(&self) -> bool {
        !self.lock().windows.is_empty()
    }

    /// Whether `token` opens a window here.
    pub fn token_ok(&self, token: &str) -> bool {
        self.lock().windows.iter().any(|w| {
            w.token.len() == token.len() && subtle_eq(w.token.as_bytes(), token.as_bytes())
        })
    }

    /// The window a joiner names by its tag.
    pub fn by_tag(&self, tag: &str) -> Option<Window> {
        self.lock().windows.iter().find(|w| w.tag == tag).cloned()
    }
}

fn subtle_eq(a: &[u8], b: &[u8]) -> bool {
    let mut d = 0u8;
    for (x, y) in a.iter().zip(b) {
        d |= x ^ y;
    }
    d == 0 && a.len() == b.len()
}
