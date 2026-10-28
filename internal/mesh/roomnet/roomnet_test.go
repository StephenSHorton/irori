package roomnet

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/hex"
	"io"
	"net"
	"net/netip"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	room "github.com/StephenSHorton/go-room"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/store"
)

// These tests run real pion peers on 127.0.0.1 only, with loopback
// candidates and no STUN. Signaling is a MemLAN (an in-process broadcast
// domain: no socket) and real hubs listening on 127.0.0.1. Nothing binds a
// wildcard address, broadcasts or reaches the internet. They fail, not
// skip, when the peers can't connect.

func TestMeshSecretIsGoRoomsSecretFromKey(t *testing.T) {
	seed := make([]byte, 32)
	seed[3] = 9
	got := mesh.MeshSecret(seed, "abc", 4)
	want := room.SecretFromKey(mesh.MeshApp, seed, mesh.MeshInfo("abc", 4))
	if got.Key != want.Key || got.Topic != want.Topic {
		t.Fatalf("mesh %s, go-room %s", got.Topic, want.Topic)
	}
}

// testLAN is one MemLAN the test devices share.
type testLAN struct {
	lan  *room.MemLAN
	mu   sync.Mutex
	next byte
}

func newTestLAN() *testLAN { return &testLAN{lan: room.NewMemLAN()} }

func (l *testLAN) conn() net.PacketConn {
	l.mu.Lock()
	l.next++
	n := l.next
	l.mu.Unlock()
	return l.lan.Conn(netip.AddrPortFrom(netip.AddrFrom4([4]byte{10, 0, 0, n}), mesh.LANPort))
}

// how a test device reaches the others.
type how struct {
	lan  *testLAN          // on this LAN; nil: no LAN carrier
	host bool              // runs a hub on 127.0.0.1, as a home server does
	turn bool              // the hub's port also runs TURN (UDP, and TCP through SplitTCP)
	dial map[string]string // home servers to dial: device id → address
	// relayOnly forces relay-only ICE, so a connection can only come up
	// through a home server's TURN.
	relayOnly bool
}

type dev struct {
	t       *testing.T
	name    string
	dir     string
	ident   *identity.Identity
	chain   *identity.Chain
	st      *store.Store
	files   *blob.Files
	node    *mesh.Node
	tr      *Transport
	servers *mesh.Servers
	hub     *room.Hub
	hubAddr string
	stop    func()
}

func newDev(t *testing.T, name string, personFrom string) *dev {
	t.Helper()
	d := &dev{t: t, name: name, dir: t.TempDir()}
	if personFrom != "" {
		raw, err := os.ReadFile(filepath.Join(personFrom, identity.PersonFile))
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(d.dir, identity.PersonFile), raw, 0o600); err != nil {
			t.Fatal(err)
		}
	}
	dv, err := device.LoadOrCreate(d.dir)
	if err != nil {
		t.Fatal(err)
	}
	if d.ident, err = identity.Load(d.dir, dv, identity.FileKeystore{}); err != nil {
		t.Fatal(err)
	}
	d.chain = identity.NewChain(d.ident.Person.ID, d.ident.Person.RootDeviceID, dv.ID)
	if d.st, err = store.OpenWith(context.Background(), filepath.Join(d.dir, "irori.db"), dv.ID, hlc.New(nil),
		store.OpenOptions{Signer: d.ident, Verifier: d.chain}); err != nil {
		t.Fatal(err)
	}
	if err := d.ident.EnsureSelfCert(context.Background(), d.st, d.chain, identity.RoleDesktop); err != nil {
		t.Fatal(err)
	}
	if d.files, err = blob.OpenFiles(filepath.Join(d.dir, "files")); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if d.stop != nil {
			d.stop()
		}
		d.st.Close()
	})
	return d
}

func (d *dev) id() string { return d.ident.Device.ID }

func (d *dev) logf(f string, a ...any) { d.t.Logf(d.name+": "+f, a...) }

