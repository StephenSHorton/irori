// Package roster is v0.4's one roster across devices (design/v0.4-devices.md
// §6.2): this device's live presence plus every other device's agents,
// folded from their agent.* rows. Inbox routing, the bypass sender check and
// the GUI read it (Core.MeshAgents); attach, Revive and presence recording
// keep reading this device's presence alone (Core.LocalAgents).
//
// A remote card lives until its device writes agent.gone for it, or a newer
// device.started from that device: a host that crashed never wrote gone, and
// its restart retires everything it showed before. An offline device's cards
// stay (with DeviceState "offline"): offline is not gone (§6.2).
package roster

import (
	"context"
	"encoding/json"
	"fmt"
	"sort"
	"sync"

	"github.com/StephenSHorton/irori/internal/owner"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

// Device liveness states (§6.2), as AgentCard.DeviceState shows them on
// another device's cards.
const (
	Connected = "connected"
	Recent    = "recent"
	Offline   = "offline"
)

var kinds = []string{store.KindDeviceStarted, store.KindAgentSeen, store.KindAgentStatus, store.KindAgentGone}

// Options configure a Roster.
type Options struct {
	Store *store.Store
	// Self is this device: its rows are presence's, not folded here.
	Self string
	// Local is this device's live roster (presence plus Grok members).
	Local func() []presence.AgentCard
	// Liveness is another device's state (mesh.Node.Liveness); nil counts
	// every other device offline.
	Liveness func(device string) string
	// Revoked reports a removed device: its cards are retired like a
	// restart's (§3, Revoke). Nil revokes nobody.
	Revoked func(device string) bool
}

// Roster is the mesh roster. Safe for concurrent use.
type Roster struct {
	o Options

	mu        sync.Mutex
	cards     map[string]presence.AgentCard // other devices' live cards, by key
	last      map[string]int64              // device -> highest seq folded
	ignored   int
	ignoredBy map[string]int
	subs      map[chan struct{}]struct{}
}

// Start replays every other device's agent rows and keeps folding new ones
// until ctx ends.
func Start(ctx context.Context, o Options) (*Roster, error) {
	if o.Local == nil {
		o.Local = func() []presence.AgentCard { return nil }
	}
	r := &Roster{o: o, subs: map[chan struct{}]struct{}{}}
	sub := o.Store.Subscribe(ctx) // before the replay, so nothing falls between
	if err := r.rebuild(ctx); err != nil {
		return nil, err
	}
	go func() {
		for ev := range sub {
			if ev.Seq == 0 {
				if err := r.rebuild(ctx); err != nil && ctx.Err() == nil {
					continue
				}
				r.notify()
				continue
			}
			if !wanted(ev.Kind) || ev.DeviceID == o.Self {
				continue
			}
			r.mu.Lock()
			changed := r.apply(ev)
			r.mu.Unlock()
			if changed {
				r.notify()
			}
		}
	}()
	return r, nil
}

func wanted(kind string) bool {
	for _, k := range kinds {
		if k == kind {
			return true
		}
	}
	return false
}

// rebuild folds the log from the start: the resync signal, and Start.
func (r *Roster) rebuild(ctx context.Context) error {
	cards, last := map[string]presence.AgentCard{}, map[string]int64{}
	f := store.Filter{Kinds: kinds, Limit: store.MaxListLimit}
	r.mu.Lock()
	defer r.mu.Unlock()
	r.cards, r.last, r.ignored, r.ignoredBy = cards, last, 0, nil
	for {
		evs, err := r.o.Store.List(ctx, f)
		if err != nil {
			return fmt.Errorf("roster: replay: %w", err)
		}
		for _, ev := range evs {
			if ev.DeviceID != r.o.Self {
				r.apply(ev)
			}
		}
		if len(evs) < f.Limit {
			return nil
		}
		f.After = evs[len(evs)-1].Cursor()
	}
}

// apply folds one row of another device. Each device's rows arrive in its
// seq order (the store inserts them gapless), so a row at or below the last
// one folded is a repeat. r.mu is held.
func (r *Roster) apply(ev store.Event) bool {
	dev := ev.DeviceID
	if ev.Seq <= r.last[dev] {
		return false
	}
	r.last[dev] = ev.Seq
	switch ev.Kind {
	case store.KindDeviceStarted:
		changed := false
		for k, c := range r.cards {
			if c.DeviceID == dev {
				delete(r.cards, k)
				changed = true
			}
		}
		return changed
	case store.KindAgentSeen:
		var c presence.AgentCard
		if json.Unmarshal(ev.Payload, &c) != nil || c.Key == "" {
			return false
		}
		if owner.DeviceOf(c.Key) != dev {
			r.ignore(dev) // a card for another device's agent: only its host writes it
			return false
		}
		c.DeviceID = dev
		// Older rows carried these; they mean nothing on this machine (§2).
		c.LastText, c.Transcript, c.PID = "", "", 0
		if c.Status == "" || c.Status == "gone" {
			c.Status = "idle"
		}
		r.cards[c.Key] = c
		return true
	case store.KindAgentStatus:
		var p struct {
			Key    string `json:"key"`
			Status string `json:"status"`
			Name   string `json:"name"`
			// PermissionMode is set when the host learned the mode after
			// agent.seen (older rows have none: the mode stays as it was).
			PermissionMode string `json:"permissionMode"`
			NameSource     string `json:"nameSource"` // card #21; "" on older rows
		}
		if json.Unmarshal(ev.Payload, &p) != nil {
			return false
		}
		c, ok := r.cards[p.Key]
		if !ok || owner.DeviceOf(p.Key) != dev {
			return false
		}
		if p.Status == "gone" {
			delete(r.cards, p.Key)
			return true
		}
		c.Status, c.Name, c.NameSource = p.Status, p.Name, p.NameSource
		if p.PermissionMode != "" {
			c.PermissionMode = p.PermissionMode
		}
		r.cards[p.Key] = c
		return true
	case store.KindAgentGone:
		var p struct {
			Key string `json:"key"`
		}
		if json.Unmarshal(ev.Payload, &p) != nil || owner.DeviceOf(p.Key) != dev {
			return false
		}
		if _, ok := r.cards[p.Key]; ok {
			delete(r.cards, p.Key)
			return true
		}
	}
	return false
}

// Agents is the mesh roster: this device's live cards, then every other
// device's, by key, each with its device's liveness. A removed device's
// cards are left out.
func (r *Roster) Agents() []presence.AgentCard {
	return append(r.o.Local(), r.Remote()...)
}

// Remote is the other devices' cards alone, with liveness.
func (r *Roster) Remote() []presence.AgentCard {
	out := []presence.AgentCard{}
	r.mu.Lock()
	remote := make([]presence.AgentCard, 0, len(r.cards))
	for _, c := range r.cards {
		remote = append(remote, c)
	}
	r.mu.Unlock()
	sort.Slice(remote, func(i, j int) bool { return remote[i].Key < remote[j].Key })
	state := map[string]string{}
	for _, c := range remote {
		if r.o.Revoked != nil && r.o.Revoked(c.DeviceID) {
			continue
		}
		s, ok := state[c.DeviceID]
		if !ok {
			s = Offline
			if r.o.Liveness != nil {
				s = r.o.Liveness(c.DeviceID)
			}
			state[c.DeviceID] = s
		}
		c.DeviceState = s
		out = append(out, c)
	}
	return out
}

// Ignored counts agent rows written by a device other than the agent's host.
func (r *Roster) Ignored() int {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.ignored
}

// Subscribe signals (coalesced) whenever another device's cards change.
func (r *Roster) Subscribe(ctx context.Context) <-chan struct{} {
	ch := make(chan struct{}, 1)
	r.mu.Lock()
	r.subs[ch] = struct{}{}
	r.mu.Unlock()
	go func() {
		<-ctx.Done()
		r.mu.Lock()
		delete(r.subs, ch)
		r.mu.Unlock()
	}()
	return ch
}

func (r *Roster) notify() {
	r.mu.Lock()
	defer r.mu.Unlock()
	for ch := range r.subs {
		select {
		case ch <- struct{}{}:
		default:
		}
	}
}

// ignore counts one row of dev ignored by the writer rules (§6.0). r.mu is
// held.
func (r *Roster) ignore(dev string) {
	r.ignored++
	if r.ignoredBy == nil {
		r.ignoredBy = map[string]int{}
	}
	r.ignoredBy[dev]++
}

// IgnoredBy counts the ignored rows per writing device, for Devices' warning
// line (§8, §9).
func (r *Roster) IgnoredBy() map[string]int {
	r.mu.Lock()
	defer r.mu.Unlock()
	out := make(map[string]int, len(r.ignoredBy))
	for d, n := range r.ignoredBy {
		out[d] = n
	}
	return out
}
