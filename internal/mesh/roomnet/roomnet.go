// Package roomnet is mesh's Transport on go-room (design/v0.4-devices.md §4,
// decisions 31 and 33): sealed signaling over the LAN carrier and the home
// server's hub, pion data channels between devices, and ICE through the
// home server's STUN and TURN (plus an advanced TURN relay, if one is set).
// No third party is involved: nothing goes through a public server, for
// signaling or for STUN. It and internal/home are the only packages in
// Irori that import go-room.
package roomnet

import (
	"context"
	"crypto/ed25519"
	"errors"
	"fmt"
	"io"
	"net"
	"net/netip"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	room "github.com/StephenSHorton/go-room"

	"github.com/StephenSHorton/irori/internal/mesh"
)

// Options configure a Transport.
type Options struct {
	// Self is this device's id; Pub and Sign are its key, which signs the
	// hub handshake (§4.2) as it signs every connection's (§4.3).
	Self string
	Pub  ed25519.PublicKey
	Sign func(msg []byte) []byte
	// Servers is the home server directory: the hubs to dial and where.
	// nil dials no hub (only the LAN, and this device's own server).
	Servers *mesh.Servers
	// Expect checks a hub's welcome: the server device's id and the key it
	// proved (Irori: a certified, unrevoked device whose cert names that
	// key). Required with Servers.
	Expect func(id string, pub ed25519.PublicKey) error
	// Relay is the advanced TURN relay; the zero value is none.
	Relay mesh.Relay
	// LANPort is the LAN carrier's UDP port (default mesh.LANPort).
	// LANConn and LANTargets replace the wildcard socket and the directed
	// broadcast targets (tests: a MemLAN conn, or a 127.0.0.1 socket).
	// NoLAN leaves the LAN carrier off.
	LANPort    int
	LANConn    net.PacketConn
	LANTargets func() []netip.AddrPort
	NoLAN      bool
	// Dial opens hub connections (default net.Dialer; tests: 127.0.0.1).
	Dial func(ctx context.Context, addr string) (net.Conn, error)
	// LoopbackOnly keeps every candidate on 127.0.0.1 (tests): nothing
	// binds a LAN address or raises a firewall dialog.
	LoopbackOnly bool
	// RelayOnly makes every connection go through a TURN relay (tests that
	// prove the away-from-home path).
	RelayOnly bool
	Logf      func(format string, args ...any)
}

// Transport opens go-room rooms. Nothing touches the network until the
// first Join: a single device binds no socket and dials no hub.
type Transport struct {
	o Options

	mu      sync.Mutex
	started bool
	closed  bool
	lan     *room.LAN
	lanErr  string
	relay   *room.Relay // the advanced relay
	hubs    map[string]*hubConn
	local   *local
	legs    map[*muxLeg]struct{}
	guests  map[string]guestWindow // pairing topics open to guests
	carMu   sync.Mutex             // serializes adding and removing carriers
	quit    chan struct{}          // closed by Close
}

// guestWindow is one pairing topic open to guests: until when, and the
// context every hub's guest.open waits under, which closeGuests cancels so
// a hub that comes up after the pairing closed doesn't open it again.
type guestWindow struct {
	until  time.Time
	ctx    context.Context
	cancel context.CancelFunc
}

// hubConn is one home server's hub this device is a member of, and the
// credential loop for its TURN relay.
type hubConn struct {
	id     string
	client *room.HubClient
	relay  *room.Relay
	since  time.Time // when dialing started
}

// local is this device's own home server (internal/home): its hub carries
// this device's rooms in process, and ice lists its own STUN and TURN.
type local struct {
	hub *room.Hub
	ice func() []room.ICEServer
}

const (
	localKey = "\x00local"
	lanKey   = "\x00lan"
)

// lanRetry is how often a LAN carrier that couldn't open (its port was
// taken) is tried again. A variable so tests can run fast.
var lanRetry = 30 * time.Second

var _ mesh.Transport = (*Transport)(nil)