// start runs the device's net as the Core does: the home server directory
// following the log, the transport on h's carriers, and the mesh node.
func (d *dev) start(h how) {
	ctx, cancel := context.WithCancel(context.Background())
	d.servers = mesh.NewServers(d.id(), d.chain, "")
	if err := d.servers.Load(ctx, d.st); err != nil {
		d.t.Fatal(err)
	}
	followDone := make(chan struct{})
	go func() { defer close(followDone); d.servers.Follow(ctx, d.st) }()
	for id, addr := range h.dial {
		d.servers.Hint(id, []string{addr})
	}
	o := Options{Self: d.id(), Pub: d.ident.DevicePub(), Sign: d.ident.Sign, Servers: d.servers,
		Expect:       func(id string, pub ed25519.PublicKey) error { return mesh.CheckDevice(d.chain, d.id(), id, pub) },
		LoopbackOnly: true, RelayOnly: h.relayOnly, NoLAN: h.lan == nil, Logf: d.logf}
	if h.lan != nil {
		o.LANConn = h.lan.conn()
		o.LANTargets = func() []netip.AddrPort { return []netip.AddrPort{h.lan.lan.Broadcast()} }
	}
	d.tr = New(o)
	if h.host {
		d.host(h.turn)
	}
	d.node = mesh.New(mesh.Options{Store: d.st, Ident: d.ident, Chain: d.chain, Net: d.tr, Version: "0.4.0-test", Files: d.files,
		Intervals: mesh.Intervals{Presence: 200 * time.Millisecond, Ping: 200 * time.Millisecond, Vector: 300 * time.Millisecond},
		Logf:      d.logf})
	done := make(chan struct{})
	go func() {
		defer close(done)
		_ = d.node.Run(ctx)
	}()
	d.stop = func() {
		cancel()
		<-done
		<-followDone
		if d.hub != nil {
			d.hub.Close()
		}
		d.tr.Close()
		d.stop = nil
	}
}

// host runs a real hub on 127.0.0.1 for this device's rooms, admitting the
// person's certified devices and bridging guest topics onto its LAN, as
// the home server does (internal/home adds port mapping). With withTURN,
// the same port number runs TURN as internal/home sets it up, except that
// relayed sockets are on 127.0.0.1 and may relay to loopback.
func (d *dev) host(withTURN bool) {
	var ln net.Listener
	var udp net.PacketConn
	for range 20 {
		var err error
		if ln, err = net.Listen("tcp4", "127.0.0.1:0"); err != nil {
			d.t.Fatal(err)
		}
		if !withTURN {
			break
		}
		if udp, err = net.ListenPacket("udp4", ln.Addr().String()); err == nil {
			break
		}
		ln.Close()
		ln = nil
	}
	if ln == nil {
		d.t.Fatal("no port free for TCP and UDP both")
	}
	var ts *room.TURNServer
	hubLn := ln
	if withTURN {
		var turnLn net.Listener
		hubLn, turnLn = room.SplitTCP(ln)
		cfg := room.TURNConfig{UDP: udp, TCP: turnLn, RelayIP: netip.MustParseAddr("127.0.0.1"),
			Authorize: func(user string) error {
				if _, ok := d.chain.Cert(user); !ok {
					return mesh.ErrRefused
				}
				return nil
			},
			AllowPeer: func(string, netip.Addr) bool { return true }, // loopback, for the test
			Logf:      d.logf}
		_, _ = rand.Read(cfg.Key[:])
		var err error
		if ts, err = room.NewTURN(cfg); err != nil {
			d.t.Fatal(err)
		}
		d.t.Cleanup(ts.Close)
	}
	hub, err := room.NewHub(room.HubConfig{
		Self:     d.id(),
		Identity: room.Identity{Pub: d.ident.DevicePub(), Sign: d.ident.Sign},
		Authorize: func(id string, pub ed25519.PublicKey) error {
			return mesh.CheckDevice(d.chain, d.id(), id, pub)
		},
		TURN:   ts,
		Bridge: d.tr.LAN(),
		Limits: room.HubLimits{FailBackoff: -1},
		Logf:   d.logf,
	})
	if err != nil {
		d.t.Fatal(err)
	}
	go func() { _ = hub.Serve(hubLn) }()
	d.hub, d.hubAddr = hub, ln.Addr().String()
	var ice func() []room.ICEServer
	if ts != nil {
		addr := d.hubAddr
		ice = func() []room.ICEServer {
			c := ts.Issue(d.id())
			return []room.ICEServer{{URLs: []string{"turn:" + addr + "?transport=udp", "turn:" + addr + "?transport=tcp"},
				Username: c.Username, Credential: c.Credential}}
		}
	}
	d.tr.SetLocal(hub, ice)
}

