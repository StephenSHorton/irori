// Package roomnet is mesh's Transport on go-room (design/v0.4-devices.md §4,
// decision 31): sealed signaling on the public MQTT brokers, pion data
// channels between devices, STUN and the optional TURN relay. It is the only
// package in Irori that imports go-room.
package roomnet

import (
	"context"
	"crypto/ed25519"
	"errors"
	"io"
	"sync"
	"time"

	room "github.com/StephenSHorton/go-room"

	"github.com/StephenSHorton/irori/internal/mesh"
)

// Options configure a Transport.
type Options struct {
	// Self is this device's id (the relay's credential requests name it).
	Self string
	// Hub, when set, signals in process instead of on the public brokers
	// (tests).
	Hub *room.MemHub
	// Relay is the TURN setting; the zero value is "Direct only".
	Relay mesh.Relay
	// LoopbackOnly keeps every candidate on 127.0.0.1 and uses no STUN
	// (tests): nothing binds a LAN address or raises a firewall dialog.
	LoopbackOnly bool
	Logf         func(format string, args ...any)
}

// Transport opens go-room rooms. Nothing touches the network until the
// first Join: a single device never opens a broker connection or a socket.
type Transport struct {
	o Options

	mu     sync.Mutex
	relay  *room.Relay
	closed bool
}

var _ mesh.Transport = (*Transport)(nil)

// New returns a Transport.
func New(o Options) *Transport {
	if o.Logf == nil {
		o.Logf = func(string, ...any) {}
	}
	return &Transport{o: o}
}

// ice is the ICE server list for each new connection: none on loopback,
// STUN (go-room's default) when Direct only, and TURN too while the
// relay's credentials are good.
func (t *Transport) ice() func() []room.ICEServer {
	if t.o.LoopbackOnly {
		return func() []room.ICEServer { return nil }
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.o.Relay.Direct() {
		return nil
	}
	if t.relay == nil && !t.closed {
		t.relay = room.NewRelay(room.RelayConfig{CredentialURL: t.o.Relay.CredentialURL, TurnURLs: t.o.Relay.TurnURLs, Logf: t.o.Logf}, t.o.Self, nil)
		t.relay.Start()
	}
	if t.relay == nil {
		return nil
	}
	return t.relay.Servers
}

// RelayStatus reports the TURN relay: none (Direct only), pending, ok or
// error.
func (t *Transport) RelayStatus() mesh.RelayStatus {
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.relay == nil {
		if t.o.Relay.Direct() {
			return mesh.RelayStatus{State: room.RelayNone}
		}
		return mesh.RelayStatus{State: room.RelayPending}
	}
	st := t.relay.Status()
	return mesh.RelayStatus{State: st.State, Detail: st.Detail}
}

// Close stops the relay's credential loop. Rooms close on their own.
func (t *Transport) Close() {
	t.mu.Lock()
	r := t.relay
	t.closed, t.relay = true, nil
	t.mu.Unlock()
	if r != nil {
		r.Stop()
	}
}

// Join implements mesh.Transport: one Signaler and one Mesh on s's topic.
func (t *Transport) Join(s mesh.Secret, cfg mesh.RoomConfig) (mesh.Room, error) {
	r := &roomConn{ready: make(chan struct{})}
	so := room.SignalOptions{
		ClientPrefix: "irori-",
		MaxAge:       room.DefaultMaxAge,
		OnMsg: func(env room.Envelope) {
			<-r.ready
			if cfg.OnMsg != nil {
				cfg.OnMsg(mesh.Envelope{From: env.From, Type: env.Type, Body: env.Body})
			}
		},
		OnConnect: func() {
			<-r.ready
			if cfg.OnConnect != nil {
				cfg.OnConnect()
			}
		},
	}
	secret := room.Secret{Key: s.Key, Topic: s.Topic}
	var sig room.Signaler
	if t.o.Hub != nil {
		sig = t.o.Hub.Open(secret, cfg.Self, so)
	} else {
		var err error
		if sig, err = room.OpenSignal(secret, cfg.Self, so); err != nil {
			close(r.ready)
			return nil, err
		}
	}
	rc := room.Config{
		Self:       cfg.Self,
		Identity:   room.Identity{Pub: cfg.Pub, Sign: cfg.Sign},
		Accept:     cfg.Accept,
		Allow:      cfg.Allow,
		Offerer:    cfg.Offerer,
		ICE:        t.ice(),
		MaxMessage: mesh.MaxMessage,
		MaxStreams: 4,
		Logf:       t.o.Logf,
	}
	if t.o.LoopbackOnly {
		rc.Tune = room.LoopbackOnly()
	}
	m, err := room.NewMesh(sig, rc)
	if err != nil {
		sig.Close()
		close(r.ready)
		return nil, err
	}
	m.OnPeer(func(p *room.Peer) {
		if cfg.OnPeer != nil {
			cfg.OnPeer(&peerConn{p: p})
		}
	})
	r.sig, r.mesh = sig, m
	close(r.ready)
	return r, nil
}

type roomConn struct {
	ready chan struct{}
	sig   room.Signaler
	mesh  *room.Mesh
	once  sync.Once
}

func (r *roomConn) Send(to, typ string, body any) error { return r.sig.Send(to, typ, body) }
func (r *roomConn) Connect(peer string)                 { r.mesh.Connect(peer) }

// Close closes every connection (go-room sends rtc.bye and doesn't
// reconnect after a local close) and leaves the topic.
func (r *roomConn) Close() {
	r.once.Do(func() {
		r.mesh.Close()
		r.sig.Close()
	})
}

// peerConn is a go-room *Peer as a mesh.Conn.
type peerConn struct{ p *room.Peer }

func (c *peerConn) ID() string             { return c.p.ID() }
func (c *peerConn) Pub() ed25519.PublicKey { return c.p.Pub() }
func (c *peerConn) Send(ctx context.Context, msg []byte) error {
	return c.p.Messages().Send(ctx, msg)
}
func (c *peerConn) Recv(ctx context.Context) ([]byte, error) { return c.p.Messages().Recv(ctx) }
func (c *peerConn) OpenStream(ctx context.Context, label string) (io.ReadWriteCloser, error) {
	s, err := c.p.OpenStream(ctx, label)
	if err != nil {
		return nil, err
	}
	return s, nil
}
func (c *peerConn) OnStream(fn func(string, io.ReadWriteCloser)) {
	c.p.OnStream(func(label string, s *room.Stream) { fn(label, s) })
}
func (c *peerConn) Done() <-chan struct{}          { return c.p.Done() }
func (c *peerConn) Err() error                     { return c.p.Err() }
func (c *peerConn) Close(reason string)            { c.p.Close(reason) }
func (c *peerConn) Path() string                   { return c.p.Path() }
func (c *peerConn) RTT() time.Duration             { return c.p.RTT() }
func (c *peerConn) Fingerprints() (string, string) { return c.p.Fingerprints() }

// IsReplaced reports a connection that went down because a newer one to the
// same device came up in its place.
func IsReplaced(err error) bool { return errors.Is(err, room.ErrReplaced) }
