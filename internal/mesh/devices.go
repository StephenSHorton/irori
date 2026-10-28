package mesh

import (
	"context"
	"encoding/json"
	"fmt"
	"time"

	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/store"
)

// DeviceRow is one of the person's devices as Settings → Devices and GET
// /v1/devices show it (§6.8, §9): this device first, then every other
// certified one, each with one state and at most one warning line.
type DeviceRow struct {
	DeviceID string `json:"deviceId"`
	Name     string `json:"name"`
	Role     string `json:"role"`
	OS       string `json:"os,omitempty"`
	This     bool   `json:"this,omitempty"`
	// State is connected, recent or offline (this device: "this").
	State   string `json:"state"`
	Revoked bool   `json:"revoked,omitempty"`
	// Warning is the one line shown under a device when something is wrong.
	Warning string `json:"warning,omitempty"`
	// Action is what the warning asks for, when this dialog can do it:
	// "reset" (this device) or "remove" (another one).
	Action string `json:"action,omitempty"`
	// LastSeen is the newer of its last ping and its last sync here (zero
	// for this device, and for one never heard from).
	LastSeen time.Time `json:"lastSeen,omitzero"`
	// Detail is what is behind the state and the warning (?detail=1 only).
	Detail *PeerState `json:"detail,omitempty"`
}

// Devices lists the person's devices, this one first. detail fills
// DeviceRow.Detail for the other devices.
func (n *Node) Devices(detail bool) []DeviceRow {
	st := n.Status()
	self := n.o.Ident.Device
	me := DeviceRow{DeviceID: self.ID, Name: self.Name, Role: identity.RoleDesktop, OS: self.OS, This: true, State: "this"}
	if cert, ok := n.o.Chain.Cert(self.ID); ok && cert.Role != "" {
		me.Role = cert.Role
	}
	switch {
	case st.State == StateRemoved:
		me.Warning, me.Action = "Removed from your devices. Reset this device to pair it again.", "reset"
		if by := n.revokedBy(self.ID); by != "" {
			me.Warning = "Removed from your devices by " + by + ". Reset this device to pair it again."
		}
	case n.isStopped(self.ID):
		me.Warning = "This device signed two different histories (restored from a copy?). Reset it."
		me.Action = "reset"
	case st.Restoring != "":
		me.Warning = "Restoring this device's rows from another device."
	case st.State == StateError && st.Detail != "":
		me.Warning = st.Detail
	}
	out := []DeviceRow{me}
	for _, p := range st.Peers {
		r := DeviceRow{DeviceID: p.DeviceID, Name: p.Label, Role: p.Role, OS: n.deviceOS(p.DeviceID), State: p.State, Revoked: p.Revoked,
			LastSeen: p.LastPing}
		if p.LastSync.After(r.LastSeen) {
			r.LastSeen = p.LastSync
		}
		switch {
		case p.Revoked && p.Pending < p.Remaining:
			r.Warning = fmt.Sprintf("Removal pending: seen by %d of %d devices.", p.Pending, p.Remaining)
		case p.Revoked:
			r.Warning = "Removed."
		case p.Stopped != "":
			r.Warning = "It " + p.Stopped + ". Remove it."
			r.Action = "remove"
		case p.Old:
			r.Warning = "It runs an older Irori. Update it."
		case p.Ahead:
			r.Warning = "Its clock is ahead: its rows are waiting."
		case p.Refused > 0:
			r.Warning = fmt.Sprintf("%d of its rows were refused.", p.Refused)
		case p.Ignored > 0:
			r.Warning = fmt.Sprintf("%d of its rows were ignored: written by the wrong device. Remove it if you don't know why.", p.Ignored)
			r.Action = "remove"
		}
		if detail {
			d := p
			r.Detail = &d
		}
		out = append(out, r)
	}
	return out
}

// noteVector keeps peer's newest vector, for "Removal pending".
func (n *Node) noteVector(peer string, v store.Vector) {
	n.mu.Lock()
	defer n.mu.Unlock()
	n.info(peer).vec = copyVec(v)
}

// removalSeen counts the remaining devices (certified, unrevoked, this one
// included) whose last vector holds a revoke of dev, of how many there are
// (§3: "Removal pending: seen by 1 of 3 devices"). A device never heard
// from since counts as not having seen it.
func (n *Node) removalSeen(dev string, known map[string]peerInfo) (seen, of int) {
	ctx := context.Background()
	var revokes []store.Event
	evs, _ := n.o.Store.List(ctx, store.Filter{Kinds: []string{store.KindPersonDeviceRevoked}, Limit: store.MaxListLimit})
	for _, ev := range evs {
		var r identity.DeviceRevoked
		if json.Unmarshal(ev.Payload, &r) == nil && r.Revoke.DeviceID == dev {
			revokes = append(revokes, ev)
		}
	}
	for _, d := range n.o.Chain.Devices() {
		id := d.Cert.DeviceID
		if d.Revoked {
			continue
		}
		of++
		if id == n.self {
			seen++
			continue
		}
		for _, ev := range revokes {
			if ev.DeviceID == id || known[id].vec[ev.DeviceID] >= ev.Seq {
				seen++
				break
			}
		}
	}
	return seen, of
}

// revokedBy names the device that wrote the first held revoke of dev (its
// label, else its id), or "".
func (n *Node) revokedBy(dev string) string {
	ctx := context.Background()
	evs, _ := n.o.Store.List(ctx, store.Filter{Kinds: []string{store.KindPersonDeviceRevoked}, Limit: store.MaxListLimit})
	if ref, err := n.o.Store.Refused(ctx, store.MaxListLimit); err == nil {
		evs = append(evs, ref...)
	}
	for _, ev := range evs {
		if ev.Kind != store.KindPersonDeviceRevoked {
			continue
		}
		var r identity.DeviceRevoked
		if json.Unmarshal(ev.Payload, &r) != nil || r.Revoke.DeviceID != dev {
			continue
		}
		if cert, ok := n.o.Chain.Cert(ev.DeviceID); ok && cert.Label != "" {
			return cert.Label
		}
		return ev.DeviceID
	}
	return ""
}

// deviceOS is the os its newest device.started row names.
func (n *Node) deviceOS(dev string) string {
	evs, err := n.o.Store.List(context.Background(), store.Filter{Kinds: []string{store.KindDeviceStarted}, DeviceID: dev, Desc: true, Limit: 1})
	if err != nil || len(evs) == 0 {
		return ""
	}
	var p struct {
		OS string `json:"os"`
	}
	if json.Unmarshal(evs[0].Payload, &p) != nil || len(p.OS) > 20 {
		return ""
	}
	return p.OS
}

// Connected counts the other devices connected now (health's net.peers).
func (n *Node) Connected() int {
	c := 0
	for _, p := range n.Status().Peers {
		if p.State == "connected" {
			c++
		}
	}
	return c
}
