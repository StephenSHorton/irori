//! Board announcements on iroh-gossip (architecture §3.3, D13).
//!
//! One topic per board and epoch: `TopicId = sha256(board topic string)`,
//! the topic from `identity::derive_board_topic`, so only holders of the
//! board secret can compute it. Gossip carries announcements only,
//! `{"boardId","epoch","vector"}`: rows still move on the gated `board/<id>`
//! pull streams, and an announcement only tells the mesh a pull is worth
//! making (`KickBoards`). Gossip connections use ALPN `irori/gossip/1` and
//! pass the endpoint's `Admit` gate like the mesh.

use std::collections::BTreeMap;

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
    rx: mpsc::Receiver<Announcement>,
    task: tokio::task::JoinHandle<()>,
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

    /// The next announcement from another member.
    pub async fn next(&mut self) -> Option<Announcement> {
        self.rx.recv().await
    }

    /// Add members to dial into the topic.
    pub async fn join(&self, net: &IrohNet, device_ids: &[String]) -> Result<(), TransportError> {
        let ids = net.endpoint_ids(device_ids);
        self.sender
            .join_peers(ids)
            .await
            .map_err(|e| TransportError(format!("gossip: {e}")))
    }
}

impl Drop for BoardChannel {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl IrohNet {
    /// The EndpointIds the address book has for these devices.
    pub fn endpoint_ids(&self, device_ids: &[String]) -> Vec<iroh::EndpointId> {
        self.inner().endpoint_ids(device_ids)
    }

    /// Join a board's gossip topic, bootstrapping from the given member
    /// devices (looked up in the address book).
    pub async fn board_channel(
        &self,
        board_id: &str,
        topic: &BoardTopic,
        bootstrap: &[String],
    ) -> Result<BoardChannel, TransportError> {
        self.bind().await?;
        let g = self
            .inner()
            .gossip
            .get()
            .ok_or_else(|| TransportError("gossip: off".into()))?;
        let ids = self.endpoint_ids(bootstrap);
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
            rx,
            task,
        })
    }
}
