package core

import (
	"errors"
	"time"

	"github.com/StephenSHorton/irori/internal/home"
)

// The home server (v0.4 §7.1, decision 33) as the Devices dialog sees it.

// ErrNoHomeServer is the answer on a Core whose transport isn't go-room
// (tests with a FakeNet).
var ErrNoHomeServer = errors.New("this device can't run a home server")

// HomeServer is this device's home server status: on, reachable at home,
// from outside, or needing a port forward, and the one line when something
// is wrong.
func (c *Core) HomeServer() home.Status {
	if c.home == nil {
		return home.Status{}
	}
	return c.home.Status()
}

// SetHomeServer switches this device's home server on or off, with
// "Reachable from outside home" and an optional public address for when
// the router can't map ports. GUI only.
func (c *Core) SetHomeServer(on, outside bool, public string) error {
	if c.home == nil {
		return ErrNoHomeServer
	}
	return c.home.Set(on, outside, public)
}

// CanServe reports whether this device can run a home server (it runs on
// go-room, not a test transport).
func (c *Core) CanServe() bool { return c.home != nil }

// Role is this device's cert role ("desktop" or "node"; "" before it has
// a cert): Devices offers the home server first on the node.
func (c *Core) Role() string {
	if cert, ok := c.chain.Cert(c.device.ID); ok {
		return cert.Role
	}
	return ""
}

// RemoteServer is another device's home server as this device reaches it.
type RemoteServer struct {
	DeviceID string `json:"deviceId"`
	Up       bool   `json:"up"`
	// Unreachable is true once it has been down past a first connect's
	// grace, so Devices doesn't flash a warning at every start.
	Unreachable bool `json:"unreachable,omitempty"`
}

// reachGrace is how long a hub may take to connect before Devices says it
// can't be reached.
const reachGrace = 30 * time.Second

// RemoteServers lists the home servers on other devices this device dials,
// and whether each hub connection is up (Devices shows one line when one
// can't be reached).
func (c *Core) RemoteServers() []RemoteServer {
	if c.rt == nil {
		return nil
	}
	var out []RemoteServer
	for _, h := range c.rt.Hubs() {
		out = append(out, RemoteServer{DeviceID: h.ID, Up: h.Up, Unreachable: !h.Up && time.Since(h.Since) > reachGrace})
	}
	return out
}

// OtherServer reports whether another of the person's devices runs a home
// server (its node.addr row, or a pairing hint).
func (c *Core) OtherServer() bool { return len(c.servers.List()) > 0 }

// LANError says why the LAN carrier is off ("" when it runs).
func (c *Core) LANError() string {
	if c.rt == nil {
		return ""
	}
	return c.rt.LANError()
}

// serverAddr is the address a device away from home dials to join (shown
// under the pairing code, §3): this device's own server's, or the first
// other server reachable from outside.
func (c *Core) serverAddr() string {
	if c.home != nil {
		if st := c.home.Status(); st.Running && st.Address != "" {
			return st.Address
		}
	}
	for _, s := range c.servers.List() {
		if row, ok := c.servers.Row(s.ID); ok {
			if a := row.Outside(); a != "" {
				return a
			}
		}
	}
	return ""
}

// pairingMoved runs on every pairing change: the home server starts while
// a window is open, even before there is a second device (§4.5).
func (c *Core) pairingMoved() {
	if c.home != nil {
		c.home.Kick()
	}
}