// New returns a Transport.
func New(o Options) *Transport {
	if o.Logf == nil {
		o.Logf = func(string, ...any) {}
	}
	if o.LANPort == 0 {
		o.LANPort = mesh.LANPort
	}
	t := &Transport{o: o, hubs: map[string]*hubConn{}, legs: map[*muxLeg]struct{}{}, guests: map[string]guestWindow{},
		quit: make(chan struct{})}
	if o.Servers != nil {
		o.Servers.OnChange(t.refresh)
	}
	return t
}

// start opens the LAN carrier and dials the known hubs, once. Called on
// the first Join (or LAN), so a lone device opens nothing.
func (t *Transport) start() {
	t.mu.Lock()
	if t.started || t.closed {
		t.mu.Unlock()
		return
	}
	t.started = true
	t.mu.Unlock()
	if !t.o.NoLAN && !t.openLAN() {
		go t.retryLAN()
	}
	t.refresh()
}

// openLAN opens the LAN carrier and adds it to every room; false when it
// couldn't (the hub still works, and Devices says why the LAN is off).
func (t *Transport) openLAN() bool {
	lan, err := room.OpenLAN(room.LANConfig{Port: t.o.LANPort, Conn: t.o.LANConn, Targets: t.o.LANTargets, Logf: t.o.Logf})
	t.carMu.Lock()
	defer t.carMu.Unlock()
	t.mu.Lock()
	if err != nil {
		if t.lanErr != err.Error() {
			t.o.Logf("roomnet: the LAN carrier is off: %v", err)
		}
		t.lanErr = err.Error()
		t.mu.Unlock()
		return false
	}
	if t.closed {
		t.mu.Unlock()
		lan.Close()
		return true
	}
	t.lan, t.lanErr = lan, ""
	legs := t.legList()
	t.mu.Unlock()
	for _, l := range legs {
		l.add(lanKey, lan.Carrier(), "")
	}
	return true
}

// retryLAN tries the LAN carrier again until it opens or the transport
// closes: a port another program held a moment ago may be free now.
func (t *Transport) retryLAN() {
	for {
		select {
		case <-t.quit:
			return
		case <-time.After(lanRetry):
		}
		if t.openLAN() {
			return
		}
	}
}

// LAN is the LAN carrier (opening it if nothing has yet), for the home
// server's bridge of guest topics; nil when it is off.
func (t *Transport) LAN() *room.LAN {
	t.start()
	t.mu.Lock()
	defer t.mu.Unlock()
	return t.lan
}

// LANError says, in a sentence for Devices, why devices on this network
// can't find this one directly: the LAN carrier couldn't open, or it has
// nowhere to send (no private network, or Windows has every one as
// Public). "" while it works or before anything opened it.
func (t *Transport) LANError() string {
	t.mu.Lock()
	lan, lanErr, started := t.lan, t.lanErr, t.started
	t.mu.Unlock()
	switch {
	case lanErr != "":
		return fmt.Sprintf("Irori can't use UDP port %d on this device, so devices on this network can't find it directly. Another program may be using it; Irori tries again on its own.", t.o.LANPort)
	case started && lan != nil && len(lan.Targets()) == 0:
		return "Devices on this network can't find this one directly: Windows has this network as Public, or this device isn't on a home network. In Windows Settings, open Network & internet and set your home network to Private."
	}
	return ""
}

// SetLocal makes this device's own home server's hub a carrier of every
// room, now and later, with ice as its STUN and TURN entries; nil takes it
// out again.
func (t *Transport) SetLocal(hub *room.Hub, ice func() []room.ICEServer) {
	t.carMu.Lock()
	defer t.carMu.Unlock()
	t.mu.Lock()
	old := t.local
	if hub == nil {
		t.local = nil
	} else {
		t.local = &local{hub: hub, ice: ice}
	}
	legs := t.legList()
	guests := t.guestList()
	t.mu.Unlock()
	for _, l := range legs {
		l.remove(localKey)
		if hub != nil {
			l.add(localKey, hub.Local(), "")
		}
	}
	if hub != nil && (old == nil || old.hub != hub) {
		for topic, g := range guests {
			_ = hub.OpenGuest(topic, time.Until(g.until))
		}
	}
}

