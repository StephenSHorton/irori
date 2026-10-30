//! Board invites (card #44, v0.5-shared-boards §4.1): the Core drives one
//! invite at a time, either side, over `irori_invite`'s Host and Guest and
//! the endpoint's `irori/invite/1` room.
//!
//! - **Inviting** ([`Core::invite`], owners): a fresh code, the link with
//!   where this device can be reached, and an invite handler on the
//!   endpoint (a guest window on its relay and its home servers) for an
//!   hour. The first invitee that proves the code is served; the person
//!   types the invitee's five words ([`Core::confirm_invite_words`]).
//! - **Joining** ([`Core::join_invite`]): the link is read, the inviter is
//!   dialed as a relay guest, and the invitee's screen shows the words.
//! - **Done**, on both sides: the board opens on the network
//!   (`Core::open_board_net`) and the invitee dials the inviter's addresses
//!   from the link on `irori/board/1`.
//!
//! The window seam (`crates/window` `seams::Invites`) maps onto these calls
//! one to one; [`Core::watch_invites`] is its `boards:invite` event.

use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::SystemTime;

use gocompat::time::GoTime;
use irori_identity::{BoardSecrets, BoardVerifier, MemberPin};
use irori_invite::flow::{STATE_DONE, STATE_FAILED};
use irori_invite::{Guest, Host, INVITE_TTL, InviteBoards, Status, Ticket, Timing};
use irori_meshseam::Conn;
use irori_net::invite::{InviteAddr, InviteHandler};
use serde::Serialize;
use store::Store;

use super::{Core, Services};

/// One invite as the screens show it: the flow's status, which side this
/// device is on (`host`, `guest`, or "" for none), and on the host the link
/// to send and when it stops working (RFC 3339).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteStatus {
    #[serde(flatten)]
    pub status: Status,
    pub role: String,
    pub link: String,
    pub expires_at: String,
}

/// Told every change of the open invite.
pub type InviteWatchFn = Arc<dyn Fn(InviteStatus) + Send + Sync>;

enum Side {
    Host(Arc<Host>),
    Guest(Arc<Guest>),
}

struct Current {
    /// Bumped per invite, so an old invite's tasks stop speaking.
    serial: u64,
    side: Side,
    link: String,
    expires_at: String,
}

impl Current {
    fn status(&self) -> InviteStatus {
        let (status, role) = match &self.side {
            Side::Host(h) => (h.status(), "host"),
            Side::Guest(g) => (g.status(), "guest"),
        };
        InviteStatus {
            status,
            role: role.into(),
            link: self.link.clone(),
            expires_at: self.expires_at.clone(),
        }
    }
}

/// How long [`Core::invite`] waits for this device's own home relay to
/// come up, so the link names it (an invitee away from home dials there).
const RELAY_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// The Core's invite state.
#[derive(Default)]
pub(crate) struct Invites {
    /// An invite is open on this device as the inviter: the home server is
    /// wanted while it is (as for an open pairing window), so the link can
    /// name its relay.
    pub(crate) hosting: Arc<std::sync::atomic::AtomicBool>,
    cur: Mutex<Option<Current>>,
    next: Mutex<u64>,
    watch: Mutex<Option<InviteWatchFn>>,
    /// Tests shorten the steps.
    timing: Mutex<Option<Timing>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn ended(st: &Status) -> bool {
    st.state == STATE_DONE || st.state == STATE_FAILED
}

impl Invites {
    fn timing(&self) -> Timing {
        lock(&self.timing).unwrap_or_default()
    }

    /// Refuses a second invite while one is still going.
    fn free(&self) -> Result<(), String> {
        match lock(&self.cur).as_ref() {
            Some(c) if !ended(&c.status().status) => {
                Err("An invite is already open on this device. Finish or cancel it first.".into())
            }
            _ => Ok(()),
        }
    }

    fn status(&self) -> InviteStatus {
        lock(&self.cur)
            .as_ref()
            .map(Current::status)
            .unwrap_or_default()
    }

    fn tell(&self, serial: u64) {
        let st = match lock(&self.cur).as_ref() {
            Some(c) if c.serial == serial => c.status(),
            _ => return,
        };
        let f = lock(&self.watch).clone();
        if let Some(f) = f {
            f(st);
        }
    }

