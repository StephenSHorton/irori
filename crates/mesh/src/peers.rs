//! `mesh-peers.json` (mesh.md §2.5; I:internal/mesh/peers.go): the epoch
//! and person key each other device last said hello with. Device-local,
//! no secret in it: after a restart the node knows at once which devices
//! already have the current key, so it doesn't reopen an older epoch's
//! topic for them.

use std::collections::BTreeMap;
use std::path::Path;

use gocompat::json::{self, Out, Schema};
use serde::Serialize;

/// The file name in the data dir.
pub const PEERS_FILE: &str = "mesh-peers.json";

/// One device's entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PeerEpoch {
    pub epoch: i64,
    #[serde(rename = "personPub", skip_serializing_if = "String::is_empty")]
    pub person_pub: String,
}

/// Go `loadPeers`: an unreadable or missing file is empty.
pub fn load_peers(dir: Option<&Path>) -> BTreeMap<String, PeerEpoch> {
    let Some(dir) = dir else {
        return BTreeMap::new();
    };
    let Ok(raw) = std::fs::read(dir.join(PEERS_FILE)) else {
        return BTreeMap::new();
    };
    let schema = Schema::Map(Box::new(Schema::object(vec![
        ("epoch", Schema::Int(64)),
        ("personPub", Schema::String),
    ])));
    let Ok(Out::Map(Some(m))) = json::unmarshal(&raw, &schema) else {
        return BTreeMap::new();
    };
    m.into_iter()
        .map(|(id, v)| {
            let epoch = match v.get("epoch") {
                Some(Out::Int(n)) => *n,
                _ => 0,
            };
            let person_pub = v.get_str("personPub").to_string();
            (id, PeerEpoch { epoch, person_pub })
        })
        .collect()
}

/// The bytes Go writes: `MarshalIndent(out, "", "  ")`, keys sorted.
pub fn peers_file_bytes(peers: &BTreeMap<String, PeerEpoch>) -> Vec<u8> {
    json::marshal_indent(peers, "", "  ").expect("peers always marshal")
}

/// Go `savePeers`' write: best effort, the caller logs a failure.
pub fn save_peers(dir: &Path, peers: &BTreeMap<String, PeerEpoch>) -> std::io::Result<()> {
    crate::fsutil::write_replace(&dir.join(PEERS_FILE), &peers_file_bytes(peers))
}