// refresh dials every home server the directory lists and drops the ones
// it no longer does (switched off, revoked).
func (t *Transport) refresh() {
	if t.o.Servers == nil {
		return
	}
	t.carMu.Lock()
	defer t.carMu.Unlock()
	t.mu.Lock()
	if !t.started || t.closed {
		t.mu.Unlock()
		return
	}
	t.mu.Unlock()
	want := map[string]bool{}
	for _, s := range t.o.Servers.List() {
		want[s.ID] = true
	}
	t.mu.Lock()
	var add []*hubConn
	for id := range want {
		if t.hubs[id] == nil {
			h := t.dialHub(id)
			t.hubs[id] = h
			add = append(add, h)
		}
	}
	var drop []*hubConn
	for id, h := range t.hubs {
		if !want[id] {
			drop = append(drop, h)
			delete(t.hubs, id)
		}
	}
	legs := t.legList()
	guests := t.guestList()
	t.mu.Unlock()
	for _, h := range drop {
		for _, l := range legs {
			l.remove(h.id)
		}
		h.close()
	}
	for _, h := range add {
		for _, l := range legs {
			l.add(h.id, h.client.Carrier(), h.id)
		}
		for topic, g := range guests {
			go openGuest(g.ctx, h.client, topic, g.until)
		}
	}
}

// dialHub starts a member connection to server id. t.mu is held.
func (t *Transport) dialHub(id string) *hubConn {
	ident := room.Identity{Pub: t.o.Pub, Sign: t.o.Sign}
	var relay *room.Relay
	var relayMu sync.Mutex
	c := room.DialHub(room.HubDialConfig{
		Self:     t.o.Self,
		Identity: &ident,
		Addrs:    func() []string { return t.o.Servers.Addrs(id) },
		Expect: func(got string, pub ed25519.PublicKey) error {
			if got != id {
				return fmt.Errorf("hub: %s answered for %s", got, id)
			}
			if t.o.Expect == nil {
				return mesh.ErrRefused
			}
			return t.o.Expect(got, pub)
		},
		Dial:     t.o.Dial,
		TURNURIs: func(addr string) []string { return t.turnURIs(id, addr) },
		// TURN credentials are fetched again as soon as the hub is back,
		// not after a backoff that grew while it was down.
		OnUp: func() {
			relayMu.Lock()
			r := relay
			relayMu.Unlock()
			if r != nil {
				r.Kick()
			}
		},
		Logf: t.o.Logf,
	})
	r := room.NewRelay(room.RelayConfig{Fetch: c.TURN, Logf: scrubbed(t.o.Logf)}, t.o.Self, nil)
	relayMu.Lock()
	relay = r
	relayMu.Unlock()
	r.Start()
	return &hubConn{id: id, client: c, relay: r, since: time.Now()}
}

func (h *hubConn) close() {
	h.relay.Stop()
	h.client.Close()
}

// turnURIs are server id's STUN and TURN URLs as reached at addr: the
// hub's port number serves both at home; away, the router may have mapped
// TCP and UDP to different external ports, which the server's row says.
func (t *Transport) turnURIs(id, addr string) []string {
	host, port, err := net.SplitHostPort(addr)
	if err != nil {
		return nil
	}
	udp := port
	if t.o.Servers != nil {
		if row, ok := t.o.Servers.Row(id); ok && row.Public != nil && row.Public.IP == host && row.Public.UDP > 0 {
			udp = strconv.Itoa(row.Public.UDP)
		}
	}
	hu := net.JoinHostPort(host, udp)
	return []string{"stun:" + hu, "turn:" + hu + "?transport=udp", "turn:" + addr + "?transport=tcp"}
}