    fn is_current(&self, serial: u64) -> bool {
        lock(&self.cur).as_ref().is_some_and(|c| c.serial == serial)
    }
}

/// The Core's boards as the invite needs them.
struct CoreBoards(Weak<Services>);

impl CoreBoards {
    fn s(&self) -> Option<Arc<Services>> {
        self.0.upgrade()
    }
}

impl InviteBoards for CoreBoards {
    fn open_board(&self, board_id: &str, creator: Option<MemberPin>) -> Result<(), String> {
        let s = self.s().ok_or("Irori is closing")?;
        s.open_board(board_id, false, creator.as_ref())
            .map_err(|e| e.to_string())?;
        s.boards.moved();
        Ok(())
    }
    fn board_store(&self, board_id: &str) -> Option<Arc<Store>> {
        self.s()?.board(board_id).ok().map(|b| b.store.clone())
    }
    fn board_verifier(&self, board_id: &str) -> Option<Arc<BoardVerifier>> {
        self.s()?.board(board_id).ok().map(|b| b.verifier.clone())
    }
    fn board_secrets(&self, board_id: &str) -> Option<Arc<BoardSecrets>> {
        self.s()?.board(board_id).ok().map(|b| b.secrets.clone())
    }
    fn pinned_key(&self, person_id: &str) -> Option<String> {
        let s = self.s()?;
        s.boards.list().iter().find_map(|b| {
            b.verifier
                .membership()
                .person(person_id)
                .map(|p| p.pin.person_pub.clone())
        })
    }
}

/// The open boards' secrets, for `board.secret` to the person's other
/// devices (`mesh::ownboards`).
pub(super) struct OwnBoardSecrets(pub(super) Weak<Services>);

impl mesh::ownboards::OwnBoards for OwnBoardSecrets {
    fn open_boards(&self) -> Vec<(String, Arc<BoardSecrets>)> {
        self.0
            .upgrade()
            .map(|s| {
                s.boards
                    .list()
                    .iter()
                    .map(|b| (b.id.clone(), b.secrets.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn vouched(&self, board_id: &str, maker: &str) {
        if let Some(s) = self.0.upgrade() {
            s.boards
                .vouched
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert((board_id.to_string(), maker.to_string()));
        }
    }
}

/// Where the link says the inviter is.
fn invite_addr(t: &Ticket) -> Result<InviteAddr, String> {
    let key = |h: &str| -> Option<iroh::EndpointId> {
        let b: [u8; 32] = hex::decode(h).ok()?.try_into().ok()?;
        iroh::EndpointId::from_bytes(&b).ok()
    };
    Ok(InviteAddr {
        id: key(&t.endpoint).ok_or("That invite link is damaged.")?,
        relay: t.relay.parse().ok(),
        relay_quic_port: t.relay_quic_port,
        relay_pub: key(&t.relay_key),
        direct: t.addrs.iter().filter_map(|a| a.parse().ok()).collect(),
    })
}

impl Core {
    fn invite_boards(&self) -> Arc<dyn InviteBoards> {
        Arc::new(CoreBoards(Arc::downgrade(&self.s)))
    }

    /// Shortens the invite's steps (tests).
    pub fn set_invite_timing(&self, t: Timing) {
        *lock(&self.s.invites.timing) = Some(t);
    }

    /// Opens an invite to board `board_id` (owners): a fresh code and the
    /// link to send, good for an hour or one invitee.
    pub async fn invite(&self, board_id: &str) -> Result<InviteStatus, String> {
        let inv = &self.s.invites;
        inv.free()?;
        let host = Host::new(
            self.s.ident.clone(),
            self.invite_boards(),
            board_id,
            inv.timing(),
        )
        .map_err(|e| e.0)?;
        let host = Arc::new(host);
        let net = self.s.mesh.net.clone();
        self.set_hosting(true);
        if let Some(h) = &self.s.home
            && h.settings().on
        {
            // The server starts for the invite; the link names its relay
            // once the endpoint homes on it.
            let start = std::time::Instant::now();
            while start.elapsed() < RELAY_WAIT && !net.home_relays().iter().any(|(_, up)| *up) {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
        let a = match net.invite_addr().await {
            Ok(a) => a,
            Err(e) => {
                self.set_hosting(false);
                return Err(e.0);
            }
        };
        let link = host
            .ticket(
                &hex::encode(a.id.as_bytes()),
                &a.relay.as_ref().map(|u| u.to_string()).unwrap_or_default(),
                &a.relay_pub
                    .map(|k| hex::encode(k.as_bytes()))
                    .unwrap_or_default(),
                a.relay_quic_port,
                a.direct.iter().map(|d| d.to_string()).collect(),
            )
            .to_link();
        let expires = GoTime::from_system(SystemTime::now() + INVITE_TTL).format_rfc3339();
        let serial = match self.install(Side::Host(host.clone()), link, expires) {
            Ok(s) => s,
            Err(e) => {
                self.set_hosting(false);
                return Err(e);
            }
        };
        let rt = self.s.pairing.handle.clone();
        let (s, board) = (Arc::downgrade(&self.s), board_id.to_string());
        let h = host.clone();
        let handler: InviteHandler = Arc::new(move |c: Arc<dyn Conn>| {
            let (h, s, board) = (h.clone(), s.clone(), board.clone());
            rt.spawn(async move {
                let peer = c.id().to_string();
                if h.run(c).await.is_ok()
                    && let Some(s) = s.upgrade()
                {
                    s.invite_joined(&board, &peer, Vec::new()).await;
                }
            });
        });
        if let Err(e) = net.set_invite_handler(Some(host.room().clone()), Some(handler)) {
            host.cancel();
            return Err(e.0);
        }
        self.follow(serial, host.subscribe());
        // The hour is up: the invite ends if nobody finished it.
        {
            let (h, s) = (host.clone(), Arc::downgrade(&self.s));
            self.s.pairing.handle.spawn(async move {
                tokio::time::sleep(INVITE_TTL).await;
                if s.upgrade().is_some_and(|s| s.invites.is_current(serial)) {
                    h.expire();
                }
            });
        }
        Ok(inv.status())
    }

    /// Joins a board from an invite link: reads it, dials the inviter and
    /// shows the words to read out.
    pub async fn join_invite(&self, link: &str) -> Result<InviteStatus, String> {
        let inv = &self.s.invites;
        inv.free()?;
        let (ident, boards, link_s, timing) = (
            self.s.ident.clone(),
            self.invite_boards(),
            link.to_string(),
            inv.timing(),
        );
        // Deriving the room is scrypt: slow on purpose.
        let guest = tokio::task::spawn_blocking(move || Guest::new(ident, boards, &link_s, timing))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.0)?;
        let guest = Arc::new(guest);
        let to = invite_addr(guest.ticket())?;
        let serial = self.install(Side::Guest(guest.clone()), String::new(), String::new())?;
        self.follow(serial, guest.subscribe());
        let (net, s, g) = (
            self.s.mesh.net.clone(),
            Arc::downgrade(&self.s),
            guest.clone(),
        );
        self.s.pairing.handle.spawn(async move {
            let c = match net.dial_invite(&to, g.room()).await {
                Ok(c) => c,
                Err(e) => {
                    crate::log(&format!("invite: dial: {}", e.0));
                    g.fail("Couldn't reach the person who invited you. Check that their Irori is open, or ask for a new link.");
                    return;
                }
            };
            let peer = c.id().to_string();
            if g.run(c).await.is_ok()
                && let Some(s) = s.upgrade()
            {
                s.invite_joined(&g.ticket().board_id, &peer, to.direct.clone())
                    .await;
            }
        });
        Ok(inv.status())
    }

    /// The inviter typing the invitee's five words. A miss counts; the
    /// second ends the invite.
    pub fn confirm_invite_words(&self, typed: &str) -> Result<InviteStatus, String> {
        let host = match lock(&self.s.invites.cur).as_ref().map(|c| &c.side) {
            Some(Side::Host(h)) => h.clone(),
            _ => return Err("No invite is waiting for words.".into()),
        };
        match host.confirm(typed) {
            Ok(true) => {}
            Ok(false) => {
                return Err("Those aren't the words the other person's screen shows.".into());
            }
            Err(e) => return Err(e.0),
        }
        Ok(self.invite_status())
    }

    /// The open invite, or the last one's end; "" state when none.
    pub fn invite_status(&self) -> InviteStatus {
        self.s.invites.status()
    }

    /// Ends the open invite (the person closed it).
    pub fn cancel_invite(&self) {
        let side = lock(&self.s.invites.cur).as_ref().map(|c| match &c.side {
            Side::Host(h) => Side::Host(h.clone()),
            Side::Guest(g) => Side::Guest(g.clone()),
        });
        match side {
            Some(Side::Host(h)) => {
                h.cancel();
                let _ = self.s.mesh.net.set_invite_handler(None, None);
            }
            Some(Side::Guest(g)) => g.fail("Canceled."),
            None => {}
        }
    }

    /// Calls `f` with every change of the open invite; `None` stops.
    pub fn watch_invites(&self, f: Option<InviteWatchFn>) {
        *lock(&self.s.invites.watch) = f;
    }

    /// Whether this device is the inviter of an open invite (the home
    /// server's `wanted`).
    fn set_hosting(&self, on: bool) {
        let was = self
            .s
            .invites
            .hosting
            .swap(on, std::sync::atomic::Ordering::SeqCst);
        if was != on
            && let Some(h) = &self.s.home
        {
            h.kick();
        }
    }

    fn install(&self, side: Side, link: String, expires_at: String) -> Result<u64, String> {
        let inv = &self.s.invites;
        let mut cur = lock(&inv.cur);
        if cur.as_ref().is_some_and(|c| !ended(&c.status().status)) {
            return Err(
                "An invite is already open on this device. Finish or cancel it first.".into(),
            );
        }
        let serial = {
            let mut n = lock(&inv.next);
            *n += 1;
            *n
        };
        *cur = Some(Current {
            serial,
            side,
            link,
            expires_at,
        });
        drop(cur);
        inv.tell(serial);
        Ok(serial)
    }

    /// Passes the flow's changes on to the watcher; once the invite ends,
    /// the inviter's endpoint stops taking invitees.
    fn follow(&self, serial: u64, mut rx: tokio::sync::watch::Receiver<Status>) {
        let (s, net) = (Arc::downgrade(&self.s), self.s.mesh.net.clone());
        self.s.pairing.handle.spawn(async move {
            loop {
                let end = ended(&rx.borrow_and_update());
                let Some(sv) = s.upgrade() else { return };
                sv.invites.tell(serial);
                if end {
                    let host = matches!(
                        lock(&sv.invites.cur).as_ref().map(|c| (&c.side, c.serial)),
                        Some((Side::Host(_), g)) if g == serial
                    );
                    if host {
                        let _ = net.set_invite_handler(None, None);
                        sv.invites
                            .hosting
                            .store(false, std::sync::atomic::Ordering::SeqCst);
                        if let Some(h) = &sv.home {
                            h.kick();
                        }
                    }
                    return;
                }
                drop(sv);
                if rx.changed().await.is_err() {
                    return;
                }
            }
        });
    }
}

impl Services {
    /// Writes the private board.joined row for a board joined by invite.
    fn board_joined(&self, b: &super::boards::BoardLog) {
        let m = b.verifier.membership();
        let Some(creator) = m.pins.get(&m.creator).cloned() else {
            crate::log(&format!("invite: board {}: no creator to pin", b.id));
            return;
        };
        let joined = super::boards_own::BoardJoinedRow {
            board_id: b.id.clone(),
            name: m.name,
            tag: m.tag,
            creator,
            at: (self.now)(),
        };
        if let Err(e) = self
            .store
            .append("irori", store::kinds::BOARD_JOINED, &joined)
        {
            crate::log(&format!("invite: board {}: board.joined: {e}", b.id));
        }
    }

    /// Both sides once the bundle is in: the board opens on the network,
    /// and the invitee dials the inviter at the link's addresses.
    async fn invite_joined(
        self: &Arc<Self>,
        board_id: &str,
        peer: &str,
        addrs: Vec<std::net::SocketAddr>,
    ) {
        let b = match self.board(board_id) {
            Ok(b) => b,
            Err(e) => {
                crate::log(&format!("invite: board {board_id}: {e}"));
                return;
            }
        };
        let parts = crate::BoardParts {
            board_id: board_id.into(),
            person_id: self.ident.current().id,
            log: b.store.clone(),
            verifier: b.verifier.clone(),
            secrets: b.secrets.clone(),
        };
        if !addrs.is_empty() {
            // The invitee: its other devices open the board from the
            // private log (§4.1 step 7), pinned to the creator, and this
            // device certifies them in the board now its rows are in.
            self.board_joined(&b);
            if let Err(e) = self.keep_current(&b) {
                crate::log(&format!("invite: board {board_id}: identity rows: {e}"));
            }
            self.mesh.board_hint(peer, addrs);
        }
        if let Err(e) = self.mesh.open_board(parts).await {
            crate::log(&format!(
                "invite: open board {board_id} on the network: {e}"
            ));
            return;
        }
        // A board already open re-checks its members (the new one counts).
        if let Some(m) = self.mesh.board(board_id) {
            m.recheck();
            m.kick();
        }
        self.boards.moved();
    }
}
