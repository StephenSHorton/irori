//! Board announcements on iroh-gossip (architecture §3.3, D13).
//!
//! One topic per board and epoch: `TopicId = sha256(board topic string)`,
//! the topic from `identity::derive_board_topic`, so only holders of the
//! board secret can compute it. Gossip carries announcements only,
//! `{"boardId","epoch","vector"}`: rows still move on the gated `board/<id>`
//! pull streams, and an announcement only tells the mesh a pull is worth
//! making (`KickBoards`). Gossip connections use ALPN `irori/gossip/1` and
//! pass the endpoint's `Admit` gate like the mesh.
//!
//! Stage 2 (card #43): the gate is `MemberAdmit`, so a gossip connection
//! comes only from this person's devices or a device counted on a board
//! held here ([`crate::boardadmit::HeldBoards`]). On top of that, a board's
//! channel can drop announcements whose delivering neighbour is not a
//! member of *that* board ([`IrohNet::board_channel_gated`]), and
//! [`BoardChannel::kicks`] turns an announcement that is ahead of the local
//! log into a pull (`BoardMesh::kick`).

use std::collections::BTreeMap;
use std::sync::Arc;

use iroh_gossip::api::{Event, GossipSender};
use iroh_gossip::proto::TopicId;
use irori_identity::BoardTopic;
use irori_meshseam::TransportError;
use n0_future::StreamExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::transport::IrohNet;

/// Received announcements buffered per board (Go's sub buffer, drop when full).
const ANNOUNCE_BUF: usize = 64;

/// One announcement: a member's vector of the board log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Announcement {
    #[serde(rename = "boardId")]
    pub board_id: String,
    pub epoch: i64,
    pub vector: BTreeMap<String, i64>,
}

/// Which neighbours' deliveries a board channel keeps (see
/// [`crate::boardadmit::HeldBoards::gossip_accept`]).
pub type GossipAccept = Arc<dyn Fn(&iroh::EndpointId) -> bool + Send + Sync>;
/// This device's vector of the board log, read when an announcement comes.
pub type HaveFn = Arc<dyn Fn() -> BTreeMap<String, i64> + Send + Sync>;
/// Called with each announcement that is ahead of the local log: pull now.
pub type KickFn = Arc<dyn Fn(&Announcement) + Send + Sync>;

/// Whether `theirs` holds a row `ours` lacks: some device's seq is higher.
pub fn ahead(theirs: &BTreeMap<String, i64>, ours: &BTreeMap<String, i64>) -> bool {
    theirs
        .iter()
        .any(|(dev, n)| *n > ours.get(dev).copied().unwrap_or(0))
}

/// The gossip topic of one board epoch.
pub fn topic_id(topic: &BoardTopic) -> TopicId {
    let h: [u8; 32] = Sha256::digest(topic.topic.as_bytes()).into();
    TopicId::from_bytes(h)
}

/// This device on one board's gossip topic.
pub struct BoardChannel {
    pub board_id: String,
    pub epoch: i64,
    sender: GossipSender,
    rx: Option<mpsc::Receiver<Announcement>>,
    task: tokio::task::JoinHandle<()>,
    kicks: Option<tokio::task::JoinHandle<()>>,
}

impl BoardChannel {
    /// Announce this device's vector of the board log.
    pub async fn announce(&self, vector: BTreeMap<String, i64>) -> Result<(), TransportError> {
        let a = Announcement {
            board_id: self.board_id.clone(),
            epoch: self.epoch,
            vector,
        };
        let b = serde_json::to_vec(&a).expect("announcement");
        self.sender
            .broadcast(b.into())
            .await
            .map_err(|e| TransportError(format!("gossip: {e}")))
    }

    /// The next announcement from another member (`None` once
    /// [`BoardChannel::kicks`] took them over).
    pub async fn next(&mut self) -> Option<Announcement> {
        self.rx.as_mut()?.recv().await
    }

    /// From now on every announcement whose vector is [`ahead`] of
    /// `have()` calls `kick` (the board mesh pulls); the others are
    /// dropped. Announcements stop coming through [`BoardChannel::next`].
    pub fn kicks(&mut self, have: HaveFn, kick: KickFn) {
        let Some(mut rx) = self.rx.take() else {
            return;
        };
        self.kicks = Some(tokio::spawn(async move {
            while let Some(a) = rx.recv().await {
                if ahead(&a.vector, &have()) {
                    kick(&a);
                }
            }
        }));
    }

    /// Add members to dial into the topic.
    pub async fn join(&self, net: &IrohNet, device_ids: &[String]) -> Result<(), TransportError> {
        let ids = net.board_endpoint_ids(&self.board_id, device_ids);
        self.join_ids(ids).await
    }