// openGuest opens topic to guests on one hub, waiting for its connection
// until the window ends or ctx (the window's) is cancelled.
func openGuest(ctx context.Context, c *room.HubClient, topic string, until time.Time) {
	ttl := time.Until(until)
	if ttl <= 0 {
		return
	}
	ctx, cancel := context.WithTimeout(ctx, ttl)
	defer cancel()
	_ = c.OpenGuest(ctx, topic, ttl)
}

func (t *Transport) legList() []*muxLeg {
	out := make([]*muxLeg, 0, len(t.legs))
	for l := range t.legs {
		out = append(out, l)
	}
	return out
}

func (t *Transport) guestList() map[string]guestWindow {
	out := map[string]guestWindow{}
	for k, v := range t.guests {
		if time.Until(v.until) > 0 {
			out[k] = v
		}
	}
	return out
}

// HubState is one home server's hub as this device sees it, for Devices.
type HubState struct {
	ID   string `json:"id"`
	Up   bool   `json:"up"`
	Addr string `json:"addr,omitempty"` // the address the handshake completed on
	// Since is when this device started dialing it, so a hub that is
	// still connecting isn't reported as unreachable.
	Since time.Time `json:"since"`
}

// Hubs lists the home servers this device dials and whether each is up.
func (t *Transport) Hubs() []HubState {
	t.mu.Lock()
	defer t.mu.Unlock()
	out := make([]HubState, 0, len(t.hubs))
	for id, h := range t.hubs {
		out = append(out, HubState{ID: id, Up: h.client.Up(), Addr: h.client.Addr(), Since: h.since})
	}
	sort.Slice(out, func(i, j int) bool { return out[i].ID < out[j].ID })
	return out
}

// ice is the ICE list for each new connection (§4.4), read when the
// connection is made, so a server or relay that comes up later applies to
// the next one: this device's own server's STUN and TURN, each connected
// hub's, a guest room's, and the advanced relay's TURN. Never a public STUN
// server: with none of these, host candidates are all there is.
func (t *Transport) ice(guest *hubConn) []room.ICEServer {
	t.mu.Lock()
	var out []room.ICEServer
	if t.local != nil && t.local.ice != nil {
		out = append(out, t.local.ice()...)
	}
	hubs := make([]*hubConn, 0, len(t.hubs)+1)
	for _, h := range t.hubs {
		hubs = append(hubs, h)
	}
	if guest != nil {
		hubs = append(hubs, guest)
	}
	relay := t.relay
	if relay == nil && !t.o.Relay.Direct() && !t.closed {
		t.relay = room.NewRelay(room.RelayConfig{CredentialURL: t.o.Relay.CredentialURL, TurnURLs: t.o.Relay.TurnURLs, Logf: scrubbed(t.o.Logf)}, t.o.Self, nil)
		t.relay.Start()
		relay = t.relay
	}
	t.mu.Unlock()
	for _, h := range hubs {
		addr := h.client.Addr()
		if addr == "" {
			continue
		}
		out = append(out, t.hubICE(h, addr)...)
	}
	if relay != nil {
		out = append(out, relay.TURN()...)
	}
	return out
}

// hubICE is a connected hub's STUN and TURN entries at the address its
// connection went through now: credentials outlive an address change (a
// new DHCP lease, coming home), so the URLs are rebuilt from addr.
func (t *Transport) hubICE(h *hubConn, addr string) []room.ICEServer {
	var user, cred string
	for _, s := range h.relay.TURN() {
		if s.Username != "" {
			user, cred = s.Username, s.Credential
			break
		}
	}
	uris := t.turnURIs(h.id, addr)
	if len(uris) == 0 {
		return nil
	}
	out := []room.ICEServer{{URLs: uris[:1]}}
	if user != "" {
		out = append(out, room.ICEServer{URLs: uris[1:], Username: user, Credential: cred})
	}
	return out
}

// SetRelay replaces the advanced relay setting (Settings → Devices →
// Advanced, §4.4). The old credential loop stops; connections made from
// now on use the new setting, and ones already up keep their path.
func (t *Transport) SetRelay(r mesh.Relay) {
	t.mu.Lock()
	old := t.relay
	t.o.Relay, t.relay = r, nil
	t.mu.Unlock()
	if old != nil {
		old.Stop()
	}
}

