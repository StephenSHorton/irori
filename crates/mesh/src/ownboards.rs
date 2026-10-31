//! A shared board's secret to the person's own devices
//! (docs/design/boards2-transport.md §4): on the private mesh `sync`
//! channel, sealed under the mesh key, as `person.secret` is.
//!
//! - **Send.** On each certified device's hello, and whenever the Core
//!   says its boards moved ([`Node::boards_changed`]), every open board's
//!   newest secret goes as `board.secret {boardId, epoch, maker, from,
//!   sealed}`: `sealed` is [`seal_board_secret`] under the mesh key of the
//!   connection's epoch `from`, to the peer's device id. Once per
//!   connection per (epoch, maker).
//! - **Take.** Opened under the mesh key of `from` and added to the board's
//!   `BoardSecrets` (its lower-maker rule settles two rotations to one
//!   epoch). The frame can come before the private log's `board.joined`
//!   opens the board here: it waits (one per board, the newest epoch, at
//!   most [`MAX_PENDING_BOARDS`]) until the board opens.

use std::sync::Arc;

use identity::BoardSecrets;
use zeroize::Zeroizing;

use crate::boardmesh::{open_board_secret, seal_board_secret};
use crate::frames::{FRAME_BOARD_SECRET, Frame};
use crate::node::Node;
use crate::secret::mesh_secret;
use crate::session::Session;

/// Board secrets held for boards not open here yet.
pub const MAX_PENDING_BOARDS: usize = 64;

/// The shared boards open on this device (the Core's).
pub trait OwnBoards: Send + Sync {
    /// Each open board's id and secrets.
    fn open_boards(&self) -> Vec<(String, Arc<BoardSecrets>)>;
    /// One of this person's certified devices sent `maker`'s secret for
    /// `board_id`: before the board's rows certify `maker` here, that
    /// device's word is what makes it count. Called before the secret is
    /// added.
    fn vouched(&self, _board_id: &str, _maker: &str) {}
}

/// A `board.secret` opened before its board.
pub struct PendingBoardSecret {
    epoch: i64,
    maker: String,
    secret: Zeroizing<Vec<u8>>,
}

impl Node {
    /// Sets the boards whose secrets go to the person's other devices.
    pub fn set_own_boards(&self, b: Option<Arc<dyn OwnBoards>>) {
        *self.own_boards.lock().unwrap_or_else(|e| e.into_inner()) = b;
        self.boards_changed();
    }

    fn vouch(&self, board_id: &str, maker: &str) {
        let b = self
            .own_boards
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(b) = b {
            b.vouched(board_id, maker);
        }
    }

    fn open_boards(&self) -> Vec<(String, Arc<BoardSecrets>)> {
        let b = self
            .own_boards
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        b.map(|b| b.open_boards()).unwrap_or_default()
    }

    /// A board opened, or a secret or membership changed: secrets waiting
    /// for a board that is open now are taken, and every connection gets
    /// the newest secrets.
    pub fn boards_changed(&self) {
        let open = self.open_boards();
        {
            let mut p = self.board_pending.lock().unwrap_or_else(|e| e.into_inner());
            for (id, secrets) in &open {
                if let Some(w) = p.remove(id) {
                    self.vouch(id, &w.maker);
                    if let Err(e) = secrets.add(w.epoch, &w.maker, &w.secret) {
                        self.log(&format!("mesh: board.secret for {id}: {e}"));
                    }
                }
            }
        }
        for s in self.session_list() {
            if s.hello_done() {
                self.send_board_secrets_of(&s, &open);
            }
        }
    }

    pub(crate) fn send_board_secrets(&self, s: &Session) {
        let open = self.open_boards();
        self.send_board_secrets_of(s, &open);
    }

    fn send_board_secrets_of(&self, s: &Session, open: &[(String, Arc<BoardSecrets>)]) {
        let peer = s.conn.id();
        if open.is_empty() || !self.certified(&peer) {
            return;
        }
        let cur = self.o.ident.current();
        let Some(seed) = self.o.ident.person_seed(s.epoch) else {
            return;
        };
        let key = mesh_secret(&seed[..], &cur.id, s.epoch).key;
        for (board, secrets) in open {
            let epoch = secrets.epoch();
            let Some((secret, maker)) = secrets.secret(epoch) else {
                continue;
            };
            if !s.mark_board_secret(board, &format!("{epoch}/{maker}")) {
                continue;
            }
            match seal_board_secret(&key, board, epoch, &secret, &peer) {
                Ok(sealed) => s.control(Frame {
                    board_id: board.clone(),
                    epoch,
                    maker,
                    from: s.epoch,
                    sealed,
                    ..Frame::of_type(FRAME_BOARD_SECRET)
                }),
                Err(e) => self.log(&format!("mesh: seal board secret: {e}")),
            }
        }
    }

    /// Opens a `board.secret` frame from a certified device of the person.
    pub(crate) fn take_board_secret(&self, f: &Frame) {
        if f.board_id.is_empty() || f.maker.is_empty() || f.epoch < 1 {
            return;
        }
        let cur = self.o.ident.current();
        let Some(seed) = self.o.ident.person_seed(f.from) else {
            return;
        };
        let key = mesh_secret(&seed[..], &cur.id, f.from).key;
        let secret = match open_board_secret(&key, &f.board_id, f.epoch, &f.sealed, &self.self_id) {
            Ok(s) => Zeroizing::new(s),
            Err(e) => {
                self.log(&format!("mesh: board.secret for {}: {e}", f.board_id));
                return;
            }
        };
        let open = self.open_boards();
        if let Some((_, secrets)) = open.iter().find(|(id, _)| id == &f.board_id) {
            self.vouch(&f.board_id, &f.maker);
            if let Err(e) = secrets.add(f.epoch, &f.maker, &secret) {
                self.log(&format!("mesh: board.secret for {}: {e}", f.board_id));
            }
            return;
        }
        let mut p = self.board_pending.lock().unwrap_or_else(|e| e.into_inner());
        if p.get(&f.board_id).is_some_and(|w| w.epoch > f.epoch) {
            return;
        }
        if !p.contains_key(&f.board_id) && p.len() >= MAX_PENDING_BOARDS {
            return;
        }
        p.insert(
            f.board_id.clone(),
            PendingBoardSecret {
                epoch: f.epoch,
                maker: f.maker.clone(),
                secret,
            },
        );
    }
}
