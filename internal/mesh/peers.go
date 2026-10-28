package mesh

import (
	"encoding/json"
	"os"
	"path/filepath"
)

// PeersFile is where a node keeps the epoch and person key each other
// device last said hello with (§4.2). It is device-local and holds no
// secret: after a restart the node knows at once which devices already have
// the current key, so it doesn't reopen an older epoch's topic for them,
// and which older topic a device that is still behind needs.
const PeersFile = "mesh-peers.json"

type peerEpoch struct {
	Epoch     int    `json:"epoch"`
	PersonPub string `json:"personPub,omitempty"`
}

func loadPeers(dir string) map[string]peerEpoch {
	if dir == "" {
		return nil
	}
	raw, err := os.ReadFile(filepath.Join(dir, PeersFile))
	if err != nil {
		return nil
	}
	var out map[string]peerEpoch
	if json.Unmarshal(raw, &out) != nil {
		return nil
	}
	return out
}

// savePeers writes every known device's epoch and key, best effort: a
// failed write only costs an older topic open a little longer after a
// restart.
func (n *Node) savePeers() {
	if n.o.DataDir == "" {
		return
	}
	n.mu.Lock()
	out := map[string]peerEpoch{}
	for id, k := range n.known {
		if k.epoch > 0 {
			out[id] = peerEpoch{Epoch: k.epoch, PersonPub: k.pub}
		}
	}
	n.mu.Unlock()
	raw, _ := json.MarshalIndent(out, "", "  ")
	n.peersMu.Lock()
	defer n.peersMu.Unlock()
	path := filepath.Join(n.o.DataDir, PeersFile)
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, raw, 0o600); err != nil {
		n.logf("mesh: save %s: %v", PeersFile, err)
		return
	}
	if err := os.Rename(tmp, path); err != nil {
		n.logf("mesh: save %s: %v", PeersFile, err)
	}
}