// RelayStatus reports the advanced TURN relay: none, pending, ok or error.
// Detail never carries a credential URL's query (§8: nothing logs a TURN
// credential).
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
	return mesh.RelayStatus{State: st.State, Detail: mesh.ScrubURLs(st.Detail)}
}

// scrubbed wraps logf so a credential URL's query never reaches the log: a
// failed fetch's *url.Error prints the whole URL.
func scrubbed(logf func(string, ...any)) func(string, ...any) {
	return func(format string, args ...any) {
		logf("%s", mesh.ScrubURLs(fmt.Sprintf(format, args...)))
	}
}

// Close stops every hub connection, credential loop and the LAN carrier.
// Rooms still open lose their carriers.
func (t *Transport) Close() {
	t.carMu.Lock()
	t.mu.Lock()
	if !t.closed {
		close(t.quit)
	}
	r := t.relay
	t.closed, t.relay = true, nil
	hubs := t.hubs
	t.hubs = map[string]*hubConn{}
	lan := t.lan
	t.lan = nil
	t.local = nil
	t.mu.Unlock()
	t.carMu.Unlock()
	if r != nil {
		r.Stop()
	}
	for _, h := range hubs {
		h.close()
	}
	if lan != nil {
		lan.Close()
	}
}

// Join implements mesh.Transport: one Signaler on s's topic over the LAN
// carrier and every hub (this device's own server's in process), and one
// Mesh on it.
func (t *Transport) Join(s mesh.Secret, cfg mesh.RoomConfig) (mesh.Room, error) {
	t.start()
	t.mu.Lock()
	if t.closed {
		t.mu.Unlock()
		return nil, room.ErrClosed
	}
	t.mu.Unlock()
	r := &roomConn{t: t, topic: s.Topic, ready: make(chan struct{})}
	so := room.SignalOptions{
		MaxAge: room.DefaultMaxAge,
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
		OnHeard: func(from string, addr netip.AddrPort) {
			// How a device learns its home server's LAN address (§4.2).
			if t.o.Servers != nil {
				t.o.Servers.Heard(from, addr.Addr())
			}
		},
	}
	// The LAN carrier and every hub ride in one mux, so carriers that come
	// later (a hub's row arrives, the LAN port frees up) join this room.
	carriers := []room.Carrier{muxCarrier{t}}
	var guest *hubConn
	if cfg.HubAddr != "" {
		// The joining device away from home: the typed address's hub admits
		// it as a guest of this pairing room only (§3).
		addr := cfg.HubAddr
		var gr *room.Relay
		var grMu sync.Mutex
		c := room.DialHub(room.HubDialConfig{
			Self:       t.o.Self,
			GuestTopic: s.Topic,
			Addrs:      func() []string { return []string{addr} },
			Dial:       t.o.Dial,
			OnUp: func() {
				grMu.Lock()
				r := gr
				grMu.Unlock()
				if r != nil {
					r.Kick()
				}
			},
			Logf: t.o.Logf,
		})
		grMu.Lock()
		gr = room.NewRelay(room.RelayConfig{Fetch: c.TURN, Logf: scrubbed(t.o.Logf)}, t.o.Self, nil)
		grMu.Unlock()
		gr.Start()
		guest = &hubConn{client: c, relay: gr}
		r.guest = guest
		carriers = append(carriers, c.Carrier())
	}
	if cfg.GuestTTL > 0 {
		r.guestTopic = true
		t.openGuests(s.Topic, cfg.GuestTTL)
	}
	secret := room.Secret{Key: s.Key, Topic: s.Topic}
	sig, err := room.Open(secret, cfg.Self, so, carriers...)
	if err != nil {
		close(r.ready)
		r.closeGuest()
		return nil, err
	}
	rc := room.Config{
		Self:       cfg.Self,
		Identity:   room.Identity{Pub: cfg.Pub, Sign: cfg.Sign},
		Accept:     cfg.Accept,
		Allow:      cfg.Allow,
		Offerer:    cfg.Offerer,
		ICE:        func() []room.ICEServer { return t.ice(guest) },
		MaxMessage: mesh.MaxMessage,
		MaxStreams: 4,
		Logf:       scrubbed(t.o.Logf),
	}
	if t.o.LoopbackOnly {
		rc.Tune = room.LoopbackOnly()
	}
	rc.RelayOnly = t.o.RelayOnly
	m, err := room.NewMesh(sig, rc)
	if err != nil {
		sig.Close()
		close(r.ready)
		r.closeGuest()
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

// openGuests admits guests to topic on every hub for ttl: this device's
// own server's and every server it is a member of (and ones added later).
func (t *Transport) openGuests(topic string, ttl time.Duration) {
	if ttl > room.MaxGuestTTL {
		ttl = room.MaxGuestTTL
	}
	until := time.Now().Add(ttl)
	ctx, cancel := context.WithDeadline(context.Background(), until)
	g := guestWindow{until: until, ctx: ctx, cancel: cancel}
	t.mu.Lock()
	if old, ok := t.guests[topic]; ok {
		old.cancel()
	}
	t.guests[topic] = g
	var hubs []*hubConn
	for _, h := range t.hubs {
		hubs = append(hubs, h)
	}
	loc := t.local
	t.mu.Unlock()
	if loc != nil {
		if err := loc.hub.OpenGuest(topic, ttl); err != nil {
			t.o.Logf("roomnet: open the pairing room on this server: %v", err)
		}
	}
	for _, h := range hubs {
		go openGuest(ctx, h.client, topic, until)
	}
}

func (t *Transport) closeGuests(topic string) {
	t.mu.Lock()
	if g, ok := t.guests[topic]; ok {
		g.cancel()
	}
	delete(t.guests, topic)
	var hubs []*hubConn
	for _, h := range t.hubs {
		hubs = append(hubs, h)
	}
	loc := t.local
	t.mu.Unlock()
	if loc != nil {
		loc.hub.CloseGuest(topic)
	}
	for _, h := range hubs {
		h.client.CloseGuest(topic)
	}
}

type roomConn struct {
	t          *Transport
	topic      string
	ready      chan struct{}
	sig        room.Signaler
	mesh       *room.Mesh
	guest      *hubConn
	guestTopic bool
	once       sync.Once
}

func (r *roomConn) Send(to, typ string, body any) error { return r.sig.Send(to, typ, body) }
func (r *roomConn) Connect(peer string)                 { r.mesh.Connect(peer) }

// Close closes every connection (go-room sends rtc.bye and doesn't
// reconnect after a local close) and leaves the topic, and closes the
// pairing room to guests.
func (r *roomConn) Close() {
	r.once.Do(func() {
		r.mesh.Close()
		r.sig.Close()
		r.closeGuest()
	})
}

func (r *roomConn) closeGuest() {
	if r.guest != nil {
		r.guest.close()
	}
	if r.guestTopic {
		r.t.closeGuests(r.topic)
	}
}

// muxCarrier is every hub as one carrier: room.Open takes a fixed list,
// but hubs come and go (a node.addr row arrives, a server switches off,
// this device's own server starts), so each leg fans out to whichever
// hubs there are now and joins ones added later.
type muxCarrier struct{ t *Transport }

func (m muxCarrier) Join(topic string, deliver func([]byte, netip.AddrPort), up func()) (room.Leg, error) {
	t := m.t
	l := &muxLeg{t: t, topic: topic, deliver: deliver, up: up, subs: map[string]room.Leg{}}
	t.carMu.Lock()
	defer t.carMu.Unlock()
	t.mu.Lock()
	t.legs[l] = struct{}{}
	type car struct {
		key, server string
		c           room.Carrier
	}
	var cars []car
	if t.lan != nil {
		cars = append(cars, car{lanKey, "", t.lan.Carrier()})
	}
	if t.local != nil {
		cars = append(cars, car{localKey, "", t.local.hub.Local()})
	}
	for id, h := range t.hubs {
		cars = append(cars, car{id, id, h.client.Carrier()})
	}
	t.mu.Unlock()
	for _, c := range cars {
		l.add(c.key, c.c, c.server)
	}
	return l, nil
}

type muxLeg struct {
	t       *Transport
	topic   string
	deliver func([]byte, netip.AddrPort)
	up      func()

	mu     sync.Mutex
	subs   map[string]room.Leg
	closed bool
}

// add joins carrier c for this leg's topic under key; server, when set, is
// the hub's device id, whose working address is recorded when it comes up.
func (l *muxLeg) add(key string, c room.Carrier, server string) {
	l.mu.Lock()
	if l.closed || l.subs[key] != nil {
		l.mu.Unlock()
		return
	}
	l.mu.Unlock()
	up := func() {
		if server != "" && l.t.o.Servers != nil {
			l.t.mu.Lock()
			h := l.t.hubs[server]
			l.t.mu.Unlock()
			if h != nil {
				if addr := h.client.Addr(); addr != "" {
					l.t.o.Servers.Worked(server, addr)
				}
			}
		}
		if l.up != nil {
			l.up()
		}
	}
	sub, err := c.Join(l.topic, l.deliver, up)
	if err != nil {
		l.t.o.Logf("roomnet: join a hub: %v", err)
		return
	}
	l.mu.Lock()
	if l.closed || l.subs[key] != nil {
		l.mu.Unlock()
		sub.Close()
		return
	}
	l.subs[key] = sub
	l.mu.Unlock()
}

func (l *muxLeg) remove(key string) {
	l.mu.Lock()
	sub := l.subs[key]
	delete(l.subs, key)
	l.mu.Unlock()
	if sub != nil {
		sub.Close()
	}
}

func (l *muxLeg) list() []room.Leg {
	l.mu.Lock()
	defer l.mu.Unlock()
	out := make([]room.Leg, 0, len(l.subs))
	for _, s := range l.subs {
		out = append(out, s)
	}
	return out
}

func (l *muxLeg) Publish(to string, payload []byte) error {
	sent := 0
	var last error
	for _, s := range l.list() {
		if !s.Up() {
			continue
		}
		if err := s.Publish(to, payload); err != nil {
			last = err
			continue
		}
		sent++
	}
	if sent == 0 {
		if last != nil {
			return last
		}
		return room.ErrNotConnected
	}
	return nil
}

func (l *muxLeg) Seen(member string, from netip.AddrPort) {
	for _, s := range l.list() {
		s.Seen(member, from)
	}
}

func (l *muxLeg) Up() bool {
	for _, s := range l.list() {
		if s.Up() {
			return true
		}
	}
	return false
}

func (l *muxLeg) Close() {
	l.t.carMu.Lock()
	l.t.mu.Lock()
	delete(l.t.legs, l)
	l.t.mu.Unlock()
	l.t.carMu.Unlock()
	l.mu.Lock()
	l.closed = true
	subs := l.subs
	l.subs = map[string]room.Leg{}
	l.mu.Unlock()
	for _, s := range subs {
		s.Close()
	}
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

// HostPort is a typed server address with the port defaulted: "1.2.3.4"
// or "pc.example" alone gets port, an IPv6 address may go without brackets.
func HostPort(in string, port int) (string, error) {
	in = strings.TrimSpace(in)
	if in == "" {
		return "", errors.New("no address")
	}
	if h, p, err := net.SplitHostPort(in); err == nil {
		n, err := strconv.Atoi(p)
		if err != nil || n <= 0 || n > 65535 || h == "" {
			return "", errors.New("that isn't a host and port")
		}
		return net.JoinHostPort(h, p), nil
	}
	if port <= 0 {
		return "", errors.New("the address needs a port, like 203.0.113.7:24817")
	}
	if strings.ContainsAny(in, "/ ") {
		return "", errors.New("that isn't a host name or address")
	}
	return net.JoinHostPort(strings.Trim(in, "[]"), strconv.Itoa(port)), nil
}