    /// Add members to dial into the topic, by key.
    pub async fn join_ids(&self, ids: Vec<iroh::EndpointId>) -> Result<(), TransportError> {
        if ids.is_empty() {
            return Ok(());
        }
        self.sender
            .join_peers(ids)
            .await
            .map_err(|e| TransportError(format!("gossip: {e}")))
    }
}

impl Drop for BoardChannel {
    fn drop(&mut self) {
        self.task.abort();
        if let Some(k) = &self.kicks {
            k.abort();
        }
    }
}

impl IrohNet {
    /// The EndpointIds the address book has for these devices.
    pub fn endpoint_ids(&self, device_ids: &[String]) -> Vec<iroh::EndpointId> {
        self.inner().endpoint_ids(device_ids)
    }

    /// The EndpointIds board `board_id`'s address book
    /// ([`IrohNet::set_board_peers`]) or else the private one has for
    /// these devices.
    pub fn board_endpoint_ids(
        &self,
        board_id: &str,
        device_ids: &[String],
    ) -> Vec<iroh::EndpointId> {
        let inner = self.inner();
        let board: Vec<iroh::EndpointId> = {
            let books = inner.board_book.lock().expect("board book");
            let book = books.get(board_id);
            device_ids
                .iter()
                .filter_map(|d| book.and_then(|b| b.get(d)).map(|p| p.id))
                .collect()
        };
        let mut ids = board;
        for id in inner.endpoint_ids(device_ids) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids
    }

    /// Join a board's gossip topic, bootstrapping from the given member
    /// devices (looked up in the board's address book, then the private one).
    pub async fn board_channel(
        &self,
        board_id: &str,
        topic: &BoardTopic,
        bootstrap: &[String],
    ) -> Result<BoardChannel, TransportError> {
        self.board_channel_gated(board_id, topic, bootstrap, None)
            .await
    }

    /// [`IrohNet::board_channel`], keeping only announcements delivered by a
    /// neighbour `accept` passes (a member device of this board; stage 2).
    pub async fn board_channel_gated(
        &self,
        board_id: &str,
        topic: &BoardTopic,
        bootstrap: &[String],
        accept: Option<GossipAccept>,
    ) -> Result<BoardChannel, TransportError> {
        self.bind().await?;
        let ids = self.board_endpoint_ids(board_id, bootstrap);
        self.board_channel_ids(board_id, topic, ids, accept).await
    }

    /// [`IrohNet::board_channel_gated`], bootstrapping from member keys.
    pub async fn board_channel_ids(
        &self,
        board_id: &str,
        topic: &BoardTopic,
        ids: Vec<iroh::EndpointId>,
        accept: Option<GossipAccept>,
    ) -> Result<BoardChannel, TransportError> {
        self.bind().await?;
        let g = self
            .inner()
            .gossip
            .get()
            .ok_or_else(|| TransportError("gossip: off".into()))?;
        let sub = g
            .subscribe(topic_id(topic), ids)
            .await
            .map_err(|e| TransportError(format!("gossip: {e}")))?;
        let (sender, mut receiver) = sub.split();
        let (tx, rx) = mpsc::channel(ANNOUNCE_BUF);
        let (want_board, want_epoch) = (board_id.to_string(), topic.epoch);
        let task = tokio::spawn(async move {
            while let Some(ev) = receiver.next().await {
                let Ok(Event::Received(m)) = ev else {
                    continue;
                };
                if let Some(ok) = &accept
                    && !ok(&m.delivered_from)
                {
                    continue;
                }
                let Ok(a) = serde_json::from_slice::<Announcement>(&m.content) else {
                    continue;
                };
                if a.board_id != want_board || a.epoch != want_epoch {
                    continue;
                }
                let _ = tx.try_send(a);
            }
        });
        Ok(BoardChannel {
            board_id: board_id.to_string(),
            epoch: topic.epoch,
            sender,
            rx: Some(rx),
            task,
            kicks: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ahead_means_a_row_we_lack() {
        let v = |xs: &[(&str, i64)]| -> BTreeMap<String, i64> {
            xs.iter().map(|(k, n)| (k.to_string(), *n)).collect()
        };
        assert!(ahead(&v(&[("a", 2)]), &v(&[("a", 1)])));
        assert!(ahead(&v(&[("b", 1)]), &v(&[("a", 5)])));
        assert!(!ahead(&v(&[("a", 1)]), &v(&[("a", 1), ("b", 3)])));
        assert!(!ahead(&v(&[]), &v(&[])));
        assert!(!ahead(&v(&[("a", 0)]), &v(&[])));
    }
}
