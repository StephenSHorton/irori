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

/// Invite windows (stage 2, card #44): an invite lives at most an hour, and
/// a member may have 2 open beside its 2 pairing windows (v0.5 §4.1).
pub const INVITE_MAX_TTL: Duration = Duration::from_secs(60 * 60);
pub const MAX_INVITE_WINDOWS_PER_MEMBER: usize = 2;

const TOKEN_LABEL: &[u8] = b"irori/pair-guest/1";
const INVITE_TOKEN_LABEL: &[u8] = b"irori/invite-guest/1";
const INVITE_TAG_LABEL: &[u8] = b"irori/invite-tag/1";
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

/// The relay auth token an invitee presents for the invite room `key`
/// (accepted by [`GuestWindows::token_ok`] while its invite window is open).
pub fn invite_token(key: &[u8; 32]) -> String {
    let mut m = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("any key length");
    m.update(INVITE_TOKEN_LABEL);
    hex::encode(&m.finalize().into_bytes()[..16])
}

/// The tag an invitee names the invite room by on the LAN bridge.
pub fn invite_tag(topic: &str) -> String {
    let mut h = Sha256::new();
    h.update(INVITE_TAG_LABEL);
    h.update(topic.as_bytes());
    hex::encode(&h.finalize()[..16])
}

/// Which kind of guest window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    /// A pairing window (10 minutes, 2 per member).
    Pair,
    /// A board invite window (1 hour, 2 per member, beside pairing).
    Invite,
}

/// One open window.
#[derive(Clone)]
pub struct Window {
    pub id: u64,
    pub kind: WindowKind,
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

    /// Opens a pairing window owned by `owner` for `ttl` (capped at
    /// [`MAX_TTL`]). `None` when the owner already has
    /// [`MAX_WINDOWS_PER_MEMBER`] pairing windows open.
    pub fn open_window(&self, owner: EndpointId, secret: Secret, ttl: Duration) -> Option<u64> {
        self.open_kind(WindowKind::Pair, owner, secret, ttl)
    }

    /// Opens an invite window owned by `owner` for `ttl` (capped at
    /// [`INVITE_MAX_TTL`]), at most [`MAX_INVITE_WINDOWS_PER_MEMBER`] per
    /// owner beside its pairing windows; its token is [`invite_token`] and
    /// its tag [`invite_tag`]. While it is open the home relay admits a
    /// client presenting the token. Close it with
    /// [`GuestWindows::close_window`].
    pub fn open_invite_window(
        &self,
        owner: EndpointId,
        secret: Secret,
        ttl: Duration,
    ) -> Result<u64, irori_meshseam::TransportError> {
        self.open_kind(WindowKind::Invite, owner, secret, ttl)
            .ok_or_else(|| {
                irori_meshseam::TransportError(format!(
                    "at most {MAX_INVITE_WINDOWS_PER_MEMBER} invites open at a time"
                ))
            })
    }

    fn open_kind(
        &self,
        kind: WindowKind,
        owner: EndpointId,
        secret: Secret,
        ttl: Duration,
    ) -> Option<u64> {
        let (cap, max_ttl, token, tag) = match kind {
            WindowKind::Pair => (
                MAX_WINDOWS_PER_MEMBER,
                MAX_TTL,
                guest_token(&secret.key),
                room_tag(&secret.topic),
            ),
            WindowKind::Invite => (
                MAX_INVITE_WINDOWS_PER_MEMBER,
                INVITE_MAX_TTL,
                invite_token(&secret.key),
                invite_tag(&secret.topic),
            ),
        };
        let mut g = self.lock();
        let held = g
            .windows
            .iter()
            .filter(|w| w.owner == owner && w.kind == kind)
            .count();
        if held >= cap {
            return None;
        }
        g.next += 1;
        let id = g.next;
        g.windows.push(Window {
            id,
            kind,
            owner,
            token,
            tag,
            secret,
            until: Instant::now() + ttl.min(max_ttl),
        });
        Some(id)
    }

