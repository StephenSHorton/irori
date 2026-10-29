package core

import (
	"context"
	"encoding/json"
	"errors"
	"log"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/names"
	"github.com/StephenSHorton/irori/internal/presence"
)

// namesFile holds the friendly names this device gave its Claude sessions
// (card #21). It is device-local: only the device that hosts a session
// names it, and the name reaches other devices on the session's card.
const namesFile = "names.json"

// namesKeep is how long a name stays reserved for a session that isn't
// running, so a resumed session keeps its name.
const namesKeep = 30 * 24 * time.Hour

type nameEntry struct {
	Name string    `json:"name"`
	At   time.Time `json:"at"` // when it was given, or last seen in use
}

// namer gives sessions nobody named a friendly name (presence.Namer) and
// settles clashes with other devices' agents.
type namer struct {
	path string
	self string // this device's id
	now  func() time.Time
	// mesh is every live agent on every device (Core.MeshAgents), set once
	// the roster exists.
	mesh func() []presence.AgentCard

	mu sync.Mutex
	m  map[string]nameEntry // by session id
}

func newNamer(dataDir, self string) *namer {
	n := &namer{path: filepath.Join(dataDir, namesFile), self: self, now: time.Now, m: map[string]nameEntry{}}
	raw, err := os.ReadFile(n.path)
	if err == nil {
		if err := json.Unmarshal(raw, &n.m); err != nil {
			log.Printf("names: %s unreadable, starting over: %v", n.path, err)
			n.m = map[string]nameEntry{}
		}
	} else if !errors.Is(err, os.ErrNotExist) {
		log.Printf("names: %v", err)
	}
	return n
}

// Assigned is the name session id was given, or "".
func (n *namer) Assigned(id string) string {
	n.mu.Lock()
	defer n.mu.Unlock()
	return n.m[id].Name
}

// Assign gives session id a name if it has none: the word its id hashes to,
// or the next one no live agent on any device is called.
func (n *namer) Assign(id string) string {
	cards := n.cards()
	n.mu.Lock()
	defer n.mu.Unlock()
	if e, ok := n.m[id]; ok {
		return e.Name
	}
	name := n.pickLocked(id, cards)
	n.m[id] = nameEntry{Name: name, At: n.now()}
	n.saveLocked()
	return name
}

func (n *namer) cards() []presence.AgentCard {
	if n.mesh == nil {
		return nil
	}
	return n.mesh()
}

// pickLocked picks a name for id that no live agent but id has, and that no
// other session here holds. n.mu is held.
func (n *namer) pickLocked(id string, cards []presence.AgentCard) string {
	key := n.self + "/" + id
	taken := map[string]bool{}
	for _, c := range cards {
		if c.Key != key && c.Status != "gone" {
			taken[names.Fold(c.Name)] = true
		}
	}
	for sid, e := range n.m {
		if sid != id {
			taken[names.Fold(e.Name)] = true
		}
	}
	return names.Pick(id, func(s string) bool { return taken[s] })
}

// Settle gives a new name to each session of this device whose Irori name
// another live agent also has, when that agent keeps it: a name someone
// chose always keeps it; between two Irori names, the agent that started
// first keeps it, then the lower key. Every device reads the same cards, so
// they agree on who moves, and only the loser's host renames. It also
// forgets names of sessions not seen for namesKeep. It reports whether any
// name moved.
func (n *namer) Settle() bool {
	cards := n.cards()
	n.mu.Lock()
	defer n.mu.Unlock()
	now := n.now()
	moved, dirty := false, false
	local := map[string]bool{}
	for _, c := range cards {
		if c.DeviceID != n.self || c.Agent != "claude" || c.Status == "gone" {
			continue
		}
		local[c.SessionID] = true
		e, ok := n.m[c.SessionID]
		if !ok {
			continue
		}
		if now.Sub(e.At) > time.Hour { // in use: keep it reserved
			e.At = now
			n.m[c.SessionID] = e
			dirty = true
		}
		if c.NameSource != presence.SourceIrori || names.Fold(c.Name) != names.Fold(e.Name) {
			continue
		}
		for _, o := range cards {
			if o.Key == c.Key || o.Status == "gone" || names.Fold(o.Name) != names.Fold(c.Name) {
				continue
			}
			if !keeps(o, c) {
				continue
			}
			delete(n.m, c.SessionID)
			n.m[c.SessionID] = nameEntry{Name: n.pickLocked(c.SessionID, cards), At: now}
			moved, dirty = true, true
			break
		}
	}
	for sid, e := range n.m {
		if !local[sid] && now.Sub(e.At) > namesKeep {
			delete(n.m, sid)
			dirty = true
		}
	}
	if dirty {
		n.saveLocked()
	}
	return moved
}

// keeps reports whether agent a keeps a name it shares with b (an Irori
// name): a name someone chose wins, then the earlier start, then the lower
// key.
func keeps(a, b presence.AgentCard) bool {
	if (a.NameSource == presence.SourceIrori) != (b.NameSource == presence.SourceIrori) {
		return a.NameSource != presence.SourceIrori
	}
	if !a.StartedAt.Equal(b.StartedAt) {
		return a.StartedAt.Before(b.StartedAt)
	}
	return a.Key < b.Key
}

// saveLocked writes the file through a temp file. n.mu is held.
func (n *namer) saveLocked() {
	raw, err := json.MarshalIndent(n.m, "", "  ")
	if err != nil {
		return
	}
	tmp := n.path + ".tmp"
	if err := os.WriteFile(tmp, raw, 0o600); err != nil {
		log.Printf("names: %v", err)
		return
	}
	if err := os.Rename(tmp, n.path); err != nil {
		log.Printf("names: %v", err)
	}
}

// settleNames settles name clashes (namer.Settle) whenever this device's
// cards or another device's change, and has presence pick up a moved name.
func (c *Core) settleNames(ctx context.Context) {
	remote := c.roster.Subscribe(ctx)
	for {
		select {
		case <-ctx.Done():
			return
		case <-remote:
		case <-c.namesK:
		}
		if c.names.Settle() {
			c.presence.Rescan()
		}
	}
}

// kickNames asks settleNames to run (coalesced, never blocks).
func (c *Core) kickNames() {
	select {
	case c.namesK <- struct{}{}:
	default:
	}
}