// writeAddr writes this device's node.addr for its hub, as the home server
// does, so devices it pairs learn where it is.
func (d *dev) writeAddr() {
	_, port, _ := net.SplitHostPort(d.hubAddr)
	n, _ := strconv.Atoi(port)
	if _, err := d.st.Append(context.Background(), "irori", store.KindNodeAddr,
		mesh.NodeAddr{V: 1, Port: n, LAN: []string{"127.0.0.1"}, Reach: mesh.ReachUnmapped, At: time.Now().UnixMilli()}); err != nil {
		d.t.Fatal(err)
	}
}

// certify makes b one of a's person's devices, as pairing does (§3 step
// 6): b was made with a's person.json, a writes b's cert, b learns the rows.
func certify(t *testing.T, a, b *dev) {
	t.Helper()
	ctx := context.Background()
	add, err := a.ident.CertFor(b.id(), hex.EncodeToString(b.ident.DevicePub()), b.name, identity.RoleDesktop, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := a.st.Append(ctx, "irori", store.KindPersonDeviceAdded, add); err != nil {
		t.Fatal(err)
	}
	learn(t, a, b)
}

// learn folds every person row a holds into b's chain.
func learn(t *testing.T, a, b *dev) {
	t.Helper()
	person, _ := a.st.List(context.Background(), store.Filter{Kinds: []string{"person."}})
	for range 2 {
		for _, ev := range person {
			_ = b.chain.Learn(ev)
		}
	}
}

func (d *dev) vector() store.Vector {
	v, err := d.st.Vector(context.Background())
	if err != nil {
		d.t.Fatal(err)
	}
	return v
}

func (d *dev) has(ev store.Event) bool {
	evs, err := d.st.List(context.Background(), store.Filter{DeviceID: ev.DeviceID, AfterSeq: ev.Seq - 1, Limit: 1})
	return err == nil && len(evs) == 1 && evs[0].Seq == ev.Seq
}

func (d *dev) peerUp(peer string) bool {
	for _, p := range d.node.Status().Peers {
		if p.DeviceID == peer && p.State == "connected" && p.Path != "" {
			return true
		}
	}
	return false
}

func eventually(t *testing.T, what string, within time.Duration, ok func() bool) {
	t.Helper()
	deadline := time.Now().Add(within)
	for time.Now().Before(deadline) {
		if ok() {
			return
		}
		time.Sleep(20 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for: %s", what)
}

func sameVector(a, b store.Vector) bool {
	if len(a) != len(b) {
		return false
	}
	for k, v := range a {
		if b[k] != v {
			return false
		}
	}
	return true
}

// Two Cores' nets in one process over a real hub on 127.0.0.1 (the home
// server's, no LAN): a backfill and live rows both ways, then a revoke
// closes the connection.
func TestTwoNodesSyncOverTheHub(t *testing.T) {
	ctx := context.Background()
	a := newDev(t, "pc", "")
	big := strings.Repeat("y", 8000)
	for i := range 600 { // a backfill bigger than one batch and one MiB
		if _, err := a.st.Append(ctx, "irori", "capture.turn", map[string]any{"i": i, "text": big}); err != nil {
			t.Fatal(err)
		}
	}
	b := newDev(t, "mac", a.dir)
	certify(t, a, b)
	eb, err := b.st.Append(ctx, "irori", store.KindAgentSeen, map[string]any{"key": b.id() + "/s"})
	if err != nil {
		t.Fatal(err)
	}
	a.start(how{host: true})
	b.start(how{dial: map[string]string{a.id(): a.hubAddr}})
	eventually(t, "the backfill and b's row cross", 60*time.Second, func() bool {
		return sameVector(a.vector(), b.vector()) && a.has(eb)
	})
	hubs := b.tr.Hubs()
	if len(hubs) != 1 || hubs[0].ID != a.id() || !hubs[0].Up || hubs[0].Addr != a.hubAddr {
		t.Fatalf("b's hubs: %+v", hubs)
	}
	ea, _ := a.st.Append(ctx, "irori", store.KindAgentSeen, map[string]any{"key": a.id() + "/live"})
	eventually(t, "a live row crosses", 20*time.Second, func() bool { return b.has(ea) })
	var connected bool
	for _, p := range a.node.Status().Peers {
		connected = connected || (p.DeviceID == b.id() && p.State == "connected" && p.Path == "direct")
	}
	if !connected {
		t.Fatalf("a's view of b: %+v", a.node.Status().Peers)
	}
	// Revoke b: the connection closes, and b's later rows are refused.
	if err := a.node.Revoke(ctx, b.id(), "test"); err != nil {
		t.Fatal(err)
	}
	eventually(t, "the revoked device drops", 20*time.Second, func() bool {
		for _, p := range a.node.Status().Peers {
			if p.DeviceID == b.id() {
				return p.Path == ""
			}
		}
		return false
	})
	late, _ := b.st.Append(ctx, "irori", store.KindAgentSeen, map[string]any{"key": b.id() + "/late"})
	time.Sleep(time.Second)
	if a.has(late) {
		t.Fatal("a stored a row the revoked device wrote after its cut")
	}
}

// Two devices on one network with no home server at all find each other
// on the LAN carrier and sync (§4.2, §7.1 "the physics").
func TestTwoNodesSyncOverTheLANAlone(t *testing.T) {
	ctx := context.Background()
	lan := newTestLAN()
	a := newDev(t, "pc", "")
	b := newDev(t, "mac", a.dir)
	certify(t, a, b)
	ea, _ := a.st.Append(ctx, "irori", store.KindAgentSeen, map[string]any{"key": a.id() + "/s"})
	eb, _ := b.st.Append(ctx, "irori", store.KindAgentSeen, map[string]any{"key": b.id() + "/s"})
	a.start(how{lan: lan})
	b.start(how{lan: lan})
	eventually(t, "rows cross on the LAN", 60*time.Second, func() bool { return a.has(eb) && b.has(ea) })
	if len(a.tr.Hubs()) != 0 || len(b.tr.Hubs()) != 0 {
		t.Fatal("a hub was dialed with no home server")
	}
}

// Three devices with the one that runs the server stopped: the other two
// still sync over the LAN (§10).
func TestServerOffTheOthersStillSyncOnTheLAN(t *testing.T) {
	ctx := context.Background()
	lan := newTestLAN()
	s := newDev(t, "node", "")
	a := newDev(t, "pc", s.dir)
	b := newDev(t, "mac", s.dir)
	certify(t, s, a)
	certify(t, s, b)
	learn(t, s, a) // b's cert, written after a learned
	s.start(how{lan: lan, host: true})
	s.writeAddr()
	a.start(how{lan: lan, dial: map[string]string{s.id(): s.hubAddr}})
	b.start(how{lan: lan, dial: map[string]string{s.id(): s.hubAddr}})
	eventually(t, "a and b each reach the server", 60*time.Second, func() bool { return s.peerUp(a.id()) && s.peerUp(b.id()) })
	s.stop()
	ea, _ := a.st.Append(ctx, "irori", store.KindAgentSeen, map[string]any{"key": a.id() + "/after"})
	eb, _ := b.st.Append(ctx, "irori", store.KindAgentSeen, map[string]any{"key": b.id() + "/after"})
	eventually(t, "a and b sync with the server stopped", 60*time.Second, func() bool { return a.has(eb) && b.has(ea) })
}

// Streams on a real go-room connection move bytes both ways while the sync
// channel stays live (blobs use them, §4.3).
func TestConnStreamsOverGoRoomOnLoopback(t *testing.T) {
	lan := newTestLAN()
	key := make([]byte, 32)
	_, _ = rand.Read(key)
	s := mesh.MeshSecret(key, "p", 1)
	type side struct {
		id   string
		key  ed25519.PrivateKey
		conn chan mesh.Conn
		room mesh.Room
	}
	mk := func(id string) *side {
		_, k, _ := ed25519.GenerateKey(rand.Reader)
		return &side{id: id, key: k, conn: make(chan mesh.Conn, 1)}
	}
	x, y := mk("aaaa"), mk("bbbb")
	pubs := map[string]ed25519.PublicKey{x.id: x.key.Public().(ed25519.PublicKey), y.id: y.key.Public().(ed25519.PublicKey)}
	for _, sd := range []*side{x, y} {
		sd := sd
		tr := New(Options{Self: sd.id, LoopbackOnly: true, LANConn: lan.conn(),
			LANTargets: func() []netip.AddrPort { return []netip.AddrPort{lan.lan.Broadcast()} }, Logf: t.Logf})
		t.Cleanup(tr.Close)
		r, err := tr.Join(s, mesh.RoomConfig{
			Self: sd.id, Pub: pubs[sd.id], Sign: func(m []byte) []byte { return ed25519.Sign(sd.key, m) },
			Accept: func(peer string, pub ed25519.PublicKey) error {
				if !pub.Equal(pubs[peer]) {
					return mesh.ErrRefused
				}
				return nil
			},
			OnPeer: func(c mesh.Conn) { sd.conn <- c },
		})
		if err != nil {
			t.Fatal(err)
		}
		sd.room = r
		t.Cleanup(r.Close)
	}
	y.room.Connect(x.id)
	x.room.Connect(y.id)
	var cx, cy mesh.Conn
	for _, w := range []struct {
		sd *side
		c  *mesh.Conn
	}{{x, &cx}, {y, &cy}} {
		select {
		case *w.c = <-w.sd.conn:
		case <-time.After(30 * time.Second):
			t.Fatalf("%s: no connection on loopback", w.sd.id)
		}
	}
	payload := bytes.Repeat([]byte("blob"), 64<<10) // 256 KiB
	cy.OnStream(func(label string, st io.ReadWriteCloser) {
		defer st.Close()
		got, _ := io.ReadAll(st)
		if label != "blob/1" || !bytes.Equal(got, payload) {
			t.Errorf("stream %s: %d bytes", label, len(got))
		}
	})
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	st, err := cx.OpenStream(ctx, "blob/1")
	if err != nil {
		t.Fatal(err)
	}
	go func() {
		_ = cx.Send(ctx, []byte("live row"))
	}()
	if _, err := st.Write(payload); err != nil {
		t.Fatal(err)
	}
	st.Close()
	msg, err := cy.Recv(ctx)
	if err != nil || string(msg) != "live row" {
		t.Fatalf("sync message: %q %v", msg, err)
	}
	if l, r := cx.Fingerprints(); l == "" || r == "" {
		t.Fatal("no DTLS fingerprints")
	}
	time.Sleep(200 * time.Millisecond)
}

// A lone transport opens nothing until a room is joined: no LAN socket and
// no hub connection (§4.5).
func TestNothingOpensBeforeTheFirstJoin(t *testing.T) {
	var dialed bool
	conn, err := net.ListenPacket("udp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	srv := mesh.NewServers("me", nil, "")
	srv.Hint("node", []string{"127.0.0.1:1"})
	tr := New(Options{Self: "me", Servers: srv, LANConn: conn, LANTargets: func() []netip.AddrPort { return nil },
		Expect: func(string, ed25519.PublicKey) error { return nil },
		Dial: func(ctx context.Context, addr string) (net.Conn, error) {
			dialed = true
			return nil, io.EOF
		}})
	time.Sleep(200 * time.Millisecond)
	if dialed || len(tr.Hubs()) != 0 {
		t.Fatal("a hub was dialed before any room")
	}
	tr.Close()
}

// The ICE list names only this device's own servers and the advanced
// relay: never a public STUN server (§4.4).
func TestICENeverListsPublicSTUN(t *testing.T) {
	tr := New(Options{Self: "me", NoLAN: true})
	defer tr.Close()
	if got := tr.ice(nil); len(got) != 0 {
		t.Fatalf("ICE with no server: %+v", got)
	}
	ln, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	_, key, _ := ed25519.GenerateKey(rand.Reader)
	hub, err := room.NewHub(room.HubConfig{Self: "me", Identity: room.KeyIdentity(key),
		Authorize: func(string, ed25519.PublicKey) error { return mesh.ErrRefused }})
	if err != nil {
		t.Fatal(err)
	}
	go func() { _ = hub.Serve(ln) }()
	defer hub.Close()
	own := []room.ICEServer{{URLs: []string{"stun:127.0.0.1:9"}}}
	tr.SetLocal(hub, func() []room.ICEServer { return own })
	got := tr.ice(nil)
	if len(got) != 1 || got[0].URLs[0] != "stun:127.0.0.1:9" {
		t.Fatalf("ICE with this device's server: %+v", got)
	}
	for _, s := range got {
		for _, u := range s.URLs {
			for _, d := range room.DefaultSTUN {
				for _, du := range d.URLs {
					if u == du {
						t.Fatalf("public STUN in the list: %s", u)
					}
				}
			}
		}
	}
}

// The away-from-home path: with relay-only ICE on both devices, the only
// way up is through the home server's TURN, with credentials from turn.get
// over its hub and the URLs roomnet builds from the hub's address.
func TestTwoNodesConnectThroughTheServersTURN(t *testing.T) {
	ctx := context.Background()
	a := newDev(t, "pc", "")
	b := newDev(t, "mac", a.dir)
	certify(t, a, b)
	eb, err := b.st.Append(ctx, "irori", store.KindAgentSeen, map[string]any{"key": b.id() + "/s"})
	if err != nil {
		t.Fatal(err)
	}
	a.start(how{host: true, turn: true, relayOnly: true})
	b.start(how{dial: map[string]string{a.id(): a.hubAddr}, relayOnly: true})
	eventually(t, "b's row crosses a relayed connection", 60*time.Second, func() bool { return a.has(eb) })
	for _, p := range a.node.Status().Peers {
		if p.DeviceID == b.id() && p.State == "connected" {
			if p.Path != "relay" {
				t.Fatalf("path %q, want relay", p.Path)
			}
			return
		}
	}
	t.Fatalf("a's view of b: %+v", a.node.Status().Peers)
}

// Devices says why the LAN carrier sends nothing.
func TestLANErrorWithNowhereToSend(t *testing.T) {
	tr := New(Options{Self: "me", LANConn: room.NewMemLAN().Conn(netip.MustParseAddrPort("10.0.0.9:38419")),
		LANTargets: func() []netip.AddrPort { return nil }})
	defer tr.Close()
	if tr.LANError() != "" {
		t.Fatal("a LAN error before anything opened the carrier")
	}
	_ = tr.LAN()
	if !strings.Contains(tr.LANError(), "Public") {
		t.Fatalf("LANError %q", tr.LANError())
	}
}