    /// The kind of window `id`, while it is open.
    pub fn kind(&self, id: u64) -> Option<WindowKind> {
        self.lock()
            .windows
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.kind)
    }

    /// Whether a window of `kind` is open. The endpoint lets strangers in on
    /// `irori/pair/1` only while a pairing window is open, never for an
    /// invite window.
    pub fn any_open_of(&self, kind: WindowKind) -> bool {
        self.lock().windows.iter().any(|w| w.kind == kind)
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

    /// The kind of the open window whose token is `token`.
    pub fn kind_of_token(&self, token: &str) -> Option<WindowKind> {
        self.lock()
            .windows
            .iter()
            .find(|w| {
                w.token.len() == token.len() && subtle_eq(w.token.as_bytes(), token.as_bytes())
            })
            .map(|w| w.kind)
    }
}

fn subtle_eq(a: &[u8], b: &[u8]) -> bool {
    let mut d = 0u8;
    for (x, y) in a.iter().zip(b) {
        d |= x ^ y;
    }
    d == 0 && a.len() == b.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(b: u8) -> Secret {
        Secret {
            key: [b; 32],
            topic: format!("irori/invite/1/{b}"),
            epoch: 0,
        }
    }

    fn owner(b: u8) -> EndpointId {
        iroh::SecretKey::from_bytes(&[b; 32]).public()
    }

    #[test]
    fn invite_windows_admit_their_token_only_while_open() {
        let g = GuestWindows::default();
        let s = secret(1);
        let tok = invite_token(&s.key);
        assert!(!g.token_ok(&tok));
        let id = g
            .open_invite_window(owner(1), s.clone(), Duration::from_secs(60))
            .unwrap();
        assert_eq!(g.kind(id), Some(WindowKind::Invite));
        assert!(g.token_ok(&tok));
        assert_eq!(g.kind_of_token(&tok), Some(WindowKind::Invite));
        // The pairing token of the same key opens nothing.
        assert!(!g.token_ok(&guest_token(&s.key)));
        // An invite window never opens pairing to strangers.
        assert!(g.any_open() && !g.any_open_of(WindowKind::Pair));
        assert_eq!(g.by_tag(&invite_tag(&s.topic)).map(|w| w.id), Some(id));
        g.close_window(id);
        assert!(!g.token_ok(&tok));
        assert_eq!(g.kind(id), None);
    }

    #[test]
    fn invite_windows_cap_per_owner_beside_pairing_and_ttl() {
        let g = GuestWindows::default();
        for i in 0..MAX_INVITE_WINDOWS_PER_MEMBER {
            g.open_invite_window(owner(1), secret(i as u8), Duration::from_secs(60))
                .unwrap();
        }
        assert!(
            g.open_invite_window(owner(1), secret(9), Duration::from_secs(60))
                .is_err()
        );
        // Pairing windows count apart; another owner has its own.
        for i in 0..MAX_WINDOWS_PER_MEMBER {
            let p = g.open_window(owner(1), secret(20 + i as u8), Duration::from_secs(60));
            assert_eq!(g.kind(p.unwrap()), Some(WindowKind::Pair));
        }
        assert!(g.any_open_of(WindowKind::Pair));
        g.open_invite_window(owner(2), secret(30), Duration::from_secs(60))
            .unwrap();
        // The TTL is capped at an hour.
        let id = g
            .open_invite_window(owner(3), secret(31), Duration::from_secs(10 * 3600))
            .unwrap();
        let w = g.open().into_iter().find(|w| w.id == id).unwrap();
        assert!(w.until <= Instant::now() + INVITE_MAX_TTL);
        // An expired window admits nothing.
        let s = secret(40);
        g.open_invite_window(owner(4), s.clone(), Duration::from_millis(1))
            .unwrap();
        std::thread::sleep(Duration::from_millis(5));
        assert!(!g.token_ok(&invite_token(&s.key)));
    }
}
