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
		me.Warning = "Removed from your devices. Reset this device to pair it again."
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
		case p.Revoked:
			r.Warning = "Removed."
		case p.Stopped != "":
			r.Warning = "It " + p.Stopped + ". Remove it."
		case p.Old:
			r.Warning = "It runs an older Irori. Update it."
		case p.Ahead:
			r.Warning = "Its clock is ahead: its rows are waiting."
		case p.Refused > 0:
			r.Warning = fmt.Sprintf("%d of its rows were refused.", p.Refused)
		}
		if detail {
			d := p
			r.Detail = &d
		}
		out = append(out, r)
	}
	return out
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
